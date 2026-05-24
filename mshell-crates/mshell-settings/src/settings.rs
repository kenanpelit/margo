use crate::bar_settings::bar_settings::{BarSettingsInit, BarSettingsModel};
use crate::display_settings::{DisplaySettingsInit, DisplaySettingsModel};
use crate::fonts_settings::{FontsSettingsInit, FontsSettingsModel};
use crate::gestures_settings::{GesturesSettingsInit, GesturesSettingsModel};
use crate::general_settings::{GeneralSettingsInit, GeneralSettingsModel};
use crate::weather_settings::{WeatherSettingsInit, WeatherSettingsModel};
use crate::idle_settings::{IdleSettingsInit, IdleSettingsModel};
use crate::launcher_settings::{LauncherSettingsInit, LauncherSettingsModel};
use crate::menu_settings::menu_settings::{MenuSettingsInit, MenuSettingsModel};
use crate::notification_settings::{NotificationSettingsInit, NotificationSettingsModel};
use crate::session_settings::{SessionSettingsInit, SessionSettingsModel};
use crate::setup_settings::{SetupSettingsInit, SetupSettingsModel};
use crate::theme_settings::theme_settings::{ThemeSettingsInit, ThemeSettingsModel};
use crate::wallpaper_settings::{WallpaperSettingsInit, WallpaperSettingsModel};
use crate::bar_pill_settings::{BarPillKind, BarPillSettingsInit, BarPillSettingsModel};
use crate::widget_menu_settings::{MenuKind, WidgetMenuSettingsInit, WidgetMenuSettingsModel};
use relm4::gtk::prelude::{
    BoxExt, ButtonExt, EditableExt, MonitorExt, OrientableExt, ToggleButtonExt, WidgetExt,
};
use relm4::{Component, ComponentController, ComponentParts, ComponentSender, Controller, gtk};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub struct SettingsWindowModel {
    general_settings_controller: Controller<GeneralSettingsModel>,
    setup_settings_controller: Controller<SetupSettingsModel>,
    weather_settings_controller: Controller<WeatherSettingsModel>,
    wallpaper_settings_controller: Controller<WallpaperSettingsModel>,
    theme_settings_controller: Controller<ThemeSettingsModel>,
    fonts_settings_controller: Controller<FontsSettingsModel>,
    gestures_settings_controller: Controller<GesturesSettingsModel>,
    display_settings_controller: Controller<DisplaySettingsModel>,
    bar_settings_controller: Controller<BarSettingsModel>,
    menu_settings_controller: Controller<MenuSettingsModel>,
    notification_settings_controller: Controller<NotificationSettingsModel>,
    idle_settings_controller: Controller<IdleSettingsModel>,
    launcher_settings_controller: Controller<LauncherSettingsModel>,
    session_settings_controller: Controller<SessionSettingsModel>,
    /// Panel width — computed from the monitor's geometry in
    /// `init`. 4:3 aspect with height set to `monitor_h * 3 / 4`
    /// so the panel covers most of the screen vertically without
    /// overflowing. Falls back to 780 if no monitor is available.
    panel_width: i32,
    /// Panel height — `monitor_h * 3 / 4` if monitor known, else 600.
    panel_height: i32,
    /// Widgets-group sub-sidebar buttons, keyed by their sub-stack name
    /// (`clipboard` / `network` / `notifications` / …). Lets a deep link
    /// like `widgets/clipboard` activate the right sub-page, not just the
    /// top-level Widgets section. Shared `Rc` because the buttons are
    /// built after the model in `init`; the build loop fills the same map.
    subsection_buttons: Rc<RefCell<HashMap<String, gtk::ToggleButton>>>,
    /// Search targets: `(lowercased label, route)` for every section and
    /// widgets sub-page. `route` is what `ActivateSection` understands
    /// (`theme`, `widgets/clipboard`, …). Filled like `subsection_buttons`.
    search_index: Rc<RefCell<Vec<(String, String)>>>,
}

