use std::fs::File;
use std::io;
use std::{
    error::Error,
    path::{Path, PathBuf},
};

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
mod ui;

use auth::try_validate;
use config::Config;
use post_login::PostLoginEnvironment;

use crate::{
    auth::utmpx::add_utmpx_entry,
    cli::{Cli, Commands},
};

use self::{
    auth::{open_session, AuthenticationError, ValidatedCredentials},
    post_login::env_variables::{
        remove_xdg, set_basic_variables, set_display, set_seat_vars, set_session_params,
        set_session_vars, set_xdg_common_paths,
    },
};

const DEFAULT_VARIABLES_PATH: &str = "/etc/mlogind/variables.toml";
const DEFAULT_CONFIG_PATH: &str = "/etc/mlogind/config.toml";
const PREVIEW_LOG_PATH: &str = "mlogind.log";

/// `mlogind sync-theme`: copy the active margo matugen palette
/// (`~/.config/margo/mlogind-variables.toml`, written by mshell-matugen on
/// every theme change) into `/etc/mlogind/variables.toml`, so the
/// pre-login greeter matches the user's wallpaper. Run privileged; under
/// sudo the *invoking* user is resolved via `SUDO_USER`.
fn sync_theme() -> Result<(), Box<dyn Error>> {
    use uzers::os::unix::UserExt;

    let home: PathBuf = match std::env::var_os("SUDO_USER") {
        Some(name) => uzers::get_user_by_name(&name)
            .map(|u| u.home_dir().to_path_buf())
            .ok_or_else(|| format!("unknown SUDO_USER '{}'", name.to_string_lossy()))?,
        None => std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or("neither SUDO_USER nor HOME is set; pass the palette explicitly")?,
    };

    let src = home.join(".config/margo/mlogind-variables.toml");
    let dst = Path::new(DEFAULT_VARIABLES_PATH);

    let body = std::fs::read_to_string(&src).map_err(|e| {
        format!(
            "cannot read {} ({e}) — apply a margo matugen theme first",
            src.display()
        )
    })?;

    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(dst, &body).map_err(|e| {
        format!(
            "cannot write {} ({e}) — run privileged, e.g. `sudo mlogind sync-theme`",
            dst.display()
        )
    })?;

    println!(
        "mlogind: synced palette {} → {}",
        src.display(),
        dst.display()
    );
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

fn setup_logger(log_path: &str) {
    let log_file = Box::new(File::create(log_path).unwrap_or_else(|_| {
        eprintln!("Failed to open log file: '{log_path}'");
        std::process::exit(1);
    }));

    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .target(env_logger::Target::Pipe(log_file))
        .format_timestamp_secs()
        .init();
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
        let greet_has_result = cli.greet && std::env::var_os("MLOGIND_RESULT_PATH").is_some();
        let log_path: &str = if cli.preview || (cli.greet && !greet_has_result) {
            PREVIEW_LOG_PATH
        } else if greet_has_result {
            &config.client_log_path
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

    // ORCHESTRATOR MODE: host the greeter in cage+foot so every monitor renders
    // at its own native KMS mode (dynamic, EDID-derived), then launch the chosen
    // session on the bare VT. Falls through to the in-process TTY greeter below
    // if `[display] host` is not "cage", or if the cage/foot host can't run.
    if !cli.preview && config.display.host.eq_ignore_ascii_case("cage") {
        match run_cage_host(&config) {
            Ok(()) => {
                info!("mlogind is booting down");
                return Ok(());
            }
            Err(e) => {
                warn!("cage host unavailable ({e}); falling back to the TTY greeter");
            }
        }
    }

    // Start application (classic in-process greeter + fallback).
    let mut terminal = tui_enable()?;
    let login_form = ui::LoginForm::new(config, cli.preview);
    login_form.run(&mut terminal)?;
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

struct Hooks<'a> {
    pre_validate: Option<&'a dyn Fn()>,
    pre_auth: Option<&'a dyn Fn()>,
    pre_environment: Option<&'a dyn Fn()>,
    pre_wait: Option<&'a dyn Fn()>,
    pre_return: Option<&'a dyn Fn()>,
}

pub enum StartSessionError {
    AuthenticationError(AuthenticationError),
    ForkFailed,
}

impl From<AuthenticationError> for StartSessionError {
    fn from(value: AuthenticationError) -> Self {
        Self::AuthenticationError(value)
    }
}

fn start_session(
    username: &str,
    password: &str,
    post_login_env: &PostLoginEnvironment,
    hooks: &Hooks<'_>,
    config: &Config,
) -> Result<(), StartSessionError> {
    info!(
        "Starting new session for '{}' in environment '{:?}'",
        username, post_login_env
    );

    if let Some(pre_validate_hook) = hooks.pre_validate {
        pre_validate_hook();
    }

    if let Some(pre_auth_hook) = hooks.pre_auth {
        pre_auth_hook();
    }

    // Validate credentials before opening a session.
    let creds = try_validate(username, password, &config.pam_service)?;

    if let Some(pre_environment_hook) = hooks.pre_environment {
        pre_environment_hook();
    }

    // Fork the session. The session is opened inside the child process after fork(), so that the
    // session lifetime is coupled to the child PID.  For systemd-logind, it sees the
    // session-leader PID gone and cleans up immediately.
    let child_pid = unsafe { libc::fork() };
    if child_pid == -1 {
        error!("fork() failed ({})", unsafe { *libc::__errno_location() });
        return Err(StartSessionError::ForkFailed);
    }

    if child_pid == 0 {
        session_child(creds, post_login_env, username, config);
    }

    // The creditionals (i.e. the PAM handle) should be forgotten. The child owns it.
    std::mem::forget(creds);

    if let Some(pre_wait_hook) = hooks.pre_wait {
        pre_wait_hook();
    }

    info!("Waiting for session child (pid {child_pid}) to exit");

    let mut status: libc::c_int = 0;
    unsafe { libc::waitpid(child_pid, &mut status, 0) };

    info!("Session child exited. Returning to mlogind...");

    if let Some(pre_return_hook) = hooks.pre_return {
        pre_return_hook();
    }

    Ok(())
}

/// Body of the forked child process.
///
/// Opens the PAM session (so logind registers this PID as the session leader),
/// spawns the compositor, waits for it to exit, then terminates.  The `-> !`
/// return type makes explicit that this function never returns to the caller.
fn session_child(
    creds: ValidatedCredentials<'_>,
    post_login_env: &PostLoginEnvironment,
    username: &str,
    config: &Config,
) -> ! {
    let tty = config.tty;
    let uid = creds.uid;
    let homedir = creds.home_dir.clone();
    let shell = creds.shell.clone();

    // Set the vars pam_systemd needs to register the session on the right
    // seat/VT before calling open_session.
    if matches!(post_login_env, PostLoginEnvironment::X { .. }) {
        set_display(&config.x11.x11_display);
    }
    remove_xdg();
    set_session_params(post_login_env);
    set_seat_vars(tty);

    let auth_session = match open_session(creds) {
        Ok(s) => s,
        Err(err) => {
            error!("Child: failed to open PAM session: {err}");
            std::process::exit(1);
        }
    };

    // Set the remaining variables after pam_open_session has run — pam_systemd
    // populates XDG_RUNTIME_DIR and XDG_SESSION_ID, which set_session_vars /
    // set_xdg_common_paths adopt via set_or_own.
    set_session_vars(uid);
    set_basic_variables(username, &homedir, &shell, &config.initial_path);
    set_xdg_common_paths(&homedir);

    let spawned_environment = match post_login_env.spawn(&auth_session, config) {
        Ok(env) => env,
        Err(err) => {
            error!("Child: failed to start environment: {err}");
            std::process::exit(1);
        }
    };

    let pid = spawned_environment.pid();
    let utmpx_session = add_utmpx_entry(username, tty, pid);

    info!("Child: waiting for environment to terminate");
    spawned_environment.wait();
    info!("Child: environment terminated");

    drop(utmpx_session);
    drop(auth_session);
    std::process::exit(0);
}

/// Greeter entry point (`mlogind --greet`), run inside the cage/foot host.
/// Renders the normal login UI in foot (truecolor, native resolution) and — on
/// a validated login — writes the credentials to `$MLOGIND_RESULT_PATH` for the
/// orchestrator, then exits. With no result path it is a preview-style UI
/// dry-run (so `mlogind --greet` can be eyeballed in an ordinary terminal).
fn run_greeter(config: Config) -> Result<(), Box<dyn Error>> {
    let result_path = std::env::var_os("MLOGIND_RESULT_PATH").map(PathBuf::from);

    initialize_panic_handler();
    // We're in foot (truecolor emulator), not the bare VT — take the
    // pass-through palette path, exactly like `--preview`.
    console_palette::init(true);

    let mut terminal = tui_enable()?;
    // With no hand-off path this is a UI dry-run (`mlogind --greet` in a plain
    // terminal): run it as a preview so Esc quits and Enter merely animates —
    // there is no orchestrator to receive a login.
    let login_form = if result_path.is_some() {
        ui::LoginForm::new(config, false).into_greeter(result_path)
    } else {
        ui::LoginForm::new(config, true)
    };
    login_form.run(&mut terminal)?;
    tui_disable(terminal)?;

    info!("greeter exiting");
    Ok(())
}

/// The validated login handed back by the greeter.
struct GreetResult {
    username: String,
    env_name: String,
    password: zeroize::Zeroizing<String>,
}

/// Read the credential hand-off file, then overwrite + remove it so the
/// password never lingers in the tmpfs. Returns `None` if the greeter produced
/// no login (quit, crash, or reboot/poweroff handled inside the greeter).
fn read_and_shred_greet_result(path: &Path) -> Option<GreetResult> {
    let raw = match std::fs::read_to_string(path) {
        Ok(s) => zeroize::Zeroizing::new(s),
        Err(_) => return None,
    };

    // Best-effort shred: overwrite with zeros of the same length, then unlink.
    if let Ok(meta) = std::fs::metadata(path) {
        let _ = std::fs::write(path, vec![0u8; meta.len() as usize]);
    }
    let _ = std::fs::remove_file(path);

    // `LOGIN\n<user>\n<session>\n<password>` — password is the final field.
    let mut lines = raw.splitn(4, '\n');
    match (lines.next(), lines.next(), lines.next(), lines.next()) {
        (Some("LOGIN"), Some(user), Some(env), Some(pass)) => Some(GreetResult {
            username: user.to_string(),
            env_name: env.to_string(),
            password: zeroize::Zeroizing::new(pass.to_string()),
        }),
        _ => None,
    }
}

/// Resolve an executable by scanning `PATH`, falling back to `/usr/bin`.
fn which(cmd: &str) -> Option<PathBuf> {
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

/// Ensure a seat provider exists for cage. libseat's `builtin` backend is
/// compiled out on most distros (incl. Arch), and the root orchestrator has no
/// logind session — so cage's only viable backend is seatd. If its socket isn't
/// already present (an enabled `seatd.service`), start seatd ourselves and wait
/// briefly for the socket. The returned child is kept alive for the host loop;
/// `None` means seatd was already running or its binary is missing (in which
/// case cage will fail and we fall back to the TTY greeter).
fn ensure_seatd() -> Option<std::process::Child> {
    let sock = Path::new("/run/seatd.sock");
    if sock.exists() {
        return None;
    }
    let seatd = which("seatd")?;
    info!("orchestrator: starting seatd (no logind session; libseat builtin absent)");
    let child = std::process::Command::new(seatd).spawn().ok()?;
    for _ in 0..40 {
        if sock.exists() {
            info!("orchestrator: seatd socket is up");
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    Some(child)
}

/// Orchestrator: host the greeter inside `cage -s -- foot <self> --greet` so
/// every connected monitor renders at its native KMS mode, read the validated
/// login it hands back, and launch the session on the bare VT. Loops
/// (re-greeting after logout) until the greeter produces no login. Returns
/// `Err` — so `main` falls back to the in-process TTY greeter — when the
/// cage/foot host cannot run at all (missing binaries or a failed cage init),
/// so a broken host never locks the user out.
fn run_cage_host(config: &Config) -> Result<(), Box<dyn Error>> {
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;

    let cage = which("cage").ok_or("`cage` not found in PATH")?;
    let foot = which("foot").ok_or("`foot` not found in PATH")?;
    let self_exe = std::env::current_exe()?;

    // Root has no XDG_RUNTIME_DIR; give cage a private tmpfs dir (0700) that
    // also holds the one-shot credential hand-off file.
    let runtime_dir = PathBuf::from("/run/mlogind");
    std::fs::create_dir_all(&runtime_dir)?;
    std::fs::set_permissions(&runtime_dir, std::fs::Permissions::from_mode(0o700))?;
    let result_path = runtime_dir.join("result");
    let cage_log = runtime_dir.join("cage.log");

    // cage needs a seat; make sure seatd is up (see ensure_seatd). Kept alive
    // for the whole host loop.
    let _seatd = ensure_seatd();

    // The orchestrator runs the full PAM conversation itself, so no UI hooks.
    let hooks = Hooks {
        pre_validate: None,
        pre_auth: None,
        pre_environment: None,
        pre_wait: None,
        pre_return: None,
    };

    loop {
        // Never let a stale hand-off file leak a previous password.
        let _ = std::fs::remove_file(&result_path);

        info!("orchestrator: launching cage+foot greeter");
        let mut cmd = Command::new(&cage);
        cmd.arg("-s") // allow VT switching → escape hatch stays open
            .arg("--")
            .arg(&foot)
            .arg(&self_exe)
            .arg("--greet")
            .env("XDG_RUNTIME_DIR", &runtime_dir)
            .env("MLOGIND_RESULT_PATH", &result_path)
            // libseat: logind (no session) → fails; force seatd, the only
            // backend available to a session-less root process here.
            .env("LIBSEAT_BACKEND", "seatd");
        // Capture cage's own stdout/stderr — it inherits the greeter's VT
        // otherwise, where a later TUI redraw wipes any error it printed.
        if let Ok(out) = std::fs::File::create(&cage_log) {
            if let Ok(err) = out.try_clone() {
                cmd.stdout(out).stderr(err);
            }
        }
        let status = cmd
            .status()
            .map_err(|e| format!("failed to spawn cage: {e}"))?;

        if !status.success() {
            // Surface cage's own diagnostics into our log before falling back.
            if let Ok(text) = std::fs::read_to_string(&cage_log) {
                let tail: Vec<&str> = text.lines().rev().take(12).collect();
                for line in tail.into_iter().rev() {
                    error!("cage: {line}");
                }
            }
            return Err(format!("cage exited abnormally ({status})").into());
        }

        match read_and_shred_greet_result(&result_path) {
            Some(result) => {
                // Re-resolve the environment by name (the greeter and the
                // orchestrator both derive it from get_envs, same order).
                let post_login_env = post_login::get_envs(config)
                    .into_iter()
                    .find(|(name, _)| *name == result.env_name)
                    .map(|(_, content)| content);
                let Some(post_login_env) = post_login_env else {
                    error!(
                        "orchestrator: greeter chose unknown session '{}'",
                        result.env_name
                    );
                    continue;
                };

                info!("orchestrator: launching session for '{}'", result.username);
                match start_session(
                    &result.username,
                    &result.password,
                    &post_login_env,
                    &hooks,
                    config,
                ) {
                    Ok(()) => info!("orchestrator: session ended; re-greeting"),
                    Err(StartSessionError::AuthenticationError(err)) => {
                        error!("orchestrator: authentication failed: {err}");
                    }
                    Err(StartSessionError::ForkFailed) => {
                        error!("orchestrator: failed to fork the session");
                    }
                }
                // `result` (and its zeroizing password) drops here → scrubbed.
            }
            None => {
                info!("orchestrator: greeter produced no login; exiting host");
                return Ok(());
            }
        }
    }
}
