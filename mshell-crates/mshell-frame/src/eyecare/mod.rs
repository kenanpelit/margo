//! The shell's 20-20-20 eye-care reminder — shared by the `EyeCare` bar
//! pill and `mshellctl eyecare`.
//!
//! Off by default: nothing runs (no thread, no timer) until
//! `config.eyecare.enabled`. While on, two things drive it:
//!
//! * a [`mshell_idle::presence`] watcher (its own `ext-idle-notify`
//!   notification, threshold `away_seconds`) that blocks on the Wayland
//!   socket and reports away / back — away pauses and resets the count;
//! * one one-shot glib timer armed for the next deadline (break due /
//!   break over). A re-arm bumps a generation counter instead of removing
//!   the old source, so a stale wake-up simply returns.
//!
//! State is main-thread only and transient (never persisted).

pub mod engine;

use engine::{Event, EyeCare, Mode, Settings};
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, EyeCare as EyeCareConfig};
use mshell_idle::presence::{Presence, watch_presence};
use mshell_process::desktop::{Notice, notify};
use reactive_graph::effect::Effect;
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::glib;
use std::cell::{Cell, RefCell};
use std::sync::LazyLock;
use std::time::{Duration, Instant};
use tokio::sync::watch;

struct Service {
    engine: EyeCare,
    /// `None` when the compositor has no `ext-idle-notify` (then all time
    /// counts as active).
    presence: Option<Presence>,
}

thread_local! {
    static SERVICE: RefCell<Option<Service>> = const { RefCell::new(None) };
    static GENERATION: Cell<u64> = const { Cell::new(0) };
}

/// What a subscriber needs to schedule its repaint off the GTK thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Published {
    pub(crate) generation: u64,
    pub(crate) enabled: bool,
    /// The mode's deadline: break due (active) or break over (on break).
    pub(crate) deadline: Option<Instant>,
    pub(crate) on_break: bool,
}

static CHANGES: LazyLock<watch::Sender<Published>> =
    LazyLock::new(|| watch::channel(Published::default()).0);

pub(crate) fn changes() -> watch::Receiver<Published> {
    CHANGES.subscribe()
}

fn notify_changed() {
    let (enabled, deadline, on_break) = SERVICE.with(|s| match s.borrow().as_ref() {
        Some(svc) => (
            true,
            svc.engine.next_deadline(),
            matches!(svc.engine.mode(Instant::now()), Mode::Break { .. }),
        ),
        None => (false, None, false),
    });
    CHANGES.send_modify(|p| {
        p.generation = p.generation.wrapping_add(1);
        p.enabled = enabled;
        p.deadline = deadline;
        p.on_break = on_break;
    });
}

/// How long until a readout of `deadline` in whole `unit`s (rounded up)
/// next changes; `None` when nothing runs.
pub(crate) fn next_readout_change(
    deadline: Option<Instant>,
    now: Instant,
    unit: Duration,
) -> Option<Duration> {
    let left = deadline?.saturating_duration_since(now);
    if left.is_zero() || unit.is_zero() {
        return None;
    }
    let unit_ns = unit.as_nanos();
    let rem = left.as_nanos() % unit_ns;
    let step = if rem == 0 { unit_ns } else { rem };
    let step = Duration::from_nanos(u64::try_from(step).unwrap_or(u64::MAX));
    Some(step + Duration::from_millis(5))
}

/// A control verb (`mshellctl eyecare <verb>`, the pill).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Take a break now.
    Break,
    /// End the running break early.
    Skip,
    /// Restart the count from zero.
    Reset,
    On,
    Off,
    Toggle,
    Status,
}

impl Command {
    pub fn parse(verb: &str) -> Result<Self, String> {
        match verb.trim().to_ascii_lowercase().as_str() {
            "break" | "start-break" => Ok(Self::Break),
            "skip" | "skip-break" | "finish" => Ok(Self::Skip),
            "reset" => Ok(Self::Reset),
            "on" | "enable" => Ok(Self::On),
            "off" | "disable" => Ok(Self::Off),
            "toggle" => Ok(Self::Toggle),
            "status" | "" => Ok(Self::Status),
            other => Err(format!(
                "unknown eyecare action '{other}' (break|skip|reset|on|off|toggle|status)"
            )),
        }
    }
}

/// What the pill and the CLI show. `mode` is `None` while disabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Snapshot {
    pub mode: Option<Mode>,
    pub settings: Settings,
}

