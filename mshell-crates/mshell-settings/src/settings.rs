use crate::bar_settings::bar_settings::{BarSettingsInit, BarSettingsModel};
use crate::display_settings::{DisplaySettingsInit, DisplaySettingsModel};
use crate::fonts_settings::{FontsSettingsInit, FontsSettingsModel};
use crate::general_settings::{GeneralSettingsInit, GeneralSettingsModel};
use crate::idle_settings::{IdleSettingsInit, IdleSettingsModel};
use crate::menu_settings::menu_settings::{MenuSettingsInit, MenuSettingsModel};
use crate::notification_settings::{NotificationSettingsInit, NotificationSettingsModel};
use crate::session_settings::{SessionSettingsInit, SessionSettingsModel};
use crate::theme_settings::theme_settings::{ThemeSettingsInit, ThemeSettingsModel};
use crate::wallpaper_settings::{WallpaperSettingsInit, WallpaperSettingsModel};
use crate::bar_pill_settings::{BarPillKind, BarPillSettingsInit, BarPillSettingsModel};
use crate::widget_menu_settings::{MenuKind, WidgetMenuSettingsInit, WidgetMenuSettingsModel};
use relm4::gtk::prelude::{
    BoxExt, ButtonExt, MonitorExt, OrientableExt, ToggleButtonExt, WidgetExt,
};
use relm4::{Component, ComponentController, ComponentParts, ComponentSender, Controller, gtk};

pub struct SettingsWindowModel {
    general_settings_controller: Controller<GeneralSettingsModel>,
    wallpaper_settings_controller: Controller<WallpaperSettingsModel>,
    theme_settings_controller: Controller<ThemeSettingsModel>,
    fonts_settings_controller: Controller<FontsSettingsModel>,
    display_settings_controller: Controller<DisplaySettingsModel>,
    bar_settings_controller: Controller<BarSettingsModel>,
    menu_settings_controller: Controller<MenuSettingsModel>,
    notification_settings_controller: Controller<NotificationSettingsModel>,
    idle_settings_controller: Controller<IdleSettingsModel>,
    session_settings_controller: Controller<SessionSettingsModel>,
    /// Panel width — computed from the monitor's geometry in
    /// `init`. 4:3 aspect with height set to `monitor_h * 3 / 4`
    /// so the panel covers most of the screen vertically without
    /// overflowing. Falls back to 780 if no monitor is available.
    panel_width: i32,
    /// Panel height — `monitor_h * 3 / 4` if monitor known, else 600.
    panel_height: i32,
}

#[derive(Debug)]
pub enum SettingsWindowInput {}

#[derive(Debug)]
pub enum SettingsWindowOutput {}

pub struct SettingsWindowInit {
    /// Monitor whose geometry drives the panel's sizing. The
    /// frame is per-monitor, so passing the monitor at build
    /// time lets the panel scale per-display (a 4K screen gets
    /// a bigger panel than a 1080p one).
    pub monitor: Option<relm4::gtk::gdk::Monitor>,
}

#[derive(Debug)]
pub enum SettingsWindowCommandOutput {}

#[relm4::component(pub)]
impl Component for SettingsWindowModel {
    type CommandOutput = SettingsWindowCommandOutput;
    type Input = SettingsWindowInput;
    type Output = SettingsWindowOutput;
    type Init = SettingsWindowInit;

