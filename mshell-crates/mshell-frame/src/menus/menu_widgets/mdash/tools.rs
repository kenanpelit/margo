//! The **Tools** tab: a labelled launcher grid of the shell's menus. Only
//! menus that exist; opening one closes mdash and opens it in its own
//! place (the menu-launcher quick action's `OpenMenu` path).

use super::card::GAP;
use crate::frame::MenuId;
use relm4::gtk;
use relm4::gtk::prelude::{BoxExt, ButtonExt, GridExt, WidgetExt};
use std::rc::Rc;

/// Where a tool leads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolTarget {
    Menu(MenuId),
    /// The Settings panel (not a `MenuId`: it is built on demand).
    Settings,
}

/// One launcher: icon, label, target.
pub(crate) struct Tool {
    pub(crate) icon: &'static str,
    pub(crate) label: &'static str,
    pub(crate) target: ToolTarget,
}

const fn tool(icon: &'static str, label: &'static str, menu: MenuId) -> Tool {
    Tool {
        icon,
        label,
        target: ToolTarget::Menu(menu),
    }
}

/// Columns of the grid (five full rows of six).
pub(crate) const COLUMNS: usize = 6;

/// Every tool, in reading order: capture & write, assist, system,
/// network, developer & look.
pub(crate) const TOOLS: &[Tool] = &[
    tool("screenshot-area-symbolic", "Screenshot", MenuId::Screenshot),
    tool("color-select-symbolic", "Color Picker", MenuId::ColorPicker),
    tool("screenshot-qr-symbolic", "QR Code", MenuId::Qr),
    tool("edit-paste-symbolic", "Clipboard", MenuId::Clipboard),
    tool("notes-symbolic", "Notes", MenuId::Notes),
    tool("folder-download-symbolic", "Shelf", MenuId::Shelf),
    tool(
        "preferences-desktop-locale-symbolic",
        "Translate",
        MenuId::Translate,
    ),
    tool("starred-symbolic", "AI Chat", MenuId::Ai),
    tool("system-search-symbolic", "Launcher", MenuId::AppLauncher),
    tool("input-keyboard-symbolic", "Keybinds", MenuId::Keybinds),
    tool("alarm-symbolic", "Alarm Clock", MenuId::AlarmClock),
    tool("x-office-calendar-symbolic", "Clock", MenuId::Clock),
    Tool {
        icon: "settings-symbolic",
        label: "Settings",
        target: ToolTarget::Settings,
    },
    tool("cpu-symbolic", "Monitor", MenuId::CpuDashboard),
    tool(
        "audio-volume-high-symbolic",
        "Audio",
        MenuId::AudioDashboard,
    ),
    tool(
        "software-update-available-symbolic",
        "Updates",
        MenuId::SystemUpdate,
    ),
    tool("dialog-warning-symbolic", "Crashes", MenuId::Crashes),
    tool("camera-video-symbolic", "Privacy", MenuId::Privacy),
    tool("network-wireless-symbolic", "Network", MenuId::Network),
    tool("bluetooth-active-symbolic", "Bluetooth", MenuId::Bluetooth),
    tool("vpn-symbolic", "VPN", MenuId::Vpn),
    tool("globe-symbolic", "DNS", MenuId::Dns),
    tool("network-server-symbolic", "Public IP", MenuId::Ip),
    tool("phone-symbolic", "Phone", MenuId::Valent),
    tool("ai-agents-symbolic", "Agents", MenuId::Agents),
    tool("github-symbolic", "GitHub", MenuId::Github),
    tool("package-symbolic", "Podman", MenuId::Podman),
    tool("utilities-terminal-symbolic", "SSH", MenuId::SshSessions),
    tool("drive-removable-media-symbolic", "USB", MenuId::Usb),
    tool("wallpaper-symbolic", "Theme", MenuId::Wallpaper),
];

pub(crate) struct ToolsTab {
    pub(crate) root: gtk::Grid,
}

