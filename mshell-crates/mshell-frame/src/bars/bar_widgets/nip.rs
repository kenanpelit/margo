//! Public-IP bar pill — port of the noctalia `nip` plugin's bar
//! half.
//!
//! Render-only widget. Polls ipinfo.io every 300 s, draws a globe
//! icon + tooltip. Click emits `NipOutput::Clicked`; frame toggles
//! the layer-shell `MenuType::Nip` (the detail panel lives in
//! `menu_widgets/nip/nip_menu_widget.rs`).
//!
//! ipinfo.io's free tier returns a flat JSON — `ip`, `city`,
//! `region`, `country`, `org`, `loc`, `timezone` — no auth needed
//! up to ~1000 req/day, well under our 288/day cadence. Fetch goes
//! through `curl(1)` so we don't pull a TLS stack into the binary.

use relm4::gtk::prelude::{ButtonExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::Duration;

const REFRESH_INTERVAL: Duration = Duration::from_secs(300);
const STARTUP_DELAY: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FetchState {
    Loading,
    Ok,
    Err,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct IpInfo {
    pub(crate) ip: String,
    pub(crate) city: String,
    pub(crate) region: String,
    pub(crate) country: String,
    pub(crate) org: String,
    /// "lat,lon" string from ipinfo's `loc` field.
    pub(crate) loc: String,
    pub(crate) timezone: String,
}

/// What the bar poll loop produces — used by both the bar pill
/// and the menu widget (which runs its own poll).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NipSnapshot {
    pub(crate) state: FetchState,
    pub(crate) info: Option<IpInfo>,
    pub(crate) error: Option<String>,
}

impl Default for NipSnapshot {
    fn default() -> Self {
        Self {
            state: FetchState::Loading,
            info: None,
            error: None,
        }
    }
}

#[derive(Debug)]
pub(crate) struct NipModel {
    snapshot: NipSnapshot,
}

#[derive(Debug)]
pub(crate) enum NipInput {
    Clicked,
}

#[derive(Debug)]
pub(crate) enum NipOutput {
    Clicked,
}

pub(crate) struct NipInit {}

#[derive(Debug)]
pub(crate) enum NipCommandOutput {
    Refreshed(NipSnapshot),
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
                    let snap = fetch_snapshot().await;
                    let _ = out.send(NipCommandOutput::Refreshed(snap));
                }
            }
        });

        let model = NipModel {
            snapshot: NipSnapshot::default(),
        };
        let widgets = view_output!();
        apply_visual(&widgets.image, &root, &model.snapshot);
        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            NipInput::Clicked => {
                let _ = sender.output(NipOutput::Clicked);
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
            NipCommandOutput::Refreshed(snap) => {
                if self.snapshot != snap {
                    self.snapshot = snap;
                    apply_visual(&widgets.image, root, &self.snapshot);
                }
            }
        }
    }
}

fn apply_visual(image: &gtk::Image, root: &gtk::Box, snap: &NipSnapshot) {
    let icon = match snap.state {
        FetchState::Loading => "network-wired-acquiring-symbolic",
        FetchState::Ok => "globe-symbolic",
        FetchState::Err => "network-wired-disconnected-symbolic",
    };
    image.set_icon_name(Some(icon));

    let tooltip = match (snap.state, &snap.info) {
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
        (FetchState::Err, _) => snap
            .error
            .clone()
            .unwrap_or_else(|| "Failed to fetch public IP".to_string()),
    };
    root.set_tooltip_text(Some(&tooltip));

    root.remove_css_class("error");
    if matches!(snap.state, FetchState::Err) {
        root.add_css_class("error");
    }
}

/// One ipinfo.io fetch wrapped in a `NipSnapshot`. Exposed
/// pub(crate) so the menu widget can re-run it on demand.
pub(crate) async fn fetch_snapshot() -> NipSnapshot {
    match fetch_ipinfo().await {
        Ok(info) => NipSnapshot {
            state: FetchState::Ok,
            info: Some(info),
            error: None,
        },
        Err(e) => NipSnapshot {
            state: FetchState::Err,
            info: None,
            error: Some(e),
        },
    }
}

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
        loc: s("loc"),
        timezone: s("timezone"),
    })
}
