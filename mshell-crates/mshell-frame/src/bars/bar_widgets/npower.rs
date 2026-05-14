//! Power profile bar pill — port of the noctalia `npower`
//! plugin's bar half.
//!
//! Render-only widget. Polls `powerprofilesctl` + the battery
//! sysfs nodes every 8 s and draws a profile icon + tooltip.
//! Click emits `NpowerOutput::Clicked`; frame toggles
//! `MenuType::Npower`.
//!
//! The system already ships a `PowerProfile` bar widget backed
//! by `power-profiles-daemon` over D-Bus, but it's icon-only with
//! no panel. This is the richer port: a profile switcher panel
//! plus battery / power-source readout, with the bar pill
//! colour-coded — performance = red, balanced = neutral,
//! power-saver = green.

use relm4::gtk::prelude::{ButtonExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::Duration;

// Short poll so the pill tracks profile switches (made from the
// menu, `powerprofilesctl`, or anything else) without a visible
// lag. The probe is cheap — one subprocess + a few sysfs reads.
const REFRESH_INTERVAL: Duration = Duration::from_secs(2);
const STARTUP_DELAY: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Profile {
    PowerSaver,
    Balanced,
    Performance,
    Unknown,
}

impl Profile {
    pub(crate) fn from_ppd(s: &str) -> Self {
        match s.trim() {
            "power-saver" => Profile::PowerSaver,
            "balanced" => Profile::Balanced,
            "performance" => Profile::Performance,
            _ => Profile::Unknown,
        }
    }

    /// The `powerprofilesctl set <id>` argument.
    pub(crate) fn ppd_id(self) -> &'static str {
        match self {
            Profile::PowerSaver => "power-saver",
            Profile::Balanced => "balanced",
            Profile::Performance => "performance",
            Profile::Unknown => "balanced",
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Profile::PowerSaver => "Power Saver",
            Profile::Balanced => "Balanced",
            Profile::Performance => "Performance",
            Profile::Unknown => "Unknown",
        }
    }

    pub(crate) fn icon(self) -> &'static str {
        match self {
            Profile::PowerSaver => "power-profile-power-saver-symbolic",
            Profile::Balanced => "power-profile-balanced-symbolic",
            Profile::Performance => "power-profile-performance-symbolic",
            Profile::Unknown => "power-profile-balanced-symbolic",
        }
    }

    /// CSS state class on the bar pill / menu button:
    /// performance → red, power-saver → green, balanced → neutral.
    pub(crate) fn css_class(self) -> &'static str {
        match self {
            Profile::PowerSaver => "profile-saver",
            Profile::Balanced => "profile-balanced",
            Profile::Performance => "profile-performance",
            Profile::Unknown => "profile-unknown",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PowerState {
    pub(crate) profile: Option<Profile>,
    /// "ac" / "battery" / "unknown".
    pub(crate) power_source: String,
    pub(crate) battery_available: bool,
    /// 0..=100, or None when no battery.
    pub(crate) battery_percent: Option<u8>,
    /// "Charging" / "Discharging" / "Full" / "Not charging" …
    pub(crate) battery_status: String,
    pub(crate) error: Option<String>,
}

#[derive(Debug)]
pub(crate) struct NpowerModel {
    state: PowerState,
}

#[derive(Debug)]
pub(crate) enum NpowerInput {
    Clicked,
}

#[derive(Debug)]
pub(crate) enum NpowerOutput {
    Clicked,
}

pub(crate) struct NpowerInit {}

#[derive(Debug)]
pub(crate) enum NpowerCommandOutput {
    Refreshed(PowerState),
}

#[relm4::component(pub)]
impl Component for NpowerModel {
    type CommandOutput = NpowerCommandOutput;
    type Input = NpowerInput;
    type Output = NpowerOutput;
    type Init = NpowerInit;

    view! {
        #[root]
        gtk::Box {
            set_css_classes: &["ok-button-surface", "ok-bar-widget", "npower-bar-widget"],
            set_hexpand: false,
            set_vexpand: false,
            set_has_tooltip: true,

            #[name="button"]
            gtk::Button {
                set_css_classes: &["ok-button-flat"],
                set_hexpand: true,
                set_vexpand: true,
                connect_clicked[sender] => move |_| {
                    sender.input(NpowerInput::Clicked);
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
                    let s = probe_power_state().await;
                    let _ = out.send(NpowerCommandOutput::Refreshed(s));
                }
            }
        });

        let model = NpowerModel {
            state: PowerState::default(),
        };
        let widgets = view_output!();
        apply_visual(&widgets.image, &root, &model.state);
        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            NpowerInput::Clicked => {
                let _ = sender.output(NpowerOutput::Clicked);
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
            NpowerCommandOutput::Refreshed(state) => {
                if self.state != state {
                    self.state = state;
                    apply_visual(&widgets.image, root, &self.state);
                }
            }
        }
    }
}

