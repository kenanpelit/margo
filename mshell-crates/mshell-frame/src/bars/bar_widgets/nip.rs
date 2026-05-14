//! Public-IP bar widget — port of the `nip` noctalia plugin.
//!
//! Periodically (default 300 s) fetches the host's public IP from
//! ipinfo.io and exposes:
//!
//!   * an icon — `network-symbolic` on success, `network-error-
//!     symbolic` on failure, `content-loading-symbolic` while a
//!     request is in flight
//!   * a tooltip — current IP + city / region / country / org,
//!     refreshed in lockstep with each successful fetch
//!   * left-click — opens the ipinfo.io page for the IP in the
//!     user's default browser via `xdg-open`. The noctalia plugin
//!     shipped a richer side panel; that's deferred for now in
//!     favour of the bar-pill MVP — the browser open keeps parity
//!     for the "I want to see more" gesture without the GTK4
//!     port of the QML panel.
//!
//! Network calls go out through `curl(1)` rather than reqwest:
//!   * keeps the binary size down (no TLS stack pulled in)
//!   * matches the upstream `nip/scripts/state.sh` semantics 1:1
//!   * lets the user override timeouts / providers by editing
//!     `~/.cachy/.../nip/*.sh` later if they want
//!
//! The fetch task is owned by the widget — when the widget is
//! dropped the tokio task spawns die naturally (the JoinHandle is
//! held in `_handle`).

