//! The sink side of the shared audio device row
//! ([`crate::menus::menu_widgets::audio_device_revealer_button`]): default
//! output tracking, stream migration on switch, and combine-sink groups.

use crate::menus::menu_widgets::audio_device_revealer_button::{
    AudioDeviceKind, DeviceRevealerButtonCommandOutput, DeviceRevealerButtonInit,
    DeviceRevealerButtonModel,
};
use mshell_services::audio_service;
use mshell_utils::audio::{get_audio_out_icon_device_aware, spawn_default_output_watcher};
use relm4::ComponentSender;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use wayle_audio::core::device::output::OutputDevice;

pub(crate) use crate::menus::menu_widgets::audio_device_revealer_button::DeviceRevealerButtonInput as OutputDeviceRevealerButtonInput;

pub(crate) struct OutputKind;

pub(crate) type OutputDeviceRevealerButtonModel = DeviceRevealerButtonModel<OutputKind>;
pub(crate) type OutputDeviceRevealerButtonInit = DeviceRevealerButtonInit<OutputKind>;

impl AudioDeviceKind for OutputKind {
    type Device = OutputDevice;

    fn name(device: &OutputDevice) -> String {
        device.name.get()
    }

    fn description(device: &OutputDevice) -> String {
        device.description.get()
    }

    fn icon(device: &Arc<OutputDevice>) -> &'static str {
        get_audio_out_icon_device_aware(device)
    }

    fn is_default(device: &Arc<OutputDevice>) -> bool {
        audio_service()
            .default_output
            .get()
            .map(|d| d.eq(device))
            .unwrap_or(false)
    }

    fn spawn_default_watcher(
        sender: &ComponentSender<DeviceRevealerButtonModel<Self>>,
        token: CancellationToken,
    ) {
        spawn_default_output_watcher(sender, Some(token), || {
            DeviceRevealerButtonCommandOutput::DefaultDeviceChanged
        });
    }

    /// Switching the sink also moves the running playback streams onto it.
    fn make_default(device: Arc<OutputDevice>) {
        tokio::spawn(async move {
            if device.set_as_default().await.is_ok() {
                mshell_utils::audio::migrate_playback_streams_to(&device).await;
            }
        });
    }

    fn is_group(name: &str) -> bool {
        mshell_services::audio_groups::is_group(name)
    }

    fn disband(name: String) {
        tokio::spawn(async move {
            if let Some(group) = mshell_services::audio_groups::list_groups()
                .await
                .into_iter()
                .find(|g| g.sink_name == name)
            {
                mshell_services::audio_groups::disband_group(&group).await;
            }
        });
    }
}
