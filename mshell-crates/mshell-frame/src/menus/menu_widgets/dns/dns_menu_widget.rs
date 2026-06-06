//! DNS / VPN menu widget — content surface for `MenuType::Dns`.
//!
//! Mirrors the noctalia `dns/Panel.qml` layout but built from the
//! same primitives every other mshell menu widget uses. Three
//! sections:
//!
//!   1. **Hero header** — `dns-status-badge` chip carrying the mode
//!      (Protected / Auto / Preset / Blocked / Idle), title /
//!      subtitle, then three info lines (VPN state, Blocky state,
//!      current DNS list).
//!   2. **Primary actions** — four equal-width buttons (Mullvad,
//!      Blocky, Default, Toggle). The one matching the current
//!      mode gets the `.selected` class so the matugen primary
//!      accent paints it.
//!   3. **Preset rows** — Google / Cloudflare / OpenDNS / AdGuard
//!      / Quad9 with icon + label + IPs and an Apply / Active
//!      button. Active highlight is order-insensitive against the
//!      preset's DNS list.
//!
//! Action handlers:
//!   * `mullvad` — `mullvad connect / disconnect` (rootless per-
//!     user daemon socket).
//!   * `blocky` — `systemctl start / stop blocky.service`.
//!   * `default` — `nmcli connection modify <primary> ipv4.dns ""
//!     && ipv4.ignore-auto-dns no && up`, falling back to
//!     `resolvectl revert <iface>` when nmcli isn't on PATH.
//!   * preset — `resolvectl dns <iface> <ips…>`, scoped to the
//!     primary non-loopback / non-wireguard interface the probe
//!     discovered.
//!   * `toggle` — protected → off (default), idle → on (mullvad).
//!
//! Privileged steps go through `run_privileged`, which prefers
//! `sudo -n` when the user has passwordless sudo — silent, no GUI
//! agent to lose keyboard focus to under the compositor (the same
//! approach noctalia's `apply.sh` takes) — and only falls back to
//! pkexec's graphical agent when `sudo -n` isn't available.

use crate::bars::bar_widgets::dns::{DnsState, Mode, probe_dns_state};
use relm4::gtk::prelude::{BoxExt, ButtonExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tracing::warn;

const REFRESH_INTERVAL: Duration = Duration::from_secs(30);
const POST_ACTION_DELAY: Duration = Duration::from_millis(750);

/// Static preset list — matches the upstream plugin's `presets:`
/// array. (id, label, dns IPs joined by space, icon name.)
const PRESETS: &[(&str, &str, &str, &str)] = &[
    ("google", "Google", "8.8.8.8 8.8.4.4", "globe-symbolic"),
    (
        "cloudflare",
        "Cloudflare",
        "1.1.1.1 1.0.0.1",
        "globe-symbolic",
    ),
    (
        "opedns",
        "OpenDNS",
        "208.67.222.222 208.67.220.220",
        "globe-symbolic",
    ),
    (
        "adguard",
        "AdGuard",
        "94.140.14.14 94.140.15.15",
        "shield-check-symbolic",
    ),
    (
        "quad9",
        "Quad9",
        "9.9.9.9 149.112.112.112",
        "shield-check-symbolic",
    ),
];

pub(crate) struct DnsMenuWidgetModel {
    state: DnsState,
    badge: gtk::Label,
    vpn_line: gtk::Label,
    blocky_line: gtk::Label,
    dns_line: gtk::Label,
    /// Action buttons keyed by their id so `sync_view` can flip
    /// the `.selected` class onto whichever matches the current
    /// mode.
    action_buttons: Vec<(String, gtk::Button)>,
    /// Preset apply buttons keyed by preset id — text + class
    /// toggle between "Apply" / "Active".
    preset_apply_buttons: Vec<(String, gtk::Button)>,
    /// `true` once the poll loop has been spawned (on first reveal).
    poll_started: bool,
    /// A privileged action is in flight. Serialises toggles (osc-mullvad uses a
    /// flock for the same reason) so a second click — or hitting Mullvad +
    /// Blocky together — can't stack overlapping VPN/DNS/sudo operations.
    action_busy: bool,
    /// Shared with the poll loop; gates the probes so they only run
    /// while the panel is visible.
    visible: Arc<AtomicBool>,
}

impl std::fmt::Debug for DnsMenuWidgetModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DnsMenuWidgetModel")
            .field("state", &self.state)
            .finish()
    }
}

