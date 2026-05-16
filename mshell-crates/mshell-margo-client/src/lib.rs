//! Margo compositor client for MShell — replaces `wayle-hyprland`.
//!
//! The OkShell tree was written against `wayle-hyprland 0.2`, which
//! exposes a reactive view of Hyprland's IPC: a [`MargoService`]
//! handle with `workspaces` / `clients` / `monitors` properties
//! (each a reactive container with `.get()` snapshot + `.watch()`
//! stream), a `dispatch()` method that ships raw Hyprland command
//! strings, an `eval()` method for queries, and an `events()` async
//! stream of typed variants. The four `hyprland_*.rs` bar widgets
//! and two helper modules in `mshell-utils` / `mshell-services`
//! consume that surface.
//!
//! This crate mirrors the upstream API **field-for-field**, with the
//! same type names (`MargoService`, `MargoEvent`, `Workspace`,
//! `WorkspaceInfo`, `WorkspaceId`, `Client`, `Address`, `MonitorId`)
//! and the same field layout, so each widget compiles after a single
//! `use wayle_hyprland::*` → `use mshell_margo_client::*` edit. The
//! backend is intentionally stubbed in this Phase 2b commit:
//!
//!   * [`MargoService::new`] returns a service with empty
//!     reactive properties.
//!   * `dispatch()` / `eval()` / `events()` are no-ops.
//!   * Bar widgets render their empty state without crashing.
//!
//! Phase 2c will wire the same surface to margo's `dwl-ipc-v2` +
//! `foreign-toplevel-list` + `state.json`, with margo's tag bitmask
//! folded into the `WorkspaceId` axis (9 tags → IDs 1..=9, signed
//! to leave room for the negative IDs Hyprland uses for special
//! workspaces — margo emits none, so values stay positive).

use std::fmt;
use std::pin::Pin;
use std::sync::{Arc, RwLock};

use anyhow::Result;
use futures::{Stream, StreamExt};

pub mod state_json;
mod sync;

/// Re-export so callers that want to peek at the raw snapshot
/// (debug tooling, integration tests) can do so without a
/// second JSON round-trip.
pub use state_json::{read as read_state_json, StateJson};

// ── Reactive property ────────────────────────────────────────────────────────

/// Reactive container — mirrors `wayle_core::Reactive<T>`.
///
/// Supports the two access patterns the widgets use:
///   * `.get()`  — snapshot read, clones the current value.
///   * `.watch()` — `Stream<Item = T>` that yields on every change.
///
/// Internally an `Arc<RwLock<T>>` plus a tokio broadcast channel
/// (created lazily; absent until the first `.watch()` call).
/// Reactive<T> is intentionally `Clone` so a single backend can
/// hand the same reactive cell to multiple widget subscribers.
#[derive(Debug)]
pub struct Reactive<T: Clone + Send + Sync + 'static> {
    inner: Arc<PropertyInner<T>>,
}

#[derive(Debug)]
struct PropertyInner<T: Clone + Send + Sync + 'static> {
    value: RwLock<T>,
    /// Lazily-created channel. `None` until somebody subscribes.
    /// `set()` writes to this channel if it exists.
    sender: RwLock<Option<tokio::sync::broadcast::Sender<T>>>,
}

impl<T: Clone + Send + Sync + 'static> Reactive<T> {
    pub fn new(v: T) -> Self {
        Self {
            inner: Arc::new(PropertyInner {
                value: RwLock::new(v),
                sender: RwLock::new(None),
            }),
        }
    }

    /// Snapshot read — clones the inner value.
    pub fn get(&self) -> T {
        self.inner
            .value
            .read()
            .expect("Reactive<T> RwLock poisoned")
            .clone()
    }

    /// In-place write — used by the (future) margo backend to
    /// publish updates. Notifies any active subscribers.
    pub fn set(&self, v: T) {
        *self
            .inner
            .value
            .write()
            .expect("Reactive<T> RwLock poisoned") = v.clone();
        if let Some(tx) = self.inner.sender.read().unwrap().as_ref() {
            let _ = tx.send(v);
        }
    }

    /// Subscribe — returns a `Stream<Item = T>` that yields the
    /// new value on every `set()`. Lossy on slow consumers
    /// (broadcast `Lagged` errors are silently dropped, mirroring
    /// the upstream's `wayle_core::Reactive::watch` behaviour).
    pub fn watch(&self) -> Pin<Box<dyn Stream<Item = T> + Send>> {
        let mut sender_slot = self.inner.sender.write().unwrap();
        let sender = sender_slot.get_or_insert_with(|| {
            let (tx, _) = tokio::sync::broadcast::channel(64);
            tx
        });
        let rx = sender.subscribe();
        drop(sender_slot);
        Box::pin(tokio_stream::wrappers::BroadcastStream::new(rx).filter_map(
            |r| async move { r.ok() },
        ))
    }
}

