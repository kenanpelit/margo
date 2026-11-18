//! The pure half of the downloader: the job queue, the yt-dlp command
//! line and the parser for what yt-dlp prints.
//!
//! yt-dlp runs with `--newline` and a progress template that prints
//! bytes (`progress:<done>|<total>|<speed>|<eta>`) rather than its percent
//! text, which lags on fast downloads; `--print` adds a `title:` line
//! before the download and a `file:` line once the file is in place.

use mshell_config::schema::config::DownloadsConfig;
use std::path::{Path, PathBuf};

/// Finished jobs kept in the list (oldest go first).
pub(crate) const KEEP_FINISHED: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Video,
    /// Audio only, in `downloads.audio_format`.
    Audio,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Status {
    Queued,
    Running,
    Done,
    Failed(String),
    Cancelled,
}

impl Status {
    pub(crate) fn finished(&self) -> bool {
        matches!(self, Status::Done | Status::Failed(_) | Status::Cancelled)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Job {
    pub(crate) id: u64,
    pub(crate) url: String,
    pub(crate) mode: Mode,
    pub(crate) status: Status,
    /// 0.0 ..= 1.0 of the current file.
    pub(crate) progress: f64,
    pub(crate) speed: String,
    pub(crate) eta: String,
    pub(crate) title: Option<String>,
    pub(crate) file: Option<PathBuf>,
    /// The folder it downloads into.
    pub(crate) dir: PathBuf,
    /// The last error-looking line yt-dlp printed.
    pub(crate) last_error: Option<String>,
}

impl Job {
    pub(crate) fn label(&self) -> &str {
        self.title.as_deref().unwrap_or(&self.url)
    }
}

/// An http(s) link (the only thing handed to yt-dlp).
pub(crate) fn is_link(s: &str) -> bool {
    let s = s.trim();
    (s.starts_with("https://") || s.starts_with("http://"))
        && s.len() > 10
        && !s.chars().any(char::is_whitespace)
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Queue {
    pub(crate) jobs: Vec<Job>,
    next_id: u64,
}

impl Queue {
    pub(crate) fn add(&mut self, url: &str, mode: Mode, dir: PathBuf) -> u64 {
        self.next_id += 1;
        self.jobs.push(Job {
            id: self.next_id,
            url: url.trim().to_string(),
            mode,
            status: Status::Queued,
            progress: 0.0,
            speed: String::new(),
            eta: String::new(),
            title: None,
            file: None,
            dir,
            last_error: None,
        });
        self.prune();
        self.next_id
    }

    pub(crate) fn get_mut(&mut self, id: u64) -> Option<&mut Job> {
        self.jobs.iter_mut().find(|j| j.id == id)
    }

    /// The job to start next, if none is running.
    pub(crate) fn next_to_run(&self) -> Option<u64> {
        if self.jobs.iter().any(|j| j.status == Status::Running) {
            return None;
        }
        self.jobs
            .iter()
            .find(|j| j.status == Status::Queued)
            .map(|j| j.id)
    }

    pub(crate) fn remove(&mut self, id: u64) {
        self.jobs
            .retain(|j| j.id != id || j.status == Status::Running);
    }

    pub(crate) fn clear_finished(&mut self) {
        self.jobs.retain(|j| !j.status.finished());
    }

    fn prune(&mut self) {
        let finished = self.jobs.iter().filter(|j| j.status.finished()).count();
        let mut drop = finished.saturating_sub(KEEP_FINISHED);
        self.jobs.retain(|j| {
            if drop > 0 && j.status.finished() {
                drop -= 1;
                false
            } else {
                true
            }
        });
    }
}

/// Apply one line of yt-dlp output to `job`.
pub(crate) fn apply_line(job: &mut Job, line: &str) {
    let line = line.trim();
    if let Some(p) = line.strip_prefix("progress:") {
        let f: Vec<&str> = p.split('|').collect();
        if f.len() == 4 {
            let done = f[0].trim().parse::<f64>().ok();
            let total = f[1].trim().parse::<f64>().ok().filter(|t| *t > 0.0);
            if let (Some(d), Some(t)) = (done, total) {
                job.progress = (d / t).clamp(0.0, 1.0);
            }
            job.speed = clean(f[2]);
            job.eta = clean(f[3]);
        }
    } else if let Some(t) = line.strip_prefix("title:") {
        if !t.trim().is_empty() {
            job.title = Some(t.trim().to_string());
        }
    } else if let Some(f) = line.strip_prefix("file:") {
        if !f.trim().is_empty() {
            job.file = Some(PathBuf::from(f.trim()));
        }
    } else if line.starts_with("ERROR:") {
        job.last_error = Some(line.trim_start_matches("ERROR:").trim().to_string());
    }
}

/// yt-dlp's `NA` / `Unknown` placeholders read as nothing.
fn clean(s: &str) -> String {
    let s = s.trim();
    if s.is_empty() || s == "NA" || s.starts_with("Unknown") || s == "None" {
        String::new()
    } else {
        s.to_string()
    }
}

/// The folder a `mode` download goes to: the configured one (`~`
/// expanded), else `xdg` (the XDG videos / music folder), else `~/Videos`
/// / `~/Music`.
pub(crate) fn target_dir(
    cfg: &DownloadsConfig,
    mode: Mode,
    home: &Path,
    xdg: Option<PathBuf>,
) -> PathBuf {
    let set = match mode {
        Mode::Video => cfg.video_dir.trim(),
        Mode::Audio => cfg.audio_dir.trim(),
    };
    if !set.is_empty() {
        return match set.strip_prefix("~/") {
            Some(rest) => home.join(rest),
            None if set == "~" => home.to_path_buf(),
            None => PathBuf::from(set),
        };
    }
    xdg.unwrap_or_else(|| {
        home.join(match mode {
            Mode::Video => "Videos",
            Mode::Audio => "Music",
        })
    })
}

const PROGRESS_TEMPLATE: &str = "download:progress:%(progress.downloaded_bytes)s|%(progress.total_bytes,progress.total_bytes_estimate)s|%(progress._speed_str)s|%(progress._eta_str)s";

/// The yt-dlp arguments for `job` (program name not included).
pub(crate) fn ytdlp_args(job: &Job, cfg: &DownloadsConfig) -> Vec<String> {
    let mut a: Vec<String> = [
        "--newline",
        "--no-colors",
        "--no-warnings",
        "--progress",
        "--progress-template",
        PROGRESS_TEMPLATE,
        "--print",
        "before_dl:title:%(title)s",
        "--print",
        "after_move:file:%(filepath)s",
        "--embed-metadata",
        "--output",
        "%(title)s.%(ext)s",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    a.push("--paths".into());
    a.push(job.dir.to_string_lossy().into_owned());
    if !cfg.playlist {
        a.push("--no-playlist".into());
    }
    match job.mode {
        Mode::Audio => {
            a.push("--extract-audio".into());
            let f = cfg.audio_format.trim();
            if !f.is_empty() && f != "best" {
                a.push("--audio-format".into());
                a.push(f.to_string());
            }
        }
        Mode::Video => {
            let q = cfg.video_quality.trim();
            if !q.is_empty() && q != "best" {
                a.push("--format-sort".into());
                a.push(format!("res:{q}"));
            }
            a.push("--merge-output-format".into());
            a.push("mp4/mkv".into());
        }
    }
    a.extend(cfg.extra_args.split_whitespace().map(str::to_string));
    // `--` keeps a link that starts with `-` from being read as an option.
    a.push("--".into());
    a.push(job.url.clone());
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(mode: Mode) -> Job {
        let mut q = Queue::default();
        q.add("https://youtu.be/abc", mode, PathBuf::from("/tmp/v"));
        q.jobs.remove(0)
    }

    #[test]
    fn progress_title_file_and_errors_are_parsed() {
        let mut j = job(Mode::Video);
        apply_line(&mut j, "title:A Talk");
        apply_line(&mut j, "progress:512|1024|1.20MiB/s|00:03");
        assert_eq!(j.title.as_deref(), Some("A Talk"));
        assert!((j.progress - 0.5).abs() < 1e-9);
        assert_eq!((j.speed.as_str(), j.eta.as_str()), ("1.20MiB/s", "00:03"));
        apply_line(&mut j, "progress:600|NA|Unknown B/s|NA");
        assert!(
            (j.progress - 0.5).abs() < 1e-9,
            "unknown total keeps the last value"
        );
        assert_eq!(j.speed, "");
        apply_line(&mut j, "file:/tmp/v/A Talk.mp4");
        assert_eq!(j.file, Some(PathBuf::from("/tmp/v/A Talk.mp4")));
        apply_line(&mut j, "ERROR: [youtube] abc: Video unavailable");
        assert_eq!(
            j.last_error.as_deref(),
            Some("[youtube] abc: Video unavailable")
        );
        apply_line(&mut j, "[download] Destination: x");
        assert_eq!(j.label(), "A Talk");
    }

    #[test]
    fn the_queue_runs_one_at_a_time_and_keeps_few_finished() {
        let mut q = Queue::default();
        let a = q.add("https://a.example/1", Mode::Video, PathBuf::new());
        let b = q.add("https://a.example/2", Mode::Audio, PathBuf::new());
        assert_eq!(q.next_to_run(), Some(a));
        if let Some(j) = q.get_mut(a) {
            j.status = Status::Running;
        }
        assert_eq!(q.next_to_run(), None);
        q.remove(a);
        assert_eq!(q.jobs.len(), 2, "a running job is not removed");
        if let Some(j) = q.get_mut(a) {
            j.status = Status::Done;
        }
        assert_eq!(q.next_to_run(), Some(b));
        for i in 0..30 {
            let id = q.add(
                &format!("https://a.example/x{i}"),
                Mode::Video,
                PathBuf::new(),
            );
            if let Some(j) = q.get_mut(id) {
                j.status = Status::Failed("x".into());
            }
        }
        q.add("https://a.example/last", Mode::Video, PathBuf::new());
        let finished = q.jobs.iter().filter(|j| j.status.finished()).count();
        assert_eq!(finished, KEEP_FINISHED);
        q.clear_finished();
        assert_eq!(q.jobs.len(), 2);
    }

    #[test]
    fn links_are_validated() {
        assert!(is_link("https://www.youtube.com/watch?v=x"));
        assert!(is_link("  http://example.com/v  "));
        assert!(!is_link("ftp://example.com/v"));
        assert!(!is_link("https://a b"));
        assert!(!is_link("--exec rm"));
    }

    #[test]
    fn args_follow_mode_and_config() {
        let cfg = DownloadsConfig::default();
        let v = ytdlp_args(&job(Mode::Video), &cfg);
        assert!(v.contains(&"--no-playlist".to_string()));
        assert!(v.contains(&"mp4/mkv".to_string()));
        assert!(!v.contains(&"--format-sort".to_string()));
        assert_eq!(&v[v.len() - 2..], ["--", "https://youtu.be/abc"]);
        let cfg = DownloadsConfig {
            audio_format: "opus".into(),
            video_quality: "720".into(),
            playlist: true,
            extra_args: "--cookies-from-browser firefox".into(),
            ..Default::default()
        };
        let a = ytdlp_args(&job(Mode::Audio), &cfg);
        let s = a.join(" ");
        assert!(s.contains("--extract-audio --audio-format opus"));
        assert!(s.contains("--cookies-from-browser firefox -- https://youtu.be/abc"));
        assert!(!s.contains("--no-playlist"));
        let v = ytdlp_args(&job(Mode::Video), &cfg).join(" ");
        assert!(v.contains("--format-sort res:720"));
    }

    #[test]
    fn target_dirs_expand_home_and_fall_back_to_xdg() {
        let home = Path::new("/home/u");
        let mut cfg = DownloadsConfig::default();
        assert_eq!(
            target_dir(&cfg, Mode::Video, home, Some("/home/u/Vids".into())),
            PathBuf::from("/home/u/Vids")
        );
        assert_eq!(
            target_dir(&cfg, Mode::Audio, home, None),
            PathBuf::from("/home/u/Music")
        );
        cfg.audio_dir = "~/dl/music".into();
        assert_eq!(
            target_dir(&cfg, Mode::Audio, home, None),
            PathBuf::from("/home/u/dl/music")
        );
    }
}
