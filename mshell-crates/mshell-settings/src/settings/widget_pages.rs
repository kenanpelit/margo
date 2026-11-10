//! The Widgets group's sub-pages: the entry catalogue and the loop that
//! builds each entry's sub-sidebar button and registers its page factory.

use super::*;

// Per-entry sub-sidebar rows, sorted alphabetically by
// the visible label so widget pages, bar-pill info pages
// and the rich Notifications / Session pages interleave
// into one easy-to-scan list.
pub(super) enum WidgetEntry {
    Menu {
        kind: MenuKind,
        stack_name: &'static str,
        label: &'static str,
        icon: &'static str,
    },
    Pill {
        kind: BarPillKind,
        stack_name: &'static str,
        label: &'static str,
        icon: &'static str,
    },
    Notifications,
    Session,
    Weather,
    MediaPlayer,
    Tune,
    Lyrics,
    HiddenBar,
    Catwalk,
    Privacy,
    Clipboard,
    SystemUpdate,
    Dock,
    SystemTray,
    /// A page stacked from several settings components (each scrolls as
    /// part of one outer page) — e.g. Alarm Clock = Pomodoro rules + the
    /// generic menu geometry page.
    Composite {
        stack_name: &'static str,
        label: &'static str,
        icon: &'static str,
        parts: fn() -> Vec<PagePart>,
    },
}

/// One component of a [`WidgetEntry::Composite`] page: its scrolled root
/// plus the controller to keep alive.
pub(super) type PagePart = (gtk::ScrolledWindow, Box<dyn std::any::Any>);

/// Launch a settings component as a [`PagePart`].
macro_rules! page_part {
    ($model:path => $init:expr) => {{
        let ctrl = <$model>::builder().launch($init).detach();
        let sw: gtk::ScrolledWindow = ctrl.widget().clone();
        (sw, Box::new(ctrl) as Box<dyn std::any::Any>)
    }};
}

/// Stack `parts` into one scrolling page (each part sizes to its content,
/// the outer scroller does the scrolling — no nested scrollbars).
fn stack_parts(parts: Vec<PagePart>) -> (gtk::Widget, Vec<Box<dyn std::any::Any>>) {
    let inner = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let mut alive = Vec::with_capacity(parts.len());
    for (sw, ctrl) in parts {
        sw.set_vscrollbar_policy(gtk::PolicyType::Never);
        sw.set_propagate_natural_height(true);
        sw.set_vexpand(false);
        inner.append(&sw);
        alive.push(ctrl);
    }
    let outer = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .hexpand(true)
        .vexpand(true)
        .child(&inner)
        .build();
    (outer.into(), alive)
}

/// Alarm Clock: the Pomodoro rules and how timers ring, above the menu
/// geometry.
fn alarm_clock_parts() -> Vec<PagePart> {
    vec![
        page_part!(crate::pomodoro_settings::PomodoroSettingsModel => crate::pomodoro_settings::PomodoroSettingsInit {}),
        page_part!(crate::timer_settings::TimerSettingsModel => crate::timer_settings::TimerSettingsInit {}),
        page_part!(WidgetMenuSettingsModel => WidgetMenuSettingsInit { kind: MenuKind::AlarmClock }),
    ]
}

/// Clock: the world-clock options above the menu geometry.
fn clock_parts() -> Vec<PagePart> {
    vec![
        page_part!(crate::world_clock_settings::WorldClockSettingsModel => crate::world_clock_settings::WorldClockSettingsInit {}),
        page_part!(WidgetMenuSettingsModel => WidgetMenuSettingsInit { kind: MenuKind::Clock }),
    ]
}

/// CPU Dashboard: the bar capsule's metrics above the menu geometry.
fn cpu_dashboard_parts() -> Vec<PagePart> {
    vec![
        page_part!(crate::cpu_dashboard_pill_settings::CpuDashboardPillSettingsModel => crate::cpu_dashboard_pill_settings::CpuDashboardPillSettingsInit {}),
        page_part!(WidgetMenuSettingsModel => WidgetMenuSettingsInit { kind: MenuKind::CpuDashboard }),
    ]
}

/// Notes Hub: the Markdown folder / AI options above the menu geometry.
fn notes_parts() -> Vec<PagePart> {
    vec![
        page_part!(crate::notes_settings::NotesSettingsModel => crate::notes_settings::NotesSettingsInit {}),
        page_part!(WidgetMenuSettingsModel => WidgetMenuSettingsInit { kind: MenuKind::Notes }),
    ]
}

/// AI Agents: the agents knobs above the menu geometry.
fn agents_parts() -> Vec<PagePart> {
    vec![
        page_part!(crate::agents_settings::AgentsSettingsModel => crate::agents_settings::AgentsSettingsInit {}),
        page_part!(WidgetMenuSettingsModel => WidgetMenuSettingsInit { kind: MenuKind::Agents }),
    ]
}

