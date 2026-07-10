//! The avatar must not size the card.
//!
//! `~/.face` is whatever resolution the user saved it at, and the greeter draws
//! it in an 84 px circle. The first attempt used a `GtkPicture` with an 84×84
//! size request, and the card came up with an avatar half the height of the
//! screen: a picture's *natural* size is its paintable's own, and
//! `gtk_widget_set_size_request` sets a floor, never a ceiling.
//!
//! So this pins the two halves of that lesson — that `GtkPicture` really does
//! grow, and that `GtkImage`'s `pixel_size` really is the ceiling — rather than
//! trusting a comment about it.

use gtk4 as gtk;

use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;

/// Bigger than the circle by a factor nobody would mistake for rounding.
const SOURCE_PX: i32 = 512;
const AVATAR_PX: i32 = 84;

/// An opaque `SOURCE_PX` square. Its contents are irrelevant; only its
/// intrinsic size is under test.
fn texture() -> gdk::Texture {
    let stride = SOURCE_PX as usize * 4;
    let pixels = vec![0u8; stride * SOURCE_PX as usize];
    gdk::MemoryTexture::new(
        SOURCE_PX,
        SOURCE_PX,
        gdk::MemoryFormat::B8g8r8a8Premultiplied,
        &glib::Bytes::from_owned(pixels),
        stride,
    )
    .upcast()
}

fn natural_height(widget: &impl IsA<gtk::Widget>) -> i32 {
    widget.measure(gtk::Orientation::Vertical, -1).1
}

#[test]
fn the_avatar_is_the_size_of_the_circle_not_the_photograph() {
    if gtk::init().is_err() {
        return; // headless CI: no display, nothing to measure
    }

    let image = gtk::Image::from_paintable(Some(&texture()));
    image.set_pixel_size(AVATAR_PX);
    assert_eq!(
        natural_height(&image),
        AVATAR_PX,
        "a GtkImage with a pixel size must not grow to its paintable"
    );
}

#[test]
fn a_size_request_would_not_have_held_a_picture_back() {
    if gtk::init().is_err() {
        return;
    }

    let picture = gtk::Picture::for_paintable(&texture());
    picture.set_can_shrink(true);
    picture.set_size_request(AVATAR_PX, AVATAR_PX);

    // The bug, reproduced: `can_shrink` lowers the *minimum* to zero and the
    // size request raises it back to 84, but the natural height is still the
    // photograph's. A GtkBox hands its child that, and the card grows.
    assert_eq!(
        natural_height(&picture),
        SOURCE_PX,
        "if this ever equals {AVATAR_PX}, GtkPicture learned to clamp and the \
         GtkImage above can be simplified"
    );
}
