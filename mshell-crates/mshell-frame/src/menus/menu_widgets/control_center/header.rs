//! Control Center header — avatar + username + uptime + action icons.
//!
//! Layout (horizontal `control-center-header`):
//!
//! ```text
//! [avatar]  username          ← hexpand spacer →  [🔒][⏻][🎛][✏]
//!           up 3h 5m
//! ```

use mshell_services::{battery_service, line_power_service};
use mshell_session::session_lock::lock_session;
use mshell_settings::open_settings;
use mshell_utils::battery::{get_battery_icon, get_charging_battery_icon};
use relm4::gtk::prelude::{BoxExt, ButtonExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::path::PathBuf;
use wayle_battery::types::DeviceState;

// ── Uptime helpers ────────────────────────────────────────────────────────────

/// Format an uptime expressed in seconds.
///
/// - < 1 hour  → `"up Nm"`
/// - < 1 day   → `"up Nh Nm"`
/// - ≥ 1 day   → `"up Nd Nh"`
pub(crate) fn fmt_uptime(secs: u64) -> String {
    let d = secs / 86_400;
    let h = (secs % 86_400) / 3_600;
    let m = (secs % 3_600) / 60;
    if d > 0 {
        format!("up {d}d {h}h")
    } else if h > 0 {
        format!("up {h}h {m}m")
    } else {
        format!("up {m}m")
    }
}

/// Read `/proc/uptime` → seconds (first whitespace-separated float).
pub(crate) fn read_uptime_secs() -> u64 {
    std::fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|s| s.split_whitespace().next().map(|x| x.to_string()))
        .and_then(|x| x.parse::<f64>().ok())
        .map(|f| f as u64)
        .unwrap_or(0)
}

// ── Battery chip ──────────────────────────────────────────────────────────────

/// Snapshot for the header battery chip: `(present, "82%", icon-name)`.
/// `present == false` hides the chip (desktops without a battery).
fn read_battery_chip() -> (bool, String, String) {
    let dev = &battery_service().device;
    if !dev.is_present.get() {
        return (false, String::new(), String::new());
    }
    let percent = dev.percentage.get();
    let charging = matches!(
        dev.state.get(),
        DeviceState::Charging | DeviceState::FullyCharged
    ) || line_power_service()
        .map(|s| s.device.online.get())
        .unwrap_or(false);
    let icon = if charging {
        get_charging_battery_icon(percent)
    } else {
        get_battery_icon(percent)
    };
    (
        true,
        format!("{}%", percent.round().clamp(0.0, 100.0) as u8),
        icon.to_string(),
    )
}

// ── Avatar path resolution ────────────────────────────────────────────────────

/// Resolve the avatar for the current user.
///
/// Priority:
/// 1. `~/.face` (current-user picture, no privilege)
/// 2. `/var/lib/AccountsService/icons/<username>`
/// 3. `None` → caller falls back to `"avatar-default-symbolic"`.
fn resolve_avatar(username: &str) -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("HOME") {
        let face = PathBuf::from(home).join(".face");
        if face.exists() {
            return Some(face);
        }
    }
    let asvc = PathBuf::from(format!("/var/lib/AccountsService/icons/{username}"));
    asvc.exists().then_some(asvc)
}

// ── Component ─────────────────────────────────────────────────────────────────

pub(crate) struct ControlCenterHeaderModel {
    uptime: String,
    username: String,
    /// Battery chip state, refreshed on reveal alongside uptime.
    battery_present: bool,
    battery_percent: String,
    battery_icon: String,
    /// Edit-mode toggle state (inert until Task 6).
    pub(crate) edit_mode: bool,
}

#[derive(Debug)]
pub(crate) enum ControlCenterHeaderInput {
    /// Re-read `/proc/uptime` and refresh the label (called on reveal).
    RecomputeUptime,
    LockClicked,
    SessionPowerClicked,
    SettingsClicked,
    ToggleEditClicked,
}

#[derive(Debug)]
pub(crate) enum ControlCenterHeaderOutput {
    Lock,
    SessionPower,
    Settings,
    ToggleEdit,
}

pub(crate) struct ControlCenterHeaderInit {}

#[relm4::component(pub(crate))]
impl Component for ControlCenterHeaderModel {
    type CommandOutput = ();
    type Input = ControlCenterHeaderInput;
    type Output = ControlCenterHeaderOutput;
    type Init = ControlCenterHeaderInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "control-center-header",
            set_orientation: gtk::Orientation::Horizontal,
            set_spacing: 12,
            set_hexpand: true,
            set_valign: gtk::Align::Center,

            // ── Avatar ──
            gtk::Box {
                add_css_class: "control-center-avatar",
                set_overflow: gtk::Overflow::Hidden,
                set_width_request: 48,
                set_height_request: 48,
                set_valign: gtk::Align::Center,
                set_halign: gtk::Align::Center,

                #[name = "avatar_image"]
                gtk::Image {
                    set_pixel_size: 48,
                    set_valign: gtk::Align::Center,
                    set_halign: gtk::Align::Center,
                },
            },