    // Embedded menu surface — the frame mounts this widget into
    // its menu stack alongside `wallpaper_menu`, `notifications`,
    // etc. No `gtk::Window` because that would create a separate
    // toplevel; the panel lives inside the same layer-shell
    // surface that hosts the rest of the shell's UI.
    view! {
        #[root]
        gtk::Box {
            add_css_class: "settings-panel",
            set_orientation: gtk::Orientation::Horizontal,
            set_width_request: model.panel_width,
            set_height_request: model.panel_height,

            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,

                gtk::Box {
                    add_css_class: "settings-sidebar",
                    set_orientation: gtk::Orientation::Vertical,
                    set_width_request: 170,
                    set_spacing: 4,
                    set_hexpand: false,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 12,
                        gtk::Label {
                            add_css_class: "label-large-bold",
                            set_margin_start: 8,
                            set_margin_bottom: 8,
                            set_margin_end: 8,
                            set_margin_top: 8,
                            set_label: "Settings",
                            set_halign: gtk::Align::Start,
                            set_hexpand: true,
                        },
                    },

                    gtk::Separator {},

                    // Sidebar order: `General` is always first
                    // (it's the landing page), the rest are
                    // alphabetical. Top-level entries are big
                    // structural buckets (Bar, Display, Fonts,
                    // Idle, Theme, Wallpaper) plus a `Widgets`
                    // collection that holds per-menu config
                    // pages via its own sub-sidebar.
                    #[name = "general_btn"]
                    gtk::ToggleButton {
                        add_css_class: "sidebar-button",
                        set_active: true,
                        connect_toggled[stack] => move |b| {
                            if b.is_active() { stack.set_visible_child_name("general"); }
                        },

                        gtk::Box {
                            set_orientation: gtk::Orientation::Horizontal,
                            set_spacing: 12,
                            gtk::Image { set_icon_name: Some("settings-symbolic") },
                            gtk::Label {
                                add_css_class: "label-medium",
                                set_label: "General",
                                set_halign: gtk::Align::Start,
                                set_hexpand: true,
                            },
                        },
                    },

                    gtk::ToggleButton {
                        add_css_class: "sidebar-button",
                        set_group: Some(&general_btn),
                        connect_toggled[stack] => move |b| {
                            if b.is_active() { stack.set_visible_child_name("bar"); }
                        },

                        gtk::Box {
                            set_orientation: gtk::Orientation::Horizontal,
                            set_spacing: 12,
                            gtk::Image { set_icon_name: Some("sidebar-symbolic") },
                            gtk::Label {
                                add_css_class: "label-medium",
                                set_label: "Bar",
                                set_halign: gtk::Align::Start,
                                set_hexpand: true,
                            },
                        },
                    },

                    gtk::ToggleButton {
                        add_css_class: "sidebar-button",
                        set_group: Some(&general_btn),
                        connect_toggled[stack] => move |b| {
                            if b.is_active() { stack.set_visible_child_name("display"); }
                        },

                        gtk::Box {
                            set_orientation: gtk::Orientation::Horizontal,
                            set_spacing: 12,
                            gtk::Image { set_icon_name: Some("video-display-symbolic") },
                            gtk::Label {
                                add_css_class: "label-medium",
                                set_label: "Display",
                                set_halign: gtk::Align::Start,
                                set_hexpand: true,
                            },
                        },
                    },

                    gtk::ToggleButton {
                        add_css_class: "sidebar-button",
                        set_group: Some(&general_btn),
                        connect_toggled[stack] => move |b| {
                            if b.is_active() { stack.set_visible_child_name("fonts"); }
                        },

                        gtk::Box {
                            set_orientation: gtk::Orientation::Horizontal,
                            set_spacing: 12,
                            gtk::Image { set_icon_name: Some("xsi-font-symbolic") },
                            gtk::Label {
                                add_css_class: "label-medium",
                                set_label: "Fonts",
                                set_halign: gtk::Align::Start,
                                set_hexpand: true,
                            },
                        },
                    },

                    gtk::ToggleButton {
                        add_css_class: "sidebar-button",
                        set_group: Some(&general_btn),
                        connect_toggled[stack] => move |b| {
                            if b.is_active() { stack.set_visible_child_name("idle"); }
                        },

                        gtk::Box {
                            set_orientation: gtk::Orientation::Horizontal,
                            set_spacing: 12,
                            gtk::Image { set_icon_name: Some("coffee-symbolic") },
                            gtk::Label {
                                add_css_class: "label-medium",
                                set_label: "Idle",
                                set_halign: gtk::Align::Start,
                                set_hexpand: true,
                            },
                        },
                    },

                    gtk::ToggleButton {
                        add_css_class: "sidebar-button",
                        set_group: Some(&general_btn),
                        connect_toggled[stack] => move |b| {
                            if b.is_active() { stack.set_visible_child_name("menus"); }
                        },

                        gtk::Box {
                            set_orientation: gtk::Orientation::Horizontal,
                            set_spacing: 12,
                            gtk::Image { set_icon_name: Some("view-list-symbolic") },
                            gtk::Label {
                                add_css_class: "label-medium",
                                set_label: "Menus",
                                set_halign: gtk::Align::Start,
                                set_hexpand: true,
                            },
                        },
                    },

                    gtk::ToggleButton {
                        add_css_class: "sidebar-button",
                        set_group: Some(&general_btn),
                        connect_toggled[stack] => move |b| {
                            if b.is_active() { stack.set_visible_child_name("theme"); }
                        },

                        gtk::Box {
                            set_orientation: gtk::Orientation::Horizontal,
                            set_spacing: 12,
                            gtk::Image { set_icon_name: Some("palette-symbolic") },
                            gtk::Label {
                                add_css_class: "label-medium",
                                set_label: "Theme",
                                set_halign: gtk::Align::Start,
                                set_hexpand: true,
                            },
                        },
                    },

                    gtk::ToggleButton {
                        add_css_class: "sidebar-button",
                        set_group: Some(&general_btn),
                        connect_toggled[stack] => move |b| {
                            if b.is_active() { stack.set_visible_child_name("wallpaper"); }
                        },

                        gtk::Box {
                            set_orientation: gtk::Orientation::Horizontal,
                            set_spacing: 12,
                            gtk::Image { set_icon_name: Some("wallpaper-symbolic") },
                            gtk::Label {
                                add_css_class: "label-medium",
                                set_label: "Wallpaper",
                                set_halign: gtk::Align::Start,
                                set_hexpand: true,
                            },
                        },
                    },

                    gtk::ToggleButton {
                        add_css_class: "sidebar-button",
                        set_group: Some(&general_btn),
                        connect_toggled[stack] => move |b| {
                            if b.is_active() { stack.set_visible_child_name("widgets"); }
                        },

                        gtk::Box {
                            set_orientation: gtk::Orientation::Horizontal,
                            set_spacing: 12,
                            gtk::Image { set_icon_name: Some("view-grid-symbolic") },
                            gtk::Label {
                                add_css_class: "label-medium",
                                set_label: "Widgets",
                                set_halign: gtk::Align::Start,
                                set_hexpand: true,
                            },
                        },
                    },
                },

                #[name = "stack"]
                gtk::Stack {
                    add_css_class: "settings-stack",
                    set_transition_type: gtk::StackTransitionType::Crossfade,
                    set_transition_duration: 50,
                    set_hexpand: true,
                    set_vexpand: true,
                },
            },
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        _sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        // Scale the panel to the host monitor so a 4K screen
        // gets a bigger panel than a 1080p one. Height covers
        // 3/4 of the screen (gives breathing room above + below
        // the menu); width keeps a 4:3 aspect against that
        // height so the sidebar + content read comfortably.
        // Clamp to a sane floor in case the monitor query
        // returns something degenerate (headless / virtual).
        let (panel_width, panel_height) = match params.monitor.as_ref() {
            Some(monitor) => {
                let geom = monitor.geometry();
                let scaled_h = (geom.height() * 3 / 4).max(600);
                let scaled_w = (scaled_h * 4 / 3).max(780);
                (scaled_w, scaled_h)
            }
            None => (780, 600),
        };

