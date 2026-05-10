//! Infinite Spatial Overview — Phase 3 foundation.
//!
//! Design document: `docs/design/spatial-overview.md`. Read that
//! first; this module is the implementation kernel. Three commits
//! land Phase 3:
//!
//! 1. **This file + design doc** — types, camera math, world layout
//!    helpers, config + mode enum. No behaviour change yet — Grid
//!    overview stays the default.
//! 2. Next commit — `MargoState::spatial` field, `arrange_monitor`
//!    spatial branch, render path passthrough.
//! 3. Final commit — input handlers (mouse pan, scroll zoom,
//!    keyboard pan/zoom dispatches), frame-tick momentum decay.
//!
//! Margin between coordinate spaces:
//!
//! * **World** — every tag has a fixed anchor `(tag_world_x,
//!   tag_world_y)`; each client's `geom` *inside its tag's slot* is
//!   the layout's normal output (Tile / Scroller / Grid / …). World
//!   units = logical pixels at zoom 1.0.
//! * **Camera viewport** — `(cam_x, cam_y, zoom)` defines what
//!   subset of world space the user is currently looking at.
//! * **Screen** — physical pixels post-DRM transform; smithay
//!   handles this last leg.
//!
//! The two transforms callers need:
//!
//! ```ignore
//! screen = (world - camera_origin) * zoom + work_area_origin
//! world  = (screen - work_area_origin) / zoom + camera_origin
//! ```
//!
//! Centralised in [`world_to_screen`] and [`screen_to_world`];
//! arrange, render, and input all go through these so the three
//! sites can't drift out of step the way the 0.1.8 per-tag grid
//! helpers did.

#![allow(dead_code)]

use serde::{Deserialize, Serialize};

use margo_layouts::Rect;

/// Which overview style is active.
///
/// `Spatial` is the Phase 3 default once commits 2 + 3 land; for now
/// (commit 1) the config field exists and serializes round-trip but
/// `arrange_monitor` still always takes the `Grid` branch. Flipping
/// the default at the *config* level decouples the foundation from
/// the live-rendering changes — easier to bisect if a regression
/// shows up later.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum OverviewMode {
    /// Legacy single-Grid arrangement over the zoomed work area —
    /// every client visible in one big Grid, every tag mixed
    /// together. Kept as opt-in fallback (`overview_mode = grid`)
    /// for users who don't want the spatial canvas.
    Grid,
    /// Infinite spatial canvas — every tag has its own slot in
    /// world space, camera pans / zooms over them. Default starting
    /// commit 2.
    #[default]
    Spatial,
}

impl OverviewMode {
    /// Parser helper for the `overview_mode = ...` config line.
    /// Unknown values fall back to `Spatial` (the default) with a
    /// trace log so the user sees what happened.
    pub fn from_config_str(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "grid" | "legacy" | "flat" => Self::Grid,
            "spatial" | "infinite" | "canvas" => Self::Spatial,
            other => {
                tracing::warn!(
                    target: "spatial_overview",
                    value = %other,
                    "unknown overview_mode, falling back to Spatial",
                );
                Self::Spatial
            }
        }
    }
}

// ── Camera ───────────────────────────────────────────────────────────────────

/// Lower bound for the spatial-overview zoom. At 0.1 the entire 3×3
/// world (9 monitor-sized tag slots + padding) fits on a 1080p
/// panel — every tag visible at ~360 px square. Going below this is
/// readable territory, beyond it everything turns into tiny squares.
pub const ZOOM_MIN: f64 = 0.1;

/// Upper bound — zoomed beyond 1:1 is mostly for accessibility
/// ("squint at this thumbnail"); above 1.5 the user might as well
/// just leave overview and use the live tag.
pub const ZOOM_MAX: f64 = 1.5;

/// Velocity floor below which we treat momentum as zero. Prevents
/// the camera from drifting indefinitely after a soft release.
/// Tuned to 0.5 logical px/frame — anything slower is invisible at
/// 60 Hz over a 16 ms frame anyway.
pub const VELOCITY_FLOOR: f64 = 0.5;

