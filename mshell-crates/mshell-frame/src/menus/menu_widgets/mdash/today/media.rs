//! Today: the media card. Now playing (cover, title, artist, progress,
//! transport, player switch) or — when nothing plays — a compact idle
//! state that still offers something to start: resume the last Tune
//! track, or one of the saved radio stations. The card never disappears,
//! so the Today grid keeps its shape.

use super::super::card::Card;
use relm4::gtk;
use relm4::gtk::prelude::{BoxExt, ButtonExt, WidgetExt};
use std::rc::Rc;
use std::time::Duration;

/// What a playing player shows.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct NowPlaying {
    pub(crate) player: String,
    pub(crate) title: String,
    pub(crate) artist: String,
    /// A local cover file (`file://` URLs are resolved by the adapter).
    pub(crate) art: Option<String>,
    pub(crate) position: Duration,
    pub(crate) length: Option<Duration>,
    pub(crate) playing: bool,
    pub(crate) can_prev: bool,
    pub(crate) can_next: bool,
    /// How many players can be switched between.
    pub(crate) players: usize,
}

/// The idle card's offers.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Idle {
    /// The last Tune track ("title — artist"), resumable.
    pub(crate) resume: Option<String>,
    /// Saved radio stations: (key, name), at most three shown.
    pub(crate) stations: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum MediaView {
    Playing(NowPlaying),
    Idle(Idle),
}

/// A click on the card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MediaEvent {
    PlayPause,
    Previous,
    Next,
    /// Show the next (true) / previous player.
    SwitchPlayer(bool),
    /// Resume the last Tune track.
    Resume,
    PlayStation(String),
    /// Open the music menu.
    OpenMusic,
}

