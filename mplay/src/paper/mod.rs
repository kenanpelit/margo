//! Native video-wallpaper engine: wlr-layer-shell background surface +
//! EGL + libmpv render context. (Built out in the engine tasks.)

use crate::geometry::ScaleMode;
use anyhow::Result;

/// Wallpaper playback options.
pub struct PaperOpts {
    pub mute: bool,
    pub looping: bool,
    pub scale: ScaleMode,
}

/// Play `src` as a wallpaper on `output` (or all outputs).
pub fn run(_src: &str, _output: Option<&str>, _opts: PaperOpts, _daemon: bool) -> Result<()> {
    anyhow::bail!("wallpaper engine not yet implemented")
}

/// Stop the wallpaper on `output` (or all).
pub fn stop(_output: Option<&str>) -> Result<()> {
    anyhow::bail!("wallpaper engine not yet implemented")
}