impl<T: Clone + Send + Sync + 'static> Clone for Reactive<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

// ── Primitive type aliases ───────────────────────────────────────────────────

/// Output identifier. Hyprland uses `i64` here; margo's connector
/// names (`DP-3`, `eDP-1`, …) are hashed to `i64` slots by the
/// future backend so the existing widget code stays valid.
pub type MonitorId = i64;

/// Workspace identifier. Hyprland uses `i64` so it can encode
/// special workspaces with negative values. Margo's tag IDs are
/// 1..=9 → these map straight through.
pub type WorkspaceId = i64;

/// Process identifier — `i32` per upstream.
pub type ProcessId = i32;

/// Focus-history slot identifier — `i32` per upstream.
pub type FocusHistoryId = i32;

// ── Address (newtype string) ─────────────────────────────────────────────────

/// Window address. Mirrors `wayle_hyprland::Address` exactly —
/// a newtype around `String` with the `0x` prefix stripped on
/// construction so two addresses with / without the prefix
/// compare equal.
#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct Address(String);

impl Address {
    pub fn new(address: String) -> Self {
        let normalized = address.strip_prefix("0x").unwrap_or(&address).to_string();
        Self(normalized)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl AsRef<str> for Address {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<String> for Address {
    fn from(s: String) -> Self {
        Self::new(s)
    }
}

// ── WorkspaceInfo ────────────────────────────────────────────────────────────

/// Snapshot returned by `monitor.active_workspace.get()`.
#[derive(Debug, Clone, PartialEq, PartialOrd)]
pub struct WorkspaceInfo {
    pub id: WorkspaceId,
    pub name: String,
}

// ── Geometry helpers ─────────────────────────────────────────────────────────

/// Window position. Upstream calls this `ClientLocation`; we use
/// a more obvious name and keep the upstream alias.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Position {
    pub x: i32,
    pub y: i32,
}

/// Upstream alias.
pub type ClientLocation = Position;

/// Window dimensions. Upstream calls this `ClientSize`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Dimensions {
    pub width: i32,
    pub height: i32,
}

/// Upstream alias.
pub type ClientSize = Dimensions;

/// Cursor position in global layout coordinates. Upstream type
/// kept verbatim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorPosition {
    pub x: i32,
    pub y: i32,
}

// ── Fullscreen mode ──────────────────────────────────────────────────────────

/// Mirrors `wayle_hyprland::FullscreenMode`. Only the
/// `None` / `Maximize` / `Fullscreen` variants are referenced
/// from the widgets we care about; the rest are kept for
/// upstream-merge cleanliness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FullscreenMode {
    #[default]
    None,
    Maximize,
    Fullscreen,
    MaximizeWithDecorations,
}

// ── Workspace ────────────────────────────────────────────────────────────────

/// A workspace with reactive state. Field set matches
/// `wayle_hyprland::Workspace` so widgets that read
/// `.id` / `.name` / `.monitor` / `.monitor_id` / `.windows`
/// all type-check.
#[derive(Debug, Clone)]
pub struct Workspace {
    pub id: Reactive<WorkspaceId>,
    pub name: Reactive<String>,
    pub monitor: Reactive<String>,
    pub monitor_id: Reactive<Option<MonitorId>>,
    pub windows: Reactive<u16>,
    pub fullscreen: Reactive<bool>,
    pub last_window: Reactive<Option<Address>>,
    pub last_window_title: Reactive<String>,
    pub persistent: Reactive<bool>,
    pub tiled_layout: Reactive<String>,
}

impl PartialEq for Workspace {
    fn eq(&self, other: &Self) -> bool {
        self.id.get() == other.id.get()
    }
}

// ── Monitor ──────────────────────────────────────────────────────────────────

/// An output. Field set matches `wayle_hyprland::Monitor`.
#[derive(Debug, Clone)]
pub struct Monitor {
    pub id: Reactive<MonitorId>,
    pub name: Reactive<String>,
    pub description: Reactive<String>,
    pub make: Reactive<String>,
    pub model: Reactive<String>,
    pub serial: Reactive<String>,
    pub width: Reactive<u32>,
    pub height: Reactive<u32>,
    pub refresh_rate: Reactive<f32>,
    pub x: Reactive<i32>,
    pub y: Reactive<i32>,
    pub active_workspace: Reactive<WorkspaceInfo>,
    pub special_workspace: Reactive<WorkspaceInfo>,
    pub scale: Reactive<f32>,
    pub focused: Reactive<bool>,
    pub dpms_status: Reactive<bool>,
    pub vrr: Reactive<bool>,
}

