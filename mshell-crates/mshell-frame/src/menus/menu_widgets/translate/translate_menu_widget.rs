//! Translate menu widget — content surface for `MenuType::Translate`.
//!
//! Manual-use surface: type or paste text, pick a target language,
//! press Translate. The "select-or-copy, then press one keybind"
//! capture flow (the primary trigger — see `mshell_translate::capture`)
//! is deliberately headless (capture → translate → clipboard + desktop
//! notification) and doesn't touch this panel at all; this is the
//! surface for everything that needs eyes on it: a manual lookup, or
//! overriding the target language / provider (Settings → Translate).

use mshell_translate::{TranslateResult, config, looks_sensitive};
use relm4::gtk::prelude::{BoxExt, ButtonExt, EditableExt, EntryExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

pub(crate) struct TranslateMenuWidgetInit {}

#[derive(Debug)]
pub(crate) struct TranslateMenuWidgetModel {
    busy: bool,
    error: Option<String>,
    /// The result, alongside the target language actually used — may
    /// differ from the `target_entry` setting when `translate_auto`
    /// flipped direction (selected text was already in that language).
    result: Option<(TranslateResult, String)>,
    /// Cloned at init so `ParentRevealChanged` can grab focus for it
    /// without needing `Widgets` access — same shape as the AI menu
    /// widget's `self.input`.
    input_entry: gtk::Entry,
}

#[derive(Debug)]
pub(crate) enum TranslateMenuWidgetInput {
    /// Broadcast by the menu stack when this panel's reveal state
    /// flips (see `menu.rs`'s `BroadcastReveal`). On becoming visible,
    /// focuses the input entry so you can start typing immediately —
    /// matches the AI menu's `mshellctl menu ai` / pill-click behavior.
    ParentRevealChanged(bool),
    /// Carries the input box's text at click/Enter time — read directly
    /// off the `gtk::Entry` in the view-macro closure, so this handler
    /// never needs widget access itself.
    Translate(String),
    TargetLangChanged(String),
    CopyResult,
}

#[derive(Debug)]
pub(crate) enum TranslateMenuWidgetCommandOutput {
    Done(Result<(TranslateResult, String), String>),
}

#[relm4::component(pub(crate))]
impl Component for TranslateMenuWidgetModel {
    type CommandOutput = TranslateMenuWidgetCommandOutput;
    type Input = TranslateMenuWidgetInput;
    type Output = ();
    type Init = TranslateMenuWidgetInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "translate-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 12,

            // ── §12 panel header (DESIGN.md §12) ──
            gtk::Box {
                add_css_class: "panel-header",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 12,

                gtk::Image {
                    add_css_class: "panel-header-icon",
                    set_valign: gtk::Align::Center,
                    set_icon_name: Some("preferences-desktop-locale-symbolic"),
                },
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_hexpand: true,
                    set_valign: gtk::Align::Center,
                    gtk::Label {
                        add_css_class: "panel-title",
                        set_halign: gtk::Align::Start,
                        set_label: "Translate",
                    },
                    gtk::Label {
                        add_css_class: "label-small",
                        set_halign: gtk::Align::Start,
                        set_xalign: 0.0,
                        set_label: "Auto-detects the source — just type, paste, or Enter",
                    },
                },
            },

            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 8,

                gtk::Label {
                    add_css_class: "label-small",
                    set_label: "to",
                },

                #[name = "target_entry"]
                gtk::Entry {
                    add_css_class: "translate-lang-entry",
                    set_hexpand: false,
                    set_width_chars: 4,
                    set_max_width_chars: 4,
                    set_placeholder_text: Some("tr"),
                    set_tooltip_text: Some("Target language code (e.g. tr, en, es)"),
                    connect_changed[sender] => move |e| {
                        sender.input(TranslateMenuWidgetInput::TargetLangChanged(e.text().to_string()));
                    },
                },

                #[name = "input_entry"]
                gtk::Entry {
                    set_hexpand: true,
                    set_placeholder_text: Some("Type or paste text…"),
                    connect_activate[sender] => move |entry| {
                        sender.input(TranslateMenuWidgetInput::Translate(entry.text().to_string()));
                    },
                },

                gtk::Button {
                    set_css_classes: &["ok-button-surface"],
                    set_label: "Translate",
                    connect_clicked[sender, input_entry] => move |_| {
                        sender.input(TranslateMenuWidgetInput::Translate(input_entry.text().to_string()));
                    },
                },
            },

            #[name = "status_label"]
            gtk::Label {
                add_css_class: "dim-label",
                set_wrap: true,
                set_xalign: 0.0,
                set_halign: gtk::Align::Start,
                set_visible: false,
            },

            // One card for the whole result — direction + translation +
            // (single-word only) dictionary alternatives + copy — so it
            // reads as one answer, not three loose labels.
            #[name = "result_card"]
            gtk::Box {
                add_css_class: "translate-result-card",
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 6,
                set_visible: false,

                #[name = "direction_label"]
                gtk::Label {
                    add_css_class: "label-small",
                    set_halign: gtk::Align::Start,
                },

                #[name = "result_label"]
                gtk::Label {
                    add_css_class: "translate-result-text",
                    set_wrap: true,
                    set_xalign: 0.0,
                    set_halign: gtk::Align::Start,
                    set_selectable: true,
                },

                // Single-word lookups only (Google's dict-chrome-ex "basic
                // dictionary" data — empty for anything multi-word): other
                // meanings grouped by part of speech, e.g. "light" also
                // meaning "hafif" (adjective) alongside the main "ışık".
                #[name = "alternatives_label"]
                gtk::Label {
                    add_css_class: "dim-label",
                    set_wrap: true,
                    set_xalign: 0.0,
                    set_halign: gtk::Align::Start,
                    set_selectable: true,
                    set_visible: false,
                },

                #[name = "copy_button"]
                gtk::Button {
                    set_css_classes: &["ok-button-flat"],
                    set_label: "Copy result",
                    set_halign: gtk::Align::Start,
                    connect_clicked[sender] => move |_| {
                        sender.input(TranslateMenuWidgetInput::CopyResult);
                    },
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let widgets = view_output!();
        let model = TranslateMenuWidgetModel {
            busy: false,
            error: None,
            result: None,
            input_entry: widgets.input_entry.clone(),
        };
        widgets.target_entry.set_text(&config::load().target_lang);

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            TranslateMenuWidgetInput::ParentRevealChanged(visible) => {
                if visible {
                    // Deferred to idle so the entry is mapped + the layer
                    // surface already has keyboard focus — same timing
                    // the AI menu widget uses for the same reason.
                    let entry = self.input_entry.clone();
                    relm4::gtk::glib::idle_add_local_once(move || {
                        entry.grab_focus();
                    });
                }
            }
            TranslateMenuWidgetInput::TargetLangChanged(lang) => {
                let mut s = config::load();
                s.target_lang = lang;
                config::save(&s);
            }
            TranslateMenuWidgetInput::CopyResult => {
                if let Some((r, _)) = &self.result {
                    copy_to_clipboard(&r.translated);
                }
            }
            TranslateMenuWidgetInput::Translate(text) => {
                // Serialise: ignore a new request while one is in flight.
                if self.busy || text.trim().is_empty() {
                    return;
                }
                if let Some(reason) = looks_sensitive(&text) {
                    self.error = Some(format!("not sent — {reason}"));
                    self.result = None;
                    return;
                }
                self.busy = true;
                self.error = None;
                let secondary_lang = config::load().secondary_lang;
                sender.command(move |out, _shutdown| async move {
                    let res = tokio::task::spawn_blocking(move || {
                        let cfg = config::resolved();
                        mshell_translate::translate_auto(&cfg, &secondary_lang, &text)
                    })
                    .await
                    .unwrap_or_else(|_| Err("worker panicked".into()));
                    let _ = out.send(TranslateMenuWidgetCommandOutput::Done(res));
                });
            }
        }
    }

    fn update_cmd_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::CommandOutput,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        self.busy = false;
        match message {
            TranslateMenuWidgetCommandOutput::Done(Ok(r)) => {
                self.result = Some(r);
                self.error = None;
            }
            TranslateMenuWidgetCommandOutput::Done(Err(e)) => {
                self.error = Some(e);
                self.result = None;
            }
        }
        refresh_view(
            self,
            &widgets.status_label,
            &widgets.result_card,
            &widgets.direction_label,
            &widgets.result_label,
            &widgets.alternatives_label,
        );
    }
}

