//! A saved theme bundle's card in the Color Scheme picker: its preview
//! (a desktop capture or its first wallpaper) under its name; a click
//! applies the whole bundle.

use mshell_cache::theme_bundle::{load_thumbnail, preview_image};
use mshell_config::schema::config::ThemeBundle;
use relm4::gtk::prelude::{BoxExt, ButtonExt, Cast, ObjectExt, WidgetExt};
use relm4::gtk::{self, gio, glib};

/// Card height in px; the preview is decoded at this height.
const CARD_HEIGHT: i32 = 96;
const CARD_WIDTH: i32 = 160;

/// Build one bundle card. `on_apply` runs with the bundle's name on click.
pub(crate) fn bundle_card(
    bundle: &ThemeBundle,
    active: bool,
    on_apply: impl Fn(String) + 'static,
) -> gtk::Button {
    let overlay = gtk::Overlay::new();
    overlay.set_size_request(CARD_WIDTH, CARD_HEIGHT);

    let picture = gtk::Picture::new();
    picture.set_content_fit(gtk::ContentFit::Cover);
    picture.set_can_shrink(true);
    picture.add_css_class("theme-bundle-preview");
    overlay.set_child(Some(&picture));

    let fallback = gtk::Image::from_icon_name("palette-symbolic");
    fallback.set_pixel_size(40);
    fallback.add_css_class("image-surface");
    fallback.set_halign(gtk::Align::Center);
    fallback.set_valign(gtk::Align::Center);
    overlay.add_overlay(&fallback);

    let caption = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    caption.add_css_class("theme-bundle-caption");
    caption.set_valign(gtk::Align::End);
    if active {
        let check = gtk::Image::from_icon_name("check-circle-symbolic");
        check.set_pixel_size(16);
        caption.append(&check);
    }
    let title = gtk::Label::new(Some(&bundle.name));
    title.add_css_class("label-small");
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    title.set_xalign(0.0);
    title.set_hexpand(true);
    caption.append(&title);
    overlay.add_overlay(&caption);

    let button = gtk::Button::new();
    button.add_css_class("theme-bundle-card");
    if active {
        button.add_css_class("active");
    }
    button.set_child(Some(&overlay));
    button.set_tooltip_text(Some(&format!("Apply the “{}” bundle", bundle.name)));
    let name = bundle.name.clone();
    button.connect_clicked(move |_| on_apply(name.clone()));

    // Decode the preview off the GTK thread; swap the icon out when it lands.
    if let Some(path) = preview_image(bundle) {
        let picture = picture.downgrade();
        let fallback = fallback.downgrade();
        let task = gio::spawn_blocking(move || load_thumbnail(&path, CARD_HEIGHT as u32 * 2));
        glib::spawn_future_local(async move {
            let Ok(Some(thumb)) = task.await else {
                return;
            };
            if let (Some(picture), Some(fallback)) = (picture.upgrade(), fallback.upgrade()) {
                picture.set_paintable(Some(thumb.texture().upcast_ref::<gtk::gdk::Paintable>()));
                fallback.set_visible(false);
            }
        });
    }
    button
}
