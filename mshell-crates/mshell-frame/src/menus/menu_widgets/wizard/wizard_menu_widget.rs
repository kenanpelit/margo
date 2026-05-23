//! In-shell setup wizard — a layer-shell MENU, never a floating window.
//!
//! Hosts the five-step first-run flow (Welcome → Theme → Keyboard →
//! Wallpaper → Done) inside a `gtk::Stack`, exactly like every other
//! mshell menu surface. Apply writes the choices LIVE through
//! `config_manager` (theme / font / clock / wallpaper) plus the xkb lines
//! in the compositor's `config.conf`, then closes the menu. Reachable
//! from the Settings → Setup button, `mshellctl wizard`, and the
//! first-launch auto-open.

use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    BarsStoreFields, ConfigStoreFields, GeneralStoreFields, HorizontalBarStoreFields,
    MatugenStoreFields, SizingStoreFields, ThemeAttributesStoreFields, ThemeStoreFields,
    WallpaperStoreFields,
};
use mshell_config::schema::themes::{MatugenMode, Themes};
use mshell_utils::session::{SessionAction, run_session_action};
use reactive_graph::prelude::GetUntracked;
use relm4::gtk::prelude::{
    BoxExt, ButtonExt, EditableExt, EntryExt, FileExt, OrientableExt, WidgetExt,
};
use relm4::gtk::{gio, glib};
use relm4::{ComponentParts, ComponentSender, SimpleComponent, gtk};
use std::path::PathBuf;

// Welcome, Theme, Keyboard, Touchpad, Network, Wallpaper, Bar, Review.
const PAGES: usize = 8;

/// Curated theme presets (full catalogue lives in Settings → Theme).
const THEMES: &[(Themes, &str)] = &[
    (Themes::Wallpaper, "Wallpaper (Material You)"),
    (Themes::Default, "Default"),
    (Themes::Margo, "Margo"),
    (Themes::Dracula, "Dracula"),
    (Themes::CatppuccinMocha, "Catppuccin Mocha"),
    (Themes::GruvboxDarkMedium, "Gruvbox Dark"),
    (Themes::KanagawaWave, "Kanagawa Wave"),
    (Themes::Cyberpunk, "Cyberpunk"),
];

const FONT_SCALES: &[(f64, &str)] = &[
    (0.9, "Compact (90%)"),
    (1.0, "Default (100%)"),
    (1.1, "Large (110%)"),
    (1.25, "Larger (125%)"),
];

/// `(xkb code, display name)`, common-first.
const LAYOUTS: &[(&str, &str)] = &[
    ("us", "English (US)"),
    ("gb", "English (UK)"),
    ("tr", "Türkçe"),
    ("de", "Deutsch"),
    ("fr", "Français"),
    ("es", "Español"),
    ("it", "Italiano"),
    ("ru", "Русский"),
    ("ua", "Українська"),
    ("ar", "العربية"),
];

/// Curated `xkb_rules_options` (single-pick; the field accepts more via
/// `mctl config edit`). `(option code, display name)`, "none" first.
const XKB_OPTIONS: &[(&str, &str)] = &[
    ("", "None"),
    ("ctrl:nocaps", "Caps Lock → Ctrl"),
    ("ctrl:swapcaps", "Swap Caps Lock ↔ Ctrl"),
    ("caps:escape", "Caps Lock → Escape"),
    ("altwin:swap_alt_win", "Swap Alt ↔ Super"),
    ("compose:ralt", "Right Alt → Compose"),
    ("grp:alt_shift_toggle", "Toggle layout: Alt+Shift"),
];

fn theme_names() -> Vec<&'static str> {
    THEMES.iter().map(|(_, n)| *n).collect()
}
fn font_names() -> Vec<&'static str> {
    FONT_SCALES.iter().map(|(_, n)| *n).collect()
}
fn layout_names() -> Vec<&'static str> {
    LAYOUTS.iter().map(|(_, n)| *n).collect()
}
fn option_names() -> Vec<&'static str> {
    XKB_OPTIONS.iter().map(|(_, n)| *n).collect()
}

pub(crate) struct WizardMenuWidgetModel {
    page: usize,
    mode: MatugenMode,
    theme_scheme: Themes,
    font_scale: f64,
    clock_24h: bool,
    xkb_layout: String,
    xkb_variant: String,
    xkb_options: String,
    tap_to_click: bool,
    natural_scroll: bool,
    disable_while_typing: bool,
    /// Scanned SSIDs (Wi-Fi step). Empty until the first list load.
    wifi_networks: Vec<String>,
    wifi_selected: usize,
    wifi_password: String,
    /// Free-text status line under the Connect button.
    wifi_status: String,
    wallpaper_dir: String,
    /// `false` = main bar on top (default), `true` = on the bottom.
    bar_at_bottom: bool,
    /// Set once the final step has written + reloaded. Flips the last
    /// page into its "applied — reboot?" state.
    applied: bool,
}