        let general_settings_controller = GeneralSettingsModel::builder()
            .launch(GeneralSettingsInit {})
            .detach();

        let wallpaper_settings_controller = WallpaperSettingsModel::builder()
            .launch(WallpaperSettingsInit {})
            .detach();

        let theme_settings_controller = ThemeSettingsModel::builder()
            .launch(ThemeSettingsInit {})
            .detach();

        let fonts_settings_controller = FontsSettingsModel::builder()
            .launch(FontsSettingsInit {})
            .detach();

        let display_settings_controller = DisplaySettingsModel::builder()
            .launch(DisplaySettingsInit {})
            .detach();

        let bar_settings_controller = BarSettingsModel::builder()
            .launch(BarSettingsInit {})
            .detach();

        let menu_settings_controller = MenuSettingsModel::builder()
            .launch(MenuSettingsInit {})
            .detach();

        let notification_settings_controller = NotificationSettingsModel::builder()
            .launch(NotificationSettingsInit {})
            .detach();

        let idle_settings_controller = IdleSettingsModel::builder()
            .launch(IdleSettingsInit {})
            .detach();

        let session_settings_controller = SessionSettingsModel::builder()
            .launch(SessionSettingsInit {})
            .detach();

        let model = SettingsWindowModel {
            general_settings_controller,
            wallpaper_settings_controller,
            theme_settings_controller,
            fonts_settings_controller,
            display_settings_controller,
            bar_settings_controller,
            menu_settings_controller,
            notification_settings_controller,
            idle_settings_controller,
            session_settings_controller,
            panel_width,
            panel_height,
        };

