//! The settings sidebar: its route table, section groups, collapsed state
//! and the button tree.

use super::*;

/// One row of the sidebar: a group header or a page button.
pub(super) enum SidebarEntry {
    Section {
        name: &'static str,
        icon: &'static str,
        /// First-run reveal state: `true` opens this group collapsed until
        /// the user has touched *any* group (once the state file exists,
        /// the persisted per-group choice takes over). Keeps the initial
        /// list short by folding the rarely-visited groups away.
        collapsed: bool,
    },
    Page {
        route: &'static str,
        icon: &'static str,
        label: &'static str,
    },
}

use SidebarEntry::{Page, Section};

/// The sidebar layout in display order, grouped KDE-System-Settings style
/// by what the user is trying to change: how it looks, the desktop shell,
/// window management, input, hardware, network, applications, and the
/// session/system. Built imperatively (relm4's `view!` can't loop), so
/// adding a page is one row here + one stack page + search entries.
///
/// Section names are also the persisted collapse-state keys, and icons are
/// unique across the whole list (see `sidebar_tests`).
pub(super) const SIDEBAR: &[SidebarEntry] = &[
    Page {
        route: "general",
        icon: "settings-symbolic",
        label: "General",
    },
    Section {
        name: "Appearance",
        icon: "wallpaper-symbolic",
        collapsed: false,
    },
    Page {
        route: "theme",
        icon: "palette-symbolic",
        label: "Theme",
    },
    Page {
        route: "appearance",
        icon: "preferences-desktop-display-symbolic",
        label: "Window Style",
    },
    Page {
        route: "effects",
        icon: "applications-graphics-symbolic",
        label: "Effects",
    },
    Page {
        route: "animations",
        icon: "preferences-desktop-screensaver-symbolic",
        label: "Animations",
    },
    Page {
        route: "osd",
        icon: "notification-symbolic",
        label: "On-screen Displays",
    },
    Section {
        name: "Workspace",
        icon: "tray-symbolic",
        collapsed: false,
    },
    Page {
        route: "bar",
        icon: "sidebar-symbolic",
        label: "Bar",
    },
    Page {
        route: "menus",
        icon: "view-list-symbolic",
        label: "Menus",
    },
    Page {
        route: "widgets",
        icon: "view-app-grid-symbolic",
        label: "Widgets",
    },
    Page {
        route: "overview",
        icon: "view-grid-symbolic",
        label: "Overview",
    },
    Page {
        route: "window_switcher",
        icon: "view-more-horizontal-symbolic",
        label: "Window Switcher",
    },
    Page {
        route: "launcher",
        icon: "system-search-symbolic",
        label: "Launcher",
    },
    Page {
        route: "toasts",
        icon: "preferences-system-notifications-symbolic",
        label: "Toasts",
    },
    Section {
        name: "Window Management",
        icon: "layout-symbolic",
        collapsed: false,
    },
    Page {
        route: "tiling_layout",
        icon: "video-joined-displays-symbolic",
        label: "Tiling Layout",
    },
    Page {
        route: "summon",
        icon: "workspace-symbolic",
        label: "Tags",
    },
    Page {
        route: "behaviour",
        icon: "preferences-system-symbolic",
        label: "Behaviour",
    },
    Page {
        route: "window_rules",
        icon: "window-new-symbolic",
        label: "Window Rules",
    },
    Page {
        route: "layer_rules",
        icon: "view-paged-symbolic",
        label: "Layer Rules",
    },
    Page {
        route: "tag_rules",
        icon: "window-symbolic",
        label: "Tag Rules",
    },
    Section {
        name: "Input",
        icon: "input-tablet-symbolic",
        collapsed: false,
    },
    Page {
        route: "input",
        icon: "input-mouse-symbolic",
        label: "Input Devices",
    },
    Page {
        route: "keybinds",
        icon: "preferences-desktop-keyboard-shortcuts-symbolic",
        label: "Keyboard Shortcuts",
    },
    Page {
        route: "keyboard",
        icon: "input-keyboard-symbolic",
        label: "On-Screen Keyboard",
    },
    Section {
        name: "Hardware",
        icon: "cpu-symbolic",
        collapsed: false,
    },
    Page {
        route: "display",
        icon: "video-display-symbolic",
        label: "Display",
    },
    Page {
        route: "sound",
        icon: "audio-volume-high-symbolic",
        label: "Sound",
    },
    Page {
        route: "bluetooth",
        icon: "bluetooth-active-symbolic",
        label: "Bluetooth",
    },
    Page {
        route: "power",
        icon: "battery-symbolic",
        label: "Power",
    },
    Page {
        route: "game_mode",
        icon: "input-gaming-symbolic",
        label: "Game Mode",
    },
    Section {
        name: "Network",
        icon: "network-workgroup-symbolic",
        collapsed: false,
    },
    Page {
        route: "network",
        icon: "network-wireless-symbolic",
        label: "Network",
    },
    Page {
        route: "vpn",
        icon: "network-vpn-symbolic",
        label: "VPN",
    },
    Section {
        name: "Applications",
        icon: "package-symbolic",
        collapsed: true,
    },
    Page {
        route: "default_apps",
        icon: "application-x-executable-symbolic",
        label: "Default Apps",
    },
    Page {
        route: "ai",
        icon: "starred-symbolic",
        label: "AI",
    },
    Page {
        route: "translate",
        icon: "globe-symbolic",
        label: "Translate",
    },
    Page {
        route: "mtune",
        icon: "org.margo.Tune-symbolic",
        label: "Tune",
    },
    Page {
        route: "calendar",
        icon: "x-office-calendar-symbolic",
        label: "Calendar",
    },
    Page {
        route: "plugins",
        icon: "application-x-addon-symbolic",
        label: "Plugins",
    },
    Section {
        name: "System",
        icon: "computer-symbolic",
        collapsed: true,
    },
    Page {
        route: "lock",
        icon: "system-lock-screen-symbolic",
        label: "Lock Screen",
    },
    Page {
        route: "login",
        icon: "avatar-default-symbolic",
        label: "Login Screen",
    },
    Page {
        route: "idle",
        icon: "coffee-symbolic",
        label: "Idle",
    },
    Page {
        route: "privacy",
        icon: "security-high-symbolic",
        label: "Privacy",
    },
    Page {
        route: "users",
        icon: "system-users-symbolic",
        label: "Users",
    },
    Page {
        route: "region",
        icon: "preferences-desktop-locale-symbolic",
        label: "Region & Language",
    },
    Page {
        route: "date_time",
        icon: "preferences-system-time-symbolic",
        label: "Date & Time",
    },
    Page {
        route: "startup_env",
        icon: "system-run-symbolic",
        label: "Startup",
    },
    Page {
        route: "backup",
        icon: "document-save-symbolic",
        label: "Backup",
    },
    Page {
        route: "logging",
        icon: "text-x-generic-symbolic",
        label: "Logging",
    },
    Page {
        route: "setup",
        icon: "emblem-system-symbolic",
        label: "Setup",
    },
    Section {
        name: "Help",
        icon: "dialog-question-symbolic",
        collapsed: false,
    },
    Page {
        route: "guide",
        icon: "help-faq-symbolic",
        label: "Guide",
    },
    Page {
        route: "about",
        icon: "help-about-symbolic",
        label: "About",
    },
];

