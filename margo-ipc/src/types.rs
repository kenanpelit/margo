use serde::{Deserialize, Serialize};

// ── Tag state ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TagState {
    None = 0,
    Active = 1,
    Urgent = 2,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagInfo {
    pub output: String,
    pub tag: u32,
    pub state: TagState,
    pub clients: u32,
    pub focused: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagMaskInfo {
    pub output: String,
    pub occupied: u32,
    pub selected: u32,
    pub urgent: u32,
}

// ── Layout ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutInfo {
    pub output: String,
    pub symbol: String,
    pub name: String,
}

// ── Client / focus info ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientInfo {
    pub output: String,
    pub appid: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientGeometry {
    pub output: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

// ── Monitor / output info ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputInfo {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub scale: f32,
}

// ── IPC requests (sent from client to compositor) ────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IpcRequest {
    /// Dispatch a command with up to 5 args (mirrors mmsg -d)
    Dispatch {
        output: Option<String>,
        command: String,
        args: Vec<String>,
    },
    /// Set tags on an output
    SetTags {
        output: Option<String>,
        tagmask: u32,
        toggle: bool,
    },
    /// Set layout on an output
    SetLayout {
        output: Option<String>,
        layout: String,
    },
    /// Set client tags
    SetClientTags {
        output: Option<String>,
        and_tags: u32,
        xor_tags: u32,
    },
    /// Quit the compositor
    Quit,
    /// One-shot get
    Get(GetQuery),
    /// Subscribe for events
    Watch(WatchQuery),
}

// ── IPC get/watch flags ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetQuery {
    pub output: Option<String>,
    pub tags: bool,
    pub layout: bool,
    pub client: bool,
    pub fullscreen: bool,
    pub floating: bool,
    pub statusbar: bool,
    pub geometry: bool,
    pub last_layer: bool,
    pub keyboard_layout: bool,
    pub keybind_mode: bool,
    pub scale: bool,
    pub all_outputs: bool,
    pub tag_count: bool,
    pub all_layouts: bool,
}

pub type WatchQuery = GetQuery;

// ── IPC events (compositor → client) ─────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IpcEvent {
    Tags(TagMaskInfo),
    Layout(LayoutInfo),
    Client(ClientInfo),
    Fullscreen { output: String, active: bool },
    Floating { output: String, active: bool },
    StatusbarVisible { output: String, visible: bool },
    Geometry(ClientGeometry),
    LastLayer { output: String, name: String },
    KeyboardLayout { output: String, name: String },
    KeybindMode { output: String, mode: String },
    Scale { output: String, scale: f32 },
    Outputs(Vec<OutputInfo>),
    TagCount { count: u32 },
    AllLayouts(Vec<String>),
}

// ── Error type ───────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum IpcError {
    #[error("wayland connection failed: {0}")]
    Connection(String),
    #[error("no compositor found (WAYLAND_DISPLAY not set?)")]
    NoCompositor,
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialisation: {0}")]
    Json(#[from] serde_json::Error),
}
