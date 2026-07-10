//! The user's face.
//!
//! In the real greeter the picture is `/var/lib/mgreet/avatar`, a byte copy of
//! `~/.face` that `mlogind`'s theme sync carried out of the user's home (the
//! greeter is `mlogind-greeter` and `$HOME` is `0710`, so it cannot go and get
//! it). Under `--preview` we are the user, and read `~/.face` directly.
//!
//! There is exactly one such file, and it belongs to whoever logged in last —
//! the same user the greeter's cache pre-fills. So the picture is only ever
//! drawn while the typed name still matches [`State::avatar_owner`]; type a
//! different name and the card falls back to that name's monogram. A greeter
//! that showed Alice's face over Bob's password field would be telling him
//! something about Alice.
//!
//! GTK decodes the file. `mlogind` bounded it and checked it opens with a PNG or
//! JPEG signature before publishing it, at both ends of the fork.
//!
//! What comes back is cropped to a centred square here, because the card draws it
//! in a circle and a `GtkImage` fits rather than crops: a 16:9 face would sit in
//! the circle with two wedges of background beside it.

use gtk4 as gtk;

use gtk::gdk;
use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use std::path::{Path, PathBuf};

/// Where the theme sync leaves it. Machine-written, so `/var/lib`.
const SYSTEM_PATH: &str = "/var/lib/mgreet/avatar";

/// `~/.face` is the freedesktop convention; `.face.icon` is KDE's older spelling.
const HOME_NAMES: &[&str] = &[".face", ".face.icon"];

/// A face, not a wallpaper. Cropping downloads the decoded pixels into main
/// memory — four bytes an edge squared — so anything past this is drawn as it
/// came, wedges and all, rather than costing the greeter 100 MB to round off.
const MAX_CROP_EDGE: i32 = 2048;

/// The avatar texture, cropped square, or `None` when the user has no `~/.face`
/// (which is most users, and not an error).
pub fn load(real_greeter: bool) -> Option<gdk::Texture> {
    let path = if real_greeter {
        PathBuf::from(SYSTEM_PATH)
    } else {
        home_face()?
    };
    if !path.exists() {
        return None;
    }
    // A file that survived the sync's signature check can still fail to decode —
    // truncated, or a format GTK was built without. No avatar, then.
    let texture = gdk::Texture::from_file(&gio::File::for_path(&path)).ok()?;
    Some(crop_to_square(&texture).unwrap_or(texture))
}

/// The largest centred square that fits in a `width × height` image.
fn centre_square(width: i32, height: i32) -> Option<(i32, i32, i32)> {
    if width <= 0 || height <= 0 || width == height {
        return None; // nothing to crop, or nothing to crop *from*
    }
    let side = width.min(height);
    Some(((width - side) / 2, (height - side) / 2, side))
}

/// Copy the centred square out of `texture`. `None` when it is already square,
/// degenerate, or too large to be worth downloading.
///
/// The pixel format is whatever `download` hands us — the crop only moves rows
/// around, so it never has to know which one that is.
fn crop_to_square(texture: &gdk::Texture) -> Option<gdk::Texture> {
    let (width, height) = (texture.width(), texture.height());
    if width > MAX_CROP_EDGE || height > MAX_CROP_EDGE {
        return None;
    }
    let (x, y, side) = centre_square(width, height)?;

    let stride = width as usize * 4;
    let mut pixels = vec![0u8; stride * height as usize];
    texture.download(&mut pixels, stride);

    let row_bytes = side as usize * 4;
    let mut cropped = Vec::with_capacity(row_bytes * side as usize);
    for row in 0..side as usize {
        let start = (row + y as usize) * stride + x as usize * 4;
        cropped.extend_from_slice(&pixels[start..start + row_bytes]);
    }

    Some(
        gdk::MemoryTexture::new(
            side,
            side,
            gdk::MemoryFormat::B8g8r8a8Premultiplied,
            &glib::Bytes::from_owned(cropped),
            row_bytes,
        )
        .upcast(),
    )
}

fn home_face() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    HOME_NAMES
        .iter()
        .map(|name| Path::new(&home).join(name))
        .find(|path| path.exists())
}

/// The letter to draw when there is no picture: the first character of the name.
///
/// `None` for a name that starts with nothing worth drawing — an empty field, or
/// punctuation — in which case the card shows no avatar at all rather than an
/// empty circle.
pub fn monogram(name: &str) -> Option<String> {
    let first = name.trim().chars().next()?;
    if !first.is_alphanumeric() {
        return None;
    }
    Some(first.to_uppercase().to_string())
}

#[cfg(test)]
mod tests {
    use super::{centre_square, monogram};

    #[test]
    fn a_square_image_is_left_alone() {
        assert_eq!(centre_square(256, 256), None);
    }

    #[test]
    fn a_wide_image_is_cropped_from_the_middle() {
        // 1600×900 → the middle 900×900, so the face and not the left shoulder.
        assert_eq!(centre_square(1600, 900), Some((350, 0, 900)));
    }

    #[test]
    fn a_tall_image_is_cropped_from_the_middle() {
        assert_eq!(centre_square(900, 1600), Some((0, 350, 900)));
    }

    #[test]
    fn an_odd_remainder_never_walks_off_the_edge() {
        // 101×100: the offset rounds down, so x + side stays inside the width.
        let (x, y, side) = centre_square(101, 100).expect("not square");
        assert!(x + side <= 101 && y + side <= 100, "{x},{y},{side}");
    }

    #[test]
    fn a_degenerate_image_is_not_cropped() {
        assert_eq!(centre_square(0, 100), None);
        assert_eq!(centre_square(100, -1), None);
    }

    #[test]
    fn the_monogram_is_the_first_letter_capitalised() {
        assert_eq!(monogram("kenan").as_deref(), Some("K"));
        assert_eq!(monogram("Ada").as_deref(), Some("A"));
    }

    #[test]
    fn leading_whitespace_is_not_the_first_letter() {
        assert_eq!(monogram("  kenan").as_deref(), Some("K"));
    }

    #[test]
    fn a_name_outside_ascii_still_has_a_first_letter() {
        // A username can be anything getpwnam accepts, and the field is free text.
        assert_eq!(monogram("şeyma").as_deref(), Some("Ş"));
        assert_eq!(monogram("Ölçer").as_deref(), Some("Ö"));
    }

    #[test]
    fn a_digit_is_a_letter_enough() {
        assert_eq!(monogram("2fast").as_deref(), Some("2"));
    }

    #[test]
    fn nothing_worth_drawing_yields_no_avatar() {
        assert_eq!(monogram(""), None);
        assert_eq!(monogram("   "), None);
        assert_eq!(monogram("-root"), None);
    }
}