#[derive(Debug)]
pub(crate) enum DnsMenuWidgetInput {
    RunAction(String),
    RefreshNow,
    /// Sent by the host menu on show/hide. The poll loop (which probes
    /// `sudo -n`, mullvad, resolvectl) is started lazily on first
    /// reveal, so a menu the user never opens runs no DNS probes.
    ParentRevealChanged(bool),
}

#[derive(Debug)]
pub(crate) enum DnsMenuWidgetOutput {}

pub(crate) struct DnsMenuWidgetInit {}

#[derive(Debug)]
pub(crate) enum DnsMenuWidgetCommandOutput {
    Refreshed(DnsState),
    /// A user action finished (vs. a background poll) — also clears the
    /// in-flight guard so the next toggle is accepted.
    ActionDone(DnsState),
}

#[relm4::component(pub(crate))]
impl Component for DnsMenuWidgetModel {
    type CommandOutput = DnsMenuWidgetCommandOutput;
    type Input = DnsMenuWidgetInput;
    type Output = DnsMenuWidgetOutput;
    type Init = DnsMenuWidgetInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "dns-menu-widget",
            set_hexpand: false,
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 10,

            // ── Hero ────────────────────────────────────────────
            gtk::Box {
                add_css_class: "dns-hero",
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 6,

                gtk::Box {
                    add_css_class: "panel-header",
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 12,

                    gtk::Image {
                        add_css_class: "panel-header-icon",
                        set_icon_name: Some("network-vpn-symbolic"),
                        set_valign: gtk::Align::Center,
                    },

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_hexpand: true,

                        gtk::Label {
                            add_css_class: "panel-title",
                            set_label: "DNS / VPN",
                            set_xalign: 0.0,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_label: "Switch between Mullvad, Blocky, and presets",
                            set_xalign: 0.0,
                        },
                    },

                    #[local_ref]
                    badge_widget -> gtk::Label {
                        add_css_class: "dns-badge",
                        set_valign: gtk::Align::Center,
                    },
                },

