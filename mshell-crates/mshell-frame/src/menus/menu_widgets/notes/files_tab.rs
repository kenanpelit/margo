//! The Notes Hub "Files" tab: Markdown notes in a folder
//! (Settings → Widgets → Notes Hub → Notes folder), with search, a
//! quick capture to `Inbox.md`, an inline editor (autosave), optional git
//! history, and AI actions through the shared `mshell-ai` provider:
//! summarise / rewrite the open note, tidy a capture, and ask questions
//! over the notes.
//!
//! All file, git and AI work runs off the GTK thread; the tab rescans the
//! folder each time it is shown and after every write.

use super::files_model::{
    self, INBOX, NoteFile, append_to_inbox, ask_prompt, capture_line, capture_prompt, contained,
    expand_tilde, matches_query, new_note_name, note_title, pick_context, rewrite_prompt,
    summarize_prompt,
};
use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::ConfigStoreFields;
use mshell_config::toast::{ToastEvent, ToastSeverity, push_toast};
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::glib;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

const SAVE_DEBOUNCE: Duration = Duration::from_millis(800);
/// Git commits wait for a quiet spell so a typing burst is one commit.
const GIT_DEBOUNCE: Duration = Duration::from_secs(8);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AiKind {
    Ask,
    Summarize,
    Rewrite,
}

impl AiKind {
    fn busy_label(self) -> &'static str {
        match self {
            Self::Ask => "Reading your notes…",
            Self::Summarize => "Summarising…",
            Self::Rewrite => "Rewriting…",
        }
    }
}

struct AiState {
    generation: u64,
    kind: AiKind,
    text: String,
    running: bool,
    error: Option<String>,
    /// Note titles a question was answered from.
    sources: Vec<String>,
    cancel: Arc<AtomicBool>,
}

struct OpenNote {
    rel: String,
    path: PathBuf,
}

pub(crate) struct FilesTabModel {
    folder: Option<PathBuf>,
    git_history: bool,
    ai_capture: bool,
    context_notes: usize,
    notes: Vec<NoteFile>,
    scan_error: Option<String>,
    scanned: bool,
    query: String,
    open: Option<OpenNote>,
    ai: AiState,
    // Widgets updated imperatively.
    pages: gtk::Stack,
    hint: gtk::Label,
    list: gtk::ListBox,
    pub(crate) scroller: gtk::ScrolledWindow,
    editor: gtk::TextView,
    editor_quiet: Rc<Cell<bool>>,
    editor_title: gtk::Label,
    rewrite_entry: gtk::Entry,
    ai_box: gtk::Box,
    ai_status: gtk::Label,
    ai_text: gtk::Label,
    ai_replace: gtk::Button,
    ai_append: gtk::Button,
    ai_stop: gtk::Button,
    save_token: Rc<Cell<u64>>,
    git_token: Rc<Cell<u64>>,
    _effects: EffectScope,
}

impl std::fmt::Debug for FilesTabModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FilesTabModel")
            .field("folder", &self.folder)
            .field("notes", &self.notes.len())
            .finish()
    }
}

#[derive(Debug)]
pub(crate) enum FilesTabInput {
    /// The tab became visible: rescan.
    Shown,
    ConfigChanged,
    Rescan,
    Query(String),
    OpenNote(String),
    Back,
    /// The editor text changed (debounced save).
    EditorChanged,
    SaveNow(u64),
    /// Create a note titled from the search text and open it.
    NewNote,
    OpenExternally,
    Capture(String),
    Ask(String),
    Summarize,
    Rewrite,
    AiStop,
    AiDismiss,
    AiCopy,
    /// Replace the open note with the rewrite.
    AiReplace,
    /// Append the summary to the open note.
    AiAppend,
    GitNow(u64),
    OpenSettings,
}

#[derive(Debug)]
pub(crate) enum FilesTabOutput {}

pub(crate) struct FilesTabInit {}

#[derive(Debug)]
pub(crate) enum FilesTabCommandOutput {
    Scanned(Result<Vec<NoteFile>, String>),
    Loaded {
        rel: String,
        res: Result<String, String>,
    },
    Saved(Result<(), String>),
    Created(Result<String, String>),
    Captured(Result<String, String>),
    AiDelta {
        generation: u64,
        text: String,
    },
    AiDone {
        generation: u64,
        res: Result<(), String>,
    },
    GitDone(Result<Option<String>, String>),
}

