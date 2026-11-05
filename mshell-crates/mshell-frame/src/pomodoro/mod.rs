//! The shell's one Pomodoro timer — shared by the Alarm Clock menu's
//! Pomodoro tab, the `Pomodoro` bar pill and `mshellctl pomodoro`.
//!
//! Runtime state is transient (never persisted) and lives on the GTK main
//! thread; the rules come from `config.pomodoro`. While a phase runs there
//! is exactly one one-shot glib timer, armed for the phase deadline; while
//! the timer is ready or paused nothing is scheduled at all. Each re-arm
//! bumps a generation counter instead of removing the old source (removing
//! a source that already fired aborts the process), so a stale wake-up just
//! sees a newer generation and returns.

pub mod engine;

use engine::{PhaseEnd, Pomodoro, RunState, Settings};
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, Pomodoro as PomodoroConfig};
use mshell_process::desktop::{Notice, notify};
use reactive_graph::effect::Effect;
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::glib;
use std::cell::{Cell, RefCell};
use std::sync::LazyLock;
use std::time::{Duration, Instant};
use tokio::sync::watch;

pub use engine::{Phase, format_remaining};

thread_local! {
    static STATE: RefCell<Option<Pomodoro>> = const { RefCell::new(None) };
    static GENERATION: Cell<u64> = const { Cell::new(0) };
    static PENDING_TAB: Cell<bool> = const { Cell::new(false) };
}

/// What a subscriber needs to schedule its own repaint off the GTK thread:
/// a change counter plus the running deadline (`None` = nothing ticks).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Published {
    pub(crate) generation: u64,
    pub(crate) deadline: Option<Instant>,
}

/// Re-published on every state change so the pill and menu can sleep until
/// something happens. (The state itself is main-thread only; a relm4
/// command future runs elsewhere, so it reads this instead.)
static CHANGES: LazyLock<watch::Sender<Published>> =
    LazyLock::new(|| watch::channel(Published::default()).0);

/// Subscribe to Pomodoro state changes.
pub(crate) fn changes() -> watch::Receiver<Published> {
    CHANGES.subscribe()
}

fn notify_changed() {
    let deadline = with_state(|p| p.deadline());
    CHANGES.send_modify(|p| {
        p.generation = p.generation.wrapping_add(1);
        p.deadline = deadline;
    });
}

/// How long until an `MM:SS` readout of `deadline` next changes; `None`
/// when nothing runs. The readout rounds up, so it rolls over each time
/// the time left crosses a whole second.
pub(crate) fn next_readout_change(deadline: Option<Instant>, now: Instant) -> Option<Duration> {
    let left = deadline?.saturating_duration_since(now);
    if left.is_zero() {
        return None; // the deadline timer takes it from here
    }
    let frac = Duration::from_nanos(u64::from(left.subsec_nanos()));
    let step = if frac.is_zero() {
        Duration::from_secs(1)
    } else {
        frac
    };
    // A hair past the boundary so the readout has definitely rolled.
    Some(step + Duration::from_millis(5))
}

/// A control verb (`mshellctl pomodoro <verb>`, the menu buttons, the pill).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Toggle,
    Start,
    Pause,
    Skip,
    Reset,
    ResetAll,
    /// No change; report the status.
    Status,
}

impl Command {
    /// Parse a CLI verb. Unknown verbs are an error so the CLI exits non-zero.
    pub fn parse(verb: &str) -> Result<Self, String> {
        match verb.trim().to_ascii_lowercase().as_str() {
            "toggle" => Ok(Self::Toggle),
            "start" | "resume" => Ok(Self::Start),
            "pause" => Ok(Self::Pause),
            "skip" | "next" => Ok(Self::Skip),
            "reset" => Ok(Self::Reset),
            "reset-all" | "resetall" => Ok(Self::ResetAll),
            "status" | "" => Ok(Self::Status),
            other => Err(format!(
                "unknown pomodoro action '{other}' (toggle|start|pause|skip|reset|reset-all|status)"
            )),
        }
    }
}

/// What the pill / menu / CLI show.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Snapshot {
    pub phase: Phase,
    pub run_state: RunState,
    pub remaining: Duration,
    pub progress: f64,
    pub session: u32,
    pub cycles: u32,
    pub completed: u32,
}

impl Snapshot {
    /// One machine-readable line for `mshellctl pomodoro status`.
    pub fn status_line(&self) -> String {
        let state = match self.run_state {
            RunState::Ready => "ready",
            RunState::Running => "running",
            RunState::Paused => "paused",
        };
        format!(
            "phase={} state={state} remaining={} session={}/{} completed={}",
            self.phase.token(),
            format_remaining(self.remaining),
            self.session,
            self.cycles,
            self.completed
        )
    }
}

fn settings_from(cfg: &PomodoroConfig) -> Settings {
    Settings::from_minutes(
        cfg.work_minutes,
        cfg.short_break_minutes,
        cfg.long_break_minutes,
        cfg.sessions_before_long_break,
        cfg.auto_start_breaks,
        cfg.auto_start_work,
    )
}

fn config() -> PomodoroConfig {
    config_manager().config().pomodoro().get_untracked()
}

/// Run `f` on the timer, creating it from the config on first use.
fn with_state<R>(f: impl FnOnce(&mut Pomodoro) -> R) -> R {
    STATE.with(|cell| {
        let mut slot = cell.borrow_mut();
        let p = slot.get_or_insert_with(|| Pomodoro::new(settings_from(&config())));
        f(p)
    })
}

/// The current state. Main thread only.
pub fn snapshot() -> Snapshot {
    let now = Instant::now();
    with_state(|p| Snapshot {
        phase: p.phase(),
        run_state: p.run_state(),
        remaining: p.remaining(now),
        progress: p.progress(now),
        session: p.session(),
        cycles: p.settings().cycles,
        completed: p.completed(),
    })
}

