//! Dashboard "Overview Intelligence" tile — top-of-left-column
//! summary that reads what's happening right now and surfaces
//! the few things worth knowing without making the user open
//! anything else.
//!
//! The widget pulls from data sources mshell already wires:
//!   - notification_service().notifications — pending count
//!   - battery_service().device — % + state
//!   - hwmon temp1_input — package CPU temperature
//!   - local OffsetDateTime — today's weekday + month-day
//!
//! Each piece of intel surfaces as a single line in a vertical
//! stack. When there's nothing urgent the widget collapses to a
//! quiet "Nothing urgent" message + the date header so the card
//! still has visual weight at the top of the column.
//!
//! Severity colouring matches the cpu-dashboard / system-status
//! tiles: warn (primary tint) for moderate signals, danger (error
//! red) for the actionable ones.

// relm4's `view!` binds `model` by value in these property closures,
// so the `&model` passed to the `has_any_alert` / `quiet_summary`
// helpers is required — clippy's needless_borrow fires as a false
// positive here (dropping the `&` fails to compile).
#![allow(clippy::needless_borrow)]

use mshell_common::scoped_effects::EffectScope;
use mshell_services::{battery_service, notification_service};
use mshell_utils::battery::spawn_battery_watcher;
use mshell_utils::notifications::spawn_notifications_watcher;
use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::path::PathBuf;
use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_secs(5);

// Severity thresholds — kept high so the panel reads calm by
// default and only fires the alert ladder when something is
// genuinely worth a look.
const BATTERY_LOW_PERCENT: i32 = 25;
const BATTERY_CRITICAL_PERCENT: i32 = 10;
const TEMP_WARN_CELSIUS: i32 = 80;
const TEMP_DANGER_CELSIUS: i32 = 90;

pub(crate) struct OverviewIntelModel {
    notification_count: usize,
    battery_percent: i32,
    battery_charging: bool,
    has_battery: bool,
    temp_celsius: i32,
    temp_sensor_path: Option<PathBuf>,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum OverviewIntelInput {
    Refresh,
    Tick,
}

#[derive(Debug)]
pub(crate) enum OverviewIntelOutput {}

pub(crate) struct OverviewIntelInit {}

#[derive(Debug)]
pub(crate) enum OverviewIntelCommandOutput {
    NotificationsChanged,
    BatteryChanged,
}

#[relm4::component(pub)]
impl Component for OverviewIntelModel {
    type CommandOutput = OverviewIntelCommandOutput;
    type Input = OverviewIntelInput;
    type Output = OverviewIntelOutput;
    type Init = OverviewIntelInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "overview-intel-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_hexpand: true,
            set_spacing: 4,

            // Date header dropped — the Clock hero sitting above
            // already shows weekday + month-day, so repeating it
            // here was duplicating compositor chrome. Bullets now
            // start at the top of the card.

            // ── Notifications line ──────────────────────────────
            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 8,
                set_halign: gtk::Align::Start,
                #[watch]
                set_visible: model.notification_count > 0,

                gtk::Label {
                    add_css_class: "overview-intel-dot",
                    set_label: "●",
                },
                gtk::Label {
                    #[watch]
                    set_css_classes: &[
                        "overview-intel-line",
                        if model.notification_count > 0 { "warn" } else { "calm" },
                    ],
                    #[watch]
                    set_label: &notification_line(model.notification_count),
                    set_halign: gtk::Align::Start,
                },
            },

            // ── Battery line ────────────────────────────────────
            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 8,
                set_halign: gtk::Align::Start,
                #[watch]
                set_visible: model.has_battery
                    && !model.battery_charging
                    && model.battery_percent <= BATTERY_LOW_PERCENT,

