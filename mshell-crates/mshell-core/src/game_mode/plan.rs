//! Game Mode's decisions, GTK- and D-Bus-free: given what game mode holds
//! now, the wanted config and the live state of the session, the steps that
//! bring the session to the wanted state.
//!
//! [`reconcile`] runs on every `config.game_mode` change, not only on the
//! on / off flip (editing *what* the mode affects while it is on re-applies
//! it). So every engagement is recorded in [`Held`] with the state it
//! replaced: engaging twice is a no-op, and releasing puts back exactly what
//! was there before — never more. One-shot actions (pausing media, clearing
//! popups, the user's hooks) fire only on the flip itself.

use mshell_config::schema::config::GameMode;

/// The compositor-effect overrides the `gamemode.conf` fragment carries.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Fragment {
    pub(crate) animations_off: bool,
    pub(crate) blur_off: bool,
    pub(crate) shadows_off: bool,
    pub(crate) tearing_on: bool,
}

impl Fragment {
    fn wanted(cfg: &GameMode, on: bool) -> Self {
        Self {
            animations_off: on && cfg.disable_animations,
            blur_off: on && cfg.disable_blur,
            shadows_off: on && cfg.disable_shadows,
            tearing_on: on && cfg.allow_tearing,
        }
    }
}

/// What game mode currently holds, each with the state it replaced.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Held {
    /// Game mode was on at the last reconcile (for the one-shot actions).
    pub(crate) active: bool,
    /// The fragment last written (`None` = never written this session).
    pub(crate) fragment: Option<Fragment>,
    /// Do Not Disturb before game mode turned it on (`Some` = engaged).
    pub(crate) dnd_before: Option<bool>,
    /// Idle inhibitor before game mode took it (`Some` = engaged).
    pub(crate) idle_before: Option<bool>,
    /// `(profile before, profile game mode set)` while engaged.
    pub(crate) profile: Option<(String, String)>,
    pub(crate) bars_hidden: bool,
    pub(crate) dock_hidden: bool,
    pub(crate) osd_suppressed: bool,
    pub(crate) wallpaper_suspended: bool,
}

impl Held {
    pub(crate) const fn new() -> Self {
        Self {
            active: false,
            fragment: None,
            dnd_before: None,
            idle_before: None,
            profile: None,
            bars_hidden: false,
            dock_hidden: false,
            osd_suppressed: false,
            wallpaper_suspended: false,
        }
    }
}

/// The session state game mode reads before acting.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Live {
    pub(crate) dnd: bool,
    pub(crate) idle_held: bool,
    /// The active power profile; `None` without power-profiles-daemon.
    pub(crate) profile: Option<String>,
}

/// One change to make.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Step {
    /// Rewrite `gamemode.conf` and reload margo.
    WriteFragment(Fragment),
    Dnd(bool),
    IdleInhibit(bool),
    PowerProfile(String),
    PauseMedia,
    DismissPopups,
    HideBars(bool),
    HideDock(bool),
    SuppressOsd(bool),
    SuspendWallpaper(bool),
    /// A user hook command line, for `sh -c`.
    Hook(String),
}

/// The power-profiles-daemon name for a configured profile (`None` = leave
/// the profile alone: empty or not a profile the daemon knows).
pub(crate) fn profile_name(raw: &str) -> Option<&'static str> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "performance" => Some("performance"),
        "balanced" => Some("balanced"),
        "power-saver" | "power_saver" | "powersaver" => Some("power-saver"),
        _ => None,
    }
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

/// A switch that game mode alone owns (no prior state to remember).
fn flag(held: &mut bool, want: bool, step: fn(bool) -> Step, out: &mut Vec<Step>) {
    if *held != want {
        *held = want;
        out.push(step(want));
    }
}

fn power(
    held: &mut Option<(String, String)>,
    target: Option<&str>,
    live: Option<&str>,
    out: &mut Vec<Step>,
) {
    match (target, held.as_mut()) {
        // No daemon: nothing to set, nothing to remember.
        (Some(_), None) if live.is_none() => {}
        (Some(target), None) => {
            let before = live.unwrap_or_default().to_string();
            if before != target {
                out.push(Step::PowerProfile(target.to_string()));
            }
            *held = Some((before, target.to_string()));
        }
        // The profile was changed in Settings while active.
        (Some(target), Some((_, ours))) if ours.as_str() != target => {
            out.push(Step::PowerProfile(target.to_string()));
            *ours = target.to_string();
        }
        (None, Some(_)) => {
            if let Some((before, ours)) = held.take()
                // Picked another profile by hand meanwhile: that choice wins.
                && live == Some(ours.as_str())
                && before != ours
            {
                out.push(Step::PowerProfile(before));
            }
        }
        _ => {}
    }
}

