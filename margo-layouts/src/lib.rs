//! Layout arithmetic for margo's 11 tiling algorithms.
//!
//! Pure functions — no Wayland, smithay, or wlroots dependencies. The
//! compositor binary re-exports this crate via `crate::layout::*`; the
//! `mvisual` design tool consumes the same algorithms to render
//! interactive previews of every layout × per-tag pinning combination.

mod algorithms;
pub use algorithms::*;

/// Axis-aligned bounding box.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Rect {
    pub fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    pub fn area(&self) -> i32 {
        self.width * self.height
    }

    /// The rect a client's border/shadow should actually be drawn at:
    /// `self` (the compositor-assigned slot -- `float_geom` for a
    /// floating window, the tile slot otherwise) shrunk on whichever
    /// axis `actual` (the client's own live committed buffer size) is
    /// smaller. Anchored at the slot's own top-left, never grown past
    /// the slot (a client larger than its slot is already being
    /// clipped elsewhere).
    ///
    /// Exists because a floating window's slot is pinned by its
    /// window rule and doesn't shrink when the client itself does --
    /// e.g. mtune switching from its full skin to a much smaller
    /// mini/strip skin inside a windowrule-fixed float_geom. Border
    /// already special-cased this inline; shadow used the raw slot
    /// unconditionally and never followed the client down.
    pub fn clamped_to_actual_size(&self, actual_width: i32, actual_height: i32) -> Rect {
        let mut r = *self;
        if actual_width > 0 && actual_width < r.width {
            r.width = actual_width;
        }
        if actual_height > 0 && actual_height < r.height {
            r.height = actual_height;
        }
        r
    }
}

/// Redistribute a 1-D stack of sibling sizes that share a leading edge
/// and a total span (a tile-family layout's master or stack column:
/// same x + width, consecutive along y) once each member's own
/// protocol-declared minimum size ("floor") has been applied.
///
/// A layout hands every column member a size that sums, with the gaps
/// between them, to exactly `span`. Growing one member up to a floor
/// bigger than its share can push that sum past `span` — most often a
/// stacked client's own xdg_toplevel minimum size exceeding its fair
/// share of the column. Left alone, that either spills the column
/// past the work area (the last member hangs off the bottom of the
/// screen) or, in a layout with side-by-side columns, overlaps
/// whatever sits next to it.
///
/// This shrinks whichever members are still at their natural,
/// layout-given size (i.e. their floor doesn't exceed it) proportionally
/// to make room, down to their own floor (or 1px if they have none),
/// so the group's total keeps summing to `span` — nothing spills past
/// the column's own span or overlaps a neighbour. A group whose summed
/// floors alone already exceed `span` is left at the summed floors: an
/// irreducible case (every member already at the smallest it declared
/// it can be) that no repacking can fix without breaking someone's own
/// protocol-declared minimum.
///
/// `sizes` and `floors` are parallel, one entry per member in their
/// original (position) order — `floors[i] <= 0` means "no floor".
/// `gaps` holds the original gap between each consecutive pair
/// (`sizes.len() - 1` entries). Returns each member's new size, same
/// order and length as `sizes`.
pub fn repack_1d(sizes: &[i32], floors: &[i32], gaps: &[i32], span: i32) -> Vec<i32> {
    let n = sizes.len();
    if n == 0 {
        return vec![];
    }
    let gap_total: i32 = gaps.iter().sum();
    // Leave at least 1px per member even if gaps alone would eat the
    // whole span (a pathological gap config) — matches the "floor
    // every layout rect to a positive size" contract every caller of
    // `layout::arrange` already relies on.
    let content_span = (span - gap_total).max(n as i32);

    let floored: Vec<i32> = sizes.iter().zip(floors).map(|(&s, &f)| s.max(f)).collect();
    let floored_total: i32 = floored.iter().sum();
    if floored_total <= content_span {
        // Applying every floor (without shrinking anyone) already fits
        // the span — nothing to redistribute.
        return floored.iter().map(|&h| h.max(1)).collect();
    }

    // Total span the floor-bound members (their own floor exceeds
    // their natural share) actually take.
    let floor_bound_total: i32 = sizes
        .iter()
        .zip(floors)
        .filter(|&(&s, &f)| f > s)
        .map(|(_, &f)| f)
        .sum();
    let flexible_natural: i32 = sizes
        .iter()
        .zip(floors)
        .filter(|&(&s, &f)| f <= s)
        .map(|(&s, _)| s)
        .sum();
    // What's left of the span for every member that's still free to
    // shrink, after the floor-bound members take their (bigger than
    // natural) share. Can go negative when floors alone overflow the
    // span — the irreducible case, handled by the `.max(1)` floor
    // below (each flexible member still gets at least 1px, so the
    // group's total honestly reports the overflow rather than lying
    // about it via zero-width windows).
    let available_for_flexible = content_span - floor_bound_total;

    sizes
        .iter()
        .zip(floors)
        .map(|(&s, &f)| {
            if f > s {
                f
            } else if flexible_natural > 0 {
                ((s as i64 * available_for_flexible.max(0) as i64) / flexible_natural as i64).max(1)
                    as i32
            } else {
                s.max(1)
            }
        })
        .collect()
}