#[derive(Debug)]
pub(crate) enum WizardMenuWidgetInput {
    Next,
    Back,
    Cancel,
    ModeChanged(MatugenMode),
    ThemeChanged(Themes),
    FontScaleChanged(f64),
    Clock24hToggled(bool),
    XkbLayoutChanged(String),
    XkbVariantChanged(String),
    XkbOptionsChanged(String),
    TapToClickToggled(bool),
    NaturalScrollToggled(bool),
    DisableWhileTypingToggled(bool),
    ScanWifi,
    WifiListLoaded(Vec<String>),
    WifiSelected(usize),
    WifiPasswordChanged(String),
    ConnectWifi,
    WifiStatus(String),
    BrowseWallpaper,
    WallpaperPicked(String),
    BarPositionChanged(bool),
    Reboot,
}

#[derive(Debug)]
pub(crate) enum WizardMenuWidgetOutput {
    CloseMenu,
}

pub(crate) struct WizardMenuWidgetInit {}

#[relm4::component(pub)]
impl SimpleComponent for WizardMenuWidgetModel {
    type Input = WizardMenuWidgetInput;
    type Output = WizardMenuWidgetOutput;
    type Init = WizardMenuWidgetInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "wizard-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 16,
            set_hexpand: true,
            set_width_request: 440,

            gtk::Label {
                add_css_class: "label-small",
                set_halign: gtk::Align::Start,
                #[watch]
                set_label: &format!("Step {} of {}", model.page + 1, PAGES),
            },

            #[name = "stack"]
            gtk::Stack {
                set_vexpand: true,
                set_transition_type: gtk::StackTransitionType::SlideLeftRight,
                set_transition_duration: 180,
                #[watch]
                set_visible_child_name: &model.page.to_string(),

                // ── 0 Welcome ─────────────────────────────────
                add_named[Some("0")] = &gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 8,
                    set_valign: gtk::Align::Center,
                    gtk::Label {
                        add_css_class: "settings-hero-title",
                        set_label: "Welcome to margo",
                        set_halign: gtk::Align::Start,
                    },
                    gtk::Label {
                        add_css_class: "label-small",
                        set_label: "A few quick choices to set up your shell. Everything applies live and can be changed later in Settings.",
                        set_halign: gtk::Align::Start,
                        set_xalign: 0.0,
                        set_wrap: true,
                    },
                },

