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

use gtk4 as gtk;

use gtk::gdk;
use gtk::gio;
use std::path::{Path, PathBuf};

/// Where the theme sync leaves it. Machine-written, so `/var/lib`.
const SYSTEM_PATH: &str = "/var/lib/mgreet/avatar";

/// `~/.face` is the freedesktop convention; `.face.icon` is KDE's older spelling.
const HOME_NAMES: &[&str] = &[".face", ".face.icon"];

/// The avatar texture, or `None` when the user has no `~/.face` (which is most
/// users, and not an error).
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
    gdk::Texture::from_file(&gio::File::for_path(&path)).ok()
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
    use super::monogram;

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
