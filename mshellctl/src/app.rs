use clap::{Parser, Subcommand};
use clap_complete::Shell;
use std::path::PathBuf;

use crate::subcommands::audio::AudioCommands;
use crate::subcommands::bar::BarCommands;
use crate::subcommands::bluetooth::BluetoothCommands;
use crate::subcommands::brightness::BrightnessCommands;
use crate::subcommands::calendar::CalendarCommands;
use crate::subcommands::color::ColorArgs;
use crate::subcommands::layout::LayoutCommands;
use crate::subcommands::lock::LockCommands;
use crate::subcommands::media::MediaCommands;
use crate::subcommands::menu::MenuCommands;
use crate::subcommands::notification::NotificationCommands;
use crate::subcommands::osk::OskCommands;
use crate::subcommands::play::PlayCommands;
use crate::subcommands::plugin::PluginCommands;
use crate::subcommands::power::PowerCommands;
use crate::subcommands::screen_record::ScreenRecordCommands;
use crate::subcommands::screenshot::ScreenshotCommands;
use crate::subcommands::session::SessionCommands;
use crate::subcommands::settings::SettingsCommands;
use crate::subcommands::theme::ThemeCommands;
use crate::subcommands::vpn::VpnCommands;
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

mshellctl is the single control surface for the SHELL — menus, bars,
audio, brightness, wallpaper, notifications, the session, and the
companion tools (calendar, VPN, power, on-screen keyboard, colour
picker, mpv, saved layouts). It is distinct from `mctl`, which controls
the COMPOSITOR (windows, tags, tiling). The two talk to different daemons.

EXAMPLES
  Surfaces      mshellctl menu control-center   # toggle quick-settings
                mshellctl menu mdash            # toggle the dashboard
                mshellctl bar toggle all        # show/hide the bars
  Audio/media   mshellctl audio volume 60       # set output volume 60%
                mshellctl audio mute            # toggle output mute
                mshellctl media toggle          # MPRIS play/pause
  Display       mshellctl brightness up         # raise backlight 5%
                mshellctl wallpaper next        # cycle wallpaper
                mshellctl theme set kenp        # switch scheme live
  Session       mshellctl lock                  # lock the session
                mshellctl session logout        # log out
                mshellctl notification dnd on   # Do Not Disturb on
  Tools         mshellctl calendar today        # today's events (mcal)
                mshellctl vpn toggle            # Mullvad on/off (mvpn)
                mshellctl power set balanced    # power profile (mpower)
                mshellctl osk toggle            # on-screen keyboard
                mshellctl color --copy          # pick a colour (mpicker)

SEE ALSO
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
    /// Open, close, or toggle a shell menu (control-center, mdash,
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
    /// Control a Hidden Bar drawer widget. With no `name`, the verb reaches
    /// every drawer; pass a `name` to target one named drawer (a
    /// `bars.widgets.hidden_bars` entry, placed via `!HiddenBarNamed <name>`).
    HiddenBar {
        /// toggle | expand | collapse | pin | unpin
        action: String,
        /// Optional drawer name to target (omit to act on all drawers).
        name: Option<String>,
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
    /// Session / power actions — lock, logout, suspend, reboot, shutdown, or
    /// open the session menu. Bind `session logout` to a key to replace a
    /// wlogout/rofi script.
    Session {
        #[command(subcommand)]
        command: SessionCommands,
    },
    /// Notification centre — open the panel, clear, dismiss popups, toggle
    /// Do Not Disturb, or print the history count.
    Notification {
        #[command(subcommand)]
        command: NotificationCommands,
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
    /// Capture a screenshot — region / window / output / full, delivered to
    /// file + clipboard (or `--copy` / `--save` / `--edit`). Drives the
    /// shell's own screenshot engine (same path as the screenshot menu).
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
    /// Show a transient state-change toast — the `notify-send` equivalent for
    /// the toast surface. Ephemeral (no notification history); usable from
    /// scripts and startup services.
    Toast {
        /// Toast title (the bold first line).
        title: String,
        /// Optional body line.
        body: Option<String>,
        /// Symbolic icon name (default: dialog-information-symbolic).
        #[arg(long)]
        icon: Option<String>,
        /// Severity tint: calm | warn | danger | positive.
        #[arg(long, default_value = "calm")]
        severity: String,
    },
    /// Game Mode — drop compositor effects, silence notifications, and keep
    /// the session awake while gaming. Configure what it affects in
    /// Settings → Game Mode.
    Gamemode {
        /// on | off | toggle | status (default: toggle).
        #[arg(value_enum, default_value = "toggle")]
        action: GameModeAction,
    },
    /// Calendar — today's / upcoming events and connected accounts
    /// (proxied to the `mcal` calendar tool). `calendar account setup
    /// google` connects a Google account.
    Calendar {
        #[command(subcommand)]
        command: CalendarCommands,
    },
    /// Mullvad VPN control — connect / disconnect / pick a relay (proxied to
    /// `mvpn`). Use `vpn menu` for the shell's DNS/VPN panel.
    Vpn {
        #[command(subcommand)]
        command: VpnCommands,
    },
    /// Power-profile control — status / cycle / set / pause / resume (proxied
    /// to the `mpower` daemon). Use `menu power` for the panel.
    Power {
        #[command(subcommand)]
        command: PowerCommands,
    },
    /// Saved tiling-layout snapshots — list / set / next / prev / preview
    /// (proxied to `mlayout`).
    Layout {
        #[command(subcommand)]
        command: LayoutCommands,
    },
    /// On-screen keyboard — show / hide / toggle (proxied to `mkeys`).
    Osk {
        #[command(subcommand)]
        command: OskCommands,
    },
    /// Pick a screen colour (proxied to `mpicker`) — prints the colour;
    /// use `--copy` / `--notify` / `--format`.
    Color(ColorArgs),
    /// mpv companion — play / toggle / stop / snapshot / video wallpaper
    /// (proxied to `mplay`).
    Play {
        #[command(subcommand)]
        command: PlayCommands,
    },
    /// Health check — is the shell on the session bus, is it the same
    /// version as this `mshellctl`, are its key services up. Run
    /// `mctl doctor` for the compositor side.
    Doctor,
    /// Generate a shell-completion script (bash / zsh / fish / …) to stdout.
    Completions {
        /// Shell to generate for.
        #[arg(value_enum)]
        shell: Shell,
    },
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum GameModeAction {
    On,
    Off,
    Toggle,
    Status,
}
