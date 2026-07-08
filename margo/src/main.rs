// Project-level lint posture — these clippy categories are
// either cosmetic (doc-list-item indentation when continuing a
// multi-paragraph sublist) or false-positives against margo's
// established conventions (`too_many_arguments` on render
// helpers that legitimately take 7-9 fields; breaking them up
// into structs hurts callsite readability without buying
// anything). Keep the lint posture explicit at the crate root
// so clippy stays a useful gate (CI: `cargo clippy ... -D
// warnings`) without false alarms drowning real findings.
#![allow(clippy::doc_overindented_list_items)]
#![allow(clippy::doc_lazy_continuation)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::needless_borrow)]
#![allow(clippy::manual_is_multiple_of)]
#![allow(clippy::collapsible_if)]
#![allow(clippy::excessive_precision)]
#![allow(clippy::needless_borrows_for_generic_args)]

#[cfg(feature = "a11y")]
mod a11y;
mod animation;
mod backend;
mod border;
mod cursor;
#[cfg(feature = "dbus")]
mod dbus;
mod dispatch;
mod frame_clock;
mod input;
mod input_handler;
mod ipc;
mod layout;
mod libinput_config;
mod plugin;
mod protocols;
mod render;
#[cfg(feature = "xdp-gnome-screencast")]
mod screencasting;
mod scripting;
mod session;
mod state;
mod twilight;
mod utils;
mod wallpaper;

// W1.6 — integration test fixture (calloop-driven Server +
// wayland-client Client harness). Compiled only under #[cfg(test)]
// so release builds don't pull in the wayland-client dependencies
// outside their dev scope.
#[cfg(test)]
mod tests;

use std::ffi::OsStr;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use smithay::reexports::{
    calloop::{EventLoop, Interest, Mode, PostAction, generic::Generic},
    wayland_server::Display,
};
use tracing::{error, info, warn};

use state::{MargoClientData, MargoState};

// ── Surface types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceType {
    XdgShell,
    LayerShell,
    X11,
}

// ── Scene layer constants ─────────────────────────────────────────────────────

pub const LYR_BG: usize = 0;
pub const LYR_BLUR: usize = 1;
pub const LYR_BOTTOM: usize = 2;
pub const LYR_TILE: usize = 3;
pub const LYR_FLOAT: usize = 4;
pub const LYR_TOP: usize = 5;
pub const LYR_FADE_OUT: usize = 6;
pub const LYR_OVERLAY: usize = 7;
pub const LYR_IM_POPUP: usize = 8;
pub const LYR_BLOCK: usize = 9;
pub const NUM_LAYERS: usize = 10;
pub const MAX_TAGS: usize = 9;

// ── Pending image-copy-capture frames ────────────────────────────────────────
//
// `ImageCopyCaptureHandler::frame()` runs on `MargoState` but the
// renderer + connector mode info live in the udev backend's
// `BackendData`. The handler stashes incoming frames here; the
// repaint loop drains them after `render_all_outputs` so we can
// reuse the renderer that just produced the live frame for the
// monitor instead of spinning up a second EGL context.
pub struct PendingImageCopyFrame {
    /// The capture target — output by name (Step 2 today; toplevel
    /// support lands in Step 2.5 with a per-window render path).
    pub source: PendingImageCopySource,
    /// The frame the udev backend will render into and signal.
    /// `Option<Frame>` because `Frame::success` consumes the value;
    /// once drained from this list it's `take()`n.
    pub frame: Option<smithay::wayland::image_copy_capture::Frame>,
}

#[derive(Debug, Clone)]
pub enum PendingImageCopySource {
    /// Capture the entire output identified by name (e.g. "DP-3").
    Output(String),
    /// Capture a single toplevel — Step 2.5. Stores the smithay
    /// `Window` directly (Arc-backed so cloning is cheap) so the
    /// index into `state.clients` can shift between frame
    /// request and render-loop drain without invalidating the
    /// reference.
    Toplevel(smithay::desktop::Window),
}

// ── Pending output mode changes (apply path crosses backends) ────────────────
//
// `wlr_output_management_v1` mode changes are accepted by the
// handler running on `MargoState` but the actual DRM use_mode call
// lives in the udev backend, where we have access to the
// `DrmCompositor` and connector mode list. The handler stashes
// requests here; the udev repaint handler drains and applies them
// before the next render.
//
// Defined at the crate root so both `state.rs` (handler) and
// `backend/udev.rs` (drainer) can name the type without a circular
// module dep.
#[derive(Debug, Clone)]
pub struct PendingOutputModeChange {
    pub output_name: String,
    pub width: i32,
    pub height: i32,
    /// Refresh rate in mHz, as the protocol delivers it. Convert
    /// to Hz at match time (drm-rs `Mode::vrefresh()` returns Hz).
    pub refresh_mhz: i32,
}

// ── CLI ───────────────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(
    name = "margo",
    version,
    about = "A feature-rich Wayland compositor",
    long_about = "\
A feature-rich Wayland compositor (Rust/Smithay rewrite of mango).

COMPANION BINARIES (each has its own --help):
  mctl              Compositor IPC client — status, layout, tag,
                    dispatch, actions catalogue (`mctl actions`).
  mshellctl         Desktop-shell IPC client — toggle menus, audio,
                    brightness, wallpaper, lock.
  mlayout           Named monitor-arrangement profiles. Capture the
                    live setup with `init` / `new`, switch with
                    `set`, `next`, `prev`, or `pick`.
  mscreenshot       Screenshot helper — `rec` / `area` / `screen` /
                    `window` / `open` / `dir`.
  mpicker           Native screen colour picker (frozen overlay + lens).
  mlock             Lock-screen binary (PAM + ext-session-lock-v1).

ENVIRONMENT:
  MARGO_LOG         tracing filter (e.g. `info`, `debug`,
                    `margo=trace,smithay=info`).

FILES:
  ~/.config/margo/config.conf   compositor configuration (hot-reloadable)
  $XDG_RUNTIME_DIR/margo/margo-ipc.sock   IPC socket (get / watch / dispatch)

