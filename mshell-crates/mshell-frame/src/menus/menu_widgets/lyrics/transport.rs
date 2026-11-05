//! The lyrics panel's player footer: seek bar + time labels, prev / play /
//! next, and the per-track timing stepper (`−  +0.25 s  +`). Built
//! imperatively and appended below the lines scroller (§5: the footer is its
//! own region, never inside the scroller). It reuses the media player card's
//! classes (`media-player-progress-row`, `ok-progress-bar`,
//! `media-player-controls`, `media-player-ctl-primary`) and its debounced
//! seek plumbing, so both read and behave alike.

use super::lyrics_menu_widget::{LyricsMenuWidgetInput, LyricsMenuWidgetModel};
use crate::lyrics::offsets;
use crate::menus::menu_widgets::media_player::seek::{
    seek_fraction, set_scale_silently, setup_debounced_seek,
};
use mshell_common::a11y;
use mshell_utils::media::format_duration;
use relm4::ComponentSender;
use relm4::gtk::prelude::*;
use relm4::gtk::{self, glib};
use std::time::Duration;

/// What the footer shows; recomputed by the menu on each position update.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct TransportState {
    pub position: Duration,
    pub length: Option<Duration>,
    pub playing: bool,
    pub can_seek: bool,
    pub can_prev: bool,
    pub can_next: bool,
    /// The lyrics are synced — the offset stepper only means something then.
    pub synced: bool,
    /// This track's nudge (ms).
    pub track_offset_ms: i32,
}

pub(super) struct Transport {
    root: gtk::Box,
    scale: gtk::Scale,
    scale_handler: glib::SignalHandlerId,
    elapsed: gtk::Label,
    length: gtk::Label,
    prev: gtk::Button,
    play_icon: gtk::Image,
    next: gtk::Button,
    offset_row: gtk::Box,
    offset_label: gtk::Button,
    last: Option<TransportState>,
}

fn icon_button(icon: &str, name: &str) -> (gtk::Button, gtk::Image) {
    let image = gtk::Image::from_icon_name(icon);
    image.set_halign(gtk::Align::Center);
    image.set_valign(gtk::Align::Center);
    let button = gtk::Button::builder()
        .css_classes(["ok-button-surface"])
        .child(&image)
        .build();
    a11y::name(&button, name);
    (button, image)
}

impl Transport {
    pub(super) fn new(sender: &ComponentSender<LyricsMenuWidgetModel>) -> Self {
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["lyrics-transport"])
            .build();

        // Seek row.
        let progress = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .css_classes(["media-player-progress-row"])
            .hexpand(true)
            .build();
        let elapsed = gtk::Label::builder()
            .css_classes(["media-player-time"])
            .label("0:00")
            .build();
        let scale = gtk::Scale::builder()
            .css_classes(["ok-progress-bar"])
            .hexpand(true)
            .can_focus(false)
            .focus_on_click(false)
            .build();
        scale.set_range(0.0, 1.0);
        a11y::name(&scale, "Seek");
        let length = gtk::Label::builder()
            .css_classes(["media-player-time"])
            .label("0:00")
            .build();
        progress.append(&elapsed);
        progress.append(&scale);
        progress.append(&length);
        let scale_handler = setup_debounced_seek(
            &scale,
            sender,
            LyricsMenuWidgetInput::SeekPreview,
            LyricsMenuWidgetInput::SeekCommit,
        );

        // Transport buttons.
        let controls = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .css_classes(["media-player-controls"])
            .halign(gtk::Align::Center)
            .spacing(8)
            .build();
        let (prev, _) = icon_button("media-skip-previous-symbolic", "Previous track");
        let (play, play_icon) = icon_button("media-play-symbolic", "Play or pause");
        play.add_css_class("media-player-ctl-primary");
        let (next, _) = icon_button("media-skip-next-symbolic", "Next track");
        let buttons: [(&gtk::Button, fn() -> LyricsMenuWidgetInput); 3] = [
            (&prev, || LyricsMenuWidgetInput::Previous),
            (&play, || LyricsMenuWidgetInput::PlayPause),
            (&next, || LyricsMenuWidgetInput::Next),
        ];
        for (b, msg) in buttons {
            let s = sender.clone();
            b.connect_clicked(move |_| {
                let _ = s.input_sender().send(msg());
            });
            controls.append(b);
        }

        // Per-track timing stepper.
        let offset_row = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .css_classes(["lyrics-offset-row"])
            .halign(gtk::Align::Center)
            .spacing(4)
            .build();
        let (earlier_btn, _) = icon_button("list-remove-symbolic", "Show lines later");
        let offset_label = gtk::Button::builder()
            .css_classes(["ok-button-flat", "lyrics-offset-label"])
            .label(offsets::format(0))
            .tooltip_text("This track's lyric timing — click to reset")
            .build();
        offset_label.update_property(&[gtk::accessible::Property::Label(
            "Reset this track's lyric timing",
        )]);
        let (later_btn, _) = icon_button("list-add-symbolic", "Show lines earlier");
        for (b, delta) in [
            (&earlier_btn, -offsets::STEP_MS),
            (&offset_label, 0),
            (&later_btn, offsets::STEP_MS),
        ] {
            b.add_css_class("lyrics-offset-btn");
            let s = sender.clone();
            b.connect_clicked(move |_| {
                let _ = s
                    .input_sender()
                    .send(LyricsMenuWidgetInput::NudgeOffset(delta));
            });
            offset_row.append(b);
        }

        root.append(&progress);
        root.append(&controls);
        root.append(&offset_row);

        Self {
            root,
            scale,
            scale_handler,
            elapsed,
            length,
            prev,
            play_icon,
            next,
            offset_row,
            offset_label,
            last: None,
        }
    }

    pub(super) fn root(&self) -> &gtk::Box {
        &self.root
    }

    /// Show a dragged-but-not-committed seek position.
    pub(super) fn preview(&self, fraction: f64, length: Duration) {
        self.elapsed
            .set_label(&format_duration(length.mul_f64(fraction)));
        set_scale_silently(&self.scale, Some(&self.scale_handler), fraction);
    }

    /// Repaint what changed since the last state.
    pub(super) fn set_state(&mut self, st: TransportState) {
        if self.last.as_ref() == Some(&st) {
            return;
        }
        let length = st.length.unwrap_or_default();
        self.elapsed.set_label(&format_duration(st.position));
        self.length.set_label(&format_duration(length));
        set_scale_silently(
            &self.scale,
            Some(&self.scale_handler),
            seek_fraction(st.position, length),
        );
        self.scale.set_sensitive(st.can_seek && !length.is_zero());
        self.prev.set_sensitive(st.can_prev);
        self.next.set_sensitive(st.can_next);
        self.play_icon.set_icon_name(Some(if st.playing {
            "media-pause-symbolic"
        } else {
            "media-play-symbolic"
        }));
        self.offset_row.set_visible(st.synced);
        self.offset_label
            .set_label(&offsets::format(st.track_offset_ms));
        self.last = Some(st);
    }
}
