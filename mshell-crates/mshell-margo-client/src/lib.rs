//! Margo compositor client for MShell — replaces `wayle-hyprland`.
//!
//! The OkShell tree was written against `wayle-hyprland 0.2`, which
//! exposes a reactive view of Hyprland's IPC: a [`HyprlandService`]
//! handle with `workspaces` / `clients` / `monitors` properties
//! (each a reactive container with `.get()` snapshot + `.watch()`
//! stream), a `dispatch()` method that ships raw Hyprland command
//! strings, an `eval()` method for queries, and an `events()` async
//! stream of typed variants. The four `hyprland_*.rs` bar widgets
//! and two helper modules in `mshell-utils` / `mshell-services`
//! consume that surface.
//!
//! This crate mirrors the upstream API **field-for-field**, with the
//! same type names (`HyprlandService`, `HyprlandEvent`, `Workspace`,
//! `WorkspaceInfo`, `WorkspaceId`, `Client`, `Address`, `MonitorId`)
//! and the same field layout, so each widget compiles after a single
//! `use wayle_hyprland::*` → `use mshell_margo_client::*` edit. The
//! backend is intentionally stubbed in this Phase 2b commit:
//!
//!   * [`HyprlandService::new`] returns a service with empty
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
pub enum HyprlandEvent {
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

/// Forward-looking alias. Identical to [`HyprlandEvent`] for the
/// duration of the upstream-shaped naming; new mshell code should
/// prefer `MargoEvent` so a future rename is mechanical.
pub type MargoEvent = HyprlandEvent;

// ── Service ──────────────────────────────────────────────────────────────────

/// The compositor handle the rest of mshell talks to. Created
/// once at startup via [`HyprlandService::new`] and stashed in
/// a `OnceLock` over in `mshell-services`.
pub struct HyprlandService {
    pub workspaces: Reactive<Vec<Arc<Workspace>>>,
    pub clients: Reactive<Vec<Arc<Client>>>,
    pub monitors: Reactive<Vec<Arc<Monitor>>>,
}

impl HyprlandService {
    /// Connect to the running margo compositor.
    ///
    /// **Phase 2b stub**: returns a handle with empty reactive
    /// properties. Bar widgets render their empty state — no
    /// crash. Phase 2c wires this up against margo's `dwl-ipc-v2`
    /// + `foreign-toplevel-list` + `state.json`.
    pub async fn new() -> Result<Arc<Self>> {
        tracing::info!("mshell-margo-client: stub service (phase 2b — no backend)");
        Ok(Arc::new(Self {
            workspaces: Reactive::new(Vec::new()),
            clients: Reactive::new(Vec::new()),
            monitors: Reactive::new(Vec::new()),
        }))
    }

    pub async fn dispatch(&self, cmd: &str) -> Result<()> {
        tracing::debug!(cmd = %cmd, "mshell-margo-client: dispatch stub");
        Ok(())
    }

    pub async fn eval(&self, query: &str) -> Result<String> {
        tracing::debug!(query = %query, "mshell-margo-client: eval stub");
        Ok(String::new())
    }

    pub async fn active_workspace(&self) -> Option<Arc<Workspace>> {
        None
    }

    pub async fn active_window(&self) -> Option<Arc<Client>> {
        None
    }

    pub fn events(&self) -> Pin<Box<dyn Stream<Item = HyprlandEvent> + Send>> {
        Box::pin(futures::stream::empty())
    }
}

/// Forward-looking alias. See [`MargoEvent`].
pub type MargoService = HyprlandService;
