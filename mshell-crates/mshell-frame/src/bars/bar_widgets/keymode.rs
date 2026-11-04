//! Keymode — bar pill for margo's active keymode (`setkeymode`).
//!
//! Shows the keymode name (e.g. "resize") while a non-default keymode is
//! active and hides itself in the default mode, so it is only on screen
//! while it carries information. Render-only and reactive: the mode comes
//! from `margo_service().keymode`, mirrored from the `keymode` field margo
//! pushes on every change. Click returns to the default keymode
//! (`setkeymode default`) — the escape hatch when a mode's exit bind is
//! forgotten. The change toast lives in the shell's single toast producer
//! (`toasts.keymode`), not here, so multi-monitor bars don't toast N times.

use futures::StreamExt;
use mshell_margo_client::indicators::{DEFAULT_KEYMODE, visible_keymode};
use mshell_services::margo_service;
use relm4::gtk::Orientation;
use relm4::gtk::prelude::{BoxExt, ButtonExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

pub(crate) struct KeymodeModel {
    /// The mode as margo reports it; `None` before the first snapshot or on
    /// a margo that predates the field.
    mode: Option<String>,
    _orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum KeymodeInput {
    /// Left click — back to the default keymode.
    Reset,
}

#[derive(Debug)]
pub(crate) enum KeymodeOutput {}

pub(crate) struct KeymodeInit {
    pub(crate) orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum KeymodeCommandOutput {
    Mode(Option<String>),
}

#[relm4::component(pub)]
impl Component for KeymodeModel {
    type CommandOutput = KeymodeCommandOutput;
    type Input = KeymodeInput;
    type Output = KeymodeOutput;
    type Init = KeymodeInit;

    view! {
        #[root]
        #[name = "root"]
        gtk::Box {
            add_css_class: "keymode-bar-widget",
            set_hexpand: model._orientation == Orientation::Vertical,
            set_vexpand: model._orientation == Orientation::Horizontal,
            set_halign: gtk::Align::Center,
            set_valign: gtk::Align::Center,
            #[watch]
            set_visible: visible_keymode(model.mode.as_deref()).is_some(),
            set_has_tooltip: true,
            #[watch]
            set_tooltip_text: Some(&tooltip(model.mode.as_deref())),

            gtk::Button {
                set_css_classes: &["ok-button-surface", "ok-bar-widget"],
                set_hexpand: false,
                set_vexpand: false,
                connect_clicked[sender] => move |_| {
                    sender.input(KeymodeInput::Reset);
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
                        add_css_class: "keymode-bar-label",
                        #[watch]
                        set_label: visible_keymode(model.mode.as_deref()).unwrap_or_default(),
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
        // `watch()` yields the current value first, then every change.
        sender.command(|out, shutdown| async move {
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            let mut stream = margo_service().keymode.watch();
            loop {
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    next = stream.next() => match next {
                        Some(mode) => {
                            let _ = out.send(KeymodeCommandOutput::Mode(mode));
                        }
                        None => break,
                    },
                }
            }
        });

        let model = KeymodeModel {
            mode: margo_service().keymode.get(),
            _orientation: params.orientation,
        };

        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            KeymodeInput::Reset => {
                // A single non-numeric dispatch arg fills `v` (the mode).
                mshell_margo_client::ctl::dispatch_detached("setkeymode", &[DEFAULT_KEYMODE]);
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
            KeymodeCommandOutput::Mode(mode) => self.mode = mode,
        }
    }
}

fn tooltip(mode: Option<&str>) -> String {
    match visible_keymode(mode) {
        Some(m) => format!("Keymode: {m}\n\nClick: back to the default keymode"),
        None => "Keymode".to_string(),
    }
}
