//! One per-application volume row in the app mixer: app icon + name +
//! a live volume slider + a mute button. The stream analogue of the
//! device volume rows — drives `wayle_audio`'s `AudioStream` directly
//! (`set_volume` / `set_mute`), with a per-stream volume/mute watcher
//! so external changes (the app's own controls, `pavucontrol`) reflect
//! here too.

use mshell_common::WatcherToken;
use mshell_utils::audio::{
    get_stream_volume_icon, set_app_stream_icon, spawn_stream_volume_mute_watcher,
    stream_display_name,
};
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use wayle_audio::core::stream::AudioStream;
use wayle_audio::volume::types::Volume;

pub(crate) struct AppVolumeRowModel {
    stream: Arc<AudioStream>,
    /// Mic-capture row → mic glyph fallback; playback → app glyph.
    recording: bool,
    /// Set while we update the slider programmatically so the
    /// value-changed handler doesn't echo an external change back to
    /// PulseAudio as a fake user drag.
    suppress: Rc<Cell<bool>>,
    watcher_token: WatcherToken,
}

#[derive(Debug)]
pub(crate) enum AppVolumeRowInput {
    SetVolume(f64),
    MuteClicked,
    Refresh,
}

#[derive(Debug)]
pub(crate) enum AppVolumeRowOutput {}

pub(crate) struct AppVolumeRowInit {
    pub stream: Arc<AudioStream>,
    pub recording: bool,
}

#[derive(Debug)]
pub(crate) enum AppVolumeRowCommandOutput {
    Changed,
}

#[relm4::component(pub)]
impl Component for AppVolumeRowModel {
    type CommandOutput = AppVolumeRowCommandOutput;
    type Input = AppVolumeRowInput;
    type Output = AppVolumeRowOutput;
    type Init = AppVolumeRowInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "app-mixer-row",
            set_orientation: gtk::Orientation::Horizontal,
            set_spacing: 8,
            set_valign: gtk::Align::Center,

            #[name = "app_icon"]
            gtk::Image {
                add_css_class: "app-mixer-icon",
            },

            gtk::Label {
                add_css_class: "app-mixer-name",
                #[watch]
                set_label: &stream_display_name(&model.stream),
                set_xalign: 0.0,
                set_width_chars: 10,
                set_max_width_chars: 12,
                set_ellipsize: gtk::pango::EllipsizeMode::End,
            },

            #[name = "scale"]
            gtk::Scale {
                add_css_class: "ok-progress-bar",
                set_hexpand: true,
                set_can_focus: false,
                set_focus_on_click: false,
                set_range: (0.0, 1.0),
                #[watch]
                set_sensitive: model.stream.volume_writable.get(),
            },

            gtk::Button {
                add_css_class: "ok-button-surface",
                set_valign: gtk::Align::Center,
                connect_clicked[sender] => move |_| {
                    sender.input(AppVolumeRowInput::MuteClicked);
                },

                gtk::Image {
                    #[watch]
                    set_icon_name: Some(get_stream_volume_icon(&model.stream)),
                },
            },
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let stream = params.stream;

        let mut watcher_token = WatcherToken::new();
        let token = watcher_token.reset();
        spawn_stream_volume_mute_watcher(&stream, token, &sender, || {
            AppVolumeRowCommandOutput::Changed
        });

        let model = AppVolumeRowModel {
            stream,
            recording: params.recording,
            suppress: Rc::new(Cell::new(false)),
            watcher_token,
        };

        let widgets = view_output!();

        // App identity (name/icon) is fixed for a stream's lifetime, so resolve
        // the icon once here rather than on every volume tick — the `.desktop`
        // lookup can scan every installed app. This shows the real application
        // icon (browsers, Electron, …) instead of a generic executable glyph
        // whenever the stream itself carries no `application.icon_name`.
        set_app_stream_icon(
            &model.stream,
            &widgets.app_icon,
            if model.recording {
                "audio-input-microphone-symbolic"
            } else {
                "application-x-executable-symbolic"
            },
        );

        // Connect the drag handler now that the scale exists; the
        // `suppress` flag gates programmatic updates.
        {
            let sender = sender.clone();
            let suppress = model.suppress.clone();
            widgets.scale.connect_value_changed(move |scale| {
                if !suppress.get() {
                    sender.input(AppVolumeRowInput::SetVolume(scale.value()));
                }
            });
        }

        model.suppress.set(true);
        widgets.scale.set_value(model.stream.volume.get().average());
        model.suppress.set(false);

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
            AppVolumeRowInput::SetVolume(value) => {
                let stream = self.stream.clone();
                let channels = stream.volume.get().channels().max(1);
                tokio::spawn(async move {
                    let _ = stream
                        .set_volume(Volume::from_percentage(value * 100.0, channels))
                        .await;
                });
            }
            AppVolumeRowInput::MuteClicked => {
                let stream = self.stream.clone();
                tokio::spawn(async move {
                    let mute = !stream.muted.get();
                    let _ = stream.set_mute(mute).await;
                });
            }
            AppVolumeRowInput::Refresh => {
                self.suppress.set(true);
                widgets.scale.set_value(self.stream.volume.get().average());
                self.suppress.set(false);
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
            AppVolumeRowCommandOutput::Changed => {
                sender.input(AppVolumeRowInput::Refresh);
            }
        }
    }
}

impl Drop for AppVolumeRowModel {
    fn drop(&mut self) {
        self.watcher_token.reset();
    }
}