/// Per-frame friction multiplier applied to momentum (`v *= FRICTION`
/// every tick). 0.92 at 60 Hz gives a paperwm-like "carries you a
/// little, then stops" feel. The same constant at 144 Hz settles
/// roughly 2.4× faster — that's intentional, faster panels deserve
/// snappier settle.
pub const FRICTION: f64 = 0.92;

/// World-space camera state. One per monitor (see commit 2 — for
/// now the foundation API takes a `&SpatialCamera` and is unbound).
///
/// The camera carries both a *current* position (`x, y, zoom`) and a
/// *target* it interpolates toward. Pan / zoom inputs set the target;
/// the per-frame tick (commit 3) drives the current toward the
/// target with critical-damped spring math, then bleeds momentum
/// (`vx, vy, vzoom`) into the target so a fast flick "carries"
/// after the user releases.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpatialCamera {
    pub x: f64,
    pub y: f64,
    pub zoom: f64,
    pub target_x: f64,
    pub target_y: f64,
    pub target_zoom: f64,
    pub vx: f64,
    pub vy: f64,
    pub vzoom: f64,
}

impl Default for SpatialCamera {
    fn default() -> Self {
        // Centered on the 3×3 world grid at zoom 0.5 — opens overview
        // at "every tag visible, balanced". `MargoState::open_overview`
        // will overwrite x/y with the active tag's centre + zoom from
        // `Config::overview_zoom` once commit 2 lands; this default is
        // for unit tests and for the foundation API.
        let world_w = 3.0 * 1920.0; // mocked panel size for default
        let world_h = 3.0 * 1080.0;
        Self {
            x: world_w / 2.0,
            y: world_h / 2.0,
            zoom: 0.5,
            target_x: world_w / 2.0,
            target_y: world_h / 2.0,
            target_zoom: 0.5,
            vx: 0.0,
            vy: 0.0,
            vzoom: 0.0,
        }
    }
}

impl SpatialCamera {
    /// Re-centre the camera on the given world-space point at the
    /// given zoom. Used by `open_overview_spatial` and by the
    /// alt+Tab cycle to pan to the newly-hovered thumbnail.
    pub fn snap_to(&mut self, x: f64, y: f64, zoom: f64) {
        let zoom = zoom.clamp(ZOOM_MIN, ZOOM_MAX);
        self.target_x = x;
        self.target_y = y;
        self.target_zoom = zoom;
        // Hard-snap current = target for `snap_to`; smooth pan/zoom
        // goes through `pan_to` / `zoom_to_target` below.
        self.x = x;
        self.y = y;
        self.zoom = zoom;
        self.vx = 0.0;
        self.vy = 0.0;
        self.vzoom = 0.0;
    }

    /// Set a new pan target without snapping. Per-frame tick will
    /// interpolate current toward target.
    pub fn pan_to(&mut self, x: f64, y: f64) {
        self.target_x = x;
        self.target_y = y;
    }

    /// Set a new zoom target without snapping.
    pub fn zoom_to_target(&mut self, zoom: f64) {
        self.target_zoom = zoom.clamp(ZOOM_MIN, ZOOM_MAX);
    }

    /// Increment camera target by a screen-space delta (e.g. a mouse
    /// drag). The delta is divided by current zoom because in world
    /// units a 10 px screen drag at zoom 0.5 covers 20 world px.
    pub fn pan_by_screen_delta(&mut self, dx: f64, dy: f64) {
        if self.zoom.abs() < f64::EPSILON {
            return;
        }
        self.target_x += dx / self.zoom;
        self.target_y += dy / self.zoom;
    }

