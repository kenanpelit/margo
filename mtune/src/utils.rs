// SPDX-FileCopyrightText: 2022  Emmanuele Bassi
// SPDX-License-Identifier: GPL-3.0-or-later

use core::cmp::Ordering;
use std::path::PathBuf;

use color_thief::{ColorFormat, get_palette};
use gtk::{gdk, gio, glib, prelude::*};
use log::{debug, warn};

use crate::{
    audio::{Queue, Song},
    config::APPLICATION_ID,
};

pub fn settings_manager() -> gio::Settings {
    // We ship a single schema for both default and development profiles
    let app_id = APPLICATION_ID.trim_end_matches(".Devel");
    gio::Settings::new(app_id)
}

pub fn format_remaining_time(t: i64) -> String {
    // We use an explicit LRM character so the MINUS SIGN character
    // stays in front of the remaining time even in RTL locales, instead
    // of getting flipped at the end of the text
    format!("\u{200e}\u{2212}{}:{:02}", (t - (t % 60)) / 60, t % 60)
}

pub fn format_time(t: i64) -> String {
    format!("{}:{:02}", (t - (t % 60)) / 60, t % 60)
}

// The base cover size is 192px, but we need to account for HiDPI;
// better to scale down when rendering on displays with a scaling
// factor of 1 than having to scale up on displays with a scaling
// factor of 2.
pub const COVER_SIZE: i32 = 192 * 2;

pub fn load_cover_texture(buffer: &glib::Bytes) -> Option<gdk_pixbuf::Pixbuf> {
    let stream = gio::MemoryInputStream::from_bytes(buffer);

    if let Ok(pixbuf) = gdk_pixbuf::Pixbuf::from_stream(&stream, gio::Cancellable::NONE) {
        let width = pixbuf.width();
        let height = pixbuf.height();
        let ratio = width as f32 / height as f32;

        let w: i32;
        let h: i32;

        if ratio > 1.0 {
            w = COVER_SIZE;
            h = (COVER_SIZE as f32 / ratio) as i32;
        } else {
            w = (COVER_SIZE as f32 * ratio) as i32;
            h = COVER_SIZE;
        }

        debug!("Cover size {width} x {height} (ratio: {ratio}), scaled: {w} x {h}");

        pixbuf.scale_simple(w, h, gdk_pixbuf::InterpType::Nearest)
    } else {
        warn!("Unable to load cover art");
        None
    }
}

/// Wrap a decoded `Pixbuf` in a `gdk::Texture`. Uses `gdk::MemoryTexture`
/// directly (`Texture::for_pixbuf` is deprecated since GTK 4.20) — the
/// pixbuf's pixel buffer is copied into the texture.
pub fn texture_from_pixbuf(pixbuf: &gdk_pixbuf::Pixbuf) -> gdk::Texture {
    let format = if pixbuf.has_alpha() {
        gdk::MemoryFormat::R8g8b8a8
    } else {
        gdk::MemoryFormat::R8g8b8
    };
    gdk::MemoryTexture::new(
        pixbuf.width(),
        pixbuf.height(),
        format,
        &pixbuf.read_pixel_bytes(),
        pixbuf.rowstride() as usize,
    )
    .upcast()
}

pub fn cache_cover_art(uuid: &str, pixbuf: &gdk_pixbuf::Pixbuf) -> Option<PathBuf> {
    let mut cache_dir = glib::user_cache_dir();
    cache_dir.push("margo");
    cache_dir.push("mtune");
    cache_dir.push("covers");
    glib::mkdir_with_parents(&cache_dir, 0o755);

    cache_dir.push(format!("{}.png", &uuid));
    let file = gio::File::for_path(&cache_dir);
    match file.create(gio::FileCreateFlags::NONE, gio::Cancellable::NONE) {
        Ok(stream) => {
            debug!("Creating cover data cache at {:?}", &cache_dir);
            pixbuf.save_to_streamv_async(
                &stream,
                "png",
                &[("tEXt::Software", "mtune")],
                gio::Cancellable::NONE,
                move |res| {
                    if let Err(e) = res {
                        warn!("Unable to cache cover data: {}", e);
                    }
                },
            );
        }
        Err(e) => {
            if let Some(file_error) = e.kind::<glib::FileError>() {
                match file_error {
                    glib::FileError::Exist => (),
                    _ => {
                        warn!("Unable to create cache file: {}", e);
                        return None;
                    }
                };
            }
        }
    };

    Some(cache_dir)
}

