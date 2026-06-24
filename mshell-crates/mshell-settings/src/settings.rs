use crate::about_settings::{AboutSettingsInit, AboutSettingsModel};
use crate::ai_settings::{AiSettingsInit, AiSettingsModel};
use crate::animations_settings::{AnimationsSettingsInit, AnimationsSettingsModel};
use crate::appearance_settings::{AppearanceInit, AppearanceModel};
use crate::backup_settings::{BackupSettingsInit, BackupSettingsModel};
use crate::bar_pill_settings::{BarPillKind, BarPillSettingsInit, BarPillSettingsModel};
use crate::bar_settings::bar_settings::{BarSettingsInit, BarSettingsModel};
use crate::bar_settings::bar_widget_factory::BarListLocation;
use crate::bar_settings::bar_widget_section::{
    BarSection, WidgetSectionInit, WidgetSectionInput, WidgetSectionModel,
};
use crate::behaviour_settings::{BehaviourInit, BehaviourModel};
use crate::bluetooth_settings::{BluetoothSettingsInit, BluetoothSettingsModel};
use crate::catwalk_settings::{CatwalkSettingsInit, CatwalkSettingsModel};
use crate::date_time_settings::{DateTimeSettingsInit, DateTimeSettingsModel};
use crate::default_apps_settings::{DefaultAppsSettingsInit, DefaultAppsSettingsModel};
use crate::display_settings::{DisplaySettingsInit, DisplaySettingsModel};
use crate::effects_settings::{EffectsInit, EffectsModel};
use crate::fonts_settings::{FontsSettingsInit, FontsSettingsModel};
use crate::general_settings::{GeneralSettingsInit, GeneralSettingsModel};
use crate::helium_theme_settings::{HeliumThemeSettingsInit, HeliumThemeSettingsModel};
use crate::hidden_bar_settings::{HiddenBarSettingsInit, HiddenBarSettingsModel};
use crate::idle_settings::{IdleSettingsInit, IdleSettingsModel};
use crate::input_settings::{InputSettingsInit, InputSettingsModel};
use crate::keybinds_settings::{KeybindsSettingsInit, KeybindsSettingsModel};
use crate::keyboard_settings::{KeyboardSettingsInit, KeyboardSettingsModel};
use crate::launcher_settings::{LauncherSettingsInit, LauncherSettingsModel};
use crate::layer_rules_settings::{LayerRulesInit, LayerRulesModel};
use crate::lock_settings::{LockSettingsInit, LockSettingsModel};
use crate::logging_settings::{LoggingInit, LoggingModel};
use crate::media_player_settings::{MediaPlayerSettingsInit, MediaPlayerSettingsModel};
use crate::menu_settings::menu_settings::{MenuSettingsInit, MenuSettingsModel};
use crate::network_settings::{NetworkSettingsInit, NetworkSettingsModel};
use crate::notification_settings::{NotificationSettingsInit, NotificationSettingsModel};
use crate::osd_settings::{OsdSettingsInit, OsdSettingsModel};
use crate::overview_settings::{OverviewSettingsInit, OverviewSettingsModel};
use crate::plugins_settings::{PluginsSettingsInit, PluginsSettingsModel};
use crate::power_settings::{PowerSettingsInit, PowerSettingsModel};
use crate::privacy_settings::{PrivacySettingsInit, PrivacySettingsModel};
use crate::region_settings::{RegionSettingsInit, RegionSettingsModel};
use crate::session_settings::{SessionSettingsInit, SessionSettingsModel};
use crate::setup_settings::{SetupSettingsInit, SetupSettingsModel};
use crate::sound_settings::{SoundSettingsInit, SoundSettingsModel};
use crate::startup_env_settings::{StartupEnvInit, StartupEnvModel};
use crate::tag_layout_settings::{TagLayoutSettingsInit, TagLayoutSettingsModel};
use crate::tag_rules_settings::{TagRulesInit, TagRulesModel};
use crate::theme_settings::theme_settings::{ThemeSettingsInit, ThemeSettingsModel};
use crate::users_settings::{UsersSettingsInit, UsersSettingsModel};
use crate::vpn_settings::{VpnSettingsInit, VpnSettingsModel};
use crate::wallpaper_settings::{WallpaperSettingsInit, WallpaperSettingsModel};
use crate::weather_settings::{WeatherSettingsInit, WeatherSettingsModel};
use crate::widget_menu_settings::{MenuKind, WidgetMenuSettingsInit, WidgetMenuSettingsModel};
use crate::window_rules_settings::{WindowRulesInit, WindowRulesModel};
use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    BarsStoreFields, ConfigStoreFields, GeneralStoreFields, HorizontalBarStoreFields,
};
use reactive_graph::prelude::{Get, GetUntracked};
use reactive_graph::traits::ReadUntracked;
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
    media_player_settings_controller: Controller<MediaPlayerSettingsModel>,
    hidden_bar_settings_controller: Controller<HiddenBarSettingsModel>,
    catwalk_settings_controller: Controller<CatwalkSettingsModel>,
    wallpaper_settings_controller: Controller<WallpaperSettingsModel>,
    theme_settings_controller: Controller<ThemeSettingsModel>,
    fonts_settings_controller: Controller<FontsSettingsModel>,
    helium_theme_settings_controller: Controller<HeliumThemeSettingsModel>,
    about_settings_controller: Controller<AboutSettingsModel>,
    animations_settings_controller: Controller<AnimationsSettingsModel>,
    appearance_settings_controller: Controller<AppearanceModel>,
    effects_settings_controller: Controller<EffectsModel>,
    behaviour_settings_controller: Controller<BehaviourModel>,
    window_rules_settings_controller: Controller<WindowRulesModel>,
    layer_rules_settings_controller: Controller<LayerRulesModel>,
    tag_rules_settings_controller: Controller<TagRulesModel>,
    startup_env_settings_controller: Controller<StartupEnvModel>,
    backup_settings_controller: Controller<BackupSettingsModel>,
    logging_settings_controller: Controller<LoggingModel>,
    osd_settings_controller: Controller<OsdSettingsModel>,
    overview_settings_controller: Controller<OverviewSettingsModel>,
    vpn_settings_controller: Controller<VpnSettingsModel>,
    ai_settings_controller: Controller<AiSettingsModel>,
    date_time_settings_controller: Controller<DateTimeSettingsModel>,
    region_settings_controller: Controller<RegionSettingsModel>,
    sound_settings_controller: Controller<SoundSettingsModel>,
    users_settings_controller: Controller<UsersSettingsModel>,
    input_settings_controller: Controller<InputSettingsModel>,
    keybinds_settings_controller: Controller<KeybindsSettingsModel>,
    summon_settings_controller: Controller<crate::summon_settings::SummonSettingsModel>,
    display_settings_controller: Controller<DisplaySettingsModel>,
    bar_settings_controller: Controller<BarSettingsModel>,
    bluetooth_settings_controller: Controller<BluetoothSettingsModel>,
    default_apps_settings_controller: Controller<DefaultAppsSettingsModel>,
    network_settings_controller: Controller<NetworkSettingsModel>,
    power_settings_controller: Controller<PowerSettingsModel>,
    privacy_settings_controller: Controller<PrivacySettingsModel>,
    menu_settings_controller: Controller<MenuSettingsModel>,
    notification_settings_controller: Controller<NotificationSettingsModel>,
    idle_settings_controller: Controller<IdleSettingsModel>,
    keyboard_settings_controller: Controller<KeyboardSettingsModel>,
    lock_settings_controller: Controller<LockSettingsModel>,
    tag_layout_settings_controller: Controller<TagLayoutSettingsModel>,
    launcher_settings_controller: Controller<LauncherSettingsModel>,
    session_settings_controller: Controller<SessionSettingsModel>,
    plugins_settings_controller: Controller<PluginsSettingsModel>,
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
    /// Top-level sidebar buttons keyed by route, filled by `build_sidebar`
    /// after the view is up (the buttons are created imperatively from the
    /// `SIDEBAR` table, not the `view!` macro). `ActivateSection` looks the
    /// button up here to deep-link.
    section_buttons: Rc<RefCell<HashMap<String, gtk::ToggleButton>>>,
    /// Search targets: `(lowercased label, route)` for every section and
    /// widgets sub-page. `route` is what `ActivateSection` understands
    /// (`theme`, `widgets/clipboard`, …). Filled like `subsection_buttons`.
    search_index: Rc<RefCell<Vec<(String, String)>>>,
    /// Display title per route (`"network"`→"Network", `"widgets/weather"`→
    /// "Weather"), for the flat search-results list. Filled from the SIDEBAR
    /// table + each widgets sub-page button as it's built.
    search_titles: Rc<RefCell<HashMap<String, String>>>,
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
    /// The sidebar search text changed — live-filter the sidebar list
    /// (hide non-matching buttons + the group headers, GNOME-style).
    SearchChanged(String),
    /// Apply a new panel size (width, height) — fired by the size-override
    /// effect when `general.settings_panel_{width,height}` changes.
    SetPanelSize(i32, i32),
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

