//! Podman container yönetimi — `npodman` plugin'inden minimal port.
//!
//! Bar göstergesi: container ikonu + "çalışan/toplam" sayısı.
//! Menü: container listesi (state + adı + image), her satırda
//! start / stop / restart / remove butonları.
//!
//! Image/pod yönetimi bu sürümde yok; npodman'da var ama çok dar
//! kullanım. Eklemek istenirse `podman images` / `podman pod ps` ile
//! aynı pattern.

use crate::{
    components::{
        ButtonSize, MenuSize, divider,
        icons::{StaticIcon, icon, icon_button},
    },
    config::PodmanModuleConfig,
    t,
    theme::use_theme,
};
use iced::{
    Alignment, Element, Length, Subscription, Task, Theme,
    time::every,
    widget::{Column, container, row, text},
};
use log::warn;
use std::time::Duration;
use tokio::process::Command;

const WATCHDOG_MIN_SECS: u64 = 10;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Container {
    pub id: String,
    pub name: String,
    pub image: String,
    /// "running" / "exited" / "paused" / "created" / vs.
    pub state: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PodmanState {
    /// podman binary mevcut mu?
    pub available: bool,
    pub containers: Vec<Container>,
    pub error: String,
}

impl PodmanState {
    pub fn running_count(&self) -> usize {
        self.containers
            .iter()
            .filter(|c| c.state == "running")
            .count()
    }
}

#[derive(Debug, Clone)]
pub enum PodmanAction {
    Start(String),
    Stop(String),
    Restart(String),
    Remove(String),
}

#[derive(Debug, Clone)]
pub enum Message {
    Poll,
    StateUpdated(PodmanState),
    Action(PodmanAction),
    ActionFinished(Result<(), String>),
}

pub struct Podman {
    config: PodmanModuleConfig,
    state: PodmanState,
    is_changing: bool,
    last_error: String,
}

impl Podman {
    pub fn new(config: PodmanModuleConfig) -> Self {
        Self {
            config,
            state: PodmanState::default(),
            is_changing: false,
            last_error: String::new(),
        }
    }

    pub fn update(&mut self, msg: Message) -> Task<Message> {
        match msg {
            Message::Poll => Task::perform(probe_state(), Message::StateUpdated),
            Message::StateUpdated(state) => {
                if state != self.state {
                    self.state = state;
                }
                if !self.last_error.is_empty() && !self.is_changing {
                    self.last_error.clear();
                }
                Task::none()
            }
            Message::Action(action) => {
                if self.is_changing {
                    return Task::none();
                }
                self.is_changing = true;
                self.last_error.clear();
                Task::perform(
                    async move { apply_action(action).await },
                    Message::ActionFinished,
                )
            }
            Message::ActionFinished(result) => {
                self.is_changing = false;
                if let Err(e) = result {
                    warn!("podman: action failed: {e}");
                    self.last_error = e;
                }
                Task::perform(probe_state(), Message::StateUpdated)
            }
        }
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let secs = self.config.watchdog_secs.max(WATCHDOG_MIN_SECS);
        every(Duration::from_secs(secs)).map(|_| Message::Poll)
    }

    pub fn view(&self) -> Element<'_, Message> {
        let space = use_theme(|t| t.space);
        let running = self.state.running_count();
        let total = self.state.containers.len();
        let label = if !self.state.available {
            "—".to_string()
        } else {
            format!("{running}/{total}")
        };
        let body = container(
            row!(icon(StaticIcon::Drive), text(label)).spacing(space.xxs),
        );
        if running > 0 {
            body.style(|theme: &Theme| container::Style {
                text_color: Some(theme.palette().success),
                ..Default::default()
            })
            .into()
        } else {
            body.into()
        }
    }

    pub fn menu_view(&self) -> Element<'_, Message> {
        let (font_size, space) = use_theme(|t| (t.font_size, t.space));

        let header = row!(
            text(t!("podman-heading"))
                .size(font_size.lg)
                .width(Length::Fill),
            icon_button(StaticIcon::Refresh)
                .on_press(Message::Poll)
                .size(ButtonSize::Small),
        )
        .align_y(Alignment::Center)
        .spacing(space.xs);

        let summary = if !self.state.available {
            t!("podman-unavailable")
        } else if self.state.containers.is_empty() {
            t!("podman-empty")
        } else {
            format!(
                "{} {} / {}",
                t!("podman-running"),
                self.state.running_count(),
                self.state.containers.len()
            )
        };

        let mut container_rows: Vec<Element<'_, Message>> = Vec::new();
        for c in &self.state.containers {
            let id = c.id.clone();
            let running = c.state == "running";

            let name = text(c.name.clone()).width(Length::Fill);
            let state_text = text(c.state.clone()).size(font_size.xs);
            let image_text = text(c.image.clone()).size(font_size.xs);

            // Aksiyon butonları — running'e göre dinamik etkinleştir.
            let mk_btn = |ico: StaticIcon, action: PodmanAction, enabled: bool| {
                let btn = icon_button(ico).size(ButtonSize::Small);
                if enabled && !self.is_changing {
                    btn.on_press(Message::Action(action))
                } else {
                    btn
                }
            };

            let actions = row!(
                mk_btn(StaticIcon::Play, PodmanAction::Start(id.clone()), !running),
                mk_btn(StaticIcon::Pause, PodmanAction::Stop(id.clone()), running),
                mk_btn(StaticIcon::Refresh, PodmanAction::Restart(id.clone()), true),
                mk_btn(StaticIcon::Delete, PodmanAction::Remove(id.clone()), !running),
            )
            .spacing(space.xxs);

            let entry = Column::with_capacity(2)
                .push(row!(name, state_text).spacing(space.xs).align_y(Alignment::Center))
                .push(row!(image_text, actions).spacing(space.xs).align_y(Alignment::Center))
                .spacing(space.xxs);
            container_rows.push(entry.into());
            container_rows.push(divider().into());
        }
        // Trailing divider'ı kaldır.
        if container_rows.last().is_some() {
            container_rows.pop();
        }

        let mut content = Column::with_capacity(5 + container_rows.len())
            .push(header)
            .push(divider())
            .push(text(summary));

        if !container_rows.is_empty() {
            content = content
                .push(divider())
                .push(Column::with_children(container_rows).spacing(space.xxs));
        }

        if !self.last_error.is_empty() {
            let err = self.last_error.clone();
            content = content.push(divider()).push(
                container(text(err))
                    .style(|theme: &Theme| container::Style {
                        text_color: Some(theme.palette().danger),
                        ..Default::default()
                    })
                    .padding(space.xxs),
            );
        }

        container(content.spacing(space.xs).padding([0.0, space.xs]))
            .width(MenuSize::Large)
            .into()
    }
}

