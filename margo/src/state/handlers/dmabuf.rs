//! `linux-dmabuf-v1` + `linux-drm-syncobj-v1` (explicit-sync) handlers.
//!
//! `DmabufHandler::dmabuf_imported` is the gate Firefox / Chromium /
//! GTK / Qt clients hit when they hand us a buffer over dmabuf
//! instead of SHM — we run the renderer-supplied `dmabuf_import_hook`
//! to validate import, then signal success / failure on the notifier.
//!
//! The drm-syncobj global is opt-in (`syncobj_enable`): the udev
//! backend creates `drm_syncobj_state` at start-up only when it is set
//! and the primary DRM node supports syncobj eventfds; otherwise
//! `drm_syncobj_state()` returns `None` and smithay refuses to bind.
//! With it up, acquire points are waited on by the pre-commit hook in
//! `handlers/compositor.rs`, and smithay signals release points when
//! the last reference to a buffer drops.

use smithay::{
    backend::allocator::dmabuf::Dmabuf,
    wayland::{
        dmabuf::{DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier},
        drm_syncobj::{DrmSyncobjHandler, DrmSyncobjState},
    },
};

use crate::state::MargoState;

impl DmabufHandler for MargoState {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.dmabuf_state
    }

    fn dmabuf_imported(
        &mut self,
        _global: &DmabufGlobal,
        dmabuf: Dmabuf,
        notifier: ImportNotifier,
    ) {
        let imported = self
            .dmabuf_import_hook
            .as_ref()
            .map(|hook| {
                let mut import = hook.borrow_mut();
                (*import)(&dmabuf)
            })
            .unwrap_or(true);

        if imported {
            let _ = notifier.successful::<Self>();
        } else {
            notifier.failed();
        }
    }
}

impl DrmSyncobjHandler for MargoState {
    fn drm_syncobj_state(&mut self) -> Option<&mut DrmSyncobjState> {
        self.drm_syncobj_state.as_mut()
    }
}
