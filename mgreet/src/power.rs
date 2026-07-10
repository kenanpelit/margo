//! Power actions — the greeter's F-key footer, mirrored from mlogind's TUI
//! (`ui/key_menu.rs`).
//!
//! The greeter used to run the command itself. It cannot any more: it is an
//! unprivileged system user, and `systemctl poweroff` is not its to run. It sends
//! `Request::Power { index }` to the root session runner instead, which resolves
//! the index against its own `power_controls` and runs what is written there.
//!
//! An index, never a command. Letting the greeter name what root executes would
//! hand back, in one line, exactly the privilege that deprivileging it took away.
//! So the runner serialises `MLOGIND_POWER` as `index<TAB>key<TAB>hint` — the
//! command is not in there at all.

#[derive(Clone)]
pub struct PowerAction {
    /// The trigger key name as GTK reports it, e.g. "F1".
    pub key: String,
    /// Short label, e.g. "Shutdown".
    pub hint: String,
    /// Position in the runner's resolved action list. This is the wire value.
    pub index: u32,
}

/// Parse `MLOGIND_POWER` (`index\tkey\thint` per line). Falls back to a sensible
/// built-in set when the env var is absent (preview / bare run), so the footer is
/// never empty — though preview never actually triggers anything.
pub fn from_env() -> Vec<PowerAction> {
    let parsed = parse_power(&std::env::var("MLOGIND_POWER").unwrap_or_default());
    if parsed.is_empty() {
        defaults()
    } else {
        parsed
    }
}

/// Parse the `MLOGIND_POWER` payload — one `index<TAB>key<TAB>hint` line per
/// action. Split from the env read so it is testable.
///
/// The index is explicit rather than implied by line order: the runner skips
/// entries with a blank key or hint when it writes this, but they still occupy a
/// slot in the list `Request::Power` indexes into. Inferring it from the line
/// number would silently shift every action after a blank one.
fn parse_power(raw: &str) -> Vec<PowerAction> {
    raw.lines()
        .filter_map(|line| {
            let mut f = line.splitn(3, '\t');
            match (f.next(), f.next(), f.next()) {
                (Some(i), Some(k), Some(h)) if !k.is_empty() && !h.is_empty() => {
                    Some(PowerAction {
                        index: i.parse().ok()?,
                        key: k.to_string(),
                        hint: h.to_string(),
                    })
                }
                _ => None,
            }
        })
        .collect()
}

/// What a `power_controls`-less mlogind would show. The indices match the order
/// the runner's own defaults land in, so a preview footer reads like the real one.
fn defaults() -> Vec<PowerAction> {
    [
        (0, "F1", "Shutdown"),
        (1, "F2", "Reboot"),
        (2, "F3", "Suspend"),
    ]
    .into_iter()
    .map(|(index, key, hint)| PowerAction {
        index,
        key: key.to_string(),
        hint: hint.to_string(),
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tab_separated_actions() {
        let a = parse_power("0\tF1\tShutdown\n1\tF2\tReboot");
        assert_eq!(a.len(), 2);
        assert_eq!(
            (a[0].index, a[0].key.as_str(), a[0].hint.as_str()),
            (0, "F1", "Shutdown")
        );
        assert_eq!((a[1].index, a[1].key.as_str()), (1, "F2"));
    }

    #[test]
    fn a_gap_in_the_indices_is_preserved() {
        // The runner skipped a blank entry at slot 1. Renumbering here would make
        // F2 reboot the machine when the user asked it to suspend.
        let a = parse_power("0\tF1\tShutdown\n2\tF2\tSuspend");
        assert_eq!(a[1].index, 2);
    }

    #[test]
    fn a_line_missing_a_field_is_dropped() {
        assert!(parse_power("0\tF1").is_empty());
        assert!(parse_power("F1\tShutdown").is_empty());
    }

    #[test]
    fn a_non_numeric_index_is_dropped_rather_than_defaulted_to_zero() {
        // Slot zero is "shut down" in every stock config. Never guess it.
        assert!(parse_power("x\tF1\tShutdown").is_empty());
        assert!(parse_power("\tF1\tShutdown").is_empty());
    }

    #[test]
    fn blank_key_or_hint_is_rejected() {
        assert!(parse_power("0\t\tShutdown").is_empty());
        assert!(parse_power("0\tF1\t").is_empty());
    }

    #[test]
    fn a_hint_keeps_embedded_tabs() {
        // splitn(3) stops after the third field.
        let a = parse_power("0\tF1\tShut\tdown");
        assert_eq!(a[0].hint, "Shut\tdown");
    }

    #[test]
    fn empty_input_parses_to_nothing() {
        assert!(parse_power("").is_empty());
    }

    #[test]
    fn defaults_provide_the_standard_three_in_order() {
        let a = defaults();
        assert_eq!(a.len(), 3);
        assert_eq!((a[0].index, a[0].hint.as_str()), (0, "Shutdown"));
        assert_eq!((a[1].index, a[1].hint.as_str()), (1, "Reboot"));
        assert_eq!((a[2].index, a[2].hint.as_str()), (2, "Suspend"));
    }
}
