//! KeyboardLayout — bar pill for the active xkb keyboard layout.
//!
//! Port of the noctalia `keyboard_layout` widget. Render-only and
//! reactive: the layout name comes from `margo_service().keyboard_layout`
//! (mirrored from the compositor's state.json `keyboard_layout` field,
//! which margo updates on every key event that toggles the xkb group).
//! Click cycles to the next configured layout via the compositor's
//! `cyclekblayout` dispatch action (`mctl dispatch cyclekblayout`).

use futures::StreamExt;
use mshell_services::{margo_service, tokio_rt_spawn};
use relm4::gtk::Orientation;
use relm4::gtk::prelude::{BoxExt, ButtonExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

pub(crate) struct KeyboardLayoutModel {
    /// Full layout name as reported by xkb (e.g. "English (US)").
    layout: String,
    _orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum KeyboardLayoutInput {
    /// Left click — cycle to the next configured layout.
    Cycle,
}

#[derive(Debug)]
pub(crate) enum KeyboardLayoutOutput {}

pub(crate) struct KeyboardLayoutInit {
    pub(crate) orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum KeyboardLayoutCommandOutput {
    /// The active layout name changed.
    Layout(String),
}

#[relm4::component(pub)]
impl Component for KeyboardLayoutModel {
    type CommandOutput = KeyboardLayoutCommandOutput;
    type Input = KeyboardLayoutInput;
    type Output = KeyboardLayoutOutput;
    type Init = KeyboardLayoutInit;

    view! {
        #[root]
        #[name = "root"]
        gtk::Box {
            add_css_class: "keyboard-layout-bar-widget",
            set_hexpand: model._orientation == Orientation::Vertical,
            set_vexpand: model._orientation == Orientation::Horizontal,
            set_halign: gtk::Align::Center,
            set_valign: gtk::Align::Center,
            set_has_tooltip: true,
            #[watch]
            set_tooltip_text: Some(&tooltip(&model.layout)),

            gtk::Button {
                set_css_classes: &["ok-button-surface", "ok-bar-widget"],
                set_hexpand: false,
                set_vexpand: false,
                connect_clicked[sender] => move |_| {
                    sender.input(KeyboardLayoutInput::Cycle);
                },

                gtk::Box {
                    set_orientation: Orientation::Horizontal,
                    set_spacing: 4,
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,

                    gtk::Image {
                        set_icon_name: Some("input-keyboard-symbolic"),
                    },
                    gtk::Label {
                        add_css_class: "keyboard-layout-bar-label",
                        #[watch]
                        set_label: &short_code(&model.layout),
                        #[watch]
                        set_visible: !model.layout.is_empty(),
                    },
                },
            }
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        // Subscribe to the reactive layout name. `watch()` yields the
        // current value first, then on every change.
        sender.command(|out, shutdown| async move {
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            let mut stream = margo_service().keyboard_layout.watch();
            loop {
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    next = stream.next() => match next {
                        Some(name) => {
                            let _ = out.send(KeyboardLayoutCommandOutput::Layout(name));
                        }
                        None => break,
                    },
                }
            }
        });

        let model = KeyboardLayoutModel {
            layout: margo_service().keyboard_layout.get(),
            _orientation: params.orientation,
        };

        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            KeyboardLayoutInput::Cycle => {
                tokio_rt_spawn(async move {
                    let _ = margo_service().dispatch("dispatch cyclekblayout").await;
                });
            }
        }
    }

    fn update_cmd(
        &mut self,
        message: Self::CommandOutput,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            KeyboardLayoutCommandOutput::Layout(name) => self.layout = name,
        }
    }
}

/// Derive a short pill code from the full xkb layout name. Prefers a
/// parenthetical country code ("English (US)" → "US"); otherwise the
/// first two letters uppercased ("Turkish" → "TU").
fn short_code(name: &str) -> String {
    if name.is_empty() {
        return String::new();
    }
    if let Some(start) = name.rfind('(')
        && let Some(end) = name[start..].find(')')
    {
        let inside = name[start + 1..start + end].trim();
        if !inside.is_empty() {
            return inside.to_uppercase();
        }
    }
    name.chars()
        .filter(|c| c.is_alphabetic())
        .take(2)
        .collect::<String>()
        .to_uppercase()
}

fn tooltip(name: &str) -> String {
    let head = if name.is_empty() {
        "Keyboard layout".to_string()
    } else {
        format!("Keyboard layout: {name}")
    };
    format!("{head}\n\nClick: next layout")
}
