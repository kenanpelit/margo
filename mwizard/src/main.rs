//! mwizard — first-launch setup wizard for margo + mshell.
//!
//! Opens a four-page GTK4 wizard on first run (when no
//! `~/.config/margo/mshell/profiles/default.yaml` exists),
//! collects a handful of user choices (theme mode, clock format,
//! wallpaper directory), writes them into a freshly-defaulted
//! `Config`, and saves the result. Re-running `mwizard` from the
//! command line re-opens the same flow even after first launch is
//! past — useful for resetting a busted profile.
//!
//! Pages:
//! 1. **Welcome** — intro to margo + what the wizard will ask.
//! 2. **Theme** — Light / Dark matugen mode + 12h / 24h clock.
//! 3. **Keyboard** — xkb layout picker (us, tr, de, …) +
//!    optional variant string for fine-tuning.
//! 4. **Wallpaper** — directory the wallpaper rotation pulls from.
//! 5. **Done** — summary + Apply that writes both files.
//!
//! Apply touches two files:
//! - `~/.config/margo/mshell/profiles/default.yaml` — shell profile,
//!   fully serialised from a default `Config` + user choices.
//! - `~/.config/margo/config.conf` — compositor config, surgical
//!   line-level edit so we don't have to ship a full margo-config
//!   serialiser. Only `xkb_rules_layout` (and `xkb_rules_variant`
//!   when non-empty) are written; everything else stays at
//!   built-in defaults until the user runs `mctl config edit`.

use anyhow::{Context, Result};
use clap::Parser;
use gtk4::prelude::*;
use mshell_config::paths::default_config_path;
use mshell_config::schema::config::Config;
use mshell_config::schema::themes::MatugenMode;
use relm4::{ComponentParts, ComponentSender, RelmApp, RelmWidgetExt, SimpleComponent, gtk};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "mwizard",
    version,
    about = "First-launch setup wizard for margo + mshell"
)]
struct Cli {
    /// Open the wizard even when a profile already exists. Useful
    /// for resetting / re-running the flow after first launch.
    #[arg(long)]
    force: bool,
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("MWIZARD_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    if !cli.force && default_config_path().exists() {
        // Default profile already exists — silently exit so that
        // hooking mwizard into a session-start script (e.g.
        // `exec-once = mwizard` in margo.conf) is a no-op on
        // every launch *after* the first. `--force` flips this
        // back on for explicit re-runs.
        tracing::info!(
            path = %default_config_path().display(),
            "profile already exists; nothing to do"
        );
        return;
    }

    let app = RelmApp::new("com.margo.mwizard");
    // Theme the wizard with the shell's own compiled stylesheet so it
    // looks like part of margo, not a stock GTK dialog. compiled_css()
    // ships a full default palette (DESIGN.md tokens + matugen-baseline
    // colours), so the DESIGN.md component classes render correctly even
    // on a true first run before matugen has produced a palette.
    relm4::set_global_css(mshell_style::compiled_css());
    app.run::<WizardModel>(WizardInit {});
}

/// Index of the visible page in `gtk::Stack`. Sequential — the
/// wizard doesn't allow free-jumping between pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Welcome,
    Theme,
    Keyboard,
    Wallpaper,
    Done,
}

impl Page {
    fn index(self) -> u32 {
        match self {
            Self::Welcome => 0,
            Self::Theme => 1,
            Self::Keyboard => 2,
            Self::Wallpaper => 3,
            Self::Done => 4,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Welcome => "welcome",
            Self::Theme => "theme",
            Self::Keyboard => "keyboard",
            Self::Wallpaper => "wallpaper",
            Self::Done => "done",
        }
    }

    fn next(self) -> Option<Self> {
        match self {
            Self::Welcome => Some(Self::Theme),
            Self::Theme => Some(Self::Keyboard),
            Self::Keyboard => Some(Self::Wallpaper),
            Self::Wallpaper => Some(Self::Done),
            Self::Done => None,
        }
    }

    fn prev(self) -> Option<Self> {
        match self {
            Self::Welcome => None,
            Self::Theme => Some(Self::Welcome),
            Self::Keyboard => Some(Self::Theme),
            Self::Wallpaper => Some(Self::Keyboard),
            Self::Done => Some(Self::Wallpaper),
        }
    }

