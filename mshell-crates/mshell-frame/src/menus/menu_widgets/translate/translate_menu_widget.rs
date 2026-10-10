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
use relm4::{Component, ComponentParts, ComponentSender, RelmWidgetExt, gtk};

pub(crate) struct TranslateMenuWidgetInit {}

#[derive(Debug)]
pub(crate) struct TranslateMenuWidgetModel {
    busy: bool,
    error: Option<String>,
    result: Option<TranslateResult>,
}

#[derive(Debug)]
pub(crate) enum TranslateMenuWidgetInput {
    /// Carries the input box's text at click/Enter time — read directly
    /// off the `gtk::Entry` in the view-macro closure, so this handler
    /// never needs widget access itself.
    Translate(String),
    TargetLangChanged(String),
    CopyResult,
}

#[derive(Debug)]
pub(crate) enum TranslateMenuWidgetCommandOutput {
    Done(Result<TranslateResult, String>),
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
            set_spacing: 10,
            set_margin_all: 14,

            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 8,

                #[name = "target_entry"]
                gtk::Entry {
                    set_hexpand: false,
                    set_width_chars: 4,
                    set_max_width_chars: 4,
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
                set_halign: gtk::Align::Start,
                set_visible: false,
            },

            #[name = "result_label"]
            gtk::Label {
                set_wrap: true,
                set_xalign: 0.0,
                set_selectable: true,
                set_visible: false,
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
                set_selectable: true,
                set_visible: false,
            },

            #[name = "copy_button"]
            gtk::Button {
                set_css_classes: &["ok-button-flat"],
                set_label: "Copy result",
                set_halign: gtk::Align::Start,
                set_visible: false,
                connect_clicked[sender] => move |_| {
                    sender.input(TranslateMenuWidgetInput::CopyResult);
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = TranslateMenuWidgetModel {
            busy: false,
            error: None,
            result: None,
        };

        let widgets = view_output!();
        widgets.target_entry.set_text(&config::load().target_lang);

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            TranslateMenuWidgetInput::TargetLangChanged(lang) => {
                let mut s = config::load();
                s.target_lang = lang;
                config::save(&s);
            }
            TranslateMenuWidgetInput::CopyResult => {
                if let Some(r) = &self.result {
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
                let cfg = config::resolved();
                sender.command(move |out, _shutdown| async move {
                    let res = tokio::task::spawn_blocking(move || {
                        mshell_translate::translate(&cfg, &text)
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
            &widgets.result_label,
            &widgets.alternatives_label,
            &widgets.copy_button,
        );
    }
}

fn refresh_view(
    model: &TranslateMenuWidgetModel,
    status_label: &gtk::Label,
    result_label: &gtk::Label,
    alternatives_label: &gtk::Label,
    copy_button: &gtk::Button,
) {
    match &model.error {
        Some(e) => {
            status_label.set_text(e);
            status_label.set_visible(true);
        }
        None => status_label.set_visible(false),
    }
    match &model.result {
        Some(r) => {
            let text = match &r.detected_source {
                Some(src) => format!("({src}) {}", r.translated),
                None => r.translated.clone(),
            };
            result_label.set_text(&text);
            result_label.set_visible(true);
            copy_button.set_visible(true);

            if r.alternatives.is_empty() {
                alternatives_label.set_visible(false);
            } else {
                let text = r
                    .alternatives
                    .iter()
                    .map(|(pos, terms)| format!("{pos}: {}", terms.join(", ")))
                    .collect::<Vec<_>>()
                    .join("\n");
                alternatives_label.set_text(&text);
                alternatives_label.set_visible(true);
            }
        }
        None => {
            result_label.set_visible(false);
            alternatives_label.set_visible(false);
            copy_button.set_visible(false);
        }
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
