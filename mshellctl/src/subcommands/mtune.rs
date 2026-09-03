//! `mshellctl mtune …` — control the native `mtune` music player from the
//! command line. Talks straight to mtune's D-Bus surface
//! (`org.mpris.MediaPlayer2.org.margo.Tune`, custom `org.margo.Tune`
//! interface at `/org/margo/Tune`) — no shell required. Actions launch
//! mtune if it isn't running; reads fail cleanly if it isn't.

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use zbus::Connection;
use zbus::zvariant::OwnedValue;

const BUS: &str = "org.mpris.MediaPlayer2.org.margo.Tune";
const PATH: &str = "/org/margo/Tune";
const IFACE: &str = "org.margo.Tune";

#[derive(Subcommand, Debug)]
pub enum MtuneCommands {
    /// Toggle play / pause.
    #[command(alias = "toggle", alias = "pp")]
    PlayPause,
    /// Resume playback.
    Play,
    /// Pause playback.
    Pause,
    /// Stop playback.
    Stop,
    /// Next track.
    Next,
    /// Previous track (or restart the current one).
    #[command(alias = "prev")]
    Previous,
    /// Seek: `90` = to 0:90, `+30` / `-15` = relative.
    Seek { pos: String },
    /// Set volume `0.0`–`1.0` (`+0.1` / `-0.1` relative); omit to print it.
    Volume { value: Option<String> },
    /// Set playback speed `0.5`–`2.0` (`up` / `down` step ±0.1); omit to
    /// print it.
    Rate { value: Option<String> },
    /// Repeat mode: `off` / `all` / `one` / `cycle`; omit to print it.
    Repeat { mode: Option<String> },
    /// Shuffle: `on` / `off` / `toggle`; omit to print it.
    Shuffle { state: Option<String> },
    /// Jump to a queue position (0-based) and play it.
    Jump { index: u32 },
    /// Open a file, folder, or `.m3u` / `.pls` playlist (replaces the queue).
    Open { path: String },
    /// Point the library at a folder and start playing it.
    Library { path: String },
    /// Rescan the configured library folders.
    Rescan,
    /// List the saved playlists.
    Playlists,
    /// Load a saved playlist by name.
    PlaylistLoad { name: String },
    /// Save the current queue as a playlist.
    PlaylistSave { name: String },
    /// Now-playing summary.
    Status {
        /// Emit JSON instead of a line of text.
        #[arg(long)]
        json: bool,
    },
    /// Full track metadata.
    Metadata {
        #[arg(long)]
        json: bool,
    },
    /// Raise the mtune window.
    Raise,
    /// Quit mtune.
    Quit,
    /// Open the shell's Tune menu (needs mshell).
    Menu,
}

