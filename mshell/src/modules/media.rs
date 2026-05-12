//! Media player module — eww `(music)` + `(music_pop)`.
//!
//! Bar row: 24 px circular cover art + title + play/pause + hover-
//! revealed prev/next. Click anywhere on the row opens a popover
//! containing the larger 140 px cover, title, artist and the
//! prev/play/next trio — same shape as saimoom's `music_pop`.
//!
//! Cover art is loaded from `mpris:artUrl`. Local `file://` paths
//! are pulled straight into the GtkImage; remote URLs are skipped
//! for now (Stage 10 follow-up: async fetcher → temp file).
//!
//! Hidden when no MPRIS player is reachable.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{
    Align, Box as GtkBox, Button, EventControllerMotion, GestureClick, Image, Label, Orientation,
    Popover, PositionType, Revealer, RevealerTransitionType,
};

use crate::services::mpris;

const POLL_SECS: u32 = 2;

const ICON_PLAY: &str = "\u{f04b}";
const ICON_PAUSE: &str = "\u{f04c}";
const ICON_PREV: &str = "\u{f04a}";
const ICON_NEXT: &str = "\u{f04e}";

const BAR_COVER_PX: i32 = 22;
const POP_COVER_PX: i32 = 140;

pub fn build() -> GtkBox {
    let row = GtkBox::builder()
        .name("media")
        .orientation(Orientation::Horizontal)
        .spacing(6)
        .build();
    row.add_css_class("module");
    row.add_css_class("media");

    let cover = Image::builder()
        .name("media-cover")
        .pixel_size(BAR_COVER_PX)
        .build();
    cover.add_css_class("media-cover");

    let title = Label::builder()
        .name("media-title")
        .max_width_chars(18)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    title.add_css_class("media-title");

    let play_btn = Button::builder().name("media-play").label(ICON_PLAY).build();
    play_btn.add_css_class("media-btn");
    play_btn.add_css_class("media-play");
    play_btn.connect_clicked(|_| mpris::play_pause());

    let prev_btn = Button::builder().name("media-prev").label(ICON_PREV).build();
    prev_btn.add_css_class("media-btn");
    prev_btn.connect_clicked(|_| mpris::previous());

    let next_btn = Button::builder().name("media-next").label(ICON_NEXT).build();
    next_btn.add_css_class("media-btn");
    next_btn.connect_clicked(|_| mpris::next());

    let controls = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(4)
        .build();
    controls.append(&prev_btn);
    controls.append(&next_btn);

    let revealer = Revealer::builder()
        .transition_type(RevealerTransitionType::SlideRight)
        .transition_duration(220)
        .child(&controls)
        .build();

    row.append(&cover);
    row.append(&title);
    row.append(&play_btn);
    row.append(&revealer);

    // Popover — eww `music_pop`. Big cover + bold title + artist
    // + the same prev/play/next trio. Click anywhere on the bar row
    // opens it.
    let popover = build_popover();
    popover.set_parent(&row);

    let last_url: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));

    refresh(
        &row,
        &cover,
        &title,
        &play_btn,
        &popover,
        &last_url,
    );

    let row_tick = row.clone();
    let cover_tick = cover.clone();
    let title_tick = title.clone();
    let play_tick = play_btn.clone();
    let popover_tick = popover.clone();
    let last_tick = last_url.clone();
    glib::timeout_add_seconds_local(POLL_SECS, move || {
        refresh(
            &row_tick,
            &cover_tick,
            &title_tick,
            &play_tick,
            &popover_tick,
            &last_tick,
        );
        glib::ControlFlow::Continue
    });

    let motion = EventControllerMotion::new();
    let rev_enter = revealer.clone();
    motion.connect_enter(move |_, _, _| rev_enter.set_reveal_child(true));
    let rev_leave = revealer.clone();
    motion.connect_leave(move |_| rev_leave.set_reveal_child(false));
    row.add_controller(motion);

    // Click on the cover (the title region) opens the popover.
    // Excludes the play button which has its own toggle handler.
    let click = GestureClick::builder().button(1).build();
    let popover_for_click = popover.clone();
    click.connect_pressed(move |gesture, _, x, _| {
        // Only popup if the click was on the left half of the row
        // (cover / title region). The play button on the right
        // already consumed clicks meant for it via its own handler.
        let widget = gesture.widget();
        let w = widget.map(|w| w.width()).unwrap_or(0);
        if w == 0 || (x as i32) < (w / 2) {
            popover_for_click.popup();
        }
    });
    row.add_controller(click);

    row
}

