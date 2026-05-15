//! SystemUpdate — bar pill showing the count of pending system
//! upgrades.
//!
//! Polls every N minutes (default 180; configurable in Settings
//! → Widgets → System Updates). Left click spawns a terminal
//! running the package manager's upgrade command. Right click
//! forces an immediate re-probe — useful right after upgrading
//! from outside mshell when you want the count to refresh
//! without waiting for the next scheduled tick.
//!
//! ## Detection
//!
//! Linux package managers vary; we probe in priority order and
//! cache the chosen backend for the widget's lifetime:
//!
//!   * **AUR helpers** (yay / paru / pikaur): preferred — they
//!     cover repo + AUR in one pass without sudo.
//!   * **pacman + checkupdates** (Arch): `checkupdates` refreshes
//!     a fake DB under `/tmp`, no sudo. Exit 2 = no updates.
//!   * **plain `pacman -Qu`** (fallback): only sees packages
//!     already cached after a previous `-Sy`. Exit 1 = no
//!     upgrades (pacman convention — *not* an error).
//!   * **dnf** (Fedora): `dnf check-update --refresh -q`. Exit
//!     100 = updates available, 0 = no updates.
//!   * **apt** (Debian/Ubuntu): `apt list --upgradable`. Always
//!     exits 0; counts `[upgradable from:...]` lines.
//!
//! If none of the above binaries exist, the widget hides itself
//! and logs once.
//!
//! ## Exit-code convention
//!
//! `pacman -Qu` / `yay -Qu` / `paru -Qu` / `pikaur -Qu` exit 1
//! when there's nothing to upgrade. We treat that as "no
//! updates" (count = 0) rather than as an error. Only spawn
//! failures or genuinely non-zero exit codes outside of
//! {0, 1, 2-for-checkupdates, 100-for-dnf} surface as errors.
//!
//! ## Click action
//!
//! Spawns the user's terminal (auto-detected: kitty → alacritty
//! → foot → wezterm → konsole → gnome-terminal → xterm) with
//! the matching upgrade command (`sudo pacman -Syu`, `sudo dnf
//! upgrade`, `sudo apt upgrade`). When an AUR helper is the
//! active backend we use it directly (no sudo needed) so a
//! single click covers AUR + repo updates.