    fn total() -> u32 {
        5
    }
}

/// Common xkb keyboard layouts surfaced in the wizard dropdown.
/// `(xkb code, display name)`. Order is roughly "most-likely
/// first" for an Anglophone install, but the user can flip the
/// custom-override entry on for any code xkbcommon understands.
const COMMON_LAYOUTS: &[(&str, &str)] = &[
    ("us", "English (US)"),
    ("gb", "English (UK)"),
    ("tr", "Türkçe"),
    ("de", "Deutsch"),
    ("fr", "Français"),
    ("es", "Español"),
    ("it", "Italiano"),
    ("pt", "Português"),
    ("ru", "Русский"),
    ("ua", "Українська"),
    ("ar", "العربية"),
    ("ja", "日本語"),
    ("cn", "中文"),
    ("kr", "한국어"),
];

/// Wizard-side mirror of every knob the pages can toggle. Folded
/// into a fresh `Config::default()` on Apply — keeping the user's
/// choices in a tiny intermediate struct lets the pages emit
/// updates without owning the entire `Config` reactive tree.
#[derive(Debug, Clone)]
struct Choices {
    matugen_mode: MatugenMode,
    clock_24h: bool,
    /// xkb layout code (e.g. `"us"`, `"tr"`, `"de"`). Written to
    /// `~/.config/margo/config.conf` as `xkb_rules_layout = …`.
    xkb_layout: String,
    /// Optional xkb variant (e.g. `"dvorak"`, `"f"`). Empty
    /// string skips writing the line entirely.
    xkb_variant: String,
    wallpaper_dir: PathBuf,
}

impl Choices {
    fn defaults() -> Self {
        Self {
            matugen_mode: MatugenMode::Dark,
            clock_24h: true,
            xkb_layout: detect_default_xkb_layout(),
            xkb_variant: String::new(),
            wallpaper_dir: dirs::picture_dir()
                .map(|p| p.join("wallpapers"))
                .unwrap_or_else(|| {
                    dirs::home_dir()
                        .map(|h| h.join("Pictures").join("wallpapers"))
                        .unwrap_or_else(|| PathBuf::from("/usr/share/backgrounds"))
                }),
        }
    }
}

/// Pick a reasonable default xkb layout from the environment.
/// `LANG` / `LC_ALL` end in the country code (e.g. `tr_TR.UTF-8`,
/// `en_US.UTF-8`) which maps 1:1 to most xkb layouts; if the
/// match looks weird the user can always override on the page.
fn detect_default_xkb_layout() -> String {
    let lang = std::env::var("LC_ALL")
        .or_else(|_| std::env::var("LANG"))
        .unwrap_or_default()
        .to_lowercase();
    // Pull the country fragment: "tr_tr.utf-8" → "tr".
    let country = lang
        .split('_')
        .nth(1)
        .and_then(|s| s.split('.').next())
        .unwrap_or("");
    // Recognise the codes we ship in the dropdown so we land on a
    // known entry; otherwise fall back to "us".
    if COMMON_LAYOUTS.iter().any(|(code, _)| *code == country) {
        country.to_string()
    } else {
        "us".to_string()
    }
}

struct WizardModel {
    page: Page,
    choices: Choices,
    apply_status: ApplyStatus,
}

#[derive(Debug, Clone)]
enum ApplyStatus {
    Pending,
    Ok(PathBuf),
    Err(String),
}

#[derive(Debug)]
enum WizardInput {
    Next,
    Back,
    Cancel,
    MatugenModeChanged(MatugenMode),
    Clock24hChanged(bool),
    XkbLayoutChanged(String),
    XkbVariantChanged(String),
    WallpaperDirPicked(PathBuf),
    OpenWallpaperPicker,
}

struct WizardInit {}

#[relm4::component]
impl SimpleComponent for WizardModel {
    type Input = WizardInput;
    type Output = ();
    type Init = WizardInit;

