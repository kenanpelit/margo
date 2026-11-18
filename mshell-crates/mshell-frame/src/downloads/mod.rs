//! The media downloader behind the Downloads pill, its menu and the
//! launcher's `dl <url>` / YouTube "Download" actions: a queue of links
//! handed to yt-dlp one at a time.
//!
//! One owner task on the shared tokio runtime holds the [`engine::Queue`];
//! the pill, the menu and the launcher send [`Command`]s and watch the
//! published [`DownloadsState`]. Each running download is a child
//! `yt-dlp` whose stdout/stderr lines are parsed by [`engine::apply_line`];
//! a cancel kills it. Nothing is installed: when `yt-dlp` (or, for audio,
//! `ffmpeg`) is not on `PATH` the job fails with an "install …" message
//! and the menu shows it. The queue is not persisted — a shell restart
//! ends a running download (yt-dlp resumes its `.part` file when the link
//! is queued again).

pub(crate) mod engine;

use engine::{Job, Mode, Queue, Status};
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, DownloadsConfig};
use mshell_process::desktop::{Notice, notify};
use reactive_graph::prelude::GetUntracked;
use std::path::PathBuf;
use std::sync::{LazyLock, OnceLock};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone)]
pub(crate) enum Command {
    /// The link, the format and the config read on the GTK thread.
    Add(String, Mode, Box<DownloadsConfig>),
    Cancel(u64),
    Remove(u64),
    ClearFinished,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct DownloadsState {
    pub(crate) jobs: Vec<Job>,
    /// Programs not on `PATH` (`yt-dlp`, `ffmpeg`), checked on each add.
    pub(crate) missing: Vec<&'static str>,
}

impl DownloadsState {
    pub(crate) fn active(&self) -> usize {
        self.jobs.iter().filter(|j| !j.status.finished()).count()
    }
}

static STATE: LazyLock<watch::Sender<DownloadsState>> =
    LazyLock::new(|| watch::channel(DownloadsState::default()).0);

pub(crate) fn subscribe() -> watch::Receiver<DownloadsState> {
    STATE.subscribe()
}

fn commands() -> &'static mpsc::UnboundedSender<Command> {
    static TX: OnceLock<mpsc::UnboundedSender<Command>> = OnceLock::new();
    TX.get_or_init(|| {
        let (tx, rx) = mpsc::unbounded_channel();
        mshell_services::tokio_rt_spawn(run(rx));
        tx
    })
}

pub(crate) fn send(cmd: Command) {
    let _ = commands().send(cmd);
}

/// Queue `url` (main thread: reads the config). Returns `false` for
/// something that is not an http(s) link.
pub(crate) fn add(url: &str, mode: Mode) -> bool {
    if !engine::is_link(url) {
        return false;
    }
    send(Command::Add(
        url.trim().to_string(),
        mode,
        Box::new(config()),
    ));
    true
}

fn config() -> DownloadsConfig {
    config_manager().config().downloads().get_untracked()
}

/// What is missing for `mode`.
fn missing_for(mode: Mode) -> Vec<&'static str> {
    let mut m = Vec::new();
    if !margo_fs::path::on_path("yt-dlp") {
        m.push("yt-dlp");
    }
    if mode == Mode::Audio && !margo_fs::path::on_path("ffmpeg") {
        m.push("ffmpeg");
    }
    m
}

fn xdg_dir(mode: Mode) -> Option<PathBuf> {
    relm4::gtk::glib::user_special_dir(match mode {
        Mode::Video => relm4::gtk::glib::UserDirectory::Videos,
        Mode::Audio => relm4::gtk::glib::UserDirectory::Music,
    })
}

enum JobEvent {
    Line(u64, String),
    Exit(u64, Result<bool, String>),
}