    /// Zoom around a screen-space anchor (where the cursor is, for
    /// scroll-zoom). The world point under the cursor stays fixed
    /// across the zoom change — niri / paperwm / any sane mapping
    /// tool's default.
    pub fn zoom_around_screen_point(
        &mut self,
        factor: f64,
        screen_anchor: (f64, f64),
        work_area: Rect,
    ) {
        let old_zoom = self.target_zoom;
        let new_zoom = (old_zoom * factor).clamp(ZOOM_MIN, ZOOM_MAX);
        if (new_zoom - old_zoom).abs() < f64::EPSILON {
            return;
        }
        // Pivot: world point under screen_anchor stays put.
        let world_anchor = screen_to_world(screen_anchor, self, work_area);
        self.target_zoom = new_zoom;
        // Solve for cam_x/y such that world_to_screen(world_anchor) ==
        // screen_anchor under the new zoom. From the forward formula:
        //
        //   screen.x = (world.x - cam.x) * zoom + wa.x + wa.w/2
        //   ⇒ cam.x = world.x - (screen.x - wa.x - wa.w/2) / zoom
        //
        // Missing the wa.w/2 term was the unit-test failure on commit 1.
        let wa_origin_x = work_area.x as f64 + (work_area.width as f64) / 2.0;
        let wa_origin_y = work_area.y as f64 + (work_area.height as f64) / 2.0;
        self.target_x = world_anchor.0 - (screen_anchor.0 - wa_origin_x) / new_zoom;
        self.target_y = world_anchor.1 - (screen_anchor.1 - wa_origin_y) / new_zoom;
    }

    /// Per-frame integration step: apply velocity to target, bleed
    /// velocity through friction, smooth-interpolate current toward
    /// target. Called from commit 3's `tick_animations` site.
    pub fn tick(&mut self, dt_seconds: f64) {
        // Apply momentum to target.
        self.target_x += self.vx * dt_seconds;
        self.target_y += self.vy * dt_seconds;
        self.target_zoom = (self.target_zoom + self.vzoom * dt_seconds)
            .clamp(ZOOM_MIN, ZOOM_MAX);

        // Friction. Floor below VELOCITY_FLOOR snaps to zero so the
        // camera doesn't creep indefinitely after a soft release.
        self.vx *= FRICTION;
        self.vy *= FRICTION;
        self.vzoom *= FRICTION;
        if self.vx.abs() < VELOCITY_FLOOR {
            self.vx = 0.0;
        }
        if self.vy.abs() < VELOCITY_FLOOR {
            self.vy = 0.0;
        }
        if self.vzoom.abs() < 0.001 {
            self.vzoom = 0.0;
        }

        // Smooth-step current → target. Plain linear lerp at α=0.25
        // per frame is critically damped enough for this scale; we'll
        // wire the proper spring engine in commit 3 if measured feel
        // is worse than this baseline.
        const LERP_ALPHA: f64 = 0.25;
        self.x += (self.target_x - self.x) * LERP_ALPHA;
        self.y += (self.target_y - self.y) * LERP_ALPHA;
        self.zoom += (self.target_zoom - self.zoom) * LERP_ALPHA;
    }
}

// ── Coordinate transforms ────────────────────────────────────────────────────

/// World → screen. The single transform every consumer should use
/// to position a client's render rect on the monitor. Arrange-side
/// callers can compute client world geom once and feed it through
/// here; render-side and hit-test callers do the same.
pub fn world_to_screen(world: (f64, f64), cam: &SpatialCamera, work_area: Rect) -> (f64, f64) {
    let wa_origin = (work_area.x as f64, work_area.y as f64);
    (
        (world.0 - cam.x) * cam.zoom + wa_origin.0 + (work_area.width as f64) / 2.0,
        (world.1 - cam.y) * cam.zoom + wa_origin.1 + (work_area.height as f64) / 2.0,
    )
}

/// Screen → world. Inverse of `world_to_screen`; used by hit-tests
/// (which client is under the cursor?) and zoom-around-cursor math.
pub fn screen_to_world(screen: (f64, f64), cam: &SpatialCamera, work_area: Rect) -> (f64, f64) {
    if cam.zoom.abs() < f64::EPSILON {
        return (cam.x, cam.y);
    }
    let wa_origin = (work_area.x as f64, work_area.y as f64);
    (
        (screen.0 - wa_origin.0 - (work_area.width as f64) / 2.0) / cam.zoom + cam.x,
        (screen.1 - wa_origin.1 - (work_area.height as f64) / 2.0) / cam.zoom + cam.y,
    )
}

// ── World layout ─────────────────────────────────────────────────────────────

/// Logical-pixel padding around each tag slot in world space. Large
/// enough for visual separation at zoom 0.5, small enough not to
/// dwarf content at zoom 1.0. Configurable in commit 2
/// (`spatial_tag_padding` config field); for now hard-coded.
pub const TAG_PADDING: f64 = 64.0;

