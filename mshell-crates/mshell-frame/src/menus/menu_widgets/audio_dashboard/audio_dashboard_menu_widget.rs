//! Audio Dashboard menu widget — the right-pane content for
//! `MenuType::AudioDashboard`.
//!
//! Composes the existing AudioOutput + AudioInput menu widgets,
//! each a collapsible revealer row (mute button + volume slider,
//! click the chevron to expand the device picker underneath) —
//! the same design language as the Bluetooth menu. Hosting the
//! two ready-made components here means the dashboard menu reuses
//! all their wayle_audio plumbing (default-device tracking,
//! per-device volume/mute watchers, device-list rebuild) instead
//! of duplicating it.

use crate::menus::menu_widgets::audio_dashboard::app_mixer::{AppMixerInit, AppMixerModel};
use crate::menus::menu_widgets::audio_dashboard::port_switcher::{
    PortSwitcherInit, PortSwitcherModel,
};
use crate::menus::menu_widgets::audio_in::audio_in_menu_widget::{
    AudioInMenuWidgetInit, AudioInMenuWidgetModel,
};
use crate::menus::menu_widgets::audio_out::audio_out_menu_widget::{
    AudioOutMenuWidgetInit, AudioOutMenuWidgetModel,
};
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentController, ComponentParts, ComponentSender, Controller, gtk};

pub(crate) struct AudioDashboardMenuWidgetModel {
    audio_out: Controller<AudioOutMenuWidgetModel>,
    audio_in: Controller<AudioInMenuWidgetModel>,
    out_ports: Controller<PortSwitcherModel>,
    in_ports: Controller<PortSwitcherModel>,
    app_mixer: Controller<AppMixerModel>,
    recording_mixer: Controller<AppMixerModel>,
}

impl std::fmt::Debug for AudioDashboardMenuWidgetModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioDashboardMenuWidgetModel").finish()
    }
}

#[derive(Debug)]
pub(crate) enum AudioDashboardMenuWidgetInput {}

#[derive(Debug)]
pub(crate) enum AudioDashboardMenuWidgetOutput {}

pub(crate) struct AudioDashboardMenuWidgetInit {}

#[relm4::component(pub(crate))]
impl Component for AudioDashboardMenuWidgetModel {
    type CommandOutput = ();
    type Input = AudioDashboardMenuWidgetInput;
    type Output = AudioDashboardMenuWidgetOutput;
    type Init = AudioDashboardMenuWidgetInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "audio-dashboard-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 10,

            // ── §12 panel header ────────────────────────────────
            gtk::Box {
                add_css_class: "panel-header",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 12,
                gtk::Image {
                    add_css_class: "panel-header-icon",
                    set_icon_name: Some("audio-volume-high-symbolic"),
                    set_valign: gtk::Align::Center,
                },
                gtk::Label {
                    add_css_class: "panel-title",
                    set_label: "Audio",
                    set_halign: gtk::Align::Start,
                    set_hexpand: true,
                    set_valign: gtk::Align::Center,
                },
            },

            gtk::Label {
                add_css_class: "audio-dashboard-section-label",
                set_label: "OUTPUT",
                set_halign: gtk::Align::Start,
            },
            model.audio_out.widget().clone() {},
            // Output route chips (Speakers ↔ Headphones …) — hidden
            // unless the default sink exposes ≥2 ports.
            model.out_ports.widget().clone() {},

            gtk::Label {
                add_css_class: "audio-dashboard-section-label",
                set_label: "INPUT",
                set_halign: gtk::Align::Start,
            },
            model.audio_in.widget().clone() {},
            model.in_ports.widget().clone() {},

            // Per-app mixers (QSAP-style). Each hides itself when no
            // stream is active, so they add no chrome when idle.
            model.app_mixer.widget().clone() {},
            model.recording_mixer.widget().clone() {},
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let audio_out = AudioOutMenuWidgetModel::builder()
            .launch(AudioOutMenuWidgetInit {})
            .detach();
        let audio_in = AudioInMenuWidgetModel::builder()
            .launch(AudioInMenuWidgetInit {})
            .detach();
        let out_ports = PortSwitcherModel::builder()
            .launch(PortSwitcherInit { recording: false })
            .detach();
        let in_ports = PortSwitcherModel::builder()
            .launch(PortSwitcherInit { recording: true })
            .detach();
        let app_mixer = AppMixerModel::builder()
            .launch(AppMixerInit { recording: false })
            .detach();
        let recording_mixer = AppMixerModel::builder()
            .launch(AppMixerInit { recording: true })
            .detach();

        let model = AudioDashboardMenuWidgetModel {
            audio_out,
            audio_in,
            out_ports,
            in_ports,
            app_mixer,
            recording_mixer,
        };
        let widgets = view_output!();
        let _ = (root, sender);
        ComponentParts { model, widgets }
    }
}
