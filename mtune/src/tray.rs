// SPDX-License-Identifier: GPL-3.0-or-later
//! StatusNotifierItem tray icon for Tune. Registers via `ksni` on the
//! session bus; margo's shell `system_tray` widget (an SNI host) picks it up
//! automatically, as does any other SNI-capable tray.
//!
//! The tray is `Send` (ksni requirement) so it holds only plain data — a
//! [`Snapshot`] copy plus the [`CommandSender`]; it never touches a GTK
//! object. `Application` refreshes it via the returned [`Handle`].

use crate::audio::RepeatMode;
use crate::bridge::{AppCommand, CommandSender, Snapshot};
use crate::config::APPLICATION_ID;
use ksni::menu::{CheckmarkItem, RadioGroup, RadioItem, StandardItem};
use ksni::{Handle, MenuItem, ToolTip, TrayMethods};

#[derive(Debug)]
pub struct TuneTray {
    snap: Snapshot,
    tx: CommandSender,
}

impl TuneTray {
    fn send(&self, cmd: AppCommand) {
        let _ = self.tx.send_blocking(cmd);
    }

    /// Replace the mirrored state (called from `Handle::update`).
    pub fn set_snapshot(&mut self, snap: Snapshot) {
        self.snap = snap;
    }
}

impl ksni::Tray for TuneTray {
    fn id(&self) -> String {
        APPLICATION_ID.into()
    }

    fn title(&self) -> String {
        "Tune".into()
    }

    fn icon_name(&self) -> String {
        APPLICATION_ID.into()
    }

    fn tool_tip(&self) -> ToolTip {
        let (title, description) = if self.snap.has_song {
            let state = if self.snap.playing {
                "Playing"
            } else {
                "Paused"
            };
            (
                format!("{state} — {}", self.snap.title),
                self.snap.artist.clone(),
            )
        } else {
            ("Tune".into(), "Idle".into())
        };
        ToolTip {
            icon_name: APPLICATION_ID.into(),
            title,
            description,
            ..Default::default()
        }
    }

    /// Left click → show / hide the window.
    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(AppCommand::ToggleWindow);
    }

    /// Scroll → volume.
    fn scroll(&mut self, delta: i32, _orientation: ksni::Orientation) {
        let step = if delta > 0 { 0.05 } else { -0.05 };
        let vol = (self.snap.volume + step).clamp(0.0, 1.0);
        self.send(AppCommand::SetVolume(vol));
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let play_label = if self.snap.playing { "Pause" } else { "Play" };
        vec![
            StandardItem {
                label: play_label.into(),
                icon_name: if self.snap.playing {
                    "media-playback-pause-symbolic".into()
                } else {
                    "media-playback-start-symbolic".into()
                },
                enabled: self.snap.has_song,
                activate: Box::new(|t: &mut Self| t.send(AppCommand::PlayPause)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Next".into(),
                icon_name: "media-skip-forward-symbolic".into(),
                enabled: self.snap.queue_len > 1,
                activate: Box::new(|t: &mut Self| t.send(AppCommand::Next)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Previous".into(),
                icon_name: "media-skip-backward-symbolic".into(),
                enabled: self.snap.queue_len > 1,
                activate: Box::new(|t: &mut Self| t.send(AppCommand::Previous)),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            CheckmarkItem {
                label: "Shuffle".into(),
                checked: self.snap.shuffle,
                activate: Box::new(|t: &mut Self| t.send(AppCommand::SetShuffle(!t.snap.shuffle))),
                ..Default::default()
            }
            .into(),
            RadioGroup {
                selected: match self.snap.repeat {
                    RepeatMode::Consecutive => 0,
                    RepeatMode::RepeatAll => 1,
                    RepeatMode::RepeatOne => 2,
                    RepeatMode::RepeatEach => 3,
                },
                select: Box::new(|t: &mut Self, i| {
                    let mode = match i {
                        1 => RepeatMode::RepeatAll,
                        2 => RepeatMode::RepeatOne,
                        3 => RepeatMode::RepeatEach,
                        _ => RepeatMode::Consecutive,
                    };
                    t.send(AppCommand::SetRepeat(mode));
                }),
                options: vec![
                    RadioItem {
                        label: "Repeat off".into(),
                        ..Default::default()
                    },
                    RadioItem {
                        label: "Repeat all".into(),
                        ..Default::default()
                    },
                    RadioItem {
                        label: "Repeat one".into(),
                        ..Default::default()
                    },
                    RadioItem {
                        label: "Repeat each".into(),
                        ..Default::default()
                    },
                ],
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Show Tune".into(),
                activate: Box::new(|t: &mut Self| t.send(AppCommand::Raise)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Quit".into(),
                icon_name: "application-exit-symbolic".into(),
                activate: Box::new(|t: &mut Self| t.send(AppCommand::Quit)),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// Spawn the tray service. `None` if the SNI registration fails (no tray host
/// / no session bus) — mtune runs fine without it.
pub async fn spawn(snap: Snapshot, tx: CommandSender) -> Option<Handle<TuneTray>> {
    match (TuneTray { snap, tx }).spawn().await {
        Ok(h) => Some(h),
        Err(e) => {
            log::warn!("mtune: tray unavailable: {e}");
            None
        }
    }
}
