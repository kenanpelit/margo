//! Dark-mode toggle bar pill.
//!
//! Single-button widget that flips `theme.matugen.mode` between
//! `Light` and `Dark`. The icon reflects the *current* mode (sun
//! when dark, moon when light — the icon previews what you'll
//! switch *to*), so a glance at the pill tells you both the
//! current state and the action.
//!
//! Persists through `config_manager().update_config` so matugen
//! re-runs and the rest of the shell repaints automatically.

use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    ConfigStoreFields, MatugenStoreFields, ThemeStoreFields,
};
use mshell_config::schema::themes::MatugenMode;
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::Orientation;
use relm4::gtk::prelude::{ButtonExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

pub(crate) struct DarkModeModel {
    /// Live tracked mode. The view's icon is bound to this.
    mode: MatugenMode,
    _orientation: Orientation,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum DarkModeInput {
    Clicked,
    /// Reactive mirror of the matugen-mode store. Emitted by the
    /// effect spawned in `init` whenever the config changes.
    SetMode(MatugenMode),
}

#[derive(Debug)]
pub(crate) enum DarkModeOutput {}

pub(crate) struct DarkModeInit {
    pub(crate) orientation: Orientation,
}

#[relm4::component(pub)]
impl Component for DarkModeModel {
    type CommandOutput = ();
    type Input = DarkModeInput;
    type Output = DarkModeOutput;
    type Init = DarkModeInit;

    view! {
        #[root]
        #[name = "root"]
        gtk::Box {
            add_css_class: "dark-mode-bar-widget",
            set_hexpand: model._orientation == Orientation::Vertical,
            set_vexpand: model._orientation == Orientation::Horizontal,
            set_halign: gtk::Align::Center,
            set_valign: gtk::Align::Center,

            gtk::Button {
                set_css_classes: &["ok-button-surface", "ok-bar-widget"],
                set_hexpand: false,
                set_vexpand: false,
                connect_clicked[sender] => move |_| {
                    sender.input(DarkModeInput::Clicked);
                },

                gtk::Image {
                    set_hexpand: true,
                    set_vexpand: true,
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,
                    #[watch]
                    set_icon_name: Some(match model.mode {
                        // Show the icon for the *target* mode — the
                        // action you'd take by clicking — not the
                        // current one. Mirrors noctalia's pattern.
                        MatugenMode::Dark => "weather-clear-symbolic",
                        MatugenMode::Light => "weather-clear-night-symbolic",
                    }),
                }
            }
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let mut effects = EffectScope::new();

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let mode = config_manager()
                .config()
                .theme()
                .matugen()
                .mode()
                .get();
            sender_clone.input(DarkModeInput::SetMode(mode));
        });

        let model = DarkModeModel {
            mode: config_manager()
                .config()
                .theme()
                .matugen()
                .mode()
                .get_untracked(),
            _orientation: params.orientation,
            _effects: effects,
        };

        let widgets = view_output!();

        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        message: Self::Input,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            DarkModeInput::Clicked => {
                config_manager().update_config(|config| {
                    config.theme.matugen.mode = match config.theme.matugen.mode {
                        MatugenMode::Light => MatugenMode::Dark,
                        MatugenMode::Dark => MatugenMode::Light,
                    };
                });
            }
            DarkModeInput::SetMode(mode) => {
                self.mode = mode;
            }
        }
    }
}