            // ── Text column: username + uptime ──
            gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_valign: gtk::Align::Center,
                set_hexpand: true,
                set_spacing: 2,

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: &model.username,
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_ellipsize: gtk::pango::EllipsizeMode::End,
                    set_max_width_chars: 20,
                },

                #[name = "uptime_label"]
                gtk::Label {
                    add_css_class: "label-small",
                    add_css_class: "dim-label",
                    #[watch]
                    set_label: &model.uptime,
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                },
            },

            // ── Battery chip (hidden on batteryless machines) ──
            gtk::Box {
                add_css_class: "control-center-battery-chip",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 4,
                set_valign: gtk::Align::Center,
                #[watch]
                set_visible: model.battery_present,

                gtk::Image {
                    add_css_class: "control-center-battery-icon",
                    #[watch]
                    set_icon_name: Some(model.battery_icon.as_str()),
                },
                gtk::Label {
                    add_css_class: "control-center-battery-label",
                    #[watch]
                    set_label: &model.battery_percent,
                },
            },

            // ── Action buttons ──
            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 4,
                set_valign: gtk::Align::Center,
                set_halign: gtk::Align::End,

                // Lock
                gtk::Button {
                    add_css_class: "panel-action-btn",
                    set_valign: gtk::Align::Center,
                    set_icon_name: "system-lock-screen-symbolic",
                    set_tooltip_text: Some("Lock screen"),
                    connect_clicked[sender] => move |_| {
                        sender.input(ControlCenterHeaderInput::LockClicked);
                    },
                },

                // Session / power
                gtk::Button {
                    add_css_class: "panel-action-btn",
                    set_valign: gtk::Align::Center,
                    set_icon_name: "system-shutdown-symbolic",
                    set_tooltip_text: Some("Power / session"),
                    connect_clicked[sender] => move |_| {
                        sender.input(ControlCenterHeaderInput::SessionPowerClicked);
                    },
                },

                // Settings
                gtk::Button {
                    add_css_class: "panel-action-btn",
                    set_valign: gtk::Align::Center,
                    set_icon_name: "tune-symbolic",
                    set_tooltip_text: Some("Settings"),
                    connect_clicked[sender] => move |_| {
                        sender.input(ControlCenterHeaderInput::SettingsClicked);
                    },
                },

                // Edit (inert until Task 6)
                gtk::Button {
                    add_css_class: "panel-action-btn",
                    set_valign: gtk::Align::Center,
                    set_icon_name: "document-edit-symbolic",
                    set_tooltip_text: Some("Edit"),
                    connect_clicked[sender] => move |_| {
                        sender.input(ControlCenterHeaderInput::ToggleEditClicked);
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
        let _ = &sender; // suppress unused-variable warning; view_output! uses it
        let username = glib_user_name();
        let uptime = fmt_uptime(read_uptime_secs());

        let (battery_present, battery_percent, battery_icon) = read_battery_chip();

        let model = ControlCenterHeaderModel {
            uptime,
            username: username.clone(),
            battery_present,
            battery_percent,
            battery_icon,
            edit_mode: false,
        };

        let widgets = view_output!();

        // Wire up the avatar image after view_output! so we can set it from
        // the resolved path (the view macro doesn't support conditional file
        // loading inline).
        match resolve_avatar(&username) {
            Some(path) => widgets.avatar_image.set_from_file(Some(&path)),
            None => widgets
                .avatar_image
                .set_icon_name(Some("avatar-default-symbolic")),
        }

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            ControlCenterHeaderInput::RecomputeUptime => {
                self.uptime = fmt_uptime(read_uptime_secs());
                let (present, percent, icon) = read_battery_chip();
                self.battery_present = present;
                self.battery_percent = percent;
                self.battery_icon = icon;
            }
            ControlCenterHeaderInput::LockClicked => {
                // Invoke lock directly — same pattern as the lock quick-action.
                lock_session();
                let _ = sender.output(ControlCenterHeaderOutput::Lock);
            }
            ControlCenterHeaderInput::SessionPowerClicked => {
                let _ = sender.output(ControlCenterHeaderOutput::SessionPower);
            }
            ControlCenterHeaderInput::SettingsClicked => {
                // open_settings() toggles the Settings overlay; it already
                // hides the parent menu — no separate CloseMenu needed.
                open_settings();
                let _ = sender.output(ControlCenterHeaderOutput::Settings);
            }
            ControlCenterHeaderInput::ToggleEditClicked => {
                self.edit_mode = !self.edit_mode;
                let _ = sender.output(ControlCenterHeaderOutput::ToggleEdit);
            }
        }
    }
}

// ── Private helper ─────────────────────────────────────────────────────────

fn glib_user_name() -> String {
    relm4::gtk::glib::user_name().to_string_lossy().into_owned()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uptime_formats() {
        assert_eq!(fmt_uptime(0), "up 0m");
        assert_eq!(fmt_uptime(54 * 60), "up 54m");
        assert_eq!(fmt_uptime(3 * 3600 + 5 * 60), "up 3h 5m");
        assert_eq!(fmt_uptime(25 * 3600), "up 1d 1h");
    }
}
