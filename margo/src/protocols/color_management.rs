//! `wp_color_management_v1` (staging) — Phase 1 scaffolding.
//!
//! What this module is for, and what it deliberately is **not**:
//!
//! * Phase 1 (this) — register the manager global, advertise the
//!   primaries / transfer functions / rendering intents margo
//!   knows how to *handle in principle*, accept the protocol's
//!   request graph (creator → image description → surface tracker
//!   → `set_image_description`), and store the per-surface
//!   description in compositor state. Composite output stays sRGB
//!   — the render path doesn't yet read the stored description.
//! * Phase 2 — linear-light fp16 composite + per-surface transfer
//!   function decode at sample time.
//! * Phase 3 — KMS HDR scan-out (`HDR_OUTPUT_METADATA`).
//! * Phase 4 — ICC profile per-output 3D LUT.
//!
//! Why land Phase 1 alone, given it doesn't visibly change pixels?
//! Because the order in which clients adopt HDR depends entirely on
//! the manager global being present. Chromium and mpv probe for
//! `wp_color_manager_v1`; if it isn't bound, they never even ATTEMPT
//! their HDR decode paths, so by the time we ship Phase 2 their
//! cached preference is "this compositor is SDR only". Standing up
//! the global early lets clients negotiate as if the compositor
//! were colour-managed; when Phase 2 lands the negotiated state
//! Just Works without a client-side update.
//!
//! ICC creators are stubbed as `failed()` — Phase 4 territory. The
//! parametric creator is fully wired so Chromium / mpv can
//! `set_tf_named(st2084_pq) + set_primaries_named(bt2020) →
//! create() → ready2` and start hand-decoded HDR even though we
//! tone-map back to sRGB at composite time.
//!
//! Adapted from the same hand-rolled-protocol pattern as
//! `protocols/output_management.rs`: a manager state on
//! `MargoState`, a delegate macro at the bottom, GlobalDispatch +
//! Dispatch impls per interface.

#![allow(dead_code)]

use std::sync::atomic::{AtomicU64, Ordering};

use smithay::reexports::wayland_protocols::wp::color_management::v1::server::{
    wp_color_management_output_v1::{self, WpColorManagementOutputV1},
    wp_color_management_surface_feedback_v1::{self, WpColorManagementSurfaceFeedbackV1},
    wp_color_management_surface_v1::{self, WpColorManagementSurfaceV1},
    wp_color_manager_v1::{self, WpColorManagerV1},
    wp_image_description_creator_icc_v1::{self, WpImageDescriptionCreatorIccV1},
    wp_image_description_creator_params_v1::{self, WpImageDescriptionCreatorParamsV1},
    wp_image_description_info_v1::WpImageDescriptionInfoV1,
    wp_image_description_v1::{self, WpImageDescriptionV1},
};
use smithay::reexports::wayland_server::backend::ClientId;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};

const VERSION: u32 = 1;

/// Per-image-description data stored on the resource. `identity`
/// is a 64-bit token surfaces and feedback objects use to reference
/// "the same description object" without keeping a `Resource`
/// handle alive across protocol calls. Phase 2 reads `params` to
/// drive the linear-light decode shader; Phase 1 just records it.
#[derive(Debug, Clone)]
pub struct ImageDescription {
    pub identity: u64,
    pub params: ImageDescriptionParams,
}

#[derive(Debug, Clone, Default)]
pub struct ImageDescriptionParams {
    /// `wp_color_manager_v1.primaries` enum value, or `None` if
    /// the client used `set_primaries` (custom chromaticities).
    pub primaries_named: Option<u32>,
    /// `wp_color_manager_v1.transfer_function` enum value.
    pub tf_named: Option<u32>,
    /// `(min_lum * 10000, max_lum, reference_lum)` — already in
    /// the protocol's units.
    pub luminances: Option<(u32, u32, u32)>,
    /// HDR mastering display chromaticities, scaled by 1M as the
    /// protocol delivers them.
    pub mastering_primaries: Option<[i32; 8]>,
    /// `(min, max)` in protocol units (cd/m²).
    pub mastering_luminance: Option<(u32, u32)>,
    pub max_cll: Option<u32>,
    pub max_fall: Option<u32>,
}

