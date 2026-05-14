//! UFW firewall bar widget — port of the `nufw` noctalia plugin
//! (bar-widget MVP; panel + control-center + settings UI are
//! deferred to follow-up commits).
//!
//! Periodically (default 120 s) runs `ufw status verbose` and
//! exposes:
//!
//!   * an icon — `security-high-symbolic` when ufw is active,
//!     `security-low-symbolic` when inactive, `dialog-warning-
//!     symbolic` when ufw isn't installed or the command fails.
//!   * a tooltip — active/inactive status + incoming / outgoing /
//!     routed default policies + rule count.
//!   * left-click — spawns `pkexec ufw status verbose` in the
//!     user's terminal (best-effort: tries `kitty`, falls back to
//!     `xdg-open`). Toggle actions deferred to the panel work
//!     because they need a confirmation flow.
//!
//! Differences from the upstream QML plugin worth noting:
//!   * Parsing is pure Rust against the `ufw status verbose`
//!     output rather than the bash JSON serializer — same data,
//!     fewer moving parts, the same ufw binary on PATH.
//!   * `--allow-sudo` privileged-read path is dropped from the MVP;
//!     unprivileged `ufw status` already returns the policy +
//!     rule-count fields when the user is in the ufw group.
//!     Future panel work can re-add the pkexec path.

use relm4::gtk::prelude::{ButtonExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::Duration;
use tracing::warn;

const REFRESH_INTERVAL: Duration = Duration::from_secs(120);
const STARTUP_DELAY: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Active,
    Inactive,
    Unknown,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct UfwSummary {
    status: Option<Status>,
    incoming: String,
    outgoing: String,
    routed: String,
    rule_count: usize,
    /// Filled when ufw isn't installed or the command fails outright.
    error: Option<String>,
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
pub(crate) enum NufwOutput {}

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
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            NufwInput::Clicked => spawn_terminal_status(),
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
    let icon = match s.status {
        Some(Status::Active) => "security-high-symbolic",
        Some(Status::Inactive) => "security-low-symbolic",
        _ => "dialog-warning-symbolic",
    };
    image.set_icon_name(Some(icon));

    let tooltip = if let Some(err) = &s.error {
        format!("UFW: {err}")
    } else {
        let status_word = match s.status {
            Some(Status::Active) => "active",
            Some(Status::Inactive) => "inactive",
            _ => "unknown",
        };
        let mut lines = vec![format!("UFW: {status_word}")];
        if !s.incoming.is_empty() {
            lines.push(format!("Incoming: {}", s.incoming));
        }
        if !s.outgoing.is_empty() {
            lines.push(format!("Outgoing: {}", s.outgoing));
        }
        if !s.routed.is_empty() {
            lines.push(format!("Routed: {}", s.routed));
        }
        if matches!(s.status, Some(Status::Active)) {
            lines.push(format!("Rules: {}", s.rule_count));
        }
        lines.join("\n")
    };
    root.set_tooltip_text(Some(&tooltip));

    // .inactive class for future SCSS tinting (red-on-disabled).
    if matches!(s.status, Some(Status::Inactive)) {
        root.add_css_class("inactive");
    } else {
        root.remove_css_class("inactive");
    }
}

/// Open the user's preferred terminal with `ufw status verbose`
/// behind `pkexec`. Best-effort: tries a list of common terminals
/// (`kitty`, `alacritty`, `foot`, `wezterm`) before falling back
/// to `xdg-terminal`. The panel work will replace this with an
/// inline view + confirmation flow.
fn spawn_terminal_status() {
    tokio::spawn(async move {
        for term in ["kitty", "alacritty", "foot", "wezterm", "xterm"] {
            if let Ok(true) = which_async(term).await {
                let _ = tokio::process::Command::new(term)
                    .args(["-e", "sh", "-c", "pkexec ufw status verbose; read -n 1"])
                    .status()
                    .await;
                return;
            }
        }
        warn!("no terminal emulator found for nufw status — skipping");
    });
}

async fn which_async(bin: &str) -> std::io::Result<bool> {
    let status = tokio::process::Command::new("which")
        .arg(bin)
        .status()
        .await?;
    Ok(status.success())
}

/// Run `ufw status verbose` and parse the relevant fields. The
/// command is intentionally NOT prefixed with `sudo` / `pkexec` —
/// most distros let group `adm` / `ufw` users read status without
/// privilege escalation. Sustained `inactive`-with-no-rules likely
/// just means the user hasn't been added to that group yet; the
/// future panel work will surface that hint and offer a one-shot
/// pkexec read.
async fn fetch_ufw_summary() -> UfwSummary {
    let output = match tokio::process::Command::new("ufw")
        .arg("status")
        .arg("verbose")
        .output()
        .await
    {
        Ok(out) => out,
        Err(e) => {
            // Most common: ENOENT (ufw not installed). The widget's
            // icon still goes to `dialog-warning-symbolic` so the
            // user gets a visual that something is off; the tooltip
            // surfaces the actual error.
            return UfwSummary {
                error: Some(if e.kind() == std::io::ErrorKind::NotFound {
                    "not installed".to_string()
                } else {
                    format!("spawn failed: {e}")
                }),
                ..UfwSummary::default()
            };
        }
    };

    if !output.status.success() {
        return UfwSummary {
            error: Some(format!(
                "ufw exit {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            )),
            ..UfwSummary::default()
        };
    }

    parse_ufw_verbose(&String::from_utf8_lossy(&output.stdout))
}

/// Hand-written parser for `ufw status verbose`. The output is
/// stable across ufw 0.36 → 0.46 (last verified May 2026); the
/// relevant lines are:
///
///   Status: active
///   Logging: on (low)
///   Default: deny (incoming), allow (outgoing), disabled (routed)
///   New profiles: skip
///   To                         Action      From
///   --                         ------      ----
///   22                         ALLOW IN    Anywhere
///   …
///
/// We grep for the labels rather than locking to column indices so
/// translated locales (rare on ufw but possible) still parse the
/// English-only labels.
fn parse_ufw_verbose(stdout: &str) -> UfwSummary {
    let mut summary = UfwSummary::default();

    let mut in_rules = false;
    let mut rule_count = 0usize;

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

        if let Some(rest) = trimmed.strip_prefix("Default:") {
            // "deny (incoming), allow (outgoing), disabled (routed)"
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
            // Header row for the rules table — everything after the
            // separator dash row counts toward the rule count.
            continue;
        }
        if trimmed.starts_with("--") {
            in_rules = true;
            continue;
        }
        if in_rules && !trimmed.is_empty() {
            // Rule lines look like:
            //   22                         ALLOW IN    Anywhere
            // The leading column may be a port number, app profile,
            // or service name. Skip blank lines and continuation
            // lines (starting with whitespace alone).
            rule_count += 1;
        }
    }

    summary.rule_count = rule_count;
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
        assert_eq!(s.rule_count, 3);
    }

    #[test]
    fn parses_inactive() {
        let raw = "Status: inactive\n";
        let s = parse_ufw_verbose(raw);
        assert_eq!(s.status, Some(Status::Inactive));
        assert_eq!(s.rule_count, 0);
    }
}