impl ToolsTab {
    pub(crate) fn new(open: impl Fn(ToolTarget) + 'static) -> Self {
        let open: Rc<dyn Fn(ToolTarget)> = Rc::new(open);
        let root = gtk::Grid::new();
        root.add_css_class("mdash-tools");
        // Six equal columns of compact card-buttons, one gap, natural
        // height: the grid never spreads to fill a taller tab.
        root.set_column_homogeneous(true);
        root.set_row_homogeneous(true);
        root.set_row_spacing(GAP.unsigned_abs());
        root.set_column_spacing(GAP.unsigned_abs());
        for (i, t) in TOOLS.iter().enumerate() {
            let button = gtk::Button::new();
            button.add_css_class("mdash-tool");
            let content = gtk::Box::new(gtk::Orientation::Vertical, 4);
            content.set_valign(gtk::Align::Center);
            let icon = gtk::Image::from_icon_name(t.icon);
            icon.add_css_class("mdash-tool-icon");
            content.append(&icon);
            let label = gtk::Label::new(Some(t.label));
            label.add_css_class("mdash-tool-label");
            // Body size; a long name wraps to a second line rather than
            // shrinking or being cut.
            label.set_wrap(true);
            label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            label.set_lines(2);
            label.set_ellipsize(gtk::pango::EllipsizeMode::End);
            label.set_justify(gtk::Justification::Center);
            content.append(&label);
            button.set_child(Some(&content));
            button.set_tooltip_text(Some(&format!("Open {}", t.label)));
            mshell_common::a11y::name(&button, &format!("Open {}", t.label));
            let (open, target) = (open.clone(), t.target);
            button.connect_clicked(move |_| open(target));
            let (col, row) = ((i % COLUMNS) as i32, (i / COLUMNS) as i32);
            root.attach(&button, col, row, 1, 1);
        }
        Self { root }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menus::menu_widgets::control_center::tiles::dash_layout::tile_for_quick_action;
    use mshell_config::schema::menu_widgets::QuickActionWidget;
    use std::collections::HashSet;

    #[test]
    fn the_grid_is_full_and_has_no_duplicates() {
        assert_eq!(TOOLS.len() % COLUMNS, 0, "the last row is short");
        let mut seen = HashSet::new();
        for t in TOOLS {
            assert!(seen.insert(format!("{:?}", t.target)), "{} twice", t.label);
            assert!(!t.label.is_empty() && t.icon.ends_with("-symbolic"));
        }
    }

    /// Every old quick action has a home on the new dashboard: a Controls
    /// tile (toggles), a Tools launcher (menus), or the header (power).
    #[test]
    fn every_quick_action_is_covered() {
        let in_tools = |m: MenuId| TOOLS.iter().any(|t| t.target == ToolTarget::Menu(m));
        for qa in QuickActionWidget::all() {
            use QuickActionWidget as Q;
            let covered = match qa {
                Q::Lock | Q::Logout | Q::Reboot | Q::Shutdown | Q::Session => true,
                Q::Settings => TOOLS.iter().any(|t| t.target == ToolTarget::Settings),
                Q::Wallpaper => in_tools(MenuId::Wallpaper),
                Q::Screenshot => in_tools(MenuId::Screenshot),
                Q::Network => in_tools(MenuId::Network),
                Q::Bluetooth => in_tools(MenuId::Bluetooth),
                Q::CpuDashboard => in_tools(MenuId::CpuDashboard),
                Q::AudioDashboard => in_tools(MenuId::AudioDashboard),
                Q::Vpn => in_tools(MenuId::Vpn),
                Q::Dns => in_tools(MenuId::Dns),
                Q::Ip => in_tools(MenuId::Ip),
                Q::AlarmClock => in_tools(MenuId::AlarmClock),
                Q::SystemUpdate => in_tools(MenuId::SystemUpdate),
                Q::Keybinds => in_tools(MenuId::Keybinds),
                // Twilight: its Controls tile's chevron; Power: the battery
                // chip and card; Control Center: its tiles ARE the
                // Controls tab.
                Q::Twilight | Q::Power | Q::ControlCenter => true,
                toggle => tile_for_quick_action(toggle).is_some(),
            };
            assert!(covered, "{qa:?} has no place on the dashboard");
        }
    }
}
