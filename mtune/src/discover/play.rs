// SPDX-License-Identifier: GPL-3.0-or-later

//! Turning directory stations and feed episodes into queue entries.

use crate::{
    audio::{Song, StreamInfo, StreamKind},
    i18n::i18n,
    net,
    podcasts::{Episode, Podcast},
    radio::{Station, api},
};

use super::Ctx;

pub fn station_song(s: &Station) -> Song {
    Song::from_stream(StreamInfo {
        kind: StreamKind::Radio,
        uri: s.url.clone(),
        key: s.key(),
        title: s.name.clone(),
        artist: s.subtitle(),
        album: i18n("Internet radio"),
        duration: 0,
        art_url: s.favicon.clone(),
    })
}

/// Episode entries read "Episode title — Podcast" (MPRIS title / artist),
/// which is what the shell's media widget shows.
pub fn episode_song(p: &Podcast, e: &Episode) -> Song {
    Song::from_stream(StreamInfo {
        kind: StreamKind::Episode,
        uri: e.url.clone(),
        key: e.key(&p.feed_url),
        title: e.title.clone(),
        artist: p.title.clone(),
        album: if p.author.is_empty() {
            p.title.clone()
        } else {
            p.author.clone()
        },
        duration: e.duration,
        art_url: if e.image.is_empty() {
            p.artwork.clone()
        } else {
            e.image.clone()
        },
    })
}

/// Play `list[index]` with the whole list queued (next/previous step
/// through it), and count the play with the directory.
pub fn play_stations(ctx: &Ctx, list: &[Station], index: usize) {
    let Some(win) = ctx.window() else { return };
    let songs: Vec<Song> = list.iter().map(station_song).collect();
    win.play_station_list(&songs, index as u32);
    if let Some(st) = list.get(index)
        && !st.custom
    {
        let uuid = st.uuid.clone();
        glib_spawn(async move {
            net::off_main(move || api::count_click(&uuid)).await;
        });
    }
}

/// Queue (and with `play_now`, play) one episode, resuming where it was
/// left.
pub fn play_episode(ctx: &Ctx, p: &Podcast, e: &Episode, play_now: bool) {
    let Some(win) = ctx.window() else { return };
    let resume = super::shared::podcasts()
        .data
        .borrow()
        .resume_at(&e.key(&p.feed_url));
    win.queue_episode(&episode_song(p, e), play_now, resume);
    if play_now && resume > 0 {
        ctx.toast(&crate::i18n::i18n_k(
            // Translators: `{time}` is a position like 12:34.
            "Resuming at {time}",
            &[("time", &crate::utils::format_time(resume as i64))],
        ));
    }
}

fn glib_spawn(f: impl std::future::Future<Output = ()> + 'static) {
    gtk::glib::spawn_future_local(f);
}
