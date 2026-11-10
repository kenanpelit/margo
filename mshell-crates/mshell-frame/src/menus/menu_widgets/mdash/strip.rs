//! The status strip under mdash's header: one line of small chips —
//! battery, network, volume, weather now, unread notifications, pending
//! updates. A chip shows only when it has something to say (no battery
//! on a desktop, no "0 updates") and opens its menu on click.

use crate::frame::MenuId;
use relm4::gtk;
use relm4::gtk::prelude::{BoxExt, ButtonExt, WidgetExt};
use std::rc::Rc;

/// What the strip shows.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct StripState {
    /// Battery percent and whether it is charging; `None` = no battery.
    pub(crate) battery: Option<(u8, bool)>,
    /// Network label (SSID / "Wired") with its icon; `None` = offline.
    pub(crate) network: Option<(String, &'static str)>,
    /// A VPN tunnel is up.
    pub(crate) vpn: bool,
    /// Output volume percent and mute; `None` = no output device.
    pub(crate) volume: Option<(u8, bool)>,
    /// Weather now ("10.8°C") with its icon; `None` = no reading.
    pub(crate) weather: Option<(String, &'static str)>,
    pub(crate) unread: usize,
    pub(crate) updates: usize,
}

/// One chip on screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Chip {
    pub(crate) kind: ChipKind,
    pub(crate) icon: &'static str,
    pub(crate) label: String,
    /// The accessible name / tooltip: what a click opens.
    pub(crate) name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChipKind {
    Battery,
    Network,
    Volume,
    Weather,
    Notifications,
    Updates,
}

impl ChipKind {
    pub(crate) const ALL: [ChipKind; 6] = [
        ChipKind::Battery,
        ChipKind::Network,
        ChipKind::Volume,
        ChipKind::Weather,
        ChipKind::Notifications,
        ChipKind::Updates,
    ];

    pub(crate) fn menu(self) -> MenuId {
        match self {
            ChipKind::Battery => MenuId::Power,
            ChipKind::Network => MenuId::Network,
            ChipKind::Volume => MenuId::AudioDashboard,
            ChipKind::Weather => MenuId::Weather,
            ChipKind::Notifications => MenuId::Notification,
            ChipKind::Updates => MenuId::SystemUpdate,
        }
    }
}

fn battery_icon(percent: u8, charging: bool) -> &'static str {
    let f = f64::from(percent);
    if charging {
        mshell_utils::battery::get_charging_battery_icon(f)
    } else {
        mshell_utils::battery::get_battery_icon(f)
    }
}

/// The chips for `s`, in strip order; inapplicable ones are left out.
pub(crate) fn chips(s: &StripState) -> Vec<Chip> {
    let mut out = Vec::new();
    if let Some((pct, charging)) = s.battery {
        out.push(Chip {
            kind: ChipKind::Battery,
            icon: battery_icon(pct, charging),
            label: format!("{pct}%"),
            name: format!(
                "Battery {pct}%{} — open Power",
                if charging { ", charging" } else { "" }
            ),
        });
    }
    if let Some((label, icon)) = &s.network {
        let label = if s.vpn {
            format!("{label} · VPN")
        } else {
            label.clone()
        };
        out.push(Chip {
            kind: ChipKind::Network,
            icon,
            name: format!("{label} — open Network"),
            label,
        });
    }
    if let Some((pct, muted)) = s.volume {
        out.push(Chip {
            kind: ChipKind::Volume,
            icon: if muted {
                "audio-volume-muted-symbolic"
            } else {
                "audio-volume-high-symbolic"
            },
            label: if muted {
                "Muted".into()
            } else {
                format!("{pct}%")
            },
            name: format!("Volume {pct}% — open Audio"),
        });
    }
    if let Some((temp, icon)) = &s.weather {
        out.push(Chip {
            kind: ChipKind::Weather,
            icon,
            label: temp.clone(),
            name: format!("Weather {temp} — open Weather"),
        });
    }
    if s.unread > 0 {
        out.push(Chip {
            kind: ChipKind::Notifications,
            icon: "notification-alert-symbolic",
            label: format!("{} new", s.unread),
            name: format!(
                "{} new notification{} — open Notifications",
                s.unread,
                if s.unread == 1 { "" } else { "s" }
            ),
        });
    }
    if s.updates > 0 {
        out.push(Chip {
            kind: ChipKind::Updates,
            icon: "software-update-available-symbolic",
            label: format!(
                "{} update{}",
                s.updates,
                if s.updates == 1 { "" } else { "s" }
            ),
            name: format!("{} pending updates — open Updates", s.updates),
        });
    }
    out
}

