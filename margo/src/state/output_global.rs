//! The `wl_output` global of each output: advertised while the output
//! shows a picture of its own or of a mirror, withdrawn while it is
//! switched off and when it is unplugged (wlroots / sway behaviour).
//!
//! A withdrawn output keeps its wlr-output-management head (built from
//! `monitors`, not from this global), so Settings and wlr-randr can
//! switch it back on. A mirror target is out of the layout but lit, so
//! it stays advertised: screencopy, casts and output-bound clients keep
//! working on it.

use std::cell::RefCell;
use std::time::Duration;

use smithay::output::Output;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::reexports::wayland_server::backend::GlobalId;

use super::MargoState;

/// How long a withdrawn global lingers, disabled, before it is
/// destroyed. A client that binds it in the meantime (its registry
/// read raced the `global_remove`) still binds successfully instead of
/// being killed with a protocol error; the delay wayland-server's
/// `disable_global` docs ask for.
const REMOVE_GLOBAL_DELAY: Duration = Duration::from_secs(10);

/// The output's advertised `wl_output` global, if any (output user data).
#[derive(Default)]
struct OutputGlobal(RefCell<Option<GlobalId>>);

fn slot(output: &Output) -> &OutputGlobal {
    output.user_data().get_or_insert(OutputGlobal::default)
}

impl MargoState {
    /// Advertise `output`'s `wl_output` global. No-op while it already is.
    pub fn advertise_output(&mut self, output: &Output) {
        let mut global = slot(output).0.borrow_mut();
        if global.is_none() {
            *global = Some(output.create_global::<MargoState>(&self.display_handle));
        }
    }

    /// Stop advertising `output`'s `wl_output` global: bound clients get
    /// `global_remove`, new ones don't see it. The global itself is
    /// destroyed after [`REMOVE_GLOBAL_DELAY`]. No-op when it isn't
    /// advertised.
    pub fn withdraw_output(&mut self, output: &Output) {
        let Some(global) = slot(output).0.borrow_mut().take() else {
            return;
        };
        tracing::info!(output = %output.name(), "withdrawing wl_output");
        self.display_handle
            .disable_global::<MargoState>(global.clone());
        let removal = self.loop_handle.insert_source(
            Timer::from_duration(REMOVE_GLOBAL_DELAY),
            move |_, _, state| {
                state
                    .display_handle
                    .remove_global::<MargoState>(global.clone());
                TimeoutAction::Drop
            },
        );
        if let Err(e) = removal {
            // The global stays disabled (not advertised); only its memory
            // lingers.
            tracing::warn!("wl_output removal timer: {e}");
        }
    }
}
