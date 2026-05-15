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
use relm4::gtk::prelude::{BoxExt, MonitorExt, OrientableExt, ToggleButtonExt, WidgetExt};
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
                            if b.is_active() { stack.set_visible_child_name("menus"); }
                        },

                        gtk::Box {
                            set_orientation: gtk::Orientation::Horizontal,
                            set_spacing: 12,
                            gtk::Image { set_icon_name: Some("square-symbolic") },
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
                            if b.is_active() { stack.set_visible_child_name("notifications"); }
                        },

                        gtk::Box {
                            set_orientation: gtk::Orientation::Horizontal,
                            set_spacing: 12,
                            gtk::Image { set_icon_name: Some("notification-symbolic") },
                            gtk::Label {
                                add_css_class: "label-medium",
                                set_label: "Notifications",
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
                            if b.is_active() { stack.set_visible_child_name("session"); }
                        },

                        gtk::Box {
                            set_orientation: gtk::Orientation::Horizontal,
                            set_spacing: 12,
                            gtk::Image { set_icon_name: Some("system-shutdown-symbolic") },
                            gtk::Label {
                                add_css_class: "label-medium",
                                set_label: "Session",
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

        widgets
            .stack
            .add_titled(model.bar_settings_controller.widget(), Some("bar"), "Bar");

        widgets.stack.add_titled(
            model.menu_settings_controller.widget(),
            Some("menus"),
            "Menus",
        );

        widgets.stack.add_titled(
            model.notification_settings_controller.widget(),
            Some("notifications"),
            "Notifications",
        );

        widgets.stack.add_titled(
            model.idle_settings_controller.widget(),
            Some("idle"),
            "Idle",
        );

        widgets.stack.add_titled(
            model.session_settings_controller.widget(),
            Some("session"),
            "Session",
        );

        ComponentParts { model, widgets }
    }
}