fn label(text: &str, classes: &[&str]) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_css_classes(classes);
    l.set_xalign(0.0);
    l.set_halign(gtk::Align::Start);
    l.set_wrap(true);
    l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    l
}

fn icon_button(icon: &str, name: &str) -> gtk::Button {
    let b = gtk::Button::from_icon_name(icon);
    b.add_css_class("ok-button-flat");
    b.add_css_class("notes-tool-btn");
    mshell_common::a11y::name(&b, name);
    b
}

fn text_button(text: &str) -> gtk::Button {
    let b = gtk::Button::with_label(text);
    b.set_css_classes(&["ok-button-surface", "ok-button-cell"]);
    b
}

fn entry(placeholder: &str) -> gtk::Entry {
    let e = gtk::Entry::new();
    e.add_css_class("notes-add");
    e.set_placeholder_text(Some(placeholder));
    e.set_hexpand(true);
    e
}

fn resolved_folder(raw: &str) -> Option<PathBuf> {
    let raw = raw.trim();
    (!raw.is_empty()).then(|| expand_tilde(raw, margo_fs::xdg::home().as_deref()))
}

#[relm4::component(pub(crate))]
impl Component for FilesTabModel {
    type CommandOutput = FilesTabCommandOutput;
    type Input = FilesTabInput;
    type Output = FilesTabOutput;
    type Init = FilesTabInit;

