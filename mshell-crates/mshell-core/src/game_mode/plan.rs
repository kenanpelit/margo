//! Game Mode's decisions, GTK- and D-Bus-free: given what game mode holds
//! now, the wanted config and the live state of the session, the steps that
//! bring the session to the wanted state.
//!
//! [`reconcile`] runs on every `config.game_mode` change, not only on the
//! on / off flip (editing *what* the mode affects while it is on re-applies
//! it). So every engagement is recorded in [`Held`] with the state it
//! replaced: engaging twice is a no-op, and releasing puts back exactly what
//! was there before — never more.

use mshell_config::schema::config::GameMode;

/// What game mode currently holds, each with the state it replaced.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Held {
    /// Do Not Disturb before game mode turned it on (`Some` = engaged).
    pub(crate) dnd_before: Option<bool>,
    /// Idle inhibitor before game mode took it (`Some` = engaged).
    pub(crate) idle_before: Option<bool>,
}

/// The session state game mode reads before acting.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Live {
    pub(crate) dnd: bool,
    pub(crate) idle_held: bool,
}

/// One change to make.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Step {
    Dnd(bool),
    IdleInhibit(bool),
}

/// Engage or release one remembered setting: on engage remember `live` and
/// switch to `engaged` if it differs; on release put the remembered value
/// back if it differs from `engaged`.
fn toggle(
    slot: &mut Option<bool>,
    want: bool,
    live: bool,
    engaged: bool,
    step: fn(bool) -> Step,
    out: &mut Vec<Step>,
) {
    match (want, *slot) {
        (true, None) => {
            *slot = Some(live);
            if live != engaged {
                out.push(step(engaged));
            }
        }
        (false, Some(before)) => {
            *slot = None;
            if before != engaged {
                out.push(step(before));
            }
        }
        _ => {}
    }
}

/// The steps from `held` to `cfg` (game mode `on` or off), updating `held`.
pub(crate) fn reconcile(held: &mut Held, cfg: &GameMode, on: bool, live: &Live) -> Vec<Step> {
    let mut out = Vec::new();
    toggle(
        &mut held.dnd_before,
        on && cfg.enable_dnd,
        live.dnd,
        true,
        Step::Dnd,
        &mut out,
    );
    toggle(
        &mut held.idle_before,
        on && cfg.inhibit_idle,
        live.idle_held,
        true,
        Step::IdleInhibit,
        &mut out,
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> GameMode {
        GameMode::default()
    }

    #[test]
    fn enter_and_exit_round_trip() {
        let mut held = Held::default();
        let live = Live::default();
        assert_eq!(
            reconcile(&mut held, &cfg(), true, &live),
            [Step::Dnd(true), Step::IdleInhibit(true)]
        );
        let live = Live {
            dnd: true,
            idle_held: true,
        };
        assert_eq!(
            reconcile(&mut held, &cfg(), false, &live),
            [Step::Dnd(false), Step::IdleInhibit(false)]
        );
        assert_eq!(held, Held::default());
    }

    /// The bug: any `game_mode` edit while active re-ran the enter path,
    /// which re-captured "inhibitor held" (by game mode itself) as the state
    /// to restore — so exit never released it and the session could no
    /// longer idle, dim or lock until mshell restarted.
    #[test]
    fn re_applying_while_active_does_not_leak_the_idle_inhibitor() {
        let mut held = Held::default();
        reconcile(&mut held, &cfg(), true, &Live::default());
        let during = Live {
            dnd: true,
            idle_held: true,
        };
        let edited = GameMode {
            allow_tearing: true,
            ..cfg()
        };
        assert_eq!(reconcile(&mut held, &edited, true, &during), []);
        assert_eq!(
            reconcile(&mut held, &edited, false, &during),
            [Step::Dnd(false), Step::IdleInhibit(false)]
        );
    }

    /// Exit used to force DND off even when the user had it on before.
    #[test]
    fn exit_puts_back_what_was_there_before() {
        let mut held = Held::default();
        let before = Live {
            dnd: true,
            idle_held: true,
        };
        assert_eq!(reconcile(&mut held, &cfg(), true, &before), []);
        assert_eq!(reconcile(&mut held, &cfg(), false, &before), []);
    }

    #[test]
    fn turning_a_switch_off_while_active_releases_just_that() {
        let mut held = Held::default();
        reconcile(&mut held, &cfg(), true, &Live::default());
        let no_dnd = GameMode {
            enable_dnd: false,
            ..cfg()
        };
        let live = Live {
            dnd: true,
            idle_held: true,
        };
        assert_eq!(
            reconcile(&mut held, &no_dnd, true, &live),
            [Step::Dnd(false)]
        );
    }
}