/// World-space anchor (top-left corner) for `tag` (1..=9), given a
/// monitor logical size. 3×3 layout — tag 1 top-left, tag 9
/// bottom-right (1-9 keypad mental model, matches the old per-tag
/// grid mapping users have already internalised from 0.1.8).
pub fn tag_anchor(tag: u32, monitor_w: i32, monitor_h: i32) -> (f64, f64) {
    let tag_idx = tag.saturating_sub(1).min(8) as i32;
    let col = tag_idx % 3;
    let row = tag_idx / 3;
    let slot_w = monitor_w as f64 + 2.0 * TAG_PADDING;
    let slot_h = monitor_h as f64 + 2.0 * TAG_PADDING;
    (
        col as f64 * slot_w + TAG_PADDING,
        row as f64 * slot_h + TAG_PADDING,
    )
}

/// World-space rect for a client given:
///   * its `tag` (1..=9),
///   * its `local_rect` (the rect the layout engine produced for
///     that tag in its own work-area-sized coordinates),
///   * the monitor logical size.
///
/// Combines tag anchor + local rect; arrange-side will call this for
/// every (tag, client) pair, store result on `client.world_geom`,
/// and let the render path transform world → screen via `world_to_screen`.
pub fn client_world_rect(
    tag: u32,
    local_rect: Rect,
    monitor_w: i32,
    monitor_h: i32,
) -> Rect {
    let (ax, ay) = tag_anchor(tag, monitor_w, monitor_h);
    Rect {
        x: (ax + local_rect.x as f64) as i32,
        y: (ay + local_rect.y as f64) as i32,
        width: local_rect.width,
        height: local_rect.height,
    }
}