    view! {
        #[root]
        gtk::Box {
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 8,
            add_css_class: "notes-files",
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let pages = gtk::Stack::new();
        pages.set_transition_type(gtk::StackTransitionType::Crossfade);
        pages.set_transition_duration(120);
        pages.set_vhomogeneous(false);
        pages.set_interpolate_size(true);

        // ── List page ───────────────────────────────────────────────
        let list_page = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let capture = entry("Capture a todo to Inbox.md — Enter");
        capture.update_property(&[gtk::accessible::Property::Label("Capture to inbox")]);
        let s = sender.clone();
        capture.connect_activate(move |e| {
            let t = e.text().to_string();
            if !t.trim().is_empty() {
                e.set_text("");
                s.input(FilesTabInput::Capture(t));
            }
        });
        list_page.append(&capture);

        let search_row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        let search = gtk::SearchEntry::new();
        search.add_css_class("menu-search");
        search.set_hexpand(true);
        search.set_placeholder_text(Some("Search titles and text"));
        let s = sender.clone();
        search.connect_search_changed(move |e| s.input(FilesTabInput::Query(e.text().to_string())));
        search_row.append(&search);
        let new_btn = icon_button(
            "list-add-symbolic",
            "New note (titled from the search text)",
        );
        let s = sender.clone();
        new_btn.connect_clicked(move |_| s.input(FilesTabInput::NewNote));
        search_row.append(&new_btn);
        let rescan = icon_button("view-refresh-symbolic", "Rescan the folder");
        let s = sender.clone();
        rescan.connect_clicked(move |_| s.input(FilesTabInput::Rescan));
        search_row.append(&rescan);
        list_page.append(&search_row);

        let hint = label("", &["notes-empty"]);
        list_page.append(&hint);
        let list = gtk::ListBox::new();
        list.add_css_class("notes-list");
        list.set_selection_mode(gtk::SelectionMode::None);
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .min_content_height(0)
            .child(&list)
            .build();
        list_page.append(&scroller);

        let ask = entry("Ask your notes… — Enter");
        ask.set_primary_icon_name(Some("dialog-question-symbolic"));
        ask.update_property(&[gtk::accessible::Property::Label("Ask your notes")]);
        let s = sender.clone();
        ask.connect_activate(move |e| {
            let t = e.text().to_string();
            if !t.trim().is_empty() {
                s.input(FilesTabInput::Ask(t));
            }
        });
        list_page.append(&ask);
        pages.add_named(&list_page, Some("list"));

        // ── Editor page ─────────────────────────────────────────────
        let editor_page = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        let back = icon_button("go-previous-symbolic", "Back to the note list");
        let s = sender.clone();
        back.connect_clicked(move |_| s.input(FilesTabInput::Back));
        bar.append(&back);
        let editor_title = gtk::Label::new(None);
        editor_title.add_css_class("notes-card-title");
        editor_title.set_hexpand(true);
        editor_title.set_xalign(0.0);
        editor_title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        bar.append(&editor_title);
        let external = icon_button("document-open-symbolic", "Open in the default editor");
        let s = sender.clone();
        external.connect_clicked(move |_| s.input(FilesTabInput::OpenExternally));
        bar.append(&external);
        editor_page.append(&bar);

        let editor = gtk::TextView::new();
        editor.add_css_class("notes-scratchpad");
        editor.set_wrap_mode(gtk::WrapMode::WordChar);
        editor.set_left_margin(8);
        editor.set_right_margin(8);
        editor.set_top_margin(6);
        editor.set_bottom_margin(6);
        editor.update_property(&[gtk::accessible::Property::Label("Note text")]);
        let editor_quiet = Rc::new(Cell::new(false));
        {
            let s = sender.clone();
            let quiet = editor_quiet.clone();
            editor.buffer().connect_changed(move |_| {
                if !quiet.get() {
                    s.input(FilesTabInput::EditorChanged);
                }
            });
        }
        let editor_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .min_content_height(200)
            .child(&editor)
            .build();
        editor_page.append(&editor_scroll);

        let ai_row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        let summarize = text_button("Summarize");
        let s = sender.clone();
        summarize.connect_clicked(move |_| s.input(FilesTabInput::Summarize));
        ai_row.append(&summarize);
        let rewrite_entry = entry("Rewrite: e.g. fix grammar, shorten…");
        let s = sender.clone();
        rewrite_entry.connect_activate(move |_| s.input(FilesTabInput::Rewrite));
        ai_row.append(&rewrite_entry);
        let rewrite = text_button("Rewrite");
        let s = sender.clone();
        rewrite.connect_clicked(move |_| s.input(FilesTabInput::Rewrite));
        ai_row.append(&rewrite);
        editor_page.append(&ai_row);
        pages.add_named(&editor_page, Some("editor"));
        root.append(&pages);

        // ── AI result pane (both pages) ─────────────────────────────
        let ai_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
        ai_box.add_css_class("notes-ai-pane");
        ai_box.set_visible(false);
        let ai_status = label("", &["notes-card-meta"]);
        ai_box.append(&ai_status);
        let ai_text = label("", &["notes-ai-text"]);
        ai_text.set_selectable(true);
        let ai_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .max_content_height(260)
            .child(&ai_text)
            .build();
        ai_box.append(&ai_scroll);
        let ai_actions = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        ai_actions.set_halign(gtk::Align::End);
        let ai_stop = text_button("Stop");
        let ai_replace = text_button("Replace note");
        let ai_append = text_button("Append to note");
        let ai_copy = text_button("Copy");
        let ai_dismiss = text_button("Dismiss");
        let actions: [(&gtk::Button, fn() -> FilesTabInput); 5] = [
            (&ai_stop, || FilesTabInput::AiStop),
            (&ai_replace, || FilesTabInput::AiReplace),
            (&ai_append, || FilesTabInput::AiAppend),
            (&ai_copy, || FilesTabInput::AiCopy),
            (&ai_dismiss, || FilesTabInput::AiDismiss),
        ];
        for (b, input) in actions {
            let s = sender.clone();
            b.connect_clicked(move |_| s.input(input()));
            ai_actions.append(b);
        }
        ai_box.append(&ai_actions);
        root.append(&ai_box);

        let mut effects = EffectScope::new();
        let s = sender.clone();
        effects.push(move |_| {
            let _ = config_manager().config().notes().get();
            s.input(FilesTabInput::ConfigChanged);
        });

        let cfg = config_manager().config().notes().get_untracked();
        let model = FilesTabModel {
            folder: resolved_folder(&cfg.folder),
            git_history: cfg.git_history,
            ai_capture: cfg.ai_capture,
            context_notes: cfg.ai_context_notes.clamp(1, 12) as usize,
            notes: Vec::new(),
            scan_error: None,
            scanned: false,
            query: String::new(),
            open: None,
            ai: AiState {
                generation: 0,
                kind: AiKind::Ask,
                text: String::new(),
                running: false,
                error: None,
                sources: Vec::new(),
                cancel: Arc::new(AtomicBool::new(false)),
            },
            pages,
            hint,
            list,
            scroller,
            editor,
            editor_quiet,
            editor_title,
            rewrite_entry,
            ai_box,
            ai_status,
            ai_text,
            ai_replace,
            ai_append,
            ai_stop,
            save_token: Rc::default(),
            git_token: Rc::default(),
            _effects: effects,
        };
        let widgets = view_output!();
        model.render_list(&sender);
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            FilesTabInput::Shown | FilesTabInput::Rescan => self.rescan(&sender),
            FilesTabInput::ConfigChanged => {
                let cfg = config_manager().config().notes().get_untracked();
                let folder = resolved_folder(&cfg.folder);
                self.git_history = cfg.git_history;
                self.ai_capture = cfg.ai_capture;
                self.context_notes = cfg.ai_context_notes.clamp(1, 12) as usize;
                if folder != self.folder {
                    self.folder = folder;
                    self.open = None;
                    self.pages.set_visible_child_name("list");
                    self.notes.clear();
                    self.scanned = false;
                    self.rescan(&sender);
                }
            }
            FilesTabInput::Query(q) => {
                self.query = q;
                self.render_list(&sender);
            }
            FilesTabInput::OpenNote(rel) => {
                let Some(path) = self.folder.as_ref().and_then(|f| contained(f, &rel)) else {
                    return;
                };
                sender.oneshot_command(async move {
                    let p = path.clone();
                    let res = tokio::task::spawn_blocking(move || {
                        std::fs::read_to_string(&p).map_err(|e| e.to_string())
                    })
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()));
                    FilesTabCommandOutput::Loaded { rel, res }
                });
            }
            FilesTabInput::Back => {
                self.flush_save(&sender);
                self.open = None;
                self.clear_ai();
                self.pages.set_visible_child_name("list");
                self.rescan(&sender);
            }
            FilesTabInput::EditorChanged => {
                let next = self.save_token.get().wrapping_add(1);
                self.save_token.set(next);
                let s = sender.clone();
                glib::timeout_add_local_once(SAVE_DEBOUNCE, move || {
                    let _ = s.input_sender().send(FilesTabInput::SaveNow(next));
                });
            }
            FilesTabInput::SaveNow(token) => {
                if token == self.save_token.get() {
                    self.flush_save(&sender);
                }
            }
            FilesTabInput::NewNote => {
                let Some(folder) = self.folder.clone() else {
                    return;
                };
                let title = match self.query.trim() {
                    "" => "Untitled".to_string(),
                    q => q.to_string(),
                };
                sender.oneshot_command(async move {
                    let res = tokio::task::spawn_blocking(move || create_note(&folder, &title))
                        .await
                        .unwrap_or_else(|e| Err(e.to_string()));
                    FilesTabCommandOutput::Created(res)
                });
            }
            FilesTabInput::OpenExternally => {
                if let Some(path) = self.open.as_ref().map(|o| o.path.clone()) {
                    self.flush_save(&sender);
                    if let Err(e) = mshell_process::desktop::open(&path) {
                        tracing::warn!(error = %e, "notes: open failed");
                    }
                }
            }
            FilesTabInput::Capture(text) => self.capture(&sender, text),
            FilesTabInput::Ask(question) => {
                let context = pick_context(&self.notes, &question, self.context_notes);
                if context.is_empty() {
                    self.show_ai_error(AiKind::Ask, "No notes to ask about yet");
                    return;
                }
                self.ai.sources = context.iter().map(|n| n.title.clone()).collect();
                let (system, user) = ask_prompt(&context, &question);
                self.run_ai(&sender, AiKind::Ask, system, user);
            }
            FilesTabInput::Summarize => {
                let (title, body) = self.editor_title_and_text();
                if body.trim().is_empty() {
                    return;
                }
                self.ai.sources.clear();
                let (system, user) = summarize_prompt(&title, &body);
                self.run_ai(&sender, AiKind::Summarize, system, user);
            }
            FilesTabInput::Rewrite => {
                let instruction = self.rewrite_entry.text().to_string();
                let (_, body) = self.editor_title_and_text();
                if instruction.trim().is_empty() || body.trim().is_empty() {
                    self.rewrite_entry.grab_focus();
                    return;
                }
                self.ai.sources.clear();
                let (system, user) = rewrite_prompt(&body, &instruction);
                self.run_ai(&sender, AiKind::Rewrite, system, user);
            }
            FilesTabInput::AiStop => {
                self.ai.cancel.store(true, Ordering::Relaxed);
            }
            FilesTabInput::AiDismiss => self.clear_ai(),
            FilesTabInput::AiCopy => {
                self.ai_text.clipboard().set_text(&self.ai.text);
            }
            FilesTabInput::AiReplace => {
                if self.open.is_some() && !self.ai.running && !self.ai.text.trim().is_empty() {
                    let text = strip_fence(&self.ai.text);
                    self.editor.buffer().set_text(&text);
                    self.flush_save(&sender);
                    self.clear_ai();
                }
            }
            FilesTabInput::AiAppend => {
                if self.open.is_some() && !self.ai.running && !self.ai.text.trim().is_empty() {
                    let buf = self.editor.buffer();
                    let mut end = buf.end_iter();
                    buf.insert(
                        &mut end,
                        &format!("\n\n## Summary\n\n{}\n", self.ai.text.trim()),
                    );
                    self.flush_save(&sender);
                    self.clear_ai();
                }
            }
            FilesTabInput::GitNow(token) => {
                if token != self.git_token.get() || !self.git_history {
                    return;
                }
                let Some(folder) = self.folder.clone() else {
                    return;
                };
                sender.oneshot_command(async move {
                    let res = tokio::task::spawn_blocking(move || files_model::git_commit(&folder))
                        .await
                        .unwrap_or_else(|e| Err(e.to_string()));
                    FilesTabCommandOutput::GitDone(res)
                });
            }
            FilesTabInput::OpenSettings => {
                mshell_settings::open_settings_at_section("widgets/notes")
            }
        }
    }

    fn update_cmd(
        &mut self,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            FilesTabCommandOutput::Scanned(res) => {
                self.scanned = true;
                match res {
                    Ok(notes) => {
                        self.notes = notes;
                        self.scan_error = None;
                    }
                    Err(e) => self.scan_error = Some(e),
                }
                self.render_list(&sender);
            }
            FilesTabCommandOutput::Loaded { rel, res } => match res {
                Ok(text) => {
                    let Some(path) = self.folder.as_ref().and_then(|f| contained(f, &rel)) else {
                        return;
                    };
                    self.editor_title.set_label(&note_title(&rel, &text));
                    self.editor_title.set_tooltip_text(Some(&rel));
                    self.editor_quiet.set(true);
                    self.editor.buffer().set_text(&text);
                    self.editor_quiet.set(false);
                    self.open = Some(OpenNote { rel, path });
                    self.clear_ai();
                    self.pages.set_visible_child_name("editor");
                    self.editor.grab_focus();
                }
                Err(e) => toast_warn("Couldn't open the note", &e),
            },
            FilesTabCommandOutput::Saved(res) => match res {
                Ok(()) => self.schedule_git(&sender),
                Err(e) => toast_warn("Note not saved", &e),
            },
            FilesTabCommandOutput::Created(res) => match res {
                Ok(rel) => {
                    self.schedule_git(&sender);
                    sender.input(FilesTabInput::OpenNote(rel));
                }
                Err(e) => toast_warn("Couldn't create the note", &e),
            },
            FilesTabCommandOutput::Captured(res) => match res {
                Ok(line) => {
                    push_toast(ToastEvent {
                        icon: "document-save-symbolic".into(),
                        title: "Captured to Inbox".into(),
                        body: Some(line),
                        severity: ToastSeverity::Positive,
                    });
                    self.schedule_git(&sender);
                    self.rescan(&sender);
                }
                Err(e) => toast_warn("Capture failed", &e),
            },
            FilesTabCommandOutput::AiDelta { generation, text } => {
                if generation == self.ai.generation {
                    self.ai.text.push_str(&text);
                    self.render_ai();
                }
            }
            FilesTabCommandOutput::AiDone { generation, res } => {
                if generation == self.ai.generation {
                    self.ai.running = false;
                    if let Err(e) = res {
                        self.ai.error = Some(e);
                    } else if self.ai.text.trim().is_empty() {
                        self.ai.error = Some("The model sent an empty reply".into());
                    }
                    self.render_ai();
                }
            }
            FilesTabCommandOutput::GitDone(res) => match res {
                Ok(Some(msg)) => tracing::info!(%msg, "notes: committed"),
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!(error = %e, "notes: git history failed");
                    toast_warn("Notes git history", &e);
                }
            },
        }
    }
}

