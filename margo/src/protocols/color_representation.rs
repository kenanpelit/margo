//! `wp_color_representation_v1` (staging) — alpha-mode / matrix-
//! coefficient metadata for client buffers.
//!
//! Unlike `color_management.rs` (whose manager global is gated until
//! HDR Phase 2), this global IS advertised: the request graph is two
//! interfaces with three setters — no info-event chains for clients
//! to trip over — and the feature set we advertise is exactly what
//! the render path already does, so nothing here is a no-op lie:
//!
//! * alpha mode: `premultiplied_electrical` only. That is the
//!   convention every margo texture shader assumes today (and the
//!   protocol's default for surfaces that never bind the extension).
//!   `straight` / `premultiplied_optical` would need blend/shader
//!   changes, so they are not advertised and raise the `alpha_mode`
//!   protocol error per spec.
//! * coefficients: `(identity, full)` only — the RGB-family
//!   identity mapping, again the existing behaviour. YCbCr matrices
//!   are NOT advertised: margo imports YUV dmabufs through EGL where
//!   the driver picks the conversion, so claiming per-surface BT.601
//!   vs BT.709 control would be dishonest. When the GLES path gains
//!   its own YUV sampling, extend the advertised list.
//! * chroma location: accepted and recorded for any valid enum value
//!   (spec only errors on invalid values); with no YCbCr
//!   coefficients advertised it never influences rendering today.
//!
//! Same hand-rolled pattern as `protocols/color_management.rs`:
//! manager state on `MargoState`, GlobalDispatch + Dispatch impls,
//! delegate macro at the bottom.

use std::collections::HashSet;

use smithay::reexports::wayland_protocols::wp::color_representation::v1::server::{
    wp_color_representation_manager_v1::{self, WpColorRepresentationManagerV1},
    wp_color_representation_surface_v1::{self, WpColorRepresentationSurfaceV1},
};
use smithay::reexports::wayland_server::backend::{ClientId, ObjectId};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, WEnum,
};

const VERSION: u32 = 1;

/// Manager-side state stored on `MargoState`. Tracks which
/// `wl_surface`s already have an extension object so `get_surface`
/// can raise the mandatory `surface_exists` protocol error.
pub struct ColorRepresentationState {
    surfaces: HashSet<ObjectId>,
}

impl ColorRepresentationState {
    pub fn new<D, F>(display: &DisplayHandle, filter: F) -> Self
    where
        D: GlobalDispatch<WpColorRepresentationManagerV1, ColorRepresentationGlobalData>,
        D: Dispatch<WpColorRepresentationManagerV1, ()>,
        D: Dispatch<WpColorRepresentationSurfaceV1, ColorRepresentationSurfaceData>,
        D: ColorRepresentationHandler,
        D: 'static,
        F: for<'c> Fn(&'c Client) -> bool + Send + Sync + 'static,
    {
        display.create_global::<D, WpColorRepresentationManagerV1, _>(
            VERSION,
            ColorRepresentationGlobalData {
                filter: Box::new(filter),
            },
        );
        Self {
            surfaces: HashSet::new(),
        }
    }
}

/// Per-surface-object data. `surface` doubles as the inert check:
/// once the `wl_surface` dies the extension object goes inert and
/// every setter raises the `inert` protocol error.
pub struct ColorRepresentationSurfaceData {
    pub surface: WlSurface,
}

pub struct ColorRepresentationGlobalData {
    pub filter: Box<dyn for<'c> Fn(&'c Client) -> bool + Send + Sync>,
}

pub trait ColorRepresentationHandler {
    fn color_representation_state(&mut self) -> &mut ColorRepresentationState;
}

// ── Manager global dispatch ──────────────────────────────────────────────────

impl<D> GlobalDispatch<WpColorRepresentationManagerV1, ColorRepresentationGlobalData, D>
    for ColorRepresentationState
where
    D: GlobalDispatch<WpColorRepresentationManagerV1, ColorRepresentationGlobalData>,
    D: Dispatch<WpColorRepresentationManagerV1, ()>,
    D: Dispatch<WpColorRepresentationSurfaceV1, ColorRepresentationSurfaceData>,
    D: ColorRepresentationHandler + 'static,
{
    fn bind(
        _state: &mut D,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: New<WpColorRepresentationManagerV1>,
        _global_data: &ColorRepresentationGlobalData,
        data_init: &mut DataInit<'_, D>,
    ) {
        let manager = data_init.init(resource, ());

        // Spec: immediately send one event per supported value, then
        // done. Only what the render path genuinely does is listed —
        // see the module comment.
        manager.supported_alpha_mode(
            wp_color_representation_surface_v1::AlphaMode::PremultipliedElectrical,
        );
        manager.supported_coefficients_and_ranges(
            wp_color_representation_surface_v1::Coefficients::Identity,
            wp_color_representation_surface_v1::Range::Full,
        );
        manager.done();
    }

    fn can_view(client: Client, global_data: &ColorRepresentationGlobalData) -> bool {
        (global_data.filter)(&client)
    }
}

