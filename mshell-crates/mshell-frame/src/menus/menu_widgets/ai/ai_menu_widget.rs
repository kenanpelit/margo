//! The AI chat menu content widget.
//!
//! State comes from `mshell-ai` (provider/model/key via `config::resolved`).
//! A send spawns the blocking-but-streaming `chat_stream` on a worker thread;
//! every token is marshalled back to the GTK loop as a `Token` command output
//! and appended to the in-progress assistant bubble. Stop flips a shared
//! cancel flag. History is persisted (when enabled) to the state dir.

use mshell_ai::{Message, Provider, Role, config};
use relm4::gtk::prelude::{
    AdjustmentExt, BoxExt, ButtonExt, EditableExt, EntryExt, OrientableExt, WidgetExt,
};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub(crate) struct AiMenuWidgetModel {
    /// The conversation (engine messages). The trailing assistant message is
    /// the one currently streaming.
    messages: Vec<Message>,
    streaming: bool,
    /// Shared with the worker — set by Stop to abort the stream.
    cancel: Arc<AtomicBool>,
    messages_box: gtk::Box,
    scroller: gtk::ScrolledWindow,
    input: gtk::Entry,
    send_btn: gtk::Button,
    stop_btn: gtk::Button,
    /// "Provider · model" line under the title; refreshed on open.
    meta_label: gtk::Label,
    /// Runtime CSS provider that applies the configurable transcript font.
    font_provider: gtk::CssProvider,
    /// Label of the assistant bubble being streamed into.
    current_ai: Option<gtk::Label>,
    /// Last time the streaming bubble's label was repainted. `gtk::Label`
    /// has no incremental append — `set_label` re-lays-out the whole
    /// (growing) string, so pushing every token is O(n²) over a long
    /// reply. We coalesce repaints to ~30 Hz against this; the final
    /// text is always flushed on `Done`.
    last_token_render: Option<std::time::Instant>,
    /// History restored from disk yet? (lazy, on first reveal.)
    loaded: bool,
}

impl std::fmt::Debug for AiMenuWidgetModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AiMenuWidgetModel")
            .field("messages", &self.messages.len())
            .field("streaming", &self.streaming)
            .finish()
    }
}

#[derive(Debug)]
pub(crate) enum AiMenuWidgetInput {
    Send,
    Stop,
    New,
    Retry,
    CopyLast,
    ParentRevealChanged(bool),
}

#[derive(Debug)]
pub(crate) enum AiMenuWidgetOutput {}

pub(crate) struct AiMenuWidgetInit {}

#[derive(Debug)]
pub(crate) enum AiMenuWidgetCommandOutput {
    Token(String),
    Done(Option<String>),
}

#[relm4::component(pub(crate))]
impl Component for AiMenuWidgetModel {
    type CommandOutput = AiMenuWidgetCommandOutput;
    type Input = AiMenuWidgetInput;
    type Output = AiMenuWidgetOutput;
    type Init = AiMenuWidgetInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "ai-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 8,