fn toast_warn(title: &str, body: &str) {
    push_toast(ToastEvent {
        icon: "dialog-warning-symbolic".into(),
        title: title.into(),
        body: Some(body.into()),
        severity: ToastSeverity::Warn,
    });
}

/// A model that wraps its whole reply in a ``` fence: keep the inside.
fn strip_fence(s: &str) -> String {
    let t = s.trim();
    if let Some(inner) = t.strip_prefix("```") {
        let inner = inner.split_once('\n').map_or("", |(_, rest)| rest);
        if let Some(body) = inner.trim_end().strip_suffix("```") {
            return format!("{}\n", body.trim_end());
        }
    }
    format!("{t}\n")
}

/// Create `<slug>.md` titled `title` in `folder` (created if missing);
/// returns its relative name.
fn create_note(folder: &std::path::Path, title: &str) -> Result<String, String> {
    std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    let name = new_note_name(title, |n| folder.join(n).exists());
    let path = contained(folder, &name).ok_or("bad note name")?;
    margo_fs::atomic_write::atomic_write(&path, format!("# {}\n\n", title.trim()).as_bytes())
        .map_err(|e| e.to_string())?;
    Ok(name)
}

impl FilesTabModel {
    fn rescan(&mut self, sender: &ComponentSender<Self>) {
        let Some(folder) = self.folder.clone() else {
            self.render_list(sender);
            return;
        };
        sender.oneshot_command(async move {
            let res = tokio::task::spawn_blocking(move || {
                files_model::scan(&folder).map_err(|e| format!("{}: {e}", folder.display()))
            })
            .await
            .unwrap_or_else(|e| Err(e.to_string()));
            FilesTabCommandOutput::Scanned(res)
        });
    }

