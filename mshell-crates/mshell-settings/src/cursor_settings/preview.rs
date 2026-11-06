//! Real cursor previews: read a theme's Xcursor files and pick the image to
//! show, so the picker shows each theme's own arrow, hand, beam, … instead
//! of a name. Decoding is plain file IO + parsing; the page runs it off the
//! GTK thread and only turns the pixels into textures on it.

use std::path::Path;

/// The shapes a preview shows: the cursor-spec name first, then the
/// X11 / legacy names themes ship instead.
pub(crate) const PREVIEW_SHAPES: &[(&str, &[&str])] = &[
    ("default", &["left_ptr", "arrow", "top_left_arrow"]),
    ("pointer", &["hand2", "pointing_hand", "hand1", "hand"]),
    ("text", &["xterm", "ibeam"]),
    ("wait", &["watch"]),
    ("progress", &["left_ptr_watch", "half-busy"]),
    ("crosshair", &["cross", "tcross"]),
    ("grabbing", &["closedhand", "fleur", "dnd-move"]),
    ("not-allowed", &["crossed_circle", "forbidden", "circle"]),
];

/// One decoded cursor image: premultiplied BGRA bytes (the Xcursor file
/// layout, which `gdk::MemoryFormat::B8g8r8a8Premultiplied` takes as is).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CursorImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// The nominal size closest to `want` among `sizes` (ties: the larger).
pub(crate) fn nearest_size(sizes: &[u32], want: u32) -> Option<u32> {
    sizes
        .iter()
        .copied()
        .min_by_key(|&s| (s.abs_diff(want), std::cmp::Reverse(s)))
}

/// The distinct nominal sizes in an Xcursor file, ascending.
pub(crate) fn sizes_in(images: &[xcursor::parser::Image]) -> Vec<u32> {
    let mut sizes: Vec<u32> = images.iter().map(|i| i.size).collect();
    sizes.sort_unstable();
    sizes.dedup();
    sizes
}

/// The first frame at the size nearest `want`.
pub(crate) fn pick_image(images: &[xcursor::parser::Image], want: u32) -> Option<CursorImage> {
    let size = nearest_size(&sizes_in(images), want)?;
    let image = images.iter().find(|i| i.size == size)?;
    let expected = (image.width as usize) * (image.height as usize) * 4;
    (image.width > 0 && image.height > 0 && image.pixels_rgba.len() == expected).then(|| {
        CursorImage {
            width: image.width,
            height: image.height,
            pixels: image.pixels_rgba.clone(),
        }
    })
}

/// Parse the cursor file `name` (or the first alternative that exists) in
/// `theme_dir/cursors`.
fn load_shape(theme_dir: &Path, names: &[&str]) -> Option<Vec<xcursor::parser::Image>> {
    names.iter().find_map(|name| {
        let bytes = std::fs::read(theme_dir.join("cursors").join(name)).ok()?;
        xcursor::parser::parse_xcursor(&bytes).filter(|v| !v.is_empty())
    })
}