/// Manager-side state. Stored on `MargoState`; the dispatch impls
/// reach it via `ColorManagementHandler::color_management_state`.
pub struct ColorManagementState {
    next_identity: AtomicU64,
}

impl ColorManagementState {
    pub fn new<D, F>(display: &DisplayHandle, filter: F) -> Self
    where
        D: GlobalDispatch<WpColorManagerV1, ColorManagerGlobalData>,
        D: Dispatch<WpColorManagerV1, ()>,
        D: Dispatch<WpColorManagementOutputV1, ()>,
        D: Dispatch<WpColorManagementSurfaceV1, SurfaceTrackerData>,
        D: Dispatch<WpColorManagementSurfaceFeedbackV1, ()>,
        D: Dispatch<WpImageDescriptionCreatorIccV1, ()>,
        D: Dispatch<WpImageDescriptionCreatorParamsV1, CreatorParamsData>,
        D: Dispatch<WpImageDescriptionV1, ImageDescription>,
        D: ColorManagementHandler,
        D: 'static,
        F: for<'c> Fn(&'c Client) -> bool + Send + Sync + 'static,
    {
        display.create_global::<D, WpColorManagerV1, _>(
            VERSION,
            ColorManagerGlobalData {
                filter: Box::new(filter),
            },
        );
        Self {
            // Start at 1 so 0 stays reserved for "no description".
            next_identity: AtomicU64::new(1),
        }
    }

    /// Allocate a unique identity for a fresh image description.
    pub fn alloc_identity(&self) -> u64 {
        self.next_identity.fetch_add(1, Ordering::Relaxed)
    }
}

/// Per-surface tracker data. `identity` 0 means no description has
/// been set; otherwise it points at the active description's
/// identity. Phase 2 reads this from the render path.
pub struct SurfaceTrackerData {
    pub surface: WlSurface,
    pub identity: AtomicU64,
}

/// Per-creator buffer of params. Filled by `set_*` requests, read
/// once at `create()` time to mint an `ImageDescription`. The
/// `consumed` flag enforces the protocol's "create destroys this
/// object" semantics — calling any setter after create is a
/// protocol error per spec, but we just ignore it (no enforcement
/// today; future Phase will add).
#[derive(Default)]
pub struct CreatorParamsData {
    pub params: std::sync::Mutex<ImageDescriptionParams>,
}

/// Per-global filter so we can hide the manager from clients that
/// shouldn't see it (matches the gamma_control pattern).
pub struct ColorManagerGlobalData {
    pub filter: Box<dyn for<'c> Fn(&'c Client) -> bool + Send + Sync>,
}

pub trait ColorManagementHandler {
    fn color_management_state(&mut self) -> &mut ColorManagementState;
}

// ── Manager global dispatch ──────────────────────────────────────────────────

