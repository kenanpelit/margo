//! Presentation-pacing barrier release for `wp_fifo_v1` +
//! `wp_commit_timing_v1` (road_map P15).
//!
//! Smithay's managed protocol states install commit blockers: a FIFO
//! `wait_barrier` commit stays queued until the previously set barrier is
//! signaled, and a timestamped commit stays queued until its target time is
//! reached. This module is the scheduler that signals those barriers — the
//! piece whose absence forced the globals to be withdrawn (a hidden-then-shown
//! Chromium surface stalled behind a barrier nobody ever released).
//!
//! Release points, mirroring anvil's `pre_repaint`/`post_repaint` split but
//! adapted to margo's per-output frame clock:
//!
//! * **Per-output present** ([`MargoState::release_pacing_barriers`], called
//!   from the tail of `send_frame_callbacks`): every real vblank, empty
//!   present, and estimated vblank releases the FIFO barrier of each surface
//!   presented on that output and signals commit timers up to the *next*
//!   present deadline (`now + refresh`), so a frame targeted at the upcoming
//!   vblank is unblocked in time to be latched into it.
//! * **Hidden-surface fallback** ([`MargoState::release_hidden_pacing_barriers`],
//!   called from the frame-callback fallback timer): windows not rendered on
//!   any output — hidden tags, disabled outputs, behind a session lock — get
//!   their barriers released at the fallback cadence (1 s, 30 Hz during the
//!   hidden-map warm-up window). FIFO pacing degrades to the fallback rate
//!   instead of stalling the client's commit queue outright.
//! * **Deadline wake** ([`MargoState::schedule_commit_timer_wake`], armed from
//!   the surface pre-commit hook): a timestamped commit on an otherwise idle
//!   output has no present or fallback due soon, so a one-shot timer fires a
//!   [`MargoState::release_commit_timers_until`] pass at the exact deadline.
//!   The timer follows the house ownership-generation pattern — logically
//!   cancelled, never `LoopHandle::remove`d.
//!
//! DPMS-off outputs intentionally behave like frame callbacks do today: their
//! surfaces pause until the display wakes, at which point the resume repaint
//! releases everything due.

use std::collections::HashMap;

use smithay::{
    desktop::utils::with_surfaces_surface_tree,
    input::pointer::CursorImageStatus,
    output::Output,
    reexports::{
        calloop::timer::{TimeoutAction, Timer},
        wayland_server::{Client, Resource, backend::ClientId},
    },
    utils::{Monotonic, Time},
    wayland::{
        commit_timing::{CommitTimerBarrierStateUserData, Timestamp},
        compositor::{CompositorHandler, SurfaceData},
        fifo::FifoBarrierCachedState,
    },
};
use wayland_server::protocol::wl_surface::WlSurface;

use super::MargoState;
use smithay::desktop::layer_map_for_output;

