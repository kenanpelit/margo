//! Native video-wallpaper engine: wlr-layer-shell background surface +
//! EGL + libmpv render context, driven by a `poll(2)` loop over the
//! wayland fd + an mpv render-update eventfd (mirrors mpvpaper's model).

mod egl;
mod mpv_sys;
mod render;
mod wayland;

use crate::geometry::ScaleMode;
use anyhow::{Result, anyhow, bail};
use egl::{EglOutput, EglRoot};
use render::MpvVideo;
use std::os::raw::c_void;
use std::os::unix::io::{AsFd, AsRawFd, RawFd};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use wayland::PaperState;
use wayland_client::Proxy;

/// Wallpaper playback options.
pub struct PaperOpts {
    pub mute: bool,
    pub looping: bool,
    pub scale: ScaleMode,
}

static QUIT: AtomicBool = AtomicBool::new(false);

extern "C" fn on_term(_sig: i32) {
    QUIT.store(true, Ordering::SeqCst);
}

fn pidfile_dir() -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("/run/user/{}", unsafe { libc::getuid() })));
    base.join("mplay")
}

fn pidfile_for(output: Option<&str>) -> PathBuf {
    pidfile_dir().join(format!("{}.pid", output.unwrap_or("all")))
}

/// One live output: its EGL surface + mpv instance + current size.
struct LiveOutput {
    egl: EglOutput,
    mpv: Option<MpvVideo>,
    w: i32,
    h: i32,
}

/// Play `src` as a wallpaper on `output` (or all outputs).
pub fn run(src: &str, output: Option<&str>, opts: PaperOpts, daemon: bool) -> Result<()> {
    if daemon {
        daemonize()?;
    }

    let (conn, mut queue, mut state) = PaperState::connect()?;
    let qh = queue.handle();
    state.create_surfaces(output, &qh);
    if state.surfaces.is_empty() {
        bail!(
            "no matching output{}",
            output.map(|o| format!(" `{o}`")).unwrap_or_default()
        );
    }
    // Pump configure events until every surface knows its size.
    let mut guard_rounds = 0;
    while !state.all_configured && guard_rounds < 100 {
        queue.blocking_dispatch(&mut state)?;
        guard_rounds += 1;
    }
    if !state.all_configured {
        bail!("layer surfaces were never configured");
    }

    // pidfile
    let pidfile = pidfile_for(output);
    if let Some(dir) = pidfile.parent() {
        std::fs::create_dir_all(dir).ok();
    }
    std::fs::write(&pidfile, std::process::id().to_string()).ok();

    // EGL root (boxed + leaked-stable address for the mpv proc-address ctx).
    let wl_display = conn.backend().display_ptr() as *mut c_void;
    let egl_root = EglRoot::new(wl_display)?;
    let egl_root_ptr = &*egl_root as *const EglRoot as *mut c_void;

    // One eventfd shared by all mpv instances' update callbacks.
    let efd: RawFd = unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) };
    if efd < 0 {
        bail!("eventfd failed");
    }

    // Build a live output (EGL surface + mpv) per configured surface.
    let mut outs: Vec<LiveOutput> = Vec::new();
    for s in &state.surfaces {
        let egl_out = EglOutput::new(&egl_root, s.wl_surface.id(), s.width, s.height)?;
        let mpv = MpvVideo::new(egl_root_ptr, wl_display, src, &opts, efd)?;
        let _ = mpv.render(&egl_out, &egl_root, s.width, s.height);
        outs.push(LiveOutput {
            egl: egl_out,
            mpv: Some(mpv),
            w: s.width,
            h: s.height,
        });
    }

    install_term_handlers();

    let wl_fd = conn.as_fd().as_raw_fd();
    let result = run_loop(
        &conn, &mut queue, &mut state, &egl_root, &mut outs, wl_fd, efd,
    );

    // Cleanup: drop each mpv with its GL context current, then teardown.
    for o in &mut outs {
        let _ = o.egl.make_current(&egl_root);
        o.mpv.take();
    }
    unsafe { libc::close(efd) };
    std::fs::remove_file(&pidfile).ok();
    result
}

#[allow(clippy::too_many_arguments)]
fn run_loop(
    conn: &wayland_client::Connection,
    queue: &mut wayland_client::EventQueue<PaperState>,
    state: &mut PaperState,
    egl_root: &EglRoot,
    outs: &mut [LiveOutput],
    wl_fd: RawFd,
    efd: RawFd,
) -> Result<()> {
    while !QUIT.load(Ordering::SeqCst) {
        queue.dispatch_pending(state)?;
        conn.flush()?;

        let guard = conn.prepare_read();
        let mut fds = [
            libc::pollfd {
                fd: wl_fd,
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: efd,
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        // 1s timeout so the SIGTERM flag is re-checked promptly.
        let n = unsafe { libc::poll(fds.as_mut_ptr(), 2, 1000) };
        if n < 0 {
            drop(guard);
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            return Err(anyhow!("poll failed: {err}"));
        }

        if fds[0].revents & libc::POLLIN != 0 {
            if let Some(g) = guard {
                let _ = g.read();
            }
        } else {
            drop(guard);
        }
        queue.dispatch_pending(state)?;

        // Reconcile any size changes from re-configure events.
        for (i, o) in outs.iter_mut().enumerate() {
            if let Some(s) = state.surfaces.get(i)
                && (s.width, s.height) != (o.w, o.h)
                && s.width > 0
                && s.height > 0
            {
                o.egl.resize(s.width, s.height);
                o.w = s.width;
                o.h = s.height;
            }
        }

        if fds[1].revents & libc::POLLIN != 0 {
            let mut buf = [0u8; 8];
            unsafe {
                libc::read(efd, buf.as_mut_ptr() as *mut c_void, 8);
            }
            for o in outs.iter() {
                if let Some(mpv) = &o.mpv
                    && let Err(e) = mpv.render(&o.egl, egl_root, o.w, o.h)
                {
                    tracing::warn!("render: {e}");
                }
            }
        }
    }
    Ok(())
}

fn install_term_handlers() {
    // Coerce to a typed fn pointer before the numeric cast so clippy
    // doesn't flag a function-item-to-integer cast.
    let handler: extern "C" fn(i32) = on_term;
    unsafe {
        libc::signal(libc::SIGTERM, handler as usize);
        libc::signal(libc::SIGINT, handler as usize);
    }
}

fn daemonize() -> Result<()> {
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        bail!("fork failed");
    }
    if pid > 0 {
        std::process::exit(0);
    }
    unsafe {
        libc::setsid();
    }
    Ok(())
}

/// Stop the wallpaper on `output` (or every running instance).
pub fn stop(output: Option<&str>) -> Result<()> {
    let mut killed = 0;
    let files: Vec<PathBuf> = match output {
        Some(_) => vec![pidfile_for(output)],
        None => std::fs::read_dir(pidfile_dir())
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "pid"))
            .collect(),
    };
    for f in files {
        if let Ok(s) = std::fs::read_to_string(&f)
            && let Ok(pid) = s.trim().parse::<i32>()
        {
            unsafe {
                libc::kill(pid, libc::SIGTERM);
            }
            std::fs::remove_file(&f).ok();
            killed += 1;
        }
    }
    if killed == 0 {
        bail!("no running wallpaper found");
    }
    Ok(())
}