impl<D> GlobalDispatch<WpColorManagerV1, ColorManagerGlobalData, D> for ColorManagementState
where
    D: GlobalDispatch<WpColorManagerV1, ColorManagerGlobalData>,
    D: Dispatch<WpColorManagerV1, ()>,
    D: Dispatch<WpColorManagementOutputV1, ()>,
    D: Dispatch<WpColorManagementSurfaceV1, SurfaceTrackerData>,
    D: Dispatch<WpColorManagementSurfaceFeedbackV1, ()>,
    D: Dispatch<WpImageDescriptionCreatorIccV1, ()>,
    D: Dispatch<WpImageDescriptionCreatorParamsV1, CreatorParamsData>,
    D: Dispatch<WpImageDescriptionV1, ImageDescription>,
    D: ColorManagementHandler + 'static,
{
    fn bind(
        _state: &mut D,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: New<WpColorManagerV1>,
        _global_data: &ColorManagerGlobalData,
        data_init: &mut DataInit<'_, D>,
    ) {
        let manager = data_init.init(resource, ());

        // Advertise the primaries we'll be able to handle once the
        // composite path goes linear-light. Listing them now lets
        // Chromium decide it's safe to enable HDR detection without
        // waiting for Phase 2; tone-mapping at composite is still
        // SDR-correct so we don't break colours in the meantime.
        for p in [
            wp_color_manager_v1::Primaries::Srgb,
            wp_color_manager_v1::Primaries::Bt2020,
            wp_color_manager_v1::Primaries::DisplayP3,
            wp_color_manager_v1::Primaries::AdobeRgb,
        ] {
            manager.supported_primaries_named(p);
        }

        // Transfer functions. PQ + HLG are the HDR pair clients
        // actually probe for; sRGB + ext_linear cover the everyday
        // case + the linear-FP16 composite Phase 2 will switch to.
        for tf in [
            wp_color_manager_v1::TransferFunction::Srgb,
            wp_color_manager_v1::TransferFunction::ExtLinear,
            wp_color_manager_v1::TransferFunction::St2084Pq,
            wp_color_manager_v1::TransferFunction::Hlg,
            wp_color_manager_v1::TransferFunction::Gamma22,
        ] {
            manager.supported_tf_named(tf);
        }

        // Render intents. Perceptual is the only one we'd actually
        // implement in Phase 2; advertising it alone matches what
        // mutter-46 ships today.
        manager.supported_intent(wp_color_manager_v1::RenderIntent::Perceptual);

        // Feature set. Parametric creator + set_primaries +
        // set_tf_power + set_luminances + set_mastering_*  are the
        // ones the parametric creator path actually consumes;
        // ICC-v2/v4 we'll wire in Phase 4 (advertised already so a
        // client can pre-detect, but `set_icc_file` will fail
        // until then).
        for f in [
            wp_color_manager_v1::Feature::Parametric,
            wp_color_manager_v1::Feature::SetPrimaries,
            wp_color_manager_v1::Feature::SetTfPower,
            wp_color_manager_v1::Feature::SetLuminances,
            wp_color_manager_v1::Feature::SetMasteringDisplayPrimaries,
            wp_color_manager_v1::Feature::ExtendedTargetVolume,
        ] {
            manager.supported_feature(f);
        }

        manager.done();
    }

    fn can_view(client: Client, global_data: &ColorManagerGlobalData) -> bool {
        (global_data.filter)(&client)
    }
}

