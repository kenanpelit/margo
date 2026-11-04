//! Notes Hub bar pill — port of the noctalia `notes` plugin's
//! bar half.
//!
//! Render-only widget. Follows the shared notes store (scratch-
//! pad + notes + todos) and re-reads the file every 30 s to pick up
//! hand edits, draws an icon + tooltip with counts. Click emits `NotesOutput::Clicked`; frame toggles
//! `MenuType::Notes`.
//!
//! Persistence lives in `$XDG_DATA_HOME/mshell/notes.json` (default
//! `~/.local/share/mshell/notes.json`), behind the process-wide
//! [`notes_store`] the menu widget shares.

use mshell_config::doc_store::DocStore;
use relm4::gtk::prelude::{ButtonExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

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

/// The one notes document every bar pill and Notes menu (one per monitor)
/// shares. Edits go through [`DocStore::update`], so a menu on another
/// monitor sees them at once and never saves an older copy over them; a
/// notes.json that fails to parse is kept (plus a `.bak`) and not saved over.
pub(crate) fn notes_store() -> &'static Arc<DocStore<NotesState>> {
    static STORE: LazyLock<Arc<DocStore<NotesState>>> =
        LazyLock::new(|| DocStore::new("Notes", notes_path()));
    &STORE
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
pub(crate) struct NotesModel {
    state: NotesState,
}

#[derive(Debug)]
pub(crate) enum NotesInput {
    Clicked,
}

#[derive(Debug)]
pub(crate) enum NotesOutput {
    Clicked,
}

pub(crate) struct NotesInit {}

#[derive(Debug)]
pub(crate) enum NotesCommandOutput {
    Refreshed(NotesState),
}

#[relm4::component(pub)]
impl Component for NotesModel {
    type CommandOutput = NotesCommandOutput;
    type Input = NotesInput;
    type Output = NotesOutput;
    type Init = NotesInit;

    view! {
        #[root]
        gtk::Box {
            set_css_classes: &["ok-button-surface", "ok-bar-widget", "notes-bar-widget"],
            set_hexpand: false,
            set_vexpand: false,
            set_has_tooltip: true,

            #[name="button"]
            gtk::Button {
                set_css_classes: &["ok-button-flat"],
                set_hexpand: true,
                set_vexpand: true,
                connect_clicked[sender] => move |_| {
                    sender.input(NotesInput::Clicked);
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
        sender.command(|out, shutdown| async move {
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            let mut changes = notes_store().subscribe();
            let mut first = true;
            loop {
                let delay = if first {
                    STARTUP_DELAY
                } else {
                    REFRESH_INTERVAL
                };
                first = false;
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    // An edit from any Notes menu: repaint right away.
                    changed = changes.changed() => {
                        if changed.is_err() {
                            break;
                        }
                    }
                    _ = tokio::time::sleep(delay) => {
                        // Pick up hand edits of notes.json.
                        let _ = tokio::task::spawn_blocking(|| notes_store().reload()).await;
                    }
                }
                let s = (*changes.borrow_and_update().value).clone();
                let _ = out.send(NotesCommandOutput::Refreshed(s));
            }
        });

        let model = NotesModel {
            state: NotesState::default(),
        };
        let widgets = view_output!();
        apply_visual(&widgets.image, &root, &model.state);
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            NotesInput::Clicked => {
                let _ = sender.output(NotesOutput::Clicked);
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
            NotesCommandOutput::Refreshed(state) => {
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