                // ── 1 Theme ───────────────────────────────────
                add_named[Some("1")] = &gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 12,
                    gtk::Label { add_css_class: "label-large-bold", set_label: "Theme", set_halign: gtk::Align::Start },

                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 16,
                        gtk::Label { add_css_class: "label-medium", set_label: "Color mode", set_halign: gtk::Align::Start, set_hexpand: true },
                        gtk::DropDown {
                            set_valign: gtk::Align::Center,
                            set_model: Some(&gtk::StringList::new(&["Dark", "Light"])),
                            #[watch]
                            set_selected: match model.mode { MatugenMode::Dark => 0, MatugenMode::Light => 1 },
                            connect_selected_notify[sender] => move |dd| {
                                sender.input(WizardMenuWidgetInput::ModeChanged(
                                    if dd.selected() == 0 { MatugenMode::Dark } else { MatugenMode::Light },
                                ));
                            },
                        },
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 16,
                        gtk::Label { add_css_class: "label-medium", set_label: "Theme", set_halign: gtk::Align::Start, set_hexpand: true },
                        gtk::DropDown {
                            set_valign: gtk::Align::Center,
                            set_model: Some(&gtk::StringList::new(&theme_names())),
                            #[watch]
                            set_selected: THEMES.iter().position(|(t, _)| *t == model.theme_scheme).unwrap_or(0) as u32,
                            connect_selected_notify[sender] => move |dd| {
                                if let Some((t, _)) = THEMES.get(dd.selected() as usize) {
                                    sender.input(WizardMenuWidgetInput::ThemeChanged(*t));
                                }
                            },
                        },
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 16,
                        gtk::Label { add_css_class: "label-medium", set_label: "Font size", set_halign: gtk::Align::Start, set_hexpand: true },
                        gtk::DropDown {
                            set_valign: gtk::Align::Center,
                            set_model: Some(&gtk::StringList::new(&font_names())),
                            #[watch]
                            set_selected: FONT_SCALES.iter().position(|(v, _)| (*v - model.font_scale).abs() < 0.001).unwrap_or(1) as u32,
                            connect_selected_notify[sender] => move |dd| {
                                if let Some((v, _)) = FONT_SCALES.get(dd.selected() as usize) {
                                    sender.input(WizardMenuWidgetInput::FontScaleChanged(*v));
                                }
                            },
                        },
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 16,
                        gtk::Label { add_css_class: "label-medium", set_label: "24-hour clock", set_halign: gtk::Align::Start, set_hexpand: true },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            set_active: model.clock_24h,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(WizardMenuWidgetInput::Clock24hToggled(v));
                                glib::Propagation::Proceed
                            },
                        },
                    },
                },

                // ── 2 Keyboard ────────────────────────────────
                add_named[Some("2")] = &gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 12,
                    gtk::Label { add_css_class: "label-large-bold", set_label: "Keyboard", set_halign: gtk::Align::Start },
                    gtk::Label {
                        add_css_class: "label-small",
                        set_label: "xkb layout the compositor loads at startup. Use the variant field for anything xkbcommon understands.",
                        set_halign: gtk::Align::Start, set_xalign: 0.0, set_wrap: true,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 16,
                        gtk::Label { add_css_class: "label-medium", set_label: "Layout", set_halign: gtk::Align::Start, set_hexpand: true },
                        gtk::DropDown {
                            set_valign: gtk::Align::Center,
                            set_model: Some(&gtk::StringList::new(&layout_names())),
                            #[watch]
                            set_selected: LAYOUTS.iter().position(|(c, _)| *c == model.xkb_layout).unwrap_or(0) as u32,
                            connect_selected_notify[sender] => move |dd| {
                                if let Some((c, _)) = LAYOUTS.get(dd.selected() as usize) {
                                    sender.input(WizardMenuWidgetInput::XkbLayoutChanged((*c).to_string()));
                                }
                            },
                        },
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 16,
                        gtk::Label { add_css_class: "label-medium", set_label: "Variant (optional)", set_halign: gtk::Align::Start, set_hexpand: true },
                        gtk::Entry {
                            set_valign: gtk::Align::Center,
                            set_placeholder_text: Some("e.g. dvorak, f"),
                            connect_changed[sender] => move |e| {
                                sender.input(WizardMenuWidgetInput::XkbVariantChanged(e.text().to_string()));
                            },
                        },
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 16,
                        gtk::Label { add_css_class: "label-medium", set_label: "Options", set_halign: gtk::Align::Start, set_hexpand: true },
                        gtk::DropDown {
                            set_valign: gtk::Align::Center,
                            set_model: Some(&gtk::StringList::new(&option_names())),
                            #[watch]
                            set_selected: XKB_OPTIONS.iter().position(|(c, _)| *c == model.xkb_options).unwrap_or(0) as u32,
                            connect_selected_notify[sender] => move |dd| {
                                if let Some((c, _)) = XKB_OPTIONS.get(dd.selected() as usize) {
                                    sender.input(WizardMenuWidgetInput::XkbOptionsChanged((*c).to_string()));
                                }
                            },
                        },
                    },
                },

                // ── 3 Touchpad ────────────────────────────────
                add_named[Some("3")] = &gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 12,
                    gtk::Label { add_css_class: "label-large-bold", set_label: "Touchpad & mouse", set_halign: gtk::Align::Start },
                    gtk::Label {
                        add_css_class: "label-small",
                        set_label: "Pointer behaviour the compositor applies on startup.",
                        set_halign: gtk::Align::Start, set_xalign: 0.0, set_wrap: true,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal, set_spacing: 16,
                        gtk::Label { add_css_class: "label-medium", set_label: "Tap to click", set_halign: gtk::Align::Start, set_hexpand: true },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            set_active: model.tap_to_click,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(WizardMenuWidgetInput::TapToClickToggled(v));
                                glib::Propagation::Proceed
                            },
                        },
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal, set_spacing: 16,
                        gtk::Label { add_css_class: "label-medium", set_label: "Natural scrolling", set_halign: gtk::Align::Start, set_hexpand: true },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            set_active: model.natural_scroll,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(WizardMenuWidgetInput::NaturalScrollToggled(v));
                                glib::Propagation::Proceed
                            },
                        },
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal, set_spacing: 16,
                        gtk::Label { add_css_class: "label-medium", set_label: "Disable while typing", set_halign: gtk::Align::Start, set_hexpand: true },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            set_active: model.disable_while_typing,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(WizardMenuWidgetInput::DisableWhileTypingToggled(v));
                                glib::Propagation::Proceed
                            },
                        },
                    },
                },

                // ── 4 Network ─────────────────────────────────
                add_named[Some("4")] = &gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 12,
                    gtk::Label { add_css_class: "label-large-bold", set_label: "Wi-Fi", set_halign: gtk::Align::Start },
                    gtk::Label {
                        add_css_class: "label-small",
                        set_label: "Optional — pick a network and connect now, or skip and do it later from the bar.",
                        set_halign: gtk::Align::Start, set_xalign: 0.0, set_wrap: true,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal, set_spacing: 8,
                        gtk::DropDown {
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            #[watch]
                            set_model: Some(&gtk::StringList::new(
                                &if model.wifi_networks.is_empty() {
                                    vec!["Scan for networks…"]
                                } else {
                                    model.wifi_networks.iter().map(|s| s.as_str()).collect::<Vec<_>>()
                                },
                            )),
                            #[watch]
                            set_sensitive: !model.wifi_networks.is_empty(),
                            #[watch]
                            set_selected: model.wifi_selected as u32,
                            connect_selected_notify[sender] => move |dd| {
                                sender.input(WizardMenuWidgetInput::WifiSelected(dd.selected() as usize));
                            },
                        },
                        gtk::Button {
                            set_css_classes: &["label-medium"],
                            set_label: "Scan",
                            set_valign: gtk::Align::Center,
                            connect_clicked[sender] => move |_| sender.input(WizardMenuWidgetInput::ScanWifi),
                        },
                    },
                    gtk::Entry {
                        set_placeholder_text: Some("Password (leave blank if open / saved)"),
                        set_visibility: false,
                        #[watch]
                        set_sensitive: !model.wifi_networks.is_empty(),
                        connect_changed[sender] => move |e| {
                            sender.input(WizardMenuWidgetInput::WifiPasswordChanged(e.text().to_string()));
                        },
                    },
                    gtk::Button {
                        set_css_classes: &["label-medium", "ok-button-primary"],
                        set_label: "Connect",
                        set_halign: gtk::Align::Start,
                        #[watch]
                        set_sensitive: !model.wifi_networks.is_empty(),
                        connect_clicked[sender] => move |_| sender.input(WizardMenuWidgetInput::ConnectWifi),
                    },
                    gtk::Label {
                        add_css_class: "label-small",
                        #[watch]
                        set_label: &model.wifi_status,
                        #[watch]
                        set_visible: !model.wifi_status.is_empty(),
                        set_halign: gtk::Align::Start, set_xalign: 0.0, set_wrap: true,
                    },
                },

                // ── 5 Wallpaper ───────────────────────────────
                add_named[Some("5")] = &gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 12,
                    gtk::Label { add_css_class: "label-large-bold", set_label: "Wallpaper", set_halign: gtk::Align::Start },
                    gtk::Label {
                        add_css_class: "label-small",
                        #[watch]
                        set_label: &model.wallpaper_dir,
                        set_halign: gtk::Align::Start, set_xalign: 0.0, set_wrap: true,
                    },
                    gtk::Button {
                        set_css_classes: &["label-medium", "ok-button-primary"],
                        set_label: "Browse…",
                        set_halign: gtk::Align::Start,
                        connect_clicked[sender] => move |_| sender.input(WizardMenuWidgetInput::BrowseWallpaper),
                    },
                },

                // ── 6 Bar ─────────────────────────────────────
                add_named[Some("6")] = &gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 12,
                    gtk::Label { add_css_class: "label-large-bold", set_label: "Bar", set_halign: gtk::Align::Start },
                    gtk::Label {
                        add_css_class: "label-small",
                        set_label: "Where the main bar sits. Your widgets move with it; fine-tune slots later in Settings → Bar.",
                        set_halign: gtk::Align::Start, set_xalign: 0.0, set_wrap: true,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal, set_spacing: 16,
                        gtk::Label { add_css_class: "label-medium", set_label: "Position", set_halign: gtk::Align::Start, set_hexpand: true },
                        gtk::DropDown {
                            set_valign: gtk::Align::Center,
                            set_model: Some(&gtk::StringList::new(&["Top", "Bottom"])),
                            #[watch]
                            set_selected: u32::from(model.bar_at_bottom),
                            connect_selected_notify[sender] => move |dd| {
                                sender.input(WizardMenuWidgetInput::BarPositionChanged(dd.selected() == 1));
                            },
                        },
                    },
                },

                // ── 7 Review ──────────────────────────────────
                add_named[Some("7")] = &gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 8,
                    gtk::Label {
                        add_css_class: "label-large-bold",
                        #[watch]
                        set_label: if model.applied { "All set" } else { "Review" },
                        set_halign: gtk::Align::Start,
                    },
                    gtk::Label {
                        add_css_class: "label-small",
                        #[watch]
                        set_label: &model.review_text(),
                        set_halign: gtk::Align::Start, set_xalign: 0.0, set_wrap: true,
                    },
                },
            },

            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 8,
                set_halign: gtk::Align::End,
                gtk::Button {
                    set_label: "Cancel",
                    #[watch]
                    set_visible: !model.applied,
                    connect_clicked[sender] => move |_| sender.input(WizardMenuWidgetInput::Cancel),
                },
                gtk::Button {
                    set_label: "Back",
                    #[watch]
                    set_visible: !model.applied,
                    #[watch]
                    set_sensitive: model.page > 0,
                    connect_clicked[sender] => move |_| sender.input(WizardMenuWidgetInput::Back),
                },
                gtk::Button {
                    set_css_classes: &["label-medium", "session-reboot"],
                    set_label: "Reboot now",
                    #[watch]
                    set_visible: model.applied,
                    connect_clicked[sender] => move |_| sender.input(WizardMenuWidgetInput::Reboot),
                },
                gtk::Button {
                    set_css_classes: &["label-medium", "ok-button-primary"],
                    #[watch]
                    set_label: if model.applied {
                        "Close"
                    } else if model.page + 1 == PAGES {
                        "Apply & finish"
                    } else {
                        "Next"
                    },
                    connect_clicked[sender] => move |_| sender.input(WizardMenuWidgetInput::Next),
                },
            },
        }
    }

    fn init(
        _init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = read_live();
        let widgets = view_output!();
        let _ = root;
        // Warm the Wi-Fi list from NetworkManager's last scan (no rescan)
        // so the dropdown is populated by the time the user reaches the
        // Network step. The Scan button forces a fresh rescan.
        spawn_wifi_list(sender.input_sender().clone());
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>) {
        match message {
            WizardMenuWidgetInput::Next => {
                if self.page + 1 == PAGES {
                    if self.applied {
                        // Last page already applied → the primary button
                        // is now "Close". Reset for a clean re-open.
                        let _ = sender.output(WizardMenuWidgetOutput::CloseMenu);
                        self.page = 0;
                        self.applied = false;
                    } else {
                        // Apply (writes + live `mctl config reload`) and
                        // stay open so the reboot offer can show.
                        self.apply();
                        self.applied = true;
                    }
                } else {
                    self.page += 1;
                }
            }
            WizardMenuWidgetInput::Back => {
                self.applied = false;
                self.page = self.page.saturating_sub(1);
            }
            WizardMenuWidgetInput::Cancel => {
                let _ = sender.output(WizardMenuWidgetOutput::CloseMenu);
                self.page = 0;
                self.applied = false;
            }
            WizardMenuWidgetInput::Reboot => run_session_action(SessionAction::Reboot),
            WizardMenuWidgetInput::ModeChanged(m) => self.mode = m,
            WizardMenuWidgetInput::ThemeChanged(t) => self.theme_scheme = t,
            WizardMenuWidgetInput::FontScaleChanged(v) => self.font_scale = v,
            WizardMenuWidgetInput::Clock24hToggled(v) => self.clock_24h = v,
            WizardMenuWidgetInput::XkbLayoutChanged(s) => self.xkb_layout = s,
            WizardMenuWidgetInput::XkbVariantChanged(s) => self.xkb_variant = s.trim().to_string(),
            WizardMenuWidgetInput::XkbOptionsChanged(s) => self.xkb_options = s,
            WizardMenuWidgetInput::TapToClickToggled(v) => self.tap_to_click = v,
            WizardMenuWidgetInput::NaturalScrollToggled(v) => self.natural_scroll = v,
            WizardMenuWidgetInput::DisableWhileTypingToggled(v) => self.disable_while_typing = v,
            WizardMenuWidgetInput::ScanWifi => {
                self.wifi_status = "Scanning…".to_string();
                spawn_wifi_scan(sender.input_sender().clone());
            }
            WizardMenuWidgetInput::WifiListLoaded(list) => {
                self.wifi_selected = self.wifi_selected.min(list.len().saturating_sub(1));
                if self.wifi_status == "Scanning…" {
                    self.wifi_status = if list.is_empty() {
                        "No networks found.".to_string()
                    } else {
                        String::new()
                    };
                }
                self.wifi_networks = list;
            }
            WizardMenuWidgetInput::WifiSelected(idx) => self.wifi_selected = idx,
            WizardMenuWidgetInput::WifiPasswordChanged(s) => self.wifi_password = s,
            WizardMenuWidgetInput::ConnectWifi => {
                if let Some(ssid) = self.wifi_networks.get(self.wifi_selected).cloned() {
                    self.wifi_status = format!("Connecting to {ssid}…");
                    spawn_wifi_connect(
                        sender.input_sender().clone(),
                        ssid,
                        self.wifi_password.clone(),
                    );
                }
            }
            WizardMenuWidgetInput::WifiStatus(s) => self.wifi_status = s,
            WizardMenuWidgetInput::BarPositionChanged(bottom) => self.bar_at_bottom = bottom,
            WizardMenuWidgetInput::WallpaperPicked(p) => self.wallpaper_dir = p,
            WizardMenuWidgetInput::BrowseWallpaper => {
                let s = sender.clone();
                let dialog = gtk::FileDialog::builder()
                    .title("Choose Wallpaper Directory")
                    .modal(true)
                    .build();
                dialog.select_folder(gtk::Window::NONE, gio::Cancellable::NONE, move |res| {
                    if let Ok(file) = res
                        && let Some(path) = file.path()
                    {
                        s.input(WizardMenuWidgetInput::WallpaperPicked(
                            path.to_string_lossy().to_string(),
                        ));
                    }
                });
            }
        }
    }
}

