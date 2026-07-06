//! Per-application volume mixer section (QSAP "application mixer").
//!
//! Watches `wayle_audio`'s `playback_streams` (or `recording_streams`
//! for the mic variant) and renders one [`AppVolumeRowModel`] per
//! active stream. The whole section hides when nothing is playing /
//! capturing, so it adds no empty chrome to the audio dashboard.
//!
//! Rows are full child components (each owning its slider + per-stream
//! watcher); the list is small and changes only when an app starts or
//! stops, so a clear-and-relaunch rebuild is the right tool (no
//! factory / virtualization needed).

use crate::menus::menu_widgets::audio_dashboard::app_volume_row::{
    AppVolumeRowInit, AppVolumeRowModel,
};
use mshell_services::audio_service;
use mshell_utils::audio::{spawn_playback_streams_watcher, spawn_recording_streams_watcher};
use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentController, ComponentParts, ComponentSender, Controller, gtk};

pub(crate) struct AppMixerModel {
    recording: bool,
    rows: Vec<Controller<AppVolumeRowModel>>,
    has_streams: bool,
}

#[derive(Debug)]
pub(crate) enum AppMixerInput {
    StreamsChanged,
}

#[derive(Debug)]
pub(crate) enum AppMixerOutput {}

pub(crate) struct AppMixerInit {
    /// `true` → mic-capture streams (`recording_streams`); `false` →
    /// playback streams.
    pub recording: bool,
}

#[derive(Debug)]
pub(crate) enum AppMixerCommandOutput {
    StreamsChanged,
}

#[relm4::component(pub)]
impl Component for AppMixerModel {
    type CommandOutput = AppMixerCommandOutput;
    type Input = AppMixerInput;
    type Output = AppMixerOutput;
    type Init = AppMixerInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "app-mixer-section",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 6,
            #[watch]
            set_visible: model.has_streams,

            gtk::Label {
                add_css_class: "audio-dashboard-section-label",
                set_halign: gtk::Align::Start,
                set_label: if model.recording { "RECORDING" } else { "APPLICATIONS" },
            },

            #[name = "rows_box"]
            gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 6,
            },
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        if params.recording {
            spawn_recording_streams_watcher(&sender, || AppMixerCommandOutput::StreamsChanged);
        } else {
            spawn_playback_streams_watcher(&sender, || AppMixerCommandOutput::StreamsChanged);
        }

        let model = AppMixerModel {
            recording: params.recording,
            rows: Vec::new(),
            has_streams: false,
        };

        let widgets = view_output!();

        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            AppMixerInput::StreamsChanged => {
                // Tear down old rows (each Controller's drop stops its
                // per-stream watcher), then relaunch one per stream.
                while let Some(child) = widgets.rows_box.first_child() {
                    widgets.rows_box.remove(&child);
                }
                self.rows.clear();

                let streams = if self.recording {
                    audio_service().recording_streams.get()
                } else {
                    audio_service().playback_streams.get()
                };

                for stream in streams {
                    let row = AppVolumeRowModel::builder()
                        .launch(AppVolumeRowInit {
                            stream,
                            recording: self.recording,
                        })
                        .detach();
                    widgets.rows_box.append(row.widget());
                    self.rows.push(row);
                }

                self.has_streams = !self.rows.is_empty();
            }
        }

        self.update_view(widgets, sender);
    }

    fn update_cmd(
        &mut self,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            AppMixerCommandOutput::StreamsChanged => {
                sender.input(AppMixerInput::StreamsChanged);
            }
        }
    }
}