                #[local_ref]
                vpn_line_widget -> gtk::Label {
                    add_css_class: "label-small",
                    set_xalign: 0.0,
                },
                #[local_ref]
                blocky_line_widget -> gtk::Label {
                    add_css_class: "label-small",
                    set_xalign: 0.0,
                },
                #[local_ref]
                dns_line_widget -> gtk::Label {
                    add_css_class: "label-small",
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_wrap_mode: gtk::pango::WrapMode::WordChar,
                },
            },

            // ── 4 primary actions ───────────────────────────────
            #[local_ref]
            actions_box -> gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 6,
                set_homogeneous: true,
            },

            gtk::Separator { set_orientation: gtk::Orientation::Horizontal },

            gtk::Label {
                add_css_class: "label-medium-bold",
                set_label: "DNS Presets",
                set_xalign: 0.0,
            },

            // ── 5 preset rows ───────────────────────────────────
            #[local_ref]
            presets_box -> gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 6,
            },

            // ── Footer ──────────────────────────────────────────
            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 6,
                set_margin_top: 4,

                gtk::Button {
                    set_css_classes: &["ok-button-surface", "ok-button-cell"],
                    set_label: "Refresh",
                    connect_clicked[sender] => move |_| {
                        sender.input(DnsMenuWidgetInput::RefreshNow);
                    },
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let badge_widget = gtk::Label::new(Some("Idle"));
        let vpn_line_widget = gtk::Label::new(Some("VPN: off"));
        let blocky_line_widget = gtk::Label::new(Some("Blocky: inactive"));
        let dns_line_widget = gtk::Label::new(Some("DNS: —"));

        // Build action buttons row.
        let actions_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let mut action_buttons: Vec<(String, gtk::Button)> = Vec::with_capacity(4);
        for (id, label, icon) in [
            ("mullvad", "Mullvad", "vpn-symbolic"),
            ("blocky", "Blocky", "server-symbolic"),
            ("default", "Default", "network-wired-symbolic"),
            ("toggle", "Toggle", "settings-symbolic"),
        ] {
            let btn = make_action_button(label, icon);
            let s = sender.clone();
            let id_owned = id.to_string();
            btn.connect_clicked(move |_| s.input(DnsMenuWidgetInput::RunAction(id_owned.clone())));
            action_buttons.push((id.to_string(), btn));
        }

        // Build preset rows + collect their apply buttons.
        let presets_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let mut preset_apply_buttons: Vec<(String, gtk::Button)> =
            Vec::with_capacity(PRESETS.len());
        for (id, label, ips, icon) in PRESETS {
            let (row, apply_btn) = make_preset_row(label, ips, icon);
            let s = sender.clone();
            let action = format!("provider:{id}");
            apply_btn
                .connect_clicked(move |_| s.input(DnsMenuWidgetInput::RunAction(action.clone())));
            preset_apply_buttons.push((id.to_string(), apply_btn));
            presets_box.append(&row);
        }

        // The poll loop is started lazily on first reveal — see
        // `ParentRevealChanged` — so a menu the user never opens runs
        // no DNS / sudo probes.
        let model = DnsMenuWidgetModel {
            state: DnsState::default(),
            badge: badge_widget.clone(),
            vpn_line: vpn_line_widget.clone(),
            blocky_line: blocky_line_widget.clone(),
            dns_line: dns_line_widget.clone(),
            action_buttons: action_buttons.clone(),
            preset_apply_buttons: preset_apply_buttons.clone(),
            poll_started: false,
            action_busy: false,
            visible: Arc::new(AtomicBool::new(false)),
        };

        // Attach the action buttons we built to the actions_box
        // before view_output! splices it into the view tree.
        for (_, btn) in &action_buttons {
            actions_box.append(btn);
        }

        let widgets = view_output!();
        sync_view(&model);

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            DnsMenuWidgetInput::RunAction(action) => {
                // Serialise: ignore a new action while one is still running.
                // This is what made "press Mullvad + Blocky together" pile up
                // overlapping VPN/DNS/sudo work — now the second is dropped.
                if self.action_busy {
                    return;
                }
                self.action_busy = true;
                run_action(action, self.state.clone(), sender.clone());
            }
            DnsMenuWidgetInput::RefreshNow => {
                sender.command(|out, _shutdown| async move {
                    let s = probe_dns_state().await;
                    let _ = out.send(DnsMenuWidgetCommandOutput::Refreshed(s));
                });
            }
            DnsMenuWidgetInput::ParentRevealChanged(visible) => {
                self.visible.store(visible, Ordering::Relaxed);
                if visible {
                    if !self.poll_started {
                        self.poll_started = true;
                        start_polling(&sender, self.visible.clone());
                    }
                    sender.input(DnsMenuWidgetInput::RefreshNow);
                }
            }
        }
    }

    fn update_cmd_with_view(
        &mut self,
        _widgets: &mut Self::Widgets,
        message: Self::CommandOutput,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            DnsMenuWidgetCommandOutput::Refreshed(state) => {
                if self.state != state {
                    self.state = state;
                    sync_view(self);
                }
            }
            DnsMenuWidgetCommandOutput::ActionDone(state) => {
                self.action_busy = false;
                if self.state != state {
                    self.state = state;
                    sync_view(self);
                }
            }
        }
    }
}