// ── Client ───────────────────────────────────────────────────────────────────

/// A toplevel window. Field set matches `wayle_hyprland::Client`.
#[derive(Debug, Clone)]
pub struct Client {
    pub address: Reactive<Address>,
    pub mapped: Reactive<bool>,
    pub hidden: Reactive<bool>,
    pub at: Reactive<ClientLocation>,
    pub size: Reactive<ClientSize>,
    pub workspace: Reactive<WorkspaceInfo>,
    pub floating: Reactive<bool>,
    pub monitor: Reactive<MonitorId>,
    pub class: Reactive<String>,
    pub title: Reactive<String>,
    pub initial_class: Reactive<String>,
    pub initial_title: Reactive<String>,
    pub pid: Reactive<ProcessId>,
    pub xwayland: Reactive<bool>,
    pub pinned: Reactive<bool>,
    pub fullscreen: Reactive<FullscreenMode>,
    pub fullscreen_client: Reactive<FullscreenMode>,
    pub over_fullscreen: Reactive<bool>,
    pub grouped: Reactive<Vec<Address>>,
    pub tags: Reactive<Vec<String>>,
    pub swallowing: Reactive<Option<Address>>,
    pub focus_history_id: Reactive<FocusHistoryId>,
    pub inhibiting_idle: Reactive<bool>,
    pub xdg_tag: Reactive<Option<String>>,
    pub xdg_description: Reactive<Option<String>>,
    pub stable_id: Reactive<String>,
}

impl PartialEq for Client {
    fn eq(&self, other: &Self) -> bool {
        self.address.get() == other.address.get()
    }
}

// ── Event stream ─────────────────────────────────────────────────────────────

/// Compositor events. The names mirror Hyprland's event taxonomy
/// (the `*V2` suffix is part of Hyprland's protocol versioning;
/// kept as-is so existing widget `match` arms compile).
#[derive(Debug, Clone)]
pub enum MargoEvent {
    // Workspace lifecycle / focus — all named-field struct variants
    // matching upstream wayle-hyprland 0.2 exactly.
    WorkspaceV2 { id: WorkspaceId, name: String },
    CreateWorkspaceV2 { id: WorkspaceId, name: String },
    DestroyWorkspaceV2 { id: WorkspaceId, name: String },
    MoveWorkspaceV2 { id: WorkspaceId, name: String, monitor: String },
    RenameWorkspace { id: WorkspaceId, name: String },
    ActiveSpecialV2 { id: WorkspaceId, name: String, monitor: String },

    // Client / focus.
    ActiveWindowV2 { address: Address },
    OpenWindow {
        address: Address,
        workspace_name: String,
        class: String,
        title: String,
    },
    CloseWindow { address: Address },
    MoveWindowV2 {
        address: Address,
        workspace_id: WorkspaceId,
        workspace_name: String,
    },

    // Monitor hotplug.
    MonitorAddedV2 { id: MonitorId, name: String, description: String },
    MonitorRemovedV2 { id: MonitorId, name: String },

    /// Catch-all for upstream variants we haven't mirrored.
    #[doc(hidden)]
    Other(String),
}

// ── Service ──────────────────────────────────────────────────────────────────

/// The compositor handle the rest of mshell talks to. Created
/// once at startup via [`MargoService::new`] and stashed in
/// a `OnceLock` over in `mshell-services`.
pub struct MargoService {
    pub workspaces: Reactive<Vec<Arc<Workspace>>>,
    pub clients: Reactive<Vec<Arc<Client>>>,
    pub monitors: Reactive<Vec<Arc<Monitor>>>,
    /// The globally focused client, resolved from `state.json`'s
    /// `focused_idx`. `None` when nothing is focused (empty
    /// desktop). This is the authoritative focus signal — the
    /// per-`Client` `focus_history_id` is per-monitor and matched
    /// by app-id, so it can't identify the single focused window.
    pub focused_client: Reactive<Option<Arc<Client>>>,
    /// Diff-driven typed-event channel for the OkShell widget
    /// pattern (`hyprland.events()` consumers). `sync::apply`
    /// computes the diff between two state.json snapshots and
    /// pushes synthetic Hyprland-shaped events here so the
    /// margo_dock / margo_tags / margo_layout watchers light up
    /// the same way they do on Hyprland (where wayle-hyprland
    /// surfaces real IPC events). Without this channel the
    /// widgets sit on `events.next().await` forever — the bar
    /// renders empty on first paint and only "fills in" when
    /// the user toggles another widget through Settings (which
    /// indirectly forces a fresh subscription). 64-slot buffer
    /// matches the upstream lossy semantics.
    pub(crate) event_tx: tokio::sync::broadcast::Sender<MargoEvent>,
}

