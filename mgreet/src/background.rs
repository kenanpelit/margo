//! The greeter's backdrop.
//!
//! `mlogind`'s theme sync bakes a downscaled, blurred copy of the desktop's
//! wallpaper into `/var/lib/mgreet/background.raw` — `[u32 LE width][u32 LE
//! height][RGBA]`, the same header `mshell` writes its own wallpaper cache with.
//! Already decoded, so nothing here links an image library; already blurred, so
//! nothing here touches the GPU beyond uploading a 2 MB texture once.
//!
//! Absent or malformed, there is simply no backdrop and the greeter renders the
//! flat scrim it always has. A login screen that cannot find its wallpaper still
//! logs you in.

use gtk4 as gtk;

use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;

/// Written by `mlogind`'s theme sync, world-readable. Machine-written, so it
/// lives in `/var/lib` rather than `/etc` — see `docs/config-conventions.md`.
const PATH: &str = "/var/lib/mgreet/background.raw";

const HEADER: usize = 8;

/// The long edge the sync bounds its output to. Anything larger did not come
/// from the sync, whatever the header claims.
const MAX_EDGE: u32 = 960;

/// `(width, height, pixels)` of a `[u32 LE w][u32 LE h][RGBA]` buffer.
///
/// Rejects a header that disagrees with the byte count, a zero dimension, and a
/// `w*h*4` that would overflow rather than wrapping into a length that happens
/// to match.
fn parse(bytes: &[u8]) -> Option<(u32, u32, &[u8])> {
    if bytes.len() < HEADER {
        return None;
    }
    let width = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let height = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);

    if width == 0 || height == 0 || width > MAX_EDGE || height > MAX_EDGE {
        return None;
    }
    let body = (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(4)?;
    if bytes.len() != body.checked_add(HEADER)? {
        return None;
    }
    Some((width, height, &bytes[HEADER..]))
}

/// A background the runner picked from `[display] background_dir` — one random
/// image per greeter start, the same file the compositor wallpaper shows, so
/// the two can never disagree. Decoded by GTK's own loaders; mgreet still
/// links no image crate. Not blurred: the `.mgreet-dim` overlay carries
/// legibility for a photographic backdrop.
fn from_env() -> Option<gdk::Texture> {
    let path = std::env::var_os("MLOGIND_BACKGROUND")?;
    match gdk::Texture::from_filename(&path) {
        Ok(texture) => Some(texture),
        Err(err) => {
            // Fall back to the baked backdrop rather than the flat scrim —
            // a background that cannot be decoded should degrade to the one
            // that always can be.
            eprintln!(
                "[mgreet] cannot decode background {path:?} ({err}); using the baked backdrop"
            );
            None
        }
    }
}

/// The backdrop texture: the runner's `background_dir` pick when there is
/// one, else the synced blurred wallpaper copy. Uploaded once and shared by
/// every per-monitor window — `gdk::Texture` is refcounted, so cloning is free.
pub fn load() -> Option<gdk::Texture> {
    if let Some(texture) = from_env() {
        return Some(texture);
    }
    let bytes = std::fs::read(PATH).ok()?;
    let (width, height, pixels) = parse(&bytes)?;

    let texture = gdk::MemoryTexture::new(
        width as i32,
        height as i32,
        gdk::MemoryFormat::R8g8b8a8,
        &glib::Bytes::from(pixels),
        width as usize * 4,
    );
    Some(texture.upcast())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(width: u32, height: u32) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&width.to_le_bytes());
        buf.extend_from_slice(&height.to_le_bytes());
        buf.resize(HEADER + (width as usize) * (height as usize) * 4, 0xAB);
        buf
    }

    #[test]
    fn a_well_formed_buffer_yields_its_pixels() {
        let buf = raw(4, 3);
        let (w, h, pixels) = parse(&buf).expect("a good buffer parses");
        assert_eq!((w, h), (4, 3));
        assert_eq!(pixels.len(), 4 * 3 * 4);
    }

    #[test]
    fn a_truncated_buffer_is_rejected_rather_than_indexed() {
        assert!(parse(&[]).is_none());
        assert!(parse(&[1, 2, 3, 4]).is_none());
        let mut buf = raw(4, 3);
        buf.pop();
        assert!(parse(&buf).is_none());
    }

    #[test]
    fn a_lying_header_is_rejected() {
        let mut buf = raw(4, 3);
        buf[0] = 9; // claims 9×3, carries 4×3
        assert!(parse(&buf).is_none());
    }

    #[test]
    fn a_zero_dimension_is_rejected() {
        assert!(parse(&raw(0, 3)).is_none());
        assert!(parse(&raw(4, 0)).is_none());
    }

    #[test]
    fn an_image_larger_than_the_sync_would_ever_write_is_rejected() {
        // Not from our sync. Refuse it rather than upload an unbounded texture
        // in the process that gates the machine.
        let mut buf = Vec::new();
        buf.extend_from_slice(&(MAX_EDGE + 1).to_le_bytes());
        buf.extend_from_slice(&1u32.to_le_bytes());
        buf.resize(HEADER + (MAX_EDGE as usize + 1) * 4, 0);
        assert!(parse(&buf).is_none());
    }

    #[test]
    fn a_dimension_product_that_would_overflow_is_rejected() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&u32::MAX.to_le_bytes());
        buf.extend_from_slice(&u32::MAX.to_le_bytes());
        buf.resize(HEADER + 4, 0);
        assert!(parse(&buf).is_none());
    }
}