/// GitHub: the account / PR rules above the menu geometry.
fn github_parts() -> Vec<PagePart> {
    vec![
        page_part!(crate::github_settings::GithubSettingsModel => crate::github_settings::GithubSettingsInit {}),
        page_part!(WidgetMenuSettingsModel => WidgetMenuSettingsInit { kind: MenuKind::Github }),
    ]
}

/// Discord Voice: saved channels / mic options above the menu geometry.
fn discord_voice_parts() -> Vec<PagePart> {
    vec![
        page_part!(crate::discord_settings::DiscordSettingsModel => crate::discord_settings::DiscordSettingsInit {}),
        page_part!(WidgetMenuSettingsModel => WidgetMenuSettingsInit { kind: MenuKind::DiscordVoice }),
    ]
}

/// UFW Firewall: the pill's rule count above the menu geometry.
fn ufw_parts() -> Vec<PagePart> {
    vec![
        page_part!(crate::ufw_settings::UfwSettingsModel => crate::ufw_settings::UfwSettingsInit {}),
        page_part!(WidgetMenuSettingsModel => WidgetMenuSettingsInit { kind: MenuKind::Ufw }),
    ]
}

/// Eye Care: the reminder's rules (a bar-only pill, no menu geometry).
fn eyecare_parts() -> Vec<PagePart> {
    vec![
        page_part!(crate::eyecare_settings::EyeCareSettingsModel => crate::eyecare_settings::EyeCareSettingsInit {}),
    ]
}

impl WidgetEntry {
    pub(super) fn label(&self) -> &'static str {
        match self {
            Self::Clipboard => "Clipboard",
            Self::SystemUpdate => "System Updates",
            Self::Dock => "Margo Dock",
            Self::SystemTray => "System Tray",
            Self::Menu { label, .. } | Self::Pill { label, .. } | Self::Composite { label, .. } => {
                label
            }
            Self::Notifications => "Notifications",
            Self::Session => "Session",
            Self::Weather => "Weather",
            Self::MediaPlayer => "Media Player",
            Self::Tune => "Tune",
            Self::Lyrics => "Lyrics",
            Self::HiddenBar => "Hidden Bar",
            Self::Catwalk => "Catwalk",
            Self::Privacy => "Privacy",
        }
    }

    /// The sub-stack child name this entry is registered under — must
    /// match the `stack_name` passed to `make_sub_btn` / the dedicated
    /// `add_named` in each loop arm. Used to pick the default sub-page
    /// (the first entry after the alphabetical sort) to build when the
    /// Widgets container is first shown.
    pub(super) fn stack_name(&self) -> &'static str {
        match self {
            Self::Menu { stack_name, .. }
            | Self::Pill { stack_name, .. }
            | Self::Composite { stack_name, .. } => stack_name,
            Self::Notifications => "notifications",
            Self::Session => "session",
            Self::Weather => "weather",
            Self::MediaPlayer => "media_player",
            Self::Tune => "tune",
            Self::Lyrics => "lyrics",
            Self::HiddenBar => "hidden_bar",
            Self::Catwalk => "catwalk",
            Self::Privacy => "privacy",
            Self::Clipboard => "clipboard",
            Self::SystemUpdate => "system_update",
            Self::Dock => "dock",
            Self::SystemTray => "system_tray",
        }
    }
}

