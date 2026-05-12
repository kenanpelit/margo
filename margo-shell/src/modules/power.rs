//! Laptop güç profili + batarya + oturum kontrolü — `npower` port.
//!
//! Sağladığı şeyler:
//!   - Güç kaynağı (AC / pil) ve pil yüzdesi (/sys/class/power_supply).
//!   - `powerprofilesctl` profili (power-saver / balanced / performance)
//!     ile aktif profili oku + değiştir (`set` / `cycle`).
//!   - ppd-auto-profile lock dosyası (~/.local/state/ppd-auto-profile/
//!     lock) ile otomatik profil rotation'ı kilitle / aç.
//!   - Oturum aksiyonları: lock (loginctl), suspend, lock-and-suspend.
//!
//! npower script'lerinden farkları:
//!   - QML/qs tabanlı IPC kanalları yok; kilit + suspend için doğrudan
//!     `loginctl lock-session` ve `systemctl suspend`.

use crate::{
    components::{
        ButtonSize, MenuSize, divider,
        icons::{StaticIcon, icon, icon_button},
    },
    config::PowerModuleConfig,
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

const WATCHDOG_MIN_SECS: u64 = 5;
const LOCK_FILE_REL: &str = ".local/state/ppd-auto-profile/lock";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerSource {
    Ac,
    Battery,
    Unknown,
}

impl Default for PowerSource {
    fn default() -> Self {
        Self::Unknown
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PowerState {
    pub source: PowerSource,
    pub battery_available: bool,
    /// 0..=100; bilinmiyorsa None.
    pub battery_percent: Option<i32>,
    /// "Charging" / "Discharging" / "Full" / "Not charging" / "Unknown"
    pub battery_status: String,
    /// "power-saver" / "balanced" / "performance" / "unknown"
    pub profile: String,
    pub auto_profile_locked: bool,
    pub ppd_timer_active: bool,
}

#[derive(Debug, Clone)]
pub enum PowerAction {
    SetProfile(String),
    CycleProfile,
    ToggleLock,
    Lock,
    Suspend,
}

#[derive(Debug, Clone)]
pub enum Message {
    Poll,
    StateUpdated(PowerState),
    Action(PowerAction),
    ActionFinished(Result<(), String>),
}

pub struct Power {
    config: PowerModuleConfig,
    state: PowerState,
    is_changing: bool,
    last_error: String,
}

impl Power {
    pub fn new(config: PowerModuleConfig) -> Self {
        Self {
            config,
            state: PowerState::default(),
            is_changing: false,
            last_error: String::new(),
        }
    }

    fn profile_icon(profile: &str) -> StaticIcon {
        match profile {
            "power-saver" => StaticIcon::PowerSaver,
            "balanced" => StaticIcon::Balanced,
            "performance" => StaticIcon::Performance,
            _ => StaticIcon::Balanced,
        }
    }

    fn battery_icon(state: &PowerState) -> StaticIcon {
        if !state.battery_available {
            return StaticIcon::Power;
        }
        let charging = state.battery_status == "Charging";
        if charging {
            return StaticIcon::BatteryCharging;
        }
        match state.battery_percent.unwrap_or(0) {
            0..=10 => StaticIcon::Battery0,
            11..=30 => StaticIcon::Battery1,
            31..=60 => StaticIcon::Battery2,
            61..=85 => StaticIcon::Battery3,
            _ => StaticIcon::Battery4,
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
                    warn!("power: action failed: {e}");
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
        let label = if self.state.battery_available {
            match self.state.battery_percent {
                Some(p) => format!("{p}%"),
                None => "—".to_string(),
            }
        } else {
            // Desktop: profil metnini göster
            self.state.profile.clone()
        };
        let ico = if self.state.battery_available {
            Self::battery_icon(&self.state)
        } else {
            Self::profile_icon(&self.state.profile)
        };
        let body = container(row!(icon(ico), text(label)).spacing(space.xxs));
        let percent = self.state.battery_percent.unwrap_or(100);
        if self.state.battery_available && percent <= 15 {
            body.style(|theme: &Theme| container::Style {
                text_color: Some(theme.palette().danger),
                ..Default::default()
            })
            .into()
        } else if self.state.battery_available && percent <= 30 {
            body.style(|theme: &Theme| container::Style {
                text_color: Some(theme.palette().warning),
                ..Default::default()
            })
            .into()
        } else {
            body.into()
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
                           action: PowerAction,
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
            text(t!("power-heading"))
                .size(font_size.lg)
                .width(Length::Fill),
            icon_button(StaticIcon::Refresh)
                .on_press(Message::Poll)
                .size(ButtonSize::Small),
        )
        .align_y(Alignment::Center)
        .spacing(space.xs);

        let source_text = match self.state.source {
            PowerSource::Ac => t!("power-source-ac"),
            PowerSource::Battery => t!("power-source-battery"),
            PowerSource::Unknown => t!("power-source-unknown"),
        };

        let battery_text = if !self.state.battery_available {
            "—".to_string()
        } else {
            let pct = self
                .state
                .battery_percent
                .map(|p| format!("{p}%"))
                .unwrap_or_else(|| "—".to_string());
            format!("{pct} ({})", self.state.battery_status)
        };

        let profiles_row = Row::with_capacity(3)
            .push(mode_button(
                t!("power-profile-power-saver"),
                StaticIcon::PowerSaver,
                PowerAction::SetProfile("power-saver".into()),
                self.state.profile == "power-saver",
            ))
            .push(mode_button(
                t!("power-profile-balanced"),
                StaticIcon::Balanced,
                PowerAction::SetProfile("balanced".into()),
                self.state.profile == "balanced",
            ))
            .push(mode_button(
                t!("power-profile-performance"),
                StaticIcon::Performance,
                PowerAction::SetProfile("performance".into()),
                self.state.profile == "performance",
            ))
            .spacing(space.xxs);

        let session_row = Row::with_capacity(4)
            .push(mode_button(
                t!("power-action-cycle"),
                StaticIcon::Refresh,
                PowerAction::CycleProfile,
                false,
            ))
            .push(mode_button(
                if self.state.auto_profile_locked {
                    t!("power-action-unlock")
                } else {
                    t!("power-action-lock-auto")
                },
                StaticIcon::Lock,
                PowerAction::ToggleLock,
                self.state.auto_profile_locked,
            ))
            .push(mode_button(
                t!("power-action-suspend"),
                StaticIcon::Suspend,
                PowerAction::Suspend,
                false,
            ))
            .push(mode_button(
                t!("power-action-lock-screen"),
                StaticIcon::Lock,
                PowerAction::Lock,
                false,
            ))
            .spacing(space.xxs);

        let mut content = Column::with_capacity(12)
            .push(header)
            .push(divider())
            .push(row_kv(t!("power-source"), source_text))
            .push(row_kv(t!("power-battery"), battery_text))
            .push(row_kv(
                t!("power-profile"),
                self.state.profile.clone(),
            ))
            .push(row_kv(
                t!("power-auto-lock"),
                if self.state.auto_profile_locked {
                    t!("power-auto-lock-locked")
                } else {
                    t!("power-auto-lock-unlocked")
                },
            ))
            .push(divider())
            .push(text(t!("power-profiles-title")).size(font_size.sm))
            .push(profiles_row)
            .push(divider())
            .push(text(t!("power-actions-title")).size(font_size.sm))
            .push(session_row);

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

async fn probe_state() -> PowerState {
    // powerprofilesctl get → mevcut profil
    let profile = run_capture("powerprofilesctl", &["get"])
        .await
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());

    // /sys/class/power_supply/* taraması
    let mut power_source = PowerSource::Unknown;
    let mut battery_available = false;
    let mut battery_percent: Option<i32> = None;
    let mut battery_status = "unknown".to_string();

    if let Ok(mut entries) = tokio::fs::read_dir("/sys/class/power_supply").await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let base = entry.path();
            let type_path = base.join("type");
            let Ok(type_str) = tokio::fs::read_to_string(&type_path).await else {
                continue;
            };
            let type_str = type_str.trim();
            match type_str {
                "Mains" => {
                    if let Ok(online) = tokio::fs::read_to_string(base.join("online")).await {
                        if online.trim() == "1" {
                            power_source = PowerSource::Ac;
                        }
                    }
                }
                "Battery" => {
                    battery_available = true;
                    if let Ok(cap) = tokio::fs::read_to_string(base.join("capacity")).await {
                        if let Ok(n) = cap.trim().parse::<i32>() {
                            battery_percent = Some(n);
                        }
                    }
                    if let Ok(s) = tokio::fs::read_to_string(base.join("status")).await {
                        battery_status = s.trim().to_string();
                    }
                }
                _ => {}
            }
        }
    }
    if power_source == PowerSource::Unknown && battery_available {
        power_source = PowerSource::Battery;
    }

    // systemd user services
    let ppd_timer_active = run_capture("systemctl", &["--user", "is-active", "ppp-auto-profile.timer"])
        .await
        .map(|s| s.trim() == "active")
        .unwrap_or(false);

    let auto_profile_locked = match dirs_home() {
        Some(home) => tokio::fs::metadata(home.join(LOCK_FILE_REL)).await.is_ok(),
        None => false,
    };

    PowerState {
        source: power_source,
        battery_available,
        battery_percent,
        battery_status,
        profile,
        auto_profile_locked,
        ppd_timer_active,
    }
}

async fn apply_action(action: PowerAction) -> Result<(), String> {
    match action {
        PowerAction::SetProfile(mode) => run_check("powerprofilesctl", &["set", &mode]).await,
        PowerAction::CycleProfile => {
            let current = run_capture("powerprofilesctl", &["get"])
                .await
                .map(|s| s.trim().to_string())
                .unwrap_or_default();
            let next = match current.as_str() {
                "power-saver" => "balanced",
                "balanced" => "performance",
                "performance" => "power-saver",
                _ => "balanced",
            };
            run_check("powerprofilesctl", &["set", next]).await
        }
        PowerAction::ToggleLock => {
            let home = dirs_home().ok_or_else(|| "HOME bulunamadı".to_string())?;
            let lock = home.join(LOCK_FILE_REL);
            if tokio::fs::metadata(&lock).await.is_ok() {
                tokio::fs::remove_file(&lock)
                    .await
                    .map_err(|e| format!("rm lock: {e}"))?;
            } else {
                if let Some(parent) = lock.parent() {
                    tokio::fs::create_dir_all(parent)
                        .await
                        .map_err(|e| format!("mkdir state: {e}"))?;
                }
                tokio::fs::write(&lock, b"")
                    .await
                    .map_err(|e| format!("touch lock: {e}"))?;
            }
            Ok(())
        }
        PowerAction::Lock => run_check("loginctl", &["lock-session"]).await,
        PowerAction::Suspend => run_check("systemctl", &["suspend"]).await,
    }
}

fn dirs_home() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(std::path::PathBuf::from)
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
