use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::time::{Duration, Instant};
use std::{
    error::Error,
    path::{Path, PathBuf},
};

use mlogind_proto::{Conn, FdTransport};

use crossterm::{
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use log::{error, info, warn};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

mod auth;
mod chvt;
mod cli;
mod config;
mod console_palette;
mod info_caching;
mod post_login;
mod runner;
mod theme_sync;
mod ui;
mod vt_blank;

use config::Config;

use crate::cli::{Cli, Commands};
use crate::runner::Host;

pub(crate) const DEFAULT_VARIABLES_PATH: &str = "/etc/mlogind/variables.toml";
const DEFAULT_CONFIG_PATH: &str = "/etc/mlogind/config.toml";
const PREVIEW_LOG_PATH: &str = "mlogind.log";

/// `mlogind sync-theme`: refresh everything the pre-login greeters render from
/// the user's desktop — the TUI's matugen palette, and mgreet's matugen CSS plus
/// a blurred copy of the wallpaper. Run privileged; under sudo the *invoking*
/// user is resolved via `SUDO_USER`.
///
/// The reading is done by a forked child running as that user, never as root:
/// see [`crate::theme_sync`].
fn sync_theme() -> Result<(), Box<dyn Error>> {
    let username = match std::env::var("SUDO_USER") {
        Ok(name) => name,
        // Not under sudo. Whoever we are is whose desktop we sync — which for a
        // bare `mlogind sync-theme` as root means root's, and that is honest.
        Err(_) => uzers::get_current_username()
            .and_then(|name| name.into_string().ok())
            .ok_or("cannot tell whose theme to sync; run it as `sudo mlogind sync-theme`")?,
    };

    let user = auth::lookup(&username)?;
    let written = theme_sync::sync(&user)
        .map_err(|err| format!("{err} — run privileged, e.g. `sudo mlogind sync-theme`"))?;

    if written.is_empty() {
        return Err(format!(
            "nothing to sync from {username}'s desktop — apply a margo matugen theme first"
        )
        .into());
    }
    for path in written {
        println!("mlogind: wrote {}", path.display());
    }
    Ok(())
}

fn merge_in_configuration(config: &mut Config, cli: &Cli) {
    let load_variables_path = cli
        .variables
        .as_deref()
        .unwrap_or_else(|| Path::new(DEFAULT_VARIABLES_PATH));

    if let Some(initial_path) = &cli.initial_path {
        config.initial_path = initial_path.clone();
    }

    let variables = match config::Variables::from_file(load_variables_path) {
        Ok(variables) => {
            info!(
                "Successfully loaded variables file from '{}'",
                load_variables_path.display()
            );

            Some(variables)
        }
        Err(err) => {
            // If we have given it a specific config path, it should crash if this file cannot be
            // loaded. If it is the default config location just put a warning in the logs.
            if let Some(variables_path) = cli.variables.as_ref() {
                eprintln!(
                    "The variables file '{}' cannot be loaded.\nReason: {}",
                    variables_path.display(),
                    err
                );
                std::process::exit(1);
            } else {
                info!(
                    "No variables file loaded from the default location ({}). Reason: {}",
                    DEFAULT_CONFIG_PATH, err
                );
            }

            // Never fall through with no palette: substitute margo's baked
            // Dracula variables so a stale/absent `/etc/mlogind/variables.toml`
            // can't leave the greeter on ratatui's bare defaults.
            Some(config::Variables::baked_default())
        }
    };

    let load_config_path = cli
        .config
        .as_deref()
        .unwrap_or_else(|| Path::new(DEFAULT_CONFIG_PATH));

    match config::PartialConfig::from_file(load_config_path, variables.as_ref()) {
        Ok(partial_config) => {
            info!(
                "Successfully loaded configuration file from '{}'",
                load_config_path.display()
            );
            config.merge_in_partial(partial_config)
        }
        Err(err) => {
            // If we have given it a specific config path, it should crash if this file cannot be
            // loaded. If it is the default config location just put a warning in the logs.
            if let Some(config_path) = cli.config.as_ref() {
                eprintln!(
                    "The config file '{}' cannot be loaded.\nReason: {}",
                    config_path.display(),
                    err
                );
                std::process::exit(1);
            } else {
                warn!(
                    "No configuration file loaded from the expected location ({}). Reason: {}",
                    DEFAULT_CONFIG_PATH, err
                );
            }
        }
    }

    if let Some(xsessions) = cli.xsessions.as_ref() {
        config.x11.xsessions_path = xsessions.display().to_string();
    }

    if let Some(wlsessions) = cli.wlsessions.as_ref() {
        config.wayland.wayland_sessions_path = wlsessions.display().to_string();
    }
}

pub fn initialize_panic_handler() {
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        crossterm::execute!(std::io::stderr(), crossterm::terminal::LeaveAlternateScreen).unwrap();
        crossterm::terminal::disable_raw_mode().unwrap();

        original_hook(panic_info);
    }));
}

