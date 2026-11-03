//! Seek-bar plumbing shared by the MPRIS card ([`super::media_player`]) and
//! the MPD card ([`super::mpd_player`]).

use relm4::gtk::glib;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentSender, gtk};
use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

/// How long the seek bar must stay still before the seek is committed.
const SEEK_DEBOUNCE: Duration = Duration::from_millis(300);

/// Debounced seek: every value change is forwarded at once as a display-only
/// `preview(value)`, and the real seek `commit(value)` is sent once the bar
/// has been still for [`SEEK_DEBOUNCE`]. Returns the `value-changed` handler
/// so programmatic position updates can block it.
pub(super) fn setup_debounced_seek<C>(
    scale: &gtk::Scale,
    sender: &ComponentSender<C>,
    preview: fn(f64) -> C::Input,
    commit: fn(f64) -> C::Input,
) -> glib::SignalHandlerId
where
    C: Component,
{
    let pending_source: Rc<Cell<Option<glib::SourceId>>> = Rc::new(Cell::new(None));

    let sender = sender.clone();
    scale.connect_value_changed(move |scale| {
        // Cancel the previous debounced seek.
        if let Some(source_id) = pending_source.take() {
            source_id.remove();
        }

        let value = scale.value();

        // Immediately update the display so it doesn't jump while dragging.
        // Use the fallible sender, not `input()`: `value_changed` also fires
        // when the position is set programmatically, which can happen as the
        // component tears down — and `input()` *panics* if the runtime is gone.
        // From a glib callback that panic can't unwind (it aborts the whole
        // process), so swallow a send-after-shutdown instead.
        let _ = sender.input_sender().send(preview(value));

        let seek_sender = sender.clone();
        let pending = pending_source.clone();

        // Only actually seek after dragging stops. This timeout can fire after
        // the component is dropped, so it must not panic either.
        let source_id = glib::timeout_add_local_once(SEEK_DEBOUNCE, move || {
            pending.set(None);
            let _ = seek_sender.input_sender().send(commit(value));
        });

        pending_source.set(Some(source_id));
    })
}

/// Set the seek bar without it reporting the change as a user seek: the
/// `value-changed` handler from [`setup_debounced_seek`] is blocked around a
/// programmatic update.
pub(super) fn set_scale_silently(
    scale: &gtk::Scale,
    handler: Option<&glib::SignalHandlerId>,
    value: f64,
) {
    match handler {
        Some(handler) => {
            scale.block_signal(handler);
            scale.set_value(value);
            scale.unblock_signal(handler);
        }
        None => scale.set_value(value),
    }
}

/// Seek-bar position for `position` into a track of `length`, in `0.0..=1.0`;
/// `0.0` for a zero-length (unknown) track.
pub(super) fn seek_fraction(position: Duration, length: Duration) -> f64 {
    if length.is_zero() {
        0.0
    } else {
        (position.as_secs_f64() / length.as_secs_f64()).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fraction_is_position_over_length() {
        let f = seek_fraction(Duration::from_secs(30), Duration::from_secs(120));
        assert!((f - 0.25).abs() < 1e-9);
    }

    #[test]
    fn fraction_of_an_unknown_length_is_zero() {
        assert_eq!(seek_fraction(Duration::from_secs(5), Duration::ZERO), 0.0);
    }

    #[test]
    fn fraction_past_the_end_saturates() {
        assert_eq!(
            seek_fraction(Duration::from_secs(200), Duration::from_secs(120)),
            1.0
        );
    }
}
