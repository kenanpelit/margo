//! DNS / VPN menu widget — content surface for `MenuType::Ndns`.
//!
//! Mirrors the noctalia `ndns/Panel.qml` layout but built from the
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

use crate::bars::bar_widgets::ndns::{DnsState, Mode, probe_dns_state};
use relm4::gtk::prelude::{BoxExt, ButtonExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::Duration;
use tracing::warn;

const REFRESH_INTERVAL: Duration = Duration::from_secs(30);
const STARTUP_DELAY: Duration = Duration::from_millis(250);
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
        "opendns",
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

pub(crate) struct NdnsMenuWidgetModel {
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
}

impl std::fmt::Debug for NdnsMenuWidgetModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NdnsMenuWidgetModel")
            .field("state", &self.state)
            .finish()
    }
}

#[derive(Debug)]
pub(crate) enum NdnsMenuWidgetInput {
    RunAction(String),
    RefreshNow,
}

#[derive(Debug)]
pub(crate) enum NdnsMenuWidgetOutput {}

pub(crate) struct NdnsMenuWidgetInit {}

#[derive(Debug)]
pub(crate) enum NdnsMenuWidgetCommandOutput {
    Refreshed(DnsState),
}

#[relm4::component(pub(crate))]
impl Component for NdnsMenuWidgetModel {
    type CommandOutput = NdnsMenuWidgetCommandOutput;
    type Input = NdnsMenuWidgetInput;
    type Output = NdnsMenuWidgetOutput;
    type Init = NdnsMenuWidgetInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "ndns-menu-widget",
            set_hexpand: false,
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 10,

            // ── Hero ────────────────────────────────────────────
            gtk::Box {
                add_css_class: "ndns-hero",
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 6,

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 10,

                    gtk::Image {
                        set_icon_name: Some("vpn-symbolic"),
                        set_pixel_size: 28,
                    },

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_hexpand: true,

                        gtk::Label {
                            add_css_class: "label-large-bold",
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
                        add_css_class: "ndns-badge",
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
                    set_css_classes: &["ok-button-surface"],
                    set_label: "Refresh",
                    connect_clicked[sender] => move |_| {
                        sender.input(NdnsMenuWidgetInput::RefreshNow);
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
            btn.connect_clicked(move |_| s.input(NdnsMenuWidgetInput::RunAction(id_owned.clone())));
            action_buttons.push((id.to_string(), btn));
        }

        // Build preset rows + collect their apply buttons.
        let presets_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let mut preset_apply_buttons: Vec<(String, gtk::Button)> = Vec::with_capacity(PRESETS.len());
        for (id, label, ips, icon) in PRESETS {
            let (row, apply_btn) = make_preset_row(label, ips, icon);
            let s = sender.clone();
            let action = format!("provider:{id}");
            apply_btn.connect_clicked(move |_| {
                s.input(NdnsMenuWidgetInput::RunAction(action.clone()))
            });
            preset_apply_buttons.push((id.to_string(), apply_btn));
            presets_box.append(&row);
        }

        // Periodic poll bound to widget lifetime.
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
                    let _ = out.send(NdnsMenuWidgetCommandOutput::Refreshed(s));
                }
            }
        });

        let model = NdnsMenuWidgetModel {
            state: DnsState::default(),
            badge: badge_widget.clone(),
            vpn_line: vpn_line_widget.clone(),
            blocky_line: blocky_line_widget.clone(),
            dns_line: dns_line_widget.clone(),
            action_buttons: action_buttons.clone(),
            preset_apply_buttons: preset_apply_buttons.clone(),
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

    fn update(
        &mut self,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            NdnsMenuWidgetInput::RunAction(action) => {
                run_action(action, self.state.clone(), sender.clone());
            }
            NdnsMenuWidgetInput::RefreshNow => {
                sender.command(|out, _shutdown| async move {
                    let s = probe_dns_state().await;
                    let _ = out.send(NdnsMenuWidgetCommandOutput::Refreshed(s));
                });
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
            NdnsMenuWidgetCommandOutput::Refreshed(state) => {
                if self.state != state {
                    self.state = state;
                    sync_view(self);
                }
            }
        }
    }
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
        .css_classes(vec!["ok-button-surface", "ndns-action"])
        .hexpand(true)
        .build()
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

fn sync_view(model: &NdnsMenuWidgetModel) {
    let s = &model.state;

    let (badge_text, badge_class) = match s.mode_id() {
        Mode::Mullvad | Mode::Blocky | Mode::Mixed => ("Protected", "ndns-badge-secure"),
        Mode::Default => ("Auto", "ndns-badge-default"),
        Mode::Custom => ("Preset", "ndns-badge-preset"),
        Mode::Blocked => ("Blocked", "ndns-badge-blocked"),
        Mode::Idle => ("Idle", "ndns-badge-idle"),
    };
    model.badge.set_label(badge_text);
    model.badge.set_css_classes(&["ndns-badge", badge_class]);

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
fn run_action(action: String, state: DnsState, sender: ComponentSender<NdnsMenuWidgetModel>) {
    sender.command(move |out, _shutdown| async move {
        let result = match action.as_str() {
            "mullvad" => action_mullvad(!state.vpn).await,
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
        let s = probe_dns_state().await;
        let _ = out.send(NdnsMenuWidgetCommandOutput::Refreshed(s));
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
    run_privileged(&["systemctl", subcmd, "blocky.service"]).await
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

async fn action_apply_preset(device: Option<String>, ips: &str) -> Result<(), String> {
    let iface = device.ok_or_else(|| "no primary network device".to_string())?;
    let mut args: Vec<String> = vec!["resolvectl".to_string(), "dns".to_string(), iface];
    for ip in ips.split_whitespace() {
        args.push(ip.to_string());
    }
    let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    run_privileged(&refs).await
}

async fn action_toggle(state: DnsState) -> Result<(), String> {
    if state.vpn || state.blocky {
        if state.vpn {
            let _ = action_mullvad(false).await;
        }
        if state.blocky {
            let _ = action_blocky(false).await;
        }
        action_default(state.primary_conn.clone()).await
    } else {
        action_mullvad(true).await
    }
}

/// Run a privileged command.
///
/// Probes `sudo -n true` first: if passwordless sudo is set up
/// for the user, the command runs through `sudo -n` — silent, no
/// graphical agent. Under a Wayland compositor pkexec's polkit
/// prompt can come up without keyboard focus (you can't type the
/// password into it), so `sudo -n` is strongly preferred and
/// pkexec is only the fallback for users without NOPASSWD sudo.
async fn run_privileged(args: &[&str]) -> Result<(), String> {
    let have_sudo_n = tokio::process::Command::new("sudo")
        .args(["-n", "true"])
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false);

    let (bin, prefix): (&str, &[&str]) = if have_sudo_n {
        ("sudo", &["-n"])
    } else {
        ("pkexec", &[])
    };

    let s = tokio::process::Command::new(bin)
        .args(prefix)
        .args(args)
        .status()
        .await
        .map_err(|e| format!("{bin} spawn: {e}"))?;
    if !s.success() {
        return Err(format!("{bin} {} exit {s}", args.join(" ")));
    }
    Ok(())
}
