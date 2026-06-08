//! In-shell setup wizard — a layer-shell MENU, never a floating window.
//!
//! A **hardware-aware** first-run flow inside a `gtk::Stack`: the visited
//! steps are computed at open time from a [`hw_info::HwInfo`] probe (no
//! Touchpad step on a desktop, no Wi-Fi without a card, no Power without a
//! battery, no Display on a single monitor — see [`steps::build_steps`]).
//! Steps: Welcome → Theme → Keyboard → [Touchpad] → [Display] → [Power] →
//! Night light → [Wi-Fi] → Wallpaper → Bar → Review. The Review step lets
//! the user jump back to edit any step; Enter advances, Escape cancels.
//!
//! Apply writes the choices LIVE through `config_manager` (theme / font /
//! clock / wallpaper / bar) plus the xkb + touchpad + twilight lines in
//! the compositor's `config.conf`, runs `mpower`, then drops a
//! `~/.config/margo/.wizard-done` sentinel so first-launch auto-open stops
//! nagging. Reachable from the Settings → Setup button, `mshellctl
//! wizard`, and (once, gated by the sentinel) the first-launch auto-open.

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
    BoxExt, ButtonExt, EditableExt, EntryExt, FileExt, ListModelExt, OrientableExt, WidgetExt,
};
use relm4::gtk::{gio, glib};
use relm4::{ComponentParts, ComponentSender, SimpleComponent, gtk};
use std::path::PathBuf;

