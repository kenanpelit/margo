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

            model.audio_out.widget().clone() {},
            model.audio_in.widget().clone() {},
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

        let model = AudioDashboardMenuWidgetModel { audio_out, audio_in };
        let widgets = view_output!();
        let _ = (root, sender);
        ComponentParts { model, widgets }
    }
}
