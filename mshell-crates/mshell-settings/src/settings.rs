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
use crate::calendar_settings::{CalendarSettingsInit, CalendarSettingsModel};
use crate::catwalk_settings::{CatwalkSettingsInit, CatwalkSettingsModel};
use crate::date_time_settings::{DateTimeSettingsInit, DateTimeSettingsModel};
use crate::default_apps_settings::{DefaultAppsSettingsInit, DefaultAppsSettingsModel};
use crate::display_settings::{DisplaySettingsInit, DisplaySettingsModel};
use crate::effects_settings::{EffectsInit, EffectsModel};
use crate::fonts_settings::{FontsSettingsInit, FontsSettingsModel};
use crate::game_mode_settings::{GameModeSettingsInit, GameModeSettingsModel};
use crate::general_settings::{GeneralSettingsInit, GeneralSettingsModel};
use crate::guide_settings::{GuideSettingsInit, GuideSettingsModel};
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
use crate::login_settings::{LoginSettingsInit, LoginSettingsModel};
use crate::media_player_settings::{MediaPlayerSettingsInit, MediaPlayerSettingsModel};
use crate::menu_settings::menu_settings::{MenuSettingsInit, MenuSettingsModel};
use crate::mtune_settings::{MtuneSettingsInit, MtuneSettingsModel};
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
use crate::toast_settings::{ToastSettingsInit, ToastSettingsModel};
use crate::translate_settings::{TranslateSettingsInit, TranslateSettingsModel};
use crate::users_settings::{UsersSettingsInit, UsersSettingsModel};
use crate::vpn_settings::{VpnSettingsInit, VpnSettingsModel};
use crate::wallpaper_settings::{WallpaperSettingsInit, WallpaperSettingsModel};
use crate::weather_settings::{WeatherSettingsInit, WeatherSettingsModel};
use crate::widget_menu_settings::{MenuKind, WidgetMenuSettingsInit, WidgetMenuSettingsModel};
use crate::window_rules_settings::{WindowRulesInit, WindowRulesModel};
use crate::window_switcher_settings::{WindowSwitcherInit, WindowSwitcherModel};
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
use relm4::{Component, ComponentController, ComponentParts, ComponentSender, gtk};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

// A one-controller page factory: launches the model on first
// navigation and returns its widget + the controller as keep-alive
// token. Used by the theme registry in `init` and by `widget_pages`;
// composed pages (Weather / Lyrics / …) spell their factories out.
macro_rules! page_factory {
    ($model:path => $init:expr) => {
        Rc::new(move || {
            let ctrl = <$model>::builder().launch($init).detach();
            let widget: gtk::Widget = ctrl.widget().clone().into();
            (widget, vec![Box::new(ctrl) as Box<dyn std::any::Any>])
        }) as PageFactory
    };
}

mod pages;
mod search;
mod sidebar;
mod widget_pages;
use pages::*;
use search::*;
use sidebar::*;
use widget_pages::*;