/// Point the logger at `log_path`, or run without a log.
///
/// It used to `exit(1)` when the file could not be opened. The greeter is
/// unprivileged now and cannot write `/var/log`, so that was a lockout with
/// extra steps — and it was always the wrong trade: a login manager that refuses
/// to start because a log is unwritable is worse than one with no log.
fn setup_logger(log_path: &str) {
    let mut builder = env_logger::builder();
    builder
        .filter_level(log::LevelFilter::Info)
        .format_timestamp_secs();

    match File::create(log_path) {
        Ok(file) => {
            builder.target(env_logger::Target::Pipe(Box::new(file)));
        }
        Err(err) => {
            eprintln!("mlogind: cannot open log file '{log_path}' ({err}); logging to stderr");
        }
    }
    builder.init();
}

/// Where a greeter hosted by the session runner should log.
///
/// `config.client_log_path` is `/var/log/…`, which the unprivileged greeter user
/// cannot write. `pam_systemd` gave the greeter session its own runtime dir; use
/// that. Falls back to the configured path when there is none (a root greeter),
/// and [`setup_logger`] tolerates it being unwritable either way.
fn greeter_log_path(config: &Config) -> String {
    match std::env::var("XDG_RUNTIME_DIR") {
        Ok(dir) if !dir.is_empty() => format!("{dir}/mlogind-greeter.log"),
        _ => config.client_log_path.clone(),
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse().unwrap_or_else(|err| {
        eprintln!("{err}\n");
        cli::usage();
        std::process::exit(2);
    });

    let mut config = Config::default();
    merge_in_configuration(&mut config, &cli);

    if let Some(cmd) = cli.command {
        match cmd {
            Commands::Envs => {
                let envs = post_login::get_envs(&config);

                for (env_name, _) in envs.into_iter() {
                    println!("{env_name}");
                }
            }
            Commands::Cache => {
                let cached_info = info_caching::get_cached_information(&config);

                let environment = cached_info.environment().unwrap_or("No cached value");
                let username = cached_info.username().unwrap_or("No cached value");

                println!(
                    "Information currently cached within '{}'\n",
                    config.cache_path
                );

                println!("environment: '{environment}'");
                println!("username: '{username}'");
            }
            Commands::Help => {
                cli::usage();
            }
            Commands::ShowConfig => {
                println!("{}", toml::to_string(&config)?);
            }
            Commands::Version => {
                println!("{}", env!("CARGO_PKG_VERSION"));
            }
            Commands::SyncTheme => {
                sync_theme()?;
            }
        }

        return Ok(());
    }

    // Setup the logger. A `--greet` UI dry-run (no orchestrator hand-off path)
    // logs like `--preview`, into the CWD, so a non-root test can actually write
    // it. The real greeter (spawned by the orchestrator) logs to its own client
    // log so it doesn't clobber the orchestrator's main log.
    if !cli.no_log {
        let greet_is_hosted = cli.greet && std::env::var_os("MLOGIND_SOCK_FD").is_some();
        let hosted_log = greet_is_hosted.then(|| greeter_log_path(&config));
        let log_path: &str = if cli.preview || (cli.greet && !greet_is_hosted) {
            PREVIEW_LOG_PATH
        } else if let Some(path) = hosted_log.as_deref() {
            path
        } else {
            &config.main_log_path
        };
        setup_logger(log_path);
        info!("Main mlogind logger is running");
    } else {
        config.do_log = false;
    }

    // GREETER MODE (`--greet`): we run inside the cage+foot host spawned by the
    // root orchestrator. Skip the bare-VT dance (chvt / XDG-session refusal /
    // palette reprogram) — we're in a Wayland terminal — but still do real PAM
    // auth and hand the validated login back to the orchestrator.
    if cli.greet {
        return run_greeter(config);
    }

    if !cli.preview {
        if std::env::var("XDG_SESSION_TYPE").is_ok() {
            eprintln!(
                "mlogind cannot be ran without `--preview` within an existing session. Namely, `XDG_SESSION_TYPE` is set."
            );
            error!(
                "mlogind cannot be started when within an existing session. Namely, `XDG_SESSION_TYPE` is set."
            );
            std::process::exit(1);
        }

        let uid = uzers::get_current_uid();
        if uzers::get_current_uid() != 0 {
            eprintln!("mlogind needs to be ran as root. Found user id '{uid}'");
            error!("mlogind not ran as root. Found user id '{uid}'");
            std::process::exit(1);
        }

        if let Some(tty) = cli.tty {
            info!("Overwritten the tty to '{tty}' with the --tty flag");
            config.tty = tty;
        }

        // Switch to the proper tty
        info!("Switching to tty {}", config.tty);

        unsafe { chvt::chvt(config.tty.into()) }.unwrap_or_else(|err| {
            error!("Failed to switch tty {}. Reason: {err}", config.tty);
        });
    }

    initialize_panic_handler();

    // Decide how colours render: preview → truecolor in the emulator; real VT
    // → reprogram the console palette so it matches preview (see console_palette).
    console_palette::init(cli.preview);

    // ORCHESTRATOR MODE: host the greeter under a compositor so every monitor
    // renders at its own native KMS mode (dynamic, EDID-derived), then launch
    // the chosen session on the bare VT. Layered fallback driven by
    // `[display] host`:
    //   gui  → margo + mgreet (GTK, a login card on EVERY output) → cage → TTY
    //   cage → cage + foot + the TUI greeter (single output)      → TTY
    //   *    → the in-process TTY greeter below
    // Each host returns Err only when it cannot run at all, so a broken host
    // degrades to the next one and never locks the user out.
    if !cli.preview {
        let host = config.display.host.to_ascii_lowercase();
        // A graphical host owns the DRM master, but not until its compositor's
        // first modeset (~1.5 s). Hold the VT in graphics mode from now so the
        // kernel text console (a bare blinking cursor) never flashes in that gap
        // — and stays black across greeter↔session handovers too. The guard
        // restores text on drop; the graphical `Ok` arms exit mlogind (so drop
        // is right), and the fall-through drops it before the TTY greeter, whose
        // prompt would be invisible on a blanked console.
        let graphical = host == "gui" || host == "cage";
        let vt = if graphical {
            vt_blank::graphics(config.tty)
        } else {
            None
        };

        if host == "gui" {
            match run_hosted(&config, Host::Gui) {
                Ok(()) => {
                    info!("mlogind is booting down");
                    return Ok(());
                }
                Err(e) => warn!("gui host unavailable ({e}); falling back to the cage host"),
            }
        }
        if graphical {
            match run_hosted(&config, Host::Cage) {
                Ok(()) => {
                    info!("mlogind is booting down");
                    return Ok(());
                }
                Err(e) => warn!("cage host unavailable ({e}); falling back to the TTY greeter"),
            }
        }

        // Falling through to the TTY greeter: hand the console back to text
        // before it draws (no-op when we never blanked).
        drop(vt);
        run_tty_host(&config)?;
        info!("mlogind is booting down");
        return Ok(());
    }

    // `--preview`: the TUI in a terminal emulator. No fork, no PAM, no session.
    let mut terminal = tui_enable()?;
    let _ = ui::LoginForm::new(config, true).run(&mut terminal, None)?;
    tui_disable(terminal)?;

    info!("mlogind is booting down");

    Ok(())
}

pub fn tui_enable() -> io::Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;

    info!("UI booted up");

    Ok(terminal)
}

