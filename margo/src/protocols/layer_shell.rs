#![allow(dead_code)]
/// wlr-layer-shell-unstable-v1 state.
/// Layer-shell surfaces (bars, overlays, notifications) are placed on one of
/// the scene layers and excluded from tiling layout.
use crate::layout::Rect;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    Background,
    Bottom,
    Top,
    Overlay,
}

#[derive(Debug)]
pub struct LayerSurface {
    pub id: u64,
    pub output_name: String,
    pub layer: Layer,
    pub geometry: Rect,
    pub exclusive_zone: i32,
    pub mapped: bool,
    pub animation_type_open: String,
    pub animation_type_close: String,
}

impl LayerSurface {
    /// Returns true if this surface steals space from the work area.
    pub fn affects_work_area(&self) -> bool {
        self.exclusive_zone > 0
    }
}
