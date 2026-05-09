//! AccessKit accessibility-tree emission — W2.4 from the
//! catch-and-surpass-niri plan. Pure scope: expose margo's
//! window list as an accessibility tree on the freedesktop a11y
//! D-Bus surface so screen readers (Orca, AT-SPI consumers)
//! can announce window changes and navigate the compositor.
//!
//! Pattern is a direct port of niri's `src/a11y.rs`:
//!
//!   * The AccessKit `Adapter` lives on its own thread because
//!     it can deadlock against the compositor mainloop under
//!     load. Communication is via a bounded `mpsc::sync_channel`
//!     of `TreeUpdate` messages.
//!   * On every arrange / focus change the compositor side
//!     calls [`A11yState::publish_window_list`] which builds a
//!     fresh `TreeUpdate` and sends it to the adapter thread.
//!     Drops on full channel — losing one update is acceptable
//!     (the next arrange re-publishes), and back-pressure on
//!     the render loop is unacceptable.
//!   * On startup the adapter thread waits for the first
//!     `update_if_active` call before activating; we send an
//!     `InitialTree` message once so it has something to show
//!     when a screen reader connects.
//!
//! Scope intentionally narrow: window list + per-window
//! `(app_id, title)` only. Per-tag grouping, MRU, overview,
//! exit-confirmation, screenshot UI surfaces are all niri-only
//! features that can be added later as their tree-node owners
//! arrive.

#![cfg(feature = "a11y")]

use std::sync::mpsc;
use std::thread;

use accesskit::{
    ActionHandler, ActionRequest, ActivationHandler, DeactivationHandler, Live, Node, NodeId,
    Role, Tree, TreeUpdate,
};
use accesskit_unix::Adapter;

const ID_ROOT: NodeId = NodeId(0);
/// First window id; per-window ids are `ID_FIRST_WINDOW + idx`.
const ID_FIRST_WINDOW: u64 = 100;

/// Lightweight description of a single client — the bits a screen
/// reader cares about. Built by the compositor side from
/// `MargoClient` fields per emit so the a11y thread doesn't
/// borrow against `MargoState`.
#[derive(Debug, Clone)]
pub struct WindowSnapshot {
    pub app_id: String,
    pub title: String,
    pub is_focused: bool,
}

/// Per-MargoState a11y context. `start()` spawns the adapter
/// thread; `publish_window_list()` flushes a fresh tree to it.
pub struct A11yState {
    to_accesskit: Option<mpsc::SyncSender<TreeUpdate>>,
}

impl A11yState {
    pub fn new() -> Self {
        Self { to_accesskit: None }
    }

    /// Spawn the AccessKit adapter thread. Idempotent — if the
    /// channel is already wired the call is a no-op. Logs and
    /// drops the spawn error on failure (a11y is best-effort;
    /// margo keeps running without it).
    pub fn start(&mut self) {
        if self.to_accesskit.is_some() {
            return;
        }
        let (to_accesskit, from_main) = mpsc::sync_channel::<TreeUpdate>(8);

        // Adapter must live on a dedicated thread — niri's bug
        // report: it can deadlock against the compositor's
        // wayland event loop if both touch zbus on the same
        // thread under contention.
        let res = thread::Builder::new()
            .name("margo-a11y-adapter".to_owned())
            .spawn(move || {
                // Empty-shell handlers — margo doesn't act on
                // Orca-side requests (no "make this window
                // focused" feedback path yet); we just publish.
                let handler = NoopHandler;
                let mut adapter = Adapter::new(handler.clone(), handler.clone(), handler);
                while let Ok(tree) = from_main.recv() {
                    let is_focused = tree.focus != ID_ROOT;
                    adapter.update_if_active(move || tree);
                    adapter.update_window_focus_state(is_focused);
                }
                tracing::info!("a11y adapter thread exiting");
            });

        match res {
            Ok(_) => {
                self.to_accesskit = Some(to_accesskit);
                tracing::info!("AccessKit adapter started");
            }
            Err(e) => {
                tracing::warn!("a11y adapter thread spawn failed: {e:?}");
            }
        }
    }