impl WizardMenuWidgetModel {
    fn apply(&self) {
        let mode = self.mode;
        let theme = self.theme_scheme;
        let scale = self.font_scale;
        let clock = self.clock_24h;
        let dir = self.wallpaper_dir.clone();
        let want_bottom = self.bar_at_bottom;
        config_manager().update_config(move |c| {
            c.theme.matugen.mode = mode;
            c.theme.theme = theme;
            c.theme.attributes.sizing.font_scale = scale;
            c.general.clock_format_24_h = clock;
            c.wallpaper.wallpaper_dir = dir;

            // Bar position: relocate the populated bar to the chosen edge,
            // preserving the user's widget arrangement; reveal whichever
            // bar ends up populated and hide the empty one.
            let top_empty = c.bars.top_bar.left_widgets.is_empty()
                && c.bars.top_bar.center_widgets.is_empty()
                && c.bars.top_bar.right_widgets.is_empty();
            let currently_bottom = top_empty;
            if want_bottom != currently_bottom {
                std::mem::swap(&mut c.bars.top_bar, &mut c.bars.bottom_bar);
            }
            let top_now_empty = c.bars.top_bar.left_widgets.is_empty()
                && c.bars.top_bar.center_widgets.is_empty()
                && c.bars.top_bar.right_widgets.is_empty();
            c.bars.top_bar.reveal_by_default = !top_now_empty;
            c.bars.bottom_bar.reveal_by_default = top_now_empty;
        });

        let bit = |on: bool| if on { "1" } else { "0" }.to_string();
        let opt = |s: &str| (!s.is_empty()).then(|| s.to_string());
        let updates = [
            ("xkb_rules_layout", Some(self.xkb_layout.clone())),
            ("xkb_rules_variant", opt(&self.xkb_variant)),
            ("xkb_rules_options", opt(&self.xkb_options)),
            ("tap_to_click", Some(bit(self.tap_to_click))),
            ("trackpad_natural_scrolling", Some(bit(self.natural_scroll))),
            ("mouse_natural_scrolling", Some(bit(self.natural_scroll))),
            ("disable_while_typing", Some(bit(self.disable_while_typing))),
        ];
        match patch_margo_conf(&updates) {
            // Re-apply the compositor config live so the new keymap + input
            // settings take effect immediately — margo's `reload_config`
            // calls `set_xkb_config`, so no logout/reboot is needed.
            Ok(()) => reload_margo_config(),
            Err(e) => tracing::warn!(error = %e, "wizard: failed to write compositor config"),
        }
    }