            // ── Header ──────────────────────────────────────────
            gtk::Box {
                add_css_class: "panel-header",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 12,

                gtk::Image {
                    add_css_class: "panel-header-icon",
                    set_icon_name: Some("starred-symbolic"),
                    set_valign: gtk::Align::Center,
                },
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_hexpand: true,
                    set_valign: gtk::Align::Center,

                    gtk::Label {
                        add_css_class: "panel-title",
                        set_label: "AI",
                        set_xalign: 0.0,
                    },
                    // Active provider · model (small, refreshed on open).
                    #[local_ref]
                    meta_label_widget -> gtk::Label {
                        add_css_class: "label-small",
                        add_css_class: "dim-label",
                        set_xalign: 0.0,
                    },
                },
                gtk::Button {
                    set_css_classes: &["ok-button-surface", "dns-action"],
                    set_icon_name: "edit-copy-symbolic",
                    set_tooltip_text: Some("Copy the last reply"),
                    connect_clicked => AiMenuWidgetInput::CopyLast,
                },
                gtk::Button {
                    set_css_classes: &["ok-button-surface", "dns-action"],
                    set_icon_name: "document-new-symbolic",
                    set_tooltip_text: Some("New conversation"),
                    connect_clicked => AiMenuWidgetInput::New,
                },
            },

            // ── Transcript ──────────────────────────────────────
            #[name = "scroller"]
            gtk::ScrolledWindow {
                add_css_class: "ai-transcript",
                set_vscrollbar_policy: gtk::PolicyType::Automatic,
                set_hscrollbar_policy: gtk::PolicyType::Never,
                set_vexpand: true,
                // Small floor only — the menu is fixed-height, so its config
                // `maximum_height` (Settings → Widgets → AI) governs the real
                // size and the transcript vexpands to fill it.
                set_min_content_height: 80,

                #[name = "messages_box"]
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 8,
                },
            },

            // ── Composer ────────────────────────────────────────
            // The prompt is its own full-width single line; the actions sit
            // on a compact row below, sized like the mode buttons.
            #[name = "input"]
            gtk::Entry {
                set_hexpand: true,
                set_placeholder_text: Some("Ask anything… (Enter to send)"),
                connect_activate => AiMenuWidgetInput::Send,
            },

            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 6,
                set_homogeneous: true,

                #[name = "send_btn"]
                gtk::Button {
                    set_css_classes: &["ok-button-surface", "dns-action"],
                    set_label: "Send",
                    connect_clicked => AiMenuWidgetInput::Send,
                },
                #[name = "stop_btn"]
                gtk::Button {
                    set_css_classes: &["ok-button-surface", "dns-action"],
                    set_label: "Stop",
                    set_visible: false,
                    connect_clicked => AiMenuWidgetInput::Stop,
                },
                gtk::Button {
                    set_css_classes: &["ok-button-surface", "dns-action"],
                    set_label: "Retry",
                    connect_clicked => AiMenuWidgetInput::Retry,
                },
            },
        }
    }

    fn init(
        _init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let meta_label_widget = gtk::Label::new(Some(&provider_model_label()));
        let widgets = view_output!();
        let model = AiMenuWidgetModel {
            messages: Vec::new(),
            streaming: false,
            cancel: Arc::new(AtomicBool::new(false)),
            messages_box: widgets.messages_box.clone(),
            scroller: widgets.scroller.clone(),
            input: widgets.input.clone(),
            send_btn: widgets.send_btn.clone(),
            stop_btn: widgets.stop_btn.clone(),
            meta_label: meta_label_widget.clone(),
            font_provider: gtk::CssProvider::new(),
            current_ai: None,
            last_token_render: None,
            loaded: false,
        };
        // Register the transcript-font provider on the display (USER priority so
        // it overrides the baked `.ai-bubble-text` rule) and apply the saved font.
        if let Some(display) = gtk::gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                &model.font_provider,
                gtk::STYLE_PROVIDER_PRIORITY_USER,
            );
        }
        apply_chat_font(&model.font_provider);
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            AiMenuWidgetInput::ParentRevealChanged(visible) => {
                if visible {
                    if !self.loaded {
                        self.loaded = true;
                        self.restore_history();
                    }
                    // Reflect any provider/model/font change from Settings.
                    self.meta_label.set_label(&provider_model_label());
                    apply_chat_font(&self.font_provider);
                    // Focus the prompt so you can type immediately on open
                    // (`mshellctl menu ai` / the pill). Deferred to idle so the
                    // entry is mapped + the layer surface has keyboard focus.
                    let entry = self.input.clone();
                    relm4::gtk::glib::idle_add_local_once(move || {
                        entry.grab_focus();
                    });
                }
            }
            AiMenuWidgetInput::Send => {
                if self.streaming {
                    return;
                }
                let prompt = self.input.text().trim().to_string();
                if prompt.is_empty() {
                    return;
                }
                self.input.set_text("");
                self.push_bubble(Role::User, &prompt);
                self.messages.push(Message::user(prompt));
                self.start_stream(&sender);
            }
            AiMenuWidgetInput::Retry => {
                if self.streaming {
                    return;
                }
                // Drop the trailing assistant turn (if any) and re-run.
                if matches!(self.messages.last(), Some(m) if m.role == Role::Assistant) {
                    self.messages.pop();
                }
                if self.messages.is_empty() {
                    return;
                }
                self.rebuild_transcript();
                self.start_stream(&sender);
            }
            AiMenuWidgetInput::Stop => {
                self.cancel.store(true, Ordering::Relaxed);
            }
            AiMenuWidgetInput::New => {
                self.cancel.store(true, Ordering::Relaxed);
                self.messages.clear();
                self.current_ai = None;
                self.streaming = false;
                clear_box(&self.messages_box);
                self.set_busy(false);
                self.persist();
            }
            AiMenuWidgetInput::CopyLast => {
                if let Some(last) = self
                    .messages
                    .iter()
                    .rev()
                    .find(|m| m.role == Role::Assistant)
                {
                    self.messages_box.clipboard().set_text(&last.text);
                }
            }
        }
    }

    fn update_cmd_with_view(
        &mut self,
        _widgets: &mut Self::Widgets,
        message: Self::CommandOutput,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            AiMenuWidgetCommandOutput::Token(delta) => {
                if let Some(last) = self.messages.last_mut()
                    && last.role == Role::Assistant
                {
                    last.text.push_str(&delta);
                }
                // Coalesce the (full-string, O(n)) label repaint to ~30 Hz
                // so a fast stream of small tokens doesn't trigger a Pango
                // relayout of the whole growing reply per token. The tail
                // not yet painted here is flushed by `Done`.
                let now = std::time::Instant::now();
                let due = self
                    .last_token_render
                    .is_none_or(|t| now.duration_since(t) >= std::time::Duration::from_millis(33));
                if due {
                    if let Some(label) = &self.current_ai
                        && let Some(last) = self.messages.last()
                    {
                        label.set_label(&last.text);
                    }
                    self.last_token_render = Some(now);
                    self.scroll_to_bottom();
                }
            }
            AiMenuWidgetCommandOutput::Done(err) => {
                self.streaming = false;
                self.set_busy(false);
                if let Some(e) = err {
                    // Surface the error in the (empty) assistant bubble.
                    if let Some(last) = self.messages.last_mut()
                        && last.role == Role::Assistant
                        && last.text.is_empty()
                    {
                        last.text = format!("⚠ {e}");
                    }
                }
                // Final flush: the token throttle above may have skipped
                // the last repaint, so always paint the complete reply
                // here (covers both the normal-finish and error paths).
                if let Some(label) = &self.current_ai
                    && let Some(last) = self.messages.last()
                {
                    label.set_label(&last.text);
                }
                self.scroll_to_bottom();
                self.last_token_render = None;
                self.current_ai = None;
                self.persist();
            }
        }
    }
}

