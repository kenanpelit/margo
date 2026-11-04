use crate::common_widgets::option_list::{OptionItem, OptionsList};
use mshell_screenshot::record::RecordBackend;

#[derive(Debug, Clone)]
pub struct BackendOption {
    pub value: RecordBackend,
}

impl OptionItem for BackendOption {
    fn label(&self) -> String {
        self.value.label().to_string()
    }

    fn icon_name(&self) -> Option<String> {
        Some(backend_icon_name(self.value).to_string())
    }
}

pub fn backend_icon_name(backend: RecordBackend) -> &'static str {
    match backend {
        RecordBackend::WfRecorder => "cpu-symbolic",
        RecordBackend::GpuScreenRecorder => "video-display-symbolic",
    }
}

pub type BackendOptionsList = OptionsList<BackendOption>;