use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{BarWidgetsStoreFields, BarsStoreFields, ConfigStoreFields, SystemUpdateBarWidgetStoreFields};
use reactive_graph::traits::{Get, GetUntracked};
use relm4::gtk::Orientation;
use relm4::gtk::prelude::{BoxExt, ButtonExt, GestureSingleExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::Duration;
use tokio::sync::Notify;
use tracing::warn;

/// First probe lands shortly after launch — long enough that we
/// don't fight idle-CPU-drain testing on cold boot, short enough
/// that the user sees a meaningful count by the time they look at
/// the bar.
const STARTUP_DELAY: Duration = Duration::from_secs(10);
/// Defensive floor on the configured interval. Anything smaller
/// would hammer the repo mirrors.
const MIN_INTERVAL: Duration = Duration::from_secs(60);

/// Which package manager is in charge here. Discovered once on
/// first poll and cached in the model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Backend {
    /// Pacman without an AUR helper — `checkupdates` if present,
    /// else `pacman -Qu` against the cached DB.
    PacmanRepoOnly,
    PacmanWithHelper(&'static str),
    Dnf,
    Apt,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct UpdateState {
    /// Pending update count. `None` until the first poll
    /// completes; rendered as a "checking…" state by the tooltip.
    count: Option<u32>,
    /// `Some` when probing failed; the pill renders with an
    /// error tint and the message lands in the tooltip. The user
    /// can right-click to retry without waiting for the next
    /// scheduled tick.
    error: Option<String>,
}

pub(crate) struct SystemUpdateModel {
    state: UpdateState,
    backend: Option<Backend>,
    _orientation: Orientation,
    /// Wakes the polling task for an immediate re-probe. Fired
    /// from `ManualRefresh` (right-click) and held in an Arc so
    /// the spawned command task and the model both refer to the
    /// same Notify instance.
    refresh_notify: std::sync::Arc<Notify>,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum SystemUpdateInput {
    /// Left click → spawn a terminal running the upgrade
    /// command.
    Clicked,
    /// Right click → immediate manual re-probe. Wakes the
    /// polling task via the shared Notify so we don't grow a
    /// parallel probe task; the existing one just runs ahead of
    /// schedule.
    ManualRefresh,
}

#[derive(Debug)]
pub(crate) enum SystemUpdateOutput {}

pub(crate) struct SystemUpdateInit {
    pub(crate) orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum SystemUpdateCommandOutput {
    /// Background poll landed a fresh state. `None` for backend
    /// means the probe could not pick one (no supported binary on
    /// $PATH) — the widget hides in that case.
    Refreshed {
        backend: Option<Backend>,
        state: UpdateState,
    },
    /// Right-click hit — clear the cached count so the icon falls
    /// back to "checking…" while the probe re-runs. Separate from
    /// `Refreshed` so the loading state is visible even if the
    /// probe completes quickly.
    Checking,
}

#[relm4::component(pub)]
impl Component for SystemUpdateModel {
    type CommandOutput = SystemUpdateCommandOutput;
    type Input = SystemUpdateInput;
    type Output = SystemUpdateOutput;
    type Init = SystemUpdateInit;

    view! {
        #[root]
        #[name = "root"]
        gtk::Box {
            #[watch]
            set_css_classes: &css_classes(&model.state),
            set_hexpand: model._orientation == Orientation::Vertical,
            set_vexpand: model._orientation == Orientation::Horizontal,
            set_halign: gtk::Align::Center,
            set_valign: gtk::Align::Center,
            set_has_tooltip: true,
            #[watch]
            set_tooltip_text: Some(&tooltip(model.backend, &model.state)),
            // Hide the pill entirely when no backend was found.
            // Less noise on distros where this widget can't
            // function.
            #[watch]
            set_visible: model.backend.is_some(),

            gtk::Button {
                set_css_classes: &["ok-button-flat", "ok-bar-widget"],
                set_hexpand: false,
                set_vexpand: false,
                connect_clicked[sender] => move |_| {
                    sender.input(SystemUpdateInput::Clicked);
                },

                gtk::Box {
                    set_orientation: Orientation::Horizontal,
                    set_spacing: 4,
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,

                    gtk::Image {
                        #[watch]
                        set_icon_name: Some(icon_for(&model.state)),
                    },
                    gtk::Label {
                        add_css_class: "system-update-bar-label",
                        #[watch]
                        set_label: &label_for(&model.state),
                        #[watch]
                        set_visible: should_show_label(&model.state),
                    },
                },
            },
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let refresh_notify = std::sync::Arc::new(Notify::new());

        // Background polling task. Reads the configured interval
        // on each iteration (via `get_untracked`) so a Settings
        // change takes effect on the next loop tick without a
        // separate signal.
        let notify_for_task = refresh_notify.clone();
        sender.command(move |out, shutdown| {
            let notify = notify_for_task;
            async move {
                let shutdown_fut = shutdown.wait();
                tokio::pin!(shutdown_fut);
                let mut first = true;
                let mut backend: Option<Backend> = None;
                loop {
                    let delay = if first {
                        STARTUP_DELAY
                    } else {
                        configured_interval()
                    };
                    first = false;
                    tokio::select! {
                        () = &mut shutdown_fut => break,
                        _ = tokio::time::sleep(delay) => {}
                        // Right-click → run ahead of schedule.
                        // tokio::sync::Notify.notified() resolves
                        // on the next `notify_one`, so a click
                        // during sleep wakes us straight into the
                        // probe block below.
                        _ = notify.notified() => {}
                    }
                    if backend.is_none() {
                        backend = detect_backend().await;
                        if backend.is_none() {
                            static LOGGED: std::sync::atomic::AtomicBool =
                                std::sync::atomic::AtomicBool::new(false);
                            if !LOGGED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                                warn!(
                                    "system_update: no supported package manager on PATH (tried yay, paru, pikaur, pacman, dnf, apt)"
                                );
                            }
                        }
                    }
                    let state = match backend {
                        Some(b) => probe(b).await,
                        None => UpdateState::default(),
                    };
                    let _ = out.send(SystemUpdateCommandOutput::Refreshed { backend, state });
                }
            }
        });

        // Reactive effect: when the user changes the interval in
        // Settings, the next loop iteration will pick up the new
        // value via `configured_interval()`. We don't poke the
        // Notify here — that would re-probe on every config
        // touch (including unrelated settings).
        let mut effects = EffectScope::new();
        effects.push(|_| {
            // Subscribe so a future migration to "wake on
            // interval change" can plug in here without
            // restructuring.
            let _ = config_manager()
                .config()
                .bars()
                .widgets()
                .system_update()
                .check_interval_minutes()
                .get();
        });

        let model = SystemUpdateModel {
            state: UpdateState::default(),
            backend: None,
            _orientation: params.orientation,
            refresh_notify,
            _effects: effects,
        };
        let widgets = view_output!();

        // Right-click — runs the manual probe. Wired on the root
        // Box so the entire pill area is clickable, not just the
        // Button (which already eats left clicks for the upgrade
        // launcher).
        let gesture = gtk::GestureClick::new();
        gesture.set_button(gtk::gdk::BUTTON_SECONDARY);
        let refresh_sender = sender.clone();
        gesture.connect_pressed(move |_, _, _, _| {
            refresh_sender.input(SystemUpdateInput::ManualRefresh);
        });
        widgets.root.add_controller(gesture);

        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            SystemUpdateInput::Clicked => {
                let backend = self.backend;
                relm4::spawn(async move {
                    if let Some(b) = backend {
                        launch_terminal_upgrade(b).await;
                    }
                });
            }
            SystemUpdateInput::ManualRefresh => {
                // Show the "checking…" state straight away so the
                // user sees feedback for their right-click even
                // before the probe completes.
                let cmd_sender = sender.command_sender().clone();
                let _ = cmd_sender.send(SystemUpdateCommandOutput::Checking);
                // Wake the polling task.
                self.refresh_notify.notify_one();
            }
        }
    }

    fn update_cmd(
        &mut self,
        message: Self::CommandOutput,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            SystemUpdateCommandOutput::Refreshed { backend, state } => {
                self.backend = backend;
                self.state = state;
            }
            SystemUpdateCommandOutput::Checking => {
                self.state = UpdateState::default();
            }
        }
    }
}

// ── Config helper ───────────────────────────────────────────────

fn configured_interval() -> Duration {
    let minutes = config_manager()
        .config()
        .bars()
        .widgets()
        .system_update()
        .check_interval_minutes()
        .get_untracked();
    let dur = Duration::from_secs((minutes as u64).saturating_mul(60));
    if dur < MIN_INTERVAL { MIN_INTERVAL } else { dur }
}

// ── View helpers ────────────────────────────────────────────────

fn css_classes(state: &UpdateState) -> Vec<&'static str> {
    let mut classes = vec!["ok-button-surface", "ok-bar-widget", "system-update-bar-widget"];
    if state.error.is_some() {
        classes.push("error");
    } else if state.count.unwrap_or(0) > 0 {
        classes.push("has-updates");
    }
    classes
}

fn icon_for(state: &UpdateState) -> &'static str {
    if state.error.is_some() {
        "software-update-urgent-symbolic"
    } else if state.count.unwrap_or(0) > 0 {
        "software-update-available-symbolic"
    } else {
        "package-symbolic"
    }
}

