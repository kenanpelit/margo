// SPDX-FileCopyrightText: 2022  Emmanuele Bassi
// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    collections::HashMap,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    sync::Mutex,
};

use gtk::{gdk, gio, glib, prelude::*};
use log::debug;
use once_cell::sync::OnceCell;
use sha2::{Digest, Sha256};

use crate::utils;

#[derive(Clone, Debug)]
pub struct CoverArt {
    texture: gdk::Texture,
    palette: Vec<gdk::RGBA>,
    cache: Option<PathBuf>,
}

impl CoverArt {
    pub fn texture(&self) -> &gdk::Texture {
        self.texture.as_ref()
    }

    pub fn palette(&self) -> &Vec<gdk::RGBA> {
        self.palette.as_ref()
    }

    pub fn cache(&self) -> Option<&PathBuf> {
        self.cache.as_ref()
    }
}

#[derive(Debug)]
pub struct CoverCache {
    entries: HashMap<String, CoverArt>,
}

impl CoverCache {
    pub fn global() -> &'static Mutex<CoverCache> {
        static CACHE: OnceCell<Mutex<CoverCache>> = OnceCell::new();

        CACHE.get_or_init(|| {
            let c = CoverCache::new();
            Mutex::new(c)
        })
    }

    fn new() -> Self {
        CoverCache {
            entries: HashMap::new(),
        }
    }

    fn add_entry(&mut self, uuid: &str, cover: CoverArt) -> &CoverArt {
        self.entries.entry(uuid.to_string()).or_insert(cover)
    }

    fn lookup(&self, uuid: &String) -> Option<&CoverArt> {
        self.entries.get(uuid)
    }

    fn load_cover_art(&self, tag: &lofty::tag::Tag, path: Option<&Path>) -> Option<glib::Bytes> {
        if let Some(picture) = tag.get_picture_type(lofty::picture::PictureType::CoverFront) {
            debug!("Found CoverFront");
            return Some(glib::Bytes::from(picture.data()));
        } else {
            // If we don't have a CoverFront picture, we fall back to Other
            // and BandLogo types
            for picture in tag.pictures() {
                let cover_art = match picture.pic_type() {
                    lofty::picture::PictureType::Other => Some(glib::Bytes::from(picture.data())),
                    lofty::picture::PictureType::BandLogo => {
                        Some(glib::Bytes::from(picture.data()))
                    }
                    _ => None,
                };

                if cover_art.is_some() {
                    debug!("Found fallback");
                    return cover_art;
                }
            }
        }

        // We always favour the cover art in the song metadata because it's going
        // to be in a hot cache; looking for a separate file will blow a bunch of
        // caches out of the water, which will slow down loading the song into the
        // playlist model
        if let Some(p) = path {
            let ext_cover_basename = ["Cover", "cover", "Folder", "folder"];
            let ext_cover_ext = ["jpg", "png"];

            let ext_covers = ext_cover_basename
                .iter()
                .map(|&b| {
                    ext_cover_ext
                        .iter()
                        .map(move |&e| format!("{}.{}", &b, &e))
                        .collect::<Vec<_>>()
                })
                .fold(vec![], |mut v, mut dat| {
                    v.append(&mut dat);
                    v
                });
            for name in ext_covers {
                let mut cover_file = PathBuf::from(p);
                cover_file.push(name);
                debug!("Looking for external cover file: {:?}", &cover_file);

                let f = gio::File::for_path(&cover_file);
                if let Ok((res, _)) = f.load_bytes(None::<&gio::Cancellable>) {
                    debug!("Loading cover from external cover file");
                    return Some(res);
                }
            }
        }

        debug!("No cover art");

        None
    }

    pub fn cover_art(&mut self, path: &Path, tag: &lofty::tag::Tag) -> Option<(CoverArt, String)> {
        let mut album_artist = None;
        let mut track_artist = None;
        let mut album = None;

        fn get_text_value(value: &lofty::tag::ItemValue) -> Option<String> {
            match value {
                lofty::tag::ItemValue::Text(s) => Some(s.to_string()),
                _ => None,
            }
        }

        for item in tag.items() {
            match item.key() {
                lofty::prelude::ItemKey::AlbumTitle => album = get_text_value(item.value()),
                lofty::prelude::ItemKey::AlbumArtist => album_artist = get_text_value(item.value()),
                lofty::prelude::ItemKey::TrackArtist => track_artist = get_text_value(item.value()),
                _ => (),
            };
        }

        let uuid = cover_uuid(path, album, album_artist, track_artist);

        match self.lookup(&uuid) {
            Some(c) => {
                debug!("Found cover for UUID '{}'", &uuid);
                Some((c.clone(), uuid))
            }
            None => {
                debug!("Loading cover art for UUID: {}", &uuid);

                let cover_art = self.load_cover_art(tag, path.parent());

                // The pixel buffer for the cover art
                let cover_pixbuf = if let Some(ref cover_art) = cover_art {
                    utils::load_cover_texture(cover_art)
                } else {
                    None
                };

                // Cache the pixel buffer, so that the MPRIS controller can
                // reference it later
                let cache_path = if let Some(ref pixbuf) = cover_pixbuf {
                    utils::cache_cover_art(&uuid, pixbuf)
                } else {
                    None
                };

                // The texture we draw on screen.
                let texture = cover_pixbuf.as_ref().map(utils::texture_from_pixbuf);

                // The color palette we use for styling the UI
                let palette = if let Some(ref pixbuf) = cover_pixbuf {
                    utils::load_palette(pixbuf)
                } else {
                    None
                };

                // We want both texture and palette
                if let (Some(texture), Some(palette)) = (texture, palette) {
                    let res = CoverArt {
                        texture,
                        palette,
                        cache: cache_path,
                    };

                    self.add_entry(&uuid, res.clone());

                    Some((res, uuid))
                } else {
                    None
                }
            }
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

/// The cache key for a track's cover. We use the album and artist to ensure
/// we share the same cover data for every track in the album; if we don't
/// have an album, we use the file name.
fn cover_uuid(
    path: &Path,
    album: Option<String>,
    album_artist: Option<String>,
    track_artist: Option<String>,
) -> String {
    let mut hasher = Sha256::new();
    if let Some(album) = album {
        hasher.update(&album);

        if let Some(artist) = album_artist {
            hasher.update(&artist);
        } else if let Some(artist) = track_artist {
            hasher.update(&artist);
        }

        if let Some(parent) = path.parent() {
            hasher.update(parent.as_os_str().as_bytes());
        }
    } else {
        // The raw path bytes: the same key `to_str()` gave for a UTF-8 path,
        // and no panic for a Latin-1 file name.
        hasher.update(path.as_os_str().as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

/// Lock a cover-cache mutex, recovering it when poisoned: a panic in an
/// earlier holder (a malformed cover in a decoder) leaves a cache that is
/// still a valid map, so later song loads keep working.
pub fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod lock_tests {
    use super::lock;
    use std::sync::Mutex;

    #[test]
    fn a_poisoned_cache_is_still_usable() {
        // A panic while a cover was being decoded (a malformed image)
        // must not take every later song load down with it.
        let m = Mutex::new(1);
        let _ = std::thread::scope(|s| {
            s.spawn(|| {
                let _g = m.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                panic!("decoder bug");
            })
            .join()
        });
        assert!(m.is_poisoned());
        *lock(&m) += 1;
        assert_eq!(*lock(&m), 2);
    }
}

#[cfg(test)]
mod cover_uuid_tests {
    use super::cover_uuid;
    use sha2::{Digest, Sha256};
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    #[test]
    fn utf8_keys_are_unchanged() {
        // Same key as before for UTF-8 paths, so existing caches stay valid.
        let path = Path::new("/music/Album/01.flac");
        let by_file = format!("{:x}", Sha256::digest("/music/Album/01.flac"));
        assert_eq!(cover_uuid(path, None, None, None), by_file);
        let by_album = format!("{:x}", Sha256::digest("AlbumArtist/music/Album"));
        assert_eq!(
            cover_uuid(path, Some("Album".into()), Some("Artist".into()), None),
            by_album
        );
    }

    #[test]
    fn a_non_utf8_file_name_does_not_panic() {
        // Latin-1 file names are common in old music libraries.
        let path = Path::new(OsStr::from_bytes(b"/music/caf\xe9/01.flac"));
        let by_file = cover_uuid(path, None, None, None);
        let by_album = cover_uuid(path, Some("Album".into()), None, None);
        assert_eq!(by_file.len(), 64);
        assert_ne!(by_file, by_album);
    }
}