    fn render_list(&self, sender: &ComponentSender<Self>) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let hint = if self.folder.is_none() {
            Some(
                "Markdown notes live in a folder you pick in Settings → Widgets → Notes Hub."
                    .to_string(),
            )
        } else if let Some(e) = &self.scan_error {
            Some(format!(
                "Can't read the notes folder ({e}). Capture or + creates it."
            ))
        } else if !self.scanned {
            Some("Loading…".to_string())
        } else if self.notes.is_empty() {
            Some("No Markdown notes yet — capture a todo or press + for a new note.".to_string())
        } else {
            None
        };
        self.hint.set_label(hint.as_deref().unwrap_or(""));
        self.hint.set_visible(hint.is_some());
        if self.folder.is_none() {
            let row = gtk::ListBoxRow::new();
            row.set_activatable(false);
            let b = text_button("Open Notes Hub settings");
            b.set_halign(gtk::Align::Start);
            let s = sender.clone();
            b.connect_clicked(move |_| s.input(FilesTabInput::OpenSettings));
            row.set_child(Some(&b));
            self.list.append(&row);
            return;
        }
        let now = SystemTime::now();
        let shown: Vec<&NoteFile> = self
            .notes
            .iter()
            .filter(|n| matches_query(n, &self.query))
            .collect();
        if shown.is_empty() && !self.notes.is_empty() {
            self.hint.set_label("No note matches the search");
            self.hint.set_visible(true);
        }
        for n in shown {
            let row = gtk::ListBoxRow::new();
            row.set_activatable(false);
            let b = gtk::Button::new();
            b.add_css_class("notes-file-row");
            let col = gtk::Box::new(gtk::Orientation::Vertical, 2);
            let t = gtk::Label::new(Some(&n.title));
            t.add_css_class("notes-card-title");
            t.set_xalign(0.0);
            t.set_ellipsize(gtk::pango::EllipsizeMode::End);
            col.append(&t);
            let age = n
                .modified
                .and_then(|m| now.duration_since(m).ok())
                .map(|d| crate::github::ago(d.as_millis() as i64, 0))
                .unwrap_or_default();
            let mut meta = n.rel.clone();
            if !age.is_empty() {
                meta.push_str(&format!(" · {age} ago"));
            }
            if n.too_big {
                meta.push_str(" · too large to open here");
            }
            let m = gtk::Label::new(Some(&meta));
            m.add_css_class("notes-card-meta");
            m.set_xalign(0.0);
            m.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            col.append(&m);
            b.set_child(Some(&col));
            if n.too_big {
                b.set_sensitive(false);
            }
            let s = sender.clone();
            let rel = n.rel.clone();
            b.connect_clicked(move |_| s.input(FilesTabInput::OpenNote(rel.clone())));
            row.set_child(Some(&b));
            self.list.append(&row);
        }
    }

    fn editor_title_and_text(&self) -> (String, String) {
        let buf = self.editor.buffer();
        let text = buf
            .text(&buf.start_iter(), &buf.end_iter(), false)
            .to_string();
        let title = self
            .open
            .as_ref()
            .map(|o| note_title(&o.rel, &text))
            .unwrap_or_default();
        (title, text)
    }

    /// Write the open note now (if it changed since the last save token).
    fn flush_save(&self, sender: &ComponentSender<Self>) {
        let Some(open) = &self.open else { return };
        // Invalidate any pending debounce.
        self.save_token.set(self.save_token.get().wrapping_add(1));
        let (title, text) = self.editor_title_and_text();
        self.editor_title.set_label(&title);
        let path = open.path.clone();
        sender.oneshot_command(async move {
            let res = tokio::task::spawn_blocking(move || {
                let unchanged = std::fs::read_to_string(&path).is_ok_and(|t| t == text);
                if unchanged {
                    return Ok(());
                }
                margo_fs::atomic_write::atomic_write(&path, text.as_bytes())
                    .map_err(|e| e.to_string())
            })
            .await
            .unwrap_or_else(|e| Err(e.to_string()));
            FilesTabCommandOutput::Saved(res)
        });
    }

    fn schedule_git(&self, sender: &ComponentSender<Self>) {
        if !self.git_history {
            return;
        }
        let next = self.git_token.get().wrapping_add(1);
        self.git_token.set(next);
        let s = sender.clone();
        glib::timeout_add_local_once(GIT_DEBOUNCE, move || {
            let _ = s.input_sender().send(FilesTabInput::GitNow(next));
        });
    }

    fn capture(&mut self, sender: &ComponentSender<Self>, text: String) {
        let Some(folder) = self.folder.clone() else {
            toast_warn(
                "Capture failed",
                "Pick a notes folder in Settings → Widgets → Notes Hub",
            );
            return;
        };
        let use_ai = self.ai_capture;
        sender.oneshot_command(async move {
            let res = tokio::task::spawn_blocking(move || -> Result<String, String> {
                let raw = if use_ai {
                    // The AI only tidies the wording; any failure falls back
                    // to the raw text, so a capture is never lost.
                    let (system, user) = capture_prompt(&text);
                    ai_complete(system, user).unwrap_or_else(|e| {
                        tracing::warn!(error = %e, "notes: AI capture failed; keeping raw text");
                        text.clone()
                    })
                } else {
                    text.clone()
                };
                let line = capture_line(&raw)
                    .or_else(|| capture_line(&text))
                    .ok_or("nothing to capture")?;
                std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
                let path = contained(&folder, INBOX).ok_or("bad inbox path")?;
                let existing = std::fs::read_to_string(&path).ok();
                let updated = append_to_inbox(existing.as_deref(), &line);
                margo_fs::atomic_write::atomic_write(&path, updated.as_bytes())
                    .map_err(|e| e.to_string())?;
                Ok(line)
            })
            .await
            .unwrap_or_else(|e| Err(e.to_string()));
            FilesTabCommandOutput::Captured(res)
        });
    }

    fn run_ai(
        &mut self,
        sender: &ComponentSender<Self>,
        kind: AiKind,
        system: String,
        user: String,
    ) {
        self.ai.cancel.store(true, Ordering::Relaxed);
        self.ai.generation += 1;
        self.ai.kind = kind;
        self.ai.text.clear();
        self.ai.error = None;
        self.ai.running = true;
        self.ai.cancel = Arc::new(AtomicBool::new(false));
        self.render_ai();
        let generation = self.ai.generation;
        let cancel = self.ai.cancel.clone();
        sender.command(move |out, _shutdown| async move {
            let token_out = out.clone();
            let res = tokio::task::spawn_blocking(move || -> Result<(), String> {
                let cfg = ai_config(system)?;
                mshell_ai::chat_stream(&cfg, &[mshell_ai::Message::user(user)], &cancel, |d| {
                    let _ = token_out.send(FilesTabCommandOutput::AiDelta {
                        generation,
                        text: d.to_string(),
                    });
                })
            })
            .await
            .unwrap_or_else(|e| Err(e.to_string()));
            let _ = out.send(FilesTabCommandOutput::AiDone { generation, res });
        });
    }

    fn show_ai_error(&mut self, kind: AiKind, error: &str) {
        self.ai.generation += 1;
        self.ai.kind = kind;
        self.ai.text.clear();
        self.ai.running = false;
        self.ai.error = Some(error.to_string());
        self.render_ai();
    }

    fn clear_ai(&mut self) {
        self.ai.cancel.store(true, Ordering::Relaxed);
        self.ai.generation += 1;
        self.ai.running = false;
        self.ai.text.clear();
        self.ai.error = None;
        self.ai.sources.clear();
        self.render_ai();
    }

    fn render_ai(&self) {
        let ai = &self.ai;
        let visible = ai.running || ai.error.is_some() || !ai.text.is_empty();
        self.ai_box.set_visible(visible);
        let status = if let Some(e) = &ai.error {
            e.clone()
        } else if ai.running {
            ai.kind.busy_label().to_string()
        } else if !ai.sources.is_empty() {
            format!("From: {}", ai.sources.join(", "))
        } else {
            String::new()
        };
        self.ai_status.set_label(&status);
        self.ai_status.set_visible(!status.is_empty());
        if ai.error.is_some() {
            self.ai_status.add_css_class("label-small-error");
        } else {
            self.ai_status.remove_css_class("label-small-error");
        }
        self.ai_text.set_label(&ai.text);
        self.ai_text.set_visible(!ai.text.is_empty());
        let done = !ai.running && ai.error.is_none() && !ai.text.trim().is_empty();
        self.ai_stop.set_visible(ai.running);
        self.ai_replace
            .set_visible(done && ai.kind == AiKind::Rewrite && self.open.is_some());
        self.ai_append
            .set_visible(done && ai.kind == AiKind::Summarize && self.open.is_some());
    }
}

