#![allow(dead_code)]

//! Layout module — re-exports `margo-layouts`, the standalone crate
//! that holds the 14 tiling algorithms as pure functions. The split
//! lets `mvisual` consume the same arithmetic without dragging in
//! the smithay/wlroots dependency tree. See `margo-layouts/src/lib.rs`.

pub use margo_layouts::*;

#[cfg(test)]
mod snapshot_tests;
