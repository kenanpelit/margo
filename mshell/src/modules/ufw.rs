//! UFW firewall durum + kontrol modülü — `nufw` noctalia plugin'inden port.
//!
//! Davranış:
//!   - Watchdog `ufw status verbose` çıktısını parse eder: durum (active/
//!     inactive), Logging seviyesi, Default policy (incoming/outgoing/
//!     routed). Detay isteğinde `ufw status numbered` ile kural sayısı +
//!     ilk 5 kuralın özetini de gösterir.
//!   - Aksiyonlar (enable/disable/reload/toggle) `sudo -n` → `pkexec`
//!     fallback'iyle root yetkisi alarak çalıştırılır.
//!   - Salt-okunur sorgular önce plain `ufw`, sonra `sudo -n ufw`
//!     (config'de `allow_privileged_reads = true` ise).
//!
//! Aşağıdaki davranış nufw'dan birebir aktarıldı; sadece QML/shell yerine
//! Rust + iced + tokio::process.

use crate::{
    components::{
        ButtonSize, MenuSize, divider,
        icons::{StaticIcon, icon, icon_button},
    },
    config::UfwModuleConfig,
    t,
    theme::use_theme,
};
use iced::{
    Alignment, Element, Length, Subscription, Task, Theme,
    time::every,
    widget::{Column, Row, button, container, row, text},
};
use log::warn;
use std::time::Duration;
use tokio::process::Command;

const WATCHDOG_MIN_SECS: u64 = 10;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UfwState {
    pub available: bool,
    pub readable: bool,
    /// "active" | "inactive" | "unknown" | "unavailable"
    pub status: String,
    pub logging_level: String,
    pub incoming_policy: String,
    pub outgoing_policy: String,
    pub routed_policy: String,
    pub rule_count: u32,
    pub rules_preview: Vec<String>,
    pub error: String,
}

#[derive(Debug, Clone)]
pub enum UfwAction {
    Toggle,
    Enable,
    Disable,
    Reload,
}

#[derive(Debug, Clone)]
pub enum Message {
    Poll,
    StateUpdated(UfwState),
    Action(UfwAction),
    ActionFinished(Result<(), String>),
}

pub struct Ufw {
    config: UfwModuleConfig,
    state: UfwState,
    is_changing: bool,
    last_error: String,
}

impl Ufw {
    pub fn new(config: UfwModuleConfig) -> Self {
        Self {
            config,
            state: UfwState::default(),
            is_changing: false,
            last_error: String::new(),
        }
    }

    fn is_active(&self) -> bool {
        self.state.status == "active"
    }

