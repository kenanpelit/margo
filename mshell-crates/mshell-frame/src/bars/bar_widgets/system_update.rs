//! SystemUpdate — bar pill showing the count of pending system
//! upgrades.
//!
//! Polls every 30 min. Click spawns a terminal running the
//! package manager's upgrade command.
//!
//! ## Detection
//!
//! Linux package managers vary; we probe in priority order and
//! cache the chosen backend for the widget's lifetime:
//!
//!   * **pacman** (Arch): prefer `checkupdates` (refreshes a
//!     fake DB in `/tmp`, no sudo needed). If absent, fall back
//!     to `pacman -Qu` which only shows packages whose newer
//!     versions are already cached locally — less accurate but
//!     no privilege required.
//!   * **dnf** (Fedora): `dnf check-update --refresh -q` exits 100
//!     when updates exist; one line per package on stdout.
//!   * **apt** (Debian/Ubuntu): `apt list --upgradable` after a
//!     hands-off `apt-get -s upgrade`. Counts the
//!     `[upgradable from:...]` lines.
//!
//! If none of the above binaries exist, the widget hides itself
//! and logs once.
//!
//! ## Click action
//!
//! Spawns the user's terminal (auto-detected: kitty → alacritty
//! → foot → wezterm → konsole → gnome-terminal → xterm) with
//! the matching upgrade command (`sudo pacman -Syu`, `sudo dnf
//! upgrade`, `sudo apt upgrade`). Pacman/AUR users with a helper
//! (yay, paru, pikaur) are detected first and preferred — they
//! handle AUR + repo updates in one pass and don't need sudo.

use relm4::gtk::Orientation;
use relm4::gtk::prelude::{BoxExt, ButtonExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::Duration;
use tracing::warn;

/// 30 min refresh. Lighter cadence than other pills because
/// `checkupdates` does network I/O against the repo mirrors and
/// we don't want to hammer them.
const REFRESH_INTERVAL: Duration = Duration::from_secs(1800);
/// First probe lands shortly after launch — not instant, so we
/// don't fight idle-CPU-drain testing, but quickly enough that
/// the count is fresh by the time the user looks at the bar.
const STARTUP_DELAY: Duration = Duration::from_secs(10);

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
    /// completes; rendered as a spinner-like state by the view.
    count: Option<u32>,
    /// `Some` when probing failed; the pill renders as a small
    /// error icon with the message in the tooltip.
    error: Option<String>,
}

pub(crate) struct SystemUpdateModel {
    state: UpdateState,
    backend: Option<Backend>,
    _orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum SystemUpdateInput {
    Clicked,
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
}

#[relm4::component(pub)]
impl Component for SystemUpdateModel {
    type CommandOutput = SystemUpdateCommandOutput;
    type Input = SystemUpdateInput;
    type Output = SystemUpdateOutput;
    type Init = SystemUpdateInit;

