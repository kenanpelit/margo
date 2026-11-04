use mshell_screenshot::record::{RecordBackend, RecordHandle};
use reactive_stores::{ArcStore, KeyMap, PatchField, Store, StorePath};
use std::sync::{Mutex, OnceLock};

#[derive(Clone, Default, Store)]
pub struct RecordingState {
    pub handle: Option<RecordHandle>,
}

impl PatchField for RecordingState {
    fn patch_field(
        &mut self,
        new: Self,
        path: &StorePath,
        notify: &mut dyn FnMut(&StorePath),
        _keys: Option<&KeyMap>,
    ) {
        // No way to compare RecordHandles, so always replace and notify
        self.handle = new.handle;
        notify(path);
    }
}

static RECORDING_STATE: OnceLock<ArcStore<RecordingState>> = OnceLock::new();

pub fn recording_state() -> &'static ArcStore<RecordingState> {
    RECORDING_STATE.get_or_init(|| ArcStore::new(RecordingState::default()))
}

/// Backend picked in the screen-record menu; also used by `mshellctl
/// screenrecord` when it doesn't pass `--backend`. Lives for the shell's
/// lifetime (not persisted), starting at the wf-recorder default.
static PREFERRED_BACKEND: Mutex<RecordBackend> = Mutex::new(RecordBackend::WfRecorder);

pub fn preferred_backend() -> RecordBackend {
    *PREFERRED_BACKEND
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub fn set_preferred_backend(backend: RecordBackend) {
    *PREFERRED_BACKEND
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = backend;
}

/// The backend a recording actually uses: an explicit request, else the
/// menu's choice — degraded to wf-recorder when gpu-screen-recorder is
/// not installed, so a stale choice or a `--backend gsr` on a machine
/// without it still records instead of failing to spawn.
pub fn effective_backend(
    requested: Option<RecordBackend>,
    preferred: RecordBackend,
    gsr_installed: bool,
) -> RecordBackend {
    match requested.unwrap_or(preferred) {
        RecordBackend::GpuScreenRecorder if !gsr_installed => RecordBackend::WfRecorder,
        b => b,
    }
}

pub fn gsr_installed() -> bool {
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|dir| dir.join("gpu-screen-recorder").is_file())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use RecordBackend::*;

    #[test]
    fn explicit_request_beats_the_menu_choice() {
        assert_eq!(
            effective_backend(Some(WfRecorder), GpuScreenRecorder, true),
            WfRecorder
        );
        assert_eq!(
            effective_backend(Some(GpuScreenRecorder), WfRecorder, true),
            GpuScreenRecorder
        );
        assert_eq!(
            effective_backend(None, GpuScreenRecorder, true),
            GpuScreenRecorder
        );
        assert_eq!(effective_backend(None, WfRecorder, true), WfRecorder);
    }

    #[test]
    fn gsr_degrades_to_wf_when_missing() {
        assert_eq!(
            effective_backend(Some(GpuScreenRecorder), WfRecorder, false),
            WfRecorder
        );
        assert_eq!(
            effective_backend(None, GpuScreenRecorder, false),
            WfRecorder
        );
    }
}
