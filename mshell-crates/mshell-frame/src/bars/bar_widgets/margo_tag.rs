//! Per-tag pill button for the bar's MargoTags row.
//!
//! Each pill is a `gtk::Button` rendering the tag's 1-indexed
//! number ("1".."9") and switching CSS classes to convey the three
//! tag states the user asked for:
//!
//!   * **active**   — this tag is the focused workspace on its
//!                    owner monitor. Styled with the accent
//!                    `ok-button-primary` + `.tag-active`.
//!   * **occupied** — there's at least one (non-minimised) client
//!                    on the tag, but it isn't focused. Styled
//!                    with `ok-button-surface` + `.tag-occupied`
//!                    (a small accent dot under the digit).
//!   * **empty**    — no clients, not focused. Styled with
//!                    `ok-button-surface` + `.tag-empty` (digit
//!                    rendered at reduced opacity).
//!
//! Window-count and active-state are both reactive: per-tag
//! `workspace.windows.watch()` + `workspace.monitor_id.watch()`
//! streams are spawned at init so the pill picks up content
//! changes without waiting for the next state.json poll-cycle
//! reactive cascade.
//!
//! Click dispatches `mctl dispatch view <bitmask>` directly via a
//! subprocess — bypasses the Hyprland-shaped string parser in
//! `mshell-margo-client::dispatch` so the click path stays
//! deterministic and easy to debug.

use futures::StreamExt;
use mshell_margo_client::{Workspace, WorkspaceInfo};
use mshell_utils::margo::is_an_active_workspace;
use relm4::gtk::prelude::{ButtonExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::sync::Arc;
use tracing::warn;

#[derive(Debug)]
pub(crate) struct MargoTagModel {
    workspace: Arc<Workspace>,
    is_active: bool,
    windows: u16,
}

#[derive(Debug)]
pub(crate) enum MargoTagInput {
    ActiveUpdate(Vec<WorkspaceInfo>),
    WorkspaceClicked,
}

#[derive(Debug)]
pub(crate) enum MargoTagOutput {}

#[relm4::component(pub)]
impl Component for MargoTagModel {
    type CommandOutput = MargoTagCommandOutput;
    type Input = MargoTagInput;
    type Output = MargoTagOutput;
    type Init = Arc<Workspace>;

    view! {
        #[root]
        gtk::Box {
            set_hexpand: false,
            set_vexpand: false,

            #[name="button"]
            gtk::Button {
                #[watch]
                set_css_classes: &tag_classes(model.is_active, model.windows),
                set_hexpand: false,
                set_vexpand: false,
                connect_clicked[sender] => move |_| {
                    sender.input(MargoTagInput::WorkspaceClicked);
                },

                #[name="label"]
                gtk::Label {
                    add_css_class: "margo-tag-label",
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,
                    set_label: &model.workspace.id.get().to_string(),
                },
            }
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let is_active = is_an_active_workspace(&params);
        let windows = params.windows.get();

        let model = MargoTagModel {
            workspace: params.clone(),
            is_active,
            windows,
        };

        let widgets = view_output!();

        // Per-tag window-count watcher — translates each
        // `Reactive<u16>::set` (sync.rs in mshell-margo-client) into
        // an input the widget can render. Without this the pill
        // would only refresh on the much-coarser
        // `MargoTagsCommandOutput::WorkspacesChanged` cascade
        // (which only fires on workspace add/remove), so a client
        // opening on an already-existing tag wouldn't tick the
        // occupied dot.
        let ws_for_watch = params.clone();
        sender.command(move |out, shutdown| {
            async move {
                let mut stream = ws_for_watch.windows.watch();
                let shutdown_fut = shutdown.wait();
                tokio::pin!(shutdown_fut);
                loop {
                    tokio::select! {
                        () = &mut shutdown_fut => break,
                        next = stream.next() => {
                            match next {
                                Some(count) => {
                                    let _ = out.send(MargoTagCommandOutput::WindowsChanged(count));
                                }
                                None => break,
                            }
                        }
                    }
                }
            }
        });

        ComponentParts { model, widgets }
    }

    // `update` (NOT `update_with_view`): the framework auto-calls
    // `update_view(widgets, sender)` after this returns, which is what
    // re-evaluates the `#[watch] set_css_classes:` expression in the
    // view! macro. If we override `update_with_view` we lose that
    // auto-call — which is why an earlier version of this file left
    // the active pill stuck on the initially-focused tag forever:
    // model state changed, but the GTK button's class list didn't.
    fn update(
        &mut self,
        message: Self::Input,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            MargoTagInput::ActiveUpdate(workspace_infos) => {
                self.is_active = workspace_infos
                    .iter()
                    .any(|p| p.id == self.workspace.id.get());
            }
            MargoTagInput::WorkspaceClicked => {
                // mctl dispatch view <bitmask>. Tag id 1..=9 maps
                // directly to bit (id-1), so the bitmask is
                // `1u32 << (id - 1)`. Spawn a non-blocking
                // subprocess; we don't wait on it because the
                // visible result (focus change) is observed via the
                // next state.json poll → ActiveUpdate.
                let id = self.workspace.id.get();
                if !(1..=32).contains(&id) {
                    warn!(id, "MargoTag: out-of-range workspace id, skipping dispatch");
                    return;
                }
                let mask = 1u32 << (id - 1) as u32;
                tokio::spawn(async move {
                    let mut command = tokio::process::Command::new("mctl");
                    command.arg("dispatch").arg("view").arg(mask.to_string());
                    match command.status().await {
                        Ok(status) if status.success() => {}
                        Ok(status) => warn!(
                            ?status,
                            tag = id,
                            "mctl dispatch view returned non-zero"
                        ),
                        Err(e) => warn!(error = %e, tag = id, "mctl dispatch view spawn failed"),
                    }
                });
            }
        }
    }

    // Same reason as `update`: keep the auto-`update_view` call so
    // the occupied dot tracks window-count changes without us having
    // to invoke set_css_classes manually here.
    fn update_cmd(
        &mut self,
        message: Self::CommandOutput,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            MargoTagCommandOutput::WindowsChanged(count) => {
                self.windows = count;
            }
        }
    }
}

#[derive(Debug)]
pub(crate) enum MargoTagCommandOutput {
    WindowsChanged(u16),
}

/// Compose the CSS class list for the pill button in one place.
///
/// Each slot in the returned array is **one** CSS class name. GTK4's
/// `set_css_classes(&[&str])` adds each element as a distinct class,
/// so a string like `"margo-tag tag-active"` (space-joined) ends up
/// as a single token with a space in it — invalid as a selector match,
/// silently makes the `_margo_tag.scss` rules a no-op. Hence the
/// 4-slot array with `margo-tag` and `tag-*` as separate entries.
fn tag_classes(is_active: bool, windows: u16) -> [&'static str; 4] {
    if is_active {
        ["ok-button-primary", "ok-bar-widget", "margo-tag", "tag-active"]
    } else if windows > 0 {
        ["ok-button-surface", "ok-bar-widget", "margo-tag", "tag-occupied"]
    } else {
        ["ok-button-surface", "ok-bar-widget", "margo-tag", "tag-empty"]
    }
}
