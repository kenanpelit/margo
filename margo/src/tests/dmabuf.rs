//! Integration coverage for `DmabufHandler` + `DrmSyncobjHandler`
//! (W4.2 Phase 4 extracted impl at `state/handlers/dmabuf.rs`).
//!
//! The `linux-dmabuf-v1` global is **only advertised when the
//! udev backend has imported a real DRM node** (the global is
//! created with format set + GBM device, both gated on a real
//! GPU). Same goes for `linux-drm-syncobj-v1` — it stays
//! `Option<DrmSyncobjState>` until the udev backend tests the
//! primary DRM node for `syncobj_eventfd` support.
//!
//! Headless tests therefore cannot drive the bind / import path.
//! The handler's behaviour ends up exercised in real session use
//! (every Firefox / Chromium / GTK app hits dmabuf import). The
//! tests below pin the **negative** invariant: in headless mode
//! these globals must NOT be advertised, otherwise the udev
//! backend's later `dmabuf_state.create_global_with_default_feedback`
//! call would double-register and panic.

use super::fixture::Fixture;

#[test]
fn dmabuf_global_not_advertised_in_headless_mode() {
    let mut fx = Fixture::new();
    let id = fx.add_client();
    fx.roundtrip(id);
    let names = fx.client(id).global_names();
    assert!(
        !names.iter().any(|n| n == "zwp_linux_dmabuf_v1"),
        "headless mode should NOT advertise dmabuf — udev backend gates it on a real GPU; saw {:?}",
        names,
    );
}

#[test]
fn drm_syncobj_global_not_advertised_in_headless_mode() {
    let mut fx = Fixture::new();
    let id = fx.add_client();
    fx.roundtrip(id);
    let names = fx.client(id).global_names();
    assert!(
        !names.iter().any(|n| n == "wp_linux_drm_syncobj_manager_v1"),
        "headless mode should NOT advertise drm_syncobj — udev backend gates it on a syncobj-capable kernel/driver; saw {:?}",
        names,
    );
}

#[test]
fn dmabuf_state_exists_but_global_is_optional() {
    // The DmabufState struct lives on MargoState (it'd be wired
    // through Display when the udev backend stands the global
    // up). DrmSyncobjState is `Option<>` and starts None. These
    // assertions pin the headless field-shape so a future
    // backend-init refactor doesn't accidentally inline-construct
    // either one.
    let fx = Fixture::new();
    assert!(
        fx.server.state.drm_syncobj_state.is_none(),
        "DrmSyncobjState must start None; udev backend stands it up after testing kernel support",
    );
    let _ = &fx.server.state.dmabuf_state;
}