                gtk::Label {
                    add_css_class: "overview-intel-dot",
                    set_label: "●",
                },
                gtk::Label {
                    #[watch]
                    set_css_classes: &[
                        "overview-intel-line",
                        battery_severity(model.battery_percent, model.battery_charging),
                    ],
                    #[watch]
                    set_label: &format!(
                        "Battery {}% — {}",
                        model.battery_percent,
                        if model.battery_percent <= BATTERY_CRITICAL_PERCENT {
                            "plug in soon"
                        } else {
                            "running low"
                        }
                    ),
                    set_halign: gtk::Align::Start,
                },
            },

            // ── Temperature line ────────────────────────────────
            //
            // Always visible when a sensor exists (user asked for
            // CPU temp to be a permanent readout, not just a hot
            // alert). Severity colour + wording escalate as it
            // climbs: calm "CPU NN°C" → warn/danger "CPU running
            // hot (NN°C)".
            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 8,
                set_halign: gtk::Align::Start,
                #[watch]
                set_visible: model.temp_sensor_path.is_some(),

                gtk::Label {
                    add_css_class: "overview-intel-dot",
                    set_label: "●",
                },
                gtk::Label {
                    #[watch]
                    set_css_classes: &[
                        "overview-intel-line",
                        temp_severity(model.temp_celsius),
                    ],
                    #[watch]
                    set_label: &temp_line(model.temp_celsius),
                    set_halign: gtk::Align::Start,
                },
            },

            // ── Quiet fallback ─────────────────────────────────
            //
            // Shown only when every alert above is hidden — keeps
            // the card from collapsing to an empty rectangle on
            // a quiet system.
            gtk::Label {
                add_css_class: "overview-intel-quiet",
                #[watch]
                set_visible: !has_any_alert(&model),
                #[watch]
                set_label: &quiet_summary(&model),
                set_halign: gtk::Align::Start,
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        spawn_notifications_watcher(&sender, || {
            OverviewIntelCommandOutput::NotificationsChanged
        });
        spawn_battery_watcher(&sender, || OverviewIntelCommandOutput::BatteryChanged);

        // 5 s tick for temperature + date refresh (date crosses
        // midnight; temp is the only thing we don't subscribe to
        // reactively).
        let sender_clone = sender.clone();
        relm4::gtk::glib::timeout_add_local(POLL_INTERVAL, move || {
            if sender_clone
                .input_sender()
                .send(OverviewIntelInput::Tick)
                .is_err()
            {
                return relm4::gtk::glib::ControlFlow::Break;
            }
            relm4::gtk::glib::ControlFlow::Continue
        });

        let battery = battery_service().device.clone();
        let has_battery = battery.is_present.get();
        let battery_percent = battery.percentage.get().round().clamp(0.0, 100.0) as i32;
        let battery_charging = current_battery_charging();
        let notification_count = notification_service().notifications.get().len();
        let temp_sensor_path = find_cpu_temp_sensor();
        let temp_celsius = temp_sensor_path
            .as_ref()
            .and_then(read_temp_millideg)
            .map(|t| t / 1000)
            .unwrap_or(0);

        let model = OverviewIntelModel {
            notification_count,
            battery_percent,
            battery_charging,
            has_battery,
            temp_celsius,
            temp_sensor_path,
            _effects: EffectScope::new(),
        };

        let widgets = view_output!();

        let _ = root;
        ComponentParts { model, widgets }
    }

    fn update_cmd(
        &mut self,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            OverviewIntelCommandOutput::NotificationsChanged
            | OverviewIntelCommandOutput::BatteryChanged => {
                sender.input(OverviewIntelInput::Refresh);
            }
        }
    }

    fn update(
        &mut self,
        message: Self::Input,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            OverviewIntelInput::Refresh => {
                let battery = battery_service().device.clone();
                self.has_battery = battery.is_present.get();
                self.battery_percent =
                    battery.percentage.get().round().clamp(0.0, 100.0) as i32;
                self.battery_charging = current_battery_charging();
                self.notification_count = notification_service().notifications.get().len();
            }
            OverviewIntelInput::Tick => {
                if let Some(p) = &self.temp_sensor_path
                    && let Some(t) = read_temp_millideg(p)
                {
                    self.temp_celsius = t / 1000;
                }
            }
        }
    }
}