fn color_format(has_alpha: bool) -> ColorFormat {
    if has_alpha {
        ColorFormat::Rgba
    } else {
        ColorFormat::Rgb
    }
}

pub fn load_palette(pixbuf: &gdk_pixbuf::Pixbuf) -> Option<Vec<gdk::RGBA>> {
    let bytes = pixbuf.pixel_bytes()?;
    if let Ok(palette) = get_palette(bytes.as_ref(), color_format(pixbuf.has_alpha()), 5, 4) {
        let colors: Vec<gdk::RGBA> = palette
            .iter()
            .map(|c| {
                gdk::RGBA::new(
                    c.r as f32 / 255.0,
                    c.g as f32 / 255.0,
                    c.b as f32 / 255.0,
                    1.0,
                )
            })
            .collect();

        return Some(colors);
    }

    None
}

fn load_files_from_folder_internal(
    base: &gio::File,
    folder: &gio::File,
    recursive: bool,
) -> Vec<gio::File> {
    let mut enumerator = if let Ok(enumerator) = folder.enumerate_children(
        "standard::name,standard::type",
        gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
        None::<&gio::Cancellable>,
    ) {
        enumerator
    } else {
        warn!("Unable to enumerate {}", folder.uri());
        return Vec::new();
    };

    let mut files = Vec::new();
    while let Some(info) = enumerator.next().and_then(|s| s.ok()) {
        let child = enumerator.child(&info);
        if recursive && info.file_type() == gio::FileType::Directory {
            let mut res = load_files_from_folder_internal(base, &child, recursive);
            files.append(&mut res);
        } else if info.file_type() == gio::FileType::Regular {
            files.push(child.clone());
        }
    }

    // gio::FileEnumerator has no guaranteed order, so we should
    // rely on the basename being formatted in a way that gives us an
    // implicit order; if anything, this will queue songs in the same
    // order in which they appear in the directory when browsing its
    // contents
    files.sort_by(|a, b| cmp_two_files(Some(base), a, b));

    files
}

pub fn cmp_two_files(base: Option<&gio::File>, a: &gio::File, b: &gio::File) -> Ordering {
    // The folder part: relative to `base` when below it, else the parent's
    // own name; `/` (no parent) and a name-less URI sort as "".
    let folder = |f: &gio::File| -> PathBuf {
        let Some(parent) = f.parent() else {
            return PathBuf::new();
        };
        base.and_then(|base| base.relative_path(&parent))
            .or_else(|| parent.basename())
            .unwrap_or_default()
    };
    let parent_basename_a = folder(a);
    let parent_basename_b = folder(b);
    let basename_a = a.basename().unwrap_or_default();
    let basename_b = b.basename().unwrap_or_default();

    let mut order = cmp_like_nautilus(
        &parent_basename_a.to_string_lossy(),
        &parent_basename_b.to_string_lossy(),
    );

    if order.is_eq() {
        order = cmp_like_nautilus(&basename_a.to_string_lossy(), &basename_b.to_string_lossy());
    }

    order
}

fn cmp_like_nautilus(filename_a: &str, filename_b: &str) -> Ordering {
    let order;

    let sort_last = |name: &str| matches!(name.as_bytes().first(), Some(b'.' | b'#'));
    let sort_last_a = sort_last(filename_a);
    let sort_last_b = sort_last(filename_b);

    if !sort_last_a && sort_last_b {
        order = Ordering::Less;
    } else if sort_last_a && !sort_last_b {
        order = Ordering::Greater;
    } else {
        let key_a = glib::FilenameCollationKey::from(filename_a);
        let key_b = glib::FilenameCollationKey::from(filename_b);
        order = key_a.cmp(&key_b);
    }

    order
}