#[derive(Debug)]
pub enum SettingsWindowInput {
    /// Switch the sidebar (and the page stack via the radio
    /// group's `toggled` cascade) to the given section name —
    /// matches the stack-child names baked into the view!
    /// macro: `general` / `bar` / `display` / `fonts` / `idle`
    /// / `menus` / `theme` / `wallpaper` / `widgets`.
    ///
    /// Unknown names are silently ignored. Wired by the launcher
    /// (via `mshell_settings::open_settings_at_section`) and the
    /// future `mshellctl settings open --section` IPC.
    ActivateSection(String),
    /// The sidebar search box was submitted (Enter). Jumps to the first
    /// section / widget page whose label contains the query.
    SearchSubmitted(String),
}

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
            // GTK4 ignores CSS `overflow: hidden` on a plain GtkBox — the
            // clip is a *widget* property, not a style property. Without
            // this the opaque sidebar / stack backgrounds paint square
            // corners over the frame's rounded notch, so the panel's
            // bottom corners read as "broken". Set the clip in code so the
            // CSS `.settings-panel { border-radius }` actually rounds all
            // four corners.
            set_overflow: gtk::Overflow::Hidden,

            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,

                #[name = "sidebar_box"]
                gtk::Box {
                    add_css_class: "settings-sidebar",
                    set_orientation: gtk::Orientation::Vertical,
                    set_width_request: 170,
                    set_spacing: 4,
                    set_hexpand: false,

                    // ── §12 panel header ──
                    gtk::Box {
                        add_css_class: "panel-header",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 12,
                        set_margin_start: 8,
                        set_margin_end: 8,
                        set_margin_top: 8,
                        set_margin_bottom: 8,
                        gtk::Image {
                            add_css_class: "panel-header-icon",
                            set_valign: gtk::Align::Center,
                            set_icon_name: Some("settings-symbolic"),
                        },
                        gtk::Label {
                            add_css_class: "panel-title",
                            set_label: "Settings",
                            set_halign: gtk::Align::Start,
                            set_hexpand: true,
                        },
                    },

                    // Search box — find any section or widget page by name
                    // (DESIGN.md §12). Focused on open so you can type
                    // straight away; Tab / Down descend into the list.
                    #[name = "search_entry"]
                    gtk::SearchEntry {
                        add_css_class: "settings-search",
                        set_placeholder_text: Some("Search settings…"),
                        set_margin_start: 8,
                        set_margin_end: 8,
                        set_margin_bottom: 4,
                        // `connect_activate` is wired in `init` (this view
                        // doesn't inject `sender` into its closures).
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

                    #[name = "bar_btn"]
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

                    #[name = "display_btn"]
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

                    #[name = "fonts_btn"]
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

                    #[name = "gestures_btn"]
                    gtk::ToggleButton {
                        add_css_class: "sidebar-button",
                        set_group: Some(&general_btn),
                        connect_toggled[stack] => move |b| {
                            if b.is_active() { stack.set_visible_child_name("gestures"); }
                        },

                        gtk::Box {
                            set_orientation: gtk::Orientation::Horizontal,
                            set_spacing: 12,
                            gtk::Image { set_icon_name: Some("input-touchpad-symbolic") },
                            gtk::Label {
                                add_css_class: "label-medium",
                                set_label: "Gestures",
                                set_halign: gtk::Align::Start,
                                set_hexpand: true,
                            },
                        },
                    },

                    #[name = "idle_btn"]
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

                    #[name = "launcher_btn"]
                    gtk::ToggleButton {
                        add_css_class: "sidebar-button",
                        set_group: Some(&general_btn),
                        connect_toggled[stack] => move |b| {
                            if b.is_active() { stack.set_visible_child_name("launcher"); }
                        },

                        gtk::Box {
                            set_orientation: gtk::Orientation::Horizontal,
                            set_spacing: 12,
                            gtk::Image { set_icon_name: Some("system-search-symbolic") },
                            gtk::Label {
                                add_css_class: "label-medium",
                                set_label: "Launcher",
                                set_halign: gtk::Align::Start,
                                set_hexpand: true,
                            },
                        },
                    },

                    #[name = "menus_btn"]
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

                    #[name = "setup_btn"]
                    gtk::ToggleButton {
                        add_css_class: "sidebar-button",
                        set_group: Some(&general_btn),
                        connect_toggled[stack] => move |b| {
                            if b.is_active() { stack.set_visible_child_name("setup"); }
                        },

                        gtk::Box {
                            set_orientation: gtk::Orientation::Horizontal,
                            set_spacing: 12,
                            gtk::Image { set_icon_name: Some("emblem-system-symbolic") },
                            gtk::Label {
                                add_css_class: "label-medium",
                                set_label: "Setup",
                                set_halign: gtk::Align::Start,
                                set_hexpand: true,
                            },
                        },
                    },

                    #[name = "theme_btn"]
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

                    #[name = "wallpaper_btn"]
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

                    #[name = "weather_btn"]
                    gtk::ToggleButton {
                        add_css_class: "sidebar-button",
                        set_group: Some(&general_btn),
                        connect_toggled[stack] => move |b| {
                            if b.is_active() { stack.set_visible_child_name("weather"); }
                        },

                        gtk::Box {
                            set_orientation: gtk::Orientation::Horizontal,
                            set_spacing: 12,
                            gtk::Image { set_icon_name: Some("weather-few-clouds-symbolic") },
                            gtk::Label {
                                add_css_class: "label-medium",
                                set_label: "Weather",
                                set_halign: gtk::Align::Start,
                                set_hexpand: true,
                            },
                        },
                    },

                    #[name = "widgets_btn"]
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
        sender: ComponentSender<Self>,
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
                // A settings panel wants a comfortable, fixed-ish reading
                // size — NOT a 1:1 scale with the display. The old rule
                // (width = height * 4/3) ballooned to ~2160px wide on a 4K
                // monitor. Take a modest fraction of the monitor and clamp
                // to a calm range: roomy on 1080p, never sprawling on
                // 4K / ultrawide. The cap (1080 x 900) is the largest the
                // sidebar + content actually need to read well.
                let w = (geom.width() as f64 * 0.52).round() as i32;
                let h = (geom.height() as f64 * 0.78).round() as i32;
                (w.clamp(820, 1080), h.clamp(600, 900))
            }
            None => (820, 640),
        };

        let general_settings_controller = GeneralSettingsModel::builder()
            .launch(GeneralSettingsInit {})
            .detach();

        let setup_settings_controller = SetupSettingsModel::builder()
            .launch(SetupSettingsInit {})
            .detach();

        let weather_settings_controller = WeatherSettingsModel::builder()
            .launch(WeatherSettingsInit {})
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

        let gestures_settings_controller = GesturesSettingsModel::builder()
            .launch(GesturesSettingsInit {})
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

        let launcher_settings_controller = LauncherSettingsModel::builder()
            .launch(LauncherSettingsInit {})
            .detach();

        let session_settings_controller = SessionSettingsModel::builder()
            .launch(SessionSettingsInit {})
            .detach();

        // Built before the model so the model can hold a clone; the
        // Widgets sub-sidebar build loop (further down) fills the same map.
        let subsection_buttons: Rc<RefCell<HashMap<String, gtk::ToggleButton>>> =
            Rc::new(RefCell::new(HashMap::new()));

        // Search targets — top-level sections seeded here, widgets
        // sub-pages appended as their buttons are built (`make_sub_btn`).
        let search_index: Rc<RefCell<Vec<(String, String)>>> = Rc::new(RefCell::new(
            [
                ("general", "general"),
                ("setup", "setup"),
                ("bar", "bar"),
                ("display", "display"),
                ("fonts", "fonts"),
                ("gestures", "gestures"),
                ("idle", "idle"),
                ("launcher", "launcher"),
                ("menus", "menus"),
                ("theme", "theme"),
                ("wallpaper", "wallpaper"),
                ("widgets", "widgets"),
            ]
            .iter()
            .map(|(label, route)| (label.to_string(), route.to_string()))
            .collect(),
        ));

        let model = SettingsWindowModel {
            general_settings_controller,
            setup_settings_controller,
            weather_settings_controller,
            wallpaper_settings_controller,
            theme_settings_controller,
            fonts_settings_controller,
            gestures_settings_controller,
            display_settings_controller,
            bar_settings_controller,
            menu_settings_controller,
            notification_settings_controller,
            idle_settings_controller,
            launcher_settings_controller,
            session_settings_controller,
            panel_width,
            panel_height,
            subsection_buttons: subsection_buttons.clone(),
            search_index: search_index.clone(),
        };

        let widgets = view_output!();

        // Keyboard navigation on the sidebar — Tab + Up/Down walk
        // through the ToggleButton children, activating each
        // selection along the way so the right-side page updates.
        // GTK4's default focus chain on a Box-of-Buttons should
        // already make Tab work, but layer-shell + Exclusive
        // keyboard-mode swallows the first Tab and the radio
        // group's focus-on-click behaviour can land focus on the
        // page content first instead of the sidebar. This
        // controller is the belt-and-suspenders backup.
        {
            use gtk::gdk;
            use gtk::glib;
            use gtk::prelude::*;

            let sidebar_weak = widgets.sidebar_box.downgrade();
            let search_weak_kc = widgets.search_entry.downgrade();
            let key_controller = gtk::EventControllerKey::new();
            key_controller.set_propagation_phase(gtk::PropagationPhase::Capture);
            key_controller.connect_key_pressed(move |_, keyval, _, modifiers| {
                let Some(sidebar) = sidebar_weak.upgrade() else {
                    return glib::Propagation::Proceed;
                };
                let shift = modifiers.contains(gdk::ModifierType::SHIFT_MASK);
                let dir = match keyval {
                    gdk::Key::Down | gdk::Key::Tab if !shift => 1i32,
                    gdk::Key::Up => -1i32,
                    gdk::Key::ISO_Left_Tab => -1i32,
                    _ => return glib::Propagation::Proceed,
                };

                // Collect the focusable ToggleButton children of the
                // sidebar Box, find which one currently has focus
                // (or is active), and grab focus on the neighbour.
                let mut buttons: Vec<gtk::ToggleButton> = Vec::new();
                let mut child = sidebar.first_child();
                while let Some(c) = child {
                    if let Ok(btn) = c.clone().downcast::<gtk::ToggleButton>()
                        && btn.has_css_class("sidebar-button") {
                            buttons.push(btn);
                        }
                    child = c.next_sibling();
                }
                if buttons.is_empty() {
                    return glib::Propagation::Proceed;
                }
                // From the search box: Down / Tab descends into the list
                // (landing on the active section so arrows continue from
                // there); typing and Up stay with the entry.
                let in_search = search_weak_kc
                    .upgrade()
                    .map(|e| e.has_focus())
                    .unwrap_or(false);
                if in_search {
                    if dir == 1 {
                        let target =
                            buttons.iter().find(|b| b.is_active()).unwrap_or(&buttons[0]);
                        target.grab_focus();
                        return glib::Propagation::Stop;
                    }
                    return glib::Propagation::Proceed;
                }
                let current = buttons
                    .iter()
                    .position(|b| b.has_focus() || b.is_active())
                    .unwrap_or(0);
                let next = ((current as i32 + dir).rem_euclid(buttons.len() as i32)) as usize;
                let target = &buttons[next];
                target.grab_focus();
                target.set_active(true);
                glib::Propagation::Stop
            });
            widgets.sidebar_box.add_controller(key_controller);
            // Submit search on Enter — wired here because this view does
            // not inject `sender` into its closures.
            {
                let sender = sender.clone();
                widgets.search_entry.connect_activate(move |e| {
                    sender.input(SettingsWindowInput::SearchSubmitted(e.text().to_string()));
                });
            }

            // Focus the search box every time the panel is shown — not
            // just once at build. The menu lives in a Revealer + Stack, so
            // its root maps on each reveal; focusing here means you can
            // type to search immediately, and Tab / Down descend into the
            // sidebar list — both work from the very first open without a
            // click. (The prior one-shot idle ran once at startup while the
            // panel was hidden, so its focus was lost.) Deferred to idle so
            // focus lands after the map / realize settles.
            let search_weak2 = widgets.search_entry.downgrade();
            root.connect_map(move |_| {
                let search_weak2 = search_weak2.clone();
                glib::idle_add_local_once(move || {
                    if let Some(entry) = search_weak2.upgrade() {
                        entry.grab_focus();
                    }
                });
            });
        }

        // widgets.sidebar.set_stack(&widgets.stack);

        widgets.stack.add_titled(
            model.general_settings_controller.widget(),
            Some("general"),
            "General",
        );

        widgets.stack.add_titled(
            model.setup_settings_controller.widget(),
            Some("setup"),
            "Setup",
        );

        widgets.stack.add_titled(
            model.weather_settings_controller.widget(),
            Some("weather"),
            "Weather",
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
            model.gestures_settings_controller.widget(),
            Some("gestures"),
            "Gestures",
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

        widgets.stack.add_titled(
            model.launcher_settings_controller.widget(),
            Some("launcher"),
            "Launcher",
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
            // Index by sub-stack name so `widgets/<name>` deep links can
            // activate this exact page.
            subsection_buttons
                .borrow_mut()
                .insert(stack_name.to_string(), btn.clone());
            // And make the page findable from the sidebar search box.
            search_index
                .borrow_mut()
                .push((label.to_lowercase(), format!("widgets/{stack_name}")));
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
            Clipboard,
            SystemUpdate,
        }

        impl WidgetEntry {
            fn label(&self) -> &'static str {
                match self {
                    Self::Clipboard => "Clipboard",
                    Self::SystemUpdate => "System Updates",
                    Self::Menu { label, .. } | Self::Pill { label, .. } => label,
                    Self::Notifications => "Notifications",
                    Self::Session => "Session",
                }
            }
        }

        let mut entries: Vec<WidgetEntry> = vec![
            // Menu surfaces (own their own widget_menu_settings page).
            WidgetEntry::Menu { kind: MenuKind::AppLauncher, stack_name: "app_launcher", label: "App Launcher", icon: "view-grid-symbolic" },
            // Clipboard owns a richer page (menu size + history
            // behaviour), so it's a dedicated entry rather than the
            // generic per-menu settings.
            WidgetEntry::Clipboard,
            WidgetEntry::Menu { kind: MenuKind::Clock, stack_name: "clock", label: "Clock", icon: "alarm-symbolic" },
            WidgetEntry::Menu { kind: MenuKind::Dashboard, stack_name: "dashboard", label: "Dashboard", icon: "view-grid-symbolic" },
            WidgetEntry::Menu { kind: MenuKind::Dns, stack_name: "dns", label: "DNS / VPN", icon: "network-vpn-symbolic" },
            WidgetEntry::Menu { kind: MenuKind::MediaPlayer, stack_name: "media_player", label: "Media Player", icon: "media-playback-start-symbolic" },
            WidgetEntry::Menu { kind: MenuKind::Network, stack_name: "network", label: "Network Console", icon: "network-workgroup-symbolic" },
            WidgetEntry::Menu { kind: MenuKind::Notes, stack_name: "notes", label: "Notes Hub", icon: "notes-symbolic" },
            WidgetEntry::Menu { kind: MenuKind::Podman, stack_name: "podman", label: "Podman", icon: "package-symbolic" },
            WidgetEntry::Menu { kind: MenuKind::Power, stack_name: "power", label: "Power Profile", icon: "power-profile-balanced-symbolic" },
            WidgetEntry::Menu { kind: MenuKind::Ip, stack_name: "ip", label: "Public IP", icon: "network-wired-symbolic" },
            WidgetEntry::Menu { kind: MenuKind::Screenshot, stack_name: "screenshot", label: "Screenshot", icon: "camera-photo-symbolic" },
            WidgetEntry::Menu { kind: MenuKind::Ufw, stack_name: "ufw", label: "UFW Firewall", icon: "firewall-symbolic" },
            WidgetEntry::Menu { kind: MenuKind::Bluetooth, stack_name: "bluetooth", label: "Bluetooth", icon: "bluetooth-active-symbolic" },
            WidgetEntry::Menu { kind: MenuKind::CpuDashboard, stack_name: "cpu_dashboard", label: "CPU Dashboard", icon: "computer-symbolic" },
            WidgetEntry::Menu { kind: MenuKind::AudioDashboard, stack_name: "audio_dashboard", label: "Audio Dashboard", icon: "audio-volume-high-symbolic" },
            // System Updates owns a richer page (menu size + check
            // interval + per-source toggles), so it's a dedicated entry.
            WidgetEntry::SystemUpdate,
            WidgetEntry::Menu { kind: MenuKind::Valent, stack_name: "valent", label: "Valent Connect", icon: "phone-symbolic" },
            WidgetEntry::Menu { kind: MenuKind::Weather, stack_name: "weather", label: "Weather", icon: "weather-few-clouds-symbolic" },
            WidgetEntry::Menu { kind: MenuKind::KeepAwake, stack_name: "keep_awake", label: "Keep Awake", icon: "eye-symbolic" },
            WidgetEntry::Menu { kind: MenuKind::Twilight, stack_name: "twilight", label: "Twilight", icon: "weather-clear-night-symbolic" },
            WidgetEntry::Menu { kind: MenuKind::Keybinds, stack_name: "keybinds", label: "Keyboard Shortcuts", icon: "input-keyboard-symbolic" },
            WidgetEntry::Menu { kind: MenuKind::SshSessions, stack_name: "ssh_sessions", label: "SSH Sessions", icon: "utilities-terminal-symbolic" },
            WidgetEntry::Menu { kind: MenuKind::MargoLayout, stack_name: "margo_layout", label: "Margo Layout Switcher", icon: "view-grid-symbolic" },
            // Bar-only pills (no menu surface — just info pages).
            WidgetEntry::Pill { kind: BarPillKind::ActiveWindow, stack_name: "pill_active_window", label: "Active Window", icon: "window-symbolic" },
            WidgetEntry::Pill { kind: BarPillKind::DarkMode, stack_name: "pill_dark_mode", label: "Dark Mode Toggle", icon: "weather-clear-night-symbolic" },
            WidgetEntry::Pill { kind: BarPillKind::ColorPicker, stack_name: "pill_color_picker", label: "ColorPicker", icon: "color-select-symbolic" },
            WidgetEntry::Pill { kind: BarPillKind::Lock, stack_name: "pill_lock", label: "Lock", icon: "system-lock-screen-symbolic" },
            WidgetEntry::Pill { kind: BarPillKind::Logout, stack_name: "pill_logout", label: "Logout", icon: "system-log-out-symbolic" },
            WidgetEntry::Pill { kind: BarPillKind::MargoDock, stack_name: "pill_margo_dock", label: "Margo Dock", icon: "view-grid-symbolic" },
            WidgetEntry::Pill { kind: BarPillKind::MargoTags, stack_name: "pill_margo_tags", label: "Margo Tags", icon: "square-symbolic" },
            WidgetEntry::Pill { kind: BarPillKind::Privacy, stack_name: "pill_privacy", label: "Privacy", icon: "microphone-sensitivity-high-symbolic" },
            WidgetEntry::Pill { kind: BarPillKind::Reboot, stack_name: "pill_reboot", label: "Reboot", icon: "system-reboot-symbolic" },
            WidgetEntry::Pill { kind: BarPillKind::RecordingIndicator, stack_name: "pill_recording", label: "Recording Indicator", icon: "media-record-symbolic" },
            WidgetEntry::Pill { kind: BarPillKind::Shutdown, stack_name: "pill_shutdown", label: "Shutdown", icon: "system-shutdown-symbolic" },
            WidgetEntry::Pill { kind: BarPillKind::Tray, stack_name: "pill_tray", label: "System Tray", icon: "view-list-symbolic" },
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
                    let ctrl = crate::clipboard_settings::ClipboardSettingsModel::builder()
                        .launch(crate::clipboard_settings::ClipboardSettingsInit {})
                        .detach();
                    widgets_sub_stack.add_named(ctrl.widget(), Some("clipboard"));
                    Box::leak(Box::new(ctrl));
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
                    let ctrl = crate::system_update_settings::SystemUpdateSettingsModel::builder()
                        .launch(crate::system_update_settings::SystemUpdateSettingsInit {})
                        .detach();
                    widgets_sub_stack.add_named(ctrl.widget(), Some("system_update"));
                    Box::leak(Box::new(ctrl));
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

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        use relm4::gtk::prelude::ToggleButtonExt;
        match message {
            SettingsWindowInput::ActivateSection(name) => {
                // Activating a sidebar button fires its
                // `connect_toggled` handler which in turn updates
                // the stack — radio-group cascade does the rest
                // (every other sidebar button auto-deactivates).
                // Unknown section names fall through to a no-op
                // so a stale request can't panic the UI.
                // Accept a plain section ("widgets") or a deep link into
                // the Widgets group's sub-sidebar ("widgets/clipboard") so
                // a widget's own settings gear lands on its exact page.
                let (section, sub) = match name.split_once('/') {
                    Some((s, t)) => (s, Some(t)),
                    None => (name.as_str(), None),
                };
                let button: Option<&relm4::gtk::ToggleButton> = match section {
                    "general" => Some(&widgets.general_btn),
                    "setup" => Some(&widgets.setup_btn),
                    "bar" => Some(&widgets.bar_btn),
                    "display" => Some(&widgets.display_btn),
                    "fonts" => Some(&widgets.fonts_btn),
                    "gestures" => Some(&widgets.gestures_btn),
                    "idle" => Some(&widgets.idle_btn),
                    "launcher" => Some(&widgets.launcher_btn),
                    "menus" => Some(&widgets.menus_btn),
                    "theme" => Some(&widgets.theme_btn),
                    "wallpaper" => Some(&widgets.wallpaper_btn),
                    "widgets" => Some(&widgets.widgets_btn),
                    _ => {
                        tracing::warn!(section = %name, "settings: unknown section name");
                        None
                    }
                };
                if let Some(btn) = button {
                    btn.set_active(true);
                }
                // Then flip the Widgets sub-sidebar to the requested page
                // (its toggle cascades the sub-stack). Cloned out so the
                // RefCell borrow isn't held across `set_active`.
                if let Some(sub) = sub {
                    let sub_btn = self.subsection_buttons.borrow().get(sub).cloned();
                    match sub_btn {
                        Some(b) => b.set_active(true),
                        None => tracing::warn!(%sub, "settings: unknown widgets sub-page"),
                    }
                }
            }
            SettingsWindowInput::SearchSubmitted(query) => {
                let q = query.trim().to_lowercase();
                if !q.is_empty() {
                    // First label that contains the query wins; jump there
                    // via the normal section router and clear the box.
                    let route = self
                        .search_index
                        .borrow()
                        .iter()
                        .find(|(label, _)| label.contains(&q))
                        .map(|(_, route)| route.clone());
                    if let Some(route) = route {
                        widgets.search_entry.set_text("");
                        sender.input(SettingsWindowInput::ActivateSection(route));
                    }
                }
            }
        }
        self.update_view(widgets, sender);
    }
}
