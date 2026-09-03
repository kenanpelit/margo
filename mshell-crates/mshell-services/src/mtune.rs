//! Bridge to the `mtune` music player's supplementary `org.margo.Tune`
//! D-Bus interface — the library / queue surface standard MPRIS can't
//! express. Powers the shell's dedicated Tune bar pill + menu.
//!
//! `mtune` may simply not be installed or running; that is a normal
//! desktop, not an error. The watcher connects when the name appears and
//! goes back to `running = false` when it disappears, so the pill just
//! shows a "launch Tune" affordance in the meantime.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use tracing::debug;
use wayle_core::Property;
use zbus::Connection;
use zbus::fdo::DBusProxy;
use zbus::names::BusName;

use crate::tokio_rt;

const BUS_NAME: &str = "org.margo.Tune";
const OBJECT_PATH: &str = "/org/margo/Tune";
const IFACE: &str = "org.margo.Tune";

/// Live `org.margo.Tune` state, read/watched by the Tune bar pill + menu
/// with the same `Property<T>` shape as every wayle-* field.
pub struct MtunePlayer {
    /// `mtune` is running and owns `org.margo.Tune`.
    pub running: Property<bool>,
    pub playing: Property<bool>,
    pub has_song: Property<bool>,
    pub title: Property<String>,
    pub artist: Property<String>,
    pub album: Property<String>,
    /// Absolute path to the current track's cached cover, or `None`.
    pub cover_art: Property<Option<String>>,
    pub position: Property<Duration>,
    pub duration: Property<Duration>,
    pub shuffle: Property<bool>,
    /// `"consecutive"` / `"repeat-all"` / `"repeat-one"`.
    pub repeat_mode: Property<String>,
    pub queue_len: Property<u32>,
    pub current_index: Property<i64>,
    pub library_roots: Property<Vec<String>>,
    pub scanning: Property<bool>,
    /// `(done, total)` during a scan, `(0, 0)` otherwise.
    pub scan_progress: Property<(u32, u32)>,
}

impl MtunePlayer {
    fn new() -> Self {
        Self {
            running: Property::new(false),
            playing: Property::new(false),
            has_song: Property::new(false),
            title: Property::new(String::new()),
            artist: Property::new(String::new()),
            album: Property::new(String::new()),
            cover_art: Property::new(None),
            position: Property::new(Duration::ZERO),
            duration: Property::new(Duration::ZERO),
            shuffle: Property::new(false),
            repeat_mode: Property::new("consecutive".into()),
            queue_len: Property::new(0),
            current_index: Property::new(-1),
            library_roots: Property::new(Vec::new()),
            scanning: Property::new(false),
            scan_progress: Property::new((0, 0)),
        }
    }

    async fn proxy(&self) -> Option<zbus::Proxy<'static>> {
        let conn = Connection::session().await.ok()?;
        zbus::Proxy::new(&conn, BUS_NAME, OBJECT_PATH, IFACE)
            .await
            .ok()
    }

    async fn call(
        &self,
        method: &str,
        body: &(impl serde::Serialize + zbus::zvariant::DynamicType),
    ) {
        match self.proxy().await {
            Some(p) => {
                if let Err(e) = p.call_method(method, body).await {
                    debug!(error = %e, method, "mtune: D-Bus call failed");
                }
            }
            None => {
                // Not running — launch it; the action is lost for this
                // click but the player comes up and the watcher reconnects.
                spawn_mtune();
            }
        }
    }

    pub async fn play_pause(&self) {
        self.call("PlayPause", &()).await;
    }
    pub async fn next(&self) {
        self.call("Next", &()).await;
    }
    pub async fn previous(&self) {
        self.call("Previous", &()).await;
    }
    pub async fn set_shuffle(&self, on: bool) {
        self.call("SetShuffle", &(on,)).await;
    }
    pub async fn set_repeat_mode(&self, mode: &str) {
        self.call("SetRepeatMode", &(mode,)).await;
    }
    pub async fn play_index(&self, index: u32) {
        self.call("PlayIndex", &(index,)).await;
    }
    pub async fn play_folder(&self, path: &str) {
        self.call("PlayFolder", &(path,)).await;
    }
    pub async fn set_library_roots(&self, roots: Vec<String>) {
        self.call("SetLibraryRoots", &(roots,)).await;
    }
    pub async fn rescan_library(&self) {
        self.call("RescanLibrary", &()).await;
    }
    pub async fn raise(&self) {
        self.call("Raise", &()).await;
    }
}