/// Build the secondary popover ("music_pop" in eww).
fn build_popover() -> Popover {
    let body = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(10)
        .halign(Align::Center)
        .build();
    body.add_css_class("music-pop");

    let cover = Image::builder()
        .name("music-pop-cover")
        .pixel_size(POP_COVER_PX)
        .build();
    cover.add_css_class("music-pop-cover");

    let title = Label::builder()
        .name("music-pop-title")
        .max_width_chars(22)
        .wrap(true)
        .justify(gtk::Justification::Center)
        .build();
    title.add_css_class("music-pop-title");

    let artist = Label::builder()
        .name("music-pop-artist")
        .max_width_chars(28)
        .wrap(true)
        .justify(gtk::Justification::Center)
        .build();
    artist.add_css_class("music-pop-artist");

    let prev = Button::builder().label(ICON_PREV).build();
    prev.add_css_class("music-pop-btn");
    prev.connect_clicked(|_| mpris::previous());
    let play = Button::builder().label(ICON_PLAY).build();
    play.add_css_class("music-pop-btn");
    play.add_css_class("music-pop-play");
    play.connect_clicked(|_| mpris::play_pause());
    let next = Button::builder().label(ICON_NEXT).build();
    next.add_css_class("music-pop-btn");
    next.connect_clicked(|_| mpris::next());

    let controls = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(18)
        .halign(Align::Center)
        .build();
    controls.append(&prev);
    controls.append(&play);
    controls.append(&next);

    body.append(&cover);
    body.append(&title);
    body.append(&artist);
    body.append(&controls);

    let popover = Popover::builder()
        .child(&body)
        .position(PositionType::Bottom)
        .has_arrow(true)
        .autohide(true)
        .build();
    popover.add_css_class("popover-music");
    popover
}

/// Resolve `mpris:artUrl` to a local file path the GtkImage can
/// actually load. Returns `None` for empty or remote URLs.
fn local_art_path(url: &str) -> Option<String> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(rest) = trimmed.strip_prefix("file://") {
        // URL-decoded percent-escapes are common in cover paths
        // (`%20` etc). Most cover-art on this stack uses ASCII
        // filenames so a naive strip is fine; if we hit unicode
        // titles we can pull in `urlencoding` later.
        return Some(rest.to_string());
    }
    None
}

fn refresh(
    row: &GtkBox,
    bar_cover: &Image,
    title: &Label,
    play_btn: &Button,
    popover: &Popover,
    last_url: &Rc<RefCell<String>>,
) {
    match mpris::current() {
        Some(snap) => {
            row.set_visible(true);
            let txt = if !snap.title.is_empty() {
                snap.title.clone()
            } else {
                snap.artist.clone()
            };
            title.set_text(&txt);
            play_btn.set_label(if snap.playing { ICON_PAUSE } else { ICON_PLAY });
            row.remove_css_class("paused");
            if !snap.playing {
                row.add_css_class("paused");
            }

            // Refresh cover art only when the URL actually changed
            // — GtkImage::set_from_file decodes on every call, no
            // need to repeat that twice a second.
            if *last_url.borrow() != snap.art_url {
                last_url.replace(snap.art_url.clone());
                match local_art_path(&snap.art_url) {
                    Some(path) => {
                        bar_cover.set_from_file(Some(&path));
                        // Mirror into the popover's cover; we look
                        // it up by name so we don't have to thread
                        // an Image clone through the refresh args.
                        if let Some(pop_cover) = find_pop_cover(popover) {
                            pop_cover.set_from_file(Some(&path));
                        }
                    }
                    None => {
                        bar_cover.set_icon_name(Some("audio-x-generic-symbolic"));
                        if let Some(pop_cover) = find_pop_cover(popover) {
                            pop_cover.set_icon_name(Some("audio-x-generic-symbolic"));
                        }
                    }
                }
            }

            // Sync the popover's title + artist on every poll —
            // cheap and avoids stale text when the popover opens
            // mid-track.
            sync_popover_text(popover, &snap);
        }
        None => {
            row.set_visible(false);
        }
    }
}

/// Walk the popover's widget tree to find the cover GtkImage we
/// constructed in `build_popover`. Cheap O(depth) — the tree is a
/// handful of nodes deep.
fn find_pop_cover(popover: &Popover) -> Option<Image> {
    let mut child = popover.child();
    while let Some(w) = child {
        if let Some(image) = w.downcast_ref::<Image>() {
            if image.widget_name() == "music-pop-cover" {
                return Some(image.clone());
            }
        }
        // Descend into the first child + walk siblings.
        if let Some(first) = w.first_child() {
            if let Some(found) = find_pop_cover_in_tree(&first) {
                return Some(found);
            }
        }
        child = w.next_sibling();
    }
    None
}

fn find_pop_cover_in_tree(widget: &gtk::Widget) -> Option<Image> {
    if let Some(image) = widget.downcast_ref::<Image>() {
        if image.widget_name() == "music-pop-cover" {
            return Some(image.clone());
        }
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        if let Some(found) = find_pop_cover_in_tree(&c) {
            return Some(found);
        }
        child = c.next_sibling();
    }
    None
}

fn sync_popover_text(popover: &Popover, snap: &mpris::Snapshot) {
    if let Some(child) = popover.child() {
        for_each_label(&child, &mut |lbl| match lbl.widget_name().as_str() {
            "music-pop-title" => lbl.set_text(&snap.title),
            "music-pop-artist" => lbl.set_text(&snap.artist),
            _ => {}
        });
    }
}

fn for_each_label(widget: &gtk::Widget, f: &mut impl FnMut(&Label)) {
    if let Some(lbl) = widget.downcast_ref::<Label>() {
        f(lbl);
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        for_each_label(&c, f);
        child = c.next_sibling();
    }
}