pub async fn execute(command: MtuneCommands) -> Result<()> {
    match command {
        MtuneCommands::Menu => {
            crate::bus::bus_command("Mtune").await?;
            return Ok(());
        }
        MtuneCommands::Status { json } => return status(json).await,
        MtuneCommands::Metadata { json } => return metadata(json).await,
        MtuneCommands::Playlists => {
            let conn = read_conn().await?;
            let names: Vec<String> = get(&conn, "Playlists").await?;
            for n in names {
                println!("{n}");
            }
            return Ok(());
        }
        _ => {}
    }

    let conn = action_conn().await?;
    match command {
        MtuneCommands::PlayPause => call(&conn, "PlayPause", &()).await?,
        MtuneCommands::Play => mpris(&conn, "Play").await?,
        MtuneCommands::Pause => mpris(&conn, "Pause").await?,
        MtuneCommands::Stop => call(&conn, "Stop", &()).await?,
        MtuneCommands::Next => call(&conn, "Next", &()).await?,
        MtuneCommands::Previous => call(&conn, "Previous", &()).await?,
        MtuneCommands::Seek { pos } => {
            let target = resolve_seek(&conn, &pos).await?;
            call(&conn, "Seek", &(target,)).await?;
        }
        MtuneCommands::Volume { value } => match value {
            None => println!("{:.2}", get::<f64>(&conn, "Volume").await?),
            Some(v) => {
                let target = resolve_delta(get::<f64>(&conn, "Volume").await.unwrap_or(1.0), &v)?
                    .clamp(0.0, 1.0);
                call(&conn, "SetVolume", &(target,)).await?;
            }
        },
        MtuneCommands::Rate { value } => match value {
            None => println!("{:.2}", get::<f64>(&conn, "Rate").await?),
            Some(v) => {
                let cur = get::<f64>(&conn, "Rate").await.unwrap_or(1.0);
                let target = match v.as_str() {
                    "up" => cur + 0.1,
                    "down" => cur - 0.1,
                    other => resolve_delta(cur, other)?,
                }
                .clamp(0.5, 2.0);
                call(&conn, "SetRate", &(target,)).await?;
            }
        },
        MtuneCommands::Repeat { mode } => match mode {
            None => println!("{}", get::<String>(&conn, "RepeatMode").await?),
            Some(m) => {
                let cur = get::<String>(&conn, "RepeatMode").await.unwrap_or_default();
                let target = match m.as_str() {
                    "off" | "none" | "consecutive" => "consecutive",
                    "all" | "playlist" | "repeat-all" => "repeat-all",
                    "one" | "track" | "repeat-one" => "repeat-one",
                    "cycle" | "next" => match cur.as_str() {
                        "consecutive" => "repeat-all",
                        "repeat-all" => "repeat-one",
                        _ => "consecutive",
                    },
                    other => bail!("unknown repeat mode '{other}' (off / all / one / cycle)"),
                };
                call(&conn, "SetRepeatMode", &(target,)).await?;
            }
        },
        MtuneCommands::Shuffle { state } => match state {
            None => println!(
                "{}",
                if get::<bool>(&conn, "Shuffle").await? {
                    "on"
                } else {
                    "off"
                }
            ),
            Some(s) => {
                let on = match s.as_str() {
                    "on" | "true" | "1" => true,
                    "off" | "false" | "0" => false,
                    "toggle" => !get::<bool>(&conn, "Shuffle").await.unwrap_or(false),
                    other => bail!("unknown shuffle state '{other}' (on / off / toggle)"),
                };
                call(&conn, "SetShuffle", &(on,)).await?;
            }
        },
        MtuneCommands::Jump { index } => call(&conn, "PlayIndex", &(index,)).await?,
        MtuneCommands::Open { path } => {
            let p = abspath(&path)?;
            let ext = std::path::Path::new(&p)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            if matches!(ext.as_str(), "m3u" | "m3u8" | "pls") {
                call(&conn, "OpenPlaylist", &(p,)).await?;
            } else {
                call(&conn, "PlayFolder", &(p,)).await?;
            }
        }
        MtuneCommands::Library { path } => {
            let p = abspath(&path)?;
            call(&conn, "SetLibraryRoots", &(vec![p.clone()],)).await?;
            call(&conn, "PlayFolder", &(p,)).await?;
        }
        MtuneCommands::Rescan => call(&conn, "RescanLibrary", &()).await?,
        MtuneCommands::PlaylistLoad { name } => call(&conn, "LoadPlaylist", &(name,)).await?,
        MtuneCommands::PlaylistSave { name } => call(&conn, "SavePlaylist", &(name,)).await?,
        MtuneCommands::Raise => call(&conn, "Raise", &()).await?,
        MtuneCommands::Quit => call(&conn, "Quit", &()).await?,
        // Already handled (with an early `return`) before the connection.
        MtuneCommands::Status { .. }
        | MtuneCommands::Metadata { .. }
        | MtuneCommands::Playlists
        | MtuneCommands::Menu => {}
    }
    Ok(())
}

// ── D-Bus plumbing ──────────────────────────────────────────────────

async fn mpris(conn: &Connection, method: &str) -> Result<()> {
    conn.call_method(
        Some(BUS),
        "/org/mpris/MediaPlayer2",
        Some("org.mpris.MediaPlayer2.Player"),
        method,
        &(),
    )
    .await
    .with_context(|| format!("mtune MPRIS: {method} failed"))?;
    Ok(())
}

/// Live playback position in seconds. The custom `Position` property is a
/// coalesced snapshot; MPRIS reports it continuously.
async fn live_position(conn: &Connection) -> u64 {
    let reply = conn
        .call_method(
            Some(BUS),
            "/org/mpris/MediaPlayer2",
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &("org.mpris.MediaPlayer2.Player", "Position"),
        )
        .await;
    match reply {
        Ok(r) => r
            .body()
            .deserialize::<OwnedValue>()
            .ok()
            .and_then(|v| i64::try_from(v).ok())
            .map(|us| (us.max(0) as u64) / 1_000_000)
            .unwrap_or(0),
        Err(_) => 0,
    }
}

async fn read_conn() -> Result<Connection> {
    let conn = Connection::session().await.context("no session bus")?;
    if !name_has_owner(&conn).await? {
        bail!("mtune is not running");
    }
    Ok(conn)
}

async fn action_conn() -> Result<Connection> {
    let conn = Connection::session().await.context("no session bus")?;
    if !name_has_owner(&conn).await? {
        // Launch it and wait briefly for the name.
        let _ = std::process::Command::new("mtune")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .context("failed to launch mtune")?;
        for _ in 0..40 {
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            if name_has_owner(&conn).await.unwrap_or(false) {
                return Ok(conn);
            }
        }
        bail!("mtune did not come up");
    }
    Ok(conn)
}

async fn name_has_owner(conn: &Connection) -> Result<bool> {
    let dbus = zbus::fdo::DBusProxy::new(conn).await?;
    Ok(dbus
        .name_has_owner(BUS.try_into().map_err(|e| anyhow::anyhow!("{e}"))?)
        .await?)
}

async fn call<B>(conn: &Connection, method: &str, body: &B) -> Result<()>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
{
    conn.call_method(Some(BUS), PATH, Some(IFACE), method, body)
        .await
        .with_context(|| format!("mtune: {method} failed"))?;
    Ok(())
}

