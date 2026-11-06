// SPDX-FileCopyrightText: 2022  Emmanuele Bassi
// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    cell::{Cell, RefCell},
    fmt::{self, Display, Formatter},
    path::PathBuf,
    time::Instant,
};

use glib::{ParamSpec, ParamSpecBoolean, ParamSpecObject, ParamSpecString, ParamSpecUInt, Value};
use gtk::{gdk, gio, glib, prelude::*, subclass::prelude::*};
use itertools::Itertools;
use lofty::{
    prelude::{Accessor, TaggedFileExt},
    probe::Probe,
    tag::ItemKey,
};
use log::{debug, warn};
use once_cell::sync::Lazy;
use sha2::{Digest, Sha256};

use crate::{
    audio::{
        cover_cache::{CoverArt, CoverCache},
        stream::{self, StreamInfo, StreamKind},
    },
    i18n::i18n,
};

/// Where a network song came from, and what it reports while playing.
#[derive(Debug, Clone, Default)]
struct StreamMeta {
    key: String,
    /// Radio: the station name / its description (country, genres).
    station: String,
    description: String,
    /// Radio: the current programme from the stream's `StreamTitle`.
    now_playing: Option<String>,
    art_url: String,
}

#[derive(Debug, Clone)]
pub struct SongData {
    artist: Option<String>,
    title: Option<String>,
    album: Option<String>,
    lyrics: Option<String>,
    cover_art: Option<CoverArt>,
    cover_uuid: Option<String>,
    uuid: Option<String>,
    duration: u64,
    file: gio::File,
    kind: StreamKind,
    stream: Option<StreamMeta>,
}

