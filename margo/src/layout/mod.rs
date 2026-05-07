#![allow(dead_code)]

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
        Rect { x, y, width, height }
    }

    pub fn area(&self) -> i32 {
        self.width * self.height
    }
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
    VerticalScroller,
    VerticalTile,
    VerticalGrid,
    VerticalDeck,
    TgMix,
    Canvas,
    Dwindle,
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
            LayoutId::VerticalScroller => "VS",
            LayoutId::VerticalTile => "VT",
            LayoutId::VerticalGrid => "VG",
            LayoutId::VerticalDeck => "VK",
            LayoutId::TgMix => "TG",
            LayoutId::Canvas => "CV",
            LayoutId::Dwindle => "DW",
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
            LayoutId::VerticalScroller => "vertical_scroller",
            LayoutId::VerticalTile => "vertical_tile",
            LayoutId::VerticalGrid => "vertical_grid",
            LayoutId::VerticalDeck => "vertical_deck",
            LayoutId::TgMix => "tgmix",
            LayoutId::Canvas => "canvas",
            LayoutId::Dwindle => "dwindle",
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
            LayoutId::VerticalScroller,
            LayoutId::VerticalTile,
            LayoutId::VerticalGrid,
            LayoutId::VerticalDeck,
            LayoutId::TgMix,
            LayoutId::Canvas,
            LayoutId::Dwindle,
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
            LayoutId::VerticalScroller,
            LayoutId::VerticalTile,
            LayoutId::VerticalGrid,
            LayoutId::VerticalDeck,
            LayoutId::TgMix,
            LayoutId::Canvas,
            LayoutId::Dwindle,
        ];
        all.iter().find(|l| l.name() == s).copied()
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