/// Spawn `mtune` detached (fire-and-forget). Used when a control is hit
/// while the player isn't running.
pub fn spawn_mtune() {
    let _ = std::process::Command::new("mtune")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

pub struct MtuneService {
    pub player: Arc<MtunePlayer>,
}

static MTUNE_SERVICE: OnceLock<Arc<MtuneService>> = OnceLock::new();

/// The Tune service singleton — lazily constructed, always succeeds,
/// starts `running = false`. [`spawn_mtune_watcher`] connects it.
pub fn mtune_service() -> Arc<MtuneService> {
    MTUNE_SERVICE
        .get_or_init(|| {
            Arc::new(MtuneService {
                player: Arc::new(MtunePlayer::new()),
            })
        })
        .clone()
}

/// Watch `org.margo.Tune`: mirror its properties into [`MtunePlayer`],
/// refreshing on every `Changed` signal and on name owner changes.
pub fn spawn_mtune_watcher() {
    let service = mtune_service();
    tokio_rt().spawn(async move {
        loop {
            if let Err(err) = run(&service.player).await {
                debug!(error = %err, "mtune: watch ended");
            }
            reset(&service.player);
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    });
}

fn reset(p: &MtunePlayer) {
    p.running.set(false);
    p.playing.set(false);
    p.has_song.set(false);
    p.scanning.set(false);
}

async fn run(p: &MtunePlayer) -> zbus::Result<()> {
    use futures::StreamExt;

    let conn = Connection::session().await?;
    let proxy = zbus::Proxy::new(&conn, BUS_NAME, OBJECT_PATH, IFACE).await?;

    // Wait for the name to have an owner (mtune running).
    let dbus = DBusProxy::new(&conn).await?;
    let name = BusName::try_from(BUS_NAME).map_err(|e| zbus::Error::Failure(e.to_string()))?;
    if dbus.name_has_owner(name.clone()).await? {
        p.running.set(true);
        refresh(&proxy, p).await;
    }

    let mut owner_changes = dbus.receive_name_owner_changed().await?;
    let mut changed = proxy.receive_signal("Changed").await?;

    loop {
        tokio::select! {
            Some(sig) = owner_changes.next() => {
                if let Ok(args) = sig.args()
                    && args.name.as_str() == BUS_NAME
                {
                    let up = args.new_owner.is_some();
                    p.running.set(up);
                    if up {
                        refresh(&proxy, p).await;
                    } else {
                        reset(p);
                    }
                }
            }
            Some(_) = changed.next() => {
                refresh(&proxy, p).await;
            }
            else => return Ok(()),
        }
    }
}

async fn refresh(proxy: &zbus::Proxy<'_>, p: &MtunePlayer) {
    macro_rules! get {
        ($name:literal, $ty:ty) => {
            proxy.get_property::<$ty>($name).await.ok()
        };
    }

    if let Some(v) = get!("Playing", bool) {
        p.playing.set(v);
    }
    if let Some(v) = get!("HasSong", bool) {
        p.has_song.set(v);
    }
    if let Some(v) = get!("Title", String) {
        p.title.set(v);
    }
    if let Some(v) = get!("Artist", String) {
        p.artist.set(v);
    }
    if let Some(v) = get!("Album", String) {
        p.album.set(v);
    }
    if let Some(v) = get!("CoverArt", String) {
        p.cover_art.set((!v.is_empty()).then_some(v));
    }
    if let Some(v) = get!("Position", u64) {
        p.position.set(Duration::from_secs(v));
    }
    if let Some(v) = get!("Duration", u64) {
        p.duration.set(Duration::from_secs(v));
    }
    if let Some(v) = get!("Shuffle", bool) {
        p.shuffle.set(v);
    }
    if let Some(v) = get!("RepeatMode", String) {
        p.repeat_mode.set(v);
    }
    if let Some(v) = get!("QueueLength", u32) {
        p.queue_len.set(v);
    }
    if let Some(v) = get!("CurrentIndex", i64) {
        p.current_index.set(v);
    }
    if let Some(v) = get!("LibraryRoots", Vec<String>) {
        p.library_roots.set(v);
    }
    if let Some(v) = get!("Scanning", bool) {
        p.scanning.set(v);
    }
    if let Some(v) = get!("ScanProgress", (u32, u32)) {
        p.scan_progress.set(v);
    }
}