pub fn tui_disable(mut terminal: Terminal<CrosstermBackend<io::Stdout>>) -> io::Result<()> {
    // Hand the console back its default palette (no-op in preview).
    console_palette::reset();
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    info!("Reset terminal environment");

    Ok(())
}

/// Greeter mode (`mlogind --greet`), run inside the cage/foot host.
///
/// Renders the normal login UI in foot (truecolor, native resolution) and
/// speaks the runner's protocol over `$MLOGIND_SOCK_FD`. It runs no PAM of its
/// own — that is the whole point of A1 — so a fingerprint or OTP module prompts
/// exactly once, here, and the runner answers PAM with what we type.
///
/// With no socket it is a preview-style UI dry-run, so `mlogind --greet` can
/// still be eyeballed in an ordinary terminal.
fn run_greeter(config: Config) -> Result<(), Box<dyn Error>> {
    initialize_panic_handler();
    // We're in foot (a truecolor emulator), not the bare VT — take the
    // pass-through palette path, exactly like `--preview`.
    console_palette::init(true);

    let sock = greeter_socket();
    let mut terminal = tui_enable()?;
    let form = ui::LoginForm::new(config, sock.is_none());
    let outcome = match sock.as_ref() {
        // SAFETY: `fd` is the inherited socket, owned by `sock`, which outlives
        // the `Conn` — `run` joins its event thread before returning.
        Some(fd) => {
            let conn = Conn::new(unsafe { FdTransport::new(fd.as_raw_fd()) });
            form.run(&mut terminal, Some(conn))
        }
        None => form.run(&mut terminal, None),
    };
    tui_disable(terminal)?;
    let outcome = outcome?;

    info!("greeter exiting ({outcome:?})");
    Ok(())
}