fn label_for(state: &UpdateState) -> String {
    match state.count {
        Some(n) if n > 0 => n.to_string(),
        _ => String::new(),
    }
}

fn should_show_label(state: &UpdateState) -> bool {
    state.error.is_none() && state.count.unwrap_or(0) > 0
}

fn tooltip(backend: Option<Backend>, state: &UpdateState) -> String {
    let footer = "\n\nRight-click to re-check now.";
    if let Some(err) = &state.error {
        return format!("Updates: {err}{footer}");
    }
    let backend_label = backend.map(backend_label).unwrap_or("none");
    match state.count {
        None => format!("Updates ({backend_label}): checking…{footer}"),
        Some(0) => format!("Updates ({backend_label}): system is up to date{footer}"),
        Some(1) => format!("Updates ({backend_label}): 1 pending{footer}"),
        Some(n) => format!("Updates ({backend_label}): {n} pending{footer}"),
    }
}

fn backend_label(b: Backend) -> &'static str {
    match b {
        Backend::PacmanRepoOnly => "pacman",
        Backend::PacmanWithHelper(helper) => helper,
        Backend::Dnf => "dnf",
        Backend::Apt => "apt",
    }
}

// ── Backend detection + probing ─────────────────────────────────

async fn detect_backend() -> Option<Backend> {
    // AUR helpers first: they cover both repo + AUR in one pass
    // and the user almost certainly has one if they're on Arch.
    for helper in ["yay", "paru", "pikaur"] {
        if which(helper).await {
            return Some(Backend::PacmanWithHelper(helper));
        }
    }
    if which("pacman").await {
        return Some(Backend::PacmanRepoOnly);
    }
    if which("dnf").await {
        return Some(Backend::Dnf);
    }
    if which("apt").await {
        return Some(Backend::Apt);
    }
    None
}