use relm4::gtk::prelude::{ButtonExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::Duration;
use tracing::warn;

/// 5-minute default refresh cadence matches the upstream nip
/// plugin's `defaultSettings.refreshInterval` (300 s).
const REFRESH_INTERVAL: Duration = Duration::from_secs(300);

/// Initial fetch fires this fast on widget start so the bar isn't
/// stuck at "Loading…" for 5 minutes after login.
const STARTUP_DELAY: Duration = Duration::from_secs(2);

#[derive(Debug)]
pub(crate) struct NipModel {
    state: FetchState,
    info: Option<IpInfo>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct IpInfo {
    ip: String,
    city: String,
    region: String,
    country: String,
    org: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FetchState {
    Loading,
    Ok,
    Err,
}

#[derive(Debug)]
pub(crate) enum NipInput {
    /// Left-click → open the ipinfo.io page for the current IP in
    /// the system browser. No-op when state is Err and no info has
    /// been fetched yet.
    Clicked,
}

#[derive(Debug)]
pub(crate) enum NipOutput {}

pub(crate) struct NipInit {}

#[derive(Debug)]
pub(crate) enum NipCommandOutput {
    Started,
    Fetched(IpInfo),
    Failed(String),
}

#[relm4::component(pub)]
impl Component for NipModel {
    type CommandOutput = NipCommandOutput;
    type Input = NipInput;
    type Output = NipOutput;
    type Init = NipInit;

    view! {
        #[root]
        gtk::Box {
            set_css_classes: &["ok-button-surface", "ok-bar-widget", "nip-bar-widget"],
            set_hexpand: false,
            set_vexpand: false,
            set_has_tooltip: true,

            #[name="button"]
            gtk::Button {
                set_css_classes: &["ok-button-flat"],
                set_hexpand: true,
                set_vexpand: true,
                connect_clicked[sender] => move |_| {
                    sender.input(NipInput::Clicked);
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
        // Periodic fetch loop. `sender.command` ties the task's
        // lifetime to the component, so the loop exits when the
        // widget is unbuilt (config reload, bar reorder).
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
                    let _ = out.send(NipCommandOutput::Started);
                    match fetch_ipinfo().await {
                        Ok(info) => {
                            let _ = out.send(NipCommandOutput::Fetched(info));
                        }
                        Err(e) => {
                            let _ = out.send(NipCommandOutput::Failed(e));
                        }
                    }
                }
            }
        });

        let model = NipModel {
            state: FetchState::Loading,
            info: None,
        };

        let widgets = view_output!();

        apply_visual(&widgets.image, &root, model.state, model.info.as_ref());

        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        message: Self::Input,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            NipInput::Clicked => {
                let url = match self.info.as_ref() {
                    Some(info) if !info.ip.is_empty() => format!("https://ipinfo.io/{}", info.ip),
                    _ => "https://ipinfo.io".to_string(),
                };
                tokio::spawn(async move {
                    let mut cmd = tokio::process::Command::new("xdg-open");
                    cmd.arg(&url);
                    if let Err(e) = cmd.status().await {
                        warn!(error = %e, url, "xdg-open spawn failed");
                    }
                });
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
            NipCommandOutput::Started => {
                self.state = FetchState::Loading;
            }
            NipCommandOutput::Fetched(info) => {
                self.state = FetchState::Ok;
                self.info = Some(info);
            }
            NipCommandOutput::Failed(msg) => {
                warn!(error = msg, "nip: public-IP fetch failed");
                self.state = FetchState::Err;
            }
        }
        apply_visual(&widgets.image, root, self.state, self.info.as_ref());
    }
}

fn apply_visual(image: &gtk::Image, root: &gtk::Box, state: FetchState, info: Option<&IpInfo>) {
    // Globe glyph for the success state — `network-wireless` (wifi
    // signal) read wrong for a "public IP" widget that has nothing
    // to do with the local radio. `globe-symbolic` ships in
    // Adwaita / Papirus / Tela / most modern symbolic themes;
    // `network-acquiring-symbolic` carries the standard refreshing
    // / spinning visual that GNOME's own connection panel uses,
    // which is more legible than the generic
    // `content-loading-symbolic` hourglass at bar size.
    let icon = match state {
        FetchState::Loading => "network-acquiring-symbolic",
        FetchState::Ok => "globe-symbolic",
        FetchState::Err => "network-no-route-symbolic",
    };
    image.set_icon_name(Some(icon));

    let tooltip = match (state, info) {
        (FetchState::Loading, None) => "Fetching public IP…".to_string(),
        (FetchState::Loading, Some(i)) => format!("Refreshing… ({})", i.ip),
        (FetchState::Ok, Some(i)) => {
            let mut parts = vec![format!("IP: {}", i.ip)];
            let loc: Vec<&str> = [&i.city, &i.region, &i.country]
                .iter()
                .filter(|s| !s.is_empty())
                .map(|s| s.as_str())
                .collect();
            if !loc.is_empty() {
                parts.push(loc.join(", "));
            }
            if !i.org.is_empty() {
                parts.push(i.org.clone());
            }
            parts.join("\n")
        }
        (FetchState::Ok, None) => "Public IP unavailable".to_string(),
        (FetchState::Err, Some(i)) => format!("Refresh failed (last: {})", i.ip),
        (FetchState::Err, None) => "Failed to fetch public IP".to_string(),
    };
    root.set_tooltip_text(Some(&tooltip));

    // .error class so a future SCSS hook can tint the icon red on
    // sustained failure without us needing to change the widget
    // root's primary class list (which the bar layout depends on).
    if matches!(state, FetchState::Err) {
        root.add_css_class("error");
    } else {
        root.remove_css_class("error");
    }
}

/// Fire one ipinfo.io fetch through `curl`. ipinfo's free tier
/// returns a flat JSON with `ip`, `city`, `region`, `country`,
/// `org`; no auth required up to ~1000 requests per day, well under
/// our once-every-5-minutes cadence (288 / day).
async fn fetch_ipinfo() -> Result<IpInfo, String> {
    let output = tokio::process::Command::new("curl")
        .args([
            "-fsS",
            "--connect-timeout",
            "5",
            "--max-time",
            "10",
            "-H",
            "Accept: application/json",
            "https://ipinfo.io",
        ])
        .output()
        .await
        .map_err(|e| format!("curl spawn: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "curl exit {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let raw = std::str::from_utf8(&output.stdout)
        .map_err(|e| format!("curl produced non-UTF8 output: {e}"))?;
    let json: serde_json::Value =
        serde_json::from_str(raw).map_err(|e| format!("invalid JSON: {e}"))?;

    let s = |k: &str| -> String {
        json.get(k)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };

    Ok(IpInfo {
        ip: s("ip"),
        city: s("city"),
        region: s("region"),
        country: s("country"),
        org: s("org"),
    })
}