/// Per-tag layout state stored on each monitor.
#[derive(Debug, Clone)]
pub struct Pertag {
    /// Current tag index (1-based).
    pub curtag: usize,
    /// Previous tag index.
    pub prevtag: usize,
    /// Layouts per tag (indexed 0 = overview, 1..=MAXTAGS).
    pub ltidxs: Vec<LayoutId>,
    /// mfact per tag.
    pub mfacts: Vec<f32>,
    /// nmaster per tag.
    pub nmasters: Vec<u32>,
    /// Gap config per tag.
    pub gaps: Vec<GapConfig>,
    /// `true` for tags where the user explicitly picked the layout
    /// via `setlayout` / `switch_layout` action. Adaptive layout
    /// (`Config::auto_layout = true`) skips auto-selection on these
    /// tags so a deliberate user choice is never overridden by a
    /// heuristic.
    pub user_picked_layout: Vec<bool>,
    /// Per-tag wallpaper hint set by `tagrule = id:N, wallpaper:path`
    /// (W3.6). Compositor stores the string verbatim; wallpaper
    /// daemons (swaybg / noctalia / custom) read it from the
    /// dwl-ipc broadcast or state.json on tag-switch and swap
    /// accordingly. Empty string = "no per-tag override; use the
    /// session-default wallpaper".
    pub wallpapers: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct GapConfig {
    pub gappih: i32,
    pub gappiv: i32,
    pub gappoh: i32,
    pub gappov: i32,
}

pub const MAX_TAGS: usize = 9;

impl Pertag {
    pub fn new(default_layout: LayoutId, default_mfact: f32, default_nmaster: u32) -> Self {
        Pertag {
            curtag: 1,
            prevtag: 1,
            ltidxs: vec![default_layout; MAX_TAGS + 1],
            mfacts: vec![default_mfact; MAX_TAGS + 1],
            nmasters: vec![default_nmaster; MAX_TAGS + 1],
            gaps: vec![GapConfig::default(); MAX_TAGS + 1],
            user_picked_layout: vec![false; MAX_TAGS + 1],
            wallpapers: vec![String::new(); MAX_TAGS + 1],
        }
    }

    /// Override per-tag default layouts from config `taglayout` entries —
    /// `(tag_1_based, layout_name)`. Out-of-range tags and unknown layout
    /// names are ignored. Marks each seeded tag as user-picked so the
    /// auto-layout heuristic doesn't override an explicit choice.
    pub fn seed_taglayouts(&mut self, taglayouts: &[(u32, String)]) {
        for (tag, name) in taglayouts {
            let t = *tag as usize;
            if t == 0 || t >= self.ltidxs.len() {
                continue;
            }
            if let Some(id) = LayoutId::from_name(name) {
                self.ltidxs[t] = id;
                if t < self.user_picked_layout.len() {
                    self.user_picked_layout[t] = true;
                }
            }
        }
    }
}

/// Layout identifier matching C `enum { TILE, SCROLLER, ... }`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LayoutId {
    #[default]
    Tile,
    Scroller,
    Grid,
    Monocle,
    Deck,
    CenterTile,
    RightTile,
    TgMix,
    Dwindle,
    /// Stacking / floating desktop — the tiler produces no geometry;
    /// every client keeps its own `float_geom`. The compositor
    /// auto-floats tiled clients and cascades their placement in
    /// `reconcile_floating_layout`.
    Floating,
    /// Content-aware, self-arranging desktop (GNOME's "Mosaic" concept).
    /// Like `Floating`, the tiler produces no geometry here — the
    /// compositor auto-floats governed clients and packs them with
    /// `mosaic_arrange` in `reconcile_mosaic_layout`, honouring each
    /// client's real xdg_toplevel min/max size and its own requested
    /// ("ideal") size instead of a fixed grid.
    Mosaic,
    Overview,
}

