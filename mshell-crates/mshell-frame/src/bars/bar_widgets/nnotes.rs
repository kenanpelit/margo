//! Notes Hub bar pill — port of the noctalia `notes` plugin's
//! bar half.
//!
//! Render-only widget. Reads the persisted notes state (scratch-
//! pad + notes + todos) every 30 s, draws an icon + tooltip with
//! counts. Click emits `NnotesOutput::Clicked`; frame toggles
//! `MenuType::Nnotes`.
//!
//! Persistence lives in `$XDG_DATA_HOME/mshell/notes.json` (default
//! `~/.local/share/mshell/notes.json`). The store is exposed
//! `pub(crate)` so the menu widget can share the same path.

use relm4::gtk::prelude::{ButtonExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;
use tracing::warn;

const REFRESH_INTERVAL: Duration = Duration::from_secs(30);
const STARTUP_DELAY: Duration = Duration::from_millis(500);

/// Persistent state for the Notes Hub plugin. Mirrors the
/// upstream QML plugin's `pluginSettings` shape — scratchpad
/// text + arrays of notes / todos — but we own the on-disk
/// representation (a single JSON document at
/// `$XDG_DATA_HOME/mshell/notes.json`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct NotesState {
    #[serde(default)]
    pub(crate) scratchpad: String,
    #[serde(default)]
    pub(crate) notes: Vec<Note>,
    #[serde(default)]
    pub(crate) todos: Vec<Todo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub(crate) struct Note {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub(crate) struct Todo {
    pub(crate) id: String,
    pub(crate) text: String,
    #[serde(default)]
    pub(crate) completed: bool,
}

/// Canonical on-disk path: `$XDG_DATA_HOME/mshell/notes.json`,
/// falling back to `$HOME/.local/share/mshell/notes.json`.
pub(crate) fn notes_path() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join("mshell").join("notes.json")
}

/// Load + parse the notes JSON. Missing / unreadable / malformed
/// → empty state (so a first-launch user gets a clean slate
/// instead of an error banner).
pub(crate) async fn load_notes() -> NotesState {
    let path = notes_path();
    match tokio::fs::read_to_string(&path).await {
        Ok(raw) => serde_json::from_str(&raw).unwrap_or_else(|e| {
            warn!(path = %path.display(), error = %e, "notes JSON parse failed; using empty state");
            NotesState::default()
        }),
        Err(_) => NotesState::default(),
    }
}

/// Atomic save — write to a sibling `.tmp` file and rename. The
/// rename is atomic on common filesystems, so a crash mid-write
/// can never leave the user with a half-written notes.json.
pub(crate) async fn save_notes(state: &NotesState) -> Result<(), String> {
    let path = notes_path();
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("create dir {}: {e}", parent.display()))?;
    }
    let raw = serde_json::to_string_pretty(state).map_err(|e| format!("serialize: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    tokio::fs::write(&tmp, raw)
        .await
        .map_err(|e| format!("write tmp: {e}"))?;
    tokio::fs::rename(&tmp, &path)
        .await
        .map_err(|e| format!("rename: {e}"))?;
    Ok(())
}

/// Fresh time-based id. JavaScript's `Date.now()` equivalent —
/// good enough for "two notes added in the same millisecond
/// would collide", which doesn't happen with human-driven flow.
pub(crate) fn new_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| format!("{}", d.as_millis()))
        .unwrap_or_else(|_| "0".to_string())
}

#[derive(Debug)]
pub(crate) struct NnotesModel {
    state: NotesState,
}

#[derive(Debug)]
pub(crate) enum NnotesInput {
    Clicked,
}

#[derive(Debug)]
pub(crate) enum NnotesOutput {
    Clicked,
}

pub(crate) struct NnotesInit {}

#[derive(Debug)]
pub(crate) enum NnotesCommandOutput {
    Refreshed(NotesState),
}

#[relm4::component(pub)]
impl Component for NnotesModel {
    type CommandOutput = NnotesCommandOutput;
    type Input = NnotesInput;
    type Output = NnotesOutput;
    type Init = NnotesInit;

    view! {
        #[root]
        gtk::Box {
            set_css_classes: &["ok-button-surface", "ok-bar-widget", "nnotes-bar-widget"],
            set_hexpand: false,
            set_vexpand: false,
            set_has_tooltip: true,

            #[name="button"]
            gtk::Button {
                set_css_classes: &["ok-button-flat"],
                set_hexpand: true,
                set_vexpand: true,
                connect_clicked[sender] => move |_| {
                    sender.input(NnotesInput::Clicked);
                },

                #[name="image"]
                gtk::Image {
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,
                }
            }
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        sender.command(|out, shutdown| {
            async move {
                let shutdown_fut = shutdown.wait();
                tokio::pin!(shutdown_fut);
                let mut first = true;
                loop {
                    let delay = if first { STARTUP_DELAY } else { REFRESH_INTERVAL };
                    first = false;
                    tokio::select! {
                        () = &mut shutdown_fut => break,
                        _ = tokio::time::sleep(delay) => {}
                    }
                    let s = load_notes().await;
                    let _ = out.send(NnotesCommandOutput::Refreshed(s));
                }
            }
        });

        let model = NnotesModel {
            state: NotesState::default(),
        };
        let widgets = view_output!();
        apply_visual(&widgets.image, &root, &model.state);
        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            NnotesInput::Clicked => {
                let _ = sender.output(NnotesOutput::Clicked);
            }
        }
    }

    fn update_cmd_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::CommandOutput,
        _sender: ComponentSender<Self>,
        root: &Self::Root,
    ) {
        match message {
            NnotesCommandOutput::Refreshed(state) => {
                if self.state != state {
                    self.state = state;
                    apply_visual(&widgets.image, root, &self.state);
                }
            }
        }
    }
}

fn apply_visual(image: &gtk::Image, root: &gtk::Box, s: &NotesState) {
    image.set_icon_name(Some("notes-symbolic"));

    let open_todos = s.todos.iter().filter(|t| !t.completed).count();
    let mut lines = Vec::with_capacity(3);
    lines.push("Notes Hub".to_string());
    if s.scratchpad.trim().is_empty() {
        lines.push("Scratchpad: (empty)".to_string());
    } else {
        lines.push(format!("Scratchpad: {} chars", s.scratchpad.trim().len()));
    }
    lines.push(format!("Notes: {}", s.notes.len()));
    lines.push(format!(
        "Todos: {} open / {} total",
        open_todos,
        s.todos.len()
    ));
    root.set_tooltip_text(Some(&lines.join("\n")));

    root.remove_css_class("has-pending");
    if open_todos > 0 {
        root.add_css_class("has-pending");
    }
}