DOCUMENTATION:
  man margo, man mctl, man mshellctl
  /usr/share/doc/margo/        config example, road map, READMEs
  https://github.com/kenanpelit/margo"
)]
struct Args {
    /// Path to config file (default: ~/.config/margo/config.conf)
    #[arg(short, long, value_name = "FILE")]
    config: Option<std::path::PathBuf>,

    /// Startup command to run once the compositor is ready
    #[arg(short = 's', long, value_name = "CMD")]
    startup_command: Option<String>,

    /// Use winit backend (nested Wayland/X11) instead of udev/DRM
    #[arg(long)]
    winit: bool,

    /// Disable margo's in-tree Smithay XWayland and instead run
    /// Supreeeme's `xwayland-satellite` as a separate process. The
    /// out-of-process model is more resilient — an X11 client crash
    /// (or a misbehaving X11Wm) can't take the compositor down — and
    /// inherits xwayland-satellite's bug fixes for HiDPI cursor
    /// scaling, primary-selection bridging, and clipboard MIME
    /// negotiation. Pass without an arg to spawn `xwayland-satellite`
    /// from PATH; pass with `=PATH` to use a specific binary. Niri
    /// pattern, see <https://github.com/Supreeeme/xwayland-satellite>.
    #[arg(long, value_name = "BINARY", num_args = 0..=1, default_missing_value = "xwayland-satellite")]
    xwayland_satellite: Option<String>,

    /// Disable XWayland entirely. No X11 client support — pure
    /// Wayland session. Useful for benchmarks, headless sessions,
    /// containers that don't need X11. Mutually exclusive with
    /// `--xwayland-satellite`; if both are set, `--no-xwayland`
    /// wins.
    #[arg(long)]
    no_xwayland: bool,
}

// ── Entry point ───────────────────────────────────────────────────────────────

/// Live handle to margo's file logger. Set once in `main` after the config is
/// parsed; read by the `loglevel` / `logenabled` dispatch actions and by
/// `reload_config` to retune at runtime. Holds the file sink alive for the
/// whole process.
pub static LOG_HANDLE: std::sync::OnceLock<margo_logging::LogHandle> = std::sync::OnceLock::new();

fn notify_sockaddr(socket_path: &OsStr) -> io::Result<(libc::sockaddr_un, libc::socklen_t)> {
    let bytes = socket_path.as_bytes();
    if bytes.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "empty NOTIFY_SOCKET",
        ));
    }

    // SAFETY: all-zero is a valid initial sockaddr_un before we fill family
    // and path bytes.
    let mut addr: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    addr.sun_family = libc::AF_UNIX as libc::sa_family_t;
    let offset = (&addr.sun_path as *const _ as usize) - (&addr as *const _ as usize);

    let len = if bytes[0] == b'@' {
        let name = &bytes[1..];
        if name.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "empty abstract NOTIFY_SOCKET",
            ));
        }
        if name.len() + 1 > addr.sun_path.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "abstract NOTIFY_SOCKET too long",
            ));
        }
        addr.sun_path[0] = 0;
        for (dst, src) in addr.sun_path[1..].iter_mut().zip(name) {
            *dst = *src as libc::c_char;
        }
        offset + 1 + name.len()
    } else {
        if bytes.len() + 1 > addr.sun_path.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "NOTIFY_SOCKET path too long",
            ));
        }
        for (dst, src) in addr.sun_path.iter_mut().zip(bytes) {
            *dst = *src as libc::c_char;
        }
        offset + bytes.len() + 1
    };

    Ok((addr, len as libc::socklen_t))
}

fn send_notify_message(socket_path: &OsStr, state: &str) -> io::Result<()> {
    let (addr, len) = notify_sockaddr(socket_path)?;
    // SAFETY: socket(2) returns a new fd or -1 with errno set.
    let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }

    // SAFETY: fd is valid, addr points at a sockaddr_un initialised by
    // notify_sockaddr, and state.as_ptr/len describe a live byte slice.
    let sent = unsafe {
        libc::sendto(
            fd,
            state.as_ptr().cast(),
            state.len(),
            0,
            (&addr as *const libc::sockaddr_un).cast(),
            len,
        )
    };
    let result = if sent < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    };
    // SAFETY: fd was returned by socket above.
    unsafe {
        libc::close(fd);
    }
    result
}

/// Tell start-margo / systemd the compositor is genuinely usable.
///
/// When `keep_open` is set the readiness pipe fd is left open for the
/// caller to drive watchdog heartbeats over (see the heartbeat timer in
/// `run`); otherwise it is closed right after the `READY=1` line, which is
/// the default, watchdog-less path.
fn signal_session_ready(ready_fd: Option<libc::c_int>, keep_open: bool) {
    if let Some(fd) = ready_fd {
        let msg = b"READY=1\n";
        // SAFETY: fd is inherited from start-margo and points at a pipe
        // write end. A failed write only means the supervisor is gone or
        // no longer waiting.
        unsafe {
            libc::write(fd, msg.as_ptr().cast(), msg.len());
            if !keep_open {
                libc::close(fd);
            }
        }
        return;
    }

    if let Some(socket_path) = std::env::var_os("NOTIFY_SOCKET")
        && let Err(e) =
            send_notify_message(socket_path.as_os_str(), "READY=1\nSTATUS=margo ready\n")
    {
        warn!(?socket_path, "sd_notify READY failed: {e}");
    }
}