    pub fn update(&mut self, msg: Message) -> Task<Message> {
        match msg {
            Message::Poll => {
                if self.is_changing {
                    return Task::none();
                }
                let allow_sudo = self.config.allow_privileged_reads;
                Task::perform(
                    async move { probe_state(allow_sudo).await },
                    Message::StateUpdated,
                )
            }
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
                    warn!("ufw: action failed: {e}");
                    self.last_error = e;
                }
                let allow_sudo = self.config.allow_privileged_reads;
                Task::perform(
                    async move { probe_state(allow_sudo).await },
                    Message::StateUpdated,
                )
            }
        }
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let secs = self.config.watchdog_secs.max(WATCHDOG_MIN_SECS);
        every(Duration::from_secs(secs)).map(|_| Message::Poll)
    }

    pub fn view(&self) -> Element<'_, Message> {
        // Bar'da metin yok — sadece kale/kalkan ikonu, renkle durumu söyler:
        //   yeşil = active, kırmızı = inactive, sarı = ufw kurulu değil.
        let bar_font = use_theme(|t| t.bar_font_size);
        let cell = container(icon(StaticIcon::Lock).size(bar_font));
        if !self.state.available {
            cell.style(|theme: &Theme| container::Style {
                text_color: Some(theme.palette().warning),
                ..Default::default()
            })
            .into()
        } else if self.is_active() {
            cell.style(|theme: &Theme| container::Style {
                text_color: Some(theme.palette().success),
                ..Default::default()
            })
            .into()
        } else {
            cell.style(|theme: &Theme| container::Style {
                text_color: Some(theme.palette().danger),
                ..Default::default()
            })
            .into()
        }
    }

    pub fn menu_view(&self) -> Element<'_, Message> {
        let (font_size, space) = use_theme(|t| (t.font_size, t.space));

        let row_kv = |label: String, value: String| {
            row!(text(label).width(Length::Fill), text(value))
                .align_y(Alignment::Center)
                .spacing(space.xs)
        };

        let mode_button = |label: String,
                           ico: StaticIcon,
                           action: UfwAction,
                           active: bool|
         -> Element<'_, Message> {
            let style = use_theme(|t| t.quick_settings_button_style(active));
            let body = row!(icon(ico), text(label))
                .spacing(space.xxs)
                .align_y(Alignment::Center);
            let btn = button(body).padding([space.xxs, space.xs]).style(style);
            let btn = if self.is_changing {
                btn
            } else {
                btn.on_press(Message::Action(action))
            };
            btn.into()
        };

        let header = row!(
            text(t!("ufw-heading"))
                .size(font_size.lg)
                .width(Length::Fill),
            icon_button(StaticIcon::Refresh)
                .on_press(Message::Poll)
                .size(ButtonSize::Small),
        )
        .align_y(Alignment::Center)
        .spacing(space.xs);

        let active = self.is_active();
        let status_label = if !self.state.available {
            t!("ufw-unavailable")
        } else if active {
            t!("ufw-active")
        } else {
            t!("ufw-inactive")
        };

        let mut rules_block: Vec<Element<'_, Message>> = Vec::new();
        if !self.state.rules_preview.is_empty() {
            rules_block.push(text(t!("ufw-rules-title")).size(font_size.sm).into());
            for rule in &self.state.rules_preview {
                rules_block.push(text(rule.clone()).size(font_size.xs).into());
            }
        }

        let actions_row = Row::with_capacity(4)
            .push(mode_button(
                t!("ufw-action-toggle"),
                StaticIcon::Refresh,
                UfwAction::Toggle,
                active,
            ))
            .push(mode_button(
                t!("ufw-action-enable"),
                StaticIcon::Lock,
                UfwAction::Enable,
                active,
            ))
            .push(mode_button(
                t!("ufw-action-disable"),
                StaticIcon::Close,
                UfwAction::Disable,
                !active && self.state.available,
            ))
            .push(mode_button(
                t!("ufw-action-reload"),
                StaticIcon::Refresh,
                UfwAction::Reload,
                false,
            ))
            .spacing(space.xxs);

        let mut content = Column::with_capacity(12)
            .push(header)
            .push(divider())
            .push(row_kv(t!("ufw-status"), status_label))
            .push(row_kv(
                t!("ufw-incoming"),
                self.state.incoming_policy.clone(),
            ))
            .push(row_kv(
                t!("ufw-outgoing"),
                self.state.outgoing_policy.clone(),
            ))
            .push(row_kv(t!("ufw-routed"), self.state.routed_policy.clone()))
            .push(row_kv(t!("ufw-logging"), self.state.logging_level.clone()))
            .push(row_kv(
                t!("ufw-rule-count"),
                self.state.rule_count.to_string(),
            ))
            .push(divider())
            .push(actions_row);

        if !rules_block.is_empty() {
            content = content
                .push(divider())
                .push(Column::with_children(rules_block).spacing(space.xxs));
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
            .width(MenuSize::Medium)
            .into()
    }
}

// ─── async helpers ───────────────────────────────────────────────────────────