/// Spawn the perpetual poll loop. Started lazily on first reveal; the
/// probe is gated on `visible`, so while the panel is hidden the loop
/// only does a cheap timer wake — no DNS / sudo / mullvad probe.
fn start_polling(sender: &ComponentSender<DnsMenuWidgetModel>, visible: Arc<AtomicBool>) {
    sender.command(move |out, shutdown| async move {
        let shutdown_fut = shutdown.wait();
        tokio::pin!(shutdown_fut);
        loop {
            tokio::select! {
                () = &mut shutdown_fut => break,
                _ = tokio::time::sleep(REFRESH_INTERVAL) => {}
            }
            if visible.load(Ordering::Relaxed) {
                let s = probe_dns_state().await;
                let _ = out.send(DnsMenuWidgetCommandOutput::Refreshed(s));
            }
        }
    });
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
    gtk::Button::builder()
        .child(&inner)
        .css_classes(vec!["ok-button-surface", "ok-button-cell", "dns-action"])
        .hexpand(true)
        .build()
}

fn make_preset_row(label: &str, ips: &str, icon: &str) -> (gtk::Box, gtk::Button) {
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .css_classes(vec!["dns-preset-row"])
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
    // `dns-preset-apply` pins a fixed min-width so the button doesn't
    // resize when its label toggles between "Apply" (5) and the wider
    // "Active" (6) — every preset row's button then lines up.
    apply.set_css_classes(&["ok-button-surface", "dns-preset-apply"]);
    apply.set_valign(gtk::Align::Center);
    row.append(&apply);
    (row, apply)
}