impl Snapshot {
    /// One machine-readable line for `mshellctl eyecare status`.
    pub fn status_line(&self) -> String {
        match self.mode {
            None => "state=off".to_string(),
            Some(Mode::Away) => "state=away".to_string(),
            Some(Mode::Active { left }) => {
                format!("state=active next-break-in={}s", left.as_secs())
            }
            Some(Mode::Break { left }) => {
                format!("state=break break-left={}s", ceil_secs(left))
            }
        }
    }
}

pub(crate) fn ceil_secs(d: Duration) -> u64 {
    d.as_secs() + u64::from(d.subsec_nanos() > 0)
}

fn config() -> EyeCareConfig {
    config_manager().config().eyecare().get_untracked()
}

fn settings_from(cfg: &EyeCareConfig) -> Settings {
    Settings::new(cfg.active_minutes, cfg.break_seconds, cfg.away_seconds)
}

/// The current state. Main thread only.
pub fn snapshot() -> Snapshot {
    let now = Instant::now();
    SERVICE.with(|s| match s.borrow().as_ref() {
        Some(svc) => Snapshot {
            mode: Some(svc.engine.mode(now)),
            settings: svc.engine.settings(),
        },
        None => Snapshot {
            mode: None,
            settings: settings_from(&config()),
        },
    })
}

/// Apply `cmd`; returns the resulting status. Main thread only. On / Off /
/// Toggle write `eyecare.enabled`, which the config effect reconciles.
pub fn run(cmd: Command) -> Snapshot {
    let now = Instant::now();
    let set_enabled = |on: bool| config_manager().update_config(move |c| c.eyecare.enabled = on);
    match cmd {
        Command::On => set_enabled(true),
        Command::Off => set_enabled(false),
        Command::Toggle => set_enabled(!config().enabled),
        Command::Status => {}
        Command::Break | Command::Skip | Command::Reset => {
            let changed = with_engine(|e| match cmd {
                Command::Break => {
                    let started = e.start_break(now);
                    started.then_some(Event::BreakStarted)
                }
                Command::Skip => {
                    e.skip_break(now);
                    None
                }
                _ => {
                    e.reset(now);
                    None
                }
            });
            if let Some(Some(event)) = changed {
                announce(event);
            }
            if changed.is_some() {
                rearm();
                notify_changed();
            }
        }
    }
    snapshot()
}

/// Run `f` on the engine if the service is up.
fn with_engine<R>(f: impl FnOnce(&mut EyeCare) -> R) -> Option<R> {
    SERVICE.with(|s| s.borrow_mut().as_mut().map(|svc| f(&mut svc.engine)))
}

/// Start / stop / reconfigure the service from `config.eyecare`. Call once
/// from the shell's startup on the main thread.
pub fn spawn_service() {
    Effect::new(|_| {
        let cfg = config_manager().config().eyecare().get();
        reconcile(&cfg);
    });
}

fn reconcile(cfg: &EyeCareConfig) {
    let settings = settings_from(cfg);
    let running = SERVICE.with(|s| s.borrow().is_some());
    match (cfg.enabled, running) {
        (false, false) => return,
        (false, true) => {
            // Dropping the Presence handle stops its thread.
            SERVICE.with(|s| s.borrow_mut().take());
        }
        (true, false) => {
            let svc = Service {
                engine: EyeCare::new(settings, Instant::now()),
                presence: start_presence(settings.away),
            };
            SERVICE.with(|s| *s.borrow_mut() = Some(svc));
        }
        (true, true) => {
            let old = SERVICE.with(|s| {
                s.borrow_mut().as_mut().map(|svc| {
                    let old = svc.engine.settings();
                    svc.engine.set_settings(settings);
                    old
                })
            });
            if old == Some(settings) {
                return;
            }
            if old.map(|o| o.away) != Some(settings.away) {
                let presence = start_presence(settings.away);
                SERVICE.with(|s| {
                    if let Some(svc) = s.borrow_mut().as_mut() {
                        svc.presence = presence;
                    }
                });
            }
        }
    }
    rearm();
    notify_changed();
}

/// Watch for `away` of no input and feed it to the engine. `None` (with a
/// log line) when the compositor can't tell us.
fn start_presence(away: Duration) -> Option<Presence> {
    let presence = match watch_presence(away) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, "eye care: no idle notifications; counting all time as active");
            return None;
        }
    };
    let mut rx = presence.subscribe();
    // Ends by itself when this Presence is dropped (its sender goes away).
    glib::spawn_future_local(async move {
        while rx.changed().await.is_ok() {
            let away = *rx.borrow_and_update();
            if with_engine(|e| e.set_away(away, Instant::now())).is_some() {
                rearm();
                notify_changed();
            }
        }
        // The watcher is gone (replaced, or its thread failed): nobody will
        // report a return, so don't leave the count parked on "away".
        if with_engine(|e| e.set_away(false, Instant::now())).is_some() {
            rearm();
            notify_changed();
        }
    });
    Some(presence)
}