    view! {
        gtk::Window {
            set_title: Some("Margo Setup"),
            set_default_size: (640, 480),
            set_resizable: false,

            gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_margin_all: 24,
                set_spacing: 16,

                // Step indicator
                gtk::Label {
                    add_css_class: "label-small",
                    #[watch]
                    set_label: &format!(
                        "Step {} of {}",
                        model.page.index() + 1,
                        Page::total(),
                    ),
                    set_halign: gtk::Align::Start,
                },

                #[name = "stack"]
                gtk::Stack {
                    set_vexpand: true,
                    set_transition_type: gtk::StackTransitionType::SlideLeftRight,
                    set_transition_duration: 200,
                    #[watch]
                    set_visible_child_name: model.page.name(),

                    add_named[Some(Page::Welcome.name())] = &gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_spacing: 12,
                        set_valign: gtk::Align::Center,

                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "Welcome to margo",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            set_label: "This wizard will set up your shell profile with a handful of sensible defaults. You can change everything later in Settings or by editing the YAML profile under ~/.config/margo/mshell/profiles/.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_max_width_chars: 60,
                        },
                        gtk::Label {
                            set_label: "Click Next to begin.",
                            set_halign: gtk::Align::Start,
                            set_margin_top: 12,
                        },
                    },

                    add_named[Some(Page::Theme.name())] = &gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_spacing: 16,

                        gtk::Label {
                            add_css_class: "label-large-bold",
                            set_label: "Theme",
                            set_halign: gtk::Align::Start,
                        },

                        gtk::Box {
                            set_orientation: gtk::Orientation::Horizontal,
                            set_spacing: 16,
                            gtk::Label {
                                set_label: "Color mode:",
                                set_halign: gtk::Align::Start,
                                set_hexpand: true,
                            },
                            #[name = "mode_dropdown"]
                            gtk::DropDown {
                                set_model: Some(&gtk::StringList::new(&["Dark", "Light"])),
                                #[watch]
                                set_selected: match model.choices.matugen_mode {
                                    MatugenMode::Dark => 0,
                                    MatugenMode::Light => 1,
                                },
                                connect_selected_notify[sender] => move |dd| {
                                    let mode = if dd.selected() == 0 {
                                        MatugenMode::Dark
                                    } else {
                                        MatugenMode::Light
                                    };
                                    sender.input(WizardInput::MatugenModeChanged(mode));
                                },
                            },
                        },

                        gtk::Box {
                            set_orientation: gtk::Orientation::Horizontal,
                            set_spacing: 16,
                            gtk::Label {
                                set_label: "Clock format:",
                                set_halign: gtk::Align::Start,
                                set_hexpand: true,
                            },
                            #[name = "clock_dropdown"]
                            gtk::DropDown {
                                set_model: Some(&gtk::StringList::new(&["24-hour (14:30)", "12-hour (2:30 PM)"])),
                                #[watch]
                                set_selected: if model.choices.clock_24h { 0 } else { 1 },
                                connect_selected_notify[sender] => move |dd| {
                                    sender.input(WizardInput::Clock24hChanged(dd.selected() == 0));
                                },
                            },
                        },

                        gtk::Label {
                            add_css_class: "label-small",
                            set_label: "Both knobs are live — Settings → Theme will let you tweak the matugen palette, accent tints, and font sizes once mshell is running.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_max_width_chars: 60,
                            set_margin_top: 12,
                        },
                    },

                    add_named[Some(Page::Keyboard.name())] = &gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_spacing: 16,

                        gtk::Label {
                            add_css_class: "label-large-bold",
                            set_label: "Keyboard layout",
                            set_halign: gtk::Align::Start,
                        },

                        gtk::Label {
                            set_label: "Pick the xkb layout the compositor should load on startup. The list covers the most common entries; override the layout / variant text fields below for anything xkbcommon understands.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_max_width_chars: 60,
                        },