/// Build a batch of detached settings-page controllers from one list.
///
/// Each entry is `field = ModelPath => InitExpr`; it expands to the
/// `let field = <ModelPath>::builder().launch(InitExpr).detach();` binding
/// every page used to spell out by hand. Adding a (unit-init or otherwise)
/// page is now one line in the [`Component::init`] list instead of a 3-line
/// copy that could pick the wrong `Init` type. The page-stack and sidebar are
/// already table-driven (`stack_pages` + the `SIDEBAR` const); this closes the
/// last per-page boilerplate block.
macro_rules! build_pages {
    ($($ctrl:ident = $model:path => $init:expr),+ $(,)?) => {
        $( let $ctrl = <$model>::builder().launch($init).detach(); )+
    };
}

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
            #[watch]
            set_width_request: model.panel_width,
            #[watch]
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
                // Fill the root's width_request: without this the inner box
                // stays at its natural width and any extra width from the
                // panel size override leaks into empty trailing space (the
                // sidebar + content never widen). Height worked regardless
                // because it's the horizontal box's cross-axis (always
                // filled); width is the main axis and needs the expand.
                set_hexpand: true,

                gtk::Box {
                    add_css_class: "settings-sidebar",
                    set_orientation: gtk::Orientation::Vertical,
                    set_width_request: 170,
                    set_hexpand: false,

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

                    gtk::Separator {
                        // Breathing room so the search pill and the first
                        // nav row ("General") don't sit flush against the
                        // rule from either side.
                        set_margin_top: 2,
                        set_margin_bottom: 2,
                    },

                    gtk::ScrolledWindow {
                        set_hscrollbar_policy: gtk::PolicyType::Never,
                        set_vscrollbar_policy: gtk::PolicyType::Automatic,
                        set_vexpand: true,

                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,

                            #[name = "sidebar_box"]
                            gtk::Box {
                                set_orientation: gtk::Orientation::Vertical,
                                // Tight nav rhythm: the buttons carry their own
                                // min-height; 2px keeps rows visually grouped
                                // without reading as a gapped list.
                                set_spacing: 2,
                            },

                            // Flat search results (page name → navigate), shown
                            // only while the search box has text; the grouped
                            // sidebar above is hidden then.
                            #[name = "search_results_box"]
                            gtk::Box {
                                set_orientation: gtk::Orientation::Vertical,
                                set_spacing: 4,
                                set_visible: false,
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
                    // GtkStack defaults to hhomogeneous = true, which forces
                    // every page onto the WIDEST page's width. That pinned the
                    // panel's minimum to ~1660 (sidebar + the widest editor),
                    // so the width override could grow but never shrink below
                    // it. Size to the visible page instead, so the width
                    // control works downward too.
                    set_hhomogeneous: false,
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
        let (auto_width, auto_height) = match params.monitor.as_ref() {
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
        // A user override (Settings → General → "Settings panel") pins an
        // exact size; `0` keeps the auto fraction above. A leaked effect
        // (below) re-applies this live when the override changes.
        let apply_override = |auto: i32, override_px: i32| {
            if override_px > 0 { override_px } else { auto }
        };
        let panel_width = apply_override(
            auto_width,
            config_manager()
                .config()
                .general()
                .settings_panel_width()
                .get_untracked(),
        );
        let panel_height = apply_override(
            auto_height,
            config_manager()
                .config()
                .general()
                .settings_panel_height()
                .get_untracked(),
        );

        // Re-apply the size override live: when the General page edits
        // `settings_panel_{width,height}`, recompute the request from the
        // stored auto size and push it via `SetPanelSize` so the open panel
        // resizes without an mshell restart.
        let mut size_effects = EffectScope::new();
        let size_sender = sender.clone();
        size_effects.push(move |_| {
            let w = config_manager()
                .config()
                .general()
                .settings_panel_width()
                .get();
            let h = config_manager()
                .config()
                .general()
                .settings_panel_height()
                .get();
            let final_w = if w > 0 { w } else { auto_width };
            let final_h = if h > 0 { h } else { auto_height };
            size_sender.input(SettingsWindowInput::SetPanelSize(final_w, final_h));
        });
        Box::leak(Box::new(size_effects));

        build_pages! {
            general_settings_controller = GeneralSettingsModel => GeneralSettingsInit {},
            setup_settings_controller = SetupSettingsModel => SetupSettingsInit {},
            weather_settings_controller = WeatherSettingsModel => WeatherSettingsInit {},
            media_player_settings_controller = MediaPlayerSettingsModel => MediaPlayerSettingsInit {},
            hidden_bar_settings_controller = HiddenBarSettingsModel => HiddenBarSettingsInit {},
            catwalk_settings_controller = CatwalkSettingsModel => CatwalkSettingsInit {},
            wallpaper_settings_controller = WallpaperSettingsModel => WallpaperSettingsInit {},
            theme_settings_controller = ThemeSettingsModel => ThemeSettingsInit {},
            fonts_settings_controller = FontsSettingsModel => FontsSettingsInit {},
            helium_theme_settings_controller = HeliumThemeSettingsModel => HeliumThemeSettingsInit {},
            about_settings_controller = AboutSettingsModel => AboutSettingsInit {},
            appearance_settings_controller = AppearanceModel => AppearanceInit {},
            effects_settings_controller = EffectsModel => EffectsInit {},
            behaviour_settings_controller = BehaviourModel => BehaviourInit {},
            window_rules_settings_controller = WindowRulesModel => WindowRulesInit {},
            layer_rules_settings_controller = LayerRulesModel => LayerRulesInit {},
            tag_rules_settings_controller = TagRulesModel => TagRulesInit {},
            startup_env_settings_controller = StartupEnvModel => StartupEnvInit {},
            backup_settings_controller = BackupSettingsModel => BackupSettingsInit {},
            logging_settings_controller = LoggingModel => LoggingInit {},
            animations_settings_controller = AnimationsSettingsModel => AnimationsSettingsInit {},
            osd_settings_controller = OsdSettingsModel => OsdSettingsInit {},
            overview_settings_controller = OverviewSettingsModel => OverviewSettingsInit {},
            vpn_settings_controller = VpnSettingsModel => VpnSettingsInit {},
            ai_settings_controller = AiSettingsModel => AiSettingsInit {},
            date_time_settings_controller = DateTimeSettingsModel => DateTimeSettingsInit {},
            region_settings_controller = RegionSettingsModel => RegionSettingsInit {},
            sound_settings_controller = SoundSettingsModel => SoundSettingsInit {},
            users_settings_controller = UsersSettingsModel => UsersSettingsInit {},
            keybinds_settings_controller = KeybindsSettingsModel => KeybindsSettingsInit {},
            summon_settings_controller = crate::summon_settings::SummonSettingsModel => crate::summon_settings::SummonSettingsInit {},
            input_settings_controller = InputSettingsModel => InputSettingsInit {},
            display_settings_controller = DisplaySettingsModel => DisplaySettingsInit {},
            bar_settings_controller = BarSettingsModel => BarSettingsInit {},
            bluetooth_settings_controller = BluetoothSettingsModel => BluetoothSettingsInit {},
            default_apps_settings_controller = DefaultAppsSettingsModel => DefaultAppsSettingsInit {},
            lock_settings_controller = LockSettingsModel => LockSettingsInit {},
            network_settings_controller = NetworkSettingsModel => NetworkSettingsInit {},
            power_settings_controller = PowerSettingsModel => PowerSettingsInit {},
            privacy_settings_controller = PrivacySettingsModel => PrivacySettingsInit {},
            menu_settings_controller = MenuSettingsModel => MenuSettingsInit {},
            notification_settings_controller = NotificationSettingsModel => NotificationSettingsInit {},
            idle_settings_controller = IdleSettingsModel => IdleSettingsInit {},
            keyboard_settings_controller = KeyboardSettingsModel => KeyboardSettingsInit {},
            tag_layout_settings_controller = TagLayoutSettingsModel => TagLayoutSettingsInit {},
            launcher_settings_controller = LauncherSettingsModel => LauncherSettingsInit {},
            session_settings_controller = SessionSettingsModel => SessionSettingsInit {},
            plugins_settings_controller = PluginsSettingsModel => PluginsSettingsInit {},
        }

        // Built before the model so the model can hold a clone; the
        // Widgets sub-sidebar build loop (further down) fills the same map.
        let subsection_buttons: Rc<RefCell<HashMap<String, gtk::ToggleButton>>> =
            Rc::new(RefCell::new(HashMap::new()));
        // Filled by `build_sidebar` after `view_output!` (route → button).
        let section_buttons: Rc<RefCell<HashMap<String, gtk::ToggleButton>>> =
            Rc::new(RefCell::new(HashMap::new()));

        // Search targets — top-level sections seeded here, widgets
        // sub-pages appended as their buttons are built (`make_sub_btn`).
        let search_index: Rc<RefCell<Vec<(String, String)>>> = Rc::new(RefCell::new(
            SEARCH_ALIASES
                .iter()
                .map(|(label, route)| (label.to_string(), route.to_string()))
                .collect(),
        ));
        // Display titles for the flat results list. Top-level pages seeded from
        // the SIDEBAR table; widgets sub-pages added in `make_sub_btn`.
        let search_titles: Rc<RefCell<HashMap<String, String>>> = Rc::new(RefCell::new(
            SIDEBAR
                .iter()
                .filter_map(|e| match e {
                    SidebarEntry::Page { route, label, .. } => {
                        Some((route.to_string(), label.to_string()))
                    }
                    SidebarEntry::Section { .. } => None,
                })
                .collect(),
        ));

        let model = SettingsWindowModel {
            general_settings_controller,
            setup_settings_controller,
            weather_settings_controller,
            media_player_settings_controller,
            hidden_bar_settings_controller,
            catwalk_settings_controller,
            wallpaper_settings_controller,
            theme_settings_controller,
            fonts_settings_controller,
            helium_theme_settings_controller,
            about_settings_controller,
            animations_settings_controller,
            appearance_settings_controller,
            effects_settings_controller,
            osd_settings_controller,
            behaviour_settings_controller,
            window_rules_settings_controller,
            layer_rules_settings_controller,
            tag_rules_settings_controller,
            startup_env_settings_controller,
            backup_settings_controller,
            logging_settings_controller,
            overview_settings_controller,
            vpn_settings_controller,
            ai_settings_controller,
            date_time_settings_controller,
            region_settings_controller,
            sound_settings_controller,
            users_settings_controller,
            input_settings_controller,
            keybinds_settings_controller,
            summon_settings_controller,
            display_settings_controller,
            bar_settings_controller,
            bluetooth_settings_controller,
            default_apps_settings_controller,
            network_settings_controller,
            power_settings_controller,
            privacy_settings_controller,
            menu_settings_controller,
            notification_settings_controller,
            idle_settings_controller,
            keyboard_settings_controller,
            lock_settings_controller,
            tag_layout_settings_controller,
            launcher_settings_controller,
            session_settings_controller,
            plugins_settings_controller,
            panel_width,
            panel_height,
            subsection_buttons: subsection_buttons.clone(),
            section_buttons: section_buttons.clone(),
            search_index: search_index.clone(),
            search_titles: search_titles.clone(),
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
                collect_sidebar_buttons(&sidebar, &mut buttons);
                // Skip page buttons inside a collapsed group — they aren't on
                // screen, so Tab/arrows shouldn't land on them.
                buttons.retain(|b| b.is_mapped());
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
                        let target = buttons
                            .iter()
                            .find(|b| b.is_active())
                            .unwrap_or(&buttons[0]);
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
            // Live filter: hide non-matching entries as the user types.
            {
                let sender = sender.clone();
                widgets.search_entry.connect_search_changed(move |e| {
                    sender.input(SettingsWindowInput::SearchChanged(e.text().to_string()));
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

        // ── Theme group ───────────────────────────────────────
        // Theme owns related pages (scheme, fonts, wallpaper, app themes)
        // under one top-level sidebar entry. This keeps the main nav compact
        // and makes external-app theming feel like part of the matugen chain.
        let theme_page = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .hexpand(true)
            .vexpand(true)
            .build();

        let theme_sub_sidebar = gtk::ScrolledWindow::builder()
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        let theme_sub_sidebar_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .width_request(180)
            .spacing(2)
            .hexpand(false)
            .css_classes(["settings-subsidebar"])
            .build();
        theme_sub_sidebar.set_child(Some(&theme_sub_sidebar_box));

        theme_sub_sidebar_box.append(&{
            let l = settings_sidebar_title_label("Theme");
            l.set_margin_start(8);
            l.set_margin_top(12);
            l.set_margin_bottom(6);
            l.set_margin_end(8);
            l
        });
        theme_sub_sidebar_box.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

        let theme_sub_stack = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .transition_duration(50)
            .hexpand(true)
            .vexpand(true)
            .hhomogeneous(false)
            .build();

        let make_theme_sub_btn = |label: &str,
                                  icon: &str,
                                  stack_name: &'static str,
                                  first: Option<&gtk::ToggleButton>|
         -> gtk::ToggleButton {
            let mut builder = gtk::ToggleButton::builder().css_classes(["sidebar-button"]);
            if let Some(g) = first {
                builder = builder.group(g);
            } else {
                builder = builder.active(true);
            }
            let btn = builder.build();
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            row.set_valign(gtk::Align::Center);
            row.append(&gtk::Image::from_icon_name(icon));
            row.append(&settings_sidebar_label(label));
            btn.set_child(Some(&row));
            let sub_stack = theme_sub_stack.clone();
            btn.connect_toggled(move |b| {
                if b.is_active() {
                    sub_stack.set_visible_child_name(stack_name);
                }
            });
            let route = format!("theme/{stack_name}");
            subsection_buttons
                .borrow_mut()
                .insert(route.clone(), btn.clone());
            search_index
                .borrow_mut()
                .push((label.to_lowercase(), route.clone()));
            search_titles.borrow_mut().insert(route, label.to_string());
            btn
        };

        let theme_entries: [(&str, &str, &'static str, gtk::Widget); 4] = [
            (
                "Scheme",
                "palette-symbolic",
                "scheme",
                model.theme_settings_controller.widget().clone().into(),
            ),
            (
                "Fonts",
                "xsi-font-symbolic",
                "fonts",
                model.fonts_settings_controller.widget().clone().into(),
            ),
            (
                "Wallpaper",
                "wallpaper-symbolic",
                "wallpaper",
                model.wallpaper_settings_controller.widget().clone().into(),
            ),
            (
                "Apps",
                "web-browser-symbolic",
                "apps",
                model
                    .helium_theme_settings_controller
                    .widget()
                    .clone()
                    .into(),
            ),
        ];
        let mut theme_anchor: Option<gtk::ToggleButton> = None;
        for (label, icon, stack_name, widget) in theme_entries {
            let btn = make_theme_sub_btn(label, icon, stack_name, theme_anchor.as_ref());
            if theme_anchor.is_none() {
                theme_anchor = Some(btn.clone());
            }
            theme_sub_sidebar_box.append(&btn);
            theme_sub_stack.add_named(&widget, Some(stack_name));
        }
        theme_sub_stack.set_visible_child_name("scheme");
        theme_page.append(&theme_sub_sidebar);
        theme_page.append(&theme_sub_stack);

        // Top-level stack pages. Insertion order does not affect display (the
        // sidebar buttons drive visibility) — kept as one table so the page
        // list lives in a single place instead of 36 add_titled blocks.
        let stack_pages: Vec<(&str, &str, gtk::Widget)> = vec![
            (
                "general",
                "General",
                model.general_settings_controller.widget().clone().into(),
            ),
            (
                "setup",
                "Setup",
                model.setup_settings_controller.widget().clone().into(),
            ),
            ("theme", "Theme", theme_page.into()),
            (
                "about",
                "About",
                model.about_settings_controller.widget().clone().into(),
            ),
            (
                "animations",
                "Animations",
                model.animations_settings_controller.widget().clone().into(),
            ),
            (
                "appearance",
                "Appearance",
                model.appearance_settings_controller.widget().clone().into(),
            ),
            (
                "effects",
                "Effects",
                model.effects_settings_controller.widget().clone().into(),
            ),
            (
                "osd",
                "OSD",
                model.osd_settings_controller.widget().clone().into(),
            ),
            (
                "behaviour",
                "Behaviour",
                model.behaviour_settings_controller.widget().clone().into(),
            ),
            (
                "window_rules",
                "Window Rules",
                model
                    .window_rules_settings_controller
                    .widget()
                    .clone()
                    .into(),
            ),
            (
                "layer_rules",
                "Layer Rules",
                model
                    .layer_rules_settings_controller
                    .widget()
                    .clone()
                    .into(),
            ),
            (
                "tag_rules",
                "Tag Rules",
                model.tag_rules_settings_controller.widget().clone().into(),
            ),
            (
                "startup_env",
                "Startup",
                model
                    .startup_env_settings_controller
                    .widget()
                    .clone()
                    .into(),
            ),
            (
                "backup",
                "Backup",
                model.backup_settings_controller.widget().clone().into(),
            ),
            (
                "logging",
                "Logging",
                model.logging_settings_controller.widget().clone().into(),
            ),
            (
                "overview",
                "Overview",
                model.overview_settings_controller.widget().clone().into(),
            ),
            (
                "vpn",
                "VPN",
                model.vpn_settings_controller.widget().clone().into(),
            ),
            (
                "ai",
                "AI",
                model.ai_settings_controller.widget().clone().into(),
            ),
            (
                "date_time",
                "Date & Time",
                model.date_time_settings_controller.widget().clone().into(),
            ),
            (
                "region",
                "Region & Language",
                model.region_settings_controller.widget().clone().into(),
            ),
            (
                "sound",
                "Sound",
                model.sound_settings_controller.widget().clone().into(),
            ),
            (
                "users",
                "Users",
                model.users_settings_controller.widget().clone().into(),
            ),
            (
                "input",
                "Input",
                model.input_settings_controller.widget().clone().into(),
            ),
            (
                "keybinds",
                "Keybinds",
                model.keybinds_settings_controller.widget().clone().into(),
            ),
            (
                "summon",
                "Tags",
                model.summon_settings_controller.widget().clone().into(),
            ),
            (
                "display",
                "Display",
                model.display_settings_controller.widget().clone().into(),
            ),
            (
                "bluetooth",
                "Bluetooth",
                model.bluetooth_settings_controller.widget().clone().into(),
            ),
            (
                "default_apps",
                "Default Apps",
                model
                    .default_apps_settings_controller
                    .widget()
                    .clone()
                    .into(),
            ),
            (
                "network",
                "Network",
                model.network_settings_controller.widget().clone().into(),
            ),
            (
                "power",
                "Power",
                model.power_settings_controller.widget().clone().into(),
            ),
            (
                "privacy",
                "Privacy",
                model.privacy_settings_controller.widget().clone().into(),
            ),
            (
                "idle",
                "Idle",
                model.idle_settings_controller.widget().clone().into(),
            ),
            (
                "keyboard",
                "On-Screen Keyboard",
                model.keyboard_settings_controller.widget().clone().into(),
            ),
            (
                "lock",
                "Lock Screen",
                model.lock_settings_controller.widget().clone().into(),
            ),
            (
                "tiling_layout",
                "Tiling Layout",
                model.tag_layout_settings_controller.widget().clone().into(),
            ),
            (
                "launcher",
                "Launcher",
                model.launcher_settings_controller.widget().clone().into(),
            ),
            (
                "bar",
                "Bar",
                model.bar_settings_controller.widget().clone().into(),
            ),
            (
                "plugins",
                "Plugins",
                model.plugins_settings_controller.widget().clone().into(),
            ),
            (
                "menus",
                "Menus",
                model.menu_settings_controller.widget().clone().into(),
            ),
        ];
        for (route, title, widget) in stack_pages {
            widgets.stack.add_titled(&widget, Some(route), title);
        }

        // Build the sidebar buttons + section headers imperatively from the
        // SIDEBAR table (relm4's view! can't loop). Done after the stack pages
        // exist so the first button's `set_active(true)` → "general" resolves.
        *section_buttons.borrow_mut() = build_sidebar(&widgets.sidebar_box, &widgets.stack);

        // ── Widgets group ──────────────────────────────────────
        // Owns per-widget pages. The launcher has its own top-level
        // "Launcher" page, so it intentionally does not appear in this
        // widget catalogue.
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
            // Wide enough for the longest widget names (icon + "Screen
            // Recording" / "System Bluetooth" / "Valent Connect") so the
            // sub-sidebar labels stop ellipsizing to "Notifica…" etc.
            // Paired with the .settings-subsidebar density overrides
            // (smaller label + tighter button padding) in _settings.scss.
            .width_request(216)
            .spacing(2)
            .hexpand(false)
            .css_classes(["settings-subsidebar"])
            .build();
        widgets_sub_sidebar.set_child(Some(&widgets_sub_sidebar_box));

        widgets_sub_sidebar_box.append(&{
            let l = settings_sidebar_title_label("Widgets");
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
        let make_sub_btn = |label: &str,
                            icon: &str,
                            stack_name: &'static str,
                            first: Option<&gtk::ToggleButton>|
         -> gtk::ToggleButton {
            let mut builder = gtk::ToggleButton::builder().css_classes(["sidebar-button"]);
            if let Some(g) = first {
                builder = builder.group(g);
            } else {
                builder = builder.active(true);
            }
            let btn = builder.build();
            // 8px icon→label gap (not the top-level sidebar's 12): the
            // sub-sidebar trades air for fitting the long widget names.
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            row.set_valign(gtk::Align::Center);
            row.append(&gtk::Image::from_icon_name(icon));
            let lbl = settings_sidebar_label(label);
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
            search_titles
                .borrow_mut()
                .insert(format!("widgets/{stack_name}"), label.to_string());
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
            Weather,
            MediaPlayer,
            HiddenBar,
            Catwalk,
            Privacy,
            Clipboard,
            SystemUpdate,
            Dock,
            SystemTray,
        }

        impl WidgetEntry {
            fn label(&self) -> &'static str {
                match self {
                    Self::Clipboard => "Clipboard",
                    Self::SystemUpdate => "System Updates",
                    Self::Dock => "Margo Dock",
                    Self::SystemTray => "System Tray",
                    Self::Menu { label, .. } | Self::Pill { label, .. } => label,
                    Self::Notifications => "Notifications",
                    Self::Session => "Session",
                    Self::Weather => "Weather",
                    Self::MediaPlayer => "Media Player",
                    Self::HiddenBar => "Hidden Bar",
                    Self::Catwalk => "Catwalk",
                    Self::Privacy => "Privacy",
                }
            }
        }

        let mut entries: Vec<WidgetEntry> = vec![
            // Clipboard owns a richer page (menu size + history
            // behaviour), so it's a dedicated entry rather than the
            // generic per-menu settings.
            WidgetEntry::Clipboard,
            WidgetEntry::Menu {
                kind: MenuKind::Clock,
                stack_name: "clock",
                label: "Clock",
                icon: "alarm-symbolic",
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
            WidgetEntry::MediaPlayer,
            WidgetEntry::HiddenBar,
            WidgetEntry::Catwalk,
            WidgetEntry::Menu {
                kind: MenuKind::Network,
                stack_name: "network",
                label: "Network Console",
                icon: "network-workgroup-symbolic",
            },
            WidgetEntry::Menu {
                kind: MenuKind::Notes,
                stack_name: "notes",
                label: "Notes Hub",
                icon: "notes-symbolic",
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
            WidgetEntry::Menu {
                kind: MenuKind::Ufw,
                stack_name: "ufw",
                label: "UFW Firewall",
                icon: "firewall-symbolic",
            },
            WidgetEntry::Menu {
                kind: MenuKind::Bluetooth,
                stack_name: "bluetooth",
                label: "Bluetooth",
                icon: "bluetooth-active-symbolic",
            },
            WidgetEntry::Menu {
                kind: MenuKind::CpuDashboard,
                stack_name: "cpu_dashboard",
                label: "CPU Dashboard",
                icon: "computer-symbolic",
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
            WidgetEntry::Menu {
                kind: MenuKind::AlarmClock,
                stack_name: "alarmclock",
                label: "Alarm Clock",
                icon: "alarm-symbolic",
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
                kind: BarPillKind::KeyboardLayout,
                stack_name: "pill_keyboard_layout",
                label: "Keyboard Layout",
                icon: "input-keyboard-symbolic",
            },
            WidgetEntry::Pill {
                kind: BarPillKind::DarkMode,
                stack_name: "pill_dark_mode",
                label: "Dark Mode Toggle",
                icon: "weather-clear-night-symbolic",
            },
            WidgetEntry::Pill {
                kind: BarPillKind::ColorPicker,
                stack_name: "pill_color_picker",
                label: "ColorPicker",
                icon: "color-select-symbolic",
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
            WidgetEntry::Pill {
                kind: BarPillKind::VpnIndicator,
                stack_name: "pill_vpn",
                label: "VPN Indicator",
                icon: "network-vpn-symbolic",
            },
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
                    let ctrl = WidgetMenuSettingsModel::builder()
                        .launch(WidgetMenuSettingsInit { kind })
                        .detach();
                    widgets_sub_stack.add_named(ctrl.widget(), Some(stack_name));
                    menu_controllers.push(ctrl);
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
                    widgets_sub_stack
                        .add_named(model.session_settings_controller.widget(), Some("session"));
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
                    // source (location / units → WeatherSettings) and the menu
                    // surface (position / width / height → the generic per-menu
                    // page). Compose both into one scrolling page so all of
                    // weather's settings live under this single Widgets entry.
                    let menu_ctrl = WidgetMenuSettingsModel::builder()
                        .launch(WidgetMenuSettingsInit {
                            kind: MenuKind::Weather,
                        })
                        .detach();
                    let ws = model.weather_settings_controller.widget().clone();
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
                    widgets_sub_stack.add_named(&outer, Some("weather"));
                    Box::leak(Box::new(menu_ctrl));
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
                    // menu surface (position / width / height → the generic
                    // per-menu page) and the playback knobs (seek step +
                    // album-art size → MediaPlayerSettings, ported from the
                    // mplayerplus plugin). Compose both into one scrolling page.
                    let menu_ctrl = WidgetMenuSettingsModel::builder()
                        .launch(WidgetMenuSettingsInit {
                            kind: MenuKind::MediaPlayer,
                        })
                        .detach();
                    let ms = menu_ctrl.widget().clone();
                    let ps = model.media_player_settings_controller.widget().clone();
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
                    widgets_sub_stack.add_named(&outer, Some("media_player"));
                    Box::leak(Box::new(menu_ctrl));
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
                    // exactly like the normal bar slots — that's how the user
                    // picks what the drawer hides.
                    let cfg = config_manager().config().read_untracked().clone();
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
                    inner.append(model.hidden_bar_settings_controller.widget());

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
                    widgets_sub_stack.add_named(&outer, Some("hidden_bar"));

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

                    Box::leak(Box::new(hb_effects));
                    Box::leak(Box::new(top_section));
                    Box::leak(Box::new(bottom_section));
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
                    widgets_sub_stack
                        .add_named(model.catwalk_settings_controller.widget(), Some("catwalk"));
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
                    let ctrl = crate::privacy_pill_settings::PrivacyPillSettingsModel::builder()
                        .launch(crate::privacy_pill_settings::PrivacyPillSettingsInit {})
                        .detach();
                    widgets_sub_stack.add_named(ctrl.widget(), Some("privacy"));
                    Box::leak(Box::new(ctrl));
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
                    let ctrl = crate::dock_settings::DockSettingsModel::builder()
                        .launch(crate::dock_settings::DockSettingsInit {})
                        .detach();
                    widgets_sub_stack.add_named(ctrl.widget(), Some("dock"));
                    Box::leak(Box::new(ctrl));
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
                    let ctrl = crate::system_tray_settings::SystemTraySettingsModel::builder()
                        .launch(crate::system_tray_settings::SystemTraySettingsInit {})
                        .detach();
                    widgets_sub_stack.add_named(ctrl.widget(), Some("system_tray"));
                    Box::leak(Box::new(ctrl));
                }
            }
        }

        widgets_page.append(&widgets_sub_sidebar);
        widgets_page.append(&widgets_sub_stack);
        widgets
            .stack
            .add_titled(&widgets_page, Some("widgets"), "Widgets");

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
                let name = match name.as_str() {
                    // Backwards-compatible routes used by older settings
                    // buttons / search aliases before Fonts and Wallpaper
                    // moved under the Theme sub-sidebar.
                    "fonts" => "theme/fonts".to_string(),
                    "wallpaper" => "theme/wallpaper".to_string(),
                    _ => name,
                };
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
                // Look the top-level button up in the route→button map filled
                // by `build_sidebar`. Activating it fires its connect_toggled,
                // which flips the stack (the radio group deactivates the rest).
                let button = self.section_buttons.borrow().get(section).cloned();
                match button {
                    Some(btn) => btn.set_active(true),
                    None => tracing::warn!(section = %name, "settings: unknown section name"),
                }
                // Then flip the Widgets sub-sidebar to the requested page
                // (its toggle cascades the sub-stack). Cloned out so the
                // RefCell borrow isn't held across `set_active`.
                if let Some(sub) = sub {
                    let sub_btn = self.subsection_buttons.borrow().get(&name).cloned();
                    let sub_btn =
                        sub_btn.or_else(|| self.subsection_buttons.borrow().get(sub).cloned());
                    match sub_btn {
                        Some(b) => b.set_active(true),
                        None => tracing::warn!(%sub, route = %name, "settings: unknown sub-page"),
                    }
                }
                // Clear the search box so a results-list click (or a deep link)
                // restores the grouped sidebar via SearchChanged("").
                widgets.search_entry.set_text("");
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
                        .find(|(label, _)| label.contains(&q) || keywords_for(label).contains(&q))
                        .map(|(_, route)| route.clone());
                    if let Some(route) = route {
                        widgets.search_entry.set_text("");
                        sender.input(SettingsWindowInput::ActivateSection(route));
                    }
                }
            }
            SettingsWindowInput::SearchChanged(query) => {
                use gtk::prelude::*;
                let q = query.trim().to_lowercase();
                // Rebuild the flat results list. Empty query → show the grouped
                // sidebar, hide results. Non-empty → hide the sidebar and show a
                // clickable row per matching destination (top-level pages AND
                // nested sub-pages like Weather / Clipboard), so anything in the
                // search_index — including the Widgets sub-sidebar — is reachable
                // by name without knowing where it lives.
                while let Some(c) = widgets.search_results_box.first_child() {
                    widgets.search_results_box.remove(&c);
                }
                if q.is_empty() {
                    widgets.sidebar_box.set_visible(true);
                    widgets.search_results_box.set_visible(false);
                } else {
                    widgets.sidebar_box.set_visible(false);
                    widgets.search_results_box.set_visible(true);
                    let titles = self.search_titles.borrow();
                    let mut seen = std::collections::HashSet::new();
                    for (label, route) in self.search_index.borrow().iter() {
                        if !(label.contains(&q) || keywords_for(label).contains(&q)) {
                            continue;
                        }
                        // One row per destination (a route can have several
                        // aliases); keep the first hit's order.
                        if !seen.insert(route.clone()) {
                            continue;
                        }
                        let title = titles.get(route).cloned().unwrap_or_else(|| route.clone());
                        let btn = gtk::Button::new();
                        btn.add_css_class("sidebar-button");
                        let lbl = settings_sidebar_label(&title);
                        btn.set_child(Some(&lbl));
                        let route = route.clone();
                        let s = sender.clone();
                        btn.connect_clicked(move |_| {
                            s.input(SettingsWindowInput::ActivateSection(route.clone()));
                        });
                        widgets.search_results_box.append(&btn);
                    }
                }
            }
            SettingsWindowInput::SetPanelSize(w, h) => {
                self.panel_width = w;
                self.panel_height = h;
            }
        }
        self.update_view(widgets, sender);
    }
}

/// Section-level search keywords, keyed by the lowercased sidebar
/// label (or top-level search-index route). Lets a query like
/// "brightness" surface Display or "vpn" surface Network without
/// maintaining a per-control index that would drift from the hand-built
/// pages. Returns an empty string for sections with no extra synonyms.
/// Fuzzy keyword expansion per sidebar label, used by the live search filter.
/// One row per label spelling (alias spellings get their own row); the lookup
/// is a linear scan (the list is tiny + cold). Kept as data rather than a match
/// so adding a page's keywords is a one-line table edit.
const PAGE_KEYWORDS: &[(&str, &str)] = &[
    (
        "power",
        "battery suspend sleep hibernate profile performance balanced saver lid power-button",
    ),
    (
        "network",
        "wifi wi-fi wireless ethernet wired vpn proxy dns connection ip hotspot",
    ),
    ("bluetooth", "bt pair pairing device headset"),
    (
        "display",
        "monitor screen resolution scale scaling brightness refresh rate hidpi",
    ),
    (
        "theme",
        "color colour palette matugen accent scheme dark light mode tint",
    ),
    (
        "scheme",
        "color colour palette matugen accent scheme dark light mode tint",
    ),
    ("wallpaper", "background image picture slideshow rotation"),
    (
        "apps",
        "app apps external browser chromium helium isolated profile matugen theme",
    ),
    (
        "helium",
        "app apps external browser chromium helium isolated profile matugen theme",
    ),
    (
        "sound",
        "audio volume output input microphone speaker mute device sink source",
    ),
    ("fonts", "font typeface family size weight"),
    (
        "keybinds",
        "keybind keybinding shortcut hotkey bind keyboard binding cheatsheet",
    ),
    (
        "summon",
        "tags tag apps summon bring here mango-here app hotkey per-tag launch app-id workspace move window to tag toggletag tag key",
    ),
    (
        "input",
        "keyboard mouse touchpad layout xkb repeat sensitivity natural scroll cursor",
    ),
    ("animations", "animation motion transition speed easing"),
    (
        "appearance",
        "border thickness radius corner gap gaps opacity cursor size window look",
    ),
    ("effects", "shadow shadows drop blur layer floating glow"),
    (
        "osd",
        "osd on-screen display volume brightness mic network capsule pill width position radius border size distance",
    ),
    (
        "behaviour",
        "focus sloppy warp cursor drag tile swap snap hot corner overview scroll axis scratchpad tearing sync syncobj xwayland inhibit",
    ),
    (
        "behavior",
        "focus sloppy warp cursor drag tile swap snap hot corner overview scroll axis scratchpad tearing sync syncobj xwayland inhibit",
    ),
    (
        "window_rules",
        "window rule windowrule app-id appid title regex tag float floating fullscreen size monitor pin match",
    ),
    (
        "layer_rules",
        "layer rule layerrule namespace bar menu notification osd noanim noblur noshadow surface",
    ),
    (
        "tag_rules",
        "tag rule tagrule pin monitor home layout mfact master nmaster workspace",
    ),
    (
        "startup_env",
        "startup script autostart start exec env environment variable launch \
         command session delay login working directory arguments",
    ),
    (
        "backup",
        "backup export import profile profiles reset restore bundle snapshot \
         tar config default factory save load",
    ),
    (
        "logging",
        "log logs logging level debug trace info warn error diagnostics \
         troubleshoot file session verbose",
    ),
    ("date_time", "clock time date timezone ntp format 24-hour"),
    ("date time", "clock time date timezone ntp format 24-hour"),
    ("date & time", "clock time date timezone ntp format 24-hour"),
    ("region", "locale language format measurement keyboard"),
    (
        "region & language",
        "locale language format measurement keyboard",
    ),
    (
        "idle",
        "screensaver dim timeout inhibitor dpms blank suspend",
    ),
    (
        "keyboard",
        "on-screen keyboard osk virtual keyboard touch type mkeys layout turkish",
    ),
    (
        "on-screen keyboard",
        "on-screen keyboard osk virtual keyboard touch type mkeys layout turkish",
    ),
    ("lock", "lockscreen password security blur unlock pam"),
    (
        "lock screen",
        "lockscreen password security blur unlock pam",
    ),
    (
        "privacy",
        "location camera microphone permission history geoclue recent flatpak",
    ),
    (
        "launcher",
        "app launcher run search spotlight provider calc ssh",
    ),
    ("menus", "menu popup mdash dashboard"),
    (
        "notifications",
        "notification toast popup do-not-disturb dnd inline reply sound chime quiet hours progress history search",
    ),
    ("general", "avatar profile name user greeting"),
    (
        "tiling_layout",
        "tile tiling gaps layout window split master stack",
    ),
    (
        "tiling layout",
        "tile tiling gaps layout window split master stack",
    ),
    ("plugins", "plugin wasm extension addon"),
    (
        "default_apps",
        "default browser terminal editor mime handler",
    ),
    (
        "default apps",
        "default browser terminal editor mime handler",
    ),
    ("bar", "panel pill widget topbar bottombar status"),
    ("widgets", "widget pill bar component"),
    ("users", "user account password"),
    ("about", "version build info credits"),
    ("setup", "wizard onboarding first-run"),
];

fn keywords_for(label: &str) -> &'static str {
    PAGE_KEYWORDS
        .iter()
        .find(|(k, _)| *k == label)
        .map(|(_, v)| *v)
        .unwrap_or("")
}

/// Search aliases for the sidebar search box: each `(label, route)` makes
/// `label` (matched as a substring of the query) jump to `route`. Multiple
/// aliases can point at the same route. Seeds `search_index` at startup; the
/// widgets sub-pages (`widgets/<name>`) are appended dynamically as their
/// buttons are built.
const SEARCH_ALIASES: &[(&str, &str)] = &[
    ("general", "general"),
    ("setup", "setup"),
    ("bar", "bar"),
    ("bluetooth", "bluetooth"),
    ("default apps", "default_apps"),
    ("display", "display"),
    ("fonts", "theme/fonts"),
    ("idle", "idle"),
    ("on-screen keyboard", "keyboard"),
    ("keyboard", "keyboard"),
    ("lock", "lock"),
    ("lock screen", "lock"),
    ("tiling layout", "tiling_layout"),
    ("tiling_layout", "tiling_layout"),
    ("about", "about"),
    ("animations", "animations"),
    ("appearance", "appearance"),
    ("effects", "effects"),
    ("osd", "osd"),
    ("behaviour", "behaviour"),
    ("behavior", "behaviour"),
    ("logging", "logging"),
    ("logs", "logging"),
    ("window rules", "window_rules"),
    ("windowrule", "window_rules"),
    ("monitors", "display"),
    ("displays", "display"),
    ("layer rules", "layer_rules"),
    ("layerrule", "layer_rules"),
    ("tag rules", "tag_rules"),
    ("tagrule", "tag_rules"),
    ("startup", "startup_env"),
    ("scripts", "startup_env"),
    ("autostart", "startup_env"),
    ("environment", "startup_env"),
    ("env", "startup_env"),
    ("backup", "backup"),
    ("export", "backup"),
    ("import", "backup"),
    ("profile", "backup"),
    ("profiles", "backup"),
    ("reset", "backup"),
    ("restore", "backup"),
    ("date_time", "date_time"),
    ("region", "region"),
    ("sound", "sound"),
    ("users", "users"),
    ("input", "input"),
    ("keybinds", "keybinds"),
    ("tag apps", "summon"),
    ("summon", "summon"),
    ("launcher", "launcher"),
    ("menus", "menus"),
    ("network", "network"),
    ("plugins", "plugins"),
    ("power", "power"),
    ("privacy", "privacy"),
    ("theme", "theme"),
    ("theme scheme", "theme/scheme"),
    ("scheme", "theme/scheme"),
    ("wallpaper", "theme/wallpaper"),
    ("theme wallpaper", "theme/wallpaper"),
    ("app themes", "theme/apps"),
    ("apps theme", "theme/apps"),
    ("theme apps", "theme/apps"),
    ("helium", "theme/apps"),
    ("browser theme", "theme/apps"),
    ("widgets", "widgets"),
];

/// One row of the sidebar: a group header or a page button.
enum SidebarEntry {
    Section {
        name: &'static str,
        icon: &'static str,
    },
    Page {
        route: &'static str,
        icon: &'static str,
        label: &'static str,
    },
}

use SidebarEntry::{Page, Section};

/// The sidebar layout in display order. Built imperatively (relm4's `view!`
/// can't loop), so adding a page is one row here + one stack page + search
/// entries — no hand-wired named widget / button-lookup arm.
const SIDEBAR: &[SidebarEntry] = &[
    Page {
        route: "general",
        icon: "settings-symbolic",
        label: "General",
    },
    Section {
        name: "Appearance",
        icon: "applications-graphics-symbolic",
    },
    Page {
        route: "appearance",
        icon: "preferences-desktop-display-symbolic",
        label: "Appearance",
    },
    Page {
        route: "effects",
        icon: "applications-graphics-symbolic",
        label: "Effects",
    },
    Page {
        route: "osd",
        icon: "audio-volume-high-symbolic",
        label: "On-screen Displays",
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
        icon: "view-grid-symbolic",
        label: "Tag Rules",
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
        route: "behaviour",
        icon: "preferences-system-symbolic",
        label: "Behaviour",
    },
    Page {
        route: "animations",
        icon: "preferences-desktop-screensaver-symbolic",
        label: "Animations",
    },
    Page {
        route: "theme",
        icon: "palette-symbolic",
        label: "Theme",
    },
    Section {
        name: "Shell & Desktop",
        icon: "sidebar-symbolic",
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
        route: "overview",
        icon: "view-grid-symbolic",
        label: "Overview",
    },
    Page {
        route: "vpn",
        icon: "network-vpn-symbolic",
        label: "VPN",
    },
    Page {
        route: "ai",
        icon: "starred-symbolic",
        label: "AI",
    },
    Page {
        route: "tiling_layout",
        icon: "view-grid-symbolic",
        label: "Tiling Layout",
    },
    Page {
        route: "widgets",
        icon: "view-grid-symbolic",
        label: "Widgets",
    },
    Section {
        name: "System & Devices",
        icon: "preferences-system-symbolic",
    },
    Page {
        route: "bluetooth",
        icon: "bluetooth-active-symbolic",
        label: "Bluetooth",
    },
    Page {
        route: "display",
        icon: "video-display-symbolic",
        label: "Display",
    },
    Page {
        route: "idle",
        icon: "coffee-symbolic",
        label: "Idle",
    },
    Page {
        route: "keyboard",
        icon: "input-keyboard-symbolic",
        label: "On-Screen Keyboard",
    },
    Page {
        route: "lock",
        icon: "system-lock-screen-symbolic",
        label: "Lock Screen",
    },
    Page {
        route: "network",
        icon: "network-wireless-symbolic",
        label: "Network",
    },
    Page {
        route: "power",
        icon: "battery-symbolic",
        label: "Power",
    },
    Page {
        route: "privacy",
        icon: "security-high-symbolic",
        label: "Privacy",
    },
    Page {
        route: "sound",
        icon: "audio-volume-high-symbolic",
        label: "Sound",
    },
    Section {
        name: "Input & Shortcuts",
        icon: "input-keyboard-symbolic",
    },
    Page {
        route: "input",
        icon: "input-keyboard-symbolic",
        label: "Input",
    },
    Page {
        route: "keybinds",
        icon: "preferences-desktop-keyboard-shortcuts-symbolic",
        label: "Keyboard Shortcuts",
    },
    Page {
        route: "summon",
        icon: "view-app-grid-symbolic",
        label: "Tags",
    },
    Page {
        route: "launcher",
        icon: "system-search-symbolic",
        label: "Launcher",
    },
    Section {
        name: "Locale & Accounts",
        icon: "preferences-desktop-locale-symbolic",
    },
    Page {
        route: "date_time",
        icon: "preferences-system-time-symbolic",
        label: "Date & Time",
    },
    Page {
        route: "default_apps",
        icon: "application-x-executable-symbolic",
        label: "Default Apps",
    },
    Page {
        route: "region",
        icon: "preferences-desktop-locale-symbolic",
        label: "Region & Language",
    },
    Page {
        route: "users",
        icon: "system-users-symbolic",
        label: "Users",
    },
    Section {
        name: "Advanced",
        icon: "emblem-system-symbolic",
    },
    Page {
        route: "plugins",
        icon: "application-x-addon-symbolic",
        label: "Plugins",
    },
    Page {
        route: "setup",
        icon: "emblem-system-symbolic",
        label: "Setup",
    },
    Section {
        name: "About",
        icon: "help-about-symbolic",
    },
    Page {
        route: "about",
        icon: "help-about-symbolic",
        label: "About",
    },
];

const SETTINGS_SIDEBAR_LABEL_MIN_HEIGHT: i32 = 28;
const SETTINGS_SIDEBAR_TITLE_MIN_HEIGHT: i32 = 28;
const SETTINGS_SIDEBAR_TEXT_RISE: i32 = -1024;

fn settings_sidebar_markup(label: &str) -> String {
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

fn settings_sidebar_label(label: &str) -> gtk::Label {
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

fn settings_sidebar_title_label(label: &str) -> gtk::Label {
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
fn sidebar_state_path() -> std::path::PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".cache")))
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
    base.join("mshell").join("settings_sidebar")
}

fn load_collapsed_sections() -> std::collections::HashSet<String> {
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

fn set_section_collapsed(name: &str, collapsed: bool) {
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
fn build_section_group(
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
    lbl.set_markup(&settings_sidebar_markup(name));
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
fn collect_sidebar_buttons(
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
fn build_sidebar(
    sidebar_box: &gtk::Box,
    stack: &gtk::Stack,
) -> std::collections::HashMap<String, gtk::ToggleButton> {
    use gtk::prelude::*;
    let mut buttons = std::collections::HashMap::new();
    let mut anchor: Option<gtk::ToggleButton> = None;
    let collapsed = load_collapsed_sections();
    // Where the next page button goes. Starts at the sidebar root (for any
    // ungrouped page before the first section — e.g. General), then points
    // at each section group's revealer-pages box.
    let mut current_pages: gtk::Box = sidebar_box.clone();
    // The current group's revealer + chevron, so activating one of its pages
    // (via search / IPC) auto-expands the group so the page is visible.
    let mut current_reveal: Option<(gtk::Revealer, gtk::Image)> = None;

    for entry in SIDEBAR {
        match entry {
            Section { name, icon } => {
                let expanded = !collapsed.contains(*name);
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
                btn.connect_toggled(move |b| {
                    if b.is_active() {
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
mod registry_tests {
    use super::{PAGE_KEYWORDS, SEARCH_ALIASES, keywords_for};
    use std::collections::HashSet;

    #[test]
    fn search_alias_labels_are_unique() {
        // A duplicate label would make the second entry dead (first match wins),
        // a silent registry bug. Routes may repeat (aliases), labels must not.
        let mut seen = HashSet::new();
        for (label, _) in SEARCH_ALIASES {
            assert!(seen.insert(*label), "duplicate search alias label: {label}");
        }
    }

    #[test]
    fn page_keyword_labels_are_unique_and_nonempty() {
        let mut seen = HashSet::new();
        for (label, kw) in PAGE_KEYWORDS {
            assert!(seen.insert(*label), "duplicate keyword label: {label}");
            assert!(!kw.is_empty(), "empty keywords for {label}");
        }
    }

    #[test]
    fn keywords_lookup_matches_table() {
        assert_eq!(keywords_for("power"), PAGE_KEYWORDS[0].1);
        // OR-arm spellings resolve to the same keywords.
        assert_eq!(keywords_for("behaviour"), keywords_for("behavior"));
        assert_eq!(keywords_for("date_time"), keywords_for("date & time"));
        // Unknown label → empty (sidebar filter falls back to label match).
        assert_eq!(keywords_for("nonexistent"), "");
    }

    #[test]
    fn every_search_route_is_a_known_page() {
        // Guard the "wrong route / typo" class of registry bug: every alias
        // target must be one of the real stack-child routes. (The widgets
        // route covers all `widgets/<name>` sub-pages.)
        let routes: HashSet<&str> = SEARCH_ALIASES.iter().map(|(_, r)| *r).collect();
        for r in &routes {
            assert!(!r.is_empty(), "empty route in search registry");
        }
        // Routes that have keywords should also be searchable (consistency
        // sample — not all keyword labels are top-level searchable, e.g.
        // notifications is a widget sub-page).
        for label in ["power", "network", "display", "theme"] {
            assert!(
                routes.contains(label),
                "{label} missing from search aliases"
            );
        }
    }
}
