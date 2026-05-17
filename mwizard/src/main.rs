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
//! 3. **Wallpaper** — directory the wallpaper rotation pulls from.
//! 4. **Done** — summary + Apply button that serialises the
//!    profile YAML into place.
//!
//! Wizard intentionally writes ONLY the shell-side profile
//! (`mshell-config`'s `Config`). The compositor's `margo.conf` is
//! left to its built-in defaults — touching it here would mean
//! shipping a second template + parser dependency, and the user
//! can hit `mctl config edit` later. The shell profile alone is
//! enough to flip the major visible knobs (theme, clock, wallpaper
//! source).

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
    app.run::<WizardModel>(WizardInit {});
}

/// Index of the visible page in `gtk::Stack`. Sequential — the
/// wizard doesn't allow free-jumping between pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Welcome,
    Theme,
    Wallpaper,
    Done,
}

impl Page {
    fn index(self) -> u32 {
        match self {
            Self::Welcome => 0,
            Self::Theme => 1,
            Self::Wallpaper => 2,
            Self::Done => 3,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Welcome => "welcome",
            Self::Theme => "theme",
            Self::Wallpaper => "wallpaper",
            Self::Done => "done",
        }
    }

    fn next(self) -> Option<Self> {
        match self {
            Self::Welcome => Some(Self::Theme),
            Self::Theme => Some(Self::Wallpaper),
            Self::Wallpaper => Some(Self::Done),
            Self::Done => None,
        }
    }

    fn prev(self) -> Option<Self> {
        match self {
            Self::Welcome => None,
            Self::Theme => Some(Self::Welcome),
            Self::Wallpaper => Some(Self::Theme),
            Self::Done => Some(Self::Wallpaper),
        }
    }
}

/// Wizard-side mirror of every knob the pages can toggle. Folded
/// into a fresh `Config::default()` on Apply — keeping the user's
/// choices in a tiny intermediate struct lets the pages emit
/// updates without owning the entire `Config` reactive tree.
#[derive(Debug, Clone)]
struct Choices {
    matugen_mode: MatugenMode,
    clock_24h: bool,
    wallpaper_dir: PathBuf,
}

impl Choices {
    fn defaults() -> Self {
        Self {
            matugen_mode: MatugenMode::Dark,
            clock_24h: true,
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
                    add_css_class: "title-3",
                    #[watch]
                    set_label: &format!(
                        "Step {} of 4",
                        model.page.index() + 1
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
                            add_css_class: "title-1",
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
                            add_css_class: "title-2",
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
                            add_css_class: "dim-label",
                            set_label: "Both knobs are live — Settings → Theme will let you tweak the matugen palette, accent tints, and font sizes once mshell is running.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_max_width_chars: 60,
                            set_margin_top: 12,
                        },
                    },

                    add_named[Some(Page::Wallpaper.name())] = &gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_spacing: 16,

                        gtk::Label {
                            add_css_class: "title-2",
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
                            add_css_class: "dim-label",
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
                            add_css_class: "title-2",
                            set_label: "Ready to apply",
                            set_halign: gtk::Align::Start,
                        },

                        gtk::Label {
                            #[watch]
                            set_label: &format!(
                                "Color mode:      {:?}\nClock format:    {}\nWallpaper dir:   {}\nProfile target:  {}",
                                model.choices.matugen_mode,
                                if model.choices.clock_24h { "24-hour" } else { "12-hour" },
                                model.choices.wallpaper_dir.display(),
                                default_config_path().display(),
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
                        add_css_class: "suggested-action",
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

/// Fold the user's wizard choices into a freshly-defaulted
/// `Config` and write the YAML to disk. Returns the absolute path
/// of the written file so the success label can show it.
fn apply_choices(choices: &Choices) -> Result<PathBuf> {
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
    Ok(target)
}
