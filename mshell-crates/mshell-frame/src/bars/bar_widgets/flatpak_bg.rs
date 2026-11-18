//! Flatpak Background — bar pill for [`crate::flatpak_bg`]: a package
//! glyph and how many Flatpak apps run without a window. Hidden while
//! there are none, and always when `flatpak` is not installed. Hover lists
//! them; a click opens the menu, where each can be stopped.

use crate::flatpak_bg::{self, FlatpakBgState};
use relm4::gtk::Orientation;
use relm4::gtk::prelude::{BoxExt, ButtonExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, RelmWidgetExt, gtk};

pub(crate) struct FlatpakBackgroundModel {
    label: String,
    tooltip: String,
    active: bool,
    visible: bool,
    _orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum FlatpakBackgroundInput {
    Clicked,
}

#[derive(Debug)]
pub(crate) enum FlatpakBackgroundOutput {
    Clicked,
}

pub(crate) struct FlatpakBackgroundInit {
    pub(crate) orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum FlatpakBackgroundCommandOutput {
    State(FlatpakBgState),
}

/// Pill label (the count) and tooltip.
pub(crate) fn pill_text(s: &FlatpakBgState) -> (String, String) {
    let n = s.apps.len();
    let mut tip = match n {
        0 => "No Flatpak app runs in the background".to_string(),
        1 => "1 Flatpak app runs without a window".to_string(),
        n => format!("{n} Flatpak apps run without a window"),
    };
    for a in s.apps.iter().take(8) {
        tip.push_str(&format!("\n{}", a.name));
    }
    tip.push_str("\n\nClick: stop them from the menu");
    (n.to_string(), tip)
}

#[relm4::component(pub)]
impl Component for FlatpakBackgroundModel {
    type CommandOutput = FlatpakBackgroundCommandOutput;
    type Input = FlatpakBackgroundInput;
    type Output = FlatpakBackgroundOutput;
    type Init = FlatpakBackgroundInit;

    view! {
        #[root]
        gtk::Box {
            set_css_classes: &["flatpak-bg-bar-widget", "ok-button-surface", "ok-bar-widget"],
            #[watch]
            set_class_active: ("active", model.active),
            set_hexpand: model._orientation == Orientation::Vertical,
            set_vexpand: model._orientation == Orientation::Horizontal,
            set_halign: gtk::Align::Center,
            set_valign: gtk::Align::Center,
            #[watch]
            set_visible: model.visible,
            set_has_tooltip: true,
            #[watch]
            set_tooltip_text: Some(&model.tooltip),

            gtk::Button {
                set_css_classes: &["ok-button-flat"],
                set_hexpand: false,
                set_vexpand: false,
                connect_clicked[sender] => move |_| {
                    sender.input(FlatpakBackgroundInput::Clicked);
                },

                gtk::Box {
                    set_orientation: Orientation::Horizontal,
                    set_spacing: 4,
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,

                    gtk::Image {
                        set_icon_name: Some("package-x-generic-symbolic"),
                    },
                    gtk::Label {
                        add_css_class: "flatpak-bg-bar-label",
                        #[watch]
                        set_label: &model.label,
                        #[watch]
                        set_visible: !model.label.is_empty()
                            && model._orientation == Orientation::Horizontal,
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
        let mut rx = flatpak_bg::subscribe();
        sender.command(|out, shutdown| async move {
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            loop {
                let state = rx.borrow_and_update().clone();
                let _ = out.send(FlatpakBackgroundCommandOutput::State(state));
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    r = rx.changed() => if r.is_err() { break },
                }
            }
        });
        let model = FlatpakBackgroundModel {
            label: String::new(),
            tooltip: String::new(),
            active: false,
            visible: false,
            _orientation: params.orientation,
        };
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            FlatpakBackgroundInput::Clicked => {
                let _ = sender.output(FlatpakBackgroundOutput::Clicked);
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
            FlatpakBackgroundCommandOutput::State(s) => {
                let (label, tooltip) = pill_text(&s);
                self.visible = s.available && !s.apps.is_empty();
                self.active = self.visible;
                self.label = label;
                self.tooltip = tooltip;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flatpak_bg::BgApp;

    #[test]
    fn the_pill_counts_and_lists_apps() {
        let s = FlatpakBgState {
            available: true,
            apps: vec![BgApp {
                id: "org.x.Sync".into(),
                name: "Sync".into(),
            }],
            error: None,
        };
        let (label, tip) = pill_text(&s);
        assert_eq!(label, "1");
        assert!(tip.starts_with("1 Flatpak app runs without a window\nSync"));
    }
}