/// Embedded lyrics text, if the file's tag carries any. `ItemKey::Lyrics`
/// ("possibly synchronized") covers Vorbis Comments and MP4 `©lyr`;
/// **ID3v2 does not map that key at all** — an MP3's USLT frame only
/// comes through `ItemKey::UnsyncLyrics`, so that's the fallback. v1 shows
/// whatever comes back as plain text, even if it happens to contain
/// `[mm:ss.xx]`-style timestamps (no LRC parsing here).
fn resolve_lyrics(tag: &lofty::tag::Tag) -> Option<String> {
    tag.get_string(ItemKey::Lyrics)
        .or_else(|| tag.get_string(ItemKey::UnsyncLyrics))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

impl SongData {
    pub fn artist(&self) -> Option<&str> {
        self.artist.as_deref()
    }

    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    pub fn album(&self) -> Option<&str> {
        self.album.as_deref()
    }

    pub fn lyrics(&self) -> Option<&str> {
        self.lyrics.as_deref()
    }

    pub fn uuid(&self) -> Option<&str> {
        self.uuid.as_deref()
    }

    pub fn cover_uuid(&self) -> Option<&str> {
        self.cover_uuid.as_deref()
    }

    pub fn duration(&self) -> u64 {
        self.duration
    }

    pub fn cover_texture(&self) -> Option<&gdk::Texture> {
        if let Some(cover) = &self.cover_art {
            return Some(cover.texture());
        }

        None
    }

    pub fn cover_palette(&self) -> Option<&Vec<gdk::RGBA>> {
        if let Some(cover) = &self.cover_art {
            return Some(cover.palette());
        }

        None
    }

    pub fn cover_cache(&self) -> Option<&PathBuf> {
        if let Some(cover) = &self.cover_art {
            return cover.cache();
        }

        None
    }

    pub fn from_uri(uri: &str) -> Self {
        let now = Instant::now();

        let file = gio::File::for_uri(uri);
        let Some(path) = file.path() else {
            warn!("Not a local file: {uri}");
            return SongData::default();
        };

        let (mime_type, _) = gio::content_type_guess(Some(&path), None);
        if !mime_type.starts_with("audio") || mime_type.ends_with("x-mpegurl") {
            return SongData::default();
        }

        let file_probe = match Probe::open(&path) {
            Ok(p) => p,
            Err(e) => {
                warn!("Unable to open file {:?}: {}", path, e);
                return SongData::default();
            }
        };

        let tagged_file = match file_probe.guess_file_type() {
            Ok(t) => match t.read() {
                Ok(f) => f,
                Err(e) => {
                    warn!("Unable to read parsed file {:?}: {}", path, e);
                    return SongData::default();
                }
            },
            Err(e) => {
                warn!("Unable to parse file type {:?}: {}", path, e);
                return SongData::default();
            }
        };

        let mut cover_cache = crate::audio::cover_cache::lock(CoverCache::global());

        let mut artist = None;
        let mut title = None;
        let mut album = None;
        let mut lyrics = None;
        let mut cover_art = None;
        let mut cover_uuid = None;
        if let Some(tag) = tagged_file.primary_tag() {
            debug!("Found primary tag");
            artist = Some(tag.get_strings(ItemKey::TrackArtist).join(", "));
            title = tag.title().map(|s| s.to_string());
            album = tag.album().map(|s| s.to_string());
            lyrics = resolve_lyrics(tag);
            if let Some(res) = cover_cache.cover_art(&path, tag) {
                cover_art = Some(res.0);
                cover_uuid = Some(res.1);
            }
        } else {
            warn!("Unable to load primary tag for: {}", uri);
            for tag in tagged_file.tags() {
                debug!("Found tag: {:?}", tag.tag_type());
                artist = Some(tag.get_strings(ItemKey::TrackArtist).join(", "));
                title = tag.title().map(|s| s.to_string());
                album = tag.album().map(|s| s.to_string());
                lyrics = resolve_lyrics(tag);
                if let Some(res) = cover_cache.cover_art(&path, tag) {
                    cover_art = Some(res.0);
                    cover_uuid = Some(res.1);
                }

                if artist.is_some() && title.is_some() {
                    break;
                }
            }
        };

        let uuid = match file.query_info(
            "standard::display-name",
            gio::FileQueryInfoFlags::NONE,
            gio::Cancellable::NONE,
        ) {
            Ok(info) => {
                let mut hasher = Sha256::new();

                hasher.update(info.display_name().as_str());

                if let Some(ref artist) = artist {
                    hasher.update(artist);
                }
                if let Some(ref title) = title {
                    hasher.update(title);
                }
                if let Some(ref album) = album {
                    hasher.update(album);
                }

                Some(format!("{:x}", hasher.finalize()))
            }
            _ => None,
        };

        let properties = lofty::prelude::AudioFile::properties(&tagged_file);
        let duration = properties.duration().as_secs();

        debug!(
            "Song {:?} ('{:?}') loading time: {} ms",
            &uuid,
            &title,
            now.elapsed().as_millis()
        );

        SongData {
            artist,
            title,
            album,
            lyrics,
            cover_art,
            cover_uuid,
            uuid,
            duration,
            file,
            kind: StreamKind::File,
            stream: None,
        }
    }

    /// A radio station or podcast episode: metadata from the directory /
    /// feed, no tag probing (the file is remote).
    pub fn from_stream(info: StreamInfo) -> Self {
        let (title, artist) = match info.kind {
            StreamKind::Radio => stream::radio_display(&info.title, &info.artist, None),
            _ => (info.title.clone(), info.artist.clone()),
        };
        let some = |s: String| (!s.trim().is_empty()).then_some(s);
        SongData {
            artist: some(artist),
            title: some(title),
            album: some(info.album.clone()),
            lyrics: None,
            cover_art: None,
            cover_uuid: None,
            uuid: Some(stream::stream_uuid(info.kind, &info.key)),
            duration: info.duration,
            file: gio::File::for_uri(&info.uri),
            kind: info.kind,
            stream: Some(StreamMeta {
                key: info.key,
                station: info.title,
                description: info.artist,
                now_playing: None,
                art_url: info.art_url,
            }),
        }
    }

    pub fn uri(&self) -> String {
        self.file.uri().to_string()
    }

    pub fn file(&self) -> gio::File {
        self.file.clone()
    }
}

impl Default for SongData {
    fn default() -> Self {
        SongData {
            artist: Some("Invalid Artist".to_string()),
            title: Some("Invalid Title".to_string()),
            album: Some("Invalid Album".to_string()),
            lyrics: None,
            cover_art: None,
            cover_uuid: None,
            uuid: None,
            duration: 0,
            file: gio::File::for_path("/does-not-exist"),
            kind: StreamKind::File,
            stream: None,
        }
    }
}

mod imp {
    use super::*;

    #[derive(Debug, Default)]
    pub struct Song {
        pub data: RefCell<SongData>,
        pub playing: Cell<bool>,
        pub selected: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Song {
        const NAME: &'static str = "TuneSong";
        type Type = super::Song;
    }

    impl ObjectImpl for Song {
        fn properties() -> &'static [ParamSpec] {
            static PROPERTIES: Lazy<Vec<ParamSpec>> = Lazy::new(|| {
                vec![
                    ParamSpecString::builder("uri").construct_only().build(),
                    ParamSpecString::builder("artist").read_only().build(),
                    ParamSpecString::builder("title").read_only().build(),
                    ParamSpecString::builder("album").read_only().build(),
                    ParamSpecUInt::builder("duration").read_only().build(),
                    ParamSpecObject::builder::<gdk::Texture>("cover")
                        .read_only()
                        .build(),
                    ParamSpecBoolean::builder("playing").build(),
                    ParamSpecBoolean::builder("selected").build(),
                ]
            });
            PROPERTIES.as_ref()
        }

        // GObject checks the name and value type against the registered
        // pspecs before calling this: the value `get`s cannot fail and the
        // `_` arm is unreachable.
        fn set_property(&self, _id: usize, value: &Value, pspec: &ParamSpec) {
            match pspec.name() {
                "uri" => {
                    let obj = self.obj();
                    if let Ok(p) = value.get::<&str>() {
                        self.data.replace(SongData::from_uri(p));
                        obj.notify("artist");
                        obj.notify("title");
                        obj.notify("album");
                        obj.notify("duration");
                        obj.notify("cover");
                    }
                }
                "playing" => {
                    let p = value.get::<bool>().expect("Value must be a boolean");
                    self.playing.set(p);
                }
                "selected" => {
                    let p = value.get::<bool>().expect("Value must be a boolean");
                    self.selected.set(p);
                }
                _ => unimplemented!(),
            }
        }

        // Only registered pspecs reach this: the `_` arm is unreachable.
        fn property(&self, _id: usize, pspec: &ParamSpec) -> Value {
            let obj = self.obj();
            match pspec.name() {
                "artist" => obj.artist().to_value(),
                "title" => obj.title().to_value(),
                "album" => obj.album().to_value(),
                "duration" => obj.duration().to_value(),
                "uri" => obj.uri().to_value(),
                "cover" => obj.cover_texture().to_value(),
                "playing" => self.playing.get().to_value(),
                "selected" => self.selected.get().to_value(),
                _ => unimplemented!(),
            }
        }
    }
}

glib::wrapper! {
    pub struct Song(ObjectSubclass<imp::Song>);
}

impl Song {
    pub fn new(uri: &str) -> Self {
        glib::Object::builder::<Self>().property("uri", uri).build()
    }

    pub fn from_uri(uri: &str) -> Result<Song, &'static str> {
        let res = Song::new(uri);
        if res.equals(&Song::default()) {
            Err("Invalid song")
        } else {
            Ok(res)
        }
    }

    pub fn empty() -> Self {
        glib::Object::new()
    }

    /// A radio station or podcast episode (see [`StreamInfo`]).
    pub fn from_stream(info: StreamInfo) -> Self {
        let song = Self::empty();
        song.imp().data.replace(SongData::from_stream(info));
        song
    }

    pub fn kind(&self) -> StreamKind {
        self.imp().data.borrow().kind
    }

    /// Radio or podcast: played over the network, never tag-probed, no
    /// waveform, not written to `.pls`/`.m3u` playlists.
    pub fn is_stream(&self) -> bool {
        self.kind() != StreamKind::File
    }

    /// A live radio stream: no duration, nothing to seek.
    pub fn is_live(&self) -> bool {
        self.kind() == StreamKind::Radio
    }

    /// The station / episode key the stream was queued with.
    pub fn stream_key(&self) -> Option<String> {
        self.imp()
            .data
            .borrow()
            .stream
            .as_ref()
            .map(|m| m.key.clone())
    }

    /// The logo / cover URL from the directory or feed (may be empty).
    pub fn art_url(&self) -> Option<String> {
        let data = self.imp().data.borrow();
        let url = data.stream.as_ref()?.art_url.clone();
        (!url.is_empty()).then_some(url)
    }

    /// The station name of a radio entry (its title before any programme
    /// title arrived).
    pub fn station_name(&self) -> Option<String> {
        let data = self.imp().data.borrow();
        (data.kind == StreamKind::Radio)
            .then(|| data.stream.as_ref().map(|m| m.station.clone()))
            .flatten()
    }

    /// Feed a live stream's `StreamTitle`. Returns `true` when what the
    /// entry shows changed (the caller re-announces it over MPRIS).
    pub fn set_now_playing(&self, raw: &str) -> bool {
        let changed = {
            let mut data = self.imp().data.borrow_mut();
            if data.kind != StreamKind::Radio {
                return false;
            }
            let Some(meta) = data.stream.as_mut() else {
                return false;
            };
            let np = stream::clean_stream_title(raw, &meta.station);
            if np == meta.now_playing {
                false
            } else {
                meta.now_playing = np;
                let (title, artist) = stream::radio_display(
                    &meta.station,
                    &meta.description,
                    meta.now_playing.as_deref(),
                );
                data.title = Some(title);
                data.artist = (!artist.is_empty()).then_some(artist);
                true
            }
        };
        if changed {
            self.notify("title");
            self.notify("artist");
        }
        changed
    }

    /// The real length of a podcast episode once GStreamer knows it (feeds
    /// round it, or omit it). Returns `true` when it changed.
    pub fn set_stream_duration(&self, secs: u64) -> bool {
        {
            let mut data = self.imp().data.borrow_mut();
            if data.kind != StreamKind::Episode || secs == 0 || data.duration == secs {
                return false;
            }
            data.duration = secs;
        }
        self.notify("duration");
        true
    }

    /// Artwork fetched for a stream (station logo, podcast cover).
    pub fn set_stream_cover(&self, cover: CoverArt, cover_uuid: String) {
        {
            let mut data = self.imp().data.borrow_mut();
            if data.kind == StreamKind::File {
                return;
            }
            data.cover_art = Some(cover);
            data.cover_uuid = Some(cover_uuid);
        }
        self.notify("cover");
    }

    pub fn equals(&self, other: &Self) -> bool {
        if self.uuid().is_some() && other.uuid().is_some() {
            self.uuid() == other.uuid()
        } else {
            self.uri() == other.uri()
        }
    }

    pub fn uri(&self) -> String {
        self.imp().data.borrow().uri()
    }

    pub fn artist(&self) -> String {
        match self.imp().data.borrow().artist() {
            Some(artist) => artist.to_string(),
            None => i18n("Unknown artist"),
        }
    }

    pub fn title(&self) -> String {
        match self.imp().data.borrow().title() {
            Some(title) => title.to_string(),
            None => i18n("Unknown title"),
        }
    }

    pub fn album(&self) -> String {
        match self.imp().data.borrow().album() {
            Some(album) => album.to_string(),
            None => i18n("Unknown album"),
        }
    }

    /// Plain empty string when absent — unlike `artist()`/`title()`/
    /// `album()`, this must NOT localize an "Unknown …" placeholder: it
    /// feeds `org.margo.Tune::EmbeddedLyrics`, and the shell treats a
    /// non-empty string there as "this track has embedded lyrics".
    pub fn lyrics(&self) -> String {
        match self.imp().data.borrow().lyrics() {
            Some(lyrics) => lyrics.to_string(),
            None => String::new(),
        }
    }

    pub fn cover_texture(&self) -> Option<gdk::Texture> {
        self.imp().data.borrow().cover_texture().cloned()
    }

    pub fn cover_color(&self) -> Option<gdk::RGBA> {
        self.imp().data.borrow().cover_palette().map(|p| p[0])
    }

    pub fn cover_palette(&self) -> Option<Vec<gdk::RGBA>> {
        self.imp().data.borrow().cover_palette().cloned()
    }

    pub fn cover_uuid(&self) -> Option<String> {
        self.imp().data.borrow().cover_uuid().map(|s| s.to_string())
    }

    pub fn cover_cache(&self) -> Option<PathBuf> {
        self.imp().data.borrow().cover_cache().cloned()
    }

    pub fn duration(&self) -> u64 {
        self.imp().data.borrow().duration()
    }

    pub fn playing(&self) -> bool {
        self.imp().playing.get()
    }

    pub fn set_playing(&self, playing: bool) {
        let was_playing = self.imp().playing.replace(playing);
        if was_playing != playing {
            self.notify("playing");
        }
    }

    pub fn selected(&self) -> bool {
        self.imp().selected.get()
    }

    pub fn set_selected(&self, selected: bool) {
        let was_selected = self.imp().selected.replace(selected);
        if was_selected != selected {
            self.notify("selected");
        }
    }

    pub fn uuid(&self) -> Option<String> {
        self.imp().data.borrow().uuid().map(|s| s.to_string())
    }

    pub fn search_key(&self) -> String {
        format!("{} {} {}", self.artist(), self.album(), self.title())
    }

    pub fn file(&self) -> gio::File {
        self.imp().data.borrow().file()
    }
}

