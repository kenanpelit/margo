//! Native MPD (Music Player Daemon) backend.
//!
//! MPD speaks its own TCP protocol, not MPRIS — invisible to `wayle-media`
//! unless the user runs an MPD→MPRIS bridge (mpDris2 etc.), which the media
//! widgets otherwise silently depend on. This connects directly instead,
//! using `mpd_client`'s `idle` command (exposed as a `ConnectionEvents`
//! stream) for push updates, so it feels as live as an MPRIS player rather
//! than a polling timer.
//!
//! [`MpdPlayer`] exposes the same `wayle_core::Property<T>` shape
//! (`.get()`/`.watch()`) as every wayle-* service field, so the media
//! widgets can watch it with the exact same `watch_cancellable!` pattern
//! they already use for `wayle_media::core::player::Player`.
//!
//! MPD may simply not be installed or running; that is a normal desktop,
//! not an error, so connection failure only logs at `debug` and the
//! reconnect loop just keeps trying quietly.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use mpd_client::Client;
use mpd_client::client::{ConnectionEvent, Subsystem};
use mpd_client::commands;
use mpd_client::responses::PlayState;
use tokio::net::TcpStream;
use tokio::sync::RwLock;
use tracing::{debug, info};
use wayle_core::Property;
use wayle_media::types::PlaybackState;

use crate::tokio_rt;

/// Delay between reconnect attempts while MPD is unreachable.
const RECONNECT_DELAY: Duration = Duration::from_secs(5);

/// `$MPD_HOST`/`$MPD_PORT`, matching every other MPD client (`mpc`,
/// `ncmpcpp`, …) so this needs no config of its own for the common case.
fn mpd_addr() -> String {
    let host = std::env::var("MPD_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port = std::env::var("MPD_PORT").unwrap_or_else(|_| "6600".to_string());
    format!("{host}:{port}")
}

/// Live MPD player state + controls, read/watched directly by the media
/// bar pill and menu card alongside `wayle_media::core::player::Player`.
pub struct MpdPlayer {
    pub connected: Property<bool>,
    pub playback_state: Property<PlaybackState>,
    pub title: Property<String>,
    pub artist: Property<String>,
    pub album: Property<String>,
    /// Local file path to a cached cover image, or `None`. Same contract as
    /// `TrackMetadata::cover_art` — the UI just hands it to
    /// `gtk::Image::set_from_file`.
    pub cover_art: Property<Option<String>>,
    pub position: Property<Duration>,
    pub duration: Property<Duration>,
    client: RwLock<Option<Client>>,
}

impl MpdPlayer {
    fn new() -> Self {
        Self {
            connected: Property::new(false),
            playback_state: Property::new(PlaybackState::Stopped),
            title: Property::new(String::new()),
            artist: Property::new(String::new()),
            album: Property::new(String::new()),
            cover_art: Property::new(None),
            position: Property::new(Duration::ZERO),
            duration: Property::new(Duration::ZERO),
            client: RwLock::new(None),
        }
    }

    async fn with_client<F, Fut>(&self, f: F) -> anyhow::Result<()>
    where
        F: FnOnce(Client) -> Fut,
        Fut: std::future::Future<Output = anyhow::Result<()>>,
    {
        let client = self.client.read().await.clone();
        match client {
            Some(client) => f(client).await,
            None => anyhow::bail!("mpd: not connected"),
        }
    }

    pub async fn play_pause(&self) -> anyhow::Result<()> {
        let pause = self.playback_state.get() == PlaybackState::Playing;
        self.with_client(|client| async move {
            client.command(commands::SetPause(pause)).await?;
            Ok(())
        })
        .await
    }

    pub async fn next(&self) -> anyhow::Result<()> {
        self.with_client(|client| async move {
            client.command(commands::Next).await?;
            Ok(())
        })
        .await
    }

    pub async fn previous(&self) -> anyhow::Result<()> {
        self.with_client(|client| async move {
            client.command(commands::Previous).await?;
            Ok(())
        })
        .await
    }

    pub async fn seek(&self, position: Duration) -> anyhow::Result<()> {
        self.with_client(|client| async move {
            client
                .command(commands::Seek(commands::SeekMode::Absolute(position)))
                .await?;
            Ok(())
        })
        .await
    }
}

pub struct MpdService {
    pub player: Arc<MpdPlayer>,
}

static MPD_SERVICE: OnceLock<Arc<MpdService>> = OnceLock::new();

/// The MPD service singleton. Lazily constructed on first access —
/// always succeeds, starting in a "not connected" state — so callers
/// never need to handle an uninitialized case the way `media_service()`
/// would. [`spawn_mpd_watcher`] is what actually connects.
pub fn mpd_service() -> Arc<MpdService> {
    MPD_SERVICE
        .get_or_init(|| {
            Arc::new(MpdService {
                player: Arc::new(MpdPlayer::new()),
            })
        })
        .clone()
}

/// Spawn the background connect/idle loop on the services runtime. Call
/// once, after [`crate::init_services`]. Not part of `init_services`
/// itself because — unlike the `try_join!`-built services — MPD's
/// presence is optional and ongoing rather than a one-shot construction:
/// there is nothing to await here, just a loop to start.
pub fn spawn_mpd_watcher() {
    let service = mpd_service();
    tokio_rt().spawn(async move {
        loop {
            if let Err(err) = connect_and_run(&service.player).await {
                debug!(error = %err, "mpd: connection ended");
            }
            service.player.connected.set(false);
            service.player.playback_state.set(PlaybackState::Stopped);
            tokio::time::sleep(RECONNECT_DELAY).await;
        }
    });
}

async fn connect_and_run(player: &Arc<MpdPlayer>) -> anyhow::Result<()> {
    let stream = TcpStream::connect(mpd_addr()).await?;
    let (client, mut events) = Client::connect(stream).await?;
    info!("mpd: connected");
    player.connected.set(true);
    *player.client.write().await = Some(client.clone());

    refresh(&client, player).await;

    // MPD's `idle` events fire on state *transitions* (play/pause/seek/
    // track change), not once a second — so without this the seek bar
    // would sit frozen between transitions instead of advancing while a
    // track plays. Race it against the event stream rather than a
    // separate task so there is one clear owner of `client`/`player`.
    let mut position_tick = tokio::time::interval(Duration::from_secs(1));
    position_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            event = events.next() => {
                match event {
                    Some(ConnectionEvent::SubsystemChange(
                        Subsystem::Player | Subsystem::Mixer | Subsystem::Options,
                    )) => {
                        refresh(&client, player).await;
                    }
                    Some(ConnectionEvent::SubsystemChange(_)) => {}
                    Some(ConnectionEvent::ConnectionClosed(err)) => {
                        return Err(err.into());
                    }
                    None => return Ok(()),
                }
            }
            _ = position_tick.tick() => {
                if player.playback_state.get() == PlaybackState::Playing {
                    tick_position(&client, player).await;
                }
            }
        }
    }
}