pub(super) const SETTINGS_SIDEBAR_LABEL_MIN_HEIGHT: i32 = 28;
pub(super) const SETTINGS_SIDEBAR_TITLE_MIN_HEIGHT: i32 = 28;
pub(super) const SETTINGS_SIDEBAR_TEXT_RISE: i32 = -1024;

pub(super) fn settings_sidebar_markup(label: &str) -> String {
    // Maple Mono / Nerd Font metrics can put ink right on the label's top
    // edge at certain hinted sizes. GTK clips label drawing to the widget box,
    // so a 1 device-pixel downward Pango rise gives the glyph ink
    // deterministic headroom independent of CSS/font rounding.
    format!(
        "<span rise=\"{}\">{}</span>",
        SETTINGS_SIDEBAR_TEXT_RISE,
        gtk::glib::markup_escape_text(label)
    )
}

pub(super) fn settings_sidebar_label(label: &str) -> gtk::Label {
    use gtk::prelude::*;

    let lbl = gtk::Label::new(None);
    lbl.set_markup(&settings_sidebar_markup(label));
    lbl.add_css_class("label-medium");
    lbl.add_css_class("settings-sidebar-label");
    lbl.set_halign(gtk::Align::Start);
    lbl.set_valign(gtk::Align::Center);
    lbl.set_xalign(0.0);
    lbl.set_yalign(0.5);
    lbl.set_hexpand(true);
    lbl.set_vexpand(false);
    lbl.set_wrap(false);
    lbl.set_single_line_mode(true);
    lbl.set_ellipsize(gtk::pango::EllipsizeMode::End);
    lbl.set_height_request(SETTINGS_SIDEBAR_LABEL_MIN_HEIGHT);
    lbl
}