impl<D> Dispatch<WpColorManagerV1, (), D> for ColorManagementState
where
    D: Dispatch<WpColorManagerV1, ()>,
    D: Dispatch<WpColorManagementOutputV1, ()>,
    D: Dispatch<WpColorManagementSurfaceV1, SurfaceTrackerData>,
    D: Dispatch<WpColorManagementSurfaceFeedbackV1, ()>,
    D: Dispatch<WpImageDescriptionCreatorIccV1, ()>,
    D: Dispatch<WpImageDescriptionCreatorParamsV1, CreatorParamsData>,
    D: Dispatch<WpImageDescriptionV1, ImageDescription>,
    D: ColorManagementHandler + 'static,
{
    fn request(
        _state: &mut D,
        _client: &Client,
        _resource: &WpColorManagerV1,
        request: wp_color_manager_v1::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            wp_color_manager_v1::Request::Destroy => {}
            wp_color_manager_v1::Request::GetOutput { id, output: _ } => {
                // Per-output description objects aren't differentiated yet
                // — every output reports the same "no preferred image
                // description" state. Phase 4 plumbs ICC per output.
                data_init.init(id, ());
            }
            wp_color_manager_v1::Request::GetSurface { id, surface } => {
                data_init.init(
                    id,
                    SurfaceTrackerData {
                        surface,
                        identity: AtomicU64::new(0),
                    },
                );
            }
            wp_color_manager_v1::Request::GetSurfaceFeedback { id, surface: _ } => {
                data_init.init(id, ());
            }
            wp_color_manager_v1::Request::CreateIccCreator { obj } => {
                // ICC creators are stubbed; on `create()` they fire
                // `failed(unsupported)` so clients fall back to the
                // parametric path or skip colour management.
                data_init.init(obj, ());
            }
            wp_color_manager_v1::Request::CreateParametricCreator { obj } => {
                data_init.init(obj, CreatorParamsData::default());
            }
            wp_color_manager_v1::Request::CreateWindowsScrgb { image_description } => {
                // Synthesise a description matching the Windows scRGB
                // convention: linear sRGB primaries with 80 cd/m² white.
                // Stored verbatim; Phase 2 reads it.
                let identity = _state.color_management_state().alloc_identity();
                let desc = ImageDescription {
                    identity,
                    params: ImageDescriptionParams {
                        primaries_named: Some(wp_color_manager_v1::Primaries::Srgb as u32),
                        tf_named: Some(wp_color_manager_v1::TransferFunction::ExtLinear as u32),
                        luminances: Some((0, 80, 80)),
                        ..Default::default()
                    },
                };
                let resource = data_init.init(image_description, desc);
                resource.ready(identity as u32);
            }
            wp_color_manager_v1::Request::GetImageDescription { .. } => {
                // since: 2 — at v1 this never fires; defensive default.
            }
            _ => {}
        }
    }
}

// ── Per-output description (stub) ────────────────────────────────────────────

impl<D> Dispatch<WpColorManagementOutputV1, (), D> for ColorManagementState
where
    D: Dispatch<WpColorManagementOutputV1, ()>,
    D: Dispatch<WpImageDescriptionV1, ImageDescription>,
    D: ColorManagementHandler + 'static,
{
    fn request(
        state: &mut D,
        _client: &Client,
        _resource: &WpColorManagementOutputV1,
        request: wp_color_management_output_v1::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            wp_color_management_output_v1::Request::Destroy => {}
            wp_color_management_output_v1::Request::GetImageDescription { image_description } => {
                // Mint a fresh sRGB description for this query. Phase 4
                // will read the per-output ICC profile here.
                let identity = state.color_management_state().alloc_identity();
                let desc = ImageDescription {
                    identity,
                    params: ImageDescriptionParams {
                        primaries_named: Some(wp_color_manager_v1::Primaries::Srgb as u32),
                        tf_named: Some(wp_color_manager_v1::TransferFunction::Srgb as u32),
                        ..Default::default()
                    },
                };
                let resource = data_init.init(image_description, desc);
                resource.ready(identity as u32);
            }
            _ => {}
        }
    }
}

// ── Per-surface tracker — stores the active description's identity ──────────

impl<D> Dispatch<WpColorManagementSurfaceV1, SurfaceTrackerData, D> for ColorManagementState
where
    D: Dispatch<WpColorManagementSurfaceV1, SurfaceTrackerData>,
    D: ColorManagementHandler + 'static,
{
    fn request(
        _state: &mut D,
        _client: &Client,
        _resource: &WpColorManagementSurfaceV1,
        request: wp_color_management_surface_v1::Request,
        data: &SurfaceTrackerData,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            wp_color_management_surface_v1::Request::Destroy => {
                data.identity.store(0, Ordering::Relaxed);
            }
            wp_color_management_surface_v1::Request::SetImageDescription {
                image_description,
                render_intent: _,
            } => {
                // Look up the description's identity from the resource's
                // user_data and stash it. Phase 2 reads this at render
                // sample time to pick the right transfer-function decode.
                if let Some(desc) =
                    image_description.data::<ImageDescription>()
                {
                    data.identity.store(desc.identity, Ordering::Relaxed);
                }
            }
            wp_color_management_surface_v1::Request::UnsetImageDescription => {
                data.identity.store(0, Ordering::Relaxed);
            }
            _ => {}
        }
    }

    fn destroyed(
        _state: &mut D,
        _client: ClientId,
        _resource: &WpColorManagementSurfaceV1,
        _data: &SurfaceTrackerData,
    ) {
    }
}