fn apply_visual(image: &gtk::Image, root: &gtk::Box, s: &PowerState) {
    let profile = s.profile.unwrap_or(Profile::Unknown);
    image.set_icon_name(Some(profile.icon()));

    let tooltip = if let Some(err) = &s.error {
        format!("Power: {err}")
    } else {
        let mut lines = vec![format!("Profile: {}", profile.label())];
        if s.battery_available {
            if let Some(pct) = s.battery_percent {
                lines.push(format!(
                    "Battery: {}% ({})",
                    pct,
                    if s.battery_status.is_empty() {
                        "unknown"
                    } else {
                        &s.battery_status
                    }
                ));
            }
        }
        lines.push(format!(
            "Power source: {}",
            match s.power_source.as_str() {
                "ac" => "AC adapter",
                "battery" => "Battery",
                _ => "unknown",
            }
        ));
        lines.join("\n")
    };
    root.set_tooltip_text(Some(&tooltip));

    // Three-state colour: performance = red, power-saver =
    // green, balanced = neutral. Reset all before adding the
    // current so stale state doesn't pile up across refreshes.
    for c in ["profile-saver", "profile-balanced", "profile-performance", "profile-unknown"] {
        root.remove_css_class(c);
    }
    root.add_css_class(profile.css_class());
}

/// Probe `powerprofilesctl get` + the battery sysfs nodes.
/// Exposed pub(crate) so the menu widget reuses it after each
/// profile switch.
pub(crate) async fn probe_power_state() -> PowerState {
    let mut state = PowerState::default();

    match run_capture("powerprofilesctl", &["get"]).await {
        Some(out) => state.profile = Some(Profile::from_ppd(&out)),
        None => {
            // powerprofilesctl missing / daemon down. Battery
            // readout still works from sysfs, so don't bail —
            // just leave profile = None and note it.
            state.error = Some("power-profiles-daemon not available".to_string());
        }
    }

    // Battery + power-source from /sys/class/power_supply.
    if let Ok(mut entries) = tokio::fs::read_dir("/sys/class/power_supply").await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let base = entry.path();
            let kind = tokio::fs::read_to_string(base.join("type"))
                .await
                .unwrap_or_default();
            match kind.trim() {
                "Mains" => {
                    let online = tokio::fs::read_to_string(base.join("online"))
                        .await
                        .unwrap_or_default();
                    if online.trim() == "1" {
                        state.power_source = "ac".to_string();
                    }
                }
                "Battery" => {
                    state.battery_available = true;
                    if let Ok(cap) = tokio::fs::read_to_string(base.join("capacity")).await {
                        if let Ok(pct) = cap.trim().parse::<u8>() {
                            state.battery_percent = Some(pct.min(100));
                        }
                    }
                    if let Ok(st) = tokio::fs::read_to_string(base.join("status")).await {
                        state.battery_status = st.trim().to_string();
                    }
                }
                _ => {}
            }
        }
    }
    if state.power_source.is_empty() {
        state.power_source = if state.battery_available {
            "battery".to_string()
        } else {
            "unknown".to_string()
        };
    }

    state
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