impl AiMenuWidgetModel {
    fn set_busy(&self, busy: bool) {
        self.send_btn.set_visible(!busy);
        self.stop_btn.set_visible(busy);
        self.input.set_sensitive(!busy);
    }

    /// Append a chat bubble + return its text label (so the streaming reply can
    /// keep updating it).
    fn push_bubble(&mut self, role: Role, text: &str) -> gtk::Label {
        let row = gtk::Box::new(gtk::Orientation::Vertical, 2);
        row.add_css_class("ai-bubble");
        row.add_css_class(if role == Role::User {
            "ai-bubble-user"
        } else {
            "ai-bubble-ai"
        });
        let body = gtk::Label::new(Some(text));
        body.add_css_class("ai-bubble-text");
        body.set_xalign(0.0);
        body.set_wrap(true);
        body.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        body.set_selectable(true);

        // Header row: role label + a small copy button in the top-right corner.
        // The button copies this bubble's *current* text (works mid-stream too).
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let who = gtk::Label::new(Some(if role == Role::User { "You" } else { "AI" }));
        who.add_css_class("ai-bubble-role");
        who.set_xalign(0.0);
        who.set_hexpand(true);
        let copy = gtk::Button::from_icon_name("edit-copy-symbolic");
        copy.set_css_classes(&["ai-bubble-copy", "flat"]);
        copy.set_valign(gtk::Align::Start);
        copy.set_tooltip_text(Some("Copy"));
        {
            let body = body.clone();
            copy.connect_clicked(move |b| {
                b.clipboard().set_text(body.text().as_str());
            });
        }
        header.append(&who);
        header.append(&copy);

        row.append(&header);
        row.append(&body);
        self.messages_box.append(&row);
        body
    }

