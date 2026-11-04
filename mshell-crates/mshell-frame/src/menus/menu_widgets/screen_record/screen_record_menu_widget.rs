use crate::common_widgets::big_button::BigButton;
use crate::common_widgets::option_list::{OptionsListInput, OptionsListOutput};
use crate::common_widgets::revealer_row::revealer_row::{
    RevealerRowInit, RevealerRowInput, RevealerRowModel,
};
use crate::common_widgets::revealer_row::revealer_row_label::RevealerRowLabelInput::SetLabel;
use crate::common_widgets::revealer_row::revealer_row_label::{
    RevealerRowLabelInit, RevealerRowLabelModel,
};
use crate::menus::menu_widgets::screen_record::audio_option::{
    AudioChoice, AudioOption, AudioOptionsList, get_audio_option_icon_name, get_audio_option_label,
};
use crate::menus::menu_widgets::screen_record::backend_option::{
    BackendOption, BackendOptionsList, backend_icon_name,
};
use crate::menus::menu_widgets::screen_record::recording_service::{
    RecordingState, RecordingStateStoreFields, effective_backend, gsr_installed, preferred_backend,
    recording_state, set_preferred_backend,
};
use mshell_common::scoped_effects::EffectScope;
use mshell_common::watch;
use mshell_screenshot::record::{RecordBackend, RecordHandle, RecordResult};
use mshell_screenshot::{CaptureArea, ScreenRecordRequest};
use mshell_services::audio_service;
use mshell_utils::notifications::show_file_saved_notification;
use notify_rust::Notification;
use reactive_graph::traits::{Get, GetUntracked};
use reactive_stores::Patch;
use relm4::gtk::prelude::{BoxExt, ButtonExt, EventControllerExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentController, ComponentParts, ComponentSender, Controller, gtk};
use std::sync::Arc;
use std::time::Duration;
use wayle_audio::core::device::input::InputDevice;

pub(crate) struct ScreenRecordMenuWidgetModel {
    audio_row: Controller<RevealerRowModel<RevealerRowLabelModel, AudioOptionsList>>,
    selected_audio_option: AudioOption,
    backend_row: Controller<RevealerRowModel<RevealerRowLabelModel, BackendOptionsList>>,
    /// gpu-screen-recorder on PATH; the backend row is hidden without it.
    gsr_available: bool,
    recording_handle: Option<RecordHandle>,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum ScreenRecordMenuWidgetInput {
    AudioOptionSelected(AudioOption),
    BackendSelected(BackendOption),
    AllClicked,
    MonitorClicked,
    WindowClicked,
    AreaClicked,
    StopClicked,
    RecordingHandleChanged(Option<RecordHandle>),
}

#[derive(Debug)]
pub(crate) enum ScreenRecordMenuWidgetOutput {
    CloseMenu,
}

pub(crate) struct ScreenRecordMenuWidgetInit {}

#[derive(Debug)]
pub(crate) enum ScreenRecordMenuWidgetCommandOutput {
    AudioDevicesChanges(Vec<Arc<InputDevice>>),
}

#[relm4::component(pub)]
impl Component for ScreenRecordMenuWidgetModel {
    type CommandOutput = ScreenRecordMenuWidgetCommandOutput;
    type Input = ScreenRecordMenuWidgetInput;
    type Output = ScreenRecordMenuWidgetOutput;
    type Init = ScreenRecordMenuWidgetInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "screen-recording-menu-widget",
            set_hexpand: false,
            set_orientation: gtk::Orientation::Vertical,

            // ── §12 panel header ──
            gtk::Box {
                add_css_class: "panel-header",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 12,
                set_margin_bottom: 8,
                gtk::Image {
                    add_css_class: "panel-header-icon",
                    set_valign: gtk::Align::Center,
                    set_icon_name: Some("media-record-symbolic"),
                },
                gtk::Label {
                    add_css_class: "panel-title",
                    set_label: "Screen Record",
                    set_halign: gtk::Align::Start,
                    set_hexpand: true,
                    set_valign: gtk::Align::Center,
                },
            },

            model.audio_row.widget().clone() {},
            model.backend_row.widget().clone() {
                set_visible: model.gsr_available,
            },

