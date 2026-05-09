//! `linux-dmabuf-v1` + `linux-drm-syncobj-v1` (explicit-sync) handlers.
//!
//! `DmabufHandler::dmabuf_imported` is the gate Firefox / Chromium /
//! GTK / Qt clients hit when they hand us a buffer over dmabuf
//! instead of SHM — we run the renderer-supplied `dmabuf_import_hook`
//! to validate import, then signal success / failure on the notifier.
//!
//! The drm-syncobj global is only exposed when the udev backend has
//! had a chance to test the primary DRM node for `syncobj_eventfd`
//! support and flip `drm_syncobj_state` to `Some`. Until that
//! happens `drm_syncobj_state()` returns `None` and smithay's
//! dispatch refuses to bind, so kernels / drivers without timeline
//! syncobj support don't see a global advertised at all (the
//! contract niri / sway / mutter follow). Once the global is up,
//! the per-surface `wp_linux_drm_syncobj_surface_v1` plumbs acquire
//! + release fences through smithay's compositor pre-commit hooks
//! automatically.

use smithay::{
    backend::allocator::dmabuf::Dmabuf,
    delegate_dmabuf, delegate_drm_syncobj,
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
delegate_dmabuf!(MargoState);

impl DrmSyncobjHandler for MargoState {
    fn drm_syncobj_state(&mut self) -> Option<&mut DrmSyncobjState> {
        self.drm_syncobj_state.as_mut()
    }
}
delegate_drm_syncobj!(MargoState);