// ── View helpers ────────────────────────────────────────────────

fn notification_line(count: usize) -> String {
    match count {
        0 => "No new notifications".to_string(),
        1 => "1 notification waiting".to_string(),
        n => format!("{n} notifications waiting"),
    }
}

fn battery_severity(percent: i32, charging: bool) -> &'static str {
    if charging {
        "calm"
    } else if percent <= BATTERY_CRITICAL_PERCENT {
        "danger"
    } else if percent <= BATTERY_LOW_PERCENT {
        "warn"
    } else {
        "calm"
    }
}

fn temp_line(celsius: i32) -> String {
    if celsius >= TEMP_WARN_CELSIUS {
        format!("CPU running hot ({celsius}°C)")
    } else {
        format!("CPU temperature {celsius}°C")
    }
}

fn temp_severity(celsius: i32) -> &'static str {
    if celsius >= TEMP_DANGER_CELSIUS {
        "danger"
    } else if celsius >= TEMP_WARN_CELSIUS {
        "warn"
    } else {
        "calm"
    }
}

fn has_any_alert(model: &OverviewIntelModel) -> bool {
    model.notification_count > 0
        || (model.has_battery
            && !model.battery_charging
            && model.battery_percent <= BATTERY_LOW_PERCENT)
        || (model.temp_sensor_path.is_some() && model.temp_celsius >= TEMP_WARN_CELSIUS)
}

fn quiet_summary(model: &OverviewIntelModel) -> String {
    // Temp lives on its own always-visible line now, so it's
    // intentionally omitted here to avoid showing the °C twice.
    let mut parts: Vec<String> = vec!["Quiet — nothing urgent".to_string()];
    if model.has_battery {
        let charge_word = if model.battery_charging { "charging" } else { "battery" };
        parts.push(format!("{}% {}", model.battery_percent, charge_word));
    }
    parts.join(" · ")
}

// ── Battery state helpers ──────────────────────────────────────
//
// Inlined rather than taking the Arc<BatteryDevice> type because
// wayle's surface exposes the device fields directly but not the
// named struct — keeping the call sites short with a fresh read.

fn current_battery_charging() -> bool {
    use mshell_services::{battery_service, line_power_service};
    use wayle_battery::types::DeviceState;
    let state = battery_service().device.state.get();
    line_power_service()
        .map(|s| s.device.online.get())
        .unwrap_or(state == DeviceState::Charging || state == DeviceState::FullyCharged)
}

// ── Temperature reading (same as system_status.rs) ─────────────

fn find_cpu_temp_sensor() -> Option<PathBuf> {
    let hwmon_dir = std::fs::read_dir("/sys/class/hwmon").ok()?;
    let mut k10temp: Option<PathBuf> = None;
    let mut coretemp: Option<PathBuf> = None;
    let mut acpitz: Option<PathBuf> = None;
    let mut other: Option<PathBuf> = None;
    for entry in hwmon_dir.flatten() {
        let p = entry.path();
        let Ok(name) = std::fs::read_to_string(p.join("name")) else {
            continue;
        };
        let name = name.trim();
        let temp_path = p.join("temp1_input");
        if !temp_path.exists() {
            continue;
        }
        match name {
            "k10temp" => k10temp.get_or_insert(temp_path),
            "coretemp" => coretemp.get_or_insert(temp_path),
            "acpitz" => acpitz.get_or_insert(temp_path),
            _ => other.get_or_insert(temp_path),
        };
    }
    k10temp.or(coretemp).or(acpitz).or(other)
}

fn read_temp_millideg(p: &PathBuf) -> Option<i32> {
    std::fs::read_to_string(p).ok()?.trim().parse().ok()
}
