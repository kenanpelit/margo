//! Theme bundles: apply a saved look (colour scheme + matugen settings +
//! wallpaper set) in one step, and scope wallpaper cycling to the active
//! bundle's wallpapers.
//!
//! The bundles live in the config (`theme.bundles`, `theme.active_bundle`);
//! this module only does the parts that touch files: which of a bundle's
//! wallpapers exist, which one to set, and where previews are stored.

use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, Theme, ThemeBundle};
use mshell_image::lut::decode_rgba_scaled;
use reactive_graph::prelude::GetUntracked;
use relm4::gtk::prelude::Cast;
use relm4::gtk::{gdk, glib};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use tracing::info;

/// Where captured bundle previews are written:
/// `$XDG_DATA_HOME/mshell/theme-bundles`.
pub fn preview_dir() -> PathBuf {
    margo_fs::xdg::data_home()
        .join("mshell")
        .join("theme-bundles")
}

/// The path a desktop capture for `bundle` is saved to.
pub fn capture_path(bundle: &ThemeBundle) -> PathBuf {
    preview_dir().join(format!("{}.png", bundle.slug()))
}

/// The bundle's wallpapers that exist on disk, in bundle order, without
/// repeats. `~/` is expanded.
pub fn existing_wallpapers(bundle: &ThemeBundle) -> Vec<PathBuf> {
    existing_paths(&bundle.wallpapers, |p| p.is_file())
}

fn existing_paths(paths: &[String], exists: impl Fn(&Path) -> bool) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for raw in paths {
        let path = expand_home(raw.trim());
        if path.as_os_str().is_empty() || out.contains(&path) || !exists(&path) {
            continue;
        }
        out.push(path);
    }
    out
}

fn expand_home(raw: &str) -> PathBuf {
    match raw.strip_prefix("~/") {
        Some(rest) => margo_fs::xdg::home().unwrap_or_default().join(rest),
        None => PathBuf::from(raw),
    }
}

/// The image to show on a bundle's card: its preview if that file exists,
/// else its first existing wallpaper.
pub fn preview_image(bundle: &ThemeBundle) -> Option<PathBuf> {
    let preview = expand_home(bundle.preview.trim());
    if !bundle.preview.trim().is_empty() && preview.is_file() {
        return Some(preview);
    }
    existing_wallpapers(bundle).into_iter().next()
}

/// The wallpaper to set when applying a bundle: keep the current one when
/// it already belongs to the bundle (re-applying shouldn't jump), else the
/// first.
pub fn wallpaper_to_apply(existing: &[PathBuf], current: Option<&Path>) -> Option<PathBuf> {
    if let Some(cur) = current
        && existing.iter().any(|p| p == cur)
    {
        return Some(cur.to_path_buf());
    }
    existing.first().cloned()
}

/// The wallpapers cycling steps through while a bundle is active: the
/// active bundle's existing wallpapers, or `None` (use the folder) when no
/// bundle is active or none of its wallpapers exist any more.
pub fn active_bundle_wallpapers(theme: &Theme) -> Option<Vec<PathBuf>> {
    let list = existing_wallpapers(theme.active_bundle()?);
    (!list.is_empty()).then_some(list)
}

/// A decoded, downscaled image, ready to become a texture on the GTK
/// thread. Built off it by [`load_thumbnail`].
#[derive(Debug, Clone)]
pub struct Thumbnail {
    pub width: u32,
    pub height: u32,
    /// Straight RGBA, 4 bytes per pixel.
    pub rgba: Vec<u8>,
}

impl Thumbnail {
    /// The thumbnail as a texture (GTK thread).
    pub fn texture(&self) -> gdk::Texture {
        let bytes = glib::Bytes::from(&self.rgba[..]);
        gdk::MemoryTexture::new(
            self.width as i32,
            self.height as i32,
            gdk::MemoryFormat::R8g8b8a8,
            &bytes,
            self.width as usize * 4,
        )
        .upcast()
    }
}

/// The size an image of `w`×`h` is decoded at to be `height` px tall
/// (never upscaled, never zero).
pub fn thumbnail_size(w: u32, h: u32, height: u32) -> (u32, u32) {
    if h <= height || h == 0 {
        return (w.max(1), h.max(1));
    }
    let width = (u64::from(w) * u64::from(height) / u64::from(h)).max(1);
    (u32::try_from(width).unwrap_or(u32::MAX), height.max(1))
}

/// Decode `path` scaled to `height` px tall. Blocking file IO + decode:
/// call off the GTK thread (e.g. `gio::spawn_blocking`).
pub fn load_thumbnail(path: &Path, height: u32) -> Option<Thumbnail> {
    let never = AtomicBool::new(false);
    let img = decode_rgba_scaled(path, move |w, h| thumbnail_size(w, h, height), &never)?;
    let (width, height) = img.dimensions();
    Some(Thumbnail {
        width,
        height,
        rgba: img.into_raw(),
    })
}

/// Why a bundle could not be applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyError {
    /// No bundle has that name.
    Unknown(String),
}

impl std::fmt::Display for ApplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown(name) => write!(f, "no theme bundle named '{name}'"),
        }
    }
}

impl std::error::Error for ApplyError {}