/// Apply `cmd` and return the resulting status. Main thread only.
pub fn run(cmd: Command) -> Snapshot {
    let now = Instant::now();
    let changed = with_state(|p| {
        let before = (p.phase(), p.run_state(), p.session(), p.remaining(now));
        match cmd {
            Command::Toggle => p.toggle(now),
            Command::Start => p.start(now),
            Command::Pause => p.pause(now),
            Command::Skip => p.skip(),
            Command::Reset => p.reset(),
            Command::ResetAll => p.reset_all(),
            Command::Status => {}
        }
        before != (p.phase(), p.run_state(), p.session(), p.remaining(now))
    });
    if changed {
        rearm();
        notify_changed();
    }
    snapshot()
}

/// Follow `config.pomodoro` edits (Settings page) live. Call once from the
/// shell's startup on the main thread.
pub fn spawn_settings_watch() {
    Effect::new(|_| {
        let cfg = config_manager().config().pomodoro().get();
        let settings = settings_from(&cfg);
        let changed = STATE.with(|cell| match cell.borrow_mut().as_mut() {
            Some(p) if p.settings() != settings => {
                p.set_settings(settings);
                true
            }
            _ => false,
        });
        if changed {
            notify_changed();
        }
    });
}

/// Ask the Alarm Clock menu to open on its Pomodoro tab next reveal.
pub(crate) fn request_tab() {
    PENDING_TAB.with(|p| p.set(true));
}

/// Consume the pending Pomodoro-tab request.
pub(crate) fn take_tab_request() -> bool {
    PENDING_TAB.with(|p| p.replace(false))
}

/// Arm the single deadline timer for the running phase (or nothing).
fn rearm() {
    let generation = GENERATION.with(|g| {
        g.set(g.get().wrapping_add(1));
        g.get()
    });
    let left = with_state(|p| p.is_running().then(|| p.remaining(Instant::now())));
    if let Some(left) = left {
        // glib timeouts are millisecond-granular: round up so the wake-up
        // lands at or after the deadline, not a hair before it.
        glib::timeout_add_local_once(left + Duration::from_millis(1), move || {
            on_deadline(generation)
        });
    }
}

fn on_deadline(generation: u64) {
    if GENERATION.with(Cell::get) != generation {
        return; // superseded by a later command
    }
    if let Some(end) = with_state(|p| p.tick(Instant::now())) {
        announce(end);
        notify_changed();
    }
    rearm();
}

/// The phase-end notification text: `(summary, body)`.
pub(crate) fn phase_end_message(end: PhaseEnd, next_minutes: u64) -> (String, String) {
    let summary = match end.finished {
        Phase::Work => "Focus session complete",
        Phase::ShortBreak => "Break over",
        Phase::LongBreak => "Long break over",
    }
    .to_string();
    let next = format!("{} · {next_minutes} min", end.next.label());
    let body = if end.auto_started {
        format!("{next} started.")
    } else {
        format!("Next: {next}. Start it from the bar or the Alarm Clock menu.")
    };
    (summary, body)
}

fn announce(end: PhaseEnd) {
    let cfg = config();
    if cfg.sound {
        mshell_sounds::play_timer();
    }
    if cfg.notifications {
        let minutes = with_state(|p| p.settings().duration(end.next).as_secs() / 60);
        let (summary, body) = phase_end_message(end, minutes);
        // Our own chime (or none) replaces the server's sound.
        notify(
            Notice::new("Pomodoro", summary, body)
                .icon("alarm-symbolic")
                .replace("mshell-pomodoro")
                .silent(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_verbs_parse_and_unknown_ones_are_rejected() {
        assert_eq!(Command::parse("toggle"), Ok(Command::Toggle));
        assert_eq!(Command::parse(" Start "), Ok(Command::Start));
        assert_eq!(Command::parse("resume"), Ok(Command::Start));
        assert_eq!(Command::parse("skip"), Ok(Command::Skip));
        assert_eq!(Command::parse("reset-all"), Ok(Command::ResetAll));
        assert_eq!(Command::parse("status"), Ok(Command::Status));
        assert!(Command::parse("stop-it").is_err());
    }

    #[test]
    fn the_readout_wakes_on_second_boundaries_and_never_when_idle() {
        let now = Instant::now();
        assert_eq!(next_readout_change(None, now), None);
        assert_eq!(next_readout_change(Some(now), now), None, "already due");
        assert_eq!(
            next_readout_change(Some(now + Duration::from_millis(2_300)), now),
            Some(Duration::from_millis(305))
        );
        assert_eq!(
            next_readout_change(Some(now + Duration::from_secs(4)), now),
            Some(Duration::from_millis(1_005))
        );
    }

    #[test]
    fn status_line_is_one_parseable_line() {
        let s = Snapshot {
            phase: Phase::ShortBreak,
            run_state: RunState::Paused,
            remaining: Duration::from_secs(185),
            progress: 0.4,
            session: 2,
            cycles: 4,
            completed: 2,
        };
        assert_eq!(
            s.status_line(),
            "phase=short-break state=paused remaining=03:05 session=2/4 completed=2"
        );
    }

    #[test]
    fn the_notification_says_what_ended_and_what_is_next() {
        let end = PhaseEnd {
            finished: Phase::Work,
            next: Phase::ShortBreak,
            auto_started: false,
        };
        let (summary, body) = phase_end_message(end, 5);
        assert_eq!(summary, "Focus session complete");
        assert!(body.starts_with("Next: Short break · 5 min."), "{body}");
        let auto = PhaseEnd {
            auto_started: true,
            ..end
        };
        assert_eq!(phase_end_message(auto, 5).1, "Short break · 5 min started.");
    }
}
