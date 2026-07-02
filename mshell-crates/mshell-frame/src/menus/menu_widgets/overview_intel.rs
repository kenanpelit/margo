//! Dashboard "Overview Intelligence" tile — top-of-left-column
//! summary that reads what's happening right now and surfaces
//! the few things worth knowing without making the user open
//! anything else.
//!
//! The widget pulls from data sources mshell already wires:
//!   - notification_service().notifications — pending count
//!   - battery_service().device — % + state
//!   - pending package updates — the shared system-update cache when
//!     fresh, else a cheap repo-only `checkupdates` probe this widget
//!     runs itself (CPU temperature lives on the SystemStatus tile, so
//!     it's not duplicated here)
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

use crate::system_update::{self, ProbeConfig};
use mshell_common::scoped_effects::EffectScope;
use mshell_services::{battery_service, notification_service};
use mshell_utils::battery::spawn_battery_watcher;
use mshell_utils::notifications::spawn_notifications_watcher;
use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::cell::Cell;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_secs(5);

// Severity thresholds — kept high so the panel reads calm by
// default and only fires the alert ladder when something is
// genuinely worth a look.
const BATTERY_LOW_PERCENT: i32 = 25;
const BATTERY_CRITICAL_PERCENT: i32 = 10;

// Pending-update sourcing. Prefer a fresh shared cache (the full
// repo+AUR+flatpak report the System Update widget writes) when it's
// younger than this; otherwise run our own cheap repo-only probe
// (`checkupdates` — no sudo, no AUR helper) so the count still shows
// even when that widget isn't on the bar.
const CACHE_FRESH_SECS: u64 = 1800;
const OWN_PROBE_THROTTLE_SECS: u64 = 900;
/// Last time *this widget* ran its own repo probe (Unix secs), shared
/// across monitor instances so they don't all probe at once.
static LAST_OWN_PROBE: AtomicU64 = AtomicU64::new(0);

pub(crate) struct OverviewIntelModel {
    notification_count: usize,
    battery_percent: i32,
    battery_charging: bool,
    has_battery: bool,
    /// Pending package updates (full count from the shared cache when
    /// fresh, else our own repo-only count).
    update_count: usize,
    /// True once we've established a count at least once — gates the
    /// always-visible updates line so it doesn't flash "up to date"
    /// before the first probe lands.
    updates_known: bool,
    /// Whether this menu is currently revealed. The 5 s tick — which
    /// re-reads the update cache and can spawn a `checkupdates` probe —
    /// is skipped while the menu is closed (notifications/battery stay
    /// watcher-driven, so they keep updating regardless).
    revealed: Rc<Cell<bool>>,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum OverviewIntelInput {
    Refresh,
    Tick,
    /// A pending-update count came back (cache or our own probe).
    UpdateCountProbed(usize),
    ParentRevealChanged(bool),
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
            // Pending package updates — replaces the old CPU-temp line
            // (SystemStatus already shows temperature). A permanent
            // readout once a count is known: warn "N updates available"
            // when pending, calm "System is up to date" at zero — so the
            // count is always visible, not just on a hot/alert state.
            // Sourced from the shared cache (full count) when fresh, else
            // a cheap repo-only probe this widget runs itself.
            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 8,
                set_halign: gtk::Align::Start,
                #[watch]
                set_visible: model.updates_known,

                gtk::Label {
                    add_css_class: "overview-intel-dot",
                    set_label: "●",
                },
                gtk::Label {
                    #[watch]
                    set_css_classes: &[
                        "overview-intel-line",
                        if model.update_count > 0 { "warn" } else { "calm" },
                    ],
                    #[watch]
                    set_label: &updates_line(model.update_count),
                    set_halign: gtk::Align::Start,
                },
            },