    /// Multi-line summary shown on the Review step (and the "applied"
    /// confirmation once finished).
    fn review_text(&self) -> String {
        if self.applied {
            return "Applied live — your keyboard layout and input settings are \
                    already active. Reboot only if you want a fully clean start."
                .to_string();
        }
        let on = |b: bool| if b { "on" } else { "off" };
        let theme = THEMES
            .iter()
            .find(|(t, _)| *t == self.theme_scheme)
            .map(|(_, n)| *n)
            .unwrap_or("—");
        let font = FONT_SCALES
            .iter()
            .find(|(v, _)| (*v - self.font_scale).abs() < 0.001)
            .map(|(_, n)| *n)
            .unwrap_or("—");
        let layout = LAYOUTS
            .iter()
            .find(|(c, _)| *c == self.xkb_layout)
            .map(|(_, n)| *n)
            .unwrap_or(self.xkb_layout.as_str());
        let options = XKB_OPTIONS
            .iter()
            .find(|(c, _)| *c == self.xkb_options)
            .map(|(_, n)| *n)
            .unwrap_or("None");
        let mode = match self.mode {
            MatugenMode::Dark => "Dark",
            MatugenMode::Light => "Light",
        };
        let clock = if self.clock_24h { "24h" } else { "12h" };
        let variant = if self.xkb_variant.is_empty() {
            "—"
        } else {
            self.xkb_variant.as_str()
        };
        let tap = on(self.tap_to_click);
        let nat = on(self.natural_scroll);
        let dwt = on(self.disable_while_typing);
        let wall = &self.wallpaper_dir;
        let bar = if self.bar_at_bottom { "Bottom" } else { "Top" };
        let wifi = if !self.wifi_status.is_empty() {
            self.wifi_status.clone()
        } else {
            "not configured here".to_string()
        };
        format!(
            "Theme: {theme} · {mode} · {font} · {clock}-clock\n\
             Keyboard: {layout} / {variant} / {options}\n\
             Touchpad: tap {tap} · natural-scroll {nat} · dwt {dwt}\n\
             Wi-Fi: {wifi}\n\
             Wallpaper: {wall}\n\
             Bar: {bar}"
        )
    }
}