// ─── async helpers ───────────────────────────────────────────────────────────

async fn probe_state() -> PodmanState {
    if !exists("podman").await {
        return PodmanState {
            available: false,
            containers: Vec::new(),
            error: "podman not found".into(),
        };
    }

    // ps --all --format json yerine `{{.ID}}\t{{.Names}}\t{{.Image}}\t{{.State}}`
    // pipe-friendly: image isminde tab geçemez.
    let out = match run_capture(
        "podman",
        &[
            "ps",
            "--all",
            "--format",
            "{{.ID}}\t{{.Names}}\t{{.Image}}\t{{.State}}",
        ],
    )
    .await
    {
        Ok(s) => s,
        Err(e) => {
            return PodmanState {
                available: true,
                containers: Vec::new(),
                error: e,
            };
        }
    };

    let containers: Vec<Container> = out
        .lines()
        .filter(|l| !l.is_empty())
        .map(|line| {
            let mut parts = line.splitn(4, '\t');
            Container {
                id: parts.next().unwrap_or("").to_string(),
                name: parts.next().unwrap_or("").to_string(),
                image: parts.next().unwrap_or("").to_string(),
                state: parts
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_lowercase(),
            }
        })
        .filter(|c| !c.id.is_empty())
        .collect();

    PodmanState {
        available: true,
        containers,
        error: String::new(),
    }
}

async fn apply_action(action: PodmanAction) -> Result<(), String> {
    let (verb, id) = match action {
        PodmanAction::Start(id) => ("start", id),
        PodmanAction::Stop(id) => ("stop", id),
        PodmanAction::Restart(id) => ("restart", id),
        PodmanAction::Remove(id) => ("rm", id),
    };
    let args: Vec<&str> = if verb == "rm" {
        vec!["rm", "-f", &id]
    } else {
        vec![verb, &id]
    };
    run_check("podman", &args).await
}

async fn run_capture(bin: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(bin)
        .args(args)
        .output()
        .await
        .map_err(|e| format!("{bin}: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

async fn run_check(bin: &str, args: &[&str]) -> Result<(), String> {
    let out = Command::new(bin)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("{bin}: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        let trimmed = err.trim();
        if trimmed.is_empty() {
            Err(format!("{bin} {:?} → exit {}", args, out.status))
        } else {
            Err(format!("{bin}: {trimmed}"))
        }
    }
}

async fn exists(bin: &str) -> bool {
    Command::new("sh")
        .args(["-c", &format!("command -v {} >/dev/null 2>&1", bin)])
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}