pub(super) fn settings_sidebar_title_label(label: &str) -> gtk::Label {
    use gtk::prelude::*;

    let lbl = gtk::Label::new(None);
    lbl.set_markup(&settings_sidebar_markup(label));
    lbl.add_css_class("label-medium-bold");
    lbl.add_css_class("settings-sidebar-title");
    lbl.set_halign(gtk::Align::Start);
    lbl.set_valign(gtk::Align::Center);
    lbl.set_xalign(0.0);
    lbl.set_yalign(0.5);
    lbl.set_hexpand(true);
    lbl.set_vexpand(false);
    lbl.set_wrap(false);
    lbl.set_single_line_mode(true);
    lbl.set_ellipsize(gtk::pango::EllipsizeMode::End);
    lbl.set_height_request(SETTINGS_SIDEBAR_TITLE_MIN_HEIGHT);
    lbl
}

/// A sidebar group header: a leading symbolic icon + a friendly
/// Title-Case label ("Appearance", "Shell & Desktop", …). The 12px gap +
/// `space-3` left padding (in CSS) line the icon up with the page-button
/// icons below it, so the groups scan as one column. Non-interactive.
/// Cache file holding the set of *collapsed* sidebar section names
/// (comma-separated), so the accordion state survives reopening Settings.
pub(super) fn sidebar_state_path() -> std::path::PathBuf {
    let base = margo_fs::xdg::cache_home();
    base.join("mshell").join("settings_sidebar")
}

