//! When the greeter has been left alone.
//!
//! After [`blank_secs`] of no keyboard, pointer or click activity the greeter
//! paints itself black and forgets whatever was typed into the password field.
//! Anything at all wakes it — and the key that wakes it is consumed by the wake,
//! never delivered to the field it was waiting for, or the first letter of a
//! password would be eaten by a screen that was not there to receive it.
//!
//! This is not DPMS. margo owns the backlight; the greeter only owns its own
//! surface, and blanking it is what a login screen can honestly promise: a
//! machine left at the greeter does not sit there showing which user last logged
//! in, on which host, over their wallpaper.
//!
//! The counter ticks once a second rather than rescheduling a one-shot on every
//! keystroke: a `remove()` on a source that has already fired aborts, and a
//! greeter is the wrong process to learn that in.

/// atrium's default, and a sensible one: long enough that a slow password is
/// never interrupted, short enough that a walk to the kitchen covers it.
pub const DEFAULT_BLANK_SECS: u32 = 300;

/// Read `MLOGIND_BLANK_SECS`, which the session runner sets from
/// `[display] blank_timeout`. `0` disables blanking; so does a value that is not
/// a number, because a greeter that blanks on a typo in a config file is worse
/// than one that never blanks.
pub fn blank_secs() -> u32 {
    parse(std::env::var("MLOGIND_BLANK_SECS").ok().as_deref())
}

fn parse(raw: Option<&str>) -> u32 {
    match raw {
        None => DEFAULT_BLANK_SECS,
        Some(raw) => raw.trim().parse().unwrap_or(0),
    }
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_BLANK_SECS, parse};

    #[test]
    fn an_unset_variable_keeps_the_default() {
        // `--preview`, or a runner too old to set it.
        assert_eq!(parse(None), DEFAULT_BLANK_SECS);
    }

    #[test]
    fn the_runner_decides_when_it_speaks() {
        assert_eq!(parse(Some("60")), 60);
        assert_eq!(parse(Some(" 90 ")), 90);
    }

    #[test]
    fn zero_disables_blanking() {
        assert_eq!(parse(Some("0")), 0);
    }

    #[test]
    fn a_value_that_is_not_a_number_disables_it_rather_than_guessing() {
        // The alternative is a login screen that goes black at a moment nobody
        // configured, which reads as a crash.
        assert_eq!(parse(Some("")), 0);
        assert_eq!(parse(Some("five minutes")), 0);
        assert_eq!(parse(Some("-1")), 0);
    }
}