/// Parse SSIDs from NetworkManager's current scan (terse, one per line,
/// de-duplicated, blanks dropped).
async fn wifi_list() -> Vec<String> {
    let Ok(out) = tokio::process::Command::new("nmcli")
        .args(["-t", "-f", "SSID", "device", "wifi", "list"])
        .output()
        .await
    else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut seen: Vec<String> = Vec::new();
    for line in text.lines() {
        let ssid = line.trim();
        if ssid.is_empty() || seen.iter().any(|s| s == ssid) {
            continue;
        }
        seen.push(ssid.to_string());
    }
    seen
}

/// Read the cached scan (no rescan) and feed it back to the component.
fn spawn_wifi_list(sender: relm4::Sender<WizardMenuWidgetInput>) {
    tokio::spawn(async move {
        let _ = sender.send(WizardMenuWidgetInput::WifiListLoaded(wifi_list().await));
    });
}

/// Force a rescan, wait briefly for results, then reload the list.
fn spawn_wifi_scan(sender: relm4::Sender<WizardMenuWidgetInput>) {
    tokio::spawn(async move {
        let _ = tokio::process::Command::new("nmcli")
            .args(["device", "wifi", "rescan"])
            .status()
            .await;
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let _ = sender.send(WizardMenuWidgetInput::WifiListLoaded(wifi_list().await));
    });
}