                        gtk::Box {
                            set_orientation: gtk::Orientation::Horizontal,
                            set_spacing: 16,
                            gtk::Label {
                                set_label: "Layout:",
                                set_halign: gtk::Align::Start,
                                set_hexpand: true,
                            },
                            #[name = "layout_dropdown"]
                            gtk::DropDown {
                                set_model: Some(&gtk::StringList::new(
                                    &COMMON_LAYOUTS
                                        .iter()
                                        .map(|(code, name)| format!("{name} ({code})"))
                                        .collect::<Vec<_>>()
                                        .iter()
                                        .map(|s| s.as_str())
                                        .collect::<Vec<_>>(),
                                )),
                                #[watch]
                                set_selected: COMMON_LAYOUTS
                                    .iter()
                                    .position(|(code, _)| *code == model.choices.xkb_layout)
                                    .unwrap_or(0) as u32,
                                connect_selected_notify[sender] => move |dd| {
                                    let idx = dd.selected() as usize;
                                    if let Some((code, _)) = COMMON_LAYOUTS.get(idx) {
                                        sender.input(WizardInput::XkbLayoutChanged((*code).to_string()));
                                    }
                                },
                            },
                        },

                        gtk::Box {
                            set_orientation: gtk::Orientation::Horizontal,
                            set_spacing: 16,
                            gtk::Label {
                                set_label: "Override layout code:",
                                set_halign: gtk::Align::Start,
                                set_hexpand: true,
                            },
                            #[name = "layout_override_entry"]
                            gtk::Entry {
                                set_placeholder_text: Some("e.g. us,tr or dvorak"),
                                set_width_chars: 18,
                                #[watch]
                                set_text: &model.choices.xkb_layout,
                                connect_changed[sender] => move |e| {
                                    sender.input(WizardInput::XkbLayoutChanged(e.text().to_string()));
                                },
                            },
                        },

                        gtk::Box {
                            set_orientation: gtk::Orientation::Horizontal,
                            set_spacing: 16,
                            gtk::Label {
                                set_label: "Variant (optional):",
                                set_halign: gtk::Align::Start,
                                set_hexpand: true,
                            },
                            #[name = "variant_entry"]
                            gtk::Entry {
                                set_placeholder_text: Some("e.g. f, dvorak, intl"),
                                set_width_chars: 18,
                                #[watch]
                                set_text: &model.choices.xkb_variant,
                                connect_changed[sender] => move |e| {
                                    sender.input(WizardInput::XkbVariantChanged(e.text().to_string()));
                                },
                            },
                        },