/// Lightweight position-only refresh for the once-a-second tick — just
/// `status`, not the full `refresh()` (which also re-fetches the current
/// song and re-downloads album art on every call).
async fn tick_position(client: &Client, player: &Arc<MpdPlayer>) {
    if let Ok(status) = client.command(commands::Status).await {
        player.position.set(status.elapsed.unwrap_or_default());
    }
}

/// Re-query `status` + `currentsong` and publish onto `player`'s
/// `Property`s. Best-effort: any single command failing (a mid-flight
/// disconnect, an empty playlist) just leaves the previous values in
/// place rather than tearing down the connection — the outer idle loop
/// is the source of truth for "still connected".
async fn refresh(client: &Client, player: &Arc<MpdPlayer>) {
    let Ok(status) = client.command(commands::Status).await else {
        return;
    };
    player.playback_state.set(match status.state {
        PlayState::Playing => PlaybackState::Playing,
        PlayState::Paused => PlaybackState::Paused,
        PlayState::Stopped => PlaybackState::Stopped,
    });
    player.position.set(status.elapsed.unwrap_or_default());
    player.duration.set(status.duration.unwrap_or_default());

    let Ok(Some(song_in_queue)) = client.command(commands::CurrentSong).await else {
        player.title.set(String::new());
        player.artist.set(String::new());
        player.album.set(String::new());
        player.cover_art.set(None);
        return;
    };
    let song = song_in_queue.song;
    player
        .title
        .set(song.title().unwrap_or_default().to_string());
    player.artist.set(
        song.artists()
            .first()
            .cloned()
            .unwrap_or_else(|| "Unknown".to_string()),
    );
    player
        .album
        .set(song.album().unwrap_or_default().to_string());

    match fetch_album_art(client, &song.url).await {
        Some(path) => player.cover_art.set(Some(path)),
        None => player.cover_art.set(None),
    }
}

/// Fetch cover art via MPD's `albumart` command and cache it to disk under
/// a name derived from the track URI, mirroring wayle-media's art-cache
/// contract (a local file path the UI hands straight to
/// `gtk::Image::set_from_file`). Re-fetches on every track change rather
/// than checking the cache first — MPD libraries are small enough on a
/// typical desktop that this isn't worth the extra state to avoid.
async fn fetch_album_art(client: &Client, track_uri: &str) -> Option<String> {
    let bytes = client.album_art(track_uri).await.ok().flatten()?.0;
    let dir = art_cache_dir()?;
    std::fs::create_dir_all(&dir).ok()?;
    let ext = infer_image_ext(&bytes);
    let name = format!("{:x}.{ext}", md5_like_hash(track_uri));
    let path = dir.join(name);
    std::fs::write(&path, &bytes).ok()?;
    path.to_str().map(str::to_string)
}

fn art_cache_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    Some(base.join("mshell").join("mpd-art"))
}

/// Cheap non-cryptographic hash so repeat art fetches for the same track
/// overwrite the same cache file instead of accumulating forever — this is
/// a cache key, not a security boundary.
fn md5_like_hash(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish()
}

fn infer_image_ext(bytes: &[u8]) -> &'static str {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => "png",
        [0xff, 0xd8, 0xff, ..] => "jpg",
        [b'G', b'I', b'F', ..] => "gif",
        _ => "jpg",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_signature_is_detected() {
        assert_eq!(
            infer_image_ext(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a]),
            "png"
        );
    }

    #[test]
    fn jpeg_signature_is_detected() {
        assert_eq!(infer_image_ext(&[0xff, 0xd8, 0xff, 0xe0]), "jpg");
    }

    #[test]
    fn gif_signature_is_detected() {
        assert_eq!(infer_image_ext(b"GIF89a"), "gif");
    }

    #[test]
    fn unrecognised_bytes_fall_back_to_jpg() {
        assert_eq!(infer_image_ext(&[0, 1, 2, 3]), "jpg");
        assert_eq!(infer_image_ext(&[]), "jpg");
    }

    #[test]
    fn hash_is_stable_and_distinguishes_different_uris() {
        assert_eq!(md5_like_hash("track-a"), md5_like_hash("track-a"));
        assert_ne!(md5_like_hash("track-a"), md5_like_hash("track-b"));
    }
}