/// First-run config bootstrap. When the config file margo is about to load
/// doesn't exist, write a complete, valid, usable default — `config.conf` + a
/// starter `binds.conf` + a `conf.d/colors.conf` placeholder — so a brand-new
/// session comes up working instead of with bare built-in defaults (which have
/// no keybinds or shell autostart). Best-effort: failures are reported and
/// ignored — margo still falls back to built-in defaults. Runs before logging
/// is up, so it reports via stderr.
fn ensure_default_config(config_override: Option<&std::path::Path>) {
    // Bootstrap whatever path margo will actually load. The session launcher
    // always passes the default path explicitly (`margo -c …/config.conf`), so
    // we must NOT skip just because `--config` is set — only an *existing* file
    // is left untouched.
    let config = match config_override {
        Some(p) => p.to_path_buf(),
        None => {
            let Some(home) = std::env::var_os("HOME") else {
                return;
            };
            std::path::PathBuf::from(home).join(".config/margo/config.conf")
        }
    };
    if config.exists() {
        return;
    }
    let Some(dir) = config.parent().map(std::path::Path::to_path_buf) else {
        return;
    };
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("margo: first-run: could not create {}: {e}", dir.display());
        return;
    }
    if let Err(e) = std::fs::write(&config, include_str!("../assets/default-config.conf")) {
        eprintln!("margo: first-run: could not write default config: {e}");
        return;
    }
    eprintln!(
        "margo: first-run: wrote default config to {}",
        config.display()
    );
    // Placeholder for the matugen palette fragment the default config `source`s
    // (mshell overwrites it on its first wallpaper apply).
    let conf_d = dir.join("conf.d");
    if std::fs::create_dir_all(&conf_d).is_ok() {
        let colors = conf_d.join("colors.conf");
        if !colors.exists() {
            let _ = std::fs::write(
                &colors,
                "# Auto-generated by mshell from the matugen palette. Placeholder\n\
                 # written on first run so config.conf's `source` resolves.\n",
            );
        }
    }
    // Starter binds (config.conf `source`s this). Only when absent so we never
    // clobber a binds.conf the user / Settings → Keybinds already manages.
    let binds = dir.join("binds.conf");
    if !binds.exists()
        && let Err(e) = std::fs::write(&binds, include_str!("../assets/default-binds.conf"))
    {
        eprintln!("margo: first-run: could not write default binds: {e}");
    }
}