                        gtk::Label {
                            add_css_class: "label-small",
                            set_label: "Pre-filled from your $LANG when it maps to a known layout. Empty variant skips the line entirely so the system default applies.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_max_width_chars: 60,
                        },
                    },

                    add_named[Some(Page::Wallpaper.name())] = &gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_spacing: 16,

                        gtk::Label {
                            add_css_class: "label-large-bold",
                            set_label: "Wallpaper",
                            set_halign: gtk::Align::Start,
                        },

                        gtk::Label {
                            set_label: "Pick the directory mshell should scan for wallpapers. The rotation cycles through every image found inside, including subdirectories.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_max_width_chars: 60,
                        },

                        gtk::Box {
                            set_orientation: gtk::Orientation::Horizontal,
                            set_spacing: 8,
                            gtk::Entry {
                                set_hexpand: true,
                                set_editable: false,
                                #[watch]
                                set_text: &model.choices.wallpaper_dir.display().to_string(),
                            },
                            gtk::Button {
                                set_label: "Browse…",
                                connect_clicked[sender] => move |_| {
                                    sender.input(WizardInput::OpenWallpaperPicker);
                                },
                            },
                        },

                        gtk::Label {
                            add_css_class: "label-small",
                            set_label: "If the directory doesn't exist yet, mshell will treat the field as a no-op and won't rotate. You can fix it later by dropping images into the folder or pointing Settings → Wallpaper somewhere else.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_max_width_chars: 60,
                        },
                    },

                    add_named[Some(Page::Done.name())] = &gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_spacing: 12,

                        gtk::Label {
                            add_css_class: "label-large-bold",
                            set_label: "Ready to apply",
                            set_halign: gtk::Align::Start,
                        },

                        gtk::Label {
                            #[watch]
                            set_label: &format!(
                                "Color mode:      {:?}\nClock format:    {}\nXkb layout:      {}{}\nWallpaper dir:   {}\nProfile target:  {}\nMargo config:    {}",
                                model.choices.matugen_mode,
                                if model.choices.clock_24h { "24-hour" } else { "12-hour" },
                                model.choices.xkb_layout,
                                if model.choices.xkb_variant.is_empty() {
                                    String::new()
                                } else {
                                    format!(" (variant: {})", model.choices.xkb_variant)
                                },
                                model.choices.wallpaper_dir.display(),
                                default_config_path().display(),
                                margo_conf_path().display(),
                            ),
                            add_css_class: "monospace",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },

                        gtk::Label {
                            #[watch]
                            set_label: &match &model.apply_status {
                                ApplyStatus::Pending => String::new(),
                                ApplyStatus::Ok(path) => format!(
                                    "✓ Wrote {}", path.display()
                                ),
                                ApplyStatus::Err(msg) => format!("✗ {msg}"),
                            },
                            #[watch]
                            set_visible: !matches!(model.apply_status, ApplyStatus::Pending),
                            set_halign: gtk::Align::Start,
                            set_margin_top: 12,
                        },
                    },
                },

                // Footer: Back / Cancel + Next/Apply
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 8,

                    gtk::Button {
                        set_label: "Cancel",
                        connect_clicked[sender] => move |_| {
                            sender.input(WizardInput::Cancel);
                        },
                    },

                    gtk::Box { set_hexpand: true },

                    gtk::Button {
                        set_label: "Back",
                        #[watch]
                        set_sensitive: model.page.prev().is_some(),
                        connect_clicked[sender] => move |_| {
                            sender.input(WizardInput::Back);
                        },
                    },

                    gtk::Button {
                        add_css_class: "ok-button-primary",
                        #[watch]
                        set_label: if model.page == Page::Done {
                            if matches!(model.apply_status, ApplyStatus::Ok(_)) {
                                "Close"
                            } else {
                                "Apply"
                            }
                        } else {
                            "Next"
                        },
                        connect_clicked[sender] => move |_| {
                            sender.input(WizardInput::Next);
                        },
                    },
                },
            }
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = WizardModel {
            page: Page::Welcome,
            choices: Choices::defaults(),
            apply_status: ApplyStatus::Pending,
        };
        let widgets = view_output!();
        let _ = sender;
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>) {
        match message {
            WizardInput::Next => {
                if self.page == Page::Done {
                    if matches!(self.apply_status, ApplyStatus::Ok(_)) {
                        relm4::main_application().quit();
                        return;
                    }
                    self.apply_status = match apply_choices(&self.choices) {
                        Ok(path) => ApplyStatus::Ok(path),
                        Err(err) => ApplyStatus::Err(err.to_string()),
                    };
                } else if let Some(next) = self.page.next() {
                    self.page = next;
                }
            }
            WizardInput::Back => {
                if let Some(prev) = self.page.prev() {
                    self.page = prev;
                }
            }
            WizardInput::Cancel => {
                relm4::main_application().quit();
            }
            WizardInput::MatugenModeChanged(mode) => {
                self.choices.matugen_mode = mode;
            }
            WizardInput::Clock24hChanged(v) => {
                self.choices.clock_24h = v;
            }
            WizardInput::XkbLayoutChanged(s) => {
                self.choices.xkb_layout = s.trim().to_string();
            }
            WizardInput::XkbVariantChanged(s) => {
                self.choices.xkb_variant = s.trim().to_string();
            }
            WizardInput::WallpaperDirPicked(path) => {
                self.choices.wallpaper_dir = path;
            }
            WizardInput::OpenWallpaperPicker => {
                // GTK4 FileDialog (gtk4 ≥ 4.10) — async, but
                // we wrap the result back to ourselves via the
                // input channel so the picked path lands in
                // `WallpaperDirPicked` on the next tick.
                let dialog = gtk::FileDialog::builder()
                    .title("Pick wallpaper directory")
                    .build();
                let cur = gtk::gio::File::for_path(&self.choices.wallpaper_dir);
                dialog.set_initial_folder(Some(&cur));
                let sender_clone = sender.clone();
                dialog.select_folder(
                    None::<&gtk::Window>,
                    None::<&gtk::gio::Cancellable>,
                    move |res| {
                        if let Ok(folder) = res
                            && let Some(path) = folder.path()
                        {
                            sender_clone.input(WizardInput::WallpaperDirPicked(path));
                        }
                    },
                );
            }
        }
    }
}