impl MargoState {
    /// Release pacing barriers for every surface presented on `output`.
    /// Runs at present time from the tail of `send_frame_callbacks`, in both
    /// frame-clock modes and on the empty-present path (a discarded content
    /// update still consumed its FIFO slot).
    pub fn release_pacing_barriers(&mut self, output: &Output) {
        // Commit-timer deadline: the *next* present. A commit targeted between
        // now and the upcoming vblank must be unblocked now so its content is
        // rendered into the frame presented at that vblank.
        let deadline: Timestamp = (self.clock.now() + Self::output_refresh_interval(output)).into();
        let mut clients: HashMap<ClientId, Client> = HashMap::new();

        if !self.session_locked {
            // Windows mapped on this output. `with_surfaces` covers the whole
            // tree: synchronized subsurfaces (Chromium video) and popups.
            let windows: Vec<_> = self
                .space
                .elements()
                .filter(|window| self.space.outputs_for_element(window).contains(output))
                .cloned()
                .collect();
            for window in windows {
                window.with_surfaces(|surface, states| {
                    release_surface_barriers(surface, states, deadline, &mut clients);
                });
            }

            // Every layer on the output, visible or suppressed: a suppressed
            // (exclusive-fullscreen / scroller-hidden) layer's content update
            // is discarded, and a discarded update must still release its
            // barrier or the layer client's commit queue wedges.
            {
                let map = layer_map_for_output(output);
                for layer in map.layers() {
                    layer.with_surfaces(|surface, states| {
                        release_surface_barriers(surface, states, deadline, &mut clients);
                    });
                }
                // Guard scope: drop the layer-map lock before
                // `blocker_cleared` re-enters the commit handler below.
            }
        }

        for (lock_output, lock_surface) in &self.lock_surfaces {
            if lock_output == output {
                with_surfaces_surface_tree(lock_surface.wl_surface(), |surface, states| {
                    release_surface_barriers(surface, states, deadline, &mut clients);
                });
            }
        }

        if let CursorImageStatus::Surface(surface) = &self.cursor_status
            && self
                .space
                .output_under((self.input_pointer.x, self.input_pointer.y))
                .next()
                == Some(output)
        {
            with_surfaces_surface_tree(surface, |surface, states| {
                release_surface_barriers(surface, states, deadline, &mut clients);
            });
        }

        self.notify_blocker_cleared(clients);
    }

    /// Release pacing barriers for clients the per-output present pass never
    /// visits (hidden tags, session lock, disabled outputs). Runs from the
    /// permanent frame-callback fallback timer, so FIFO clients degrade to the
    /// fallback cadence instead of deadlocking.
    pub fn release_hidden_pacing_barriers(&mut self) {
        let now: Timestamp = self.clock.now().into();
        let mut clients: HashMap<ClientId, Client> = HashMap::new();

        let hidden: Vec<_> = self
            .clients
            .iter()
            .filter(|client| {
                let rendered_now = if self.scroller_overview.is_some() {
                    self.monitors.get(client.monitor).is_some_and(|monitor| {
                        self.client_renders_on_output(client, &monitor.output)
                    })
                } else {
                    self.space
                        .outputs_for_element(&client.window)
                        .iter()
                        .any(|output| self.client_renders_on_output(client, output))
                };
                !rendered_now
            })
            .map(|client| client.window.clone())
            .collect();
        for window in hidden {
            window.with_surfaces(|surface, states| {
                release_surface_barriers(surface, states, now, &mut clients);
            });
        }

        self.notify_blocker_cleared(clients);
    }

    /// Arm (or keep) a one-shot wake at `deadline` for a commit-timing
    /// barrier. Called from the surface pre-commit hook — before Smithay's
    /// managed hook consumes the timestamp — so a timed commit on an idle
    /// output is released at its target time rather than waiting for the
    /// next unrelated present or the 1 s fallback.
    pub(crate) fn schedule_commit_timer_wake(&mut self, deadline: Timestamp) {
        if self
            .commit_timer_wake
            .is_some_and(|(armed, _)| armed <= deadline)
        {
            return;
        }
        self.next_commit_timer_wake_id = self.next_commit_timer_wake_id.wrapping_add(1).max(1);
        let wake_id = self.next_commit_timer_wake_id;

        let delay = Time::<Monotonic>::elapsed(&self.clock.now(), deadline.into());
        let timer = Timer::from_duration(delay);
        match self.loop_handle.insert_source(timer, move |_, _, state| {
            // A newer wake replaces ownership; the superseded one-shot fires
            // once, sees the mismatched id, and no-ops.
            if state
                .commit_timer_wake
                .is_some_and(|(_, armed_id)| armed_id == wake_id)
            {
                state.commit_timer_wake = None;
                let now: Timestamp = state.clock.now().into();
                state.release_commit_timers_until(now);
            }
            TimeoutAction::Drop
        }) {
            Ok(_token) => self.commit_timer_wake = Some((deadline, wake_id)),
            Err(error) => {
                // No wake will fire; the 1 s fallback pass and the next
                // present remain as bounded safety nets.
                tracing::warn!(?error, "commit-timer wake insert failed");
            }
        }
    }