/// The steps from `held` to `cfg` (game mode `on` or off), updating `held`.
pub(crate) fn reconcile(held: &mut Held, cfg: &GameMode, on: bool, live: &Live) -> Vec<Step> {
    let mut out = Vec::new();
    let entering = on && !held.active;
    let leaving = !on && held.active;
    held.active = on;

    let fragment = Fragment::wanted(cfg, on);
    if held.fragment != Some(fragment) {
        held.fragment = Some(fragment);
        out.push(Step::WriteFragment(fragment));
    }
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
    let target = profile_name(&cfg.power_profile).filter(|_| on);
    power(&mut held.profile, target, live.profile.as_deref(), &mut out);
    flag(
        &mut held.bars_hidden,
        on && cfg.hide_bars,
        Step::HideBars,
        &mut out,
    );
    flag(
        &mut held.dock_hidden,
        on && cfg.hide_dock,
        Step::HideDock,
        &mut out,
    );
    flag(
        &mut held.osd_suppressed,
        on && cfg.disable_osd,
        Step::SuppressOsd,
        &mut out,
    );
    flag(
        &mut held.wallpaper_suspended,
        on && cfg.disable_wallpaper,
        Step::SuspendWallpaper,
        &mut out,
    );

    if entering {
        if cfg.pause_media {
            out.push(Step::PauseMedia);
        }
        if cfg.clear_notifications {
            out.push(Step::DismissPopups);
        }
    }
    let hook = if entering {
        cfg.on_enter.trim()
    } else if leaving {
        cfg.on_exit.trim()
    } else {
        ""
    };
    if !hook.is_empty() {
        out.push(Step::Hook(hook.to_string()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> GameMode {
        GameMode::default()
    }

    /// Everything the new switches add, on.
    fn full() -> GameMode {
        GameMode {
            power_profile: "performance".into(),
            pause_media: true,
            hide_bars: true,
            hide_dock: true,
            disable_osd: true,
            disable_wallpaper: true,
            clear_notifications: true,
            on_enter: " notify-send in ".into(),
            on_exit: "notify-send out".into(),
            ..cfg()
        }
    }

    fn balanced() -> Live {
        Live {
            profile: Some("balanced".into()),
            ..Live::default()
        }
    }

    fn engaged(profile: Option<&str>) -> Live {
        Live {
            dnd: true,
            idle_held: true,
            profile: profile.map(str::to_string),
        }
    }

    fn without_fragment(steps: Vec<Step>) -> Vec<Step> {
        steps
            .into_iter()
            .filter(|s| !matches!(s, Step::WriteFragment(_)))
            .collect()
    }

    fn sets_profile(steps: &[Step]) -> bool {
        steps.iter().any(|s| matches!(s, Step::PowerProfile(_)))
    }

    fn runs_hook(steps: &[Step]) -> bool {
        steps.iter().any(|s| matches!(s, Step::Hook(_)))
    }

    #[test]
    fn enter_and_exit_round_trip() {
        let mut held = Held::default();
        assert_eq!(
            without_fragment(reconcile(&mut held, &cfg(), true, &Live::default())),
            [Step::Dnd(true), Step::IdleInhibit(true)]
        );
        assert_eq!(
            without_fragment(reconcile(&mut held, &cfg(), false, &engaged(None))),
            [Step::Dnd(false), Step::IdleInhibit(false)]
        );
        assert_eq!(
            held,
            Held {
                fragment: Some(Fragment::default()),
                ..Held::default()
            }
        );
    }

    /// The bug: any `game_mode` edit while active re-ran the enter path,
    /// which re-captured "inhibitor held" (by game mode itself) as the state
    /// to restore — so exit never released it and the session could no
    /// longer idle, dim or lock until mshell restarted.
    #[test]
    fn re_applying_while_active_does_not_leak_the_idle_inhibitor() {
        let mut held = Held::default();
        reconcile(&mut held, &cfg(), true, &Live::default());
        let edited = GameMode {
            allow_tearing: true,
            ..cfg()
        };
        assert_eq!(
            without_fragment(reconcile(&mut held, &edited, true, &engaged(None))),
            []
        );
        assert_eq!(
            without_fragment(reconcile(&mut held, &edited, false, &engaged(None))),
            [Step::Dnd(false), Step::IdleInhibit(false)]
        );
    }

    /// Exit used to force DND off even when the user had it on before.
    #[test]
    fn exit_puts_back_what_was_there_before() {
        let mut held = Held::default();
        let before = engaged(None);
        assert_eq!(
            without_fragment(reconcile(&mut held, &cfg(), true, &before)),
            []
        );
        assert_eq!(
            without_fragment(reconcile(&mut held, &cfg(), false, &before)),
            []
        );
    }

    #[test]
    fn turning_a_switch_off_while_active_releases_just_that() {
        let mut held = Held::default();
        reconcile(&mut held, &full(), true, &balanced());
        let fewer = GameMode {
            enable_dnd: false,
            hide_bars: false,
            ..full()
        };
        assert_eq!(
            reconcile(&mut held, &fewer, true, &engaged(Some("performance"))),
            [Step::Dnd(false), Step::HideBars(false)],
            "no one-shot or hook re-fires"
        );
    }

    #[test]
    fn the_fragment_is_rewritten_only_when_it_changes() {
        let mut held = Held::default();
        let steps = reconcile(&mut held, &cfg(), true, &Live::default());
        assert!(matches!(steps[0], Step::WriteFragment(f) if f.blur_off && !f.tearing_on));
        // A power-profile edit must not reload margo.
        let edited = GameMode {
            power_profile: "balanced".into(),
            ..cfg()
        };
        let steps = reconcile(&mut held, &edited, true, &engaged(None));
        assert!(!steps.iter().any(|s| matches!(s, Step::WriteFragment(_))));
        let steps = reconcile(&mut held, &cfg(), false, &engaged(None));
        assert_eq!(steps[0], Step::WriteFragment(Fragment::default()));
    }

    #[test]
    fn every_new_switch_engages_on_enter_and_releases_on_exit() {
        let mut held = Held::default();
        assert_eq!(
            without_fragment(reconcile(&mut held, &full(), true, &balanced())),
            [
                Step::Dnd(true),
                Step::IdleInhibit(true),
                Step::PowerProfile("performance".into()),
                Step::HideBars(true),
                Step::HideDock(true),
                Step::SuppressOsd(true),
                Step::SuspendWallpaper(true),
                Step::PauseMedia,
                Step::DismissPopups,
                Step::Hook("notify-send in".into()),
            ]
        );
        assert_eq!(
            without_fragment(reconcile(
                &mut held,
                &full(),
                false,
                &engaged(Some("performance"))
            )),
            [
                Step::Dnd(false),
                Step::IdleInhibit(false),
                Step::PowerProfile("balanced".into()),
                Step::HideBars(false),
                Step::HideDock(false),
                Step::SuppressOsd(false),
                Step::SuspendWallpaper(false),
                Step::Hook("notify-send out".into()),
            ]
        );
        assert_eq!(held.profile, None);
    }

    #[test]
    fn the_new_switches_default_off() {
        let mut held = Held::default();
        let steps = without_fragment(reconcile(&mut held, &cfg(), true, &balanced()));
        assert_eq!(steps, [Step::Dnd(true), Step::IdleInhibit(true)]);
    }

    #[test]
    fn a_profile_picked_by_hand_during_the_game_is_kept() {
        let mut held = Held::default();
        reconcile(&mut held, &full(), true, &balanced());
        let steps = reconcile(&mut held, &full(), false, &engaged(Some("power-saver")));
        assert!(!sets_profile(&steps));
    }

    #[test]
    fn power_profile_edge_cases() {
        // Already on the target: nothing to set, nothing to restore.
        let mut held = Held::default();
        let perf = engaged(Some("performance"));
        assert!(!sets_profile(&reconcile(&mut held, &full(), true, &perf)));
        assert!(!sets_profile(&reconcile(&mut held, &full(), false, &perf)));

        // No power-profiles-daemon: skipped, never remembered.
        let mut held = Held::default();
        assert!(!sets_profile(&reconcile(
            &mut held,
            &full(),
            true,
            &Live::default()
        )));
        assert_eq!(held.profile, None);

        // Retargeted in Settings while active, then restored to the original.
        let mut held = Held::default();
        reconcile(&mut held, &full(), true, &balanced());
        let saver = GameMode {
            power_profile: "power-saver".into(),
            ..full()
        };
        assert!(
            reconcile(&mut held, &saver, true, &perf)
                .contains(&Step::PowerProfile("power-saver".into()))
        );
        let now = engaged(Some("power-saver"));
        assert!(
            reconcile(&mut held, &saver, false, &now)
                .contains(&Step::PowerProfile("balanced".into()))
        );
    }

    #[test]
    fn profile_names_are_the_daemons() {
        assert_eq!(profile_name(" Performance "), Some("performance"));
        assert_eq!(profile_name("power_saver"), Some("power-saver"));
        assert_eq!(profile_name("balanced"), Some("balanced"));
        assert_eq!(profile_name(""), None);
        assert_eq!(profile_name("turbo"), None);
    }

    #[test]
    fn hooks_fire_on_the_flip_only() {
        let mut held = Held::default();
        let only_hooks = GameMode {
            on_enter: "a".into(),
            on_exit: "  ".into(),
            ..cfg()
        };
        let live = Live::default();
        assert!(reconcile(&mut held, &only_hooks, true, &live).contains(&Step::Hook("a".into())));
        assert!(!runs_hook(&reconcile(&mut held, &only_hooks, true, &live)));
        assert!(
            !runs_hook(&reconcile(&mut held, &only_hooks, false, &live)),
            "blank exit hook"
        );
        // Off → off (startup while inactive): nothing but the fragment.
        let mut held = Held::default();
        assert_eq!(
            without_fragment(reconcile(&mut held, &full(), false, &balanced())),
            []
        );
    }
}
