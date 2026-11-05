use std::os::fd::{AsFd, FromRawFd, OwnedFd};

use anyhow::{Context, Result, bail};
use wayland_client::{
    Connection, Dispatch, EventQueue, QueueHandle, delegate_noop,
    protocol::{wl_output, wl_registry},
};
use wayland_protocols_wlr::gamma_control::v1::client::{
    zwlr_gamma_control_manager_v1, zwlr_gamma_control_v1,
};

use crate::{GammaState, TEMP_NEUTRAL, ramp::build_ramp};

struct Output {
    wl: wl_output::WlOutput,
    ctrl: Option<zwlr_gamma_control_v1::ZwlrGammaControlV1>,
    ramp_size: usize,
}

struct AppData {
    manager: Option<zwlr_gamma_control_manager_v1::ZwlrGammaControlManagerV1>,
    outputs: Vec<Output>,
}

impl AppData {
    fn new() -> Self {
        Self {
            manager: None,
            outputs: Vec::new(),
        }
    }
}

// ── Dispatch: registry ────────────────────────────────────────────────────────

impl Dispatch<wl_registry::WlRegistry, ()> for AppData {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        else {
            return;
        };

        match interface.as_str() {
            "zwlr_gamma_control_manager_v1" => {
                let mgr = registry
                    .bind::<zwlr_gamma_control_manager_v1::ZwlrGammaControlManagerV1, _, _>(
                        name,
                        version.min(1),
                        qh,
                        (),
                    );
                state.manager = Some(mgr);
            }
            "wl_output" => {
                let wl = registry.bind::<wl_output::WlOutput, _, _>(name, version.min(4), qh, ());
                state.outputs.push(Output {
                    wl,
                    ctrl: None,
                    ramp_size: 0,
                });
            }
            _ => {}
        }
    }
}

// ── Dispatch: gamma control manager (no events) ───────────────────────────────

impl Dispatch<zwlr_gamma_control_manager_v1::ZwlrGammaControlManagerV1, ()> for AppData {
    fn event(
        _: &mut Self,
        _: &zwlr_gamma_control_manager_v1::ZwlrGammaControlManagerV1,
        _: zwlr_gamma_control_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

// ── Dispatch: per-output gamma control ───────────────────────────────────────

impl Dispatch<zwlr_gamma_control_v1::ZwlrGammaControlV1, usize> for AppData {
    fn event(
        state: &mut Self,
        _: &zwlr_gamma_control_v1::ZwlrGammaControlV1,
        event: zwlr_gamma_control_v1::Event,
        idx: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_gamma_control_v1::Event::GammaSize { size } => {
                if let Some(o) = state.outputs.get_mut(*idx) {
                    o.ramp_size = size as usize;
                }
            }
            zwlr_gamma_control_v1::Event::Failed => {
                eprintln!("mshell-gamma: compositor rejected gamma control for output {idx}");
            }
            _ => {}
        }
    }
}

// wl_output: we don't need any events (name is nice-to-have but not required).
delegate_noop!(AppData: ignore wl_output::WlOutput);

pub struct GammaManager {
    _conn: Connection,
    queue: EventQueue<AppData>,
    data: AppData,
}

impl GammaManager {
    /// Connect to the Wayland display and prepare gamma controls for all outputs.
    ///
    /// Returns an error if the compositor does not advertise
    /// `zwlr_gamma_control_manager_v1`
    pub fn connect() -> Result<Self> {
        let conn = Connection::connect_to_env().context("failed to connect to Wayland display")?;
        let mut queue = conn.new_event_queue();
        let qh = queue.handle();

        conn.display().get_registry(&qh, ());

        let mut data = AppData::new();

        // Roundtrip 1: discover globals.
        queue
            .roundtrip(&mut data)
            .context("Wayland roundtrip 1 failed")?;

        let Some(ref mgr) = data.manager else {
            bail!(
                "compositor does not support zwlr_gamma_control_manager_v1\n\
                 (this protocol is available on wlroots compositors like Hyprland and Sway,\n\
                  but not on GNOME or KDE which manage gamma internally)"
            );
        };

        // Bind a gamma control for each output found in roundtrip 1.
        for (idx, output) in data.outputs.iter_mut().enumerate() {
            let ctrl = mgr.get_gamma_control(&output.wl, &qh, idx);
            output.ctrl = Some(ctrl);
        }

        // Roundtrip 2: collect gamma_size events.
        queue
            .roundtrip(&mut data)
            .context("Wayland roundtrip 2 failed")?;

        Ok(Self {
            _conn: conn,
            queue,
            data,
        })
    }