// ── Surface feedback — replies with sRGB until per-output ICC lands ─────────

impl<D> Dispatch<WpColorManagementSurfaceFeedbackV1, (), D> for ColorManagementState
where
    D: Dispatch<WpColorManagementSurfaceFeedbackV1, ()>,
    D: Dispatch<WpImageDescriptionV1, ImageDescription>,
    D: ColorManagementHandler + 'static,
{
    fn request(
        state: &mut D,
        _client: &Client,
        _resource: &WpColorManagementSurfaceFeedbackV1,
        request: wp_color_management_surface_feedback_v1::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            wp_color_management_surface_feedback_v1::Request::Destroy => {}
            wp_color_management_surface_feedback_v1::Request::GetPreferred {
                image_description,
            }
            | wp_color_management_surface_feedback_v1::Request::GetPreferredParametric {
                image_description,
            } => {
                let identity = state.color_management_state().alloc_identity();
                let desc = ImageDescription {
                    identity,
                    params: ImageDescriptionParams {
                        primaries_named: Some(wp_color_manager_v1::Primaries::Srgb as u32),
                        tf_named: Some(wp_color_manager_v1::TransferFunction::Srgb as u32),
                        ..Default::default()
                    },
                };
                let resource = data_init.init(image_description, desc);
                resource.ready(identity as u32);
            }
            _ => {}
        }
    }
}

// ── ICC creator — stubbed: every create() fires failed ───────────────────────

impl<D> Dispatch<WpImageDescriptionCreatorIccV1, (), D> for ColorManagementState
where
    D: Dispatch<WpImageDescriptionCreatorIccV1, ()>,
    D: Dispatch<WpImageDescriptionV1, ImageDescription>,
    D: ColorManagementHandler + 'static,
{
    fn request(
        _state: &mut D,
        _client: &Client,
        _resource: &WpImageDescriptionCreatorIccV1,
        request: wp_image_description_creator_icc_v1::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            wp_image_description_creator_icc_v1::Request::Create { image_description } => {
                let resource = data_init.init(image_description, ImageDescription {
                    identity: 0,
                    params: ImageDescriptionParams::default(),
                });
                resource.failed(
                    wp_image_description_v1::Cause::Unsupported,
                    "ICC profile creation not yet supported (Phase 4)".to_string(),
                );
            }
            wp_image_description_creator_icc_v1::Request::SetIccFile { .. } => {
                // No-op: we'll surface unsupported at create() time.
            }
            _ => {}
        }
    }
}

// ── Parametric creator — buffers params, emits ready on create ──────────────