async fn probe_state(allow_sudo: bool) -> UfwState {
    if !exists("ufw").await {
        return UfwState {
            available: false,
            readable: false,
            status: "unavailable".into(),
            logging_level: "n/a".into(),
            incoming_policy: "n/a".into(),
            outgoing_policy: "n/a".into(),
            routed_policy: "n/a".into(),
            rule_count: 0,
            rules_preview: Vec::new(),
            error: "ufw command not found".into(),
        };
    }

    // status verbose: önce plain, sonra sudo -n (allow_sudo ise).
    let status_text = match run_capture("ufw", &["status", "verbose"]).await {
        Ok(out) if !out.is_empty() => Some(out),
        _ if allow_sudo => run_capture("sudo", &["-n", "ufw", "status", "verbose"])
            .await
            .ok(),
        _ => None,
    };
    let numbered_text = match run_capture("ufw", &["status", "numbered"]).await {
        Ok(out) if !out.is_empty() => Some(out),
        _ if allow_sudo => run_capture("sudo", &["-n", "ufw", "status", "numbered"])
            .await
            .ok(),
        _ => None,
    };

    let Some(status_text) = status_text else {
        return UfwState {
            available: true,
            readable: false,
            status: "unknown".into(),
            logging_level: "n/a".into(),
            incoming_policy: "n/a".into(),
            outgoing_policy: "n/a".into(),
            routed_policy: "n/a".into(),
            rule_count: 0,
            rules_preview: Vec::new(),
            error: "Unable to read UFW status".into(),
        };
    };

    let mut status = "unknown".to_string();
    let mut logging = "n/a".to_string();
    let mut incoming = "n/a".to_string();
    let mut outgoing = "n/a".to_string();
    let mut routed = "n/a".to_string();

    for line in status_text.lines() {
        if let Some(rest) = line.strip_prefix("Status:") {
            status = rest.trim().to_lowercase();
        } else if let Some(rest) = line.strip_prefix("Logging:") {
            logging = rest.trim().to_string();
        } else if let Some(rest) = line.strip_prefix("Default:") {
            // "deny (incoming), allow (outgoing), disabled (routed)"
            let rest = rest.trim();
            for tok in rest.split(',') {
                let tok = tok.trim();
                if let Some(p) = parse_policy(tok, "(incoming)") {
                    incoming = p;
                } else if let Some(p) = parse_policy(tok, "(outgoing)") {
                    outgoing = p;
                } else if let Some(p) = parse_policy(tok, "(routed)") {
                    routed = p;
                }
            }
        }
    }

    let (rule_count, rules_preview) = match numbered_text {
        Some(text) => parse_rules(&text),
        None => (0, Vec::new()),
    };

    UfwState {
        available: true,
        readable: true,
        status,
        logging_level: logging,
        incoming_policy: incoming,
        outgoing_policy: outgoing,
        routed_policy: routed,
        rule_count,
        rules_preview,
        error: String::new(),
    }
}

/// "deny (incoming)" → "deny" eğer marker eşleşiyorsa.
fn parse_policy(token: &str, marker: &str) -> Option<String> {
    let (policy, suffix) = token.rsplit_once(' ')?;
    if suffix == marker {
        Some(policy.trim().to_string())
    } else {
        None
    }
}

fn parse_rules(text: &str) -> (u32, Vec<String>) {
    let mut count = 0u32;
    let mut preview: Vec<String> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with('[') {
            continue;
        }
        let Some(close) = trimmed.find(']') else {
            continue;
        };
        count += 1;
        if preview.len() < 5 {
            let body = trimmed[close + 1..].trim().to_string();
            if !body.is_empty() {
                preview.push(body);
            }
        }
    }
    (count, preview)
}

async fn apply_action(action: UfwAction) -> Result<(), String> {
    match action {
        UfwAction::Toggle => {
            let st = probe_state(true).await;
            if st.status == "active" {
                run_root(&["disable"]).await
            } else {
                run_root(&["--force", "enable"]).await
            }
        }
        UfwAction::Enable => run_root(&["--force", "enable"]).await,
        UfwAction::Disable => run_root(&["disable"]).await,
        UfwAction::Reload => run_root(&["reload"]).await,
    }
}

/// `sudo -n ufw …` veya `pkexec sh -c 'ufw "$@"' nufw-root …`
async fn run_root(args: &[&str]) -> Result<(), String> {
    // 1) sudo -n
    if exists("sudo").await {
        let mut full = vec!["-n", "ufw"];
        full.extend(args);
        let status = Command::new("sudo")
            .args(&full)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await
            .map_err(|e| format!("sudo: {e}"))?;
        if status.success() {
            return Ok(());
        }
    }
    // 2) pkexec
    if exists("pkexec").await {
        let cmd = format!(
            "ufw {}",
            args.iter()
                .map(|a| shell_escape(a))
                .collect::<Vec<_>>()
                .join(" ")
        );
        let status = Command::new("pkexec")
            .args(["sh", "-c", &cmd])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await
            .map_err(|e| format!("pkexec: {e}"))?;
        if status.success() {
            return Ok(());
        }
    }
    Err("UFW değiştirmek için sudo -n veya pkexec gerek".into())
}

fn shell_escape(s: &str) -> String {
    if s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        s.to_string()
    } else {
        let escaped = s.replace('\'', "'\\''");
        format!("'{escaped}'")
    }
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
        Err(format!("{bin} exit {}", out.status))
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