    /// Apply `state` to every output.
    pub fn apply(&mut self, state: &GammaState) -> Result<()> {
        let temp = if state.enabled {
            state.night_temp
        } else {
            TEMP_NEUTRAL
        };

        // Keep all fds alive in this Vec until after the roundtrip.
        // set_gamma() only queues the request; the compositor reads the fd
        // during the flush/roundtrip, so closing early causes a size mismatch.
        let mut _fds: Vec<OwnedFd> = Vec::new();

        for output in &self.data.outputs {
            if output.ramp_size == 0 {
                // gamma_size not yet received — skip; shouldn't happen after connect().
                continue;
            }
            let Some(ref ctrl) = output.ctrl else {
                continue;
            };

            let ramp = build_ramp(temp, 1.0, output.ramp_size);
            let fd = ramp_to_memfd(&ramp).context("failed to create memfd for gamma ramp")?;
            ctrl.set_gamma(fd.as_fd());
            _fds.push(fd);
        }

        self.queue
            .roundtrip(&mut self.data)
            .context("Wayland flush after set_gamma failed")?;

        // _fds drops here, after the roundtrip has completed.
        Ok(())
    }

    /// Apply a raw temperature in Kelvin directly to all outputs.
    pub fn apply_temp(&mut self, temp_k: u32) -> Result<()> {
        let mut _fds: Vec<OwnedFd> = Vec::new();

        for output in &self.data.outputs {
            if output.ramp_size == 0 {
                continue;
            }
            let Some(ref ctrl) = output.ctrl else {
                continue;
            };

            let ramp = build_ramp(temp_k, 1.0, output.ramp_size);
            let fd = ramp_to_memfd(&ramp).context("failed to create memfd for gamma ramp")?;
            ctrl.set_gamma(fd.as_fd());
            _fds.push(fd);
        }

        self.queue
            .roundtrip(&mut self.data)
            .context("Wayland flush after set_gamma failed")?;

        Ok(())
    }
}

// ── fd helper ────────────────────────────────────────────────────────────────

/// Write `ramp` into an anonymous memfd and return its fd.
///
/// The compositor checks the size (`ramp_size * 3 * sizeof(u16)`) and margo
/// reads the ramp with plain `read()` from the fd's current offset, so the
/// file is sized with `set_len` and filled with a positional write that
/// leaves the offset at 0.
///
/// The fd is owned from the moment it exists: an error on a later step
/// returns that step's `io::Error` and the drop closes the fd afterwards
/// (closing first would overwrite errno and report the wrong error).
fn ramp_to_memfd(ramp: &[u16]) -> Result<OwnedFd> {
    use std::os::unix::fs::FileExt;

    // SAFETY: the name is a valid NUL-terminated C string and the flags are
    // a plain bitmask; memfd_create has no other preconditions.
    let raw_fd = unsafe { libc::memfd_create(c"mshell-gamma".as_ptr(), libc::MFD_CLOEXEC) };
    if raw_fd < 0 {
        return Err(std::io::Error::last_os_error()).context("memfd_create");
    }
    // SAFETY: memfd_create just returned this fd and nothing else owns it.
    let fd = unsafe { OwnedFd::from_raw_fd(raw_fd) };

    let bytes: Vec<u8> = ramp.iter().flat_map(|v| v.to_ne_bytes()).collect();
    let file = std::fs::File::from(fd);
    file.set_len(bytes.len() as u64)
        .context("size the gamma ramp memfd")?;
    file.write_all_at(&bytes, 0)
        .context("write the gamma ramp memfd")?;
    Ok(OwnedFd::from(file))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::os::fd::AsRawFd;

    fn sample_ramp() -> Vec<u16> {
        (0..3 * 256u32).map(|i| (i * 85) as u16).collect()
    }

    #[test]
    fn the_ramp_fd_reads_back_from_offset_zero_at_its_exact_size() {
        // margo reads the ramp with plain read() from the fd's current
        // offset and rejects any size other than 3 * gamma_size u16s.
        let ramp = sample_ramp();
        let fd = ramp_to_memfd(&ramp).expect("ramp fd");
        let mut file = std::fs::File::from(fd);
        assert_eq!(
            file.metadata().expect("fstat").len(),
            std::mem::size_of_val(ramp.as_slice()) as u64
        );
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).expect("read");
        let back: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|b| u16::from_ne_bytes([b[0], b[1]]))
            .collect();
        assert_eq!(back, ramp);
    }

    #[test]
    fn the_ramp_lives_in_an_anonymous_memfd_not_a_tmp_file() {
        let fd = ramp_to_memfd(&sample_ramp()).expect("ramp fd");
        let link =
            std::fs::read_link(format!("/proc/self/fd/{}", fd.as_raw_fd())).expect("fd link");
        assert!(
            link.to_string_lossy().starts_with("/memfd:mshell-gamma"),
            "{}",
            link.display()
        );
    }
}