    /// Signal every commit-timing barrier at or before `deadline` across all
    /// known surface trees, then re-arm the wake for the earliest barrier
    /// still pending.
    pub(crate) fn release_commit_timers_until(&mut self, deadline: Timestamp) {
        let mut clients: HashMap<ClientId, Client> = HashMap::new();
        let mut next: Option<Timestamp> = None;

        let windows: Vec<_> = self
            .clients
            .iter()
            .map(|client| client.window.clone())
            .collect();
        for window in windows {
            window.with_surfaces(|surface, states| {
                release_surface_commit_timers(surface, states, deadline, &mut clients, &mut next);
            });
        }

        let outputs: Vec<_> = self.space.outputs().cloned().collect();
        for output in outputs {
            let map = layer_map_for_output(&output);
            for layer in map.layers() {
                layer.with_surfaces(|surface, states| {
                    release_surface_commit_timers(
                        surface,
                        states,
                        deadline,
                        &mut clients,
                        &mut next,
                    );
                });
            }
        }

        for (_, lock_surface) in &self.lock_surfaces {
            with_surfaces_surface_tree(lock_surface.wl_surface(), |surface, states| {
                release_surface_commit_timers(surface, states, deadline, &mut clients, &mut next);
            });
        }
        if let CursorImageStatus::Surface(surface) = &self.cursor_status {
            with_surfaces_surface_tree(surface, |surface, states| {
                release_surface_commit_timers(surface, states, deadline, &mut clients, &mut next);
            });
        }

        self.notify_blocker_cleared(clients);
        if let Some(next_deadline) = next {
            self.schedule_commit_timer_wake(next_deadline);
        }
    }

    /// Re-pump the transaction queue of every client that had a barrier
    /// signaled, so the unblocked commits are applied this loop iteration.
    fn notify_blocker_cleared(&mut self, clients: HashMap<ClientId, Client>) {
        let dh = self.display_handle.clone();
        for client in clients.into_values() {
            self.client_compositor_state(&client)
                .blocker_cleared(self, &dh);
        }
    }
}

/// Signal one surface's FIFO barrier and its due commit timers.
fn release_surface_barriers(
    surface: &WlSurface,
    states: &SurfaceData,
    deadline: Timestamp,
    clients: &mut HashMap<ClientId, Client>,
) {
    let mut released = false;

    let fifo_barrier = states
        .cached_state
        .get::<FifoBarrierCachedState>()
        .current()
        .barrier
        .take();
    if let Some(barrier) = fifo_barrier {
        barrier.signal();
        released = true;
    }

    // Poisoning is unreachable on the single-threaded event loop; skipping a
    // poisoned lock only defers release to the next pass.
    if let Some(mut timer_state) = states
        .data_map
        .get::<CommitTimerBarrierStateUserData>()
        .and_then(|timers| timers.lock().ok())
    {
        released |= timer_state.signal_until(deadline);
    }

    if released && let Some(client) = surface.client() {
        clients.insert(client.id(), client);
    }
}

/// Signal only due commit timers and track the earliest remaining deadline.
fn release_surface_commit_timers(
    surface: &WlSurface,
    states: &SurfaceData,
    deadline: Timestamp,
    clients: &mut HashMap<ClientId, Client>,
    next: &mut Option<Timestamp>,
) {
    let Some(mut timer_state) = states
        .data_map
        .get::<CommitTimerBarrierStateUserData>()
        .and_then(|timers| timers.lock().ok())
    else {
        return;
    };

    if timer_state.signal_until(deadline)
        && let Some(client) = surface.client()
    {
        clients.insert(client.id(), client);
    }
    if let Some(pending) = timer_state.next_deadline() {
        *next = Some(match *next {
            Some(current) if current <= pending => current,
            _ => pending,
        });
    }
}