    /// Open the assistant bubble + spawn the streaming worker.
    fn start_stream(&mut self, sender: &ComponentSender<Self>) {
        self.messages.push(Message::assistant(""));
        let ai_label = self.push_bubble(Role::Assistant, "");
        self.current_ai = Some(ai_label);
        // First token of the new reply should paint immediately.
        self.last_token_render = None;
        self.streaming = true;
        self.set_busy(true);
        self.scroll_to_bottom();

        self.cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.cancel.clone();
        let cfg = config::resolved();
        // Send everything except the trailing empty assistant placeholder.
        let msgs: Vec<Message> = self
            .messages
            .iter()
            .filter(|m| !(m.role == Role::Assistant && m.text.is_empty()))
            .cloned()
            .collect();

        sender.command(move |out, _shutdown| async move {
            let token_out = out.clone();
            let res = tokio::task::spawn_blocking(move || {
                mshell_ai::chat_stream(&cfg, &msgs, &cancel, |delta| {
                    let _ = token_out.send(AiMenuWidgetCommandOutput::Token(delta.to_string()));
                })
            })
            .await
            .unwrap_or_else(|_| Err("worker panicked".into()));
            let _ = out.send(AiMenuWidgetCommandOutput::Done(res.err()));
        });
    }

    fn rebuild_transcript(&mut self) {
        clear_box(&self.messages_box);
        let msgs = self.messages.clone();
        for m in &msgs {
            self.push_bubble(m.role, &m.text);
        }
    }

    fn scroll_to_bottom(&self) {
        let adj = self.scroller.vadjustment();
        adj.set_value(adj.upper());
    }

    fn persist(&self) {
        let s = config::load();
        if !s.persist_history {
            return;
        }
        let dump: Vec<(&str, &str)> = self
            .messages
            .iter()
            .map(|m| {
                (
                    if m.role == Role::User { "user" } else { "ai" },
                    m.text.as_str(),
                )
            })
            .collect();
        if let Ok(json) = serde_json::to_string(&dump) {
            let p = history_path();
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(p, json);
        }
    }

    fn restore_history(&mut self) {
        if !config::load().persist_history {
            return;
        }
        let Ok(raw) = std::fs::read_to_string(history_path()) else {
            return;
        };
        let Ok(dump) = serde_json::from_str::<Vec<(String, String)>>(&raw) else {
            return;
        };
        for (role, text) in dump {
            let r = if role == "user" {
                Role::User
            } else {
                Role::Assistant
            };
            self.push_bubble(r, &text);
            self.messages.push(Message { role: r, text });
        }
        self.scroll_to_bottom();
    }
}

fn history_path() -> PathBuf {
    std::env::var("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())).join(".local/state")
        })
        .join("mshell/ai-session.json")
}

/// "Provider · model" for the header subtitle, from the live config.
fn provider_model_label() -> String {
    let s = config::load();
    let provider = Provider::parse(&s.provider);
    let model = if s.model.trim().is_empty() {
        provider.default_model().to_string()
    } else {
        s.model
    };
    format!("{} · {}", provider.label(), model)
}

/// Load the configured transcript font (size + optional family) into the
/// runtime CSS provider, overriding the baked `.ai-bubble-text` rule.
fn apply_chat_font(provider: &gtk::CssProvider) {
    let s = config::load();
    let size = s.font_size.clamp(8, 48);
    let fam = s.font_family.trim();
    let css = if fam.is_empty() {
        format!(".ai-bubble-text {{ font-size: {size}px; }}")
    } else {
        format!(".ai-bubble-text {{ font-size: {size}px; font-family: {fam}; }}")
    };
    provider.load_from_string(&css);
}

fn clear_box(b: &gtk::Box) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
}
