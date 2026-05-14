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
    let icon = match s.status {
        Some(Status::Active) => "security-high-symbolic",
        Some(Status::Inactive) => "security-low-symbolic",
        _ => "dialog-warning-symbolic",
    };
    image.set_icon_name(Some(icon));

    let tooltip = if let Some(err) = &s.error {
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
        if matches!(s.status, Some(Status::Active)) {
            lines.push(format!("Rules: {}", s.rules.len()));
        }
        lines.join("\n")
    };
    root.set_tooltip_text(Some(&tooltip));

    if matches!(s.status, Some(Status::Inactive)) {
        root.add_css_class("inactive");
    } else {
        root.remove_css_class("inactive");
    }
}

pub(crate) fn status_word(s: Option<Status>) -> &'static str {
    match s {
        Some(Status::Active) => "active",
        Some(Status::Inactive) => "inactive",
        _ => "unknown",
    }
}

/// Run `ufw status verbose` and parse the result into a summary.
/// Exposed (pub(crate)) so the menu widget can re-trigger fetches
/// after an action without duplicating the parse path.
pub(crate) async fn fetch_ufw_summary() -> UfwSummary {
    let output = match tokio::process::Command::new("ufw")
        .arg("status")
        .arg("verbose")
        .output()
        .await
    {
        Ok(out) => out,
        Err(e) => {
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
