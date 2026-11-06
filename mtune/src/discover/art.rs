// SPDX-License-Identifier: GPL-3.0-or-later

//! Station logos and podcast covers: downloaded once into
//! `$XDG_CACHE_HOME/mtune/art/`, decoded and scaled on a worker thread,
//! kept in memory per (URL, size) for the session.

use std::{cell::RefCell, collections::HashMap, path::PathBuf};

use gtk::{gdk, gio, glib, prelude::*};
use sha2::{Digest, Sha256};

use crate::net;

/// Row thumbnails (drawn at 40 px; 2x for HiDPI).
pub const ROW: i32 = 80;

thread_local! {
    /// `None` = failed this session; not retried until restart.
    static MEM: RefCell<HashMap<(String, i32), Option<gdk::Texture>>> = RefCell::new(HashMap::new());
}

pub fn url_hash(url: &str) -> String {
    format!("{:x}", Sha256::digest(url.as_bytes()))
}

fn disk_path(url: &str) -> PathBuf {
    margo_fs::xdg::cache_home()
        .join("mtune")
        .join("art")
        .join(url_hash(url))
}

/// Blocking: the image bytes, from the disk cache or the network.
fn fetch_bytes(url: &str) -> Option<Vec<u8>> {
    let path = disk_path(url);
    if let Ok(b) = std::fs::read(&path)
        && !b.is_empty()
    {
        return Some(b);
    }
    let bytes = net::get_bytes(url, &[], net::IMAGE_CAP).ok()?;
    if let Some(dir) = path.parent()
        && std::fs::create_dir_all(dir).is_ok()
    {
        let _ = margo_fs::atomic_write::atomic_write(&path, &bytes);
    }
    Some(bytes)
}

/// Decoded pixels: `Pixbuf` itself is not `Send`, its pixel bytes are.
struct Pixels {
    data: glib::Bytes,
    width: i32,
    height: i32,
    stride: i32,
    alpha: bool,
}

/// Blocking: decode `bytes` scaled to fit `size` (aspect kept).
fn decode(bytes: Vec<u8>, size: i32) -> Option<Pixels> {
    let stream = gio::MemoryInputStream::from_bytes(&glib::Bytes::from_owned(bytes));
    let pb =
        gdk_pixbuf::Pixbuf::from_stream_at_scale(&stream, size, size, true, gio::Cancellable::NONE)
            .ok()?;
    // Normalise to 8-bit RGB(A), the only layout `from_bytes` rebuilds.
    if pb.bits_per_sample() != 8 || pb.colorspace() != gdk_pixbuf::Colorspace::Rgb {
        return None;
    }
    Some(Pixels {
        data: pb.read_pixel_bytes(),
        width: pb.width(),
        height: pb.height(),
        stride: pb.rowstride(),
        alpha: pb.has_alpha(),
    })
}

/// The decoded image at `size`, fetched off the GTK thread.
pub async fn pixbuf(url: &str, size: i32) -> Option<gdk_pixbuf::Pixbuf> {
    let url = net::validate_http_url(url)?;
    let px = net::off_main(move || fetch_bytes(&url).and_then(|b| decode(b, size)))
        .await
        .flatten()?;
    Some(gdk_pixbuf::Pixbuf::from_bytes(
        &px.data,
        gdk_pixbuf::Colorspace::Rgb,
        px.alpha,
        8,
        px.width,
        px.height,
        px.stride,
    ))
}

/// A texture for a row thumbnail (memoised per session).
pub async fn texture(url: &str, size: i32) -> Option<gdk::Texture> {
    let key = (url.to_string(), size);
    if let Some(hit) = MEM.with(|m| m.borrow().get(&key).cloned()) {
        return hit;
    }
    let tex = pixbuf(url, size)
        .await
        .map(|p| crate::utils::texture_from_pixbuf(&p));
    MEM.with(|m| m.borrow_mut().insert(key, tex.clone()));
    tex
}

/// A rounded thumbnail that fills in when its image arrives; `fallback`
/// is the symbolic icon shown meanwhile (and when there is no image).
pub fn thumbnail(url: &str, fallback: &str) -> gtk::Image {
    let image = gtk::Image::builder()
        .icon_name(fallback)
        .pixel_size(40)
        .css_classes(["discover-art"])
        .valign(gtk::Align::Center)
        .build();
    if !url.is_empty() {
        let url = url.to_string();
        let weak = image.downgrade();
        glib::spawn_future_local(async move {
            if let Some(tex) = texture(&url, ROW).await
                && let Some(image) = weak.upgrade()
            {
                image.set_paintable(Some(&tex));
            }
        });
    }
    image
}
