use crate::common_widgets::option_list::{OptionItem, OptionsList};
use mshell_screenshot::webcam::WebcamSize;

/// One webcam choice in the screen-record menu (off / small / medium / large).
#[derive(Debug, Clone)]
pub struct WebcamOption {
    pub value: WebcamSize,
}

impl OptionItem for WebcamOption {
    fn label(&self) -> String {
        self.value.label().to_string()
    }

    fn icon_name(&self) -> Option<String> {
        Some(webcam_icon_name(self.value).to_string())
    }
}

pub fn webcam_icon_name(size: WebcamSize) -> &'static str {
    match size {
        WebcamSize::Off => "camera-disabled-symbolic",
        _ => "camera-web-symbolic",
    }
}

pub type WebcamOptionsList = OptionsList<WebcamOption>;