impl Default for Song {
    fn default() -> Self {
        Self::empty()
    }
}

impl Display for Song {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(
            f,
            "Song {{ uuid: {}, uri: {}, artist: '{}', title: '{}' }}",
            self.uuid().unwrap_or_default(),
            self.uri(),
            self.artist(),
            self.title()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lofty::tag::{Tag, TagType};

    #[test]
    fn prefers_the_generic_lyrics_key() {
        let mut tag = Tag::new(TagType::VorbisComments);
        tag.insert_text(ItemKey::Lyrics, "la la la".to_string());
        tag.insert_text(ItemKey::UnsyncLyrics, "should not win".to_string());
        assert_eq!(resolve_lyrics(&tag).as_deref(), Some("la la la"));
    }

    #[test]
    fn falls_back_to_unsync_lyrics_for_id3v2() {
        // ID3v2 doesn't map ItemKey::Lyrics at all — only UnsyncLyrics
        // (the USLT frame). A tag that only has UnsyncLyrics must still
        // resolve.
        let mut tag = Tag::new(TagType::Id3v2);
        tag.insert_text(ItemKey::UnsyncLyrics, "hello darkness".to_string());
        assert_eq!(resolve_lyrics(&tag).as_deref(), Some("hello darkness"));
    }

    #[test]
    fn trims_and_rejects_whitespace_only() {
        let mut tag = Tag::new(TagType::VorbisComments);
        tag.insert_text(ItemKey::Lyrics, "  padded  ".to_string());
        assert_eq!(resolve_lyrics(&tag).as_deref(), Some("padded"));

        let mut blank = Tag::new(TagType::VorbisComments);
        blank.insert_text(ItemKey::Lyrics, "   ".to_string());
        assert_eq!(resolve_lyrics(&blank), None);
    }

    #[test]
    fn a_non_local_uri_is_an_invalid_song_not_a_panic() {
        // MPRIS OpenUri / a dropped remote file hand mtune URIs that
        // have no local path.
        for uri in ["https://example.invalid/a.mp3", "sftp://host/music/a.flac"] {
            let data = SongData::from_uri(uri);
            assert!(data.uuid().is_none(), "{uri}");
        }
    }

    #[test]
    fn an_invalid_song_still_formats() {
        // Display runs in log lines; an invalid song has no uuid.
        let shown = format!("{}", Song::empty());
        assert!(shown.starts_with("Song { uuid: , uri: "), "{shown}");
    }

    fn station() -> Song {
        Song::from_stream(StreamInfo {
            kind: StreamKind::Radio,
            uri: "https://icecast.walmradio.com:8443/jazz".into(),
            key: "rb:ea8059be-d119-4de3-b27b-0d9bd6aedb17".into(),
            title: "Adroit Jazz Underground".into(),
            artist: "The United States Of America · jazz".into(),
            album: "Internet radio".into(),
            ..Default::default()
        })
    }

    #[test]
    fn a_station_is_a_valid_live_song() {
        let s = station();
        assert!(s.is_stream() && s.is_live());
        assert_eq!(s.uri(), "https://icecast.walmradio.com:8443/jazz");
        assert_eq!(s.title(), "Adroit Jazz Underground");
        assert_eq!(s.artist(), "The United States Of America · jazz");
        assert_eq!(s.duration(), 0);
        assert!(s.uuid().is_some());
        assert!(s.file().path().is_none(), "never written to a .pls");
        assert!(s.equals(&station()), "the same station is one queue entry");
    }

    #[test]
    fn the_stream_title_moves_the_station_to_the_artist_line() {
        let s = station();
        assert!(s.set_now_playing("My Foolish Heart by Bill Watrous"));
        assert_eq!(s.title(), "My Foolish Heart by Bill Watrous");
        assert_eq!(s.artist(), "Adroit Jazz Underground");
        assert!(
            !s.set_now_playing(" My Foolish  Heart by Bill Watrous "),
            "same title"
        );
        assert!(s.set_now_playing(""), "programme ended");
        assert_eq!(s.title(), "Adroit Jazz Underground");
        assert_eq!(s.station_name().as_deref(), Some("Adroit Jazz Underground"));
    }

    #[test]
    fn only_episodes_take_a_reported_duration() {
        let ep = Song::from_stream(StreamInfo {
            kind: StreamKind::Episode,
            uri: "https://example.invalid/e.mp3".into(),
            key: "feed#1".into(),
            title: "Episode".into(),
            duration: 3900,
            ..Default::default()
        });
        assert!(ep.set_stream_duration(3944));
        assert_eq!(ep.duration(), 3944);
        assert!(!ep.set_stream_duration(0));
        assert!(!ep.set_now_playing("x"), "episodes have no StreamTitle");
        assert!(!station().set_stream_duration(10));
    }

    #[test]
    fn no_lyrics_tag_at_all() {
        let tag = Tag::new(TagType::VorbisComments);
        assert_eq!(resolve_lyrics(&tag), None);
    }
}
