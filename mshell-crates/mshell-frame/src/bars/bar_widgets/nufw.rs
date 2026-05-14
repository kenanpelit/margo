//! UFW firewall bar pill — port of the noctalia `nufw` plugin's
//! bar half.
//!
//! Render-only widget. Polls `ufw status verbose` every 120 s and
//! draws an icon + tooltip from the result:
//!
//!   * `security-high-symbolic` — UFW active
//!   * `security-low-symbolic`  — UFW inactive
//!   * `dialog-warning-symbolic` — UFW not installed / errored
//!
//! Click emits `NufwOutput::Clicked`; `frame.rs` catches that and
//! toggles the proper layer-shell `MenuType::Nufw` menu (built by
//! `menus::menu`, positioned per `config.menus.nufw_menu.position`).
//! No popover anymore — the panel surface matches every other
//! mshell menu (clipboard, screenshot, quick settings) in
//! layout, theming, and configurability.

use relm4::gtk::prelude::{ButtonExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::Duration;

const REFRESH_INTERVAL: Duration = Duration::from_secs(120);
const STARTUP_DELAY: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Status {
    Active,
    Inactive,
    Unknown,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct UfwSummary {
    pub(crate) status: Option<Status>,
    pub(crate) logging: String,
    pub(crate) incoming: String,
    pub(crate) outgoing: String,
    pub(crate) routed: String,
    /// Each entry is the raw rule line verbatim from `ufw status
    /// verbose`. Carrying the literal line lets the menu's delete
    /// button shell out to `ufw delete <RULE>` rather than depend
    /// on re-numbering after every change.
    pub(crate) rules: Vec<String>,
    pub(crate) error: Option<String>,
}

#[derive(Debug)]
pub(crate) struct NufwModel {
    summary: UfwSummary,
}

#[derive(Debug)]
pub(crate) enum NufwInput {
    Clicked,
}

#[derive(Debug)]
pub(crate) enum NufwOutput {
    Clicked,
}

pub(crate) struct NufwInit {}

#[derive(Debug)]
pub(crate) enum NufwCommandOutput {
    Refreshed(UfwSummary),
}

#[relm4::component(pub)]
impl Component for NufwModel {
    type CommandOutput = NufwCommandOutput;
    type Input = NufwInput;
    type Output = NufwOutput;
    type Init = NufwInit;

    view! {
        #[root]
        gtk::Box {
            set_css_classes: &["ok-button-surface", "ok-bar-widget", "nufw-bar-widget"],
            set_hexpand: false,
            set_vexpand: false,
            set_has_tooltip: true,

            #[name="button"]
            gtk::Button {
                set_css_classes: &["ok-button-flat"],
                set_hexpand: true,
                set_vexpand: true,
                connect_clicked[sender] => move |_| {
                    sender.input(NufwInput::Clicked);
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
                    let summary = fetch_ufw_summary().await;
                    let _ = out.send(NufwCommandOutput::Refreshed(summary));
                }
            }
        });

        let model = NufwModel {
            summary: UfwSummary::default(),
        };

        let widgets = view_output!();
        apply_visual(&widgets.image, &root, &model.summary);

        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            NufwInput::Clicked => {
                let _ = sender.output(NufwOutput::Clicked);
            }
        }
    }

    fn update_cmd_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::CommandOutput,
        _sender: ComponentSender<Self>,
        root: &Self::Root,
    ) {
        match message {
            NufwCommandOutput::Refreshed(summary) => {
                self.summary = summary;
                apply_visual(&widgets.image, root, &self.summary);
            }
        }
    }
}