/// Every Widgets sub-page, sorted alphabetically by the visible label.
pub(super) fn widget_entries() -> Vec<WidgetEntry> {
    let mut entries: Vec<WidgetEntry> = vec![
        // Clipboard owns a richer page (menu size + history
        // behaviour), so it's a dedicated entry rather than the
        // generic per-menu settings.
        WidgetEntry::Clipboard,
        // Clock = world-clock options + the menu geometry.
        WidgetEntry::Composite {
            stack_name: "clock",
            label: "Clock",
            icon: "alarm-symbolic",
            parts: clock_parts,
        },
        WidgetEntry::Menu {
            kind: MenuKind::Mdash,
            stack_name: "mdash",
            label: "Mdash",
            icon: "view-grid-symbolic",
        },
        WidgetEntry::Menu {
            kind: MenuKind::Vpn,
            stack_name: "vpn",
            label: "VPN",
            icon: "network-vpn-symbolic",
        },
        WidgetEntry::Menu {
            kind: MenuKind::Dns,
            stack_name: "dns",
            label: "DNS",
            icon: "network-server-symbolic",
        },
        WidgetEntry::Menu {
            kind: MenuKind::Ai,
            stack_name: "ai_widget",
            label: "AI",
            icon: "starred-symbolic",
        },
        WidgetEntry::Menu {
            kind: MenuKind::Translate,
            stack_name: "translate",
            label: "Translate",
            icon: "preferences-desktop-locale-symbolic",
        },
        WidgetEntry::Menu {
            kind: MenuKind::ColorPicker,
            stack_name: "color_picker",
            label: "Color Picker",
            icon: "color-select-symbolic",
        },
        WidgetEntry::MediaPlayer,
        WidgetEntry::Tune,
        WidgetEntry::Lyrics,
        WidgetEntry::HiddenBar,
        WidgetEntry::Catwalk,
        WidgetEntry::Menu {
            kind: MenuKind::Network,
            stack_name: "network",
            label: "Network Console",
            icon: "network-workgroup-symbolic",
        },
        // Notes Hub = the Markdown folder / AI options above the menu geometry.
        WidgetEntry::Composite {
            stack_name: "notes",
            label: "Notes Hub",
            icon: "notes-symbolic",
            parts: notes_parts,
        },
        // AI Agents = the agents knobs above the menu geometry.
        WidgetEntry::Composite {
            stack_name: "agents",
            label: "AI Agents",
            icon: "ai-agents-symbolic",
            parts: agents_parts,
        },
        // GitHub = the account / PR rules above the menu geometry.
        WidgetEntry::Composite {
            stack_name: "github",
            label: "GitHub",
            icon: "github-symbolic",
            parts: github_parts,
        },
        WidgetEntry::Menu {
            kind: MenuKind::HostMonitor,
            stack_name: "host_monitor",
            label: "Host Monitor",
            icon: "server-symbolic",
        },
        WidgetEntry::Menu {
            kind: MenuKind::Qr,
            stack_name: "qr",
            label: "QR Code",
            icon: "screenshot-qr-symbolic",
        },
        WidgetEntry::Menu {
            kind: MenuKind::Podman,
            stack_name: "podman",
            label: "Podman",
            icon: "package-symbolic",
        },
        WidgetEntry::Menu {
            kind: MenuKind::Power,
            stack_name: "power",
            label: "Power Profile",
            icon: "power-profile-balanced-symbolic",
        },
        WidgetEntry::Menu {
            kind: MenuKind::Ip,
            stack_name: "ip",
            label: "Public IP",
            icon: "network-wired-symbolic",
        },
        WidgetEntry::Menu {
            kind: MenuKind::Screenshot,
            stack_name: "screenshot",
            label: "Screenshot",
            icon: "camera-photo-symbolic",
        },
        // UFW Firewall = the pill's rule count + the menu geometry.
        WidgetEntry::Composite {
            stack_name: "ufw",
            label: "UFW Firewall",
            icon: "firewall-symbolic",
            parts: ufw_parts,
        },
        WidgetEntry::Menu {
            kind: MenuKind::Bluetooth,
            stack_name: "bluetooth",
            label: "Bluetooth",
            icon: "bluetooth-active-symbolic",
        },
        // CPU Dashboard = the bar capsule's metrics + the menu geometry.
        WidgetEntry::Composite {
            stack_name: "cpu_dashboard",
            label: "CPU Dashboard",
            icon: "computer-symbolic",
            parts: cpu_dashboard_parts,
        },
        WidgetEntry::Menu {
            kind: MenuKind::AudioDashboard,
            stack_name: "audio_dashboard",
            label: "Audio Dashboard",
            icon: "audio-volume-high-symbolic",
        },
        // System Updates owns a richer page (menu size + check
        // interval + per-source toggles), so it's a dedicated entry.
        WidgetEntry::SystemUpdate,
        WidgetEntry::Menu {
            kind: MenuKind::Valent,
            stack_name: "valent",
            label: "Valent Connect",
            icon: "phone-symbolic",
        },
        // Weather owns a dedicated page (location query + units), and it's
        // the single home for weather config — there's no separate
        // top-level Weather entry.
        WidgetEntry::Weather,
        WidgetEntry::Menu {
            kind: MenuKind::KeepAwake,
            stack_name: "keep_awake",
            label: "Keep Awake",
            icon: "eye-symbolic",
        },
        WidgetEntry::Menu {
            kind: MenuKind::Twilight,
            stack_name: "twilight",
            label: "Twilight",
            icon: "weather-clear-night-symbolic",
        },
        WidgetEntry::Menu {
            kind: MenuKind::Keybinds,
            stack_name: "keybinds",
            label: "Keyboard Shortcuts",
            icon: "input-keyboard-symbolic",
        },
        // Alarm Clock = Pomodoro rules + the menu geometry.
        WidgetEntry::Composite {
            stack_name: "alarmclock",
            label: "Alarm Clock",
            icon: "alarm-symbolic",
            parts: alarm_clock_parts,
        },
        WidgetEntry::Menu {
            kind: MenuKind::ControlCenter,
            stack_name: "control_center",
            label: "Control Center",
            icon: "preferences-system-symbolic",
        },
        WidgetEntry::Menu {
            kind: MenuKind::SshSessions,
            stack_name: "ssh_sessions",
            label: "SSH Sessions",
            icon: "utilities-terminal-symbolic",
        },
        WidgetEntry::Menu {
            kind: MenuKind::Usb,
            stack_name: "usb",
            label: "USB Drives",
            icon: "drive-removable-media-symbolic",
        },
        WidgetEntry::Menu {
            kind: MenuKind::Shelf,
            stack_name: "shelf",
            label: "Shelf",
            icon: "folder-download-symbolic",
        },
        // Discord Voice = saved channels / mic options above the menu geometry.
        WidgetEntry::Composite {
            stack_name: "discord_voice",
            label: "Discord Voice",
            icon: "call-start-symbolic",
            parts: discord_voice_parts,
        },
        WidgetEntry::Menu {
            kind: MenuKind::Crashes,
            stack_name: "crashes",
            label: "Crashes",
            icon: "dialog-warning-symbolic",
        },
        WidgetEntry::Menu {
            kind: MenuKind::MargoLayout,
            stack_name: "margo_layout",
            label: "Margo Layout Switcher",
            icon: "view-grid-symbolic",
        },
        // Bar-only pills (no menu surface — just info pages).
        WidgetEntry::Pill {
            kind: BarPillKind::ActiveWindow,
            stack_name: "pill_active_window",
            label: "Active Window",
            icon: "window-symbolic",
        },
        WidgetEntry::Pill {
            kind: BarPillKind::AudioVisualizer,
            stack_name: "pill_audio_visualizer",
            label: "Audio Visualizer",
            icon: "audio-volume-high-symbolic",
        },
        WidgetEntry::Pill {
            kind: BarPillKind::Countdown,
            stack_name: "pill_countdown",
            label: "Countdown",
            icon: "alarm-symbolic",
        },
        WidgetEntry::Composite {
            stack_name: "eyecare",
            label: "Eye Care",
            icon: "eye-symbolic",
            parts: eyecare_parts,
        },
        WidgetEntry::Pill {
            kind: BarPillKind::KeyboardLayout,
            stack_name: "pill_keyboard_layout",
            label: "Keyboard Layout",
            icon: "input-keyboard-symbolic",
        },
        WidgetEntry::Pill {
            kind: BarPillKind::Keymode,
            stack_name: "pill_keymode",
            label: "Keymode",
            icon: "input-keyboard-symbolic",
        },
        WidgetEntry::Pill {
            kind: BarPillKind::DarkMode,
            stack_name: "pill_dark_mode",
            label: "Dark Mode Toggle",
            icon: "weather-clear-night-symbolic",
        },
        WidgetEntry::Pill {
            kind: BarPillKind::Logout,
            stack_name: "pill_logout",
            label: "Logout",
            icon: "system-log-out-symbolic",
        },
        WidgetEntry::Dock,
        WidgetEntry::Pill {
            kind: BarPillKind::MargoTags,
            stack_name: "pill_margo_tags",
            label: "Margo Tags",
            icon: "square-symbolic",
        },
        WidgetEntry::Pill {
            kind: BarPillKind::Scratchpads,
            stack_name: "pill_scratchpads",
            label: "Scratchpads",
            icon: "view-app-grid-symbolic",
        },
        WidgetEntry::Pill {
            kind: BarPillKind::ScrollerRibbon,
            stack_name: "pill_scroller_ribbon",
            label: "Scroller Ribbon",
            icon: "view-list-symbolic",
        },
        WidgetEntry::Privacy,
        WidgetEntry::Pill {
            kind: BarPillKind::Reboot,
            stack_name: "pill_reboot",
            label: "Reboot",
            icon: "system-reboot-symbolic",
        },
        WidgetEntry::Pill {
            kind: BarPillKind::RecordingIndicator,
            stack_name: "pill_recording",
            label: "Recording Indicator",
            icon: "media-record-symbolic",
        },
        WidgetEntry::Pill {
            kind: BarPillKind::Shutdown,
            stack_name: "pill_shutdown",
            label: "Shutdown",
            icon: "system-shutdown-symbolic",
        },
        // System Tray owns a dedicated page (default-expanded toggle),
        // so it's a dedicated entry rather than the generic pill info page.
        WidgetEntry::SystemTray,
        WidgetEntry::Menu {
            kind: MenuKind::VpnIndicator,
            stack_name: "vpn_indicator",
            label: "VPN Indicator",
            icon: "network-vpn-symbolic",
        },
        // Rich pages with their own controllers.
        WidgetEntry::Notifications,
        WidgetEntry::Session,
    ];
    entries.sort_by_key(|e| e.label().to_ascii_lowercase());
    entries
}

