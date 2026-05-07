mod animation;
mod backend;
mod border;
mod cursor;
mod dispatch;
mod render;
mod input;
mod input_handler;
mod libinput_config;
mod layout;
mod protocols;
mod state;
mod utils;

use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use smithay::reexports::{
    calloop::{generic::Generic, EventLoop, Interest, Mode, PostAction},
    wayland_server::Display,
};
use tracing::{error, info, warn};
use tracing_subscriber::{filter::EnvFilter, fmt};

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

// ── CLI ───────────────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(name = "margo", about = "A feature-rich Wayland compositor")]
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
}

// ── Entry point ───────────────────────────────────────────────────────────────

fn main() -> Result<()> {
    fmt()
        .with_env_filter(
            EnvFilter::try_from_env("MARGO_LOG")
                .or_else(|_| EnvFilter::try_new("info"))
                .unwrap(),
        )
        .init();

    let args = Args::parse();

    let config = margo_config::parse_config(args.config.as_deref()).unwrap_or_else(|e| {
        error!("config error: {e}, using defaults");
        margo_config::Config::default()
    });

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
            error!("udev backend failed: {e}, falling back to winit");
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
    {
        use smithay::xwayland::{XWayland, XWaylandEvent};
        use std::process::Stdio;
        match XWayland::spawn(
            &margo.display_handle,
            None,
            std::iter::empty::<(&str, &str)>(),
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
                            XWaylandEvent::Ready { x11_socket, display_number } => {
                                unsafe {
                                    std::env::set_var("DISPLAY", format!(":{display_number}"));
                                }
                                info!("XWayland ready on :{display_number}");
                                utils::import_session_environment(&[
                                    "XDG_SESSION_DESKTOP",
                                    "DESKTOP_SESSION",
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
            if c.xkb_rules.options.is_empty() { None } else { Some(c.xkb_rules.options.clone()) },
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

    // ── exec_once commands ────────────────────────────────────────────────────
    for cmd in margo.config.exec_once.clone() {
        if let Err(e) = utils::spawn_shell(&cmd) {
            error!("exec-once '{cmd}': {e}");
        }
    }
    if let Some(cmd) = args.startup_command {
        utils::spawn_shell(&cmd)?;
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
        // Animation tick — split borrow across fields
        let now = utils::now_ms();
        let animations_changed = {
            let (clients, curves) = (&mut state.clients, &state.animation_curves);
            state::tick_animations(clients, curves, now)
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
                let win_geom = window.geometry();
                state.space.map_element(window, (geom.x - win_geom.loc.x, geom.y - win_geom.loc.y), false);
            }
            border::refresh(state);
            state.request_repaint();
        }
    })?;

    Ok(())
}
