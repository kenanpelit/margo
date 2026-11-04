//! The source side of the shared audio device row
//! ([`crate::menus::menu_widgets::audio_device_revealer_button`]).

use crate::menus::menu_widgets::audio_device_revealer_button::{
    AudioDeviceKind, DeviceRevealerButtonCommandOutput, DeviceRevealerButtonInit,
    DeviceRevealerButtonModel,
};
use mshell_services::audio_service;
use mshell_utils::audio::{get_audio_in_icon_device_aware, spawn_default_input_watcher};
use relm4::ComponentSender;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use wayle_audio::core::device::input::InputDevice;

pub(crate) use crate::menus::menu_widgets::audio_device_revealer_button::DeviceRevealerButtonInput as InputDeviceRevealerButtonInput;

pub(crate) struct InputKind;

pub(crate) type InputDeviceRevealerButtonModel = DeviceRevealerButtonModel<InputKind>;
pub(crate) type InputDeviceRevealerButtonInit = DeviceRevealerButtonInit<InputKind>;

impl AudioDeviceKind for InputKind {
    type Device = InputDevice;

    fn name(device: &InputDevice) -> String {
        device.name.get()
    }

    fn description(device: &InputDevice) -> String {
        device.description.get()
    }

    fn icon(device: &Arc<InputDevice>) -> &'static str {
        get_audio_in_icon_device_aware(device)
    }

    fn is_default(device: &Arc<InputDevice>) -> bool {
        audio_service()
            .and_then(|a| a.default_input.get())
            .map(|d| d.eq(device))
            .unwrap_or(false)
    }

    fn spawn_default_watcher(
        sender: &ComponentSender<DeviceRevealerButtonModel<Self>>,
        token: CancellationToken,
    ) {
        spawn_default_input_watcher(sender, Some(token), || {
            DeviceRevealerButtonCommandOutput::DefaultDeviceChanged
        });
    }

    fn make_default(device: Arc<InputDevice>) {
        tokio::spawn(async move {
            let _ = device.set_as_default().await;
        });
    }
}