fn main() -> Result<()> {
    // Logging is brought up a few lines down — *after* the config parse — so
    // the file sink (~/.local/state/margo/logs/margo-*.log) honours the
    // user's `log_to_file` / `log_file_level` / `log_keep_sessions` knobs from
    // the very first line. `MARGO_LOG` still overrides at startup.

    // Tracy profiler. Built with `--features profile-with-tracy`, this
    // starts the client so the sprinkled `span!`s record and a Tracy GUI
    // (or `tracy-capture`) can connect over TCP; held for the whole
    // process lifetime. Without the feature it is compiled out entirely.
    #[cfg(feature = "profile-with-tracy")]
    let _tracy_client = tracy_client::Client::start();

    // Wrap the default panic hook so an unwind in the compositor (or
    // anything in a calloop dispatch closure) reaches the journal with
    // file:line + message + a backtrace. Without this the user just
    // sees `wayland-wm@margo-session.service: Main process exited`
    // and has to guess.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "<unknown>".to_string());
        let msg = if let Some(s) = info.payload().downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "<non-string panic payload>".to_string()
        };
        error!("PANIC at {location}: {msg}\n{backtrace}");
        default_hook(info);
    }));

    let args = Args::parse();
    let ready_fd = std::env::var_os("MARGO_READY_FD")
        .and_then(|raw| raw.to_string_lossy().parse::<libc::c_int>().ok());
    if ready_fd.is_some() {
        // SAFETY: still single-threaded during argument/config bootstrap.
        unsafe { std::env::remove_var("MARGO_READY_FD") };
    }
    // start-margo asks for watchdog heartbeats by passing an interval (µs).
    // When set we keep the readiness pipe open after READY and write
    // `WATCHDOG=1` from the event loop at this cadence so start-margo can
    // prove the compositor's loop is still turning to systemd's WatchdogSec.
    let heartbeat_usec = std::env::var_os("MARGO_HEARTBEAT_USEC")
        .and_then(|raw| raw.to_string_lossy().parse::<u64>().ok())
        .filter(|&usec| usec > 0);
    if heartbeat_usec.is_some() {
        // SAFETY: still single-threaded during argument/config bootstrap.
        unsafe { std::env::remove_var("MARGO_HEARTBEAT_USEC") };
    }

    // First-run bootstrap: write a usable default config if none exists, so a
    // brand-new session works no matter HOW margo is launched — a login manager
    // / start-margo, a plain `margo` from a TTY, or a custom `.desktop`. Lives
    // here (not start-margo) because margo is the one universal entry point.
    ensure_default_config(args.config.as_deref());

    let (config, config_err) =
        match margo_config::parse_config_with_defaults(args.config.as_deref()) {
            Ok(c) => (c, None),
            Err(e) => (margo_config::Config::default(), Some(e.to_string())),
        };

    // Stand up logging now that the knobs are known. Keeps the last
    // `log_keep_sessions` files in ~/.local/state/margo/logs (margo-*.log).
    let _ = LOG_HANDLE.set(margo_logging::init(margo_logging::LogInit {
        app_name: "margo".to_string(),
        dir: margo_logging::logs_dir(),
        level: config.log_file_level.clone(),
        enabled: config.log_to_file,
        keep_sessions: config.log_keep_sessions.max(1) as usize,
        to_stdout: true,
        env_override: Some("MARGO_LOG".to_string()),
    }));

    if let Some(e) = config_err {
        error!("config error: {e}, using defaults");
    }

    for (name, value) in &config.envs {
        // SAFETY: single-threaded at this point, no other threads reading env
        unsafe { std::env::set_var(name, value) };
    }

    info!("margo v{} starting", env!("CARGO_PKG_VERSION"));

    // ── Event loop + Wayland display ──────────────────────────────────────────
    let mut event_loop: EventLoop<'static, MargoState> = EventLoop::try_new()?;
    let loop_handle = event_loop.handle();

    let mut display: Display<MargoState> = Display::new()?;

    // ── Create compositor state ───────────────────────────────────────────────
    let loop_signal = event_loop.get_signal();
    let mut margo = MargoState::new(
        config,
        &mut display,
        loop_handle.clone(),
        loop_signal,
        args.config.clone(),
    );

    // Socket IPC: export MARGO_SOCKET (before any child is spawned so
    // it's inherited) and bind the control socket on the event loop.
    ipc::export_socket_env();
    ipc::insert_ipc_source(&loop_handle);

    // SIGUSR1 → dump runtime state to the journal. Lets a user staring
    // at a frozen / grey screen capture diagnostics without crashing the
    // compositor:
    //   pkill -USR1 margo
    // The dump goes through `tracing::info!` so it lands wherever the
    // user's MARGO_LOG filter sends regular output.
    match smithay::reexports::calloop::signals::Signals::new(&[
        smithay::reexports::calloop::signals::Signal::SIGUSR1,
    ]) {
        Ok(signals) => {
            if let Err(e) = loop_handle.insert_source(signals, |_, _, state: &mut MargoState| {
                state.debug_dump();
            }) {
                warn!("SIGUSR1 source: {e}");
            }
        }
        _ => {
            warn!("could not register SIGUSR1 — `pkill -USR1 margo` won't work");
        }
    }

    // Wayland display source: when the display fd is readable, dispatch
    // pending client requests, then flush any responses. Without
    // dispatch_clients, surface commits / xdg_shell requests are never
    // processed and clients render nothing (gray screen).
    let display_source = Generic::new(display, Interest::READ, Mode::Level);
    loop_handle
        .insert_source(display_source, |_, display, state: &mut MargoState| {
            // SAFETY: we don't drop the display, only borrow it for dispatch.
            unsafe {
                display.get_mut().dispatch_clients(state).ok();
            }
            state.display_handle.flush_clients().ok();
            Ok(PostAction::Continue)
        })
        .map_err(|e| anyhow::anyhow!("display source: {e}"))?;

    // ── Open Wayland socket ───────────────────────────────────────────────────
    // Save parent display env BEFORE overwriting so the winit backend
    // can connect to the real parent compositor (not our own socket).
    // Only WAYLAND_DISPLAY indicates a real nested session; DISPLAY can be
    // set by display managers on bare metal and must not trigger winit.
    let parent_wayland_display = std::env::var("WAYLAND_DISPLAY").ok();
    let parent_x_display = std::env::var("DISPLAY").ok();
    let running_nested = parent_wayland_display.is_some();

    let socket_source = smithay::wayland::socket::ListeningSocketSource::new_auto()?;
    let socket_name = socket_source.socket_name().to_os_string();
    loop_handle
        .insert_source(socket_source, |client_stream, _, state: &mut MargoState| {
            state
                .display_handle
                .insert_client(client_stream, Arc::new(MargoClientData::default()))
                .ok();
        })
        .map_err(|e| anyhow::anyhow!("socket source: {e}"))?;

    // Expose our socket to child processes
    // SAFETY: single-threaded; no other thread reading env
    unsafe { std::env::set_var("WAYLAND_DISPLAY", &socket_name) };
    info!("Wayland socket: {:?}", socket_name);

    // ── Backend ───────────────────────────────────────────────────────────────
    let use_winit = args.winit || running_nested;

    if use_winit {
        info!("using winit backend");
        // Restore parent display env so winit connects to the real parent compositor
        if let Some(wd) = &parent_wayland_display {
            unsafe { std::env::set_var("WAYLAND_DISPLAY", wd) };
        } else {
            unsafe { std::env::remove_var("WAYLAND_DISPLAY") };
        }
        if let Some(xd) = &parent_x_display {
            unsafe { std::env::set_var("DISPLAY", xd) };
        }
        let result = backend::winit::run(&mut margo, &mut event_loop);
        // Restore our socket for clients to connect to us
        unsafe { std::env::set_var("WAYLAND_DISPLAY", &socket_name) };
        result?;
    } else {
        info!("using udev backend");
        if let Err(e) = backend::udev::run(&mut margo, &mut event_loop) {
            // udev / DRM bring-up failed. Most common causes (in
            // descending order of frequency): no GPU at all (qemu
            // without virgl, container without /dev/dri), mesa
            // drivers missing, /dev/dri/card* permission denied,
            // running on the wrong VT.
            //
            // We try winit nested mode as a fallback — that needs
            // a parent wayland/x11 session, which is already the
            // case during dev iteration. On a fresh TTY login
            // with no GPU, winit will also fail here (it ALSO
            // needs EGL today; full software rendering via
            // pixman is W2.2b, not yet shipped).
            error!("udev backend failed: {e}");
            error!("");
            error!("Common fixes:");
            error!("  • Install Mesa drivers: `sudo pacman -S mesa libglvnd`");
            error!("  • Check /dev/dri/card* permission (user must be in the `video` seat group)");
            error!("  • In qemu, enable virtio-gpu with --enable-virgl");
            error!("");
            error!("Falling back to winit (nested mode — needs WAYLAND_DISPLAY or DISPLAY).");
            error!(
                "Software rendering (pixman) fallback is W2.2b in road_map.md, not yet shipped."
            );
            if let Some(wd) = &parent_wayland_display {
                unsafe { std::env::set_var("WAYLAND_DISPLAY", wd) };
            }
            let result = backend::winit::run(&mut margo, &mut event_loop);
            unsafe { std::env::set_var("WAYLAND_DISPLAY", &socket_name) };
            result?;
        }
    }

    // ── Push compositor environment into systemd/dbus activation ──────────────
    // User services and XDG autostart entries such as noctalia need these before
    // the session target starts them, otherwise they can select the wrong backend.
    utils::import_session_environment(&["XDG_SESSION_DESKTOP", "DESKTOP_SESSION"]);

    // ── XWayland ──────────────────────────────────────────────────────────────
    //
    // Three modes (W2.5):
    //   * `--no-xwayland` → don't spawn anything. Pure Wayland session.
    //   * `--xwayland-satellite[=PATH]` → spawn `xwayland-satellite`
    //     (Supreeeme) as a separate process. Resilience win: an X11
    //     client crash can't take margo down. The satellite
    //     registers as a regular Wayland client, opens its own
    //     DISPLAY socket, and forwards clipboard / primary /
    //     selection / clipboard-mime negotiation.
    //   * Default → in-tree smithay XWayland (existing behaviour;
    //     same in-process model that's been here since day one).
    let want_intree_xwayland = !args.no_xwayland && args.xwayland_satellite.is_none();
    if args.no_xwayland {
        info!("--no-xwayland set; X11 client support disabled");
    } else if let Some(satellite_bin) = args.xwayland_satellite.clone() {
        info!("xwayland-satellite mode: spawning `{satellite_bin}` as separate process");
        match std::process::Command::new(&satellite_bin)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(child) => {
                info!(
                    "spawned xwayland-satellite pid={} — set DISPLAY to the satellite's \
                     socket (typically :0; satellite logs the actual number on stderr if \
                     you redirect it)",
                    child.id()
                );
                // Don't `wait()` — satellite reparents to systemd
                // like any daemon. Strict lifetime coupling is
                // user-side: pair `--no-xwayland` with a systemd
                // user unit `PartOf=margo-session.target` instead.
                std::mem::drop(child);
            }
            Err(e) => {
                error!(
                    "failed to spawn xwayland-satellite at `{satellite_bin}`: {e}\n  \
                     install: `cargo install xwayland-satellite`\n  \
                     or pass `--xwayland-satellite=/full/path/to/binary`\n  \
                     X11 client support is now off (no fallback)"
                );
            }
        }
    }
    if want_intree_xwayland {
        use smithay::xwayland::{XWayland, XWaylandEvent};
        use std::process::Stdio;
        match XWayland::spawn(
            &margo.display_handle,
            None,
            std::iter::empty::<(&str, &str)>(),
            // `extra_args`: added upstream (smithay HEAD) between `envs`
            // and `open_abstract_socket` — we pass no extra X server args.
            std::iter::empty::<&str>(),
            true,
            Stdio::null(),
            Stdio::null(),
            |_| {},
        ) {
            Ok((xwayland, client)) => {
                let loop_handle = event_loop.handle();
                loop_handle
                    .insert_source(xwayland, move |event, _, state: &mut MargoState| {
                        match event {
                            XWaylandEvent::Ready {
                                x11_socket,
                                display_number,
                            } => {
                                unsafe {
                                    std::env::set_var("DISPLAY", format!(":{display_number}"));
                                    // XCURSOR_SIZE / XCURSOR_THEME let
                                    // XWayland apps pick up the same
                                    // cursor the native Wayland side
                                    // uses. Without these the X11
                                    // cursor falls back to libxcursor's
                                    // 16-px default and the user sees
                                    // a noticeably-smaller pointer the
                                    // moment an X11 client takes the
                                    // pointer (the classic
                                    // "Steam / Discord / Spotify
                                    // cursor shrinks on hover" bug).
                                    // Default theme is left to the
                                    // user's `XCURSOR_THEME` env if
                                    // already set; we only fill in
                                    // a missing slot so we never
                                    // override an explicit choice.
                                    let cursor_size = state.config.cursor_size.max(8);
                                    std::env::set_var("XCURSOR_SIZE", cursor_size.to_string());
                                    if std::env::var_os("XCURSOR_THEME").is_none() {
                                        if let Some(theme) = state
                                            .config
                                            .cursor_theme
                                            .as_deref()
                                            .filter(|s| !s.is_empty())
                                        {
                                            std::env::set_var("XCURSOR_THEME", theme);
                                        }
                                    }
                                }
                                info!(
                                    "XWayland ready on :{display_number} \
                                     XCURSOR_SIZE={} XCURSOR_THEME={}",
                                    state.config.cursor_size,
                                    std::env::var("XCURSOR_THEME")
                                        .unwrap_or_else(|_| "<unset>".into()),
                                );
                                utils::import_session_environment(&[
                                    "XDG_SESSION_DESKTOP",
                                    "DESKTOP_SESSION",
                                    "DISPLAY",
                                    "XCURSOR_SIZE",
                                    "XCURSOR_THEME",
                                ]);
                                match smithay::xwayland::X11Wm::start_wm(
                                    state.loop_handle.clone(),
                                    &state.display_handle,
                                    x11_socket,
                                    client.clone(),
                                ) {
                                    Ok(wm) => state.xwm = Some(wm),
                                    Err(e) => error!("X11Wm::start_wm: {e}"),
                                }
                            }
                            XWaylandEvent::Error => {
                                error!("XWayland startup error");
                            }
                        }
                    })
                    .map_err(|e| anyhow::anyhow!("XWayland source: {e}"))?;
            }
            Err(e) => warn!("XWayland spawn failed (X11 apps unavailable): {e}"),
        }
    }

    // ── Add keyboard to seat ──────────────────────────────────────────────────
    let (xkb_rules, xkb_model, xkb_layout, xkb_variant, xkb_options, repeat_delay, repeat_rate) = {
        let c = &margo.config;
        (
            c.xkb_rules.rules.clone(),
            c.xkb_rules.model.clone(),
            c.xkb_rules.layout.clone(),
            c.xkb_rules.variant.clone(),
            if c.xkb_rules.options.is_empty() {
                None
            } else {
                Some(c.xkb_rules.options.clone())
            },
            c.repeat_delay,
            c.repeat_rate,
        )
    };
    let keyboard = margo
        .seat
        .add_keyboard(
            smithay::input::keyboard::XkbConfig {
                rules: &xkb_rules,
                model: &xkb_model,
                layout: &xkb_layout,
                variant: &xkb_variant,
                options: xkb_options,
            },
            repeat_delay,
            repeat_rate,
        )
        .map_err(|e| anyhow::anyhow!("keyboard init: {e}"))?;
    let _ = keyboard;

    margo.seat.add_pointer();

    // ── D-Bus shims for xdp-gnome screencast support ──────────────────────────
    // Stand up the Mutter / Shell D-Bus interfaces in the user-bus
    // session so xdg-desktop-portal-gnome can serve the ScreenCast
    // / Screenshot / DisplayConfig portals against margo. Each
    // shim claims its own bus name; failures are logged but
    // non-fatal — a missing bus or zbus error just means xdp-gnome
    // can't serve that one portal interface, the rest of the
    // compositor keeps running. See `crate::dbus` for the
    // architecture and `docs/portal-design.md` for the rollout
    // plan. This call is the bring-up entry point niri's pattern
    // calls `DBusServers::start`.
    #[cfg(feature = "dbus")]
    {
        use crate::dbus::Start as _;
        use crate::dbus::mutter_service_channel::{NewClient, ServiceChannel};

        // Per-interface channels so blocking D-Bus callbacks can
        // hand work to the calloop thread without taking a
        // borrow on MargoState.
        let (svc_tx, svc_rx) = calloop::channel::channel::<NewClient>();
        match event_loop
            .handle()
            .insert_source(svc_rx, |event, _, state: &mut MargoState| match event {
                calloop::channel::Event::Msg(client) => {
                    // xdp-gnome opened a service-channel Wayland
                    // socket. Insert the compositor-side end into
                    // the display so xdp becomes its own client.
                    let stream = match client.client.try_clone() {
                        Ok(s) => s,
                        Err(e) => {
                            tracing::warn!("svc client clone failed: {e:?}");
                            return;
                        }
                    };
                    if let Err(e) = state
                        .display_handle
                        .insert_client(stream, std::sync::Arc::new(MargoClientData::default()))
                    {
                        tracing::warn!("insert_client (svc): {e:?}");
                    }
                }
                calloop::channel::Event::Closed => (),
            }) {
            Err(e) => {
                warn!("ServiceChannel calloop source insert failed: {e}");
            }
            _ => match ServiceChannel::new(svc_tx).start() {
                Ok(conn) => margo.dbus_servers.conn_service_channel = Some(conn),
                Err(e) => warn!("ServiceChannel D-Bus start failed: {e}"),
            },
        }
    }

    // ── DisplayConfig D-Bus shim ──────────────────────────────────────────────
    // xdp-gnome cross-references monitor enumeration on
    // `org.gnome.Mutter.DisplayConfig` when populating the
    // ScreenCast chooser's Entire Screen tab.
    #[cfg(feature = "dbus")]
    {
        use crate::dbus::Start as _;
        use crate::dbus::mutter_display_config::DisplayConfig;

        // Snapshot the live monitor list into the shared
        // `ipc_outputs` Arc once, so DisplayConfig sees the
        // current outputs without holding a stale per-service
        // copy. Subsequent hotplugs route through
        // `MargoState::refresh_ipc_outputs()` which mutates the
        // SAME Arc — both DisplayConfig and ScreenCast pick the
        // new state up on their next read.
        margo.refresh_ipc_outputs();
        match DisplayConfig::new(margo.ipc_outputs.clone()).start() {
            Ok(conn) => margo.dbus_servers.conn_display_config = Some(conn),
            Err(e) => warn!("DisplayConfig D-Bus start failed: {e}"),
        }
    }

    // ── Gnome Shell Introspect D-Bus shim ─────────────────────────────────────
    // Powers the Window tab of xdp-gnome's screencast chooser:
    // `GetWindows` returns the list of toplevels with title +
    // app_id. The compositor side answers via the from_compositor
    // async-channel — for now respond with margo's current
    // `clients` snapshot synchronously inline so the chooser
    // dialog populates without round-tripping back through
    // calloop. Live reactive updates (`windows_changed` signal)
    // are a follow-up.
    #[cfg(feature = "dbus")]
    {
        use crate::dbus::Start as _;
        use crate::dbus::gnome_shell_introspect::{
            CompositorToIntrospect, Introspect, IntrospectToCompositor, WindowProperties,
        };
        use std::collections::HashMap;

        let (intr_tx, intr_rx) = calloop::channel::channel::<IntrospectToCompositor>();
        let (resp_tx, resp_rx) = async_channel::unbounded::<CompositorToIntrospect>();

        match event_loop
            .handle()
            .insert_source(intr_rx, move |event, _, state: &mut MargoState| {
                if let calloop::channel::Event::Msg(IntrospectToCompositor::GetWindows) = event {
                    let mut map: HashMap<u64, WindowProperties> = HashMap::new();
                    for c in &state.clients {
                        // Stable per-client id (minted at construction).
                        // Must match what StartCast resolves against — a
                        // pointer-address id aliased a different window
                        // once `clients` reallocated between this snapshot
                        // and the cast request.
                        map.insert(
                            c.id,
                            WindowProperties {
                                title: c.title.clone(),
                                app_id: c.app_id.clone(),
                            },
                        );
                    }
                    let _ = resp_tx.try_send(CompositorToIntrospect::Windows(map));
                }
            }) {
            Err(e) => {
                warn!("Introspect calloop source insert failed: {e}");
            }
            _ => match Introspect::new(intr_tx, resp_rx).start() {
                Ok(conn) => margo.dbus_servers.conn_introspect = Some(conn),
                Err(e) => warn!("Introspect D-Bus start failed: {e}"),
            },
        }
    }

    // ── Gnome Shell Screenshot D-Bus shim ─────────────────────────────────────
    // Programmatic Screenshot portal path. Margo already has the
    // keybind-driven `margo-screenshot` script for users; this
    // shim handles the API path (browser screenshot APIs, GNOME
    // apps invoking the portal).
    #[cfg(feature = "dbus")]
    {
        use crate::dbus::Start as _;
        use crate::dbus::gnome_shell_screenshot::{
            CompositorToScreenshot, Screenshot, ScreenshotToCompositor,
        };

        let (shot_tx, shot_rx) = calloop::channel::channel::<ScreenshotToCompositor>();
        let (resp_tx, resp_rx) = async_channel::unbounded::<CompositorToScreenshot>();

        match event_loop.handle().insert_source(
            shot_rx,
            move |event, _, _state: &mut MargoState| {
                let calloop::channel::Event::Msg(msg) = event else {
                    return;
                };
                match msg {
                    ScreenshotToCompositor::TakeScreenshot { include_cursor } => {
                        // Capture the whole desktop to a temp PNG via
                        // `grim` — a child process that speaks
                        // wlr-screencopy, so margo's event loop keeps
                        // servicing it while it runs. We must NOT block
                        // here (that would deadlock the capture), so we
                        // wait for grim in a detached thread and report
                        // the resulting path back to the portal shim.
                        // The portal frontend turns it into the
                        // `file://` URI the requesting app receives.
                        let runtime =
                            std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_owned());
                        let stamp = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_millis())
                            .unwrap_or(0);
                        let path = std::path::PathBuf::from(format!(
                            "{runtime}/margo-portal-shot-{stamp}.png"
                        ));
                        let mut cmd = std::process::Command::new("grim");
                        if include_cursor {
                            cmd.arg("-c");
                        }
                        cmd.arg(&path);
                        match cmd.spawn() {
                            Ok(mut child) => {
                                let resp = resp_tx.clone();
                                std::thread::spawn(move || {
                                    let ok = matches!(child.wait(), Ok(s) if s.success())
                                        && path.exists();
                                    let _ =
                                        resp.try_send(CompositorToScreenshot::ScreenshotResult(
                                            ok.then_some(path),
                                        ));
                                });
                            }
                            Err(e) => {
                                warn!("grim spawn for portal screenshot failed: {e}");
                                let _ = resp_tx
                                    .try_send(CompositorToScreenshot::ScreenshotResult(None));
                            }
                        }
                    }
                    ScreenshotToCompositor::PickColor(reply) => {
                        // No in-tree color picker yet — let the
                        // requesting app handle it.
                        let _ = reply.try_send(None);
                    }
                }
            },
        ) {
            Err(e) => {
                warn!("Screenshot calloop source insert failed: {e}");
            }
            _ => match Screenshot::new(shot_tx, resp_rx).start() {
                Ok(conn) => margo.dbus_servers.conn_screen_shot = Some(conn),
                Err(e) => warn!("Screenshot D-Bus start failed: {e}"),
            },
        }
    }

    // ── Mutter ScreenCast D-Bus shim — the main event ─────────────────────────
    // This is the interface that xdp-gnome calls to mint
    // sessions / streams. Without it, the Window + Entire Screen
    // tabs in browser meeting clients stay grayed out.
    //
    // The receiver side (`ScreenCastToCompositor` channel) wires
    // into `MargoState::screencasting`; `StartCast` boots a
    // PipeWire stream against the source, `StopCast` tears it
    // down. The actual cast lifecycle / render hook for
    // emitting frames lives in
    // `crate::screencasting::pw_utils::Cast`.
    #[cfg(feature = "xdp-gnome-screencast")]
    {
        use crate::dbus::Start as _;
        use crate::dbus::mutter_screen_cast::{ScreenCast, ScreenCastToCompositor};

        let (sc_tx, sc_rx) = calloop::channel::channel::<ScreenCastToCompositor>();
        // Same shared `ipc_outputs` Arc DisplayConfig got above;
        // hotplug-driven `refresh_ipc_outputs()` mutates the map
        // in place, so ScreenCast's chooser reflects the live
        // output set without a margo restart.
        let outputs = margo.ipc_outputs.clone();

        match event_loop
            .handle()
            .insert_source(sc_rx, |event, _, state: &mut MargoState| {
                let calloop::channel::Event::Msg(msg) = event else {
                    return;
                };
                match msg {
                    ScreenCastToCompositor::StartCast {
                        session_id,
                        stream_id,
                        target,
                        cursor_mode,
                        signal_ctx,
                    } => {
                        state.start_cast(session_id, stream_id, target, cursor_mode, signal_ctx);
                    }
                    ScreenCastToCompositor::StopCast { session_id } => {
                        state.stop_cast(session_id);
                    }
                }
            }) {
            Err(e) => {
                warn!("ScreenCast calloop source insert failed: {e}");
            }
            _ => match ScreenCast::new(outputs, sc_tx).start() {
                Ok(conn) => margo.dbus_servers.conn_screen_cast = Some(conn),
                Err(e) => warn!("ScreenCast D-Bus start failed: {e}"),
            },
        }
    }

    // Tell start-margo / systemd that the compositor is genuinely usable:
    // Wayland socket is open, backend is ready, compositor env is imported,
    // and XWayland / portal services have been attempted. Do this before
    // exec_once/startup commands so dependent services do not race a half-built
    // compositor.
    signal_session_ready(ready_fd, heartbeat_usec.is_some());

    // ── Watchdog heartbeat ────────────────────────────────────────────────────
    // When start-margo requested heartbeats (systemd WatchdogSec is set), beat
    // the readiness pipe from the event loop. A wedged loop stops beating, so
    // start-margo stops pinging systemd and the watchdog recovers a *hung*
    // compositor — not just a crashed one. A broken pipe (supervisor gone)
    // closes the fd and drops the timer.
    if let (Some(fd), Some(usec)) = (ready_fd, heartbeat_usec) {
        let interval = std::time::Duration::from_micros(usec);
        let timer = calloop::timer::Timer::from_duration(interval);
        let _ = event_loop
            .handle()
            .insert_source(timer, move |_, _, _state: &mut MargoState| {
                let msg = b"WATCHDOG=1\n";
                // SAFETY: fd is the readiness pipe write end kept open by
                // signal_session_ready. Rust ignores SIGPIPE, so a dead reader
                // yields an error return, not a signal.
                let n = unsafe { libc::write(fd, msg.as_ptr().cast(), msg.len()) };
                if n < 0 {
                    // SAFETY: fd is owned here and we are dropping the timer.
                    unsafe { libc::close(fd) };
                    return calloop::timer::TimeoutAction::Drop;
                }
                calloop::timer::TimeoutAction::ToDuration(interval)
            });
    }

    // ── exec_once commands ────────────────────────────────────────────────────
    for cmd in margo.config.exec_once.clone() {
        if let Err(e) = utils::spawn_shell(&cmd) {
            error!("exec-once '{cmd}': {e}");
        }
    }
    if let Some(cmd) = args.startup_command {
        utils::spawn_shell(&cmd)?;
    }

    // ── AccessKit a11y adapter ────────────────────────────────────────────────
    // Spin up the screen-reader bridge thread before any clients
    // map. Initial tree publishes after the first `arrange_all`;
    // Orca + AT-SPI consumers see "margo" with an empty window
    // list until then. Best-effort — `a11y.start()` logs +
    // continues on failure.
    #[cfg(feature = "a11y")]
    margo.a11y.start();

    // ── User scripting (~/.config/margo/init.rhai) ───────────────────────────
    // Compiles + evaluates the user script once, after exec_once
    // but before the event loop. The ScriptingState (engine, AST,
    // registered hook FnPtrs) parks on MargoState for the lifetime
    // of the compositor so on_focus_change / on_tag_switch /
    // on_window_open handlers fire mid-event-loop (Phase 3).
    scripting::init_user_scripting(&mut margo);
    // ── Plugins (~/.config/margo/plugins/<name>/) ─────────────────────────────
    // W3.3: discover + load every plugin directory. Each plugin's
    // init.rhai runs against the same engine init.rhai used, so
    // hooks layer on top. Plugins with `enabled = false` in
    // their manifest are skipped.
    scripting::init_plugins(&mut margo);

    // ── Twilight kick-off ─────────────────────────────────────────────────────
    // First tick runs synchronously so the initial gamma ramp lands
    // before the user sees the first frame. After that, a calloop
    // timer re-arms itself on every tick at the interval returned
    // by `tick_twilight` (60 s idle, ~250 ms in transition).
    {
        let initial_delay = margo.tick_twilight();
        let loop_handle = event_loop.handle();
        let timer = calloop::timer::Timer::from_duration(initial_delay);
        let _ = loop_handle.insert_source(timer, move |_, _, state: &mut MargoState| {
            let next = state.tick_twilight();
            calloop::timer::TimeoutAction::ToDuration(next)
        });
        margo.twilight_timer_armed = true;
    }

    // ── Run the event loop ────────────────────────────────────────────────────
    event_loop.run(None, &mut margo, |state| {
        if state.should_quit {
            state.loop_signal.stop();
            return;
        }
        // Flush pending Wayland messages after each iteration
        if let Err(e) = state.display_handle.flush_clients() {
            error!("flush_clients: {e}");
        }
        // Coalesced IPC push: a burst of state changes within one
        // dispatch marks state dirty repeatedly but fans out one
        // `watch` snapshot frame here, once per iteration.
        state.flush_ipc_if_dirty();
        // Animation tick — split borrow across fields
        let now = utils::now_ms();
        let animations_changed = {
            let cfg = &state.config;
            let use_spring = cfg.animation_clock_move.eq_ignore_ascii_case("spring");
            // The spring carried in `AnimTickSpec` is a 0→1 *progress*
            // spring — its from/to/initial_velocity are unused at tick
            // time; only the params (damping/mass/stiffness/epsilon)
            // matter, and they're rebuilt every frame from config so
            // `/reload` picks up new tuning without restart.
            let spring = animation::spring::Spring {
                from: 0.0,
                to: 1.0,
                initial_velocity: 0.0,
                params: animation::spring::SpringParams::new(
                    cfg.animation_spring_damping_ratio,
                    cfg.animation_spring_stiffness,
                    0.0001,
                ),
            };
            let spec = state::AnimTickSpec {
                duration_move: cfg.animation_duration_move,
                use_spring,
                spring,
            };
            // Disjoint borrows across MargoState fields so each
            // category of animation can be advanced from a single
            // call. The compiler treats these as independent
            // because they're distinct named fields.
            let curves = &state.animation_curves;
            state::tick_animations(
                &mut state.clients,
                curves,
                now,
                spec,
                &mut state.closing_clients,
                &mut state.layer_animations,
            )
        };
        if animations_changed {
            let animated: Vec<_> = state
                .clients
                .iter()
                .filter(|client| {
                    state.monitors.get(client.monitor).is_some_and(|monitor| {
                        let tagset = if monitor.is_overview {
                            !0
                        } else {
                            monitor.current_tagset()
                        };
                        client.is_visible_on(client.monitor, tagset)
                    })
                })
                .map(|client| (client.window.clone(), client.geom))
                .collect();
            for (window, geom) in animated {
                // Match `arrange_monitor`'s convention exactly:
                // `Space::map_element` records the location of the
                // window's *geometry origin* in space coords. Render
                // path (push_client_elements) then computes
                // `render_location = element_location - window.geometry().loc`
                // to put the surface buffer in the right physical
                // spot. Subtracting `geometry().loc` HERE on top of
                // the render-path subtraction produces a double-
                // correction: the surface ends up at
                // `c.geom.loc - 2 * geometry.loc` instead of
                // `c.geom.loc` for any client with non-zero geometry
                // offset (Electron toplevels frequently report a
                // non-zero `geometry().loc` even with server-side
                // decorations). That's the "border içindeki pencere
                // border kadar hızlı hareket etmiyor" symptom — every
                // animation tick re-positioned the surface
                // `geometry.loc` short of where the border tracked.
                state.space.map_element(window, (geom.x, geom.y), false);
            }
            // smithay's `space.map_element` always moves the touched
            // element to the top of the stack, so an animated tile
            // would otherwise leap above an unrelated floating window
            // (CopyQ, pavucontrol). Re-establish the z bands after
            // every animation tick.
            state.enforce_z_order();
            border::refresh(state);
            state.request_repaint();
        }

        // Scroller-overview open/close zoom animation. Advances the
        // eased progress and keeps repainting until it settles (a close
        // also clears the overview here when it reaches 0).
        if state.tick_scroller_overview(now) {
            state.request_repaint();
        }

        // Config-error overlay timeout. The banner is armed by
        // `MargoState::reload_config` with a 10 s deadline; once the
        // deadline passes we clear it here and request one final
        // repaint so the banner doesn't visually linger on a stale
        // frame (the renderer would otherwise re-emit it on next
        // damage even though the deadline is gone). Cheap check —
        // a comparison per loop iteration when the overlay is active,
        // nothing when it isn't.
        if let Some(until) = state.config_error_overlay_until {
            if std::time::Instant::now() >= until {
                state.config_error_overlay_until = None;
                state.request_repaint();
            }
        }
    })?;

    Ok(())
}