async fn get<T>(conn: &Connection, prop: &str) -> Result<T>
where
    T: TryFrom<OwnedValue>,
    <T as TryFrom<OwnedValue>>::Error: std::fmt::Display,
{
    let reply = conn
        .call_method(
            Some(BUS),
            PATH,
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &(IFACE, prop),
        )
        .await?;
    let v: OwnedValue = reply.body().deserialize()?;
    T::try_from(v).map_err(|e| anyhow::anyhow!("{prop}: {e}"))
}

// ── formatting helpers ──────────────────────────────────────────────

fn abspath(p: &str) -> Result<String> {
    let path = std::path::Path::new(p);
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    Ok(abs.to_string_lossy().into_owned())
}

fn resolve_delta(current: f64, spec: &str) -> Result<f64> {
    if let Some(rest) = spec.strip_prefix('+') {
        Ok(current + rest.parse::<f64>()?)
    } else if spec.starts_with('-') {
        Ok(current + spec.parse::<f64>()?)
    } else {
        Ok(spec.parse::<f64>()?)
    }
}

async fn resolve_seek(conn: &Connection, spec: &str) -> Result<u64> {
    let cur = live_position(conn).await as i64;
    let target = if let Some(rest) = spec.strip_prefix('+') {
        cur + rest.parse::<i64>()?
    } else if spec.starts_with('-') {
        cur + spec.parse::<i64>()?
    } else {
        spec.parse::<i64>()?
    };
    Ok(target.max(0) as u64)
}

fn fmt_time(secs: u64) -> String {
    format!("{}:{:02}", secs / 60, secs % 60)
}

async fn status(json: bool) -> Result<()> {
    let conn = read_conn().await?;
    let playing = get::<bool>(&conn, "Playing").await.unwrap_or(false);
    let has_song = get::<bool>(&conn, "HasSong").await.unwrap_or(false);
    let title = get::<String>(&conn, "Title").await.unwrap_or_default();
    let artist = get::<String>(&conn, "Artist").await.unwrap_or_default();
    let pos = live_position(&conn).await;
    let dur = get::<u64>(&conn, "Duration").await.unwrap_or(0);
    let rate = get::<f64>(&conn, "Rate").await.unwrap_or(1.0);
    let repeat = get::<String>(&conn, "RepeatMode").await.unwrap_or_default();
    let shuffle = get::<bool>(&conn, "Shuffle").await.unwrap_or(false);
    let qlen = get::<u32>(&conn, "QueueLength").await.unwrap_or(0);
    let idx = get::<i64>(&conn, "CurrentIndex").await.unwrap_or(-1);

    if json {
        let obj = serde_json::json!({
            "playing": playing, "has_song": has_song,
            "title": title, "artist": artist,
            "position": pos, "duration": dur,
            "rate": rate, "repeat": repeat, "shuffle": shuffle,
            "queue_length": qlen, "current_index": idx,
        });
        println!("{}", serde_json::to_string_pretty(&obj)?);
        return Ok(());
    }

    let state = if !has_song {
        "stopped"
    } else if playing {
        "playing"
    } else {
        "paused"
    };
    let track = if artist.is_empty() {
        title.clone()
    } else {
        format!("{artist} — {title}")
    };
    let pos_of = if idx >= 0 {
        format!("{}/{qlen}", idx + 1)
    } else {
        format!("-/{qlen}")
    };
    let mut extra = Vec::new();
    if (rate - 1.0).abs() >= 0.01 {
        extra.push(format!("{rate:.2}×"));
    }
    if repeat != "consecutive" {
        extra.push(repeat.clone());
    }
    if shuffle {
        extra.push("shuffle".into());
    }
    let extra = if extra.is_empty() {
        String::new()
    } else {
        format!("  [{}]", extra.join(" "))
    };
    println!(
        "{state}  {track}  ({}/{}) {pos_of}{extra}",
        fmt_time(pos),
        fmt_time(dur)
    );
    Ok(())
}

async fn metadata(json: bool) -> Result<()> {
    let conn = read_conn().await?;
    let title = get::<String>(&conn, "Title").await.unwrap_or_default();
    let artist = get::<String>(&conn, "Artist").await.unwrap_or_default();
    let album = get::<String>(&conn, "Album").await.unwrap_or_default();
    let cover = get::<String>(&conn, "CoverArt").await.unwrap_or_default();
    let dur = get::<u64>(&conn, "Duration").await.unwrap_or(0);

    if json {
        let obj = serde_json::json!({
            "title": title, "artist": artist, "album": album,
            "cover_art": cover, "duration": dur,
        });
        println!("{}", serde_json::to_string_pretty(&obj)?);
    } else {
        println!("title    {title}");
        println!("artist   {artist}");
        println!("album    {album}");
        println!("duration {}", fmt_time(dur));
        if !cover.is_empty() {
            println!("cover    {cover}");
        }
    }
    Ok(())
}