pub(super) fn load_collapsed_sections() -> std::collections::HashSet<String> {
    std::fs::read_to_string(sidebar_state_path())
        .ok()
        .map(|s| {
            s.split(',')
                .map(|x| x.trim().to_string())
                .filter(|x| !x.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn set_section_collapsed(name: &str, collapsed: bool) {
    let mut set = load_collapsed_sections();
    if collapsed {
        set.insert(name.to_string());
    } else {
        set.remove(name);
    }
    let path = sidebar_state_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let body = set.into_iter().collect::<Vec<_>>().join(",");
    let _ = std::fs::write(path, body);
}

/// Build a collapsible sidebar group: a clickable header (icon + name +
/// chevron) over a `gtk::Revealer` that the page buttons get appended into.
/// Clicking the header flips the revealer + chevron and persists the state.
/// Returns `(group, revealer, pages, chevron)` — `build_sidebar` appends
/// pages into `pages` and uses `(revealer, chevron)` to auto-expand the
/// group when one of its pages is activated (e.g. via search / IPC).
pub(super) fn build_section_group(
    name: &str,
    icon: &str,
    expanded: bool,
) -> (gtk::Box, gtk::Revealer, gtk::Box, gtk::Image) {
    use gtk::prelude::*;

    let group = gtk::Box::new(gtk::Orientation::Vertical, 0);
    group.add_css_class("settings-sidebar-group");

    let header = gtk::Button::new();
    header.add_css_class("settings-sidebar-section");
    let hb = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    hb.set_valign(gtk::Align::Center);

    let img = gtk::Image::from_icon_name(icon);
    img.add_css_class("settings-sidebar-section-icon");
    img.set_valign(gtk::Align::Center);
    hb.append(&img);

    let lbl = gtk::Label::new(None);
    // Uppercased for the eyebrow-label look — GTK CSS has no
    // text-transform, so the casing is applied here. `name` itself
    // (persistence key, ActivateSection lookup) keeps its original case.
    lbl.set_markup(&settings_sidebar_markup(&name.to_uppercase()));
    lbl.add_css_class("settings-sidebar-section-label");
    lbl.set_halign(gtk::Align::Start);
    lbl.set_xalign(0.0);
    lbl.set_hexpand(true);
    lbl.set_wrap(false);
    lbl.set_single_line_mode(true);
    hb.append(&lbl);

    let chevron = gtk::Image::from_icon_name(if expanded {
        "pan-down-symbolic"
    } else {
        "pan-end-symbolic"
    });
    chevron.add_css_class("settings-sidebar-chevron");
    chevron.set_valign(gtk::Align::Center);
    hb.append(&chevron);

    header.set_child(Some(&hb));
    group.append(&header);

    let revealer = gtk::Revealer::builder()
        .transition_type(gtk::RevealerTransitionType::SlideDown)
        .transition_duration(180)
        .reveal_child(expanded)
        .build();
    let pages = gtk::Box::new(gtk::Orientation::Vertical, 2);
    pages.add_css_class("settings-sidebar-group-pages");
    revealer.set_child(Some(&pages));
    group.append(&revealer);

    let name_owned = name.to_string();
    let rev = revealer.clone();
    let chev = chevron.clone();
    header.connect_clicked(move |_| {
        let now = !rev.reveals_child();
        rev.set_reveal_child(now);
        chev.set_icon_name(Some(if now {
            "pan-down-symbolic"
        } else {
            "pan-end-symbolic"
        }));
        set_section_collapsed(&name_owned, !now);
    });

    (group, revealer, pages, chevron)
}

/// Recursively collect every `sidebar-button` toggle under `parent`, in
/// visual order. The accordion nests page buttons inside per-section
/// revealers, so the keyboard-nav walk can't assume direct children.
pub(super) fn collect_sidebar_buttons(
    parent: &impl gtk::prelude::IsA<gtk::Widget>,
    out: &mut Vec<gtk::ToggleButton>,
) {
    use gtk::prelude::*;
    let mut child = parent.first_child();
    while let Some(c) = child {
        if let Ok(btn) = c.clone().downcast::<gtk::ToggleButton>() {
            if btn.has_css_class("sidebar-button") {
                out.push(btn);
            }
        } else {
            collect_sidebar_buttons(&c, out);
        }
        child = c.next_sibling();
    }
}

/// Build the sidebar buttons + section headers from [`SIDEBAR`] into
/// `sidebar_box`, wiring each page button to flip `stack` to its route. The
/// first button is the radio-group anchor (active by default); every other
/// joins its group. Returns route→button so `ActivateSection` can deep-link.
///
/// `ensure` builds the target page into the stack on demand: the page's
/// controller is launched the first time its button is toggled, not up front.
pub(super) fn build_sidebar(
    sidebar_box: &gtk::Box,
    stack: &gtk::Stack,
    ensure: Rc<dyn Fn(&str)>,
) -> std::collections::HashMap<String, gtk::ToggleButton> {
    use gtk::prelude::*;
    let mut buttons = std::collections::HashMap::new();
    let mut anchor: Option<gtk::ToggleButton> = None;
    let collapsed_prefs = load_collapsed_sections();
    // Once the user has toggled any group the state file exists and its
    // per-group record is authoritative; before that, honour each entry's
    // `collapsed` default so the list opens short.
    let has_prefs = sidebar_state_path().exists();
    // Where the next page button goes. Starts at the sidebar root (for any
    // ungrouped page before the first section — e.g. General), then points
    // at each section group's revealer-pages box.
    let mut current_pages: gtk::Box = sidebar_box.clone();
    // The current group's revealer + chevron, so activating one of its pages
    // (via search / IPC) auto-expands the group so the page is visible.
    let mut current_reveal: Option<(gtk::Revealer, gtk::Image)> = None;

    for entry in SIDEBAR {
        match entry {
            Section {
                name,
                icon,
                collapsed,
            } => {
                let expanded = if has_prefs {
                    !collapsed_prefs.contains(*name)
                } else {
                    !*collapsed
                };
                let (group, revealer, pages, chevron) = build_section_group(name, icon, expanded);
                sidebar_box.append(&group);
                current_pages = pages;
                current_reveal = Some((revealer, chevron));
            }
            Page { route, icon, label } => {
                let btn = gtk::ToggleButton::new();
                btn.add_css_class("sidebar-button");
                match &anchor {
                    Some(a) => btn.set_group(Some(a)),
                    None => {
                        btn.set_active(true);
                        anchor = Some(btn.clone());
                    }
                }
                let hbox = gtk::Box::new(gtk::Orientation::Horizontal, 12);
                hbox.set_valign(gtk::Align::Center);
                hbox.append(&gtk::Image::from_icon_name(icon));
                let text = settings_sidebar_label(label);
                hbox.append(&text);
                btn.set_child(Some(&hbox));
                let stack = stack.clone();
                let route_owned = route.to_string();
                let reveal_for_btn = current_reveal.clone();
                let ensure = ensure.clone();
                btn.connect_toggled(move |b| {
                    if b.is_active() {
                        // Build the page into the stack before switching to it —
                        // `set_visible_child_name` is a no-op on an absent child.
                        ensure(&route_owned);
                        stack.set_visible_child_name(&route_owned);
                        // Make sure the activated page's group is open (it may
                        // have been collapsed, or a search jump landed here).
                        if let Some((rev, chev)) = &reveal_for_btn {
                            rev.set_reveal_child(true);
                            chev.set_icon_name(Some("pan-down-symbolic"));
                        }
                    }
                });
                current_pages.append(&btn);
                buttons.insert(route.to_string(), btn);
            }
        }
    }
    buttons
}

#[cfg(test)]
mod sidebar_tests {
    use super::{SIDEBAR, SidebarEntry, initial_search_index};
    use std::collections::HashSet;

    #[test]
    fn sidebar_routes_and_section_names_are_unique() {
        let mut routes = HashSet::new();
        let mut sections = HashSet::new();
        for entry in SIDEBAR {
            match entry {
                SidebarEntry::Page { route, .. } => {
                    assert!(routes.insert(*route), "duplicate sidebar route: {route}")
                }
                SidebarEntry::Section { name, .. } => {
                    assert!(sections.insert(*name), "duplicate section: {name}")
                }
            }
        }
    }

    #[test]
    fn sidebar_icons_are_unique() {
        // Same icon on two rows makes the sidebar hard to scan.
        let mut seen = HashSet::new();
        for entry in SIDEBAR {
            let icon = match entry {
                SidebarEntry::Page { icon, .. } | SidebarEntry::Section { icon, .. } => *icon,
            };
            assert!(seen.insert(icon), "duplicate sidebar icon: {icon}");
        }
    }

    #[test]
    fn every_sidebar_page_is_searchable_by_its_label() {
        // A page must be findable by the name the sidebar shows, including
        // after a rename, without a hand-kept alias.
        let index = initial_search_index();
        for entry in SIDEBAR {
            if let SidebarEntry::Page { route, label, .. } = entry {
                let label = label.to_lowercase();
                assert!(
                    index.iter().any(|(l, r)| r == route && *l == label),
                    "sidebar page {route} ({label}) is not in the search index"
                );
            }
        }
    }
}