/// The user's AI provider (Settings → AI) with our system prompt. Reads the
/// keyring — call it off the GTK thread. The key is never logged.
fn ai_config(system: String) -> Result<mshell_ai::AiConfig, String> {
    let mut cfg = mshell_ai::config::resolved();
    if cfg.provider.needs_key() && cfg.api_key.trim().is_empty() {
        return Err("No AI provider set up — add one in Settings → AI".into());
    }
    cfg.system_prompt = system;
    Ok(cfg)
}

/// A whole (non-streamed to the UI) completion; blocking.
fn ai_complete(system: String, user: String) -> Result<String, String> {
    let cfg = ai_config(system)?;
    let mut out = String::new();
    let cancel = Arc::new(AtomicBool::new(false));
    mshell_ai::chat_stream(&cfg, &[mshell_ai::Message::user(user)], &cancel, |d| {
        out.push_str(d);
    })?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fenced_replies_are_unwrapped() {
        assert_eq!(strip_fence("```markdown\n# A\n\nb\n```"), "# A\n\nb\n");
        assert_eq!(strip_fence("  # A  "), "# A\n");
        assert_eq!(strip_fence("```\nunterminated"), "```\nunterminated\n");
    }

    #[test]
    fn create_note_picks_a_free_slug() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("n");
        assert_eq!(create_note(&root, "Plan").unwrap(), "plan.md");
        assert_eq!(create_note(&root, "Plan").unwrap(), "plan-2.md");
        assert_eq!(
            std::fs::read_to_string(root.join("plan.md")).unwrap(),
            "# Plan\n\n"
        );
    }
}