/// The strip widget: one button per chip kind, shown / hidden / relabelled
/// in place (no rebuild per update).
pub(crate) struct Strip {
    pub(crate) root: gtk::Box,
    buttons: Vec<(ChipKind, gtk::Button, gtk::Image, gtk::Label)>,
}

impl Strip {
    pub(crate) fn new(on_open: impl Fn(MenuId) + 'static) -> Self {
        let on_open: Rc<dyn Fn(MenuId)> = Rc::new(on_open);
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        root.add_css_class("mdash-strip");
        let buttons = ChipKind::ALL
            .into_iter()
            .map(|kind| {
                let button = gtk::Button::new();
                button.add_css_class("mdash-chip");
                let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                let image = gtk::Image::new();
                let label = gtk::Label::new(None);
                label.set_ellipsize(gtk::pango::EllipsizeMode::End);
                label.set_max_width_chars(18);
                content.append(&image);
                content.append(&label);
                button.set_child(Some(&content));
                button.set_visible(false);
                let on_open = on_open.clone();
                button.connect_clicked(move |_| on_open(kind.menu()));
                root.append(&button);
                (kind, button, image, label)
            })
            .collect();
        Self { root, buttons }
    }

    pub(crate) fn apply(&self, state: &StripState) {
        let chips = chips(state);
        for (kind, button, image, label) in &self.buttons {
            match chips.iter().find(|c| c.kind == *kind) {
                Some(chip) => {
                    image.set_icon_name(Some(chip.icon));
                    label.set_label(&chip.label);
                    mshell_common::a11y::name(button, &chip.name);
                    button.set_tooltip_text(Some(&chip.name));
                    button.set_visible(true);
                }
                None => button.set_visible(false),
            }
        }
        self.root.set_visible(!chips.is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(s: &StripState) -> Vec<ChipKind> {
        chips(s).into_iter().map(|c| c.kind).collect()
    }

    #[test]
    fn chips_hide_when_not_applicable() {
        assert!(chips(&StripState::default()).is_empty());
        let desktop = StripState {
            network: Some(("Wired".into(), "network-wired-symbolic")),
            volume: Some((45, false)),
            ..StripState::default()
        };
        assert_eq!(kinds(&desktop), [ChipKind::Network, ChipKind::Volume]);
    }

    #[test]
    fn a_full_strip_in_order_with_counts() {
        let s = StripState {
            battery: Some((54, true)),
            network: Some(("Home-5G".into(), "network-wireless-symbolic")),
            vpn: true,
            volume: Some((45, true)),
            weather: Some(("10.8°C".into(), "weather-few-clouds-symbolic")),
            unread: 1,
            updates: 3,
        };
        let c = chips(&s);
        assert_eq!(c.iter().map(|c| c.kind).collect::<Vec<_>>(), ChipKind::ALL);
        assert_eq!(c[1].label, "Home-5G · VPN");
        assert_eq!(c[2].label, "Muted");
        assert_eq!(c[4].label, "1 new");
        assert!(c[4].name.starts_with("1 new notification —"));
        assert_eq!(c[5].label, "3 updates");
    }

    #[test]
    fn each_chip_opens_its_menu() {
        assert_eq!(ChipKind::Notifications.menu(), MenuId::Notification);
        assert_eq!(ChipKind::Updates.menu(), MenuId::SystemUpdate);
        assert_eq!(ChipKind::Battery.menu(), MenuId::Power);
    }
}