fn sync_view(model: &DnsMenuWidgetModel) {
    let s = &model.state;

    let (badge_text, badge_class) = match s.mode_id() {
        Mode::Mullvad | Mode::Blocky | Mode::Mixed => ("Protected", "dns-badge-secure"),
        Mode::Default => ("Auto", "dns-badge-default"),
        Mode::Custom => ("Preset", "dns-badge-preset"),
        Mode::Blocked => ("Blocked", "dns-badge-blocked"),
        Mode::Idle => ("Idle", "dns-badge-idle"),
    };
    model.badge.set_label(badge_text);
    model.badge.set_css_classes(&["dns-badge", badge_class]);

    model.vpn_line.set_label(&format!(
        "VPN: {}",
        if s.blocked {
            "blocked / revoked"
        } else if s.vpn {
            "connected"
        } else {
            "off"
        }
    ));
    model.blocky_line.set_label(&format!(
        "Blocky: {}",
        if s.blocky { "active" } else { "inactive" }
    ));
    let dns_text = if s.display_dns.is_empty() {
        "DNS: —".to_string()
    } else {
        format!(
            "DNS: {}{}",
            s.display_dns,
            if s.auto_dns { " (auto)" } else { "" }
        )
    };
    model.dns_line.set_label(&dns_text);

    let mode = s.mode_id();
    for (id, btn) in &model.action_buttons {
        let active = matches!(
            (id.as_str(), mode),
            ("mullvad", Mode::Mullvad)
                | ("mullvad", Mode::Mixed)
                | ("blocky", Mode::Blocky)
                | ("blocky", Mode::Mixed)
                | ("default", Mode::Default)
        );
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
/// chain on relm4's tokio executor and triggers a refresh once
/// it settles.
fn run_action(action: String, state: DnsState, sender: ComponentSender<DnsMenuWidgetModel>) {
    sender.command(move |out, _shutdown| async move {
        let result = match action.as_str() {
            "mullvad" => action_mullvad(!state.vpn).await,
            "blocky" => action_blocky(!state.blocky).await,
            "default" => action_default(state.primary_conn.clone()).await,
            "toggle" => action_toggle(state.clone()).await,
            _ if action.starts_with("provider:") => {
                let preset_id = &action["provider:".len()..];
                if let Some((_, _, ips, _)) = PRESETS.iter().find(|p| p.0 == preset_id) {
                    // Pass the NM connection NAME (not the device
                    // interface name) — preset apply goes through
                    // `nmcli con mod ipv4.dns` against the active
                    // connection, matching noctalia's apply.sh.
                    // The previous resolvectl-based path silently
                    // succeeded but the override is per-link and
                    // ephemeral; NM clobbered it on the next link
                    // refresh, so user-visible nothing changed.
                    action_apply_preset(state.primary_conn.clone(), ips).await
                } else {
                    Err(format!("unknown preset: {preset_id}"))
                }
            }
            _ => Err(format!("unknown action: {action}")),
        };
        if let Err(e) = result {
            warn!(action = %action, error = %e, "dns action failed");
        }
        tokio::time::sleep(POST_ACTION_DELAY).await;
        let s = probe_dns_state().await;
        // ActionDone (not Refreshed) so update_cmd_with_view clears the
        // in-flight guard even if nothing about the state changed.
        let _ = out.send(DnsMenuWidgetCommandOutput::ActionDone(s));
    });
}

async fn action_mullvad(connect: bool) -> Result<(), String> {
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
    run_privileged(&["systemctl", subcmd, "blocky.service"]).await?;
    if want_active {
        // Blocky is the local resolver while it runs — point resolv.conf at it
        // (osc-mullvad's blocky_set_resolver_local). Best-effort: a failure
        // here shouldn't undo the successful service start.
        let _ = run_privileged_sh(
            "rm -f /etc/resolv.conf; \
             printf 'nameserver 127.0.0.1\\nnameserver ::1\\n' > /etc/resolv.conf",
        )
        .await;
    }
    Ok(())
}

async fn action_default(primary_conn: Option<String>) -> Result<(), String> {
    if let Some(name) = primary_conn {
        run_privileged(&["nmcli", "connection", "modify", &name, "ipv4.dns", ""]).await?;
        run_privileged(&[
            "nmcli",
            "connection",
            "modify",
            &name,
            "ipv4.ignore-auto-dns",
            "no",
        ])
        .await?;
        let _ = run_privileged(&["nmcli", "connection", "up", &name]).await;
        Ok(())
    } else {
        run_privileged(&["resolvectl", "revert"]).await
    }
}

async fn action_apply_preset(conn: Option<String>, ips: &str) -> Result<(), String> {
    // Port of noctalia-nplugins/dns/scripts/apply.sh `set_dns`:
    //
    //   nmcli con mod "$con" ipv4.dns "$dns" ipv4.ignore-auto-dns yes
    //   nmcli con up  "$con"
    //
    // `nmcli con mod` only edits the saved profile — the override
    // doesn't take effect until `nmcli con up` re-applies it.
    // Skipping the `up` step was the bug behind "Apply doesn't do
    // anything" — the saved DNS would update but the active
    // resolver still pointed at whatever the previous activation
    // had pushed (typically DHCP-supplied DNS or a previous
    // override).
    //
    // We also pass `ipv4.ignore-auto-dns yes` so DHCP-supplied DNS
    // servers don't clobber the explicit list on the next renew.
    let name = conn.ok_or_else(|| "no primary NetworkManager connection".to_string())?;
    // Space-separated IP list per nmcli: "8.8.8.8 8.8.4.4"
    let dns_value = ips.split_whitespace().collect::<Vec<_>>().join(" ");
    run_privileged(&[
        "nmcli",
        "con",
        "mod",
        &name,
        "ipv4.dns",
        &dns_value,
        "ipv4.ignore-auto-dns",
        "yes",
    ])
    .await?;
    // Re-activate so the new DNS takes effect immediately.
    run_privileged(&["nmcli", "con", "up", &name]).await
}

/// VPN-centric coupled toggle, mirroring osc-mullvad's
/// `toggle_basic_vpn_with_blocky`: Blocky is the DNS ad-block **fallback** used
/// while the VPN is down, and is kept off (no resolver conflict) while the VPN
/// is up.
///
///   * VPN ON  → OFF: disconnect, then start Blocky (+ point resolv.conf at it).
///   * VPN OFF → ON : stop Blocky first, then connect.
///
/// (The heavier guards in the script — account/blocked-state checks, the
/// post-connect internet health-check + rollback — are deliberately left to the
/// `osc-mullvad` CLI for now; this is the safe coupled core.)
async fn action_toggle(state: DnsState) -> Result<(), String> {
    if state.vpn {
        let _ = action_mullvad(false).await;
        action_blocky(true).await
    } else {
        let _ = action_blocky(false).await;
        action_mullvad(true).await
    }
}

/// Resolve a **non-interactive** privilege launcher, mirroring osc-mullvad's
/// `sudo_run`. Returns the sudo prefix (`["sudo","-n"]` or `["sudo","-A"]`) and
/// an optional `SUDO_ASKPASS` value, or `None` when neither passwordless sudo
/// nor an askpass helper is available.
///
/// We must NOT fall back to an interactive prompt here. The old `pkexec`
/// fallback popped a polkit password dialog, but the DNS menu is a layer-shell
/// surface holding an **exclusive keyboard grab**, so the dialog never received
/// keyboard focus — you couldn't type the password, `pkexec` blocked forever,
/// and the whole shell appeared frozen. So: passwordless `sudo -n`; else an
/// askpass via `sudo -A` if one is configured; else `None` (notify + skip).
async fn sudo_launcher() -> Option<(&'static [&'static str], Option<std::ffi::OsString>)> {
    // NOPASSWD / cached creds — silent, no agent.
    let have_sudo_n = tokio::process::Command::new("sudo")
        .args(["-n", "true"])
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false);
    if have_sudo_n {
        return Some((&["-n"], None));
    }
    // GUI askpass: honour an existing SUDO_ASKPASS, else an `askpass` helper on
    // PATH (matches the script's `SUDO_ASKPASS=askpass`). Such a dialog is a
    // normal window, not a focus-grabbing polkit prompt, so it's safe here.
    if let Some(ap) = std::env::var_os("SUDO_ASKPASS") {
        return Some((&["-A"], Some(ap)));
    }
    if let Some(p) = find_on_path("askpass") {
        return Some((&["-A"], Some(p.into_os_string())));
    }
    None
}