            gtk::Box {
                set_margin_top: 8,
                set_orientation: gtk::Orientation::Vertical,
                set_halign: gtk::Align::Center,
                set_spacing: 16,

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 32,

                    #[template]
                    BigButton {
                        #[watch]
                        set_sensitive: model.recording_handle.is_none(),
                        connect_clicked[sender] => move |_| {
                            sender.input(ScreenRecordMenuWidgetInput::AllClicked);
                        },

                        #[template_child]
                        icon {
                            set_icon_name: Some("screenshot-all-symbolic"),
                        },
                        #[template_child]
                        label {
                            set_label: "All",
                        },
                    },

                    #[template]
                    BigButton {
                        #[watch]
                        set_sensitive: model.recording_handle.is_none(),
                        connect_clicked[sender] => move |_| {
                            sender.input(ScreenRecordMenuWidgetInput::MonitorClicked);
                        },

                        #[template_child]
                        icon {
                            set_icon_name: Some("screenshot-monitor-symbolic"),
                        },
                        #[template_child]
                        label {
                            set_label: "Monitor",
                        },
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 32,

                    #[template]
                    BigButton {
                        #[watch]
                        set_sensitive: model.recording_handle.is_none(),
                        connect_clicked[sender] => move |_| {
                            sender.input(ScreenRecordMenuWidgetInput::WindowClicked);
                        },

                        #[template_child]
                        icon {
                            set_icon_name: Some("screenshot-window-symbolic"),
                        },
                        #[template_child]
                        label {
                            set_label: "Window",
                        },
                    },

                    #[template]
                    BigButton {
                        #[watch]
                        set_sensitive: model.recording_handle.is_none(),
                        connect_clicked[sender] => move |_| {
                            sender.input(ScreenRecordMenuWidgetInput::AreaClicked);
                        },

                        #[template_child]
                        icon {
                            set_icon_name: Some("screenshot-area-symbolic"),
                        },
                        #[template_child]
                        label {
                            set_label: "Area",
                        },
                    },
                },
            },

            gtk::Box {
                add_css_class: "screen-recording-menu-indicator-box",
                set_orientation: gtk::Orientation::Vertical,
                set_margin_top: 24,
                #[watch]
                set_visible: model.recording_handle.is_some(),

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,

                    gtk::Image {
                        add_css_class: "screen-recording-menu-indicator-image",
                        set_icon_name: Some("record-symbolic"),
                    },

                    gtk::Label {
                        add_css_class: "label-medium-bold",
                        set_margin_start: 12,
                        set_label: "Recording screen…",
                    },
                },

                #[name = "stop_button"]
                gtk::Button {
                    add_css_class: "ok-button-primary",
                    set_margin_top: 12,
                    set_valign: gtk::Align::Center,
                    connect_clicked[sender] => move |_| {
                        sender.input(ScreenRecordMenuWidgetInput::StopClicked);
                    },

                    gtk::Label {
                        add_css_class: "label-medium",
                        set_label: "Stop",
                    },
                }
            },

