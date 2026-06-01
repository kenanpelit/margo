//! libmpv render-context wrapper: create an mpv instance wired to our EGL
//! context, load the source, and render frames into the layer surface.

use super::egl::{EglOutput, EglRoot, mpv_get_proc_address};
use super::mpv_sys as m;
use crate::paper::PaperOpts;
use anyhow::{Result, anyhow, bail};
use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_void};
use std::os::unix::io::RawFd;
use std::ptr;

/// One mpv instance + its OpenGL render context.
pub struct MpvVideo {
    mpv: *mut m::mpv_handle,
    rc: *mut m::mpv_render_context,
}

/// mpv's render-update callback: wake the run loop by writing to the
/// eventfd we stashed as the callback context. Runs on an mpv thread, so
/// it must stay async-signal/thread-safe — a single `write` is fine.
extern "C" fn on_mpv_update(ctx: *mut c_void) {
    let fd = ctx as isize as RawFd;
    if fd >= 0 {
        let v: u64 = 1;
        unsafe {
            libc::write(fd, &v as *const u64 as *const c_void, 8);
        }
    }
}

fn set_opt(mpv: *mut m::mpv_handle, name: &str, val: &str) -> Result<()> {
    let cn = CString::new(name)?;
    let cv = CString::new(val)?;
    let rc = unsafe { m::mpv_set_option_string(mpv, cn.as_ptr(), cv.as_ptr()) };
    if rc < 0 {
        bail!(
            "mpv_set_option_string({name}={val}) failed: {}",
            err_str(rc)
        );
    }
    Ok(())
}

fn err_str(code: c_int) -> String {
    unsafe {
        let p = m::mpv_error_string(code);
        if p.is_null() {
            return format!("mpv error {code}");
        }
        std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
    }
}

impl MpvVideo {
    /// Build an mpv render context bound to `egl_root` (passed as the GL
    /// proc-address ctx) and `wl_display`; load `src`. `update_fd` is an
    /// eventfd the loop polls — mpv writes to it when a frame is ready.
    pub fn new(
        egl_root_ptr: *mut c_void,
        wl_display: *mut c_void,
        src: &str,
        opts: &PaperOpts,
        update_fd: RawFd,
    ) -> Result<Self> {
        let mpv = unsafe { m::mpv_create() };
        if mpv.is_null() {
            bail!("mpv_create returned null");
        }

        // Render-API requires the libmpv video output.
        set_opt(mpv, "vo", "libmpv")?;
        set_opt(mpv, "hwdec", "auto-safe")?;
        set_opt(mpv, "loop-file", if opts.looping { "inf" } else { "no" })?;
        set_opt(mpv, "mute", if opts.mute { "yes" } else { "no" })?;
        // No window chrome / input — this is a wallpaper.
        set_opt(mpv, "osc", "no")?;
        set_opt(mpv, "input-default-bindings", "no")?;
        set_opt(mpv, "input-vo-keyboard", "no")?;
        for (k, v) in opts.scale.mpv_opts() {
            set_opt(mpv, k, v)?;
        }

        let rc = unsafe { m::mpv_initialize(mpv) };
        if rc < 0 {
            unsafe { m::mpv_terminate_destroy(mpv) };
            bail!("mpv_initialize failed: {}", err_str(rc));
        }

        // Render context bound to our GL proc loader + the wl_display.
        let mut init = m::mpv_opengl_init_params {
            get_proc_address: Some(mpv_get_proc_address),
            get_proc_address_ctx: egl_root_ptr,
        };
        let mut api_type = m::MPV_RENDER_API_TYPE_OPENGL.as_ptr() as *mut c_void;
        let mut params = [
            m::mpv_render_param {
                type_: m::MPV_RENDER_PARAM_API_TYPE,
                data: &mut api_type as *mut _ as *mut c_void,
            },
            m::mpv_render_param {
                type_: m::MPV_RENDER_PARAM_OPENGL_INIT_PARAMS,
                data: &mut init as *mut _ as *mut c_void,
            },
            m::mpv_render_param {
                type_: m::MPV_RENDER_PARAM_WL_DISPLAY,
                data: wl_display,
            },
            m::mpv_render_param {
                type_: m::MPV_RENDER_PARAM_INVALID,
                data: ptr::null_mut(),
            },
        ];
        let mut rc_ctx: *mut m::mpv_render_context = ptr::null_mut();
        let res = unsafe { m::mpv_render_context_create(&mut rc_ctx, mpv, params.as_mut_ptr()) };
        if res < 0 || rc_ctx.is_null() {
            unsafe { m::mpv_terminate_destroy(mpv) };
            bail!("mpv_render_context_create failed: {}", err_str(res));
        }

        unsafe {
            m::mpv_render_context_set_update_callback(
                rc_ctx,
                on_mpv_update,
                update_fd as isize as *mut c_void,
            );
        }

        let video = MpvVideo { mpv, rc: rc_ctx };
        video.loadfile(src)?;
        Ok(video)
    }

    fn loadfile(&self, src: &str) -> Result<()> {
        let load = CString::new("loadfile")?;
        let target = CString::new(src)?;
        let mut argv: [*const c_char; 3] = [load.as_ptr(), target.as_ptr(), ptr::null()];
        let rc = unsafe { m::mpv_command(self.mpv, argv.as_mut_ptr()) };
        if rc < 0 {
            bail!("mpv loadfile failed: {}", err_str(rc));
        }
        Ok(())
    }

    /// Render the current frame into FBO 0 of the (already-current) EGL
    /// surface and present it.
    pub fn render(&self, egl: &EglOutput, root: &EglRoot, w: i32, h: i32) -> Result<()> {
        egl.make_current(root)?;
        let mut fbo = m::mpv_opengl_fbo {
            fbo: 0,
            w,
            h,
            internal_format: 0,
        };
        let mut flip: c_int = 1;
        let mut params = [
            m::mpv_render_param {
                type_: m::MPV_RENDER_PARAM_OPENGL_FBO,
                data: &mut fbo as *mut _ as *mut c_void,
            },
            m::mpv_render_param {
                type_: m::MPV_RENDER_PARAM_FLIP_Y,
                data: &mut flip as *mut _ as *mut c_void,
            },
            m::mpv_render_param {
                type_: m::MPV_RENDER_PARAM_INVALID,
                data: ptr::null_mut(),
            },
        ];
        let rc = unsafe { m::mpv_render_context_render(self.rc, params.as_mut_ptr()) };
        if rc < 0 {
            return Err(anyhow!("mpv_render_context_render failed: {}", err_str(rc)));
        }
        egl.swap_buffers(root)?;
        Ok(())
    }
}

impl Drop for MpvVideo {
    fn drop(&mut self) {
        // Free the render context first (must happen before the mpv
        // handle is destroyed), then terminate the player.
        unsafe {
            if !self.rc.is_null() {
                m::mpv_render_context_free(self.rc);
            }
            if !self.mpv.is_null() {
                m::mpv_terminate_destroy(self.mpv);
            }
        }
    }
}