impl MargoService {
    /// Connect to the running margo compositor.
    ///
    /// Spawns a background tokio task that polls
    /// `$XDG_RUNTIME_DIR/margo/state.json` every 250 ms, projects
    /// the snapshot onto the service's reactive properties, and
    /// notifies subscribers. The task holds only a `Weak` reference
    /// to the service, so it exits cleanly when the last `Arc<Self>`
    /// drops.
    pub async fn new() -> Result<Arc<Self>> {
        let (event_tx, _) = tokio::sync::broadcast::channel(64);
        let service = Arc::new(Self {
            workspaces: Reactive::new(Vec::new()),
            clients: Reactive::new(Vec::new()),
            monitors: Reactive::new(Vec::new()),
            focused_client: Reactive::new(None),
            event_tx,
        });
        // Run one synchronous read so widgets see populated state
        // on the very first paint, not on the next poll tick.
        if let Some(snapshot) = state_json::read() {
            sync::apply_snapshot(&service, &snapshot);
            tracing::info!(
                outputs = snapshot.outputs.len(),
                clients = snapshot.clients.len(),
                "mshell-margo-client: initial state.json snapshot loaded"
            );
        } else {
            tracing::warn!("mshell-margo-client: state.json not readable on startup; bar will fill on the first poll tick");
        }
        sync::spawn(&service);
        Ok(service)
    }

    /// Run a compositor command. The upstream API takes raw
    /// Hyprland command strings; we translate the well-known
    /// patterns to `mctl dispatch` invocations and log everything
    /// else as a non-fatal warning.
    ///
    /// Currently handled patterns:
    ///   * `"workspace N"`        → view tag N (1..=9)
    ///   * `"workspace r-1/r+1"`  → tagtoleft / tagtoright
    ///   * `"hl.dsp.focus({ workspace = \"r-1\" })"` (the form
    ///                              mshell-utils emits)
    ///   * Any string starting with `"dispatch "` is shipped to
    ///     `mctl dispatch` verbatim.
    pub async fn dispatch(&self, cmd: &str) -> Result<()> {
        let trimmed = cmd.trim();
        tracing::debug!(cmd = %trimmed, "mshell-margo-client: dispatch");

        // Common upstream shapes → mctl translation.
        let mctl_args: Option<Vec<String>> = if let Some(rest) = trimmed.strip_prefix("workspace ")
        {
            Some(translate_workspace(rest.trim()))
        } else if trimmed.contains("hl.dsp.focus") && trimmed.contains("workspace") {
            // Pattern: `hl.dsp.focus({ workspace = "r-1" })` or
            // `… workspace = "5" …`. Extract the quoted token.
            extract_quoted(trimmed).map(|t| translate_workspace(&t))
        } else if let Some(rest) = trimmed.strip_prefix("dispatch ") {
            Some(rest.split_whitespace().map(String::from).collect())
        } else {
            None
        };

        let Some(args) = mctl_args else {
            tracing::warn!(cmd = %trimmed, "dispatch: unrecognised command, ignoring");
            return Ok(());
        };
        if args.is_empty() {
            return Ok(());
        }

        // Spawn `mctl dispatch …`. Non-blocking, error logged.
        let mut command = tokio::process::Command::new("mctl");
        command.arg("dispatch");
        for a in &args {
            command.arg(a);
        }
        match command.status().await {
            Ok(status) if status.success() => Ok(()),
            Ok(status) => {
                tracing::warn!(?status, args = ?args, "mctl dispatch returned non-zero");
                Ok(())
            }
            Err(e) => {
                tracing::warn!(error = %e, args = ?args, "mctl dispatch spawn failed");
                Ok(())
            }
        }
    }

    /// Run a query against the compositor. Used by the layout
    /// indicator widget. Returns the currently-focused output's
    /// layout name when the query mentions "layout"; otherwise an
    /// empty string.
    pub async fn eval(&self, query: &str) -> Result<String> {
        tracing::debug!(query = %query, "mshell-margo-client: eval");
        if query.contains("layout") {
            if let Some(state) = state_json::read()
                && let Some(out) = state.outputs.iter().find(|o| o.active)
                && let Some(name) = state.layouts.get(out.layout_idx)
            {
                return Ok(name.clone());
            }
        }
        Ok(String::new())
    }