/// Canonical compositor-side config path. Margo reads
/// `~/.config/margo/config.conf` at startup (see
/// `margo-config::parser::default_config_path`); we mirror the
/// path here rather than depending on margo-config just for one
/// constant.
fn margo_conf_path() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".config/margo/config.conf"))
        .unwrap_or_else(|| PathBuf::from("/tmp/margo-config.conf"))
}

/// Fold the user's wizard choices into:
/// 1. a freshly-defaulted shell-side `Config` (serialised to
///    `default.yaml`), and
/// 2. surgical `xkb_rules_layout` / `xkb_rules_variant` lines in
///    `config.conf` (created if missing, patched in-place if
///    present — non-xkb config keeps the user's existing tweaks).
///
/// Returns the profile path so the Done page can show it; the
/// margo.conf path is logged but not surfaced as the "success"
/// path since it's a secondary side-effect.
fn apply_choices(choices: &Choices) -> Result<PathBuf> {
    // ── shell profile ─────────────────────────────────────────
    let mut cfg = Config::default();
    cfg.general.clock_format_24_h = choices.clock_24h;
    cfg.theme.matugen.mode = choices.matugen_mode;
    cfg.wallpaper.wallpaper_dir = choices.wallpaper_dir.display().to_string();

    let target = default_config_path();
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!("create profile dir {}", parent.display())
        })?;
    }
    let yaml = serde_yaml::to_string(&cfg)
        .context("serialize Config to YAML")?;
    std::fs::write(&target, yaml)
        .with_context(|| format!("write profile {}", target.display()))?;
    tracing::info!(path = %target.display(), "wrote profile");

    // ── compositor xkb lines ──────────────────────────────────
    write_xkb_to_margo_conf(&choices.xkb_layout, &choices.xkb_variant)
        .with_context(|| "patch xkb_rules_* in margo config.conf")?;

    Ok(target)
}

/// Read `~/.config/margo/config.conf` (or treat as empty when
/// missing), patch `xkb_rules_layout` + `xkb_rules_variant`
/// lines in-place, and write the result back. Lines outside the
/// xkb pair are preserved verbatim so a user who already
/// hand-edited margo.conf doesn't lose unrelated tweaks.
///
/// Empty `variant` deletes the existing variant line entirely;
/// passing the system default (empty) explicitly is more useful
/// than leaving a stale variant in place.
fn write_xkb_to_margo_conf(layout: &str, variant: &str) -> Result<()> {
    let path = margo_conf_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!("create margo dir {}", parent.display())
        })?;
    }

    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let mut out = String::with_capacity(existing.len() + 128);
    let mut saw_layout = false;
    let mut saw_variant = false;

    for line in existing.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("xkb_rules_layout") {
            // matches `xkb_rules_layout = …` (with or without
            // whitespace around the `=`); replace the entire line.
            if rest.trim_start().starts_with('=') {
                out.push_str(&format!("xkb_rules_layout = {}\n", layout));
                saw_layout = true;
                continue;
            }
        }
        if let Some(rest) = trimmed.strip_prefix("xkb_rules_variant")
            && rest.trim_start().starts_with('=') {
                if variant.is_empty() {
                    // Drop the line — empty variant means "use
                    // system default", so a stale value would be
                    // surprising.
                    saw_variant = true;
                    continue;
                }
                out.push_str(&format!("xkb_rules_variant = {}\n", variant));
                saw_variant = true;
                continue;
            }
        out.push_str(line);
        out.push('\n');
    }

    if !saw_layout {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&format!("xkb_rules_layout = {}\n", layout));
    }
    if !saw_variant && !variant.is_empty() {
        out.push_str(&format!("xkb_rules_variant = {}\n", variant));
    }

    std::fs::write(&path, out)
        .with_context(|| format!("write margo config {}", path.display()))?;
    tracing::info!(path = %path.display(), layout, variant, "patched xkb rules");
    Ok(())
}