/// `nmcli device wifi connect <ssid> [password <pw>]`, reporting the
/// outcome back as a status line.
fn spawn_wifi_connect(
    sender: relm4::Sender<WizardMenuWidgetInput>,
    ssid: String,
    password: String,
) {
    tokio::spawn(async move {
        let mut args = vec![
            "device".to_string(),
            "wifi".to_string(),
            "connect".to_string(),
            ssid.clone(),
        ];
        if !password.is_empty() {
            args.push("password".to_string());
            args.push(password);
        }
        let ok = matches!(
            tokio::process::Command::new("nmcli").args(&args).status().await,
            Ok(s) if s.success()
        );
        let msg = if ok {
            format!("✓ Connected to {ssid}")
        } else {
            format!("✗ Couldn't connect to {ssid} — check the password")
        };
        let _ = sender.send(WizardMenuWidgetInput::WifiStatus(msg));
    });
}

/// Fire `mctl config reload` (detached). margo applies the new
/// `xkb_rules_*` to the live keyboard, so the layout/options change
/// without restarting the session.
fn reload_margo_config() {
    match std::process::Command::new("mctl")
        .args(["config", "reload"])
        .spawn()
    {
        Ok(_) => {}
        Err(e) => tracing::warn!(error = %e, "wizard: `mctl config reload` failed to spawn"),
    }
}