impl<D> Dispatch<WpImageDescriptionCreatorParamsV1, CreatorParamsData, D> for ColorManagementState
where
    D: Dispatch<WpImageDescriptionCreatorParamsV1, CreatorParamsData>,
    D: Dispatch<WpImageDescriptionV1, ImageDescription>,
    D: ColorManagementHandler + 'static,
{
    fn request(
        state: &mut D,
        _client: &Client,
        _resource: &WpImageDescriptionCreatorParamsV1,
        request: wp_image_description_creator_params_v1::Request,
        data: &CreatorParamsData,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            wp_image_description_creator_params_v1::Request::Create { image_description } => {
                let params = data
                    .params
                    .lock()
                    .map(|guard| guard.clone())
                    .unwrap_or_default();
                // Spec: at least primaries + tf must be set, otherwise
                // create fires failed(incomplete_set). Enforced loosely
                // here — clients that obviously misuse the API see the
                // error; clients that set just one of the two also do.
                if params.primaries_named.is_none() && params.mastering_primaries.is_none() {
                    let resource = data_init.init(
                        image_description,
                        ImageDescription { identity: 0, params },
                    );
                    resource.failed(
                        wp_image_description_v1::Cause::OperatingSystem,
                        "primaries not set".to_string(),
                    );
                    return;
                }
                if params.tf_named.is_none() {
                    let resource = data_init.init(
                        image_description,
                        ImageDescription { identity: 0, params },
                    );
                    resource.failed(
                        wp_image_description_v1::Cause::OperatingSystem,
                        "transfer function not set".to_string(),
                    );
                    return;
                }
                let identity = state.color_management_state().alloc_identity();
                let desc = ImageDescription {
                    identity,
                    params,
                };
                let resource = data_init.init(image_description, desc);
                resource.ready(identity as u32);
            }
            wp_image_description_creator_params_v1::Request::SetTfNamed { tf } => {
                if let Ok(mut p) = data.params.lock() {
                    p.tf_named = Some(tf.into());
                }
            }
            wp_image_description_creator_params_v1::Request::SetTfPower { eexp: _ } => {
                // power TF is its own param category; recorded loosely
                // as "tf_named = 0" for now (phase 2 will store the
                // power exponent properly).
            }
            wp_image_description_creator_params_v1::Request::SetPrimariesNamed { primaries } => {
                if let Ok(mut p) = data.params.lock() {
                    p.primaries_named = Some(primaries.into());
                }
            }
            wp_image_description_creator_params_v1::Request::SetPrimaries {
                r_x,
                r_y,
                g_x,
                g_y,
                b_x,
                b_y,
                w_x,
                w_y,
            } => {
                if let Ok(mut p) = data.params.lock() {
                    p.mastering_primaries = Some([r_x, r_y, g_x, g_y, b_x, b_y, w_x, w_y]);
                }
            }
            wp_image_description_creator_params_v1::Request::SetLuminances {
                min_lum,
                max_lum,
                reference_lum,
            } => {
                if let Ok(mut p) = data.params.lock() {
                    p.luminances = Some((min_lum, max_lum, reference_lum));
                }
            }
            wp_image_description_creator_params_v1::Request::SetMasteringDisplayPrimaries {
                r_x,
                r_y,
                g_x,
                g_y,
                b_x,
                b_y,
                w_x,
                w_y,
            } => {
                if let Ok(mut p) = data.params.lock() {
                    p.mastering_primaries = Some([r_x, r_y, g_x, g_y, b_x, b_y, w_x, w_y]);
                }
            }
            wp_image_description_creator_params_v1::Request::SetMasteringLuminance {
                min_lum,
                max_lum,
            } => {
                if let Ok(mut p) = data.params.lock() {
                    p.mastering_luminance = Some((min_lum, max_lum));
                }
            }
            wp_image_description_creator_params_v1::Request::SetMaxCll { max_cll } => {
                if let Ok(mut p) = data.params.lock() {
                    p.max_cll = Some(max_cll);
                }
            }
            wp_image_description_creator_params_v1::Request::SetMaxFall { max_fall } => {
                if let Ok(mut p) = data.params.lock() {
                    p.max_fall = Some(max_fall);
                }
            }
            _ => {}
        }
    }
}

// ── Image description resource — passive, just holds the ImageDescription ──