    /// Snapshot the focused workspace. Reads state.json directly
    /// so the answer always reflects the latest margo write, not
    /// the most recent poll tick (which can lag by up to
    /// `POLL_INTERVAL`).
    ///
    /// Resolves the active monitor via [`active_monitor_name`]
    /// (which prefers the focused client's monitor over the
    /// top-level `active_output` field) and then looks up the
    /// workspace whose `active_tag_mask` is currently set on that
    /// output.
    pub async fn active_workspace(&self) -> Option<Arc<Workspace>> {
        let name = self.active_monitor_name().await?;
        let ws_id = {
            let state = state_json::read()?;
            let out = state.outputs.iter().find(|o| o.name == name)?;
            state_json::lowest_tag(out.active_tag_mask) as i64
        };
        self.workspaces
            .get()
            .into_iter()
            .find(|w| w.id.get() == ws_id && w.monitor.get() == name)
    }

    /// Best-guess "where is the user looking right now" monitor
    /// connector name. Used by the IPC layer to decide which
    /// per-monitor Frame should host a newly-toggled menu.
    ///
    /// Resolution order:
    ///   1. The **focused client**'s monitor — strongest signal
    ///      for "where the user is interacting"; updates the
    ///      instant the user clicks / alt-tabs anywhere on a real
    ///      window. Bypasses the `active_output` lag we've seen
    ///      right after reboot, where margo's top-level field can
    ///      stay pinned to whichever output enumerated first
    ///      (typically `eDP-1`) until the first manual interaction
    ///      explicitly switches it.
    ///   2. `state.active_output` — margo's own active-output
    ///      notion, used when no client is focused (empty
    ///      desktop) or state.json hasn't propagated yet.
    pub async fn active_monitor_name(&self) -> Option<String> {
        let state = state_json::read()?;
        if let Some(c) = state.clients.iter().find(|c| c.focused) {
            return Some(c.monitor.clone());
        }
        Some(state.active_output)
    }

    /// Snapshot the focused client.
    pub async fn active_window(&self) -> Option<Arc<Client>> {
        let address = {
            let state = state_json::read()?;
            let c = state.clients.iter().find(|c| c.focused)?;
            // Mirror sync::client_address — keep both formulas
            // in lockstep (sync.rs comment marks the source-of-truth).
            Address::new(format!(
                "{:04x}{:08x}",
                c.monitor_idx as u16, c.idx as u32
            ))
        };
        self.clients
            .get()
            .into_iter()
            .find(|c| c.address.get() == address)
    }

    /// Event stream — subscribes to the diff-driven typed events
    /// `sync::apply` emits whenever state.json change produces a
    /// workspace add/remove/move, a client open/close/focus, or a
    /// monitor hotplug. This is the channel OkShell-style widgets
    /// (`spawn_main_watcher` → `let mut events = hyprland.events();`
    /// → `match event { MargoEvent::WorkspaceV2 => … }`) expect.
    /// Lossy: a slow consumer that misses 64+ events gets `Lagged`,
    /// which we silently drop and resume from the next live event —
    /// matches the upstream `wayle_hyprland::events` semantics.
    pub fn events(&self) -> Pin<Box<dyn Stream<Item = MargoEvent> + Send>> {
        let rx = self.event_tx.subscribe();
        Box::pin(
            tokio_stream::wrappers::BroadcastStream::new(rx)
                .filter_map(|r| async move { r.ok() }),
        )
    }
}

/// Translate the workspace argument from a Hyprland dispatch
/// string (`"5"`, `"r-1"`, `"r+1"`) to a mctl dispatch action +
/// args. Returns the args as a string vec so the caller can
/// forward them to `mctl dispatch …`.
fn translate_workspace(arg: &str) -> Vec<String> {
    let arg = arg.trim();
    if arg == "r-1" || arg == "e-1" {
        vec!["viewtoleft".to_string()]
    } else if arg == "r+1" || arg == "e+1" {
        vec!["viewtoright".to_string()]
    } else if let Ok(n) = arg.parse::<u32>() {
        if (1..=9).contains(&n) {
            // mctl dispatch view <bitmask>
            let mask = 1u32 << (n - 1);
            vec!["view".to_string(), mask.to_string()]
        } else {
            tracing::warn!(arg, "workspace number out of margo's 1..=9 range");
            vec![]
        }
    } else {
        tracing::warn!(arg, "workspace arg not recognised");
        vec![]
    }
}

/// Pull the first double-quoted token out of a string.
fn extract_quoted(s: &str) -> Option<String> {
    let start = s.find('"')? + 1;
    let rest = &s[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