    view! {
        #[root]
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
            // Live without a `null state` icon — less noise on
            // distros where this widget can't function.
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
        sender.command(|out, shutdown| async move {
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            let mut first = true;
            // Cache the discovered backend so we don't re-probe
            // `which` on every tick.
            let mut backend: Option<Backend> = None;
            loop {
                let delay = if first { STARTUP_DELAY } else { REFRESH_INTERVAL };
                first = false;
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    _ = tokio::time::sleep(delay) => {}
                }
                if backend.is_none() {
                    backend = detect_backend().await;
                    if backend.is_none() {
                        // Log once so a missing pacman/dnf/apt
                        // traces to a recognisable place.
                        static LOGGED: std::sync::atomic::AtomicBool =
                            std::sync::atomic::AtomicBool::new(false);
                        if !LOGGED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                            warn!(
                                "system_update: no supported package manager on PATH (tried pacman, dnf, apt)"
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
        });

        let model = SystemUpdateModel {
            state: UpdateState::default(),
            backend: None,
            _orientation: params.orientation,
        };
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        message: Self::Input,
        _sender: ComponentSender<Self>,
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
        }
    }
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
    if let Some(err) = &state.error {
        return format!("Updates: {err}");
    }
    let backend_label = backend.map(backend_label).unwrap_or("none");
    match state.count {
        None => format!("Updates ({backend_label}): checking…"),
        Some(0) => format!("Updates ({backend_label}): system is up to date"),
        Some(1) => format!("Updates ({backend_label}): 1 pending"),
        Some(n) => format!("Updates ({backend_label}): {n} pending"),
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
        Backend::PacmanWithHelper(helper) => probe_pacman_helper(helper).await,
        Backend::PacmanRepoOnly => probe_pacman_plain().await,
        Backend::Dnf => probe_dnf().await,
        Backend::Apt => probe_apt().await,
    }
}

/// AUR helpers all support `-Qu` against their own merged repo +
/// AUR view. Output: one package per line, e.g.:
///   `linux 6.7.0-1 -> 6.7.1-1`
async fn probe_pacman_helper(helper: &str) -> UpdateState {
    match run_capture(helper, &["-Qu"]).await {
        Ok(out) => count_nonempty_lines(&out),
        Err(e) => err_state(e),
    }
}

/// Without an AUR helper, prefer `checkupdates` (in `pacman-
/// contrib`): it refreshes a fake DB under `/tmp` so the regular
/// `pacman -Qu` doesn't need sudo. If `checkupdates` is missing,
/// fall back to plain `pacman -Qu` which only sees what's already
/// cached after a previous `-Sy`.
async fn probe_pacman_plain() -> UpdateState {
    if which("checkupdates").await {
        match run_capture("checkupdates", &[]).await {
            Ok(out) => return count_nonempty_lines(&out),
            // exit 2 from checkupdates = no updates; treat as 0.
            Err(e) if e.contains("exit 2") => return ok_state(0),
            Err(e) => return err_state(e),
        }
    }
    match run_capture("pacman", &["-Qu"]).await {
        Ok(out) => count_nonempty_lines(&out),
        Err(e) => err_state(e),
    }
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
                // First column of each non-blank, non-header line is
                // a package name. Header is empty (just blank lines).
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
    match run_capture("apt", &["list", "--upgradable"]).await {
        Ok(out) => {
            let n = out
                .lines()
                .filter(|l| l.contains("[upgradable from:"))
                .count() as u32;
            ok_state(n)
        }
        Err(e) => err_state(e),
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
    UpdateState {
        count: None,
        error: Some(msg.into()),
    }
}

async fn run_capture(cmd: &str, args: &[&str]) -> Result<String, String> {
    let res = tokio::process::Command::new(cmd)
        .args(args)
        .stderr(std::process::Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("{cmd} spawn: {e}"))?;
    if res.status.success() {
        return Ok(String::from_utf8_lossy(&res.stdout).into_owned());
    }
    let code = res.status.code().unwrap_or(-1);
    let stderr = String::from_utf8_lossy(&res.stderr).trim().to_string();
    if stderr.is_empty() {
        Err(format!("{cmd}: exit {code}"))
    } else {
        Err(format!("{cmd} exit {code}: {stderr}"))
    }
}

// ── Click action: open a terminal running the upgrade ──────────

async fn launch_terminal_upgrade(backend: Backend) {
    let (program, needs_sudo) = upgrade_command(backend);
    let Some(term) = detect_terminal().await else {
        warn!("system_update: no terminal emulator on PATH; click ignored");
        return;
    };
    // Many terminals accept `--` to delimit the inner command. We
    // prefer `-e <bin> <args...>` since it's the most portable.
    let inner = if needs_sudo {
        format!("sudo {program}")
    } else {
        program.to_string()
    };
    // Pass the upgrade as a single shell command so the user can
    // see output + react before the shell exits. `;\\ exec $SHELL`
    // would also work but pulls in a login shell config; sleep
    // gives them a beat to read the final summary.
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
    // (command-line string, needs sudo)
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
    // `kitty` first: that's what the user runs (per setup
    // notes). Standard fallbacks follow.
    for term in ["kitty", "alacritty", "foot", "wezterm", "konsole", "gnome-terminal", "xterm"] {
        if which(term).await {
            return Some(term);
        }
    }
    None
}
