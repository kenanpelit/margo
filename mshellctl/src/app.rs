use clap::{Parser, Subcommand};
use std::path::PathBuf;

use crate::subcommands::audio::AudioCommands;
use crate::subcommands::bar::BarCommands;
use crate::subcommands::bluetooth::BluetoothCommands;
use crate::subcommands::brightness::BrightnessCommands;
use crate::subcommands::lock::LockCommands;
use crate::subcommands::media::MediaCommands;
use crate::subcommands::menu::MenuCommands;
use crate::subcommands::plugin::PluginCommands;
use crate::subcommands::screen_record::ScreenRecordCommands;
use crate::subcommands::screenshot::ScreenshotCommands;
use crate::subcommands::settings::SettingsCommands;
use crate::subcommands::theme::ThemeCommands;
use crate::subcommands::wallpaper::WallpaperCommands;
use mshell_cli_style;

#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "Control the margo desktop shell (mshell) over D-Bus.",
    long_about = "\
Control the running margo desktop shell (mshell) over the session
D-Bus (service `com.mshell.Shell`).

mshellctl drives the SHELL — menus, bars, audio, brightness, wallpaper,
the lock screen — and is distinct from `mctl`, which controls the
COMPOSITOR (windows, tags, layouts). The two talk to different daemons.

EXAMPLES:
  mshellctl menu control-center     # toggle the quick-settings panel
  mshellctl menu dashboard          # toggle the dashboard
  mshellctl audio volume 60         # set output volume to 60%
  mshellctl audio mute              # toggle output mute
  mshellctl brightness up           # raise backlight 5%
  mshellctl media toggle            # MPRIS play/pause the active player
  mshellctl wallpaper next          # cycle to the next wallpaper
  mshellctl theme set eventide      # switch colour scheme live
  mshellctl lock                    # lock the session

SEE ALSO:
  man mshellctl, man mctl, man margo"
)]
#[command(styles = mshell_cli_style::get_styles())]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Quit the shell process — bars and menus disappear, but the
    /// compositor (margo) keeps running.
    Quit,
    /// Open the GTK4 inspector — handy for finding CSS node names and
    /// classes while styling.
    Inspect,
    /// Set the wallpaper to a specific image (one-shot). Use `wallpaper`
    /// to cycle through a directory instead.
    SetWallpaper {
        /// Path to the image file
        path: PathBuf,
    },
    /// Open, close, or toggle a shell menu (control-center, dashboard,
    /// clipboard, …).
    Menu {
        #[command(subcommand)]
        command: MenuCommands,
    },
    /// Show, hide, or toggle the top and bottom bars.
    Bar {
        #[command(subcommand)]
        command: BarCommands,
    },
    /// Control the Hidden Bar drawer widget.
    HiddenBar {
        /// toggle | expand | collapse | pin | unpin
        action: String,
    },
    /// Audio control — output/input volume, mute, and device switching.
    Audio {
        #[command(subcommand)]
        command: AudioCommands,
    },
    /// Bluetooth auto-connect engine: toggle / connect / disconnect the
    /// configured device(s). Bind `bluetooth toggle` to F10 to replace the
    /// old bluetooth_toggle script.
    Bluetooth {
        #[command(subcommand)]
        command: BluetoothCommands,
    },
    /// Control the active MPRIS media player — play/pause, next, previous.
    Media {
        #[command(subcommand)]
        command: MediaCommands,
    },
    /// Backlight brightness — raise, lower, or set a level.
    Brightness {
        #[command(subcommand)]
        command: BrightnessCommands,
    },
    /// Shell file-logging controls (~/.local/state/margo/logs/mshell-*.log).
    /// `level`/`enable`/`disable` retune the running shell live.
    Log {
        #[command(subcommand)]
        command: crate::subcommands::log::LogCommands,
    },
    /// Standalone mdock surface — toggle / show / hide.
    Dock {
        #[command(subcommand)]
        command: crate::subcommands::dock::DockCommands,
    },
    /// Lock the session. Bare `lock` locks; `lock check` reports state.
    Lock {
        #[command(subcommand)]
        command: Option<LockCommands>,
    },
    /// Open or close the Settings window.
    Settings {
        #[command(subcommand)]
        command: SettingsCommands,
    },
    /// Open the in-shell setup wizard (a layer-shell menu).
    Wizard,
    /// Cycle the wallpaper — next, previous, or random.
    Wallpaper {
        #[command(subcommand)]
        command: WallpaperCommands,
    },
    /// Colour scheme — list, show, or switch the active scheme live
    /// (the same picker as Settings → Theme → Color Scheme; no restart).
    Theme {
        #[command(subcommand)]
        command: ThemeCommands,
    },
    /// Manage installed WASM plugins (reload from disk, …).
    Plugin {
        #[command(subcommand)]
        command: PluginCommands,
    },
    /// Bridge commands for the mscreenshot CLI — currently the
    /// `select-region` helper that lets external tools reuse the
    /// in-shell area selector (preview state, snap-to-window,
    /// aspect info) instead of spawning `slurp`.
    Screenshot {
        #[command(subcommand)]
        command: ScreenshotCommands,
    },
    /// Screen recording — start / stop / toggle. Drives the shell's own
    /// recording engine (same as the screenshot menu's recording section).
    #[command(name = "screenrecord")]
    ScreenRecord {
        #[command(subcommand)]
        command: ScreenRecordCommands,
    },
    /// Clipboard history — list / copy / pin / delete / clear / wipe.
    /// Drives the same store as `mshellctl menu clipboard`.
    Clipboard {
        #[command(subcommand)]
        command: crate::subcommands::clipboard::ClipboardCommands,
    },
}