fn apply_visual(image: &gtk::Image, root: &gtk::Box, s: &UfwSummary) {
    // Firewall-themed glyphs from Adwaita's `firewall-applet-*`
    // family. They read as "firewall" at bar size where the
    // generic `security-{high,low}` padlock icons look like
    // password-fields. `shields_up` carries the explicit
    // "actively protecting" visual that lines up with the
    // user's mental model of UFW.
    let icon = match s.status {
        Some(Status::Active) => "firewall-applet-shields_up-symbolic",
        Some(Status::Inactive) => "firewall-applet-symbolic",
        _ => "firewall-applet-error-symbolic",
    };
    image.set_icon_name(Some(icon));

    let tooltip = if let Some(err) = &s.error
        && s.status.is_none()
    {
        format!("UFW: {err}")
    } else {
        let mut lines = vec![format!("UFW: {}", status_word(s.status))];
        if !s.incoming.is_empty() {
            lines.push(format!("Incoming: {}", s.incoming));
        }
        if !s.outgoing.is_empty() {
            lines.push(format!("Outgoing: {}", s.outgoing));
        }
        if !s.routed.is_empty() {
            lines.push(format!("Routed: {}", s.routed));
        }
        if matches!(s.status, Some(Status::Active)) && !s.rules.is_empty() {
            lines.push(format!("Rules: {}", s.rules.len()));
        }
        if s.error.is_some() {
            lines.push("(open menu to load rules)".to_string());
        }
        lines.join("\n")
    };
    root.set_tooltip_text(Some(&tooltip));

    // Three-state class set on the bar pill so the SCSS in
    // `_nufw.scss` (.nufw-bar-widget.active / .inactive / .unknown)
    // can tint the icon green / red / amber. Mutual exclusion via
    // remove_css_class before add ensures stale state from a
    // previous refresh doesn't pile up.
    root.remove_css_class("active");
    root.remove_css_class("inactive");
    root.remove_css_class("unknown");
    match s.status {
        Some(Status::Active) => root.add_css_class("active"),
        Some(Status::Inactive) => root.add_css_class("inactive"),
        _ => root.add_css_class("unknown"),
    }
}

pub(crate) fn status_word(s: Option<Status>) -> &'static str {
    match s {
        Some(Status::Active) => "active",
        Some(Status::Inactive) => "inactive",
        _ => "unknown",
    }
}

/// Get the UFW state. `ufw` itself requires root to run (the
/// binary just bails out with `You need to be root`), so the
/// background poll cannot call it directly. Two fallbacks chained:
///
///   1. `systemctl is-active ufw.service` — no privilege needed
///      and gives the active/inactive bit (the most important
///      thing for the bar icon).
///   2. `sudo -n ufw status verbose` — only succeeds if the user
///      has a NOPASSWD entry for ufw in /etc/sudoers; gives the
///      default policies + rule list when available.
///   3. `pkexec ufw status verbose` — falls into the polkit
///      graphical-agent path, which the menu's Refresh button
///      uses explicitly (`fetch_ufw_summary_pkexec`). Auto-poll
///      does NOT trigger this — we don't want a password prompt
///      every 120 s.
///
/// `pub(crate)` so the menu widget can re-trigger fetches after
/// an action without duplicating the parse path.
pub(crate) async fn fetch_ufw_summary() -> UfwSummary {
    // Detect-only first: a missing `ufw` binary surfaces as
    // "not installed" rather than the more confusing root error.
    if which("ufw").await.is_err() {
        return UfwSummary {
            error: Some("not installed".to_string()),
            ..UfwSummary::default()
        };
    }

    // Try sudo -n (NOPASSWD). If it works, we get full info.
    if let Some(out) = run_capture("sudo", &["-n", "ufw", "status", "verbose"]).await {
        return parse_ufw_verbose(&out);
    }

    // Fall back to the status-only path so the icon at least
    // reflects active/inactive.
    let mut summary = UfwSummary::default();
    if run_capture("systemctl", &["is-active", "--quiet", "ufw.service"])
        .await
        .is_some()
    {
        summary.status = Some(Status::Active);
    } else {
        // is-active returns non-zero for inactive — distinguish
        // "service missing" from "active=false" by checking the
        // unit file existence too.
        if run_capture("systemctl", &["cat", "ufw.service"])
            .await
            .is_some()
        {
            summary.status = Some(Status::Inactive);
        }
    }
    summary.error = Some("rule details need authentication".to_string());
    summary
}

