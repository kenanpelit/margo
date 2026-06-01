//! EGL context + `wl_egl_window` for a wallpaper surface. `khronos-egl`
//! (dynamic libEGL) + `wayland-egl` for the native window. We bind the
//! desktop OpenGL API so libmpv's GL renderer can draw into our surface.

use anyhow::{Result, anyhow};
use khronos_egl as egl;
use std::ffi::CStr;
use std::os::raw::{c_char, c_void};
use wayland_client::backend::ObjectId;
use wayland_egl::WlEglSurface;

type Egl = egl::DynamicInstance<egl::EGL1_4>;

/// Shared EGL display + chosen config (one per process).
pub struct EglRoot {
    egl: Egl,
    display: egl::Display,
    config: egl::Config,
}

impl EglRoot {
    /// Open the EGL display for a Wayland `wl_display` pointer and pick an
    /// RGBA8 window config.
    pub fn new(wl_display: *mut c_void) -> Result<Box<Self>> {
        let egl = unsafe { egl::DynamicInstance::<egl::EGL1_4>::load_required() }
            .map_err(|e| anyhow!("loading libEGL failed: {e}"))?;
        let display = unsafe { egl.get_display(wl_display) }
            .ok_or_else(|| anyhow!("eglGetDisplay returned no display"))?;
        egl.initialize(display)
            .map_err(|e| anyhow!("eglInitialize failed: {e}"))?;
        egl.bind_api(egl::OPENGL_API)
            .map_err(|e| anyhow!("eglBindAPI(OpenGL) failed: {e}"))?;

        let attribs = [
            egl::SURFACE_TYPE,
            egl::WINDOW_BIT,
            egl::RENDERABLE_TYPE,
            egl::OPENGL_BIT,
            egl::RED_SIZE,
            8,
            egl::GREEN_SIZE,
            8,
            egl::BLUE_SIZE,
            8,
            egl::ALPHA_SIZE,
            8,
            egl::NONE,
        ];
        let config = egl
            .choose_first_config(display, &attribs)
            .map_err(|e| anyhow!("eglChooseConfig failed: {e}"))?
            .ok_or_else(|| anyhow!("no matching EGL config"))?;

        Ok(Box::new(EglRoot {
            egl,
            display,
            config,
        }))
    }

    /// Resolve a GL/EGL function pointer (used by libmpv's render API).
    pub fn proc_address(&self, name: &str) -> *mut c_void {
        match self.egl.get_proc_address(name) {
            Some(f) => f as *mut c_void,
            None => std::ptr::null_mut(),
        }
    }
}

/// EGL window surface + context bound to one layer surface.
pub struct EglOutput {
    display: egl::Display,
    surface: egl::Surface,
    context: egl::Context,
    // Kept alive for the lifetime of the EGL surface (owns the
    // wl_egl_window the surface was created from).
    wl_egl: WlEglSurface,
}

impl EglOutput {
    pub fn new(root: &EglRoot, wl_surface_id: ObjectId, w: i32, h: i32) -> Result<Self> {
        let wl_egl = WlEglSurface::new(wl_surface_id, w.max(1), h.max(1))
            .map_err(|e| anyhow!("wl_egl_window create failed: {e}"))?;
        let surface = unsafe {
            root.egl.create_window_surface(
                root.display,
                root.config,
                wl_egl.ptr() as *mut c_void,
                None,
            )
        }
        .map_err(|e| anyhow!("eglCreateWindowSurface failed: {e}"))?;

        let ctx_attribs = [egl::CONTEXT_MAJOR_VERSION, 3, egl::NONE];
        let context = root
            .egl
            .create_context(root.display, root.config, None, &ctx_attribs)
            .map_err(|e| anyhow!("eglCreateContext failed: {e}"))?;

        Ok(EglOutput {
            display: root.display,
            surface,
            context,
            wl_egl,
        })
    }

    pub fn make_current(&self, root: &EglRoot) -> Result<()> {
        root.egl
            .make_current(
                self.display,
                Some(self.surface),
                Some(self.surface),
                Some(self.context),
            )
            .map_err(|e| anyhow!("eglMakeCurrent failed: {e}"))
    }

    pub fn swap_buffers(&self, root: &EglRoot) -> Result<()> {
        root.egl
            .swap_buffers(self.display, self.surface)
            .map_err(|e| anyhow!("eglSwapBuffers failed: {e}"))
    }

    pub fn resize(&self, w: i32, h: i32) {
        self.wl_egl.resize(w.max(1), h.max(1), 0, 0);
    }
}

/// Trampoline handed to libmpv: `ctx` is a `*const EglRoot`.
pub extern "C" fn mpv_get_proc_address(ctx: *mut c_void, name: *const c_char) -> *mut c_void {
    if ctx.is_null() || name.is_null() {
        return std::ptr::null_mut();
    }
    let root = unsafe { &*(ctx as *const EglRoot) };
    let name = unsafe { CStr::from_ptr(name) };
    match name.to_str() {
        Ok(n) => root.proc_address(n),
        Err(_) => std::ptr::null_mut(),
    }
}