/// Apply the bundle named `name` (exact, then case-insensitive): switch the
/// colour scheme + matugen settings, mark it active, and set its wallpaper.
/// Call on the GTK main thread (setting a wallpaper kicks glib work).
pub fn apply_theme_bundle(name: &str) -> Result<(), ApplyError> {
    let theme = config_manager().config().theme().get_untracked();
    let bundle = theme
        .bundle(name)
        .cloned()
        .ok_or_else(|| ApplyError::Unknown(name.to_string()))?;
    info!(bundle = %bundle.name, "applying theme bundle");
    // Wallpaper first: with the wallpaper-driven scheme the palette is
    // generated from it, so the scheme switch then sees the new image.
    let existing = existing_wallpapers(&bundle);
    let current = crate::wallpaper::current_wallpaper_path();
    if let Some(wallpaper) = wallpaper_to_apply(&existing, current.as_deref())
        && Some(&wallpaper) != current.as_ref()
    {
        crate::wallpaper::set_wallpaper(&wallpaper);
    }
    config_manager().update_config(|config| bundle.apply_to(&mut config.theme));
    Ok(())
}

/// Stop scoping wallpaper cycling to a bundle (the scheme and wallpaper
/// stay as they are).
pub fn clear_active_bundle() {
    config_manager().update_config(|config| config.theme.active_bundle.clear());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_paths_keep_order_drop_missing_and_repeats() {
        let paths = ["/w/b.png", "/w/gone.png", "/w/a.png", "/w/b.png", "  "]
            .map(String::from)
            .to_vec();
        let out = existing_paths(&paths, |p| p != Path::new("/w/gone.png"));
        assert_eq!(out, [PathBuf::from("/w/b.png"), PathBuf::from("/w/a.png")]);
    }

    #[test]
    fn existing_wallpapers_reads_the_disk() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let a = tmp.path().join("a.png");
        std::fs::write(&a, b"x").expect("write");
        let bundle = ThemeBundle {
            wallpapers: vec![
                a.display().to_string(),
                tmp.path().join("missing.png").display().to_string(),
                tmp.path().display().to_string(),
            ],
            ..ThemeBundle::default()
        };
        assert_eq!(existing_wallpapers(&bundle), [a.clone()]);
        assert_eq!(
            preview_image(&bundle),
            Some(a.clone()),
            "falls back to a wallpaper"
        );
        let shot = tmp.path().join("shot.png");
        std::fs::write(&shot, b"x").expect("write");
        let with_preview = ThemeBundle {
            preview: shot.display().to_string(),
            ..bundle.clone()
        };
        assert_eq!(preview_image(&with_preview), Some(shot));
        let stale_preview = ThemeBundle {
            preview: "/nope.png".into(),
            ..bundle
        };
        assert_eq!(preview_image(&stale_preview), Some(a));
    }

    #[test]
    fn applying_keeps_a_current_wallpaper_from_the_bundle() {
        let list = [PathBuf::from("/w/a.png"), PathBuf::from("/w/b.png")];
        assert_eq!(
            wallpaper_to_apply(&list, Some(Path::new("/w/b.png"))),
            Some(PathBuf::from("/w/b.png"))
        );
        assert_eq!(
            wallpaper_to_apply(&list, Some(Path::new("/elsewhere.png"))),
            Some(PathBuf::from("/w/a.png"))
        );
        assert_eq!(
            wallpaper_to_apply(&list, None),
            Some(PathBuf::from("/w/a.png"))
        );
        assert_eq!(wallpaper_to_apply(&[], None), None);
    }

    #[test]
    fn cycling_is_scoped_only_to_an_active_bundle_with_wallpapers() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let a = tmp.path().join("a.png");
        std::fs::write(&a, b"x").expect("write");
        let mut theme = Theme::default();
        theme.bundles = vec![
            ThemeBundle {
                name: "Has".into(),
                wallpapers: vec![a.display().to_string()],
                ..ThemeBundle::default()
            },
            ThemeBundle {
                name: "Empty".into(),
                wallpapers: vec!["/nope.png".into()],
                ..ThemeBundle::default()
            },
        ];
        assert_eq!(active_bundle_wallpapers(&theme), None, "none active");
        theme.active_bundle = "has".into();
        assert_eq!(active_bundle_wallpapers(&theme), Some(vec![a]));
        theme.active_bundle = "Empty".into();
        assert_eq!(
            active_bundle_wallpapers(&theme),
            None,
            "falls back to the folder"
        );
    }

    #[test]
    fn thumbnails_scale_down_to_the_height_only() {
        assert_eq!(thumbnail_size(3840, 2160, 192), (341, 192));
        assert_eq!(thumbnail_size(100, 50, 192), (100, 50), "no upscale");
        assert_eq!(thumbnail_size(1, 4000, 10), (1, 10));
        assert_eq!(thumbnail_size(0, 0, 10), (1, 1));
    }

    #[test]
    fn capture_path_uses_the_slug() {
        let bundle = ThemeBundle {
            name: "Rose Pine".into(),
            ..ThemeBundle::default()
        };
        assert!(capture_path(&bundle).ends_with("mshell/theme-bundles/rose-pine.png"));
    }
}