    /// Publish the current window list + focus to the adapter.
    /// `windows` is iterated in compositor order; the focused
    /// window's id becomes the tree's `focus`.
    pub fn publish_window_list<'a>(&mut self, windows: impl IntoIterator<Item = &'a WindowSnapshot>) {
        let Some(tx) = self.to_accesskit.as_ref() else {
            return;
        };

        let mut nodes = Vec::new();
        let mut child_ids = Vec::new();
        let mut focused_id = ID_ROOT;
        for (i, w) in windows.into_iter().enumerate() {
            let id = NodeId(ID_FIRST_WINDOW + i as u64);
            let mut node = Node::new(Role::Window);
            // Screen readers prefer "AppName: Title" — gives
            // context for similarly-titled windows in the same
            // app. Falls back gracefully if either is empty.
            let label = match (w.app_id.as_str(), w.title.as_str()) {
                ("", t) => t.to_string(),
                (a, "") => a.to_string(),
                (a, t) => format!("{a}: {t}"),
            };
            node.set_label(label);
            if w.is_focused {
                focused_id = id;
            }
            nodes.push((id, node));
            child_ids.push(id);
        }

        let mut root = Node::new(Role::Window);
        root.set_label("margo");
        root.set_children(child_ids);
        nodes.insert(0, (ID_ROOT, root));

        let update = TreeUpdate {
            nodes,
            tree: Some(Tree::new(ID_ROOT)),
            focus: focused_id,
        };

        // try_send — never block the render loop on a full a11y
        // channel. Losing an update is fine; the next arrange
        // re-publishes.
        match tx.try_send(update) {
            Ok(()) => {}
            Err(mpsc::TrySendError::Full(_)) => {
                tracing::trace!("a11y channel full, dropping update");
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                tracing::warn!("a11y adapter thread gone, disabling further publishes");
                self.to_accesskit = None;
            }
        }
    }

    /// Push a transient announcement (live region) — used for
    /// state changes that don't fit on the tree (e.g. "tag 5
    /// activated", "scratchpad shown"). Orca speaks it
    /// immediately. Defaults to `Live::Polite` so it doesn't
    /// interrupt a current speech.
    pub fn announce(&mut self, msg: impl Into<String>) {
        let Some(tx) = self.to_accesskit.as_ref() else {
            return;
        };
        let msg = msg.into();
        if msg.is_empty() {
            return;
        }
        let mut node = Node::new(Role::Label);
        node.set_label(msg);
        node.set_live(Live::Polite);
        let id_announce = NodeId(1);
        let mut root = Node::new(Role::Window);
        root.set_label("margo");
        root.set_children(vec![id_announce]);
        let update = TreeUpdate {
            nodes: vec![(ID_ROOT, root), (id_announce, node)],
            tree: Some(Tree::new(ID_ROOT)),
            focus: id_announce,
        };
        let _ = tx.try_send(update);
    }
}

impl Default for A11yState {
    fn default() -> Self {
        Self::new()
    }
}

/// AccessKit demands `Send + Sync + 'static` handlers; margo
/// doesn't surface a11y-side actions back into the compositor
/// (yet). Stub all three trait impls — every callback is a
/// silent no-op. `derive(Clone)` so the same handler can be
/// passed three times to `Adapter::new`.
#[derive(Clone)]
struct NoopHandler;

impl ActivationHandler for NoopHandler {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        // Empty initial tree — the first real
        // `publish_window_list` call replaces this.
        let mut root = Node::new(Role::Window);
        root.set_label("margo");
        Some(TreeUpdate {
            nodes: vec![(ID_ROOT, root)],
            tree: Some(Tree::new(ID_ROOT)),
            focus: ID_ROOT,
        })
    }
}

impl ActionHandler for NoopHandler {
    fn do_action(&mut self, _request: ActionRequest) {
        // Future: route a11y "click this window" requests into
        // a focus dispatch. For now just log so we know if a
        // real screen reader sends something.
        tracing::trace!("a11y action ignored (no handler)");
    }
}

impl DeactivationHandler for NoopHandler {
    fn deactivate_accessibility(&mut self) {
        tracing::trace!("a11y deactivated");
    }
}