use super::hw_info::HwInfo;
use super::steps;

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
    /// Ordered applicable steps for THIS machine (hardware-aware).
    steps: Vec<steps::StepKind>,
    /// Index into `steps` — the current step.
    pos: usize,
    /// Chosen starter profile ("default" or "margo"); seeded + activated live
    /// when picked on the Welcome step.
    base_profile: String,
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
    /// Display model for the SSID dropdown. Held + mutated in place so
    /// the dropdown's model is never rebuilt on every view pass — a
    /// `#[watch] set_model` there fed `selected-notify` back into the
    /// update cycle and spun the main loop at 100% CPU.
    wifi_model: gtk::StringList,
    wifi_selected: usize,
    wifi_password: String,
    /// Free-text status line under the Connect button.
    wifi_status: String,
    wallpaper_dir: String,
    /// Power: chosen mpower mode ("auto" | "balanced" | "power-saver").
    power_mode: &'static str,
    /// Twilight (blue-light) enabled.
    twilight_on: bool,
    /// Display: free-text status under the Auto-arrange button.
    display_status: String,
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
    BaseProfileSelected(&'static str),
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
    PowerModeChanged(&'static str),
    TwilightToggled(bool),
    AutoArrangeDisplays,
    DisplayStatus(String),
    EditStep(steps::StepKind),
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
            // Fills the 640 px wizard menu minus the container's 20 px
            // side margins. `hexpand` already stretches it; this floor
            // keeps the form controls at the full width if a page's
            // natural width ever comes in narrower.
            set_width_request: 600,

            gtk::Label {
                add_css_class: "label-small",
                set_halign: gtk::Align::Start,
                #[watch]
                set_label: &format!("Step {} of {}", model.pos + 1, model.steps.len()),
            },

            #[name = "stack"]
            gtk::Stack {
                set_vexpand: true,
                set_transition_type: gtk::StackTransitionType::SlideLeftRight,
                set_transition_duration: 180,
                #[watch]
                set_visible_child_name: model.cur().child_name(),

                // ── 0 Welcome ─────────────────────────────────
                add_named[Some("0")] = &gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 8,
                    set_valign: gtk::Align::Center,
                    // margo logo — resolved from the icon theme
                    // (/usr/share/icons/hicolor/scalable/apps/margo.svg),
                    // so it tracks whatever theme is installed.
                    gtk::Image {
                        add_css_class: "wizard-logo",
                        set_icon_name: Some("margo"),
                        set_pixel_size: 72,
                        set_halign: gtk::Align::Start,
                        set_margin_bottom: 4,
                    },
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

                    gtk::Label {
                        add_css_class: "label-medium-bold",
                        set_label: "Starting profile",
                        set_halign: gtk::Align::Start,
                        set_margin_top: 10,
                    },
                    gtk::Label {
                        add_css_class: "label-small",
                        set_label: "Pick a base to build on — applied right away. Re-running the wizard never overwrites a profile you've customised.",
                        set_halign: gtk::Align::Start,
                        set_xalign: 0.0,
                        set_wrap: true,
                    },

                    gtk::Button {
                        #[watch]
                        set_css_classes: if model.base_profile == "margo" {
                            &["ok-button-surface", "selected"]
                        } else {
                            &["ok-button-surface"]
                        },
                        connect_clicked[sender] => move |_| {
                            sender.input(WizardMenuWidgetInput::BaseProfileSelected("margo"));
                        },
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_spacing: 2,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_label: "margo — the full experience",
                                set_halign: gtk::Align::Start,
                                set_xalign: 0.0,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_label: "Rich top bar, every menu wired up, Material-You theming, dock + twilight. Recommended.",
                                set_halign: gtk::Align::Start,
                                set_xalign: 0.0,
                                set_wrap: true,
                            },
                        },
                    },

                    gtk::Button {
                        #[watch]
                        set_css_classes: if model.base_profile == "default" {
                            &["ok-button-surface", "selected"]
                        } else {
                            &["ok-button-surface"]
                        },
                        connect_clicked[sender] => move |_| {
                            sender.input(WizardMenuWidgetInput::BaseProfileSelected("default"));
                        },
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_spacing: 2,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_label: "Default — clean & minimal",
                                set_halign: gtk::Align::Start,
                                set_xalign: 0.0,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_label: "A single top bar with the essentials and built-in defaults — a bare canvas to build on.",
                                set_halign: gtk::Align::Start,
                                set_xalign: 0.0,
                                set_wrap: true,
                            },
                        },
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
                            // Model set ONCE; contents are spliced in
                            // place on WifiListLoaded. Never `#[watch]` it.
                            set_model: Some(&model.wifi_model),
                            #[watch]
                            set_sensitive: !model.wifi_networks.is_empty(),
                            #[watch]
                            set_selected: model.wifi_selected as u32,
                            connect_selected_notify[sender] => move |dd| {
                                sender.input(WizardMenuWidgetInput::WifiSelected(dd.selected() as usize));
                            },
                        },
                        gtk::Button {
                            set_css_classes: &["ok-button-surface", "wizard-button"],
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
                        set_css_classes: &["ok-button-primary", "wizard-button"],
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
                        set_css_classes: &["ok-button-surface", "wizard-button"],
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

                // ── 7 Display ─────────────────────────────────
                add_named[Some("7")] = &gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 12,
                    gtk::Label { add_css_class: "label-large-bold", set_label: "Display", set_halign: gtk::Align::Start },
                    gtk::Label {
                        add_css_class: "label-small",
                        set_label: "Multiple monitors detected. Auto-arrange picks a sensible layout now; fine-tune later in Settings → Display.",
                        set_halign: gtk::Align::Start, set_xalign: 0.0, set_wrap: true,
                    },
                    gtk::Button {
                        set_css_classes: &["ok-button-surface", "wizard-button"],
                        set_label: "Auto-arrange monitors",
                        set_halign: gtk::Align::Start,
                        connect_clicked[sender] => move |_| sender.input(WizardMenuWidgetInput::AutoArrangeDisplays),
                    },
                    gtk::Label {
                        add_css_class: "label-small",
                        #[watch]
                        set_label: &model.display_status,
                        #[watch]
                        set_visible: !model.display_status.is_empty(),
                        set_halign: gtk::Align::Start, set_xalign: 0.0, set_wrap: true,
                    },
                },

                // ── 8 Power ───────────────────────────────────
                add_named[Some("8")] = &gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 12,
                    gtk::Label { add_css_class: "label-large-bold", set_label: "Power", set_halign: gtk::Align::Start },
                    gtk::Label {
                        add_css_class: "label-small",
                        set_label: "Battery detected. Pick how margo manages the power profile.",
                        set_halign: gtk::Align::Start, set_xalign: 0.0, set_wrap: true,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal, set_spacing: 16,
                        gtk::Label { add_css_class: "label-medium", set_label: "Profile", set_halign: gtk::Align::Start, set_hexpand: true },
                        gtk::DropDown {
                            set_valign: gtk::Align::Center,
                            set_model: Some(&gtk::StringList::new(&["Automatic", "Balanced", "Power saver"])),
                            #[watch]
                            set_selected: match model.power_mode { "balanced" => 1, "power-saver" => 2, _ => 0 },
                            connect_selected_notify[sender] => move |dd| {
                                let m = match dd.selected() { 1 => "balanced", 2 => "power-saver", _ => "auto" };
                                sender.input(WizardMenuWidgetInput::PowerModeChanged(m));
                            },
                        },
                    },
                },

                // ── 9 Twilight (night light) ──────────────────
                add_named[Some("9")] = &gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 12,
                    gtk::Label { add_css_class: "label-large-bold", set_label: "Night light", set_halign: gtk::Align::Start },
                    gtk::Label {
                        add_css_class: "label-small",
                        set_label: "Warm the screen on a sunrise/sunset schedule. Tune temperature + location later in Settings → Display.",
                        set_halign: gtk::Align::Start, set_xalign: 0.0, set_wrap: true,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal, set_spacing: 16,
                        gtk::Label { add_css_class: "label-medium", set_label: "Enable night light", set_halign: gtk::Align::Start, set_hexpand: true },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            set_active: model.twilight_on,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(WizardMenuWidgetInput::TwilightToggled(v));
                                glib::Propagation::Proceed
                            },
                        },
                    },
                },

                // ── 10 Review ─────────────────────────────────
                add_named[Some("10")] = &gtk::Box {
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
                    #[name = "review_edit_row"]
                    gtk::FlowBox {
                        set_selection_mode: gtk::SelectionMode::None,
                        set_max_children_per_line: 4,
                        set_column_spacing: 6,
                        set_row_spacing: 6,
                        set_margin_top: 6,
                        #[watch]
                        set_visible: !model.applied,
                    },

                    // Handy default shortcuts — what the new user can open
                    // right away. These mirror the shipped binds.conf
                    // defaults (mshellctl menu …).
                    gtk::Label {
                        add_css_class: "label-medium-bold",
                        set_label: "Handy shortcuts",
                        set_halign: gtk::Align::Start,
                        set_margin_top: 10,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 8,
                        gtk::Label {
                            set_css_classes: &["keybind-chip", "keybind-mod", "mod-super"],
                            set_label: "Super + Space",
                        },
                        gtk::Label {
                            add_css_class: "keybinds-desc",
                            set_label: "Open the app launcher",
                            set_halign: gtk::Align::Start, set_xalign: 0.0, set_hexpand: true,
                        },
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 8,
                        gtk::Label {
                            set_css_classes: &["keybind-chip", "keybind-mod", "mod-super"],
                            set_label: "Super + F1",
                        },
                        gtk::Label {
                            add_css_class: "keybinds-desc",
                            set_label: "Show all keyboard shortcuts",
                            set_halign: gtk::Align::Start, set_xalign: 0.0, set_hexpand: true,
                        },
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 8,
                        gtk::Label {
                            set_css_classes: &["keybind-chip", "keybind-mod", "mod-super"],
                            set_label: "Super + D",
                        },
                        gtk::Label {
                            add_css_class: "keybinds-desc",
                            set_label: "Open the dashboard (calendar, media, weather…)",
                            set_halign: gtk::Align::Start, set_xalign: 0.0, set_hexpand: true,
                        },
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 8,
                        gtk::Label {
                            set_css_classes: &["keybind-chip", "keybind-mod", "mod-super"],
                            set_label: "Super + Shift + D",
                        },
                        gtk::Label {
                            add_css_class: "keybinds-desc",
                            set_label: "Open the control centre (quick settings)",
                            set_halign: gtk::Align::Start, set_xalign: 0.0, set_hexpand: true,
                        },
                    },
                },
            },

            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 8,
                set_halign: gtk::Align::Center,
                gtk::Button {
                    set_css_classes: &["ok-button-surface", "wizard-button"],
                    set_label: "Cancel",
                    #[watch]
                    set_visible: !model.applied,
                    connect_clicked[sender] => move |_| sender.input(WizardMenuWidgetInput::Cancel),
                },
                gtk::Button {
                    set_css_classes: &["ok-button-surface", "wizard-button"],
                    set_label: "Back",
                    #[watch]
                    set_visible: !model.applied,
                    #[watch]
                    set_sensitive: model.pos > 0,
                    connect_clicked[sender] => move |_| sender.input(WizardMenuWidgetInput::Back),
                },
                gtk::Button {
                    set_css_classes: &["ok-button-surface", "wizard-button"],
                    set_label: "Reboot now",
                    #[watch]
                    set_visible: model.applied,
                    connect_clicked[sender] => move |_| sender.input(WizardMenuWidgetInput::Reboot),
                },
                gtk::Button {
                    set_css_classes: &["ok-button-primary", "wizard-button"],
                    #[watch]
                    set_label: if model.applied {
                        "Close"
                    } else if model.is_last() {
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

        // Review-step "edit" chips: one per applicable step (skip Welcome
        // + Review themselves). relm4's `view!` can't loop, so build them
        // here from the model's hardware-aware step list.
        for kind in &model.steps {
            use steps::StepKind::{Review, Welcome};
            if matches!(kind, Welcome | Review) {
                continue;
            }
            let k = *kind;
            let btn = gtk::Button::builder()
                .label(format!("✎ {}", k.label()))
                .css_classes(["ok-button-surface"])
                .build();
            let s = sender.clone();
            btn.connect_clicked(move |_| s.input(WizardMenuWidgetInput::EditStep(k)));
            widgets.review_edit_row.append(&btn);
        }

        // Keyboard navigation: Enter advances, Escape cancels.
        let key = gtk::EventControllerKey::new();
        let ks = sender.clone();
        key.connect_key_pressed(move |_, keyval, _, _| match keyval {
            gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter => {
                ks.input(WizardMenuWidgetInput::Next);
                glib::Propagation::Stop
            }
            gtk::gdk::Key::Escape => {
                ks.input(WizardMenuWidgetInput::Cancel);
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        });
        root.add_controller(key);

        // Warm the Wi-Fi list from NetworkManager's last scan (no rescan)
        // so the dropdown is populated by the time the user reaches the
        // Network step. The Scan button forces a fresh rescan.
        spawn_wifi_list(sender.input_sender().clone());
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>) {
        match message {
            WizardMenuWidgetInput::Next => {
                if self.is_last() {
                    if self.applied {
                        // Last page already applied → the primary button
                        // is now "Close". Reset for a clean re-open.
                        let _ = sender.output(WizardMenuWidgetOutput::CloseMenu);
                        self.pos = 0;
                        self.applied = false;
                    } else {
                        // Apply (writes + live `mctl config reload`) and
                        // stay open so the reboot offer can show.
                        self.apply();
                        self.applied = true;
                    }
                } else {
                    self.pos += 1;
                }
            }
            WizardMenuWidgetInput::Back => {
                self.applied = false;
                self.pos = self.pos.saturating_sub(1);
            }
            WizardMenuWidgetInput::Cancel => {
                let _ = sender.output(WizardMenuWidgetOutput::CloseMenu);
                self.pos = 0;
                self.applied = false;
            }
            WizardMenuWidgetInput::Reboot => run_session_action(SessionAction::Reboot),
            WizardMenuWidgetInput::BaseProfileSelected(name) => {
                self.base_profile = name.to_string();
                // Seed the bundled starter profile if the user has none of
                // that name yet (never clobbers a customised one), then make
                // it active live so the choice is visible immediately.
                mshell_config::config_utils::seed_bundled_profile(name);
                config_manager().set_active_profile(Some(name.to_string()));
            }
            // Appearance picks apply LIVE (like Settings → Theme), not only at
            // "Apply & finish" — otherwise selecting a theme / mode / font in
            // the wizard showed no change until the very end, which read as
            // "nothing applies".
            WizardMenuWidgetInput::ModeChanged(m) => {
                self.mode = m;
                config_manager().update_config(move |c| c.theme.matugen.mode = m);
            }
            WizardMenuWidgetInput::ThemeChanged(t) => {
                self.theme_scheme = t;
                config_manager().update_config(move |c| c.theme.theme = t);
            }
            WizardMenuWidgetInput::FontScaleChanged(v) => {
                self.font_scale = v;
                config_manager().update_config(move |c| c.theme.attributes.sizing.font_scale = v);
            }
            WizardMenuWidgetInput::Clock24hToggled(v) => {
                self.clock_24h = v;
                config_manager().update_config(move |c| c.general.clock_format_24_h = v);
            }
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
                // Mutate the existing model in place (placeholder when
                // empty) — never swap in a fresh StringList here.
                let items: Vec<&str> = if list.is_empty() {
                    vec!["Scan for networks…"]
                } else {
                    list.iter().map(|s| s.as_str()).collect()
                };
                self.wifi_model.splice(0, self.wifi_model.n_items(), &items);
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
            WizardMenuWidgetInput::PowerModeChanged(m) => self.power_mode = m,
            WizardMenuWidgetInput::TwilightToggled(v) => self.twilight_on = v,
            WizardMenuWidgetInput::DisplayStatus(s) => self.display_status = s,
            WizardMenuWidgetInput::EditStep(kind) => self.goto(kind),
            WizardMenuWidgetInput::AutoArrangeDisplays => {
                self.display_status = "Arranging…".to_string();
                let s = sender.input_sender().clone();
                std::thread::spawn(move || {
                    let ok = matches!(
                        std::process::Command::new("mlayout").arg("suggest").status(),
                        Ok(st) if st.success()
                    );
                    let msg = if ok {
                        "✓ Applied a layout for the current monitors.".to_string()
                    } else {
                        "Couldn't auto-arrange — set it up later in Settings → Display.".to_string()
                    };
                    let _ = s.send(WizardMenuWidgetInput::DisplayStatus(msg));
                });
            }
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
    fn cur(&self) -> steps::StepKind {
        self.steps
            .get(self.pos)
            .copied()
            .unwrap_or(steps::StepKind::Review)
    }
    fn is_last(&self) -> bool {
        self.pos + 1 >= self.steps.len()
    }
    /// Jump to a step by kind (Review "edit" buttons). No-op if absent.
    fn goto(&mut self, kind: steps::StepKind) {
        if let Some(i) = self.steps.iter().position(|s| *s == kind) {
            self.pos = i;
            self.applied = false;
        }
    }

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

        // Power profile (mpower): "auto" clears any manual override; the
        // others set + hold it. Best-effort — the daemon may be off.
        let _ = match self.power_mode {
            "auto" => std::process::Command::new("mpower").arg("auto").status(),
            other => std::process::Command::new("mpower")
                .args(["set", other])
                .status(),
        };

        // Twilight (night light): write the on/off to the compositor config
        // (schedule/geo/manual stay as the user already had them).
        let tw = [(
            "twilight",
            Some(if self.twilight_on { "1" } else { "0" }.to_string()),
        )];
        if patch_margo_conf(&tw).is_ok() {
            reload_margo_config();
        }

        mshell_config::config_utils::mark_wizard_completed();
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
        let power = self.power_mode;
        let night = if self.twilight_on { "on" } else { "off" };
        let wifi = if !self.wifi_status.is_empty() {
            self.wifi_status.clone()
        } else {
            "not configured here".to_string()
        };
        format!(
            "Theme: {theme} · {mode} · {font} · {clock}-clock\n\
             Keyboard: {layout} / {variant} / {options}\n\
             Touchpad: tap {tap} · natural-scroll {nat} · dwt {dwt}\n\
             Power: {power} · Night light: {night}\n\
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
        steps: steps::build_steps(&HwInfo::probe()),
        pos: 0,
        // Pre-select whichever bundled profile is already active, else the
        // recommended "margo".
        base_profile: match config_manager().active_profile().get_untracked().as_deref() {
            Some("default") => "default".to_string(),
            _ => "margo".to_string(),
        },
        mode: config_manager()
            .config()
            .theme()
            .matugen()
            .mode()
            .get_untracked(),
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
        wifi_model: gtk::StringList::new(&["Scan for networks…"]),
        wifi_selected: 0,
        wifi_password: String::new(),
        wifi_status: String::new(),
        power_mode: "auto",
        twilight_on: read_margo_conf_bool("twilight", false),
        display_status: String::new(),
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