/// Privileged variant that asks polkit (graphical agent) for
/// auth. Used by the menu's Refresh button and after every
/// action so the user sees the full rule list once they've
/// entered their password. Polkit caches the answer for ~5 min
/// so subsequent calls within the menu session are silent.
pub(crate) async fn fetch_ufw_summary_pkexec() -> UfwSummary {
    if which("ufw").await.is_err() {
        return UfwSummary {
            error: Some("not installed".to_string()),
            ..UfwSummary::default()
        };
    }
    match tokio::process::Command::new("pkexec")
        .args(["ufw", "status", "verbose"])
        .output()
        .await
    {
        Ok(out) if out.status.success() => parse_ufw_verbose(&String::from_utf8_lossy(&out.stdout)),
        Ok(out) => UfwSummary {
            error: Some(format!(
                "pkexec ufw exit {}: {}",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            )),
            ..UfwSummary::default()
        },
        Err(e) => UfwSummary {
            error: Some(format!("pkexec spawn: {e}")),
            ..UfwSummary::default()
        },
    }
}

async fn which(bin: &str) -> std::io::Result<()> {
    let status = tokio::process::Command::new("which")
        .arg(bin)
        .status()
        .await?;
    if status.success() { Ok(()) } else { Err(std::io::Error::new(std::io::ErrorKind::NotFound, bin.to_string())) }
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

fn parse_ufw_verbose(stdout: &str) -> UfwSummary {
    let mut summary = UfwSummary::default();
    let mut in_rules = false;

    for line in stdout.lines() {
        let trimmed = line.trim();

        if let Some(rest) = trimmed.strip_prefix("Status:") {
            summary.status = Some(match rest.trim() {
                "active" => Status::Active,
                "inactive" => Status::Inactive,
                _ => Status::Unknown,
            });
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix("Logging:") {
            summary.logging = rest.trim().to_string();
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix("Default:") {
            for chunk in rest.split(',') {
                let chunk = chunk.trim();
                if let Some((policy, kind)) = chunk.split_once('(') {
                    let policy = policy.trim().to_string();
                    let kind = kind.trim_end_matches(')').trim();
                    match kind {
                        "incoming" => summary.incoming = policy,
                        "outgoing" => summary.outgoing = policy,
                        "routed" => summary.routed = policy,
                        _ => {}
                    }
                }
            }
            continue;
        }

        if trimmed.starts_with("To") && trimmed.contains("Action") && trimmed.contains("From") {
            continue;
        }
        if trimmed.starts_with("--") {
            in_rules = true;
            continue;
        }
        if in_rules && !trimmed.is_empty() {
            summary.rules.push(trimmed.to_string());
        }
    }

    summary
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_active_with_rules() {
        let raw = "\
Status: active
Logging: on (low)
Default: deny (incoming), allow (outgoing), disabled (routed)
New profiles: skip

To                         Action      From
--                         ------      ----
22                         ALLOW IN    Anywhere
80                         ALLOW IN    Anywhere
443                        ALLOW IN    Anywhere
";
        let s = parse_ufw_verbose(raw);
        assert_eq!(s.status, Some(Status::Active));
        assert_eq!(s.incoming, "deny");
        assert_eq!(s.outgoing, "allow");
        assert_eq!(s.routed, "disabled");
        assert_eq!(s.logging, "on (low)");
        assert_eq!(s.rules.len(), 3);
        assert!(s.rules[0].starts_with("22"));
    }

    #[test]
    fn parses_inactive() {
        let raw = "Status: inactive\n";
        let s = parse_ufw_verbose(raw);
        assert_eq!(s.status, Some(Status::Inactive));
        assert!(s.rules.is_empty());
    }
}