/// "3:07" / "1:02:03".
pub(crate) fn clock(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// Which player to show: the first one playing, else the service's
/// active player if it isn't stopped, else the first one that isn't
/// stopped (a browser tab that merely registered MPRIS is never shown).
pub(crate) fn pick_player(
    playing: &[bool],
    stopped: &[bool],
    active: Option<usize>,
) -> Option<usize> {
    playing
        .iter()
        .position(|p| *p)
        .or_else(|| active.filter(|&i| !stopped.get(i).copied().unwrap_or(true)))
        .or_else(|| stopped.iter().position(|s| !*s))
}

pub(crate) struct MediaCard {
    pub(crate) card: Card,
    playing_box: gtk::Box,
    idle_box: gtk::Box,
    cover: gtk::Picture,
    cover_fallback: gtk::Image,
    title: gtk::Label,
    artist: gtk::Label,
    progress: gtk::ProgressBar,
    time: gtk::Label,
    play: gtk::Button,
    prev: gtk::Button,
    next: gtk::Button,
    switch: gtk::Box,
    resume: gtk::Button,
    resume_label: gtk::Label,
    stations: gtk::Box,
    on: Rc<dyn Fn(MediaEvent)>,
}

fn icon_button(icon: &str, name: &str, css: &str) -> gtk::Button {
    let b = gtk::Button::from_icon_name(icon);
    b.add_css_class(css);
    mshell_common::a11y::name(&b, name);
    b
}

impl MediaCard {
    pub(crate) fn new(on: impl Fn(MediaEvent) + 'static) -> Self {
        let on: Rc<dyn Fn(MediaEvent)> = Rc::new(on);
        let card = Card::new("media-playback-start-symbolic", "Media");
        card.root.add_css_class("mdash-media-card");

        // Player switch in the header row's meta slot area.
        let switch = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        let prev_player = icon_button(
            "go-previous-symbolic",
            "Previous player",
            "mdash-icon-button",
        );
        let next_player = icon_button("go-next-symbolic", "Next player", "mdash-icon-button");
        {
            let on = on.clone();
            prev_player.connect_clicked(move |_| on(MediaEvent::SwitchPlayer(false)));
        }
        {
            let on = on.clone();
            next_player.connect_clicked(move |_| on(MediaEvent::SwitchPlayer(true)));
        }
        switch.append(&prev_player);
        switch.append(&next_player);
        switch.set_visible(false);

        // ── playing ──
        let playing_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
        playing_box.set_vexpand(true);
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let art = gtk::Overlay::new();
        art.add_css_class("mdash-media-art");
        let cover_fallback = gtk::Image::from_icon_name("audio-x-generic-symbolic");
        cover_fallback.add_css_class("mdash-media-art-fallback");
        art.set_child(Some(&cover_fallback));
        let cover = gtk::Picture::new();
        cover.set_content_fit(gtk::ContentFit::Cover);
        cover.set_can_shrink(true);
        art.add_overlay(&cover);
        top.append(&art);
        let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
        text.set_valign(gtk::Align::Center);
        text.set_hexpand(true);
        let title = gtk::Label::new(None);
        title.add_css_class("mdash-media-title");
        title.set_xalign(0.0);
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        text.append(&title);
        let artist = gtk::Label::new(None);
        artist.add_css_class("mdash-dim");
        artist.set_xalign(0.0);
        artist.set_ellipsize(gtk::pango::EllipsizeMode::End);
        text.append(&artist);
        top.append(&text);
        top.append(&switch);
        playing_box.append(&top);

        let progress_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let progress = gtk::ProgressBar::new();
        progress.add_css_class("mdash-media-progress");
        progress.set_hexpand(true);
        progress.set_valign(gtk::Align::Center);
        progress_row.append(&progress);
        let time = gtk::Label::new(None);
        time.add_css_class("mdash-dim");
        progress_row.append(&time);
        playing_box.append(&progress_row);

        let transport = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        transport.set_halign(gtk::Align::Center);
        let prev = icon_button(
            "media-skip-backward-symbolic",
            "Previous track",
            "mdash-transport",
        );
        let play = icon_button("media-playback-start-symbolic", "Play", "mdash-transport");
        play.add_css_class("primary");
        let next = icon_button(
            "media-skip-forward-symbolic",
            "Next track",
            "mdash-transport",
        );
        for (b, ev) in [
            (&prev, MediaEvent::Previous),
            (&play, MediaEvent::PlayPause),
            (&next, MediaEvent::Next),
        ] {
            let on = on.clone();
            b.connect_clicked(move |_| on(ev.clone()));
            transport.append(b);
        }
        playing_box.append(&transport);

        // ── idle ──
        let idle_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
        idle_box.set_vexpand(true);
        let idle_line = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let idle_art = gtk::Image::from_icon_name("audio-x-generic-symbolic");
        idle_art.add_css_class("mdash-media-art-fallback");
        idle_art.add_css_class("mdash-media-art");
        idle_line.append(&idle_art);
        let idle_text = gtk::Box::new(gtk::Orientation::Vertical, 2);
        idle_text.set_valign(gtk::Align::Center);
        let nothing = gtk::Label::new(Some("Nothing playing"));
        nothing.add_css_class("mdash-media-title");
        nothing.set_xalign(0.0);
        idle_text.append(&nothing);
        let hint = gtk::Label::new(Some("Start something below"));
        hint.add_css_class("mdash-dim");
        hint.set_xalign(0.0);
        idle_text.append(&hint);
        idle_line.append(&idle_text);
        idle_box.append(&idle_line);

        let resume = gtk::Button::new();
        resume.add_css_class("mdash-row-button");
        let resume_content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        resume_content.append(&gtk::Image::from_icon_name("media-playback-start-symbolic"));
        let resume_label = gtk::Label::new(None);
        resume_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        resume_label.set_xalign(0.0);
        resume_content.append(&resume_label);
        resume.set_child(Some(&resume_content));
        {
            let on = on.clone();
            resume.connect_clicked(move |_| on(MediaEvent::Resume));
        }
        idle_box.append(&resume);

        let stations = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        stations.set_homogeneous(true);
        stations.set_valign(gtk::Align::End);
        stations.set_vexpand(true);
        idle_box.append(&stations);

        card.body.append(&playing_box);
        card.body.append(&idle_box);
        Self {
            card,
            playing_box,
            idle_box,
            cover,
            cover_fallback,
            title,
            artist,
            progress,
            time,
            play,
            prev,
            next,
            switch,
            resume,
            resume_label,
            stations,
            on,
        }
    }

    pub(crate) fn apply(&self, view: &MediaView) {
        match view {
            MediaView::Playing(np) => {
                self.playing_box.set_visible(true);
                self.idle_box.set_visible(false);
                self.card.set_title("Now playing");
                self.card.set_meta(&np.player);
                self.title.set_label(if np.title.is_empty() {
                    "Unknown title"
                } else {
                    &np.title
                });
                self.artist.set_label(&np.artist);
                self.artist.set_visible(!np.artist.is_empty());
                match np.art.as_deref() {
                    Some(path) if std::path::Path::new(path).is_file() => {
                        self.cover.set_filename(Some(path));
                        self.cover.set_visible(true);
                    }
                    _ => {
                        self.cover.set_paintable(gtk::gdk::Paintable::NONE);
                        self.cover.set_visible(false);
                    }
                }
                self.cover_fallback.set_visible(!self.cover.is_visible());
                match np.length.filter(|l| !l.is_zero()) {
                    Some(len) => {
                        let f = np.position.as_secs_f64() / len.as_secs_f64();
                        self.progress.set_fraction(f.clamp(0.0, 1.0));
                        self.time
                            .set_label(&format!("{} / {}", clock(np.position), clock(len)));
                    }
                    None => {
                        self.progress.set_fraction(0.0);
                        self.time.set_label("Live");
                    }
                }
                self.play.set_icon_name(if np.playing {
                    "media-playback-pause-symbolic"
                } else {
                    "media-playback-start-symbolic"
                });
                mshell_common::a11y::name(&self.play, if np.playing { "Pause" } else { "Play" });
                self.prev.set_sensitive(np.can_prev);
                self.next.set_sensitive(np.can_next);
                self.switch.set_visible(np.players > 1);
            }
            MediaView::Idle(idle) => {
                self.playing_box.set_visible(false);
                self.idle_box.set_visible(true);
                self.card.set_title("Media");
                self.card.set_meta("");
                match &idle.resume {
                    Some(track) => {
                        self.resume_label.set_label(&format!("Resume {track}"));
                        self.resume.set_visible(true);
                    }
                    None => self.resume.set_visible(false),
                }
                while let Some(child) = self.stations.first_child() {
                    self.stations.remove(&child);
                }
                for (key, name) in idle.stations.iter().take(2) {
                    let b = gtk::Button::new();
                    b.add_css_class("mdash-row-button");
                    let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                    content.append(&gtk::Image::from_icon_name("audio-radio-symbolic"));
                    let l = gtk::Label::new(Some(name));
                    l.set_ellipsize(gtk::pango::EllipsizeMode::End);
                    content.append(&l);
                    b.set_child(Some(&content));
                    mshell_common::a11y::name(&b, &format!("Play {name}"));
                    let (on, key) = (self.on.clone(), key.clone());
                    b.connect_clicked(move |_| on(MediaEvent::PlayStation(key.clone())));
                    self.stations.append(&b);
                }
                let open = gtk::Button::new();
                open.add_css_class("mdash-row-button");
                let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                content.append(&gtk::Image::from_icon_name("folder-music-symbolic"));
                content.append(&gtk::Label::new(Some("Music")));
                open.set_child(Some(&content));
                mshell_common::a11y::name(&open, "Open Tune");
                let on = self.on.clone();
                open.connect_clicked(move |_| on(MediaEvent::OpenMusic));
                self.stations.append(&open);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_formats() {
        assert_eq!(clock(Duration::from_secs(187)), "3:07");
        assert_eq!(clock(Duration::from_secs(3723)), "1:02:03");
        assert_eq!(clock(Duration::ZERO), "0:00");
    }

    #[test]
    fn the_playing_player_wins_then_active_then_first_awake() {
        // Second one plays.
        assert_eq!(
            pick_player(&[false, true], &[false, false], Some(0)),
            Some(1)
        );
        // Nothing plays: the active one, unless it is stopped.
        assert_eq!(
            pick_player(&[false, false], &[false, false], Some(1)),
            Some(1)
        );
        assert_eq!(
            pick_player(&[false, false], &[false, true], Some(1)),
            Some(0)
        );
        // Everything stopped: idle.
        assert_eq!(pick_player(&[false, false], &[true, true], Some(0)), None);
        assert_eq!(pick_player(&[], &[], None), None);
    }
}
