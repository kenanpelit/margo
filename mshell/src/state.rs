//! Tiny shared reader for margo's `state.json` snapshot.
//!
//! margo rewrites `$XDG_RUNTIME_DIR/margo/state.json` atomically on
//! every focus / tag / wallpaper / twilight change. Each module that
//! wants compositor state polls this module instead of opening the
//! file itself, which keeps the parsing and the path-resolution in
//! one place.
//!
//! Stage-2 implementation: every poll re-reads the file and returns
//! a `Compositor` snapshot. Stage 9 will replace the polling with an
//! inotify watcher hooked into the glib main loop, but the public
//! API (`Compositor::current()`) stays the same so modules don't
//! need rewriting.

use serde_json::Value;
use std::path::PathBuf;

/// Lightweight projection of margo's `state.json`. We only carry the
/// fields the bar actually renders; the rest of the JSON is ignored.
#[derive(Debug, Clone, Default)]
pub struct Compositor {
    /// `state.json:active_output` — focused output's bare name.
    pub active_output: Option<String>,
    /// Outputs keyed by bare name (`"DP-3"`).
    pub outputs: Vec<Output>,
    /// Currently-focused client (or `None` when nothing is focused —
    /// e.g. focus is on a layer surface like an open mshell menu).
    pub focused_client: Option<Client>,
    /// Per-tag window count summed across every output. Tag index is
    /// 1-based; index 0 of the vec corresponds to tag 1.
    pub tag_window_counts: [u16; 9],
}

#[derive(Debug, Clone, Default)]
pub struct Output {
    pub name: String,
    /// True if this is the focused output. Currently only carried
    /// for diagnostics — the workspaces module uses `active_output`
    /// from the top level to decide which mask to highlight.
    #[allow(dead_code)]
    pub active: bool,
    /// Bitmask of currently-shown tags on this output (bit 0 = tag 1).
    pub active_tag_mask: u32,
}

#[derive(Debug, Clone, Default)]
pub struct Client {
    pub title: String,
    /// Reserved for the WindowTitle module's "class instead of title"
    /// config knob that's coming back in a later polish patch.
    #[allow(dead_code)]
    pub app_id: String,
}

impl Compositor {
    /// Read + parse the current state.json. Returns `Compositor::default()`
    /// (empty snapshot) on any I/O or parse error so caller widgets just
    /// render a blank state instead of panicking on startup races.
    pub fn current() -> Self {
        let path = state_path();
        let Ok(bytes) = std::fs::read(&path) else {
            return Self::default();
        };
        let Ok(json) = serde_json::from_slice::<Value>(&bytes) else {
            return Self::default();
        };
        parse(&json)
    }

    /// True if `tag` (1-based) is shown on the focused output.
    pub fn tag_active_on_focused(&self, tag: u8) -> bool {
        let Some(active) = self.active_output.as_deref() else {
            return false;
        };
        let Some(o) = self.outputs.iter().find(|o| o.name == active) else {
            return false;
        };
        tag >= 1 && tag <= 9 && (o.active_tag_mask & (1u32 << (tag - 1))) != 0
    }

    /// Window count for `tag` (1-based) — sum across every output, the
    /// "occupied dot" widgets read from this.
    pub fn tag_windows(&self, tag: u8) -> u16 {
        if tag == 0 || tag > 9 {
            return 0;
        }
        self.tag_window_counts[(tag - 1) as usize]
    }
}

fn state_path() -> PathBuf {
    if let Some(rt) = std::env::var_os("XDG_RUNTIME_DIR") {
        return PathBuf::from(rt).join("margo").join("state.json");
    }
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/run/user/{uid}/margo/state.json"))
}

fn parse(json: &Value) -> Compositor {
    let mut state = Compositor {
        active_output: json
            .get("active_output")
            .and_then(Value::as_str)
            .map(str::to_string),
        ..Default::default()
    };

    if let Some(outputs) = json.get("outputs").and_then(Value::as_array) {
        for o in outputs {
            let name = o
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let active = o.get("active").and_then(Value::as_bool).unwrap_or(false);
            let active_tag_mask = o
                .get("active_tag_mask")
                .and_then(Value::as_u64)
                .unwrap_or(0) as u32;
            state.outputs.push(Output {
                name,
                active,
                active_tag_mask,
            });
        }
    }

    if let Some(clients) = json.get("clients").and_then(Value::as_array) {
        // Tag-window histogram across every client. `tags` is a
        // bitmask so a single client with multiple tags counts in
        // each of them — matches margo's own counting.
        for c in clients {
            let tags = c.get("tags").and_then(Value::as_u64).unwrap_or(0) as u32;
            for tag in 1..=9u32 {
                if tags & (1 << (tag - 1)) != 0 {
                    state.tag_window_counts[(tag - 1) as usize] =
                        state.tag_window_counts[(tag - 1) as usize].saturating_add(1);
                }
            }
            if c.get("focused").and_then(Value::as_bool).unwrap_or(false) {
                state.focused_client = Some(Client {
                    title: c
                        .get("title")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    app_id: c
                        .get("app_id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                });
            }
        }
    }

    state
}