async fn which(binary: &str) -> bool {
    tokio::process::Command::new("which")
        .arg(binary)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

async fn probe(backend: Backend) -> UpdateState {
    match backend {
        Backend::PacmanWithHelper(helper) => probe_pacman_family(helper, &["-Qu"]).await,
        Backend::PacmanRepoOnly => probe_pacman_plain().await,
        Backend::Dnf => probe_dnf().await,
        Backend::Apt => probe_apt().await,
    }
}

/// pacman / yay / paru / pikaur all share the `-Qu` interface.
/// Output: one upgradable package per line. Exit codes:
///   0  → updates listed on stdout (count = nonempty lines)
///   1  → no upgrades available (count = 0; NOT an error)
///   *  → genuine failure (DB locked, etc.) → surface in tooltip
async fn probe_pacman_family(cmd: &str, args: &[&str]) -> UpdateState {
    let res = tokio::process::Command::new(cmd)
        .args(args)
        .stderr(std::process::Stdio::piped())
        .output()
        .await;
    match res {
        Ok(o) => {
            let code = o.status.code().unwrap_or(-1);
            match code {
                0 => count_nonempty_lines(&String::from_utf8_lossy(&o.stdout)),
                1 => ok_state(0),
                _ => {
                    let stderr = String::from_utf8_lossy(&o.stderr).trim().to_string();
                    if stderr.is_empty() {
                        err_state(format!("{cmd}: exit {code}"))
                    } else {
                        err_state(format!("{cmd} exit {code}: {stderr}"))
                    }
                }
            }
        }
        Err(e) => err_state(format!("{cmd} spawn: {e}")),
    }
}

/// Without an AUR helper, prefer `checkupdates` (in `pacman-
/// contrib`): refreshes a fake DB under `/tmp` so the regular
/// `pacman -Qu` doesn't need sudo. If `checkupdates` is missing,
/// fall back to plain `pacman -Qu` which only sees what's already
/// cached after a previous `-Sy`.
async fn probe_pacman_plain() -> UpdateState {
    if which("checkupdates").await {
        let res = tokio::process::Command::new("checkupdates")
            .stderr(std::process::Stdio::piped())
            .output()
            .await;
        match res {
            Ok(o) => {
                let code = o.status.code().unwrap_or(-1);
                match code {
                    0 => return count_nonempty_lines(&String::from_utf8_lossy(&o.stdout)),
                    2 => return ok_state(0),
                    _ => {
                        let stderr = String::from_utf8_lossy(&o.stderr).trim().to_string();
                        return if stderr.is_empty() {
                            err_state(format!("checkupdates: exit {code}"))
                        } else {
                            err_state(format!("checkupdates exit {code}: {stderr}"))
                        };
                    }
                }
            }
            Err(e) => return err_state(format!("checkupdates spawn: {e}")),
        }
    }
    probe_pacman_family("pacman", &["-Qu"]).await
}

/// `dnf check-update` returns:
///   exit 100 ⇒ updates available, listed on stdout
///   exit 0   ⇒ no updates
///   exit !=  ⇒ error
async fn probe_dnf() -> UpdateState {
    let res = tokio::process::Command::new("dnf")
        .args(["check-update", "--refresh", "-q"])
        .output()
        .await;
    match res {
        Err(e) => err_state(format!("dnf spawn: {e}")),
        Ok(out) => {
            let code = out.status.code().unwrap_or(-1);
            if code == 0 {
                ok_state(0)
            } else if code == 100 {
                let body = String::from_utf8_lossy(&out.stdout);
                let n = body
                    .lines()
                    .filter(|l| !l.trim().is_empty() && !l.starts_with(' '))
                    .count() as u32;
                ok_state(n)
            } else {
                let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
                err_state(format!("dnf exit {code}: {stderr}"))
            }
        }
    }
}

/// `apt list --upgradable` lists `<pkg>/<repo> <new> <arch>
/// [upgradable from: ...]`. Header line is "Listing..." — skip
/// any line that doesn't carry the upgradable marker.
async fn probe_apt() -> UpdateState {
    let res = tokio::process::Command::new("apt")
        .args(["list", "--upgradable"])
        .stderr(std::process::Stdio::piped())
        .output()
        .await;
    match res {
        Ok(o) if o.status.success() => {
            let body = String::from_utf8_lossy(&o.stdout);
            let n = body
                .lines()
                .filter(|l| l.contains("[upgradable from:"))
                .count() as u32;
            ok_state(n)
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr).trim().to_string();
            err_state(format!("apt exit {}: {}", o.status.code().unwrap_or(-1), stderr))
        }
        Err(e) => err_state(format!("apt spawn: {e}")),
    }
}