impl LayoutId {
    pub fn symbol(&self) -> &'static str {
        match self {
            LayoutId::Tile => "T",
            LayoutId::Scroller => "S",
            LayoutId::Grid => "G",
            LayoutId::Monocle => "M",
            LayoutId::Deck => "K",
            LayoutId::CenterTile => "CT",
            LayoutId::RightTile => "RT",
            LayoutId::TgMix => "TG",
            LayoutId::Dwindle => "DW",
            LayoutId::Floating => "F",
            LayoutId::Mosaic => "MO",
            LayoutId::Overview => "󰃇",
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            LayoutId::Tile => "tile",
            LayoutId::Scroller => "scroller",
            LayoutId::Grid => "grid",
            LayoutId::Monocle => "monocle",
            LayoutId::Deck => "deck",
            LayoutId::CenterTile => "center_tile",
            LayoutId::RightTile => "right_tile",
            LayoutId::TgMix => "tgmix",
            LayoutId::Dwindle => "dwindle",
            LayoutId::Floating => "floating",
            LayoutId::Mosaic => "mosaic",
            LayoutId::Overview => "overview",
        }
    }

    pub fn from_symbol(s: &str) -> Option<Self> {
        let all = [
            LayoutId::Tile,
            LayoutId::Scroller,
            LayoutId::Grid,
            LayoutId::Monocle,
            LayoutId::Deck,
            LayoutId::CenterTile,
            LayoutId::RightTile,
            LayoutId::TgMix,
            LayoutId::Dwindle,
            LayoutId::Floating,
            LayoutId::Mosaic,
        ];
        all.iter().find(|l| l.symbol() == s).copied()
    }

    pub fn from_name(s: &str) -> Option<Self> {
        let all = [
            LayoutId::Tile,
            LayoutId::Scroller,
            LayoutId::Grid,
            LayoutId::Monocle,
            LayoutId::Deck,
            LayoutId::CenterTile,
            LayoutId::RightTile,
            LayoutId::TgMix,
            LayoutId::Dwindle,
            LayoutId::Floating,
            LayoutId::Mosaic,
        ];
        all.iter().find(|l| l.name() == s).copied()
    }

    /// All layouts offered in catalogues / pickers (excludes
    /// `Overview`, which is rendered via a separate code path).
    /// "Tileable" is historical — the list includes `Floating`, which
    /// produces no tiled geometry. Useful for `mvisual` and any
    /// catalogue UI.
    pub fn all_tileable() -> &'static [LayoutId] {
        &[
            LayoutId::Tile,
            LayoutId::Scroller,
            LayoutId::Grid,
            LayoutId::Monocle,
            LayoutId::Deck,
            LayoutId::CenterTile,
            LayoutId::RightTile,
            LayoutId::TgMix,
            LayoutId::Dwindle,
            LayoutId::Floating,
            LayoutId::Mosaic,
        ]
    }
}

/// Geometry list for a single arrange pass.
pub type ArrangeResult = Vec<(usize, Rect)>;

/// Context passed to every layout algorithm.
pub struct ArrangeCtx<'a> {
    /// Available window area on the monitor.
    pub work_area: Rect,
    /// Tiled clients to arrange (indices into the compositor's client list).
    pub tiled: &'a [usize],
    /// Number of master windows.
    pub nmaster: u32,
    /// Master factor (fraction of width/height for the master area).
    pub mfact: f32,
    /// Gap config.
    pub gaps: &'a GapConfig,
    /// Scroller proportion for each client.
    pub scroller_proportions: &'a [f32],
    /// Default scroller proportion.
    pub default_scroller_proportion: f32,
    /// Position of the focused client inside `tiled`, when any.
    pub focused_tiled_pos: Option<usize>,
    /// Mango-style side margin used by scroller layouts.
    pub scroller_structs: i32,
    /// Keep the focused scroller client centered.
    pub scroller_focus_center: bool,
    /// Prefer centering when scrolling to another client.
    pub scroller_prefer_center: bool,
    /// Prefer edge overspread for first/last scroller clients.
    pub scroller_prefer_overspread: bool,
}