        let widgets = view_output!();

        // widgets.sidebar.set_stack(&widgets.stack);

        widgets.stack.add_titled(
            model.general_settings_controller.widget(),
            Some("general"),
            "General",
        );

        widgets.stack.add_titled(
            model.theme_settings_controller.widget(),
            Some("theme"),
            "Theme",
        );

        widgets.stack.add_titled(
            model.fonts_settings_controller.widget(),
            Some("fonts"),
            "Fonts",
        );

        widgets.stack.add_titled(
            model.wallpaper_settings_controller.widget(),
            Some("wallpaper"),
            "Wallpaper",
        );

        widgets.stack.add_titled(
            model.display_settings_controller.widget(),
            Some("display"),
            "Display",
        );

        widgets.stack.add_titled(
            model.idle_settings_controller.widget(),
            Some("idle"),
            "Idle",
        );

        widgets
            .stack
            .add_titled(model.bar_settings_controller.widget(), Some("bar"), "Bar");

        // `Menus` (the cross-cutting menu_settings page) used to
        // live inside the Widgets sub-sidebar. It's now its own
        // top-level entry so users can jump straight to it from
        // the main sidebar.
        widgets.stack.add_titled(
            model.menu_settings_controller.widget(),
            Some("menus"),
            "Menus",
        );

        // ── Widgets group ──────────────────────────────────────
        // Owns the per-menu settings pages (Layout + each menu's
        // own position / min-width tab). Layout is the existing
        // menu_settings controller — the cross-cutting widget-
        // list editor. The per-menu tabs use one tiny generic
        // component (`WidgetMenuSettingsModel`) instantiated 11
        // times to give every menu its own focused page.
        let widgets_page = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .hexpand(true)
            .vexpand(true)
            .build();

        let widgets_sub_sidebar = gtk::ScrolledWindow::builder()
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        let widgets_sub_sidebar_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .width_request(160)
            .spacing(4)
            .hexpand(false)
            .css_classes(["settings-subsidebar"])
            .build();
        widgets_sub_sidebar.set_child(Some(&widgets_sub_sidebar_box));

        widgets_sub_sidebar_box.append(&{
            let l = gtk::Label::new(Some("Widgets"));
            l.add_css_class("label-medium-bold");
            l.set_halign(gtk::Align::Start);
            l.set_margin_start(8);
            l.set_margin_top(12);
            l.set_margin_bottom(6);
            l.set_margin_end(8);
            l
        });
        widgets_sub_sidebar_box.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

        let widgets_sub_stack = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .transition_duration(50)
            .hexpand(true)
            .vexpand(true)
            .build();

        // Helper closure: build one sub-sidebar ToggleButton +
        // wire it to flip the sub-stack. All buttons except the
        // first share the same `group` so they radio-toggle.
        let make_sub_btn = |label: &str, icon: &str, stack_name: &'static str,
                            first: Option<&gtk::ToggleButton>|
         -> gtk::ToggleButton {
            let mut builder = gtk::ToggleButton::builder().css_classes(["sidebar-button"]);
            if let Some(g) = first {
                builder = builder.group(g);
            } else {
                builder = builder.active(true);
            }
            let btn = builder.build();
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            row.append(&gtk::Image::from_icon_name(icon));
            let lbl = gtk::Label::new(Some(label));
            lbl.add_css_class("label-medium");
            lbl.set_halign(gtk::Align::Start);
            lbl.set_hexpand(true);
            row.append(&lbl);
            btn.set_child(Some(&row));
            let sub_stack = widgets_sub_stack.clone();
            btn.connect_toggled(move |b| {
                if b.is_active() {
                    sub_stack.set_visible_child_name(stack_name);
                }
            });
            btn
        };

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

        // Per-entry sub-sidebar rows, sorted alphabetically by
        // the visible label so widget pages, bar-pill info pages
        // and the rich Notifications / Session pages interleave
        // into one easy-to-scan list.
        enum WidgetEntry {
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
        }