/// A theme's preview: one image per [`PREVIEW_SHAPES`] entry it has, at the
/// size nearest `want`, plus the sizes its arrow comes in.
pub(crate) fn load_preview(theme_dir: &Path, want: u32) -> (Vec<CursorImage>, Vec<u32>) {
    let mut sizes = Vec::new();
    let mut images = Vec::new();
    for (i, (name, alts)) in PREVIEW_SHAPES.iter().enumerate() {
        let names: Vec<&str> = std::iter::once(*name).chain(alts.iter().copied()).collect();
        let Some(parsed) = load_shape(theme_dir, &names) else {
            continue;
        };
        if i == 0 {
            sizes = sizes_in(&parsed);
        }
        if let Some(image) = pick_image(&parsed, want) {
            images.push(image);
        }
    }
    (images, sizes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-image-per-size Xcursor file, the format libXcursor writes.
    fn xcursor_file(sizes: &[u32]) -> Vec<u8> {
        let mut out = Vec::new();
        let ntoc = sizes.len() as u32;
        out.extend_from_slice(b"Xcur");
        out.extend_from_slice(&16u32.to_le_bytes()); // header size
        out.extend_from_slice(&0x1_0000u32.to_le_bytes()); // version
        out.extend_from_slice(&ntoc.to_le_bytes());
        let toc_end = 16 + 12 * ntoc;
        let mut offset = toc_end;
        let mut chunks = Vec::new();
        for &size in sizes {
            let (w, h) = (size, size);
            let mut chunk = Vec::new();
            chunk.extend_from_slice(&36u32.to_le_bytes()); // chunk header
            chunk.extend_from_slice(&0xfffd_0002u32.to_le_bytes()); // image type
            chunk.extend_from_slice(&size.to_le_bytes()); // subtype = nominal size
            chunk.extend_from_slice(&1u32.to_le_bytes()); // version
            for v in [w, h, 0, 0, 50] {
                chunk.extend_from_slice(&v.to_le_bytes()); // w h xhot yhot delay
            }
            for _ in 0..(w * h) {
                chunk.extend_from_slice(&[0x10, 0x20, 0x30, 0xff]);
            }
            out.extend_from_slice(&0xfffd_0002u32.to_le_bytes());
            out.extend_from_slice(&size.to_le_bytes());
            out.extend_from_slice(&offset.to_le_bytes());
            offset += chunk.len() as u32;
            chunks.push(chunk);
        }
        for c in chunks {
            out.extend_from_slice(&c);
        }
        out
    }

    #[test]
    fn nearest_size_prefers_the_closest_then_the_larger() {
        assert_eq!(nearest_size(&[24, 32, 48], 30), Some(32));
        assert_eq!(nearest_size(&[24, 32], 28), Some(32), "tie: larger");
        assert_eq!(nearest_size(&[24, 32, 48], 96), Some(48));
        assert_eq!(nearest_size(&[], 24), None);
    }

    #[test]
    fn preview_reads_shapes_by_name_or_alias_at_the_nearest_size() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cursors = tmp.path().join("cursors");
        std::fs::create_dir_all(&cursors).expect("mkdir");
        std::fs::write(cursors.join("left_ptr"), xcursor_file(&[24, 32, 48])).expect("write");
        std::fs::write(cursors.join("hand2"), xcursor_file(&[24])).expect("write");
        std::fs::write(cursors.join("xterm"), b"not an xcursor").expect("write");
        let (images, sizes) = load_preview(tmp.path(), 30);
        assert_eq!(sizes, [24, 32, 48]);
        assert_eq!(images.len(), 2, "arrow + hand; the broken beam is skipped");
        assert_eq!((images[0].width, images[0].height), (32, 32));
        assert_eq!(images[0].pixels.len(), 32 * 32 * 4);
        assert_eq!(&images[0].pixels[..4], &[0x10, 0x20, 0x30, 0xff]);
        assert_eq!(images[1].width, 24);
    }

    #[test]
    fn a_theme_without_cursors_previews_nothing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        assert_eq!(load_preview(tmp.path(), 24), (Vec::new(), Vec::new()));
    }

    /// Decodes an installed theme from this machine (run with `--ignored`).
    #[test]
    #[ignore = "needs an installed cursor theme"]
    fn an_installed_theme_decodes() {
        let dir = [
            "/usr/share/icons/capitaine-cursors",
            "/usr/share/icons/Adwaita",
        ]
        .iter()
        .map(Path::new)
        .find(|d| d.join("cursors").is_dir())
        .expect("an installed cursor theme");
        let (images, sizes) = load_preview(dir, 32);
        assert!(!images.is_empty() && !sizes.is_empty(), "{}", dir.display());
        let alpha_max = images[0].pixels.chunks(4).map(|p| p[3]).max();
        assert_eq!(alpha_max, Some(255), "the arrow has opaque pixels");
    }
}