/// Adopt the socket the runner left us on `$MLOGIND_SOCK_FD`.
///
/// atrium's `CREDENTIALS_FD` idiom: the fd rides across `exec` (the runner
/// cleared `FD_CLOEXEC`) and the number arrives in the environment. A missing
/// or unparsable value means nobody is orchestrating us — a UI dry-run.
fn greeter_socket() -> Option<OwnedFd> {
    let raw: RawFd = std::env::var("MLOGIND_SOCK_FD").ok()?.parse().ok()?;
    // SAFETY: the runner passed us this descriptor and closed its own copy, so
    // we are its sole owner.
    Some(unsafe { OwnedFd::from_raw_fd(raw) })
}

/// Resolve an executable by scanning `PATH`, falling back to `/usr/bin`.
pub(crate) fn which(cmd: &str) -> Option<PathBuf> {
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join(cmd);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    let fallback = PathBuf::from("/usr/bin").join(cmd);
    fallback.is_file().then_some(fallback)
}

/// Read the system console keyboard config (`/etc/vconsole.conf`, written by
/// localectl) and translate its already-xkb-shaped fields into the
/// `XKB_DEFAULT_*` env vars cage builds its keymap from — so the greeter uses
/// the machine's layout (e.g. Turkish-F: `XKBLAYOUT=tr` + `XKBVARIANT=f`)
/// instead of cage's `us` default. Empty if the file is missing.
pub(crate) fn vconsole_xkb_env() -> Vec<(&'static str, String)> {
    let mut env = Vec::new();
    let Ok(text) = std::fs::read_to_string("/etc/vconsole.conf") else {
        return env;
    };
    for line in text.lines() {
        let Some((key, val)) = line.trim().split_once('=') else {
            continue;
        };
        let val = val.trim().trim_matches('"').to_string();
        // XKBOPTIONS is often ",caps:…" (leading empty option) — keep it; only
        // skip a truly empty value so we don't blank cage's own default.
        if val.is_empty() {
            continue;
        }
        match key.trim() {
            "XKBLAYOUT" => env.push(("XKB_DEFAULT_LAYOUT", val)),
            "XKBVARIANT" => env.push(("XKB_DEFAULT_VARIANT", val)),
            "XKBOPTIONS" => env.push(("XKB_DEFAULT_OPTIONS", val)),
            "XKBMODEL" => env.push(("XKB_DEFAULT_MODEL", val)),
            _ => {}
        }
    }
    env
}

/// Build the throwaway margo config text the GUI greeter runs under: the machine
/// keyboard layout (translated from `/etc/vconsole.conf` so Turkish-F etc.
/// carries into the login prompt), and — when the baked backdrop exists — a
/// `wallpaper` line pointing margo at it, so the compositor's first frame is the
/// blurred wallpaper `mgreet` paints rather than the packaged default. Crucially
/// NO shell autostart: the greeter compositor must never launch the user's
/// desktop. Pure, so the wallpaper-present/absent branches are testable without
/// touching disk.
fn greeter_conf_text(xkb: &[(&str, String)], backdrop: Option<&Path>) -> String {
    let mut conf = String::from(
        "# Auto-generated by mlogind for the GUI greeter — DO NOT EDIT.\n\
         # Rewritten on every login. Minimal margo config: keyboard layout (+\n\
         # the greeter backdrop), no autostart (never launch the user's desktop).\n",
    );
    for (env_key, val) in xkb {
        let conf_key = match *env_key {
            "XKB_DEFAULT_LAYOUT" => "xkb_rules_layout",
            "XKB_DEFAULT_VARIANT" => "xkb_rules_variant",
            "XKB_DEFAULT_OPTIONS" => "xkb_rules_options",
            "XKB_DEFAULT_MODEL" => "xkb_rules_model",
            _ => continue,
        };
        conf.push_str(conf_key);
        conf.push_str(" = ");
        conf.push_str(val);
        conf.push('\n');
    }
    // The baked backdrop is pixel-identical to mgreet's, so pointing margo at it
    // makes the login card fade in over an unchanging wallpaper. Absent only on a
    // machine's first-ever boot (no session has baked one yet); margo then falls
    // back to its packaged default, and every later greeter carries the line.
    if let Some(path) = backdrop {
        conf.push_str("wallpaper = ");
        conf.push_str(&path.to_string_lossy());
        conf.push('\n');
    }
    conf
}

