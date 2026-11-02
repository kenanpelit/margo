//! When an active cast must wake the render loop on its own.
//!
//! Casts render from the repaint path, so any scene damage (which always
//! pings the repaint source) already gives every cast a chance to produce a
//! frame. A cast only needs its own wake-up when a tick could not deliver a
//! frame for a reason that new damage won't fix. Pure so the policy is
//! unit-testable without PipeWire.

use std::time::Duration;

/// Never retry sooner than this, even when the stream negotiated no frame
/// rate limit (`min_time_between_frames == 0`): a zero-length retry timer
/// would spin the event loop.
pub const CAST_RETRY_FLOOR: Duration = Duration::from_millis(4);

/// What one repaint tick did for one cast.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CastFrameOutcome {
    /// Not looked at this tick (inactive, target geometry empty, output
    /// missing). New damage or a target change will ping again.
    Skipped,
    /// Too soon after the previous frame; `check_time_and_schedule` armed
    /// its own redraw at the next allowed frame time.
    Throttled,
    /// A frame was rendered and queued.
    Rendered,
    /// Nothing changed since the last frame.
    NoDamage,
    /// There was damage, but PipeWire had no free buffer.
    NoBuffer,
    /// The stream is renegotiating its size.
    SizePending,
    /// Rendering failed; the next damage retries.
    Failed,
}

/// Absolute monotonic time at which the cast should wake the render loop
/// itself, or `None` to rely on damage pings (and, for `Throttled`, the
/// timer the throttle already armed).
///
/// Only `NoBuffer` and `SizePending` retry: the frame is owed but nothing
/// else will ask for it. The retry is never in the past (a past target
/// becomes a zero-length timer that re-runs the repaint at once, forever)
/// and honours the stream's frame interval.
pub fn cast_self_wake(
    outcome: CastFrameOutcome,
    now: Duration,
    last_frame_time: Duration,
    min_time_between_frames: Duration,
) -> Option<Duration> {
    match outcome {
        CastFrameOutcome::NoBuffer | CastFrameOutcome::SizePending => {
            let step = min_time_between_frames.max(CAST_RETRY_FLOOR);
            let next_allowed = if last_frame_time.is_zero() {
                Duration::ZERO
            } else {
                last_frame_time + min_time_between_frames
            };
            Some((now + step).max(next_allowed))
        }
        CastFrameOutcome::Skipped
        | CastFrameOutcome::Throttled
        | CastFrameOutcome::Rendered
        | CastFrameOutcome::NoDamage
        | CastFrameOutcome::Failed => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);
    const FRAME_60: Duration = Duration::from_micros(16_667);

    #[test]
    fn static_scene_does_not_self_schedule() {
        // Regression: a cast on a static scene re-armed a timer at
        // `last_frame_time + min`, which lies in the past once nothing
        // renders, so the repaint loop ran back to back forever.
        let now = Duration::from_secs(100);
        let last = now - Duration::from_secs(5);
        assert_eq!(
            cast_self_wake(CastFrameOutcome::NoDamage, now, last, FRAME_60),
            None
        );
        assert_eq!(
            cast_self_wake(CastFrameOutcome::Skipped, now, last, FRAME_60),
            None
        );
    }

    #[test]
    fn rendered_and_throttled_rely_on_damage_and_their_own_timer() {
        let now = Duration::from_secs(100);
        for outcome in [
            CastFrameOutcome::Rendered,
            CastFrameOutcome::Throttled,
            CastFrameOutcome::Failed,
        ] {
            assert_eq!(cast_self_wake(outcome, now, now, FRAME_60), None);
        }
    }

    #[test]
    fn missing_buffer_and_resize_retry_in_the_future() {
        let now = Duration::from_secs(100);
        let long_ago = now - Duration::from_secs(5);
        for outcome in [CastFrameOutcome::NoBuffer, CastFrameOutcome::SizePending] {
            // Long-idle cast: one frame interval from now, never in the past.
            assert_eq!(
                cast_self_wake(outcome, now, long_ago, FRAME_60),
                Some(now + FRAME_60)
            );
            // Never rendered yet.
            assert_eq!(
                cast_self_wake(outcome, now, Duration::ZERO, FRAME_60),
                Some(now + FRAME_60)
            );
            // No negotiated rate limit: the floor still prevents a spin.
            assert_eq!(
                cast_self_wake(outcome, now, long_ago, Duration::ZERO),
                Some(now + CAST_RETRY_FLOOR)
            );
            // A future frame boundary is honoured.
            let last = now + 30 * MS;
            assert_eq!(
                cast_self_wake(outcome, now, last, FRAME_60),
                Some(last + FRAME_60)
            );
        }
    }
}
