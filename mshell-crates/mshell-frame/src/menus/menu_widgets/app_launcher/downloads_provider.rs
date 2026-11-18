//! `dl <link>` — queue a video or audio download (yt-dlp) from the
//! launcher. `dl <link>` offers *Download video* and *Download audio*;
//! bare `dl` lists the queue (Enter on a running or waiting one cancels
//! it). The queue is [`crate::downloads`], shared with the Downloads pill
//! and menu and the YouTube provider's Download actions.
//!
//! Lives in `mshell-frame` (not `mshell-launcher`) because it drives the
//! shell's download queue — the same split as the Timers provider.

use crate::downloads::engine::{Mode, Status, is_link};
use crate::downloads::{self, Command};
use mshell_launcher::{LauncherItem, Provider};
use std::rc::Rc;

pub struct DownloadsProvider;

impl DownloadsProvider {
    pub fn new() -> Self {
        Self
    }
}

impl Default for DownloadsProvider {
    fn default() -> Self {
        Self::new()
    }
}

const NAME: &str = "Downloads";
const ICON: &str = "folder-download-symbolic";

/// The text after `dl` / `download`, or `None` when it isn't ours.
fn dl_text(query: &str) -> Option<&str> {
    let q = query.trim_start();
    for prefix in ["download", "dl"] {
        if let Some(rest) = q.strip_prefix(prefix)
            && (rest.is_empty() || rest.starts_with(char::is_whitespace))
        {
            return Some(rest.trim());
        }
    }
    None
}

fn item(
    id: String,
    name: String,
    description: String,
    score: f64,
    f: Rc<dyn Fn()>,
) -> LauncherItem {
    LauncherItem {
        id,
        name,
        description,
        icon: ICON.into(),
        icon_is_path: false,
        score,
        provider_name: NAME.into(),
        usage_key: None,
        on_activate: f,
    }
}

impl Provider for DownloadsProvider {
    fn name(&self) -> &str {
        NAME
    }

    fn handles_search(&self) -> bool {
        false
    }

    fn handles_command(&self, query: &str) -> bool {
        dl_text(query).is_some()
    }

    fn commands(&self) -> Vec<LauncherItem> {
        vec![item(
            "dl:palette".into(),
            "dl <link>".into(),
            "Download a video or its audio with yt-dlp".into(),
            0.0,
            Rc::new(|| {}),
        )]
    }

    fn search(&self, query: &str) -> Vec<LauncherItem> {
        let Some(text) = dl_text(query) else {
            return Vec::new();
        };
        if text.is_empty() {
            return queue_rows();
        }
        if !is_link(text) {
            return vec![item(
                "dl:hint".into(),
                "Paste an http(s) link".into(),
                "dl https://… — anything yt-dlp supports".into(),
                200.0,
                Rc::new(|| {}),
            )];
        }
        let (v, a) = (text.to_string(), text.to_string());
        vec![
            item(
                "dl:video".into(),
                "Download video".into(),
                text.to_string(),
                202.0,
                Rc::new(move || {
                    downloads::add(&v, Mode::Video);
                }),
            ),
            item(
                "dl:audio".into(),
                "Download audio".into(),
                format!("{text} · format: Settings → Widgets → Downloads"),
                201.0,
                Rc::new(move || {
                    downloads::add(&a, Mode::Audio);
                }),
            ),
        ]
    }

    fn keeps_open(&self, item: &LauncherItem, _alt: bool) -> bool {
        item.id == "dl:hint" || item.id == "dl:none"
    }
}

/// Bare `dl`: what's queued (Enter cancels one), or a hint.
fn queue_rows() -> Vec<LauncherItem> {
    let state = downloads::subscribe().borrow().clone();
    let rows: Vec<LauncherItem> = state
        .jobs
        .iter()
        .filter(|j| !j.status.finished())
        .enumerate()
        .map(|(i, j)| {
            let id = j.id;
            let what = match j.status {
                Status::Running => format!("{:.0}%", j.progress * 100.0),
                _ => "Waiting".into(),
            };
            item(
                format!("dl:job:{id}"),
                j.label().to_string(),
                format!("{what} · Enter cancels it"),
                200.0 - i as f64,
                Rc::new(move || downloads::send(Command::Cancel(id))),
            )
        })
        .collect();
    if rows.is_empty() {
        return vec![item(
            "dl:none".into(),
            "Nothing downloading — type dl <link>".into(),
            "Video or audio, with yt-dlp; the Downloads pill shows progress".into(),
            200.0,
            Rc::new(|| {}),
        )];
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_prefix_is_dl_or_download() {
        assert_eq!(
            dl_text("dl https://x.example/v"),
            Some("https://x.example/v")
        );
        assert_eq!(dl_text("download"), Some(""));
        assert_eq!(dl_text("dlna"), None);
        assert_eq!(dl_text("firefox"), None);
    }

    #[test]
    fn a_link_offers_video_and_audio_and_text_a_hint() {
        let p = DownloadsProvider::new();
        let rows = p.search("dl https://youtu.be/abc");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, "dl:video");
        assert_eq!(rows[1].id, "dl:audio");
        assert_eq!(p.search("dl not a link")[0].id, "dl:hint");
    }
}