pub fn load_files_from_folder(folder: &gio::File, recursive: bool) -> Vec<gio::File> {
    use std::time::Instant;

    let now = Instant::now();
    let res = load_files_from_folder_internal(folder, folder, recursive);
    debug!(
        "Folder enumeration: {} us (recursive: {}), total files: {}",
        now.elapsed().as_micros(),
        recursive,
        res.len()
    );

    res
}

pub fn store_playlist(queue: &Queue) {
    let pls = glib::KeyFile::new();
    pls.set_string("playlist", "X-GNOME-Title", "Tune's current playlist");

    pls.set_int64("playlist", "NumberOfEntries", queue.n_songs() as i64);

    let model = queue.model();
    // The queue model holds only Songs; a song without a local path (an
    // opened URI) can't be written as a .pls path and is skipped, as
    // playlist::write_m3u does.
    let paths =
        (0..model.n_items()).filter_map(|i| model.item(i)?.downcast::<Song>().ok()?.file().path());
    for (i, path) in paths.enumerate() {
        let path_str = path.to_string_lossy();
        pls.set_value("playlist", &format!("File{i}"), &path_str);
    }

    let mut pls_cache = glib::user_cache_dir();
    pls_cache.push("margo");
    pls_cache.push("mtune");
    pls_cache.push("playlists");
    glib::mkdir_with_parents(&pls_cache, 0o755);

    pls_cache.push("current.pls");
    match pls.save_to_file(&pls_cache) {
        Ok(_) => debug!("Current playlist updated to: {:?}", &pls_cache),
        Err(e) => debug!("Unable to save current playlist: {e}"),
    }
}

pub fn load_cached_songs() -> Option<Vec<gio::File>> {
    let mut pls_cache = glib::user_cache_dir();
    pls_cache.push("margo");
    pls_cache.push("mtune");
    pls_cache.push("playlists");
    pls_cache.push("current.pls");

    let pls = glib::KeyFile::new();
    if let Err(e) = pls.load_from_file(&pls_cache, glib::KeyFileFlags::NONE) {
        debug!("Unable to load current playlist: {e}");
        return None;
    }

    let n_entries: usize = match pls.int64("playlist", "NumberOfEntries") {
        Ok(n) => n as usize,
        Err(_) => 0,
    };

    let mut res = Vec::with_capacity(n_entries);

    for i in 0..n_entries {
        match pls.value("playlist", &format!("File{i}")) {
            Ok(p) => res.push(gio::File::for_path(p)),
            Err(e) => debug!("Skipping File{i} from playlist: {e}"),
        }
    }

    Some(res)
}

pub fn has_cached_playlist() -> bool {
    let mut pls_cache = glib::user_cache_dir();
    pls_cache.push("margo");
    pls_cache.push("mtune");
    pls_cache.push("playlists");
    pls_cache.push("current.pls");

    pls_cache.exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_without_a_parent_or_name_still_sort() {
        // `/` has no parent and no file name; a dropped or opened folder
        // can be anything.
        let root = gio::File::for_path("/");
        let a = gio::File::for_path("/music/a.flac");
        let b = gio::File::for_uri("https://example.invalid/");
        for (x, y) in [(&root, &a), (&a, &root), (&root, &root), (&b, &a)] {
            let _ = cmp_two_files(None, x, y);
            let _ = cmp_two_files(Some(&root), x, y);
        }
        assert_eq!(cmp_two_files(None, &a, &a), Ordering::Equal);
    }

    #[test]
    fn nautilus_order_handles_empty_names() {
        assert_eq!(cmp_like_nautilus("", ""), Ordering::Equal);
        assert_eq!(cmp_like_nautilus("a", ".hidden"), Ordering::Less);
        assert_eq!(cmp_like_nautilus("#x", "b"), Ordering::Greater);
        let _ = cmp_like_nautilus("", "a");
        assert_eq!(cmp_like_nautilus("track 2", "track 10"), Ordering::Less);
    }
}