/// Build each entry's sub-sidebar button (via `make_sub_btn`) into
/// `widgets_sub_sidebar_box` and register its page factory in `widget_registry`.
pub(super) fn register_widget_entries(
    entries: Vec<WidgetEntry>,
    make_sub_btn: &impl Fn(&str, &str, &'static str, Option<&gtk::ToggleButton>) -> gtk::ToggleButton,
    widgets_sub_sidebar_box: &gtk::Box,
    widget_registry: &RefCell<HashMap<String, PageFactory>>,
) {
    // Menus used to live as the pinned-top entry of the
    // Widgets sub-sidebar but is now its own top-level entry
    // (added above via `add_titled(menu_settings_controller,
    // "menus", "Menus")`). The Widgets group is therefore
    // purely the per-pill + per-menu + Notifications +
    // Session catalogue. The group's anchor toggle is tracked
    // dynamically — the first sub-button we create becomes
    // the radio-group anchor for the rest. Used to be
    // `layout_btn` (the Menus button) carrying that role.
    let mut group_anchor: Option<gtk::ToggleButton> = None;

    // Each arm builds its sub-sidebar button eagerly (cheap GTK widgets that
    // seed the sub-sidebar + the search index, which must resolve before the
    // page is ever opened) and registers a one-shot factory for its page
    // content, which `ensure_widget` launches on first navigation.
    for entry in entries {
        match entry {
            WidgetEntry::Menu {
                kind,
                stack_name,
                label,
                icon,
            } => {
                let btn = make_sub_btn(label, icon, stack_name, group_anchor.as_ref());
                if group_anchor.is_none() {
                    group_anchor = Some(btn.clone());
                }
                widgets_sub_sidebar_box.append(&btn);
                let factory =
                    page_factory!(WidgetMenuSettingsModel => WidgetMenuSettingsInit { kind });
                widget_registry
                    .borrow_mut()
                    .insert(stack_name.to_string(), factory);
            }
            WidgetEntry::Pill {
                kind,
                stack_name,
                label,
                icon,
            } => {
                let btn = make_sub_btn(label, icon, stack_name, group_anchor.as_ref());
                if group_anchor.is_none() {
                    group_anchor = Some(btn.clone());
                }
                widgets_sub_sidebar_box.append(&btn);
                widget_registry.borrow_mut().insert(
                    stack_name.to_string(),
                    page_factory!(BarPillSettingsModel => BarPillSettingsInit { kind }),
                );
            }
            WidgetEntry::Composite {
                stack_name,
                label,
                icon,
                parts,
            } => {
                let btn = make_sub_btn(label, icon, stack_name, group_anchor.as_ref());
                if group_anchor.is_none() {
                    group_anchor = Some(btn.clone());
                }
                widgets_sub_sidebar_box.append(&btn);
                let factory: PageFactory = Rc::new(move || stack_parts(parts()));
                widget_registry
                    .borrow_mut()
                    .insert(stack_name.to_string(), factory);
            }
            WidgetEntry::Notifications => {
                let btn = make_sub_btn(
                    "Notifications",
                    "notification-symbolic",
                    "notifications",
                    group_anchor.as_ref(),
                );
                if group_anchor.is_none() {
                    group_anchor = Some(btn.clone());
                }
                widgets_sub_sidebar_box.append(&btn);
                widget_registry.borrow_mut().insert(
                    "notifications".to_string(),
                    page_factory!(NotificationSettingsModel => NotificationSettingsInit {}),
                );
            }
            WidgetEntry::Session => {
                let btn = make_sub_btn(
                    "Session",
                    "system-shutdown-symbolic",
                    "session",
                    group_anchor.as_ref(),
                );
                if group_anchor.is_none() {
                    group_anchor = Some(btn.clone());
                }
                widgets_sub_sidebar_box.append(&btn);
                widget_registry.borrow_mut().insert(
                    "session".to_string(),
                    page_factory!(SessionSettingsModel => SessionSettingsInit {}),
                );
            }
            WidgetEntry::Weather => {
                let btn = make_sub_btn(
                    "Weather",
                    "weather-few-clouds-symbolic",
                    "weather",
                    group_anchor.as_ref(),
                );
                if group_anchor.is_none() {
                    group_anchor = Some(btn.clone());
                }
                widgets_sub_sidebar_box.append(&btn);
                // Weather is one widget but two config domains: the data
                // source (location / units -> WeatherSettings) and the menu
                // surface (position / width / height -> the generic per-menu
                // page). Compose both into one scrolling page so all of
                // weather's settings live under this single Widgets entry.
                let factory: PageFactory = Rc::new(|| {
                    let weather_ctrl = WeatherSettingsModel::builder()
                        .launch(WeatherSettingsInit {})
                        .detach();
                    let menu_ctrl = WidgetMenuSettingsModel::builder()
                        .launch(WidgetMenuSettingsInit {
                            kind: MenuKind::Weather,
                        })
                        .detach();
                    let ws = weather_ctrl.widget().clone();
                    let ms = menu_ctrl.widget().clone();
                    // Each sub-page sizes to its content; the outer scroller
                    // does the scrolling (no nested scrollbars).
                    for sw in [&ws, &ms] {
                        sw.set_vscrollbar_policy(gtk::PolicyType::Never);
                        sw.set_propagate_natural_height(true);
                        sw.set_vexpand(false);
                    }
                    let inner = gtk::Box::new(gtk::Orientation::Vertical, 0);
                    inner.append(&ws);
                    inner.append(&ms);
                    let outer = gtk::ScrolledWindow::builder()
                        .hscrollbar_policy(gtk::PolicyType::Never)
                        .vscrollbar_policy(gtk::PolicyType::Automatic)
                        .hexpand(true)
                        .vexpand(true)
                        .child(&inner)
                        .build();
                    let widget: gtk::Widget = outer.into();
                    (
                        widget,
                        vec![
                            Box::new(weather_ctrl) as Box<dyn std::any::Any>,
                            Box::new(menu_ctrl) as Box<dyn std::any::Any>,
                        ],
                    )
                });
                widget_registry
                    .borrow_mut()
                    .insert("weather".to_string(), factory);
            }
            WidgetEntry::MediaPlayer => {
                let btn = make_sub_btn(
                    "Media Player",
                    "media-playback-start-symbolic",
                    "media_player",
                    group_anchor.as_ref(),
                );
                if group_anchor.is_none() {
                    group_anchor = Some(btn.clone());
                }
                widgets_sub_sidebar_box.append(&btn);
                // Media Player is one widget but two config domains: the
                // menu surface (position / width / height -> the generic
                // per-menu page) and the playback knobs (seek step +
                // album-art size -> MediaPlayerSettings, ported from the
                // mplayerplus plugin). Compose both into one scrolling page.
                let factory: PageFactory = Rc::new(|| {
                    let menu_ctrl = WidgetMenuSettingsModel::builder()
                        .launch(WidgetMenuSettingsInit {
                            kind: MenuKind::MediaPlayer,
                        })
                        .detach();
                    let media_ctrl = MediaPlayerSettingsModel::builder()
                        .launch(MediaPlayerSettingsInit {})
                        .detach();
                    let ms = menu_ctrl.widget().clone();
                    let ps = media_ctrl.widget().clone();
                    for sw in [&ms, &ps] {
                        sw.set_vscrollbar_policy(gtk::PolicyType::Never);
                        sw.set_propagate_natural_height(true);
                        sw.set_vexpand(false);
                    }
                    let inner = gtk::Box::new(gtk::Orientation::Vertical, 0);
                    inner.append(&ms);
                    inner.append(&ps);
                    let outer = gtk::ScrolledWindow::builder()
                        .hscrollbar_policy(gtk::PolicyType::Never)
                        .vscrollbar_policy(gtk::PolicyType::Automatic)
                        .hexpand(true)
                        .vexpand(true)
                        .child(&inner)
                        .build();
                    let widget: gtk::Widget = outer.into();
                    (
                        widget,
                        vec![
                            Box::new(menu_ctrl) as Box<dyn std::any::Any>,
                            Box::new(media_ctrl) as Box<dyn std::any::Any>,
                        ],
                    )
                });
                widget_registry
                    .borrow_mut()
                    .insert("media_player".to_string(), factory);
            }
            WidgetEntry::Tune => {
                let btn = make_sub_btn(
                    "Tune",
                    "org.margo.Tune-symbolic",
                    "tune",
                    group_anchor.as_ref(),
                );
                if group_anchor.is_none() {
                    group_anchor = Some(btn.clone());
                }
                widgets_sub_sidebar_box.append(&btn);
                // Tune has two config domains: the bar-pill behaviour
                // (which elements show, cover/label size) and the menu
                // surface geometry (the generic per-menu page). Compose
                // both into one page, same as Lyrics.
                let factory: PageFactory = Rc::new(|| {
                    let bar_ctrl = crate::tune_bar_settings::TuneBarSettingsModel::builder()
                        .launch(crate::tune_bar_settings::TuneBarSettingsInit {})
                        .detach();
                    let menu_ctrl = WidgetMenuSettingsModel::builder()
                        .launch(WidgetMenuSettingsInit {
                            kind: MenuKind::Mtune,
                        })
                        .detach();
                    let bs = bar_ctrl.widget().clone();
                    let ms = menu_ctrl.widget().clone();
                    for sw in [&bs, &ms] {
                        sw.set_vscrollbar_policy(gtk::PolicyType::Never);
                        sw.set_propagate_natural_height(true);
                        sw.set_vexpand(false);
                    }
                    let inner = gtk::Box::new(gtk::Orientation::Vertical, 0);
                    inner.append(&bs);
                    inner.append(&ms);
                    let outer = gtk::ScrolledWindow::builder()
                        .hscrollbar_policy(gtk::PolicyType::Never)
                        .vscrollbar_policy(gtk::PolicyType::Automatic)
                        .hexpand(true)
                        .vexpand(true)
                        .child(&inner)
                        .build();
                    let widget: gtk::Widget = outer.into();
                    (
                        widget,
                        vec![
                            Box::new(bar_ctrl) as Box<dyn std::any::Any>,
                            Box::new(menu_ctrl) as Box<dyn std::any::Any>,
                        ],
                    )
                });
                widget_registry
                    .borrow_mut()
                    .insert("tune".to_string(), factory);
            }
            WidgetEntry::Lyrics => {
                let btn =
                    make_sub_btn("Lyrics", "lyrics-symbolic", "lyrics", group_anchor.as_ref());
                if group_anchor.is_none() {
                    group_anchor = Some(btn.clone());
                }
                widgets_sub_sidebar_box.append(&btn);
                // Lyrics has two config domains: the bar-pill behaviour (show
                // the current line in the bar) and the menu surface geometry
                // (the generic per-menu page). Compose both into one page.
                let factory: PageFactory = Rc::new(|| {
                    let bar_ctrl = crate::lyrics_settings::LyricsSettingsModel::builder()
                        .launch(crate::lyrics_settings::LyricsSettingsInit {})
                        .detach();
                    let menu_ctrl = WidgetMenuSettingsModel::builder()
                        .launch(WidgetMenuSettingsInit {
                            kind: MenuKind::Lyrics,
                        })
                        .detach();
                    let bs = bar_ctrl.widget().clone();
                    let ms = menu_ctrl.widget().clone();
                    for sw in [&bs, &ms] {
                        sw.set_vscrollbar_policy(gtk::PolicyType::Never);
                        sw.set_propagate_natural_height(true);
                        sw.set_vexpand(false);
                    }
                    let inner = gtk::Box::new(gtk::Orientation::Vertical, 0);
                    inner.append(&bs);
                    inner.append(&ms);
                    let outer = gtk::ScrolledWindow::builder()
                        .hscrollbar_policy(gtk::PolicyType::Never)
                        .vscrollbar_policy(gtk::PolicyType::Automatic)
                        .hexpand(true)
                        .vexpand(true)
                        .child(&inner)
                        .build();
                    let widget: gtk::Widget = outer.into();
                    (
                        widget,
                        vec![
                            Box::new(bar_ctrl) as Box<dyn std::any::Any>,
                            Box::new(menu_ctrl) as Box<dyn std::any::Any>,
                        ],
                    )
                });
                widget_registry
                    .borrow_mut()
                    .insert("lyrics".to_string(), factory);
            }
            WidgetEntry::HiddenBar => {
                let btn = make_sub_btn(
                    "Hidden Bar",
                    "view-more-horizontal-symbolic",
                    "hidden_bar",
                    group_anchor.as_ref(),
                );
                if group_anchor.is_none() {
                    group_anchor = Some(btn.clone());
                }
                widgets_sub_sidebar_box.append(&btn);
                // Behaviour knobs + a widget-list editor per bar. The list
                // editors reuse the bar-layout section component (TopHidden
                // / BottomHidden locations), so add / remove / reorder work
                // exactly like the normal bar slots -- that's how the user
                // picks what the drawer hides.
                let factory: PageFactory = Rc::new(|| {
                    let cfg = config_manager().config().read_untracked().clone();
                    let hidden_bar_ctrl = HiddenBarSettingsModel::builder()
                        .launch(HiddenBarSettingsInit {})
                        .detach();
                    let top_section = WidgetSectionModel::builder()
                        .launch(WidgetSectionInit {
                            bar_section: BarSection::Hidden,
                            location: BarListLocation::TopHidden,
                            widgets: cfg.bars.top_bar.hidden_widgets.clone(),
                        })
                        .detach();
                    let bottom_section = WidgetSectionModel::builder()
                        .launch(WidgetSectionInit {
                            bar_section: BarSection::Hidden,
                            location: BarListLocation::BottomHidden,
                            widgets: cfg.bars.bottom_bar.hidden_widgets.clone(),
                        })
                        .detach();

                    let inner = gtk::Box::new(gtk::Orientation::Vertical, 12);
                    inner.append(hidden_bar_ctrl.widget());

                    let top_label = gtk::Label::new(Some("Top bar — hidden widgets"));
                    top_label.add_css_class("label-large-bold");
                    top_label.set_halign(gtk::Align::Start);
                    inner.append(&top_label);
                    inner.append(top_section.widget());

                    let bottom_label = gtk::Label::new(Some("Bottom bar — hidden widgets"));
                    bottom_label.add_css_class("label-large-bold");
                    bottom_label.set_halign(gtk::Align::Start);
                    inner.append(&bottom_label);
                    inner.append(bottom_section.widget());

                    let outer = gtk::ScrolledWindow::builder()
                        .hscrollbar_policy(gtk::PolicyType::Never)
                        .vscrollbar_policy(gtk::PolicyType::Automatic)
                        .hexpand(true)
                        .vexpand(true)
                        .child(&inner)
                        .build();

                    // Keep the section row lists in sync when hidden_widgets
                    // changes (e.g. add/remove from this very page, or the
                    // bar updating it): watch config and re-feed each section.
                    let top_sender = top_section.sender().clone();
                    let bottom_sender = bottom_section.sender().clone();
                    let mut hb_effects = EffectScope::new();
                    hb_effects.push(move |_| {
                        let widgets = config_manager()
                            .config()
                            .bars()
                            .top_bar()
                            .hidden_widgets()
                            .get();
                        top_sender.emit(WidgetSectionInput::SetWidgetsEffect(widgets));
                    });
                    hb_effects.push(move |_| {
                        let widgets = config_manager()
                            .config()
                            .bars()
                            .bottom_bar()
                            .hidden_widgets()
                            .get();
                        bottom_sender.emit(WidgetSectionInput::SetWidgetsEffect(widgets));
                    });

                    let widget: gtk::Widget = outer.into();
                    (
                        widget,
                        vec![
                            Box::new(hidden_bar_ctrl) as Box<dyn std::any::Any>,
                            Box::new(top_section) as Box<dyn std::any::Any>,
                            Box::new(bottom_section) as Box<dyn std::any::Any>,
                            Box::new(hb_effects) as Box<dyn std::any::Any>,
                        ],
                    )
                });
                widget_registry
                    .borrow_mut()
                    .insert("hidden_bar".to_string(), factory);
            }
            WidgetEntry::Catwalk => {
                let btn = make_sub_btn(
                    "Catwalk",
                    "face-smile-symbolic",
                    "catwalk",
                    group_anchor.as_ref(),
                );
                if group_anchor.is_none() {
                    group_anchor = Some(btn.clone());
                }
                widgets_sub_sidebar_box.append(&btn);
                widget_registry.borrow_mut().insert(
                    "catwalk".to_string(),
                    page_factory!(CatwalkSettingsModel => CatwalkSettingsInit {}),
                );
            }
            WidgetEntry::Privacy => {
                let btn = make_sub_btn(
                    "Privacy",
                    "security-high-symbolic",
                    "privacy",
                    group_anchor.as_ref(),
                );
                if group_anchor.is_none() {
                    group_anchor = Some(btn.clone());
                }
                widgets_sub_sidebar_box.append(&btn);
                widget_registry.borrow_mut().insert(
                    "privacy".to_string(),
                    page_factory!(crate::privacy_pill_settings::PrivacyPillSettingsModel => crate::privacy_pill_settings::PrivacyPillSettingsInit {}),
                );
            }
            WidgetEntry::Clipboard => {
                let btn = make_sub_btn(
                    "Clipboard",
                    "edit-paste-symbolic",
                    "clipboard",
                    group_anchor.as_ref(),
                );
                if group_anchor.is_none() {
                    group_anchor = Some(btn.clone());
                }
                widgets_sub_sidebar_box.append(&btn);
                widget_registry.borrow_mut().insert(
                    "clipboard".to_string(),
                    page_factory!(crate::clipboard_settings::ClipboardSettingsModel => crate::clipboard_settings::ClipboardSettingsInit {}),
                );
            }
            WidgetEntry::SystemUpdate => {
                let btn = make_sub_btn(
                    "System Updates",
                    "software-update-available-symbolic",
                    "system_update",
                    group_anchor.as_ref(),
                );
                if group_anchor.is_none() {
                    group_anchor = Some(btn.clone());
                }
                widgets_sub_sidebar_box.append(&btn);
                widget_registry.borrow_mut().insert(
                    "system_update".to_string(),
                    page_factory!(crate::system_update_settings::SystemUpdateSettingsModel => crate::system_update_settings::SystemUpdateSettingsInit {}),
                );
            }
            WidgetEntry::Dock => {
                let btn = make_sub_btn(
                    "Margo Dock",
                    "view-grid-symbolic",
                    "dock",
                    group_anchor.as_ref(),
                );
                if group_anchor.is_none() {
                    group_anchor = Some(btn.clone());
                }
                widgets_sub_sidebar_box.append(&btn);
                widget_registry.borrow_mut().insert(
                    "dock".to_string(),
                    page_factory!(crate::dock_settings::DockSettingsModel => crate::dock_settings::DockSettingsInit {}),
                );
            }
            WidgetEntry::SystemTray => {
                let btn = make_sub_btn(
                    "System Tray",
                    "view-list-symbolic",
                    "system_tray",
                    group_anchor.as_ref(),
                );
                if group_anchor.is_none() {
                    group_anchor = Some(btn.clone());
                }
                widgets_sub_sidebar_box.append(&btn);
                widget_registry.borrow_mut().insert(
                    "system_tray".to_string(),
                    page_factory!(crate::system_tray_settings::SystemTraySettingsModel => crate::system_tray_settings::SystemTraySettingsInit {}),
                );
            }
        }
    }
}
