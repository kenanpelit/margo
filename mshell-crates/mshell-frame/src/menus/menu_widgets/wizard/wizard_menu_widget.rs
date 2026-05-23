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
    ConfigStoreFields, GeneralStoreFields, MatugenStoreFields, SizingStoreFields,
    ThemeAttributesStoreFields, ThemeStoreFields, WallpaperStoreFields,
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

const PAGES: usize = 5;

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
    wallpaper_dir: String,
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
    BrowseWallpaper,
    WallpaperPicked(String),
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

                // ── 3 Wallpaper ───────────────────────────────
                add_named[Some("3")] = &gtk::Box {
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

                // ── 4 Done ────────────────────────────────────
                add_named[Some("4")] = &gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 8,
                    gtk::Label {
                        add_css_class: "label-large-bold",
                        #[watch]
                        set_label: if model.applied { "All set" } else { "Ready" },
                        set_halign: gtk::Align::Start,
                    },
                    gtk::Label {
                        add_css_class: "label-small",
                        #[watch]
                        set_label: if model.applied {
                            "Applied live — your keyboard layout is already active. Reboot only if you want a fully clean start."
                        } else {
                            "Finish applies your choices to the active profile and the compositor. The keyboard layout takes effect immediately."
                        },
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
        let _ = &sender;
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
        let (mode, theme, scale, clock, dir) = (
            self.mode,
            self.theme_scheme,
            self.font_scale,
            self.clock_24h,
            self.wallpaper_dir.clone(),
        );
        config_manager().update_config(move |c| {
            c.theme.matugen.mode = mode;
            c.theme.theme = theme;
            c.theme.attributes.sizing.font_scale = scale;
            c.general.clock_format_24_h = clock;
            c.wallpaper.wallpaper_dir = dir;
        });
        match write_xkb_to_margo_conf(&self.xkb_layout, &self.xkb_variant, &self.xkb_options) {
            // Re-apply the compositor config live so the new keymap takes
            // effect immediately — margo's `reload_config` calls
            // `set_xkb_config`, so no logout/reboot is needed.
            Ok(()) => reload_margo_config(),
            Err(e) => tracing::warn!(error = %e, "wizard: failed to write xkb to margo config"),
        }
    }
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

/// Patch `xkb_rules_layout` / `xkb_rules_variant` / `xkb_rules_options` in
/// the compositor config in place, preserving everything else. Empty
/// variant / options drop their respective lines. (Ported from the old
/// standalone wizard, extended for options.)
fn write_xkb_to_margo_conf(layout: &str, variant: &str, options: &str) -> std::io::Result<()> {
    let path = margo_conf_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let mut out = String::with_capacity(existing.len() + 96);
    let mut saw_layout = false;
    let mut saw_variant = false;
    let mut saw_options = false;
    for line in existing.lines() {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix("xkb_rules_layout")
            && rest.trim_start().starts_with('=')
        {
            out.push_str(&format!("xkb_rules_layout = {layout}\n"));
            saw_layout = true;
            continue;
        }
        if let Some(rest) = t.strip_prefix("xkb_rules_variant")
            && rest.trim_start().starts_with('=')
        {
            saw_variant = true;
            if !variant.is_empty() {
                out.push_str(&format!("xkb_rules_variant = {variant}\n"));
            }
            continue;
        }
        if let Some(rest) = t.strip_prefix("xkb_rules_options")
            && rest.trim_start().starts_with('=')
        {
            saw_options = true;
            if !options.is_empty() {
                out.push_str(&format!("xkb_rules_options = {options}\n"));
            }
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    if !saw_layout {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&format!("xkb_rules_layout = {layout}\n"));
    }
    if !saw_variant && !variant.is_empty() {
        out.push_str(&format!("xkb_rules_variant = {variant}\n"));
    }
    if !saw_options && !options.is_empty() {
        out.push_str(&format!("xkb_rules_options = {options}\n"));
    }
    std::fs::write(&path, out)
}
