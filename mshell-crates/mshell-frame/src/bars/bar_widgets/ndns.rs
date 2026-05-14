//! DNS / VPN bar widget + panel — full port of the noctalia
//! `ndns` plugin.
//!
//! Two surfaces:
//!   1. **Bar pill** — most-secure-wins icon (shield-safe →
//!      security-high → network-server → network-wired →
//!      security-low) with tooltip showing VPN / Blocky / DNS
//!      state. Click toggles the panel popover.
//!   2. **Popover panel** — hero header (status badge + current
//!      DNS line + VPN / Blocky indicators), four primary action
//!      buttons (Mullvad, Blocky, Default, Toggle), five preset
//!      DNS provider rows (Google, Cloudflare, OpenDNS, AdGuard,
//!      Quad9), each with Apply / Active buttons.
//!
//! Actions are implemented purely in Rust subprocess calls,
//! mirroring the semantics of `scripts/apply.sh` from the noctalia
//! plugin:
//!
//!   * `mullvad` — `mullvad connect` (no privilege needed — runs
//!     against the per-user mullvad-daemon socket).
//!   * `blocky` — `pkexec systemctl start blocky.service` (and the
//!     equivalent `stop` when already active).
//!   * `default` — clears any explicit DNS overrides via
//!     `pkexec nmcli connection modify <primary> ipv4.dns ""`
//!     and reloads it, falling back to `pkexec resolvectl revert
//!     <iface>` when nmcli isn't present.
//!   * preset providers — `pkexec resolvectl dns <iface> <ips>`,
//!     scoped to the primary non-loopback / non-wireguard link.
//!   * `toggle` — smart cycle: if VPN/Blocky on → off (default);
//!     otherwise → on (mullvad).
//!
//! All privileged paths go through pkexec so the session's polkit
//! agent surfaces a graphical password prompt; the previous MVP
//! shelled out to a terminal, which silently failed because
//! `sudo` inside a non-interactive emulator can't read the user's
//! password.