async fn run(mut rx: mpsc::UnboundedReceiver<Command>) {
    let mut queue = Queue::default();
    let mut missing: Vec<&'static str> = Vec::new();
    let (ev_tx, mut ev_rx) = mpsc::unbounded_channel::<JobEvent>();
    let mut running: Option<(u64, CancellationToken)> = None;
    let mut cfgs: std::collections::HashMap<u64, DownloadsConfig> = Default::default();
    loop {
        tokio::select! {
            cmd = rx.recv() => {
                let Some(cmd) = cmd else { break };
                match cmd {
                    Command::Add(url, mode, cfg) => {
                        let cfg = *cfg;
                        let home = margo_fs::xdg::home().unwrap_or_else(|| PathBuf::from("/"));
                        let dir = engine::target_dir(&cfg, mode, &home, xdg_dir(mode));
                        let id = queue.add(&url, mode, dir);
                        missing = missing_for(mode);
                        if !missing.is_empty()
                            && let Some(j) = queue.get_mut(id)
                        {
                            j.status = Status::Failed(format!("Install {} first", missing.join(" and ")));
                        }
                        cfgs.insert(id, cfg);
                    }
                    Command::Cancel(id) => {
                        if let Some((rid, tok)) = &running
                            && *rid == id
                        {
                            tok.cancel();
                        } else if let Some(j) = queue.get_mut(id)
                            && j.status == Status::Queued
                        {
                            j.status = Status::Cancelled;
                        }
                    }
                    Command::Remove(id) => queue.remove(id),
                    Command::ClearFinished => queue.clear_finished(),
                }
            }
            ev = ev_rx.recv() => {
                let Some(ev) = ev else { break };
                match ev {
                    JobEvent::Line(id, line) => {
                        if let Some(j) = queue.get_mut(id) {
                            engine::apply_line(j, &line);
                        }
                    }
                    JobEvent::Exit(id, result) => {
                        let cancelled = running.as_ref().is_some_and(|(_, t)| t.is_cancelled());
                        running = None;
                        let cfg = cfgs.remove(&id).unwrap_or_default();
                        if let Some(j) = queue.get_mut(id) {
                            j.status = match result {
                                _ if cancelled => Status::Cancelled,
                                Ok(true) => {
                                    j.progress = 1.0;
                                    Status::Done
                                }
                                Ok(false) => Status::Failed(
                                    j.last_error.clone().unwrap_or_else(|| "yt-dlp failed".into()),
                                ),
                                Err(e) => Status::Failed(e),
                            };
                            if cfg.notify {
                                announce(j);
                            }
                        }
                    }
                }
            }
        }
        if running.is_none()
            && let Some(id) = queue.next_to_run()
            && let Some(j) = queue.get_mut(id)
        {
            j.status = Status::Running;
            let cfg = cfgs.get(&id).cloned().unwrap_or_default();
            let tok = CancellationToken::new();
            running = Some((id, tok.clone()));
            tokio::spawn(run_job(j.clone(), cfg, tok, ev_tx.clone()));
        }
        STATE.send_replace(DownloadsState {
            jobs: queue.jobs.clone(),
            missing: missing.clone(),
        });
    }
}

async fn run_job(
    job: Job,
    cfg: DownloadsConfig,
    tok: CancellationToken,
    tx: mpsc::UnboundedSender<JobEvent>,
) {
    let id = job.id;
    if let Err(e) = tokio::fs::create_dir_all(&job.dir).await {
        let _ = tx.send(JobEvent::Exit(
            id,
            Err(format!("{}: {e}", job.dir.display())),
        ));
        return;
    }
    let mut child = match tokio::process::Command::new("yt-dlp")
        .args(engine::ytdlp_args(&job, &cfg))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.send(JobEvent::Exit(id, Err(format!("yt-dlp: {e}"))));
            return;
        }
    };
    // Read both pipes on their own tasks so a full pipe never stalls
    // yt-dlp while we wait for it.
    let pipe = |r: Option<Box<dyn tokio::io::AsyncRead + Unpin + Send>>| {
        let tx = tx.clone();
        tokio::spawn(async move {
            let Some(r) = r else { return };
            let mut lines = BufReader::new(r).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                let _ = tx.send(JobEvent::Line(id, l));
            }
        })
    };
    let out = pipe(child.stdout.take().map(|s| Box::new(s) as _));
    let err = pipe(child.stderr.take().map(|s| Box::new(s) as _));
    let status = tokio::select! {
        s = child.wait() => Some(s),
        () = tok.cancelled() => None,
    };
    let status = match status {
        Some(s) => s,
        None => {
            let _ = child.start_kill();
            child.wait().await
        }
    };
    let _ = out.await;
    let _ = err.await;
    let _ = tx.send(JobEvent::Exit(
        id,
        status.map(|s| s.success()).map_err(|e| e.to_string()),
    ));
}

fn announce(j: &Job) {
    let (summary, body) = match &j.status {
        Status::Done => (
            "Download finished".to_string(),
            j.file
                .as_ref()
                .map_or_else(|| j.label().to_string(), |f| f.display().to_string()),
        ),
        Status::Failed(e) => (format!("Download failed: {}", j.label()), e.clone()),
        _ => return,
    };
    notify(
        Notice::new("Downloads", summary, body)
            .icon("folder-download-symbolic")
            .replace(format!("mshell-download-{}", j.id)),
    );
}
