use crate::common_widgets::option_list::{OptionItem, OptionsList};
use mshell_screenshot::record::AUDIO_DESKTOP_AND_MIC;
use std::sync::Arc;
use wayle_audio::core::device::input::InputDevice;

/// What the recording's audio track carries.
#[derive(Debug, Clone)]
pub enum AudioChoice {
    None,
    /// One PipeWire source (a microphone or a sink monitor).
    Device(Arc<InputDevice>),
    /// Desktop audio and the default microphone mixed into one track.
    DesktopAndMic,
}

#[derive(Debug, Clone)]
pub struct AudioOption {
    pub value: AudioChoice,
}

impl AudioOption {
    /// Audio spec handed to the recorder (see `mshell_screenshot::record`).
    pub fn spec(&self) -> Option<String> {
        match &self.value {
            AudioChoice::None => None,
            AudioChoice::Device(d) => Some(d.name.get()),
            AudioChoice::DesktopAndMic => Some(AUDIO_DESKTOP_AND_MIC.to_string()),
        }
    }
}

impl OptionItem for AudioOption {
    fn label(&self) -> String {
        get_audio_option_label(self)
    }

    fn icon_name(&self) -> Option<String> {
        Some(get_audio_option_icon_name(self))
    }
}

pub fn get_audio_option_label(option: &AudioOption) -> String {
    match &option.value {
        AudioChoice::None => "No Audio".to_string(),
        AudioChoice::Device(source) => source.description.get(),
        AudioChoice::DesktopAndMic => "Desktop + microphone".to_string(),
    }
}

pub fn get_audio_option_icon_name(option: &AudioOption) -> String {
    match &option.value {
        AudioChoice::None => "audio-volume-muted-symbolic".to_string(),
        AudioChoice::Device(source) if source.is_monitor.get() => {
            "audio-volume-medium-symbolic".to_string()
        }
        AudioChoice::Device(_) => "microphone-sensitivity-medium-symbolic".to_string(),
        AudioChoice::DesktopAndMic => "audio-volume-high-symbolic".to_string(),
    }
}

pub type AudioOptionsList = OptionsList<AudioOption>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_choices_map_to_recorder_specs() {
        let none = AudioOption {
            value: AudioChoice::None,
        };
        let mix = AudioOption {
            value: AudioChoice::DesktopAndMic,
        };
        assert_eq!(none.spec(), None);
        assert_eq!(mix.spec().as_deref(), Some("default_output|default_input"));
        assert_eq!(get_audio_option_label(&mix), "Desktop + microphone");
        assert_eq!(
            get_audio_option_icon_name(&none),
            "audio-volume-muted-symbolic"
        );
    }
}