use relm4::gtk::prelude::{BoxExt, ButtonExt, PopoverExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::Duration;
use tracing::warn;

const REFRESH_INTERVAL: Duration = Duration::from_secs(30);
const STARTUP_DELAY: Duration = Duration::from_secs(1);
const POST_ACTION_DELAY: Duration = Duration::from_millis(750);

/// Static preset list — matches the upstream plugin's `presets:`
/// array. (id, label, dns IPs joined by space, icon name.)
const PRESETS: &[(&str, &str, &str, &str)] = &[
    ("google", "Google", "8.8.8.8 8.8.4.4", "google-symbolic"),
    (
        "cloudflare",
        "Cloudflare",
        "1.1.1.1 1.0.0.1",
        "weather-clear-symbolic",
    ),
    (
        "opendns",
        "OpenDNS",
        "208.67.222.222 208.67.220.220",
        "globe-symbolic",
    ),
    (
        "adguard",
        "AdGuard",
        "94.140.14.14 94.140.15.15",
        "shield-safe-symbolic",
    ),
    (
        "quad9",
        "Quad9",
        "9.9.9.9 149.112.112.112",
        "security-high-symbolic",
    ),
];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct DnsState {
    vpn: bool,
    blocky: bool,
    blocked: bool,
    /// Pretty-printed list of nameservers (space-separated).
    display_dns: String,
    auto_dns: bool,
    /// Name of the primary NM connection (used as the link target
    /// for resolvectl / nmcli action subcommands). None when nmcli
    /// isn't present.
    primary_conn: Option<String>,
    /// Device name for the primary connection (resolvectl wants
    /// the interface, not the connection name).
    primary_device: Option<String>,
    error: Option<String>,
}

impl DnsState {
    /// "Mode" classification used by the panel's hero badge and
    /// the highlight on the four action buttons.
    fn mode_id(&self) -> Mode {
        if self.blocked {
            Mode::Blocked
        } else if self.vpn && self.blocky {
            Mode::Mixed
        } else if self.vpn {
            Mode::Mullvad
        } else if self.blocky {
            Mode::Blocky
        } else if self.auto_dns {
            Mode::Default
        } else if !self.display_dns.is_empty() {
            Mode::Custom
        } else {
            Mode::Idle
        }
    }

    /// True when the current DNS list matches the given preset's
    /// IPs (order-insensitive). Used to mark the active preset row
    /// in the panel.
    fn matches_preset(&self, preset_ips: &str) -> bool {
        let mut current: Vec<&str> = self.display_dns.split_whitespace().collect();
        let mut want: Vec<&str> = preset_ips.split_whitespace().collect();
        current.sort();
        want.sort();
        !current.is_empty() && current == want
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Mullvad,
    Blocky,
    Default,
    Mixed,
    Custom,
    Blocked,
    Idle,
}

#[derive(Debug)]
pub(crate) struct NdnsModel {
    state: DnsState,
    popover: Option<gtk::Popover>,
    badge: Option<gtk::Label>,
    vpn_line: Option<gtk::Label>,
    blocky_line: Option<gtk::Label>,
    dns_line: Option<gtk::Label>,
    /// Action buttons keyed by their id ("mullvad", "blocky",
    /// "default", "toggle") so refresh can re-highlight whichever
    /// is currently active.
    action_buttons: Vec<(String, gtk::Button)>,
    /// Preset rows keyed by their id ("google", "cloudflare", …).
    /// Each row is a Box containing label + apply button + "Active"
    /// badge — the apply button text changes between "Apply" /
    /// "Active" via `sync_popover`.
    preset_apply_buttons: Vec<(String, gtk::Button)>,
}

#[derive(Debug)]
pub(crate) enum NdnsInput {
    BarClicked,
    RunAction(String),
    RefreshNow,
}

#[derive(Debug)]
pub(crate) enum NdnsOutput {}

pub(crate) struct NdnsInit {}

#[derive(Debug)]
pub(crate) enum NdnsCommandOutput {
    Refreshed(DnsState),
    KickRefresh,
}

#[relm4::component(pub)]
impl Component for NdnsModel {
    type CommandOutput = NdnsCommandOutput;
    type Input = NdnsInput;
    type Output = NdnsOutput;
    type Init = NdnsInit;

    view! {
        #[root]
        gtk::Box {
            set_css_classes: &["ok-button-surface", "ok-bar-widget", "ndns-bar-widget"],
            set_hexpand: false,
            set_vexpand: false,
            set_has_tooltip: true,

            #[name="button"]
            gtk::Button {
                set_css_classes: &["ok-button-flat"],
                set_hexpand: true,
                set_vexpand: true,
                connect_clicked[sender] => move |_| {
                    sender.input(NdnsInput::BarClicked);
                },

                #[name="image"]
                gtk::Image {
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,
                }
            }
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        sender.command(|out, shutdown| {
            async move {
                let shutdown_fut = shutdown.wait();
                tokio::pin!(shutdown_fut);
                let mut first = true;
                loop {
                    let delay = if first { STARTUP_DELAY } else { REFRESH_INTERVAL };
                    first = false;
                    tokio::select! {
                        () = &mut shutdown_fut => break,
                        _ = tokio::time::sleep(delay) => {}
                    }
                    let s = probe_dns_state().await;
                    let _ = out.send(NdnsCommandOutput::Refreshed(s));
                }
            }
        });

        let mut model = NdnsModel {
            state: DnsState::default(),
            popover: None,
            badge: None,
            vpn_line: None,
            blocky_line: None,
            dns_line: None,
            action_buttons: Vec::new(),
            preset_apply_buttons: Vec::new(),
        };

        let widgets = view_output!();

        let popover = build_popover(&sender, &mut model);
        popover.set_parent(&widgets.button);
        model.popover = Some(popover);

        apply_visual(&widgets.image, &root, &model.state);
        sync_popover(&model);

        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            NdnsInput::BarClicked => {
                if let Some(p) = &self.popover {
                    if p.is_visible() {
                        p.popdown();
                    } else {
                        sync_popover(self);
                        p.popup();
                    }
                }
            }
            NdnsInput::RunAction(action) => {
                run_action(action, self.state.clone(), sender.clone());
            }
            NdnsInput::RefreshNow => {
                sender.spawn_command(|out| {
                    tokio::spawn(async move {
                        let s = probe_dns_state().await;
                        let _ = out.send(NdnsCommandOutput::Refreshed(s));
                    });
                });
            }
        }
    }

    fn update_cmd_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        root: &Self::Root,
    ) {
        match message {
            NdnsCommandOutput::Refreshed(state) => {
                if self.state != state {
                    self.state = state;
                    apply_visual(&widgets.image, root, &self.state);
                    sync_popover(self);
                }
            }
            NdnsCommandOutput::KickRefresh => {
                sender.input(NdnsInput::RefreshNow);
            }
        }
    }
}

fn apply_visual(image: &gtk::Image, root: &gtk::Box, s: &DnsState) {
    // Use existing Adwaita symbolics only — `shield-safe-symbolic`
    // (which an earlier version used) isn't shipped by Adwaita,
    // Papirus, or Tela, so it rendered as the missing-icon
    // placeholder. The VPN cloud (`network-vpn-symbolic`) is the
    // clearest "I'm on a VPN" cue for both the pure-Mullvad and
    // Mixed states; Blocky-only stays on the server-rack glyph;
    // a custom DNS preset gets `globe-symbolic` so it's visually
    // distinct from the DHCP path.
    let icon = match s.mode_id() {
        Mode::Blocked => "network-vpn-error-symbolic",
        Mode::Mixed => "network-vpn-symbolic",
        Mode::Mullvad => "network-vpn-symbolic",
        Mode::Blocky => "network-server-symbolic",
        Mode::Custom => "globe-symbolic",
        Mode::Default => "network-wired-symbolic",
        Mode::Idle => "dialog-question-symbolic",
    };
    image.set_icon_name(Some(icon));

    let tooltip = if let Some(err) = &s.error {
        format!("DNS: {err}")
    } else {
        let mut lines = Vec::with_capacity(4);
        lines.push(format!(
            "VPN: {}",
            if s.blocked {
                "blocked / revoked"
            } else if s.vpn {
                "connected"
            } else {
                "off"
            }
        ));
        lines.push(format!(
            "Blocky: {}",
            if s.blocky { "active" } else { "inactive" }
        ));
        if s.display_dns.is_empty() {
            lines.push("DNS: (none)".to_string());
        } else {
            lines.push(format!(
                "DNS: {}{}",
                s.display_dns,
                if s.auto_dns { " (auto)" } else { "" }
            ));
        }
        lines.join("\n")
    };
    root.set_tooltip_text(Some(&tooltip));

    root.remove_css_class("secure");
    root.remove_css_class("blocked");
    if s.blocked {
        root.add_css_class("blocked");
    } else if s.vpn || s.blocky {
        root.add_css_class("secure");
    }
}

fn build_popover(sender: &ComponentSender<NdnsModel>, model: &mut NdnsModel) -> gtk::Popover {
    let popover = gtk::Popover::new();
    popover.add_css_class("ndns-panel");
    popover.set_has_arrow(false);
    popover.set_autohide(true);

    let outer = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .margin_start(12)
        .margin_end(12)
        .margin_top(10)
        .margin_bottom(10)
        .width_request(420)
        .build();

    // ── Hero header ────────────────────────────────────────────
    let hero = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .css_classes(vec!["ndns-hero"])
        .build();
    let hero_top = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(10)
        .build();
    let icon = gtk::Image::from_icon_name("network-server-symbolic");
    icon.set_pixel_size(28);
    hero_top.append(&icon);

    let titles = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .build();
    let title = gtk::Label::new(Some("DNS / VPN"));
    title.add_css_class("label-large-bold");
    title.set_xalign(0.0);
    titles.append(&title);
    let subtitle = gtk::Label::new(Some("Switch between Mullvad, Blocky, and presets"));
    subtitle.add_css_class("label-small");
    subtitle.set_xalign(0.0);
    titles.append(&subtitle);
    hero_top.append(&titles);

    let badge = gtk::Label::new(Some("Idle"));
    badge.add_css_class("ndns-badge");
    badge.set_valign(gtk::Align::Center);
    hero_top.append(&badge);
    hero.append(&hero_top);

    let vpn_line = gtk::Label::new(Some("VPN: off"));
    vpn_line.add_css_class("label-small");
    vpn_line.set_xalign(0.0);
    hero.append(&vpn_line);

    let blocky_line = gtk::Label::new(Some("Blocky: inactive"));
    blocky_line.add_css_class("label-small");
    blocky_line.set_xalign(0.0);
    hero.append(&blocky_line);

    let dns_line = gtk::Label::new(Some("DNS: —"));
    dns_line.add_css_class("label-small");
    dns_line.set_xalign(0.0);
    dns_line.set_wrap(true);
    dns_line.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    hero.append(&dns_line);
    outer.append(&hero);

    // ── 4 primary actions ──────────────────────────────────────
    let actions = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .homogeneous(true)
        .build();
    for (id, label, icon) in [
        ("mullvad", "Mullvad", "security-high-symbolic"),
        ("blocky", "Blocky", "network-server-symbolic"),
        ("default", "Default", "network-wired-symbolic"),
        ("toggle", "Toggle", "system-switch-user-symbolic"),
    ] {
        let btn = make_action_button(label, icon);
        let s = sender.clone();
        let id_owned = id.to_string();
        btn.connect_clicked(move |_| s.input(NdnsInput::RunAction(id_owned.clone())));
        actions.append(&btn);
        model.action_buttons.push((id.to_string(), btn));
    }
    outer.append(&actions);

    outer.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

    let presets_title = gtk::Label::new(Some("DNS Presets"));
    presets_title.add_css_class("label-medium-bold");
    presets_title.set_xalign(0.0);
    outer.append(&presets_title);

    // ── 5 preset rows ──────────────────────────────────────────
    for (id, label, ips, icon) in PRESETS {
        let (row, apply_btn) = make_preset_row(label, ips, icon);
        let s = sender.clone();
        let action = format!("provider:{id}");
        apply_btn.connect_clicked(move |_| s.input(NdnsInput::RunAction(action.clone())));
        model
            .preset_apply_buttons
            .push((id.to_string(), apply_btn));
        outer.append(&row);
    }

    // ── Footer: refresh ───────────────────────────────────────
    let footer = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .margin_top(4)
        .build();
    let refresh = gtk::Button::with_label("Refresh");
    refresh.add_css_class("ok-button-surface");
    let s = sender.clone();
    refresh.connect_clicked(move |_| s.input(NdnsInput::RefreshNow));
    footer.append(&refresh);
    outer.append(&footer);

    popover.set_child(Some(&outer));

    model.badge = Some(badge);
    model.vpn_line = Some(vpn_line);
    model.blocky_line = Some(blocky_line);
    model.dns_line = Some(dns_line);

    popover
}

fn make_action_button(label: &str, icon: &str) -> gtk::Button {
    let inner = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .halign(gtk::Align::Center)
        .build();
    inner.append(&gtk::Image::from_icon_name(icon));
    let l = gtk::Label::new(Some(label));
    l.add_css_class("label-small-bold");
    inner.append(&l);
    let btn = gtk::Button::builder()
        .child(&inner)
        .css_classes(vec!["ok-button-surface", "ndns-action"])
        .build();
    btn
}

fn make_preset_row(label: &str, ips: &str, icon: &str) -> (gtk::Box, gtk::Button) {
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .css_classes(vec!["ndns-preset-row"])
        .build();
    row.append(&gtk::Image::from_icon_name(icon));
    let texts = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .build();
    let name = gtk::Label::new(Some(label));
    name.add_css_class("label-medium-bold");
    name.set_xalign(0.0);
    texts.append(&name);
    let ips_label = gtk::Label::new(Some(ips));
    ips_label.add_css_class("label-small");
    ips_label.set_xalign(0.0);
    texts.append(&ips_label);
    row.append(&texts);

    let apply = gtk::Button::with_label("Apply");
    apply.add_css_class("ok-button-surface");
    apply.set_valign(gtk::Align::Center);
    row.append(&apply);
    (row, apply)
}

/// Re-render every popover widget that depends on `model.state`.
fn sync_popover(model: &NdnsModel) {
    let s = &model.state;

    if let Some(b) = &model.badge {
        let (text, class) = match s.mode_id() {
            Mode::Mullvad | Mode::Blocky | Mode::Mixed => ("Protected", "ndns-badge-secure"),
            Mode::Default => ("Auto", "ndns-badge-default"),
            Mode::Custom => ("Preset", "ndns-badge-preset"),
            Mode::Blocked => ("Blocked", "ndns-badge-blocked"),
            Mode::Idle => ("Idle", "ndns-badge-idle"),
        };
        b.set_label(text);
        b.set_css_classes(&["ndns-badge", class]);
    }
    if let Some(l) = &model.vpn_line {
        l.set_label(&format!(
            "VPN: {}",
            if s.blocked {
                "blocked / revoked"
            } else if s.vpn {
                "connected"
            } else {
                "off"
            }
        ));
    }
    if let Some(l) = &model.blocky_line {
        l.set_label(&format!(
            "Blocky: {}",
            if s.blocky { "active" } else { "inactive" }
        ));
    }
    if let Some(l) = &model.dns_line {
        let txt = if s.display_dns.is_empty() {
            "DNS: —".to_string()
        } else {
            format!(
                "DNS: {}{}",
                s.display_dns,
                if s.auto_dns { " (auto)" } else { "" }
            )
        };
        l.set_label(&txt);
    }

    let mode = s.mode_id();
    for (id, btn) in &model.action_buttons {
        let active = match (id.as_str(), mode) {
            ("mullvad", Mode::Mullvad) | ("mullvad", Mode::Mixed) => true,
            ("blocky", Mode::Blocky) | ("blocky", Mode::Mixed) => true,
            ("default", Mode::Default) => true,
            _ => false,
        };
        if active {
            btn.add_css_class("selected");
        } else {
            btn.remove_css_class("selected");
        }
    }

    for (id, btn) in &model.preset_apply_buttons {
        let preset_ips = PRESETS
            .iter()
            .find(|(pid, _, _, _)| pid == id)
            .map(|(_, _, ips, _)| *ips)
            .unwrap_or("");
        let active = s.matches_preset(preset_ips);
        btn.set_label(if active { "Active" } else { "Apply" });
        if active {
            btn.add_css_class("selected");
        } else {
            btn.remove_css_class("selected");
        }
    }
}

/// Dispatch an action by id. Spawns the appropriate subprocess
/// chain on the tokio runtime and triggers a refresh once it
/// settles.
fn run_action(action: String, state: DnsState, sender: ComponentSender<NdnsModel>) {
    sender.spawn_command(move |out| {
        tokio::spawn(async move {
            let result = match action.as_str() {
                "mullvad" => action_mullvad(true).await,
                "blocky" => action_blocky(!state.blocky).await,
                "default" => action_default(state.primary_conn.clone()).await,
                "toggle" => action_toggle(state.clone()).await,
                _ if action.starts_with("provider:") => {
                    let preset_id = &action["provider:".len()..];
                    if let Some((_, _, ips, _)) = PRESETS.iter().find(|p| p.0 == preset_id) {
                        action_apply_preset(state.primary_device.clone(), ips).await
                    } else {
                        Err(format!("unknown preset: {preset_id}"))
                    }
                }
                _ => Err(format!("unknown action: {action}")),
            };
            if let Err(e) = result {
                warn!(action = %action, error = %e, "ndns action failed");
            }
            tokio::time::sleep(POST_ACTION_DELAY).await;
            let _ = out.send(NdnsCommandOutput::KickRefresh);
        });
    });
}

async fn action_mullvad(connect: bool) -> Result<(), String> {
    // `mullvad` daemon socket is per-user, no pkexec needed.
    let arg = if connect { "connect" } else { "disconnect" };
    let s = tokio::process::Command::new("mullvad")
        .arg(arg)
        .status()
        .await
        .map_err(|e| format!("mullvad spawn: {e}"))?;
    if !s.success() {
        return Err(format!("mullvad {arg} exit {s}"));
    }
    Ok(())
}

async fn action_blocky(want_active: bool) -> Result<(), String> {
    let subcmd = if want_active { "start" } else { "stop" };
    pkexec(&["systemctl", subcmd, "blocky.service"]).await
}

async fn action_default(primary_conn: Option<String>) -> Result<(), String> {
    if let Some(name) = primary_conn {
        // Clear any pinned DNS on the NM connection and re-up the
        // link so DHCP-provided servers take over.
        pkexec(&[
            "nmcli",
            "connection",
            "modify",
            &name,
            "ipv4.dns",
            "",
        ])
        .await?;
        pkexec(&["nmcli", "connection", "modify", &name, "ipv4.ignore-auto-dns", "no"]).await?;
        let _ = pkexec(&["nmcli", "connection", "up", &name]).await;
        Ok(())
    } else {
        // No NM in the picture: ask systemd-resolved to drop the
        // explicit override and inherit again.
        pkexec(&["resolvectl", "revert"]).await
    }
}

async fn action_apply_preset(device: Option<String>, ips: &str) -> Result<(), String> {
    let iface = device.ok_or_else(|| "no primary network device".to_string())?;
    let mut args: Vec<String> = vec![
        "resolvectl".to_string(),
        "dns".to_string(),
        iface,
    ];
    for ip in ips.split_whitespace() {
        args.push(ip.to_string());
    }
    let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    pkexec(&refs).await
}

async fn action_toggle(state: DnsState) -> Result<(), String> {
    if state.vpn || state.blocky {
        // Already protected → go back to default.
        if state.vpn {
            let _ = action_mullvad(false).await;
        }
        if state.blocky {
            let _ = action_blocky(false).await;
        }
        action_default(state.primary_conn.clone()).await
    } else {
        // Idle → bring up Mullvad as the first-choice secure path.
        action_mullvad(true).await
    }
}

async fn pkexec(args: &[&str]) -> Result<(), String> {
    let s = tokio::process::Command::new("pkexec")
        .args(args)
        .status()
        .await
        .map_err(|e| format!("pkexec spawn: {e}"))?;
    if !s.success() {
        return Err(format!("pkexec {} exit {s}", args.join(" ")));
    }
    Ok(())
}

async fn probe_dns_state() -> DnsState {
    let mut state = DnsState::default();

    if let Some(status) = run_capture("mullvad", &["status"]).await {
        if status.contains("Connected") {
            state.vpn = true;
        }
        let lower = status.to_lowercase();
        if lower.contains("blocked:") || lower.contains("device has been revoked") {
            state.blocked = true;
        }
    }

    if run_capture("systemctl", &["is-active", "--quiet", "blocky.service"])
        .await
        .is_some()
    {
        state.blocky = true;
    }

    if let Some((name, device)) = primary_nm_connection().await {
        state.primary_conn = Some(name.clone());
        state.primary_device = Some(device);
        if let Some(dns) =
            run_capture("nmcli", &["-g", "IP4.DNS", "connection", "show", &name]).await
        {
            let cleaned = dns
                .split_whitespace()
                .filter(|s| looks_like_ipv4(s))
                .collect::<Vec<_>>()
                .join(" ");
            if !cleaned.is_empty() {
                state.display_dns = cleaned;
            }
        }
        if let Some(ignore_auto) = run_capture(
            "nmcli",
            &[
                "-g",
                "ipv4.ignore-auto-dns",
                "connection",
                "show",
                &name,
            ],
        )
        .await
        {
            let v = ignore_auto.trim().to_lowercase();
            if v.is_empty() || v == "no" || v == "false" {
                state.auto_dns = true;
            }
        }
    }

    if let Some(resolvectl) = run_capture("resolvectl", &["dns"]).await {
        let global = resolvectl
            .lines()
            .filter_map(|l| l.trim().strip_prefix("Global:"))
            .map(|s| s.trim())
            .collect::<Vec<_>>()
            .join(" ");
        if !global.is_empty() {
            state.display_dns = global;
        }
    }

    if state.display_dns.is_empty() {
        if let Ok(raw) = tokio::fs::read_to_string("/etc/resolv.conf").await {
            let parsed: Vec<&str> = raw
                .lines()
                .filter_map(|l| {
                    let t = l.trim();
                    if t.starts_with('#') {
                        return None;
                    }
                    t.strip_prefix("nameserver").map(|s| s.trim())
                })
                .collect();
            state.display_dns = parsed.join(" ");
        }
    }

    if state.display_dns.is_empty()
        && !state.vpn
        && !state.blocky
        && !state.blocked
    {
        state.error = Some("no DNS probes available".to_string());
    }
    state
}

async fn primary_nm_connection() -> Option<(String, String)> {
    let out =
        run_capture("nmcli", &["-t", "-f", "NAME,DEVICE", "connection", "show", "--active"]).await?;
    for line in out.lines() {
        let mut parts = line.splitn(2, ':');
        let name = parts.next()?.to_string();
        let device = parts.next().unwrap_or("");
        if device == "lo" || device.starts_with("wg") {
            continue;
        }
        return Some((name, device.to_string()));
    }
    None
}

fn looks_like_ipv4(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.is_empty() || bytes.len() > 15 {
        return false;
    }
    let mut octets = 0;
    let mut digits = 0;
    for b in bytes {
        match b {
            b'0'..=b'9' => {
                digits += 1;
                if digits > 3 {
                    return false;
                }
            }
            b'.' => {
                if digits == 0 {
                    return false;
                }
                octets += 1;
                digits = 0;
            }
            _ => return false,
        }
    }
    octets == 3 && digits > 0
}

async fn run_capture(cmd: &str, args: &[&str]) -> Option<String> {
    let out = tokio::process::Command::new(cmd)
        .args(args)
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_preset_unordered() {
        let s = DnsState {
            display_dns: "8.8.4.4 8.8.8.8".to_string(),
            ..DnsState::default()
        };
        assert!(s.matches_preset("8.8.8.8 8.8.4.4"));
        assert!(!s.matches_preset("1.1.1.1"));
    }
}
