//! Dashboard "Overview Intelligence" tile — top-of-left-column
//! summary that reads what's happening right now and surfaces
//! the few things worth knowing without making the user open
//! anything else.
//!
//! The widget pulls from data sources mshell already wires:
//!   - notification_service().notifications — pending count
//!   - battery_service().device — % + state
//!   - system_update cache — pending package-update count (read-only;
//!     CPU temperature lives on the SystemStatus tile, not duplicated here)
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
use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_secs(5);

// Severity thresholds — kept high so the panel reads calm by
// default and only fires the alert ladder when something is
// genuinely worth a look.
const BATTERY_LOW_PERCENT: i32 = 25;
const BATTERY_CRITICAL_PERCENT: i32 = 10;

pub(crate) struct OverviewIntelModel {
    notification_count: usize,
    battery_percent: i32,
    battery_charging: bool,
    has_battery: bool,
    /// Pending package updates (cached count from the system-update
    /// probe; this widget never triggers a fresh probe).
    update_count: usize,
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

            // ── Updates line ────────────────────────────────────
            //
            // Pending package updates — an actionable "do something"
            // signal that nothing else in this column surfaces. (CPU
            // temperature used to live here, but the SystemStatus tile
            // below already shows it permanently, so this slot now
            // carries the update count instead of duplicating temp.)
            // Alert-style: shown only when updates are pending; the
            // cached count is read, never a fresh probe.
            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 8,
                set_halign: gtk::Align::Start,
                #[watch]
                set_visible: model.update_count > 0,

                gtk::Label {
                    add_css_class: "overview-intel-dot",
                    set_label: "●",
                },
                gtk::Label {
                    set_css_classes: &["overview-intel-line", "warn"],
                    #[watch]
                    set_label: &updates_line(model.update_count),
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

        // 5 s tick for the cached update count + date refresh (date
        // crosses midnight; the update count isn't a reactive store, so
        // we re-read its cache on the tick).
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
        let update_count = read_update_count();

        let model = OverviewIntelModel {
            notification_count,
            battery_percent,
            battery_charging,
            has_battery,
            update_count,
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
                self.update_count = read_update_count();
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

fn updates_line(count: usize) -> String {
    match count {
        1 => "1 update available".to_string(),
        n => format!("{n} updates available"),
    }
}

/// Pending package-update count from the system-update cache. Read-only —
/// the bar widget / update menu own the actual probe schedule, so this
/// never fires `checkupdates` / the AUR helper.
fn read_update_count() -> usize {
    crate::system_update::load_cache()
        .map(|(_checked_at, report)| report.total())
        .unwrap_or(0)
}

fn has_any_alert(model: &OverviewIntelModel) -> bool {
    model.notification_count > 0
        || (model.has_battery
            && !model.battery_charging
            && model.battery_percent <= BATTERY_LOW_PERCENT)
        || model.update_count > 0
}

fn quiet_summary(model: &OverviewIntelModel) -> String {
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