fn read_live() -> WizardMenuWidgetModel {
    // Each `config()` is a cheap ArcStore clone; the field accessors
    // consume `self`, so read each from a fresh handle.
    WizardMenuWidgetModel {
        page: 0,
        mode: config_manager().config().theme().matugen().mode().get_untracked(),
        theme_scheme: config_manager().config().theme().theme().get_untracked(),
        font_scale: config_manager()
            .config()
            .theme()
            .attributes()
            .sizing()
            .font_scale()
            .get_untracked(),
        clock_24h: config_manager()
            .config()
            .general()
            .clock_format_24_h()
            .get_untracked(),
        xkb_layout: detect_default_xkb_layout(),
        xkb_variant: String::new(),
        xkb_options: String::new(),
        // Touchpad/mouse live in the compositor config, not the shell
        // store — read them back so a re-run reflects reality. Defaults
        // match config.example.conf.
        tap_to_click: read_margo_conf_bool("tap_to_click", true),
        natural_scroll: read_margo_conf_bool("trackpad_natural_scrolling", false),
        disable_while_typing: read_margo_conf_bool("disable_while_typing", true),
        wifi_networks: Vec::new(),
        wifi_selected: 0,
        wifi_password: String::new(),
        wifi_status: String::new(),
        bar_at_bottom: {
            // Bottom is "primary" only if the top bar is empty. The field
            // accessors consume `self`, so take a fresh handle per slot.
            let top = || config_manager().config().bars().top_bar();
            top().left_widgets().get_untracked().is_empty()
                && top().center_widgets().get_untracked().is_empty()
                && top().right_widgets().get_untracked().is_empty()
        },
        wallpaper_dir: {
            // First launch leaves this empty in the schema default; fall
            // back to a real directory so rotation has something to show
            // even if the user skips the Browse step.
            let cfg = config_manager()
                .config()
                .wallpaper()
                .wallpaper_dir()
                .get_untracked();
            if cfg.trim().is_empty() {
                default_wallpaper_dir()
            } else {
                cfg
            }
        },
        applied: false,
    }
}

/// Sensible wallpaper-source fallback for first launch, when no profile
/// has set one yet. First existing of the usual spots; `~/Pictures` as a
/// last resort so the field is never blank.
fn default_wallpaper_dir() -> String {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(h) = &home {
        candidates.push(h.join("Pictures/Wallpapers"));
        candidates.push(h.join("Pictures/wallpapers"));
        candidates.push(h.join("Pictures"));
    }
    candidates.push(PathBuf::from("/usr/share/backgrounds"));
    for c in &candidates {
        if c.is_dir() {
            return c.to_string_lossy().into_owned();
        }
    }
    home.map(|h| h.join("Pictures").to_string_lossy().into_owned())
        .unwrap_or_else(|| "/usr/share/backgrounds".to_string())
}

fn detect_default_xkb_layout() -> String {
    let lang = std::env::var("LC_ALL")
        .or_else(|_| std::env::var("LANG"))
        .unwrap_or_default()
        .to_lowercase();
    let country = lang
        .split('_')
        .nth(1)
        .and_then(|s| s.split('.').next())
        .unwrap_or("");
    if LAYOUTS.iter().any(|(c, _)| *c == country) {
        country.to_string()
    } else {
        "us".to_string()
    }
}

fn margo_conf_path() -> PathBuf {
    std::env::var_os("HOME")
        .map(|h| PathBuf::from(h).join(".config/margo/config.conf"))
        .unwrap_or_else(|| PathBuf::from("/tmp/margo-config.conf"))
}

/// Patch `key = value` lines in the compositor config in place, keeping
/// everything else. A `None` value drops the key's line; keys not already
/// present are appended. Used for the wizard's xkb + touchpad settings.
fn patch_margo_conf(updates: &[(&str, Option<String>)]) -> std::io::Result<()> {
    let path = margo_conf_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let mut out = String::with_capacity(existing.len() + 128);
    let mut seen = vec![false; updates.len()];
    for line in existing.lines() {
        let t = line.trim_start();
        let mut handled = false;
        for (i, (key, val)) in updates.iter().enumerate() {
            // `strip_prefix` + a `=` after optional whitespace guards
            // against prefix collisions (`xkb_rules_layout` won't eat a
            // hypothetical `xkb_rules_layout_x` line).
            if let Some(rest) = t.strip_prefix(key)
                && rest.trim_start().starts_with('=')
            {
                seen[i] = true;
                if let Some(v) = val {
                    out.push_str(&format!("{key} = {v}\n"));
                }
                handled = true;
                break;
            }
        }
        if !handled {
            out.push_str(line);
            out.push('\n');
        }
    }
    for (i, (key, val)) in updates.iter().enumerate() {
        if !seen[i]
            && let Some(v) = val
        {
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(&format!("{key} = {v}\n"));
        }
    }
    std::fs::write(&path, out)
}

/// Read a `key = value` bool out of the compositor config (last wins),
/// matching margo's parser truthiness. Falls back to `default` when the
/// file or key is missing.
fn read_margo_conf_bool(key: &str, default: bool) -> bool {
    let Ok(text) = std::fs::read_to_string(margo_conf_path()) else {
        return default;
    };
    let mut val = default;
    for line in text.lines() {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix(key)
            && let Some(after_eq) = rest.trim_start().strip_prefix('=')
        {
            let token = after_eq.split_whitespace().next().unwrap_or("");
            val = matches!(token, "1" | "true" | "yes" | "on");
        }
    }
    val
}