/// Rewritten on every host start. Because the file already exists when margo
/// loads it, margo's first-run bootstrap leaves it untouched (it only writes a
/// full default config for a *missing* path).
pub(crate) fn write_greeter_conf(path: &Path) -> io::Result<()> {
    let backdrop = crate::theme_sync::background_path();
    let backdrop = backdrop.is_file().then_some(backdrop);
    let conf = greeter_conf_text(&vconsole_xkb_env(), backdrop.as_deref());
    std::fs::write(path, conf)?;
    // margo reads this as the unprivileged greeter user. It carries a keyboard
    // layout and the backdrop path, nothing else.
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644))
}

/// How many consecutive fast crashes of a session runner mean the host itself
/// is broken. Introduced by A1: the runner is now a fork, so a runner that dies
/// instantly would otherwise spin the daemon in a tight fork loop at boot.
/// A real backoff (timerfd, per-seat, atrium's `daemon/core/main.c`) is phase B.
const RUNNER_CRASH_LIMIT: u32 = 5;
const RUNNER_CRASH_WINDOW: Duration = Duration::from_secs(2);
/// Linear backoff per consecutive fast crash. Only the crash path waits; a
/// normal session end re-greets with no delay. Keeps a runner that dies
/// instantly from hammering DRM/logind faster than a transient failure clears.
const RUNNER_CRASH_BACKOFF: Duration = Duration::from_millis(250);

/// Orchestrate a hosted greeter: fork a session runner, let it own the login
/// from the first prompt to the last `pam_close_session`, and fork a fresh one
/// when the session ends.
///
/// The daemon deliberately does nothing else. It never calls PAM, so nothing it
/// does can pollute a session's cgroup or `loginuid`, and no PAM handle can
/// survive from one login to the next. The runner spawns the greeter itself, so
/// the session compositor cannot open DRM before the greeter compositor has
/// released it.
///
/// Returns `Err` only when the host cannot run at all, so `main` falls down the
/// `gui → cage → tty` ladder and a broken host never locks the user out.
fn run_hosted(config: &Config, host: Host) -> Result<(), Box<dyn Error>> {
    host.preflight()?;

    // No `ensure_seatd()` here any more. The greeter now runs inside its own
    // logind session (see runner::greeter_session), so libseat finds its logind
    // backend by itself — and `seatd` existed only because a session-less root
    // process could not.
    let mut fast_crashes = 0u32;
    loop {
        let (runner_fd, greeter_fd) = runner::socketpair()?;
        let started = Instant::now();

        // SAFETY: mlogind is single-threaded, so the child inherits no locks it
        // could deadlock on before `exec`.
        let pid = unsafe { libc::fork() };
        if pid < 0 {
            return Err(io::Error::last_os_error().into());
        }
        if pid == 0 {
            runner::run(config, host, runner_fd, greeter_fd);
        }
        // The runner owns both ends now: one to speak on, one to hand its greeter.
        drop(runner_fd);
        drop(greeter_fd);

        match wait_for(pid) {
            runner::EXIT_SESSION_ENDED => {
                info!("orchestrator: session ended; re-greeting");
                fast_crashes = 0;
            }
            runner::EXIT_NO_LOGIN => {
                info!("orchestrator: greeter produced no login; exiting host");
                return Ok(());
            }
            runner::EXIT_HOST_UNAVAILABLE => {
                return Err(format!("the {host:?} greeter host could not run").into());
            }
            code => {
                error!("orchestrator: session runner exited with {code}");
                if started.elapsed() < RUNNER_CRASH_WINDOW {
                    fast_crashes += 1;
                    if fast_crashes >= RUNNER_CRASH_LIMIT {
                        return Err(format!(
                            "the {host:?} session runner crashed {fast_crashes} times in a row"
                        )
                        .into());
                    }
                    // Space the retries out so an instantly-dying runner doesn't
                    // spin the fork loop; the happy path never reaches here.
                    let backoff = RUNNER_CRASH_BACKOFF * fast_crashes;
                    warn!("orchestrator: backing off {backoff:?} before the next runner");
                    std::thread::sleep(backoff);
                } else {
                    fast_crashes = 0;
                }
            }
        }
    }
}

