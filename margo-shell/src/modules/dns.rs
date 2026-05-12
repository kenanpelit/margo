//! DNS / VPN switcher module — `ndns` noctalia plugin'inin tam Rust port'u.
//!
//! Beş mod arasında geçiş yapar:
//!   1. **Mullvad**  — VPN bağla, Blocky kapat
//!   2. **Blocky**   — Mullvad'ı kes, Blocky DNS filtresini aç
//!   3. **Default**  — İkisini de durdur, NetworkManager auto-DNS'e dön
//!   4. **Toggle**   — Mullvad ↔ Blocky flip (`osc-mullvad toggle --with-blocky`)
//!   5. **Repair**   — State divergence tamiri (`osc-mullvad ensure --grace 0`)
//!
//! Ek olarak 5 DNS preset (Google / Cloudflare / OpenDNS / AdGuard / Quad9)
//! `nmcli con mod ipv4.dns` üzerinden uygulanır.
//!
//! Mimari:
//!   - State polling (subscription → `Message::Poll`) `mullvad status`,
//!     `systemctl is-active blocky.service`, `nmcli`, `resolvectl`,
//!     `/etc/resolv.conf` paralel sorgular.
//!   - Action dispatch (`Message::Action`) tokio::process üzerinden
//!     `osc-mullvad`, `nmcli`, `systemctl stop blocky.service` çağırır.
//!     Blocky için `sudo -n` → `pkexec` fallback'i var.
//!
//! Tüm subprocess çağrıları async; UI thread asla blokelenmez.

use crate::{
    components::{
        ButtonSize, MenuSize, divider,
        icons::{StaticIcon, icon, icon_button},
    },
    config::{DnsModuleConfig, DnsProviderEntry},
    t,
    theme::use_theme,
};
use iced::{
    Alignment, Element, Length, Subscription, Task, Theme,
    time::every,
    widget::{Column, Row, button, column, container, row, text},
};
use log::{debug, warn};
use std::time::Duration;
use tokio::process::Command;