            // Keyboard-shortcut hint (the keys are wired in `init`).
            gtk::Label {
                add_css_class: "label-small",
                add_css_class: "dim-label",
                set_label: "A all · M monitor · W window · R area · S stop · Esc close",
                set_halign: gtk::Align::Center,
                set_margin_top: 12,
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        Self::spawn_default_output_watcher(&sender);

        let content = RevealerRowLabelModel::builder()
            .launch(RevealerRowLabelInit {
                label: "No Audio".to_string(),
            })
            .detach();

        let revealed_content = AudioOptionsList::builder()
            .launch(fixed_audio_options())
            .forward(sender.input_sender(), |msg| match msg {
                OptionsListOutput::Selected(opt) => {
                    ScreenRecordMenuWidgetInput::AudioOptionSelected(opt)
                }
            });

        let audio_row = RevealerRowModel::<RevealerRowLabelModel, AudioOptionsList>::builder()
            .launch(RevealerRowInit {
                icon_name: "audio-volume-muted-symbolic".into(),
                action_button_sensitive: false,
                content,
                revealed_content,
            })
            .detach();

        let backend = preferred_backend();
        let backend_content = RevealerRowLabelModel::builder()
            .launch(RevealerRowLabelInit {
                label: backend.label().to_string(),
            })
            .detach();
        let backend_options = BackendOptionsList::builder()
            .launch(
                RecordBackend::ALL
                    .iter()
                    .map(|b| BackendOption { value: *b })
                    .collect(),
            )
            .forward(sender.input_sender(), |msg| match msg {
                OptionsListOutput::Selected(opt) => {
                    ScreenRecordMenuWidgetInput::BackendSelected(opt)
                }
            });
        let backend_row = RevealerRowModel::<RevealerRowLabelModel, BackendOptionsList>::builder()
            .launch(RevealerRowInit {
                icon_name: backend_icon_name(backend).into(),
                action_button_sensitive: false,
                content: backend_content,
                revealed_content: backend_options,
            })
            .detach();

        let mut effects = EffectScope::new();

        let recording = recording_state();

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let recording_state = recording.clone();
            let handle = recording_state.handle().get();
            sender_clone.input(ScreenRecordMenuWidgetInput::RecordingHandleChanged(handle));
        });

        let recording_handle = recording.clone().handle().get_untracked();

        let model = ScreenRecordMenuWidgetModel {
            audio_row,
            selected_audio_option: AudioOption {
                value: AudioChoice::None,
            },
            backend_row,
            gsr_available: gsr_installed(),
            recording_handle,
            _effects: effects,
        };

        let widgets = view_output!();

        // Keyboard shortcuts while the menu is open (DMS-style): mnemonic
        // keys per capture target + Escape to dismiss. While recording, the
        // capture keys are ignored (the buttons are insensitive too) and
        // S / Space stop the recording.
        {
            let sender = sender.clone();
            let kc = gtk::EventControllerKey::new();
            kc.set_propagation_phase(gtk::PropagationPhase::Capture);
            kc.connect_key_pressed(move |_, keyval, _, _| {
                use relm4::gtk::glib::Propagation;
                if keyval == gtk::gdk::Key::Escape {
                    let _ = sender.output(ScreenRecordMenuWidgetOutput::CloseMenu);
                    return Propagation::Stop;
                }
                if is_recording() {
                    if keyval == gtk::gdk::Key::space
                        || matches!(keyval.to_unicode(), Some('s' | 'S'))
                    {
                        sender.input(ScreenRecordMenuWidgetInput::StopClicked);
                        return Propagation::Stop;
                    }
                    return Propagation::Proceed;
                }
                let input = match keyval.to_unicode() {
                    Some('a' | 'A') => Some(ScreenRecordMenuWidgetInput::AllClicked),
                    Some('m' | 'M') => Some(ScreenRecordMenuWidgetInput::MonitorClicked),
                    Some('w' | 'W') => Some(ScreenRecordMenuWidgetInput::WindowClicked),
                    Some('r' | 'R') => Some(ScreenRecordMenuWidgetInput::AreaClicked),
                    _ => None,
                };
                if let Some(i) = input {
                    sender.input(i);
                    Propagation::Stop
                } else {
                    Propagation::Proceed
                }
            });
            root.add_controller(kc);
        }

        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        let sender_clone = sender.clone();
        match message {
            ScreenRecordMenuWidgetInput::AudioOptionSelected(opt) => {
                self.selected_audio_option = opt.clone();
                self.audio_row
                    .model()
                    .content
                    .emit(SetLabel(get_audio_option_label(&opt)));
                self.audio_row.emit(RevealerRowInput::UpdateActionIconName(
                    get_audio_option_icon_name(&opt),
                ));
                self.audio_row.emit(RevealerRowInput::SetRevealed(false));
            }
            ScreenRecordMenuWidgetInput::BackendSelected(opt) => {
                set_preferred_backend(opt.value);
                self.backend_row
                    .model()
                    .content
                    .emit(SetLabel(opt.value.label().to_string()));
                self.backend_row
                    .emit(RevealerRowInput::UpdateActionIconName(
                        backend_icon_name(opt.value).to_string(),
                    ));
                self.backend_row.emit(RevealerRowInput::SetRevealed(false));
            }
            ScreenRecordMenuWidgetInput::AllClicked => self.start_record(CaptureArea::All, sender),
            ScreenRecordMenuWidgetInput::WindowClicked => {
                self.start_record(CaptureArea::SelectWindow, sender)
            }
            ScreenRecordMenuWidgetInput::MonitorClicked => {
                self.start_record(CaptureArea::SelectMonitor, sender)
            }
            ScreenRecordMenuWidgetInput::AreaClicked => {
                self.start_record(CaptureArea::SelectRegion, sender)
            }
            ScreenRecordMenuWidgetInput::StopClicked => {
                if let Some(handle) = &self.recording_handle {
                    handle.stop();
                }
            }
            ScreenRecordMenuWidgetInput::RecordingHandleChanged(handle) => {
                self.recording_handle = handle;
            }
        }

        self.update_view(widgets, sender_clone);
    }

    fn update_cmd(
        &mut self,
        message: Self::CommandOutput,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            ScreenRecordMenuWidgetCommandOutput::AudioDevicesChanges(devices) => {
                let mut options = fixed_audio_options();
                options.extend(devices.iter().map(|d| AudioOption {
                    value: AudioChoice::Device(d.clone()),
                }));
                let devices = options;

                self.audio_row
                    .model()
                    .revealed_content
                    .emit(OptionsListInput::SetOptions(devices))
            }
        }
    }
}