impl<D> Dispatch<WpColorRepresentationManagerV1, (), D> for ColorRepresentationState
where
    D: Dispatch<WpColorRepresentationManagerV1, ()>,
    D: Dispatch<WpColorRepresentationSurfaceV1, ColorRepresentationSurfaceData>,
    D: ColorRepresentationHandler + 'static,
{
    fn request(
        state: &mut D,
        _client: &Client,
        resource: &WpColorRepresentationManagerV1,
        request: wp_color_representation_manager_v1::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            wp_color_representation_manager_v1::Request::Destroy => {}
            wp_color_representation_manager_v1::Request::GetSurface { id, surface } => {
                let duplicate = !state
                    .color_representation_state()
                    .surfaces
                    .insert(surface.id());
                // Always init the new_id before any error path — a
                // dropped new_id is itself a protocol violation (see
                // the color_management Phase 1 post-mortem).
                data_init.init(id, ColorRepresentationSurfaceData { surface });
                if duplicate {
                    resource.post_error(
                        wp_color_representation_manager_v1::Error::SurfaceExists,
                        "wl_surface already has a wp_color_representation_surface_v1",
                    );
                }
            }
            _ => {}
        }
    }
}

// ── Per-surface object ───────────────────────────────────────────────────────

impl<D> Dispatch<WpColorRepresentationSurfaceV1, ColorRepresentationSurfaceData, D>
    for ColorRepresentationState
where
    D: Dispatch<WpColorRepresentationSurfaceV1, ColorRepresentationSurfaceData>,
    D: ColorRepresentationHandler + 'static,
{
    fn request(
        _state: &mut D,
        _client: &Client,
        resource: &WpColorRepresentationSurfaceV1,
        request: wp_color_representation_surface_v1::Request,
        data: &ColorRepresentationSurfaceData,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        // Every setter first checks inertness (wl_surface gone).
        let inert = !data.surface.is_alive();
        match request {
            wp_color_representation_surface_v1::Request::Destroy => {}
            wp_color_representation_surface_v1::Request::SetAlphaMode { alpha_mode } => {
                if inert {
                    resource.post_error(
                        wp_color_representation_surface_v1::Error::Inert,
                        "wl_surface was destroyed",
                    );
                    return;
                }
                match alpha_mode {
                    WEnum::Value(
                        wp_color_representation_surface_v1::AlphaMode::PremultipliedElectrical,
                    ) => {
                        // Accepted — identical to the compositor's
                        // fixed behaviour, so nothing to store yet.
                    }
                    _ => resource.post_error(
                        wp_color_representation_surface_v1::Error::AlphaMode,
                        "unsupported alpha mode (only premultiplied_electrical is advertised)",
                    ),
                }
            }
            wp_color_representation_surface_v1::Request::SetCoefficientsAndRange {
                coefficients,
                range,
            } => {
                if inert {
                    resource.post_error(
                        wp_color_representation_surface_v1::Error::Inert,
                        "wl_surface was destroyed",
                    );
                    return;
                }
                let ok = matches!(
                    coefficients,
                    WEnum::Value(wp_color_representation_surface_v1::Coefficients::Identity)
                ) && matches!(
                    range,
                    WEnum::Value(wp_color_representation_surface_v1::Range::Full)
                );
                if ok {
                    // identity/full == the RGB passthrough margo
                    // already performs; nothing to store yet.
                } else {
                    resource.post_error(
                        wp_color_representation_surface_v1::Error::Coefficients,
                        "unsupported coefficients/range (only identity/full is advertised)",
                    );
                }
            }
            wp_color_representation_surface_v1::Request::SetChromaLocation { chroma_location } => {
                if inert {
                    resource.post_error(
                        wp_color_representation_surface_v1::Error::Inert,
                        "wl_surface was destroyed",
                    );
                    return;
                }
                match chroma_location {
                    WEnum::Value(_) => {
                        // Valid location: accepted. With no YCbCr
                        // coefficients advertised it cannot influence
                        // rendering today.
                    }
                    WEnum::Unknown(_) => resource.post_error(
                        wp_color_representation_surface_v1::Error::ChromaLocation,
                        "invalid chroma location",
                    ),
                }
            }
            _ => {}
        }
    }

    fn destroyed(
        state: &mut D,
        _client: ClientId,
        _resource: &WpColorRepresentationSurfaceV1,
        data: &ColorRepresentationSurfaceData,
    ) {
        // Free the slot so a client may create a fresh extension
        // object for the same wl_surface (spec: destroy unsets all
        // color-representation state).
        state
            .color_representation_state()
            .surfaces
            .remove(&data.surface.id());
    }
}

#[macro_export]
macro_rules! delegate_color_representation {
    ($ty:ty) => {
        smithay::reexports::wayland_server::delegate_global_dispatch!($ty:
            [smithay::reexports::wayland_protocols::wp::color_representation::v1::server::wp_color_representation_manager_v1::WpColorRepresentationManagerV1: $crate::protocols::color_representation::ColorRepresentationGlobalData] =>
                $crate::protocols::color_representation::ColorRepresentationState
        );
        smithay::reexports::wayland_server::delegate_dispatch!($ty:
            [smithay::reexports::wayland_protocols::wp::color_representation::v1::server::wp_color_representation_manager_v1::WpColorRepresentationManagerV1: ()] =>
                $crate::protocols::color_representation::ColorRepresentationState
        );
        smithay::reexports::wayland_server::delegate_dispatch!($ty:
            [smithay::reexports::wayland_protocols::wp::color_representation::v1::server::wp_color_representation_surface_v1::WpColorRepresentationSurfaceV1: $crate::protocols::color_representation::ColorRepresentationSurfaceData] =>
                $crate::protocols::color_representation::ColorRepresentationState
        );
    };
}