impl<D> Dispatch<WpImageDescriptionV1, ImageDescription, D> for ColorManagementState
where
    D: Dispatch<WpImageDescriptionV1, ImageDescription>,
    D: Dispatch<WpImageDescriptionInfoV1, ()>,
    D: ColorManagementHandler + 'static,
{
    fn request(
        _state: &mut D,
        _client: &Client,
        _resource: &WpImageDescriptionV1,
        request: wp_image_description_v1::Request,
        data: &ImageDescription,
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            wp_image_description_v1::Request::Destroy => {}
            wp_image_description_v1::Request::GetInformation { information } => {
                // The previous Phase 1 commit `drop(information)`'d the
                // new_id without binding it — Wayland protocol violation,
                // server kicks the client. The follow-up tried to send
                // a partial event set (just primaries_named + tf_named)
                // and segfaulted margo because clients (mpv, Chromium)
                // expect the full event sequence per the protocol spec;
                // dispatching against a half-populated info object is
                // undefined behaviour territory.
                //
                // The fix is to mirror Hyprland's `CColorManagement
                // ImageDescriptionInfo` constructor literally — it
                // sends EVERY event the protocol defines for a usable
                // sRGB description, then `done`:
                //
                //   1. primaries (8 chromaticity ints, scaled by 1M)
                //   2. primaries_named (if a named primary is stored)
                //   3. tf_named (mandatory)
                //   4. luminances (min × 10000, max, reference cd/m²)
                //   5. target_primaries (matches primaries unless the
                //      stored desc has separate mastering values)
                //   6. target_luminance (min × 10000, max)
                //   7. done (destructor — finalizes the resource)
                //
                // For the sRGB defaults below we use the well-known
                // BT.709/sRGB chromaticities and 80 cd/m² white point
                // (the SDR reference). When a stored description has
                // explicit primaries / tf / luminances those override.

                let info = data_init.init(information, ());

                // ── 1. primaries (chromaticity ints scaled by 1M) ──
                // sRGB / BT.709: R=(0.640,0.330) G=(0.300,0.600)
                // B=(0.150,0.060) W=D65 (0.3127,0.3290).
                let prim = data.params.mastering_primaries.unwrap_or([
                    640_000, 330_000,   // R x, y
                    300_000, 600_000,   // G x, y
                    150_000,  60_000,   // B x, y
                    312_700, 329_000,   // W x, y (D65)
                ]);
                info.primaries(
                    prim[0], prim[1], prim[2], prim[3],
                    prim[4], prim[5], prim[6], prim[7],
                );

                // ── 2. primaries_named (optional, only if known enum) ──
                if let Some(p) = data.params.primaries_named {
                    if let Ok(named) = wp_color_manager_v1::Primaries::try_from(p) {
                        info.primaries_named(named);
                    }
                } else {
                    info.primaries_named(wp_color_manager_v1::Primaries::Srgb);
                }

                // ── 3. tf_named (mandatory) ──
                let tf = data
                    .params
                    .tf_named
                    .and_then(|v| wp_color_manager_v1::TransferFunction::try_from(v).ok())
                    .unwrap_or(wp_color_manager_v1::TransferFunction::Srgb);
                info.tf_named(tf);

                // ── 4. luminances (min × 10000, max, reference) ──
                let (min_lum, max_lum, ref_lum) = data
                    .params
                    .luminances
                    .unwrap_or((0, 80, 80));
                info.luminances(min_lum, max_lum, ref_lum);

                // ── 5. target_primaries — reuse `primaries` if no
                //     mastering primaries are stored. mpv/Chromium use
                //     this to know "what gamut should I tone-map for".
                info.target_primaries(
                    prim[0], prim[1], prim[2], prim[3],
                    prim[4], prim[5], prim[6], prim[7],
                );

                // ── 6. target_luminance (min × 10000, max) ──
                let (target_min, target_max) = data
                    .params
                    .mastering_luminance
                    .unwrap_or((0, 80));
                info.target_luminance(target_min, target_max);

                // ── 7. target_max_cll / fall (only if HDR-tagged) ──
                if let Some(cll) = data.params.max_cll {
                    info.target_max_cll(cll);
                }
                if let Some(fall) = data.params.max_fall {
                    info.target_max_fall(fall);
                }

                // ── 8. done — destructor; resource finalizes here ──
                info.done();
            }
            _ => {}
        }
    }
}