const WATCHDOG_MIN_SECS: u64 = 5;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DnsState {
    pub vpn_connected: bool,
    pub blocky_active: bool,
    pub blocked: bool,
    pub auto_dns: bool,
    /// nmcli'nin yazdığı DNS listesi (config).
    pub dns: Vec<String>,
    /// Sistemin gerçekte kullandığı DNS (resolvectl / resolv.conf).
    pub display_dns: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DnsMode {
    Mullvad,
    Blocky,
    Mixed,
    Default,
    /// `config.providers` index'i.
    Provider(usize),
    Unknown,
}

#[derive(Debug, Clone)]
pub enum DnsAction {
    Toggle,
    Mullvad,
    Blocky,
    Repair,
    Default,
    Provider(usize),
}

#[derive(Debug, Clone)]
pub enum Message {
    Poll,
    StateUpdated(Result<DnsState, String>),
    Action(DnsAction),
    ActionFinished(Result<(), String>),
}

pub struct Dns {
    config: DnsModuleConfig,
    state: DnsState,
    /// Aksiyon devam ediyor mu? UI'da spinner / disable için.
    is_changing: bool,
    /// Son hata (UI'da kırmızı satır).
    last_error: String,
}

impl Dns {
    pub fn new(config: DnsModuleConfig) -> Self {
        Self {
            config,
            state: DnsState::default(),
            is_changing: false,
            last_error: String::new(),
        }
    }

    pub fn config(&self) -> &DnsModuleConfig {
        &self.config
    }

    /// Mevcut moda göre Bar göstergesinde ve menüde kullanılacak ikon.
    fn mode_icon(mode: DnsMode) -> StaticIcon {
        match mode {
            DnsMode::Mullvad => StaticIcon::Vpn,
            DnsMode::Blocky => StaticIcon::Lock,
            DnsMode::Mixed => StaticIcon::Vpn,
            DnsMode::Default => StaticIcon::Ethernet,
            DnsMode::Provider(_) => StaticIcon::IpAddress,
            DnsMode::Unknown => StaticIcon::Refresh,
        }
    }

    /// state'e bakıp mevcut modu çıkar.
    fn current_mode(&self) -> DnsMode {
        // ndns'in priority sırası: Mullvad/Blocky state > raw DNS.
        match (self.state.vpn_connected, self.state.blocky_active) {
            (true, true) => DnsMode::Mixed,
            (true, false) => DnsMode::Mullvad,
            (false, true) => DnsMode::Blocky,
            (false, false) => {
                // Ne VPN ne Blocky → DNS değerine bak.
                let normalized = normalize_dns(&self.state.dns);
                for (i, p) in self.config.providers.iter().enumerate() {
                    if normalize_dns_str(&p.ip) == normalized {
                        return DnsMode::Provider(i);
                    }
                }
                if self.state.auto_dns && normalized.is_empty() {
                    DnsMode::Default
                } else if normalized.is_empty() {
                    DnsMode::Default
                } else {
                    DnsMode::Unknown
                }
            }
        }
    }

    fn mode_label(mode: DnsMode, providers: &[DnsProviderEntry]) -> String {
        match mode {
            DnsMode::Mullvad => t!("dns-mode-mullvad"),
            DnsMode::Blocky => t!("dns-mode-blocky"),
            DnsMode::Mixed => t!("dns-mode-mixed"),
            DnsMode::Default => t!("dns-mode-default"),
            DnsMode::Provider(i) => providers
                .get(i)
                .map(|p| p.label.clone())
                .unwrap_or_else(|| t!("dns-mode-unknown")),
            DnsMode::Unknown => t!("dns-mode-unknown"),
        }
    }

    pub fn update(&mut self, msg: Message) -> Task<Message> {
        match msg {
            Message::Poll => {
                if self.is_changing {
                    return Task::none();
                }
                Task::perform(probe_state(), Message::StateUpdated)
            }
            Message::StateUpdated(result) => {
                match result {
                    Ok(state) => {
                        if state != self.state {
                            debug!("dns: state changed → {:?}", state);
                            self.state = state;
                        }
                        // Hata temizle — başarılı probe.
                        if !self.last_error.is_empty() && !self.is_changing {
                            self.last_error.clear();
                        }
                    }
                    Err(e) => {
                        warn!("dns: state probe failed: {e}");
                        self.last_error = e;
                    }
                }
                Task::none()
            }
            Message::Action(action) => {
                if self.is_changing {
                    return Task::none();
                }
                self.is_changing = true;
                self.last_error.clear();
                let osc = self.config.osc_command.clone();
                let providers = self.config.providers.clone();
                Task::perform(
                    async move { apply_action(&osc, action, &providers).await },
                    Message::ActionFinished,
                )
            }
            Message::ActionFinished(result) => {
                self.is_changing = false;
                if let Err(e) = result {
                    warn!("dns: action failed: {e}");
                    self.last_error = e;
                }
                // Aksiyon biter bitmez state'i yeniden oku.
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
        let mode = self.current_mode();
        let body = container(
            row!(
                icon(Self::mode_icon(mode)),
                text(Self::mode_label(mode, &self.config.providers))
            )
            .spacing(space.xxs),
        );
        match mode {
            DnsMode::Mullvad | DnsMode::Mixed => body
                .style(|theme: &Theme| container::Style {
                    text_color: Some(theme.palette().success),
                    ..Default::default()
                })
                .into(),
            DnsMode::Unknown => body
                .style(|theme: &Theme| container::Style {
                    text_color: Some(theme.palette().warning),
                    ..Default::default()
                })
                .into(),
            _ => body.into(),
        }
    }

    pub fn menu_view(&'_ self) -> Element<'_, Message> {
        let (font_size, space) = use_theme(|t| (t.font_size, t.space));
        let mode = self.current_mode();

        let info_row = |ico: StaticIcon, label: String, value: String| {
            row!(
                container(icon(ico).size(font_size.xl)).center_x(Length::Fixed(space.xl)),
                text(label).width(Length::Fill),
                text(value)
            )
            .align_y(Alignment::Center)
            .spacing(space.xs)
        };

        // Mode butonu — quick_setting_button kullanmadık çünkü o submenu
        // üretiyor; bizim için sadece icon + label + active state yeter.
        let mode_button = |label: String,
                           ico: StaticIcon,
                           action: DnsAction,
                           active: bool,
                           disabled: bool|
         -> Element<'_, Message> {
            let style = use_theme(|t| t.quick_settings_button_style(active));
            let body = row!(icon(ico), text(label))
                .spacing(space.xxs)
                .align_y(Alignment::Center);
            let btn = button(body)
                .padding([space.xxs, space.xs])
                .style(style);
            let btn = if disabled {
                btn
            } else {
                btn.on_press(Message::Action(action))
            };
            btn.into()
        };

        // Mod butonları — bağlı mode highlight.
        let modes_row = Row::with_capacity(5)
            .push(mode_button(
                t!("dns-mode-mullvad"),
                StaticIcon::Vpn,
                DnsAction::Mullvad,
                matches!(mode, DnsMode::Mullvad | DnsMode::Mixed),
                self.is_changing,
            ))
            .push(mode_button(
                t!("dns-mode-blocky"),
                StaticIcon::Lock,
                DnsAction::Blocky,
                matches!(mode, DnsMode::Blocky | DnsMode::Mixed),
                self.is_changing,
            ))
            .push(mode_button(
                t!("dns-mode-default"),
                StaticIcon::Ethernet,
                DnsAction::Default,
                matches!(mode, DnsMode::Default),
                self.is_changing,
            ))
            .spacing(space.xxs);

        let extra_row = Row::with_capacity(2)
            .push(mode_button(
                t!("dns-action-toggle"),
                StaticIcon::Refresh,
                DnsAction::Toggle,
                false,
                self.is_changing,
            ))
            .push(mode_button(
                t!("dns-action-repair"),
                StaticIcon::Refresh,
                DnsAction::Repair,
                false,
                self.is_changing,
            ))
            .spacing(space.xxs);

        // Provider butonları — list halinde (4-5 tane var).
        let provider_buttons: Vec<Element<'_, Message>> = self
            .config
            .providers
            .iter()
            .enumerate()
            .map(|(i, p)| {
                mode_button(
                    p.label.clone(),
                    StaticIcon::IpAddress,
                    DnsAction::Provider(i),
                    matches!(mode, DnsMode::Provider(j) if j == i),
                    self.is_changing,
                )
            })
            .collect();

        let header = row!(
            text(t!("dns-heading"))
                .size(font_size.lg)
                .width(Length::Fill),
            icon_button(StaticIcon::Refresh)
                .on_press(Message::Poll)
                .size(ButtonSize::Small),
        )
        .align_y(Alignment::Center)
        .spacing(space.xs);

        let mut content = Column::with_capacity(12)
            .push(header)
            .push(divider())
            .push(info_row(
                Self::mode_icon(mode),
                t!("dns-current-mode"),
                Self::mode_label(mode, &self.config.providers),
            ))
            .push(info_row(
                StaticIcon::IpAddress,
                t!("dns-active-dns"),
                if self.state.display_dns.is_empty() {
                    "—".to_string()
                } else {
                    self.state.display_dns.join(" ")
                },
            ))
            .push(divider())
            .push(text(t!("dns-modes-title")).size(font_size.sm))
            .push(modes_row)
            .push(extra_row)
            .push(divider())
            .push(text(t!("dns-providers-title")).size(font_size.sm))
            .push(Column::with_children(provider_buttons).spacing(space.xxs));

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

/// `mullvad status` + `systemctl is-active blocky.service` + nmcli + resolvectl
/// + /etc/resolv.conf'u paralel tarayıp `DnsState` döndür.
async fn probe_state() -> Result<DnsState, String> {
    let (mullvad_out, blocky_out, nmcli_active, resolvectl_out, resolv_conf) = tokio::join!(
        run_capture("mullvad", &["status"]),
        run_capture("systemctl", &["is-active", "blocky.service"]),
        run_capture("nmcli", &["-t", "-f", "NAME,DEVICE", "connection", "show", "--active"]),
        run_capture("resolvectl", &["dns"]),
        tokio::fs::read_to_string("/etc/resolv.conf"),
    );

    let mullvad_text = mullvad_out.unwrap_or_default();
    let vpn_connected = mullvad_text.contains("Connected");
    let blocked = mullvad_text.contains("Blocked:")
        || mullvad_text.to_lowercase().contains("device has been revoked");

    let blocky_active = blocky_out
        .map(|s| s.trim() == "active")
        .unwrap_or(false);

    // Aktif NM connection — wg* ve lo dışı ilk satır.
    let primary_con = nmcli_active
        .as_deref()
        .unwrap_or("")
        .lines()
        .find_map(|line| {
            let mut parts = line.splitn(2, ':');
            let name = parts.next()?;
            let dev = parts.next()?;
            if dev == "lo" || dev.starts_with("wg") || name.is_empty() {
                None
            } else {
                Some(name.to_string())
            }
        });

    let mut dns: Vec<String> = Vec::new();
    let mut auto_dns = false;
    if let Some(con) = &primary_con {
        if let Ok(out) = run_capture(
            "nmcli",
            &["-g", "IP4.DNS", "connection", "show", con.as_str()],
        )
        .await
        {
            for token in out.split(|c: char| c.is_whitespace() || c == '|') {
                let t = token.trim();
                if is_ipv4(t) {
                    dns.push(t.to_string());
                }
            }
        }
        if let Ok(out) = run_capture(
            "nmcli",
            &["-g", "ipv4.ignore-auto-dns", "connection", "show", con.as_str()],
        )
        .await
        {
            let v = out.lines().last().unwrap_or("").trim();
            auto_dns = v.is_empty() || v == "no" || v == "false";
        }
    }

    // resolvectl Global: satırlarını parse.
    let mut display_dns: Vec<String> = Vec::new();
    if let Ok(rv) = resolvectl_out.as_deref() {
        for line in rv.lines() {
            let line = line.trim_start();
            if let Some(rest) = line.strip_prefix("Global:") {
                for tok in rest.split_whitespace() {
                    if is_ipv4(tok) {
                        display_dns.push(tok.to_string());
                    }
                }
            }
        }
    }

    // /etc/resolv.conf fallback.
    if display_dns.is_empty() {
        if let Ok(content) = &resolv_conf {
            for line in content.lines() {
                let mut parts = line.split_whitespace();
                if parts.next() == Some("nameserver") {
                    if let Some(ip) = parts.next() {
                        if is_ipv4(ip) {
                            display_dns.push(ip.to_string());
                        }
                    }
                }
            }
        }
    }

    if dns.is_empty() && display_dns.is_empty() {
        if let Ok(content) = &resolv_conf {
            for line in content.lines() {
                for tok in line.split_whitespace() {
                    if is_ipv4(tok) {
                        dns.push(tok.to_string());
                    }
                }
            }
        }
    }

    if display_dns.is_empty() {
        display_dns = dns.clone();
    }

    Ok(DnsState {
        vpn_connected,
        blocky_active,
        blocked,
        auto_dns,
        dns: dedup(dns),
        display_dns: dedup(display_dns),
    })
}

async fn apply_action(
    osc: &str,
    action: DnsAction,
    providers: &[DnsProviderEntry],
) -> Result<(), String> {
    match action {
        DnsAction::Toggle => {
            run_osc(osc, &["toggle", "--with-blocky"]).await?;
            Ok(())
        }
        DnsAction::Mullvad => {
            let st = probe_state().await.unwrap_or_default();
            if st.vpn_connected && !st.blocky_active {
                return clear_nmcli_dns_config_only().await;
            }
            if st.vpn_connected && st.blocky_active {
                run_osc(osc, &["ensure", "--grace", "0"]).await?;
                return clear_nmcli_dns_config_only().await;
            }
            run_osc(osc, &["toggle", "--with-blocky"]).await?;
            clear_nmcli_dns_config_only().await
        }
        DnsAction::Blocky => {
            let st = probe_state().await.unwrap_or_default();
            if !st.vpn_connected && st.blocky_active {
                return clear_nmcli_dns_config_only().await;
            }
            if st.vpn_connected {
                run_osc(osc, &["toggle", "--with-blocky"]).await?;
            } else {
                run_osc(osc, &["ensure", "--grace", "0"]).await?;
            }
            clear_nmcli_dns_config_only().await
        }
        DnsAction::Repair => {
            run_osc(osc, &["ensure", "--grace", "0"]).await?;
            Ok(())
        }
        DnsAction::Default => {
            direct_prep().await?;
            clear_nmcli_dns().await
        }
        DnsAction::Provider(i) => {
            let provider = providers
                .get(i)
                .ok_or_else(|| format!("provider index {i} out of range"))?
                .clone();
            direct_prep().await?;
            set_nmcli_dns(&provider.ip).await
        }
    }
}

/// `osc-mullvad` binary'sini PATH'ten ya da `~/.local/bin/`'ten bul ve çalıştır.
async fn run_osc(osc: &str, args: &[&str]) -> Result<(), String> {
    let bin = resolve_osc(osc)?;
    run_check(&bin, args).await
}

fn resolve_osc(input: &str) -> Result<String, String> {
    let p = std::path::Path::new(input);
    if p.is_absolute() && p.exists() {
        return Ok(input.to_string());
    }
    // PATH üzerinden
    if let Ok(out) = std::process::Command::new("command")
        .args(["-v", input])
        .output()
    {
        if out.status.success() {
            let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !path.is_empty() {
                return Ok(path);
            }
        }
    }
    if !input.contains('/') {
        if let Some(home) = std::env::var_os("HOME") {
            let candidate = std::path::Path::new(&home)
                .join(".local/bin")
                .join(input);
            if candidate.exists() {
                return Ok(candidate.to_string_lossy().into_owned());
            }
        }
    }
    // PATH lookup with `which`
    if let Ok(out) = std::process::Command::new("which").arg(input).output() {
        if out.status.success() {
            let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !path.is_empty() {
                return Ok(path);
            }
        }
    }
    Err(format!("osc-mullvad not found: {input}"))
}

async fn direct_prep() -> Result<(), String> {
    // Hatalara karşı toleranslı — mullvad/blocky zaten kapalıysa bu OK.
    let _ = run_capture("mullvad", &["disconnect"]).await;
    let _ = run_capture("mullvad", &["auto-connect", "set", "off"]).await;
    let _ = run_capture("mullvad", &["lockdown-mode", "set", "off"]).await;
    stop_blocky().await
}

async fn stop_blocky() -> Result<(), String> {
    // blocky.service yoksa OK.
    let list = run_capture("systemctl", &["list-unit-files", "blocky.service"])
        .await
        .unwrap_or_default();
    if !list.contains("blocky.service") {
        return Ok(());
    }
    // Active değilse OK.
    let active = run_capture("systemctl", &["is-active", "blocky.service"])
        .await
        .unwrap_or_default();
    if active.trim() != "active" {
        return Ok(());
    }
    // 1) sudo -n
    if Command::new("sudo")
        .args(["-n", "systemctl", "stop", "blocky.service"])
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
    {
        return Ok(());
    }
    // 2) pkexec
    if Command::new("pkexec")
        .args(["sh", "-c", "systemctl stop blocky.service >/dev/null 2>&1"])
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
    {
        return Ok(());
    }
    Err("blocky.service durdurulamadı (sudo -n veya pkexec gerek)".into())
}

async fn active_connection() -> Option<String> {
    let out = run_capture(
        "nmcli",
        &["-t", "-f", "NAME,DEVICE", "connection", "show", "--active"],
    )
    .await
    .ok()?;
    out.lines().find_map(|line| {
        let mut parts = line.splitn(2, ':');
        let name = parts.next()?;
        let dev = parts.next()?;
        if dev == "lo" || dev.starts_with("wg") || name.is_empty() {
            None
        } else {
            Some(name.to_string())
        }
    })
}

async fn clear_nmcli_dns_config_only() -> Result<(), String> {
    set_nmcli_dns_config("").await.map(|_| ())
}

async fn clear_nmcli_dns() -> Result<(), String> {
    let con = set_nmcli_dns_config("").await?;
    run_check("nmcli", &["con", "up", con.as_str()]).await
}

async fn set_nmcli_dns(dns: &str) -> Result<(), String> {
    let con = set_nmcli_dns_config(dns).await?;
    run_check("nmcli", &["con", "up", con.as_str()]).await
}

async fn set_nmcli_dns_config(dns: &str) -> Result<String, String> {
    let con = active_connection()
        .await
        .ok_or_else(|| "Aktif NetworkManager bağlantısı yok".to_string())?;
    let args: Vec<&str> = if dns.is_empty() {
        vec!["con", "mod", &con, "ipv4.dns", "", "ipv4.ignore-auto-dns", "no"]
    } else {
        vec![
            "con",
            "mod",
            &con,
            "ipv4.dns",
            dns,
            "ipv4.ignore-auto-dns",
            "yes",
        ]
    };
    run_check("nmcli", &args).await?;
    Ok(con)
}

// ─── subprocess primitives ───────────────────────────────────────────────────

/// Komutu çalıştır, stdout'unu döndür. Stderr atılır. Çıkış kodu sıfır değilse
/// `Err`. None / boş çıktıyı tolere eder.
async fn run_capture(bin: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(bin)
        .args(args)
        .output()
        .await
        .map_err(|e| format!("{bin}: {e}"))?;
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Komutu çalıştır; sıfır olmayan kod = hata.
async fn run_check(bin: &str, args: &[&str]) -> Result<(), String> {
    let status = Command::new(bin)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("{bin}: {e}"))?;
    if status.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&status.stderr);
        let trimmed = err.trim();
        if trimmed.is_empty() {
            Err(format!("{bin} {:?} → exit {}", args, status.status))
        } else {
            Err(format!("{bin}: {trimmed}"))
        }
    }
}

// ─── small utilities ─────────────────────────────────────────────────────────

fn is_ipv4(s: &str) -> bool {
    let mut parts = s.split('.');
    let mut count = 0;
    for p in parts.by_ref() {
        count += 1;
        if count > 4 {
            return false;
        }
        match p.parse::<u32>() {
            Ok(n) if n <= 255 => {}
            _ => return false,
        }
    }
    count == 4
}

fn dedup(mut v: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    v.retain(|s| seen.insert(s.clone()));
    v
}

/// Sıralanmış + unique IPv4 listesi (karşılaştırma için canonical form).
fn normalize_dns(list: &[String]) -> String {
    let mut v: Vec<String> = list.iter().filter(|s| is_ipv4(s)).cloned().collect();
    v.sort();
    v.dedup();
    v.join(" ")
}

fn normalize_dns_str(s: &str) -> String {
    let v: Vec<String> = s
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|p| is_ipv4(p))
        .map(|p| p.to_string())
        .collect();
    normalize_dns(&v)
}