fn count_nonempty_lines(s: &str) -> UpdateState {
    let n = s.lines().filter(|l| !l.trim().is_empty()).count() as u32;
    ok_state(n)
}

fn ok_state(count: u32) -> UpdateState {
    UpdateState {
        count: Some(count),
        error: None,
    }
}

fn err_state<S: Into<String>>(msg: S) -> UpdateState {
    let msg = msg.into();
    warn!(error = %msg, "system_update: probe failed");
    UpdateState {
        count: None,
        error: Some(msg),
    }
}

// ── Click action: open a terminal running the upgrade ──────────

async fn launch_terminal_upgrade(backend: Backend) {
    let (program, needs_sudo) = upgrade_command(backend);
    let Some(term) = detect_terminal().await else {
        warn!("system_update: no terminal emulator on PATH; click ignored");
        return;
    };
    let inner = if needs_sudo {
        format!("sudo {program}")
    } else {
        program.to_string()
    };
    let script = format!("{inner}; echo; echo \"[mshell] done — press Enter to close.\"; read");
    let args: Vec<String> = match term {
        "kitty" => vec!["--".into(), "sh".into(), "-c".into(), script],
        "alacritty" | "wezterm" => vec!["-e".into(), "sh".into(), "-c".into(), script],
        "foot" => vec!["sh".into(), "-c".into(), script],
        "konsole" => vec!["-e".into(), "sh".into(), "-c".into(), script],
        "gnome-terminal" => vec!["--".into(), "sh".into(), "-c".into(), script],
        _ => vec!["-e".into(), "sh".into(), "-c".into(), script],
    };
    if let Err(e) = tokio::process::Command::new(term)
        .args(&args)
        .spawn()
    {
        warn!(error = %e, term, "system_update: terminal spawn failed");
    }
}

fn upgrade_command(backend: Backend) -> (&'static str, bool) {
    match backend {
        Backend::PacmanWithHelper("yay") => ("yay -Syu", false),
        Backend::PacmanWithHelper("paru") => ("paru -Syu", false),
        Backend::PacmanWithHelper("pikaur") => ("pikaur -Syu", false),
        Backend::PacmanWithHelper(_) => ("pacman -Syu", true),
        Backend::PacmanRepoOnly => ("pacman -Syu", true),
        Backend::Dnf => ("dnf upgrade", true),
        Backend::Apt => ("apt upgrade", true),
    }
}

async fn detect_terminal() -> Option<&'static str> {
    for term in ["kitty", "alacritty", "foot", "wezterm", "konsole", "gnome-terminal", "xterm"] {
        if which(term).await {
            return Some(term);
        }
    }
    None
}