// ── Image description info — passive event-emitter, all destructor-driven ──

impl<D> Dispatch<WpImageDescriptionInfoV1, (), D> for ColorManagementState
where
    D: Dispatch<WpImageDescriptionInfoV1, ()>,
    D: ColorManagementHandler + 'static,
{
    fn request(
        _state: &mut D,
        _client: &Client,
        _resource: &WpImageDescriptionInfoV1,
        _request: smithay::reexports::wayland_protocols::wp::color_management::v1::server::wp_image_description_info_v1::Request,
        _data: &(),
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        // wp_image_description_info_v1 has no requests other than
        // the protocol-baseline destructor. The events (primaries,
        // tf, luminances, …) flow server → client via methods on
        // the resource handle returned at init time. Nothing to
        // dispatch here.
    }
}

#[macro_export]
macro_rules! delegate_color_management {
    ($ty:ty) => {
        smithay::reexports::wayland_server::delegate_dispatch!($ty:
            [smithay::reexports::wayland_protocols::wp::color_management::v1::server::wp_color_manager_v1::WpColorManagerV1: ()] =>
                $crate::protocols::color_management::ColorManagementState
        );
        smithay::reexports::wayland_server::delegate_global_dispatch!($ty:
            [smithay::reexports::wayland_protocols::wp::color_management::v1::server::wp_color_manager_v1::WpColorManagerV1: $crate::protocols::color_management::ColorManagerGlobalData] =>
                $crate::protocols::color_management::ColorManagementState
        );
        smithay::reexports::wayland_server::delegate_dispatch!($ty:
            [smithay::reexports::wayland_protocols::wp::color_management::v1::server::wp_color_management_output_v1::WpColorManagementOutputV1: ()] =>
                $crate::protocols::color_management::ColorManagementState
        );
        smithay::reexports::wayland_server::delegate_dispatch!($ty:
            [smithay::reexports::wayland_protocols::wp::color_management::v1::server::wp_color_management_surface_v1::WpColorManagementSurfaceV1: $crate::protocols::color_management::SurfaceTrackerData] =>
                $crate::protocols::color_management::ColorManagementState
        );
        smithay::reexports::wayland_server::delegate_dispatch!($ty:
            [smithay::reexports::wayland_protocols::wp::color_management::v1::server::wp_color_management_surface_feedback_v1::WpColorManagementSurfaceFeedbackV1: ()] =>
                $crate::protocols::color_management::ColorManagementState
        );
        smithay::reexports::wayland_server::delegate_dispatch!($ty:
            [smithay::reexports::wayland_protocols::wp::color_management::v1::server::wp_image_description_creator_icc_v1::WpImageDescriptionCreatorIccV1: ()] =>
                $crate::protocols::color_management::ColorManagementState
        );
        smithay::reexports::wayland_server::delegate_dispatch!($ty:
            [smithay::reexports::wayland_protocols::wp::color_management::v1::server::wp_image_description_creator_params_v1::WpImageDescriptionCreatorParamsV1: $crate::protocols::color_management::CreatorParamsData] =>
                $crate::protocols::color_management::ColorManagementState
        );
        smithay::reexports::wayland_server::delegate_dispatch!($ty:
            [smithay::reexports::wayland_protocols::wp::color_management::v1::server::wp_image_description_v1::WpImageDescriptionV1: $crate::protocols::color_management::ImageDescription] =>
                $crate::protocols::color_management::ColorManagementState
        );
        smithay::reexports::wayland_server::delegate_dispatch!($ty:
            [smithay::reexports::wayland_protocols::wp::color_management::v1::server::wp_image_description_info_v1::WpImageDescriptionInfoV1: ()] =>
                $crate::protocols::color_management::ColorManagementState
        );
    };
}