impl ScreenRecordMenuWidgetModel {
    fn spawn_default_output_watcher(sender: &ComponentSender<Self>) {
        // No audio service: no input devices to offer.
        let Some(audio) = audio_service() else {
            return;
        };
        let input_devices = audio.input_devices.clone();

        watch!(sender, [input_devices.watch()], |out| {
            let _ = out.send(ScreenRecordMenuWidgetCommandOutput::AudioDevicesChanges(
                input_devices.get(),
            ));
        });
    }

    fn start_record(&self, area: CaptureArea, sender: ComponentSender<Self>) {
        if self.recording_handle.is_some() {
            return;
        }
        let _ = sender.output(ScreenRecordMenuWidgetOutput::CloseMenu);
        record_start(area, self.selected_audio_option.spec(), None);
    }
}

/// Audio choices that exist regardless of the device list: silent, and
/// desktop + microphone mixed.
fn fixed_audio_options() -> Vec<AudioOption> {
    vec![
        AudioOption {
            value: AudioChoice::None,
        },
        AudioOption {
            value: AudioChoice::DesktopAndMic,
        },
    ]
}

/// End of a clip, success or not: clear the shared recording state (so
/// the menu and the bar pill leave "recording" even when the recorder
/// died) and tell the user where the file is or why it failed.
fn finish_recording<E: std::fmt::Display>(result: Result<RecordResult, E>) {
    recording_state().patch(RecordingState { handle: None });
    match result {
        Ok(RecordResult {
            saved_path: Some(path),
        }) => show_file_saved_notification("Screen recording saved".to_string(), path),
        Ok(_) => {}
        Err(e) => {
            tracing::warn!(error = %e, "screen recording failed");
            Notification::new()
                .summary("Screen recording failed")
                .body(&format!("{e:#}"))
                .appname("mshell")
                .show()
                .ok();
        }
    }
}

// ── Headless recording controls (driven by `mshellctl screenrecord`) ───────
// Same engine + shared `recording_state` the menu uses; no menu surface.

pub(crate) fn is_recording() -> bool {
    recording_state().clone().handle().get_untracked().is_some()
}

/// Start a clip. `backend` = an explicit choice (`mshellctl screenrecord
/// --backend`), `None` = the menu's current choice.
pub(crate) fn record_start(
    area: CaptureArea,
    audio: Option<String>,
    backend: Option<RecordBackend>,
) {
    if is_recording() {
        return;
    }
    let backend = effective_backend(backend, preferred_backend(), gsr_installed());
    mshell_screenshot::record_screen(
        ScreenRecordRequest {
            area,
            audio,
            backend,
        },
        Duration::ZERO,
        |handle_result| match handle_result {
            Ok(handle) => recording_state().patch(RecordingState {
                handle: Some(handle),
            }),
            // Selection cancelled or no outputs: nothing started.
            Err(e) => tracing::debug!(error = %e, "screen recording not started"),
        },
        // The recorder thread calls this off the GTK thread; the store
        // patch and the notification must run on it.
        |done_result| {
            relm4::gtk::glib::MainContext::default().invoke(move || finish_recording(done_result))
        },
    );
}

pub(crate) fn record_stop() {
    if let Some(handle) = recording_state().clone().handle().get_untracked() {
        handle.stop();
    }
    // The recorder's `on_done` patches the handle back to None once it has
    // finished flushing the file.
}

pub(crate) fn record_toggle(
    area: CaptureArea,
    audio: Option<String>,
    backend: Option<RecordBackend>,
) {
    if is_recording() {
        record_stop();
    } else {
        record_start(area, audio, backend);
    }
}