fn refresh_view(
    model: &TranslateMenuWidgetModel,
    status_label: &gtk::Label,
    result_card: &gtk::Box,
    direction_label: &gtk::Label,
    result_label: &gtk::Label,
    alternatives_label: &gtk::Label,
) {
    match &model.error {
        Some(e) => {
            status_label.set_text(e);
            status_label.set_visible(true);
        }
        None => status_label.set_visible(false),
    }
    match &model.result {
        Some((r, used_target)) => {
            let direction = match &r.detected_source {
                Some(src) => format!("{src} → {used_target}"),
                None => format!("→ {used_target}"),
            };
            direction_label.set_text(&direction);
            result_label.set_text(&r.translated);
            result_card.set_visible(true);

            if r.alternatives.is_empty() {
                alternatives_label.set_visible(false);
            } else {
                // Each part-of-speech group as its own bolded, capitalised
                // heading ("Noun: ışık, aydınlık, nur"), with a blank line
                // between groups so "Noun" / "Adjective" read as distinct
                // entries rather than running together.
                let markup = r
                    .alternatives
                    .iter()
                    .map(|(pos, terms)| {
                        let pos = gtk::glib::markup_escape_text(&capitalize(pos));
                        let terms = terms
                            .iter()
                            .map(|t| gtk::glib::markup_escape_text(t))
                            .collect::<Vec<_>>()
                            .join(", ");
                        format!("<b>{pos}</b>: {terms}")
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n");
                alternatives_label.set_markup(&markup);
                alternatives_label.set_visible(true);
            }
        }
        None => result_card.set_visible(false),
    }
}

/// Uppercase the first character ("noun" → "Noun") — Google's part-of-speech
/// tags come back lowercase.
fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn copy_to_clipboard(text: &str) {
    use std::io::Write;
    use std::process::{Command, Stdio};
    if let Ok(mut child) = Command::new("wl-copy").stdin(Stdio::piped()).spawn() {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        let _ = child.wait();
    }
}