fn rearm() {
    let generation = GENERATION.with(|g| {
        g.set(g.get().wrapping_add(1));
        g.get()
    });
    let Some(deadline) = with_engine(|e| e.next_deadline()).flatten() else {
        return;
    };
    let wait = deadline.saturating_duration_since(Instant::now()) + Duration::from_millis(1);
    glib::timeout_add_local_once(wait, move || on_deadline(generation));
}

fn on_deadline(generation: u64) {
    if GENERATION.with(Cell::get) != generation {
        return;
    }
    if let Some(Some(event)) = with_engine(|e| e.tick(Instant::now())) {
        announce(event);
    }
    rearm();
    notify_changed();
}

/// Notification text for `event`: `(summary, body)`.
pub(crate) fn message(event: Event, settings: Settings) -> (String, String) {
    match event {
        Event::BreakStarted => (
            "Time for an eye break".to_string(),
            format!(
                "Look at something about 6 m (20 ft) away for {} seconds.",
                settings.break_len.as_secs()
            ),
        ),
        Event::BreakFinished => (
            "Eye break over".to_string(),
            format!(
                "Next break in {} minutes of screen time.",
                settings.active.as_secs() / 60
            ),
        ),
    }
}

fn announce(event: Event) {
    let cfg = config();
    let dnd = mshell_services::notification_service()
        .map(|s| s.dnd.get())
        .unwrap_or(false);
    if cfg.sound && !dnd {
        match event {
            Event::BreakStarted => mshell_sounds::play_timer(),
            Event::BreakFinished => mshell_sounds::play_notification(),
        }
    }
    if cfg.notifications {
        let (summary, body) = message(event, settings_from(&cfg));
        notify(
            Notice::new("Eye Care", summary, body)
                .icon("eye-symbolic")
                .replace("mshell-eyecare")
                .silent(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_verbs_parse_and_unknown_ones_are_rejected() {
        assert_eq!(Command::parse("break"), Ok(Command::Break));
        assert_eq!(Command::parse("skip"), Ok(Command::Skip));
        assert_eq!(Command::parse("finish"), Ok(Command::Skip));
        assert_eq!(Command::parse(" ON "), Ok(Command::On));
        assert_eq!(Command::parse(""), Ok(Command::Status));
        assert!(Command::parse("snooze").is_err());
    }

    #[test]
    fn status_lines_name_the_state_and_the_time() {
        let s = Settings::default();
        let line = |mode| Snapshot { mode, settings: s }.status_line();
        assert_eq!(line(None), "state=off");
        assert_eq!(line(Some(Mode::Away)), "state=away");
        assert_eq!(
            line(Some(Mode::Active {
                left: Duration::from_secs(600)
            })),
            "state=active next-break-in=600s"
        );
        assert_eq!(
            line(Some(Mode::Break {
                left: Duration::from_millis(12_300)
            })),
            "state=break break-left=13s"
        );
    }

    #[test]
    fn the_readout_wakes_per_minute_or_per_second() {
        let now = Instant::now();
        let min = Duration::from_secs(60);
        let sec = Duration::from_secs(1);
        assert_eq!(next_readout_change(None, now, min), None);
        assert_eq!(
            next_readout_change(Some(now + Duration::from_secs(150)), now, min),
            Some(Duration::from_millis(30_005)),
            "2.5 min left shows 3m until 2m"
        );
        assert_eq!(
            next_readout_change(Some(now + 2 * min), now, min),
            Some(min + Duration::from_millis(5))
        );
        assert_eq!(
            next_readout_change(Some(now + Duration::from_millis(4_250)), now, sec),
            Some(Duration::from_millis(255))
        );
    }

    #[test]
    fn messages_quote_the_configured_lengths() {
        let s = Settings::new(25, 30, 120);
        assert_eq!(
            message(Event::BreakStarted, s).1,
            "Look at something about 6 m (20 ft) away for 30 seconds."
        );
        assert_eq!(
            message(Event::BreakFinished, s).1,
            "Next break in 25 minutes of screen time."
        );
    }
}