            // ── Quiet fallback ─────────────────────────────────
            //
            // Shown only when every alert is hidden AND we don't yet have
            // an updates readout — keeps the card from collapsing to an
            // empty rectangle on a quiet system before the first probe.
            // Once `updates_known`, the always-visible "up to date" line
            // is the calm-state indicator, so this would be redundant.
            gtk::Label {
                add_css_class: "overview-intel-quiet",
                #[watch]
                set_visible: !has_any_alert(&model) && !model.updates_known,
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
        spawn_notifications_watcher(&sender, || OverviewIntelCommandOutput::NotificationsChanged);
        spawn_battery_watcher(&sender, || OverviewIntelCommandOutput::BatteryChanged);

        // 5 s tick for the cached update count + date refresh (date
        // crosses midnight; the update count isn't a reactive store, so
        // we re-read its cache on the tick).
        let revealed = Rc::new(Cell::new(true));
        let sender_clone = sender.clone();
        let revealed_timer = revealed.clone();
        relm4::gtk::glib::timeout_add_local(POLL_INTERVAL, move || {
            if !revealed_timer.get() {
                return relm4::gtk::glib::ControlFlow::Continue;
            }
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
        let notification_count = notification_service()
            .map(|s| s.notifications.get().len())
            .unwrap_or(0);

        let model = OverviewIntelModel {
            notification_count,
            battery_percent,
            battery_charging,
            has_battery,
            update_count: 0,
            updates_known: false,
            revealed,
            _effects: EffectScope::new(),
        };

        let widgets = view_output!();

        // Establish the update count — from the shared cache if fresh,
        // else a cheap repo-only probe.
        refresh_updates(&sender);

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

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            OverviewIntelInput::Refresh => {
                let battery = battery_service().device.clone();
                self.has_battery = battery.is_present.get();
                self.battery_percent = battery.percentage.get().round().clamp(0.0, 100.0) as i32;
                self.battery_charging = current_battery_charging();
                self.notification_count = notification_service()
                    .map(|s| s.notifications.get().len())
                    .unwrap_or(0);
            }
            OverviewIntelInput::Tick => {
                refresh_updates(&sender);
            }
            OverviewIntelInput::UpdateCountProbed(count) => {
                self.update_count = count;
                self.updates_known = true;
            }
            OverviewIntelInput::ParentRevealChanged(visible) => {
                self.revealed.set(visible);
                if visible {
                    sender.input(OverviewIntelInput::Tick);
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

fn updates_line(count: usize) -> String {
    match count {
        0 => "System is up to date".to_string(),
        1 => "1 update available".to_string(),
        n => format!("{n} updates available"),
    }
}

/// Refresh the pending-update count and feed it back as
/// `UpdateCountProbed`. Prefers the shared cache (the full
/// repo+AUR+flatpak report the System Update widget writes) when it's
/// fresh; otherwise runs our own repo-only probe — `checkupdates`, which
/// needs no sudo and never touches the AUR helper — globally throttled so
/// the per-monitor instances don't all probe at once. This keeps the
/// count alive even when the System Update bar widget isn't present (the
/// reason the line previously stayed empty).
fn refresh_updates(sender: &ComponentSender<OverviewIntelModel>) {
    if let Some((checked_at, report)) = system_update::load_cache()
        && system_update::now_secs().saturating_sub(checked_at) < CACHE_FRESH_SECS
    {
        sender.input(OverviewIntelInput::UpdateCountProbed(report.total()));
        return;
    }

    let now = system_update::now_secs();
    let last = LAST_OWN_PROBE.load(Ordering::Relaxed);
    if now.saturating_sub(last) < OWN_PROBE_THROTTLE_SECS {
        // Someone probed recently — keep the last known count.
        return;
    }
    LAST_OWN_PROBE.store(now, Ordering::Relaxed);

    let sender = sender.clone();
    relm4::spawn(async move {
        let report = system_update::probe(ProbeConfig {
            repo: true,
            aur: false,
            flatpak: false,
        })
        .await;
        sender.input(OverviewIntelInput::UpdateCountProbed(report.total()));
    });
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
        let charge_word = if model.battery_charging {
            "charging"
        } else {
            "battery"
        };
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