/// The last rung of the ladder: the TUI form, drawn by the daemon itself on the
/// bare VT.
///
/// Here the greeter is the *parent* of the runner rather than its child, but
/// the protocol is symmetric so nothing else changes — and, crucially, PAM
/// still runs in exactly one place. The daemon closes its end of the socket
/// after leaving the alternate screen; that EOF is what tells the runner the VT
/// is free and it may open DRM.
fn run_tty_host(config: &Config) -> Result<(), Box<dyn Error>> {
    loop {
        let (runner_fd, greeter_fd) = runner::socketpair()?;

        // SAFETY: single-threaded; see `run_hosted`.
        let pid = unsafe { libc::fork() };
        if pid < 0 {
            return Err(io::Error::last_os_error().into());
        }
        if pid == 0 {
            runner::run(config, Host::Tty, runner_fd, greeter_fd);
        }
        drop(runner_fd);

        // SAFETY: `greeter_fd` owns the descriptor and outlives `run`, which
        // joins its event thread before returning.
        let conn = Conn::new(unsafe { FdTransport::new(greeter_fd.as_raw_fd()) });
        let mut terminal = tui_enable()?;
        let outcome = ui::LoginForm::new(config.clone(), false).run(&mut terminal, Some(conn));
        tui_disable(terminal)?;
        let outcome = outcome?;

        // Off the screen. The runner has been waiting for exactly this.
        drop(greeter_fd);

        let code = wait_for(pid);
        match outcome {
            ui::Outcome::Quit => {
                info!("orchestrator: greeter produced no login; exiting host");
                return Ok(());
            }
            ui::Outcome::SessionStarting if code == runner::EXIT_SESSION_ENDED => {
                info!("orchestrator: session ended; re-greeting");
            }
            ui::Outcome::SessionStarting => {
                error!("orchestrator: session runner exited with {code}");
            }
            ui::Outcome::RunnerGone => {
                error!("orchestrator: session runner vanished (exit {code}); re-greeting");
            }
        }
    }
}

/// Reap `pid` and reduce its wait status to an exit code. A runner killed by a
/// signal reports `128 + signo`, the shell convention, so it can never collide
/// with one of the runner's own codes.
pub(crate) fn wait_for(pid: libc::pid_t) -> i32 {
    let mut status: libc::c_int = 0;
    // SAFETY: `status` is a valid out-pointer; `pid` is our direct child.
    while unsafe { libc::waitpid(pid, &mut status, 0) } < 0 {
        let err = io::Error::last_os_error();
        if err.kind() == io::ErrorKind::Interrupted {
            continue;
        }
        error!("orchestrator: waitpid({pid}) failed: {err}");
        return runner::EXIT_SESSION_FAILED;
    }
    if libc::WIFEXITED(status) {
        libc::WEXITSTATUS(status)
    } else if libc::WIFSIGNALED(status) {
        128 + libc::WTERMSIG(status)
    } else {
        runner::EXIT_SESSION_FAILED
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_backdrop_line_is_emitted_only_when_the_baked_file_exists() {
        let xkb: Vec<(&str, String)> = vec![("XKB_DEFAULT_LAYOUT", "tr".to_string())];

        let with = greeter_conf_text(&xkb, Some(Path::new("/var/lib/mgreet/background.raw")));
        assert!(with.contains("wallpaper = /var/lib/mgreet/background.raw\n"));
        assert!(with.contains("xkb_rules_layout = tr\n"));

        let without = greeter_conf_text(&xkb, None);
        assert!(!without.contains("wallpaper"));
        assert!(without.contains("xkb_rules_layout = tr\n"));
    }

    #[test]
    fn the_greeter_config_never_carries_an_autostart() {
        // The one invariant the login gate cannot afford to lose: the greeter
        // compositor must never launch the user's desktop. Checked on directive
        // lines only — the header comment says "no autostart" by design, so a
        // plain substring match on the whole text would (and did) false-positive.
        let conf = greeter_conf_text(&[], Some(Path::new("/var/lib/mgreet/background.raw")));
        let has_autostart_directive = conf
            .lines()
            .filter(|line| !line.trim_start().starts_with('#'))
            .any(|line| line.contains("autostart"));
        assert!(!has_autostart_directive);
    }
}