        impl WidgetEntry {
            fn label(&self) -> &'static str {
                match self {
                    Self::Menu { label, .. } | Self::Pill { label, .. } => label,
                    Self::Notifications => "Notifications",
                    Self::Session => "Session",
                }
            }
        }

        let mut entries: Vec<WidgetEntry> = vec![
            // Menu surfaces (own their own widget_menu_settings page).
            WidgetEntry::Menu { kind: MenuKind::AppLauncher, stack_name: "app_launcher", label: "App Launcher", icon: "view-grid-symbolic" },
            WidgetEntry::Menu { kind: MenuKind::Clipboard, stack_name: "clipboard", label: "Clipboard", icon: "edit-paste-symbolic" },
            WidgetEntry::Menu { kind: MenuKind::Clock, stack_name: "clock", label: "Clock", icon: "alarm-symbolic" },
            WidgetEntry::Menu { kind: MenuKind::Ndns, stack_name: "ndns", label: "DNS / VPN", icon: "network-vpn-symbolic" },
            WidgetEntry::Menu { kind: MenuKind::MediaPlayer, stack_name: "media_player", label: "Media Player", icon: "media-playback-start-symbolic" },
            WidgetEntry::Menu { kind: MenuKind::Nnetwork, stack_name: "nnetwork", label: "Network Console", icon: "network-workgroup-symbolic" },
            WidgetEntry::Menu { kind: MenuKind::Nnotes, stack_name: "nnotes", label: "Notes Hub", icon: "notes-symbolic" },
            WidgetEntry::Menu { kind: MenuKind::Npodman, stack_name: "npodman", label: "Podman", icon: "package-symbolic" },
            WidgetEntry::Menu { kind: MenuKind::Npower, stack_name: "npower", label: "Power Profile Menu", icon: "power-profile-balanced-symbolic" },
            WidgetEntry::Menu { kind: MenuKind::Nip, stack_name: "nip", label: "Public IP", icon: "network-wired-symbolic" },
            WidgetEntry::Menu { kind: MenuKind::QuickSettings, stack_name: "quick_settings", label: "Quick Settings", icon: "settings-symbolic" },
            WidgetEntry::Menu { kind: MenuKind::Screenshot, stack_name: "screenshot", label: "Screenshot", icon: "camera-photo-symbolic" },
            WidgetEntry::Menu { kind: MenuKind::Nufw, stack_name: "nufw", label: "UFW Firewall", icon: "firewall-symbolic" },
            // Bar-only pills (no menu surface — just info pages).
            WidgetEntry::Pill { kind: BarPillKind::ActiveWindow, stack_name: "pill_active_window", label: "Active Window", icon: "window-symbolic" },
            WidgetEntry::Pill { kind: BarPillKind::AudioInput, stack_name: "pill_audio_input", label: "Audio Input", icon: "microphone-sensitivity-medium-symbolic" },
            WidgetEntry::Pill { kind: BarPillKind::AudioOutput, stack_name: "pill_audio_output", label: "Audio Output", icon: "audio-volume-medium-symbolic" },
            WidgetEntry::Pill { kind: BarPillKind::Battery, stack_name: "pill_battery", label: "Battery", icon: "battery-good-symbolic" },
            WidgetEntry::Pill { kind: BarPillKind::Bluetooth, stack_name: "pill_bluetooth", label: "Bluetooth", icon: "bluetooth-active-symbolic" },
            WidgetEntry::Pill { kind: BarPillKind::DarkMode, stack_name: "pill_dark_mode", label: "Dark Mode Toggle", icon: "weather-clear-night-symbolic" },
            WidgetEntry::Pill { kind: BarPillKind::HyprPicker, stack_name: "pill_hypr_picker", label: "HyprPicker", icon: "color-select-symbolic" },
            WidgetEntry::Pill { kind: BarPillKind::KeepAwake, stack_name: "pill_keep_awake", label: "Keep Awake", icon: "eye-symbolic" },
            WidgetEntry::Pill { kind: BarPillKind::Lock, stack_name: "pill_lock", label: "Lock", icon: "system-lock-screen-symbolic" },
            WidgetEntry::Pill { kind: BarPillKind::Logout, stack_name: "pill_logout", label: "Logout", icon: "system-log-out-symbolic" },
            WidgetEntry::Pill { kind: BarPillKind::MargoDock, stack_name: "pill_margo_dock", label: "Margo Dock", icon: "view-grid-symbolic" },
            WidgetEntry::Pill { kind: BarPillKind::MargoLayoutSwitcher, stack_name: "pill_margo_layout", label: "Margo Layout Switcher", icon: "layout-symbolic" },
            WidgetEntry::Pill { kind: BarPillKind::MargoTags, stack_name: "pill_margo_tags", label: "Margo Tags", icon: "square-symbolic" },
            WidgetEntry::Pill { kind: BarPillKind::Network, stack_name: "pill_network", label: "Network", icon: "network-wired-symbolic" },
            WidgetEntry::Pill { kind: BarPillKind::PowerProfile, stack_name: "pill_power_profile", label: "Power Profile", icon: "power-profile-balanced-symbolic" },
            WidgetEntry::Pill { kind: BarPillKind::Privacy, stack_name: "pill_privacy", label: "Privacy", icon: "microphone-sensitivity-high-symbolic" },
            WidgetEntry::Pill { kind: BarPillKind::Reboot, stack_name: "pill_reboot", label: "Reboot", icon: "system-reboot-symbolic" },
            WidgetEntry::Pill { kind: BarPillKind::RecordingIndicator, stack_name: "pill_recording", label: "Recording Indicator", icon: "media-record-symbolic" },
            WidgetEntry::Pill { kind: BarPillKind::Shutdown, stack_name: "pill_shutdown", label: "Shutdown", icon: "system-shutdown-symbolic" },
            WidgetEntry::Pill { kind: BarPillKind::Tray, stack_name: "pill_tray", label: "System Tray", icon: "view-list-symbolic" },
            WidgetEntry::Pill { kind: BarPillKind::SystemUpdate, stack_name: "pill_system_update", label: "System Updates", icon: "software-update-available-symbolic" },
            WidgetEntry::Pill { kind: BarPillKind::VpnIndicator, stack_name: "pill_vpn", label: "VPN Indicator", icon: "network-vpn-symbolic" },
            // Rich pages with their own controllers.
            WidgetEntry::Notifications,
            WidgetEntry::Session,
        ];
        entries.sort_by_key(|e| e.label().to_ascii_lowercase());

        // Controllers must outlive `init()` — store them in Vecs
        // and `Box::leak` at the end. The notification and session
        // controllers already live on the model.
        let mut menu_controllers: Vec<relm4::Controller<WidgetMenuSettingsModel>> = Vec::new();
        let mut bar_pill_controllers: Vec<relm4::Controller<BarPillSettingsModel>> = Vec::new();

        for entry in entries {
            match entry {
                WidgetEntry::Menu { kind, stack_name, label, icon } => {
                    let btn = make_sub_btn(label, icon, stack_name, group_anchor.as_ref());
                    if group_anchor.is_none() {
                        group_anchor = Some(btn.clone());
                    }
                    widgets_sub_sidebar_box.append(&btn);
                    let ctrl = WidgetMenuSettingsModel::builder()
                        .launch(WidgetMenuSettingsInit { kind })
                        .detach();
                    widgets_sub_stack.add_named(ctrl.widget(), Some(stack_name));
                    menu_controllers.push(ctrl);
                }
                WidgetEntry::Pill { kind, stack_name, label, icon } => {
                    let btn = make_sub_btn(label, icon, stack_name, group_anchor.as_ref());
                    if group_anchor.is_none() {
                        group_anchor = Some(btn.clone());
                    }
                    widgets_sub_sidebar_box.append(&btn);
                    let ctrl = BarPillSettingsModel::builder()
                        .launch(BarPillSettingsInit { kind })
                        .detach();
                    widgets_sub_stack.add_named(ctrl.widget(), Some(stack_name));
                    bar_pill_controllers.push(ctrl);
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
                    widgets_sub_stack.add_named(
                        model.notification_settings_controller.widget(),
                        Some("notifications"),
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
                    widgets_sub_stack.add_named(
                        model.session_settings_controller.widget(),
                        Some("session"),
                    );
                }
            }
        }

        widgets_page.append(&widgets_sub_sidebar);
        widgets_page.append(&widgets_sub_stack);
        widgets.stack.add_titled(&widgets_page, Some("widgets"), "Widgets");

        // Park the per-menu + per-bar-pill controllers on the
        // model so they outlive `init()`. Box::leak isn't ideal
        // but matches the rest of the file's lifecycle
        // (controllers held by the model owning the window).
        Box::leak(Box::new(menu_controllers));
        Box::leak(Box::new(bar_pill_controllers));

        ComponentParts { model, widgets }
    }
}