/// Total world bounds for a 3×3 layout on a given monitor — used to
/// compute "is this camera position in valid world space?" clamps.
pub fn world_bounds(monitor_w: i32, monitor_h: i32) -> Rect {
    let slot_w = monitor_w as f64 + 2.0 * TAG_PADDING;
    let slot_h = monitor_h as f64 + 2.0 * TAG_PADDING;
    Rect {
        x: 0,
        y: 0,
        width: (3.0 * slot_w) as i32,
        height: (3.0 * slot_h) as i32,
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_to_world_inverts_world_to_screen() {
        let cam = SpatialCamera::default();
        let wa = Rect { x: 100, y: 50, width: 1920, height: 1080 };
        let world = (1234.5, -67.8);
        let screen = world_to_screen(world, &cam, wa);
        let back = screen_to_world(screen, &cam, wa);
        assert!((world.0 - back.0).abs() < 1e-6, "x: {} → {}", world.0, back.0);
        assert!((world.1 - back.1).abs() < 1e-6, "y: {} → {}", world.1, back.1);
    }

    #[test]
    fn round_trip_holds_at_various_zooms() {
        let wa = Rect { x: 0, y: 0, width: 1920, height: 1080 };
        for zoom in [0.1, 0.25, 0.5, 1.0, 1.5] {
            let cam = SpatialCamera {
                x: 500.0, y: 200.0, zoom,
                target_x: 500.0, target_y: 200.0, target_zoom: zoom,
                vx: 0.0, vy: 0.0, vzoom: 0.0,
            };
            let world = (888.0, 444.0);
            let screen = world_to_screen(world, &cam, wa);
            let back = screen_to_world(screen, &cam, wa);
            assert!((world.0 - back.0).abs() < 1e-4, "zoom {zoom}: x drift");
            assert!((world.1 - back.1).abs() < 1e-4, "zoom {zoom}: y drift");
        }
    }

    #[test]
    fn tag_anchors_are_monitor_size_aware() {
        // Slot width = 1920 + 2*64 = 2048. Tag 2 (col 1) anchor x =
        // 2048 + 64 = 2112. Tag 5 (row 1, col 1) anchor = (2112,
        // 1080 + 2*64 + 64) = (2112, 1272).
        let (x, y) = tag_anchor(5, 1920, 1080);
        assert!((x - 2112.0).abs() < 1e-9);
        assert!((y - 1272.0).abs() < 1e-9);
    }

    #[test]
    fn tag_anchor_is_monitor_independent_for_same_index() {
        // The anchor formula is purely a function of (tag, monitor
        // size); two calls with the same arguments must return the
        // same point. Useful invariant for multi-monitor work later.
        let a1 = tag_anchor(7, 2560, 1440);
        let a2 = tag_anchor(7, 2560, 1440);
        assert_eq!(a1, a2);
    }

    #[test]
    fn client_world_rect_offsets_by_tag_anchor() {
        // A client with local rect (100, 50, 200, 150) on tag 3
        // (col 2, row 0) should land at (2 × 2048 + 64 + 100,
        // 64 + 50, 200, 150) = (4260, 114, 200, 150).
        let local = Rect { x: 100, y: 50, width: 200, height: 150 };
        let world = client_world_rect(3, local, 1920, 1080);
        assert_eq!(world, Rect { x: 4260, y: 114, width: 200, height: 150 });
    }

    #[test]
    fn snap_to_clamps_zoom() {
        let mut cam = SpatialCamera::default();
        cam.snap_to(0.0, 0.0, 5.0); // way above ZOOM_MAX
        assert_eq!(cam.zoom, ZOOM_MAX);
        cam.snap_to(0.0, 0.0, 0.01); // way below ZOOM_MIN
        assert_eq!(cam.zoom, ZOOM_MIN);
    }

    #[test]
    fn pan_by_screen_delta_scales_with_zoom() {
        let mut cam = SpatialCamera::default();
        let before = cam.target_x;
        cam.zoom = 0.5;
        cam.pan_by_screen_delta(10.0, 0.0);
        // 10 screen px at zoom 0.5 = 20 world px.
        assert!((cam.target_x - before - 20.0).abs() < 1e-9);
    }

    #[test]
    fn zoom_around_screen_point_keeps_world_anchor_fixed() {
        let wa = Rect { x: 0, y: 0, width: 1920, height: 1080 };
        let mut cam = SpatialCamera::default();
        let screen_anchor = (800.0, 600.0);
        let world_anchor_before = screen_to_world(screen_anchor, &cam, wa);
        cam.zoom_around_screen_point(1.5, screen_anchor, wa);
        // Flush the smooth-step by setting current = target.
        cam.x = cam.target_x;
        cam.y = cam.target_y;
        cam.zoom = cam.target_zoom;
        let world_anchor_after = screen_to_world(screen_anchor, &cam, wa);
        assert!((world_anchor_before.0 - world_anchor_after.0).abs() < 1e-4);
        assert!((world_anchor_before.1 - world_anchor_after.1).abs() < 1e-4);
    }

    #[test]
    fn tick_decays_momentum_to_zero_below_floor() {
        let mut cam = SpatialCamera::default();
        cam.vx = 0.1; // below VELOCITY_FLOOR
        cam.vy = 0.3;
        cam.tick(1.0 / 60.0);
        assert_eq!(cam.vx, 0.0);
        assert_eq!(cam.vy, 0.0);
    }

    #[test]
    fn overview_mode_parser_round_trips_known_values() {
        assert_eq!(OverviewMode::from_config_str("grid"), OverviewMode::Grid);
        assert_eq!(OverviewMode::from_config_str("GRID"), OverviewMode::Grid);
        assert_eq!(OverviewMode::from_config_str("legacy"), OverviewMode::Grid);
        assert_eq!(OverviewMode::from_config_str("spatial"), OverviewMode::Spatial);
        assert_eq!(OverviewMode::from_config_str("INFINITE"), OverviewMode::Spatial);
        assert_eq!(OverviewMode::from_config_str("canvas"), OverviewMode::Spatial);
    }

    #[test]
    fn overview_mode_parser_falls_back_to_spatial_on_unknown() {
        assert_eq!(OverviewMode::from_config_str("nope"), OverviewMode::Spatial);
        assert_eq!(OverviewMode::from_config_str(""), OverviewMode::Spatial);
    }

    #[test]
    fn world_bounds_match_3x3_layout() {
        let b = world_bounds(1920, 1080);
        assert_eq!(b.width, 3 * (1920 + 2 * 64));
        assert_eq!(b.height, 3 * (1080 + 2 * 64));
    }
}