/// First `$PATH` entry containing an executable named `bin`.
fn find_on_path(bin: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(bin))
        .find(|p| p.is_file())
}

/// Notify + bail when no non-interactive privilege path exists, so a privileged
/// toggle degrades to a toast instead of a hang.
fn no_priv_toast() {
    mshell_launcher::notify::toast(
        "DNS / VPN",
        "Needs passwordless sudo — set up NOPASSWD for systemctl/nmcli/resolvectl (skipped, no prompt).",
    );
}

/// Run a privileged command (argv form).
async fn run_privileged(args: &[&str]) -> Result<(), String> {
    let Some((prefix, askpass)) = sudo_launcher().await else {
        no_priv_toast();
        return Err("no non-interactive privilege path".into());
    };
    let mut cmd = tokio::process::Command::new("sudo");
    cmd.args(prefix);
    if let Some(ap) = askpass {
        cmd.env("SUDO_ASKPASS", ap);
    }
    let s = cmd
        .args(args)
        .status()
        .await
        .map_err(|e| format!("sudo spawn: {e}"))?;
    if !s.success() {
        return Err(format!("sudo {} exit {s}", args.join(" ")));
    }
    Ok(())
}

/// Run a privileged shell snippet (`sudo … bash -c "<script>"`) — for the
/// multi-step resolv.conf rewrite that osc-mullvad's `blocky_set_resolver_local`
/// does. Same non-interactive policy as [`run_privileged`].
async fn run_privileged_sh(script: &str) -> Result<(), String> {
    let Some((prefix, askpass)) = sudo_launcher().await else {
        no_priv_toast();
        return Err("no non-interactive privilege path".into());
    };
    let mut cmd = tokio::process::Command::new("sudo");
    cmd.args(prefix);
    if let Some(ap) = askpass {
        cmd.env("SUDO_ASKPASS", ap);
    }
    let s = cmd
        .arg("bash")
        .arg("-c")
        .arg(script)
        .status()
        .await
        .map_err(|e| format!("sudo spawn: {e}"))?;
    if !s.success() {
        return Err(format!("sudo bash -c exit {s}"));
    }
    Ok(())
}