pub struct SettingsWindowModel {
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
    /// Keep-alive anchor for every lazily-built page. Pages are built on first
    /// navigation (see the `ensure_*` closures in `init`); a page's relm4
    /// `Controller` must outlive the panel or the page stops updating, so it is
    /// parked here rather than dropped when the builder closure returns. Shared
    /// `Rc` because those closures push into it from GTK signal handlers. Never
    /// read — the model owning the tokens is the whole point (`dead_code`).
    #[allow(dead_code)]
    alive: Alive,
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
                        // KDE's "Highlight Changed Settings": marks every row
                        // whose value differs from its default.
                        #[name = "highlight_toggle"]
                        gtk::ToggleButton {
                            add_css_class: "settings-highlight-toggle",
                            set_icon_name: "document-edit-symbolic",
                            set_tooltip_text: Some("Highlight changed settings"),
                            set_valign: gtk::Align::Center,
                            set_active: crate::page_defaults::highlight_enabled(),
                            connect_toggled => |b| crate::page_defaults::set_highlight(b.is_active()),
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

        // Every settings page is now built on first navigation, not up front:
        // opening the panel used to launch ~100 relm4 controllers in one
        // blocking call (the 3–4 s freeze). Each page's controller lands here
        // as it is built so it outlives the panel (a detached `Controller`
        // shuts its component down when dropped). See the `ensure_*` closures.
        let alive: Alive = Rc::new(RefCell::new(HashMap::new()));

        // Any config / profile change marks every built page stale (see
        // `Stale`); pages are rebuilt lazily, so this costs nothing until one
        // is shown again.
        let stale: Stale = Rc::new(RefCell::new(std::collections::HashSet::new()));
        {
            use reactive_graph::traits::Track;
            let mut stale_effects = EffectScope::new();
            let (alive, stale) = (alive.clone(), stale.clone());
            stale_effects.push(move |_| {
                config_manager().config().track();
                config_manager().active_profile().track();
                stale.borrow_mut().extend(alive.borrow().keys().cloned());
            });
            Box::leak(Box::new(stale_effects));
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
        let search_index: Rc<RefCell<Vec<(String, String)>>> =
            Rc::new(RefCell::new(initial_search_index()));
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
            panel_width,
            panel_height,
            subsection_buttons: subsection_buttons.clone(),
            section_buttons: section_buttons.clone(),
            search_index: search_index.clone(),
            search_titles: search_titles.clone(),
            alive: alive.clone(),
        };

        let widgets = view_output!();

        // Build one top-level page into the stack on first navigation. The
        // `theme` / `widgets` container pages are added eagerly (their
        // sub-sidebars seed the search index), so `child_by_name` short-circuits
        // for them; every other route resolves through `build_top_page`.
        let ensure_top: Rc<dyn Fn(&str)> = {
            let stack = widgets.stack.clone();
            let alive = alive.clone();
            let stale = stale.clone();
            Rc::new(move |route: &str| {
                if !needs_build(stack.child_by_name(route).is_some(), &stale, route) {
                    return;
                }
                let owned = route.to_string();
                let build: Rc<dyn Fn() -> Option<BuiltPage>> =
                    Rc::new(move || build_top_page(&owned));
                mount_page(&stack, route, route, build, &alive, false);
            })
        };

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

        // Theme sub-pages are built on first navigation into each entry. The
        // buttons + search index below stay eager (search must reach a theme
        // page that was never opened); only the page content is deferred.
        let theme_registry: Rc<RefCell<HashMap<String, PageFactory>>> =
            Rc::new(RefCell::new(HashMap::new()));
        let ensure_theme: Rc<dyn Fn(&str)> = {
            let sub_stack = theme_sub_stack.clone();
            let registry = theme_registry.clone();
            let alive = alive.clone();
            let stale = stale.clone();
            Rc::new(move |name: &str| {
                let route = format!("theme/{name}");
                if !needs_build(sub_stack.child_by_name(name).is_some(), &stale, &route) {
                    return;
                }
                let factory = registry.borrow().get(name).cloned();
                if let Some(factory) = factory {
                    let build: Rc<dyn Fn() -> Option<BuiltPage>> = Rc::new(move || Some(factory()));
                    mount_page(
                        &sub_stack,
                        name,
                        &format!("theme/{name}"),
                        build,
                        &alive,
                        false,
                    );
                }
            })
        };

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
            let ensure = ensure_theme.clone();
            btn.connect_toggled(move |b| {
                if b.is_active() {
                    ensure(stack_name);
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

        theme_registry.borrow_mut().extend([
            (
                "scheme".to_string(),
                page_factory!(ThemeSettingsModel => ThemeSettingsInit {}),
            ),
            (
                "fonts".to_string(),
                page_factory!(FontsSettingsModel => FontsSettingsInit {}),
            ),
            (
                "wallpaper".to_string(),
                page_factory!(WallpaperSettingsModel => WallpaperSettingsInit {}),
            ),
            (
                "apps".to_string(),
                page_factory!(HeliumThemeSettingsModel => HeliumThemeSettingsInit {}),
            ),
        ]);

        let theme_entries: [(&str, &str, &'static str); 4] = [
            ("Scheme", "palette-symbolic", "scheme"),
            ("Fonts", "xsi-font-symbolic", "fonts"),
            ("Wallpaper", "wallpaper-symbolic", "wallpaper"),
            ("Apps", "web-browser-symbolic", "apps"),
        ];
        let mut theme_anchor: Option<gtk::ToggleButton> = None;
        for (label, icon, stack_name) in theme_entries {
            let btn = make_theme_sub_btn(label, icon, stack_name, theme_anchor.as_ref());
            if theme_anchor.is_none() {
                theme_anchor = Some(btn.clone());
            }
            theme_sub_sidebar_box.append(&btn);
        }
        theme_page.append(&theme_sub_sidebar);
        theme_page.append(&theme_sub_stack);

        // Build the default sub-page ("scheme") the first time the Theme
        // container is shown, so its sub-stack isn't blank — but only when
        // empty, so re-entering Theme keeps whichever sub-page the user last
        // left it on (the sub-stack retains its visible child otherwise).
        {
            let ensure_theme = ensure_theme.clone();
            let sub_stack = theme_sub_stack.clone();
            theme_page.connect_map(move |_| {
                if sub_stack.visible_child().is_none() {
                    ensure_theme("scheme");
                    sub_stack.set_visible_child_name("scheme");
                }
            });
        }

        // The Theme container is a plain Box of sub-sidebar + (initially empty)
        // sub-stack, so it stays eager: its sub-sidebar buttons seed the search
        // index that must resolve before Theme is ever opened.
        widgets
            .stack
            .add_titled(&theme_page, Some("theme"), "Theme");

        // Every other top-level page is built lazily on first navigation (see
        // `ensure_top` + `build_top_page`); only the `theme` and `widgets`
        // containers are added to the stack up front.

        // Build the sidebar buttons + section headers imperatively from the
        // SIDEBAR table (relm4's view! can't loop). Each page button's toggle
        // runs `ensure_top` before flipping the stack, so the target page is
        // built on demand. The default "general" page is built explicitly at
        // the end of `init` (the anchor button's toggle isn't wired yet here).
        *section_buttons.borrow_mut() =
            build_sidebar(&widgets.sidebar_box, &widgets.stack, ensure_top.clone());

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

        // Widget sub-pages are built on first navigation. The loop below fills
        // this registry with one factory per stack-child name; `ensure_widget`
        // runs the factory the first time that page is shown. The sub-sidebar
        // buttons + search index stay eager (search must reach a widget page
        // that was never opened).
        let widget_registry: Rc<RefCell<HashMap<String, PageFactory>>> =
            Rc::new(RefCell::new(HashMap::new()));
        let ensure_widget: Rc<dyn Fn(&str)> = {
            let sub_stack = widgets_sub_stack.clone();
            let registry = widget_registry.clone();
            let alive = alive.clone();
            let stale = stale.clone();
            Rc::new(move |name: &str| {
                let route = format!("widgets/{name}");
                if !needs_build(sub_stack.child_by_name(name).is_some(), &stale, &route) {
                    return;
                }
                let factory = registry.borrow().get(name).cloned();
                if let Some(factory) = factory {
                    let build: Rc<dyn Fn() -> Option<BuiltPage>> = Rc::new(move || Some(factory()));
                    mount_page(
                        &sub_stack,
                        name,
                        &format!("widgets/{name}"),
                        build,
                        &alive,
                        false,
                    );
                }
            })
        };

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
            let ensure = ensure_widget.clone();
            btn.connect_toggled(move |b| {
                if b.is_active() {
                    ensure(stack_name);
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

        let entries = widget_entries();

        // The default sub-page shown when the Widgets container is first
        // opened: the alphabetically-first entry, which is also the radio-group
        // anchor built `.active(true)` below. Built lazily on first map.
        let widgets_default: Option<&'static str> = entries.first().map(WidgetEntry::stack_name);

        // Each entry builds its sub-sidebar button eagerly and registers a
        // one-shot page factory (see `widget_pages`).
        register_widget_entries(
            entries,
            &make_sub_btn,
            &widgets_sub_sidebar_box,
            &widget_registry,
        );

        widgets_page.append(&widgets_sub_sidebar);
        widgets_page.append(&widgets_sub_stack);

        // Build the default sub-page the first time the Widgets container is
        // shown, so its sub-stack isn't blank — only when empty, so re-entering
        // Widgets keeps whichever sub-page the user last left it on (the
        // sub-stack retains its visible child otherwise).
        if let Some(default_name) = widgets_default {
            let ensure_widget = ensure_widget.clone();
            let sub_stack = widgets_sub_stack.clone();
            widgets_page.connect_map(move |_| {
                if sub_stack.visible_child().is_none() {
                    ensure_widget(default_name);
                    sub_stack.set_visible_child_name(default_name);
                }
            });
        }

        // The Widgets container stays eager for the same reason as Theme: its
        // sub-sidebar buttons seed the search index. Only its page content is
        // deferred.
        widgets
            .stack
            .add_titled(&widgets_page, Some("widgets"), "Widgets");

        // Show the default page ("general") now: the sidebar's anchor button is
        // active, but its toggle handler was wired only after `set_active` ran,
        // so nothing has built the page yet. Every later navigation flows
        // through a button toggle -> `ensure_top`.
        ensure_top("general");
        widgets.stack.set_visible_child_name("general");

        // Showing the panel again (or on another monitor) rebuilds the
        // visible top-level page if the config changed meanwhile.
        {
            use gtk::prelude::WidgetExt;
            let stack = widgets.stack.clone();
            let (ensure_top, stale) = (ensure_top.clone(), stale.clone());
            root.connect_map(move |_| {
                let Some(route) = stack.visible_child_name() else {
                    return;
                };
                if stale.borrow().contains(route.as_str()) {
                    ensure_top(&route);
                    stack.set_visible_child_name(&route);
                }
            });
        }

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
                        crate::page_defaults::highlight_search(&route, &q);
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
                        let query = q.clone();
                        btn.connect_clicked(move |_| {
                            crate::page_defaults::highlight_search(&route, &query);
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
