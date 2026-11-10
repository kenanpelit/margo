//! Quick one-shot timers and reminders from one line of text: `10m tea`,
//! `14:30 meeting`, `tomorrow 9am`, `10 dk sonra çay`.
//!
//! * [`parse`] turns the line into a due time + label (pure, table-tested);
//! * [`book`] is the pending list, persisted to
//!   `$XDG_STATE_HOME/margo/timers.json` (not the user config) so timers
//!   survive an mshell restart;
//! * [`format`] words the preview, the readouts and the notifications.
//!
//! Firing belongs to the Alarm Clock scheduler in `mshell-core`
//! (`ipc/alarm.rs`): its one-second tick runs only while work is pending,
//! and [`has_pending`] is part of that test. It subscribes to [`changes`] to
//! come back when a timer is added, and calls [`take_due`] each tick; a
//! timer reached more than a minute late (the machine slept, mshell was off)
//! fires once as "missed at HH:MM" instead of ringing.
//!
//! Surfaces: the Alarm Clock menu's Timers tab, the AlarmClock bar pill's
//! countdown, the launcher's `timer` prefix and `mshellctl timer`.
//!
//! The state is a mutex, not a GTK-thread cell: the Snooze action runs on a
//! notification worker thread and `mshellctl timer` on the D-Bus task. Saves
//! go to one writer thread in order, so the file never goes backwards.

pub mod book;
pub mod format;
pub mod parse;

pub use book::{Fired, MISSED_GRACE_SECS, Timer, TimerBook};
pub use parse::{ParseError, TimerKind};

use chrono::{DateTime, Local, TimeZone};
use std::cell::Cell;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex, MutexGuard, OnceLock, mpsc};
use tokio::sync::watch;

/// One-tap lengths offered by the menu (seconds, button label).
pub const PRESETS: &[(i64, &str)] = &[
    (5 * 60, "+5m"),
    (10 * 60, "+10m"),
    (15 * 60, "+15m"),
    (25 * 60, "+25m"),
    (60 * 60, "+1h"),
];

#[derive(Default)]
struct Shared {
    book: TimerBook,
    /// The saved file has been read (nothing is saved before that).
    loaded: bool,
    /// The saved file doesn't parse: shown in the menu, never saved over.
    broken: Option<String>,
}

static SHARED: LazyLock<Mutex<Shared>> = LazyLock::new(|| Mutex::new(Shared::default()));

fn shared() -> MutexGuard<'static, Shared> {
    SHARED.lock().unwrap_or_else(|e| e.into_inner())
}

/// What subscribers need to schedule their own repaint: a change counter,
/// the soonest due time and the count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Published {
    pub generation: u64,
    /// Unix seconds of the soonest pending timer.
    pub next_due: Option<i64>,
    pub count: usize,
}

static CHANGES: LazyLock<watch::Sender<Published>> =
    LazyLock::new(|| watch::channel(Published::default()).0);

/// Subscribe to timer changes (add, cancel, fire, load).
pub fn changes() -> watch::Receiver<Published> {
    CHANGES.subscribe()
}

/// The change counter of the last publish (a repaint compares it).
pub fn generation() -> u64 {
    CHANGES.borrow().generation
}

fn publish(book: &TimerBook) {
    let next_due = book.next().map(|t| t.due);
    let count = book.timers().len();
    CHANGES.send_modify(|p| {
        p.generation = p.generation.wrapping_add(1);
        p.next_due = next_due;
        p.count = count;
    });
}

/// Run `f` on the book; publish + save when the list changed.
fn mutate<R>(f: impl FnOnce(&mut TimerBook) -> R) -> R {
    let mut s = shared();
    let before = s.book.clone();
    let out = f(&mut s.book);
    if s.book != before {
        publish(&s.book);
        if s.loaded && s.broken.is_none() {
            save(s.book.to_json());
        }
    }
    out
}

// ── Persistence ─────────────────────────────────────────────────────────

/// `$XDG_STATE_HOME/margo/timers.json`.
pub fn store_path() -> PathBuf {
    margo_fs::xdg::state_home().join("margo/timers.json")
}

/// Write `body` from the writer thread (in order; a burst collapses to the
/// last state).
fn save(body: String) {
    if cfg!(test) {
        return; // tests never touch the real state dir
    }
    static SAVER: OnceLock<Option<Mutex<mpsc::Sender<String>>>> = OnceLock::new();
    let saver = SAVER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<String>();
        std::thread::Builder::new()
            .name("timers-save".into())
            .spawn(move || {
                let path = store_path();
                while let Ok(mut body) = rx.recv() {
                    while let Ok(newer) = rx.try_recv() {
                        body = newer;
                    }
                    if let Err(e) = save_to(&path, &body) {
                        tracing::warn!(error = %e, "timers: not saved");
                    }
                }
            })
            .ok()
            .map(|_| Mutex::new(tx))
    });
    match saver {
        Some(tx) => {
            let _ = tx.lock().unwrap_or_else(|e| e.into_inner()).send(body);
        }
        None => tracing::warn!("timers: could not start the save thread"),
    }
}

/// Save `body` to `path` unless the file there doesn't parse.
fn save_to(path: &std::path::Path, body: &str) -> Result<(), margo_fs::side_file::SaveError> {
    margo_fs::side_file::save(path, body.as_bytes(), |t| {
        TimerBook::from_json(t).map(|_| ())
    })
}

/// Read the book at `path`: `(book, broken reason)`.
fn load_from(path: &std::path::Path) -> (TimerBook, Option<String>) {
    use margo_fs::side_file::{self, SideLoad};
    match side_file::load(path, TimerBook::from_json) {
        SideLoad::Loaded(book) => (book, None),
        SideLoad::Missing => (TimerBook::default(), None),
        SideLoad::Broken { error, backup } => {
            tracing::warn!(%error, ?backup, "timers: timers.json doesn't parse; not saving over it");
            (TimerBook::default(), Some(error))
        }
    }
}

/// Fold the timers added before the file was read into the loaded book.
fn merge_loaded(mut loaded: TimerBook, early: &TimerBook) -> TimerBook {
    for t in early.timers() {
        let _ = loaded.add(t.due, t.created, t.kind, &t.label);
    }
    loaded
}

/// Read the saved timers off the GTK thread. Call once at startup; the
/// scheduler's [`changes`] subscription picks the loaded timers up (and
/// fires the ones that came due while mshell was off).
pub fn spawn_service() {
    let spawned = std::thread::Builder::new()
        .name("timers-load".into())
        .spawn(|| {
            let (loaded, broken) = load_from(&store_path());
            let mut s = shared();
            let had_early = !s.book.is_empty();
            s.book = merge_loaded(loaded, &s.book);
            s.loaded = true;
            s.broken = broken;
            publish(&s.book);
            if had_early && s.broken.is_none() {
                save(s.book.to_json());
            }
        });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "timers: could not start the load thread");
    }
}

/// Why the saved file is ignored, if it is.
pub fn broken() -> Option<String> {
    shared().broken.clone()
}

// ── The API the surfaces use ────────────────────────────────────────────

pub fn now_unix() -> i64 {
    Local::now().timestamp()
}

/// A Unix time as local wall-clock time.
pub fn local(ts: i64) -> DateTime<Local> {
    Local
        .timestamp_opt(ts, 0)
        .single()
        .unwrap_or_else(Local::now)
}

/// A timer's due time for a readout: `"14:35"`, `"tomorrow 09:00"`.
pub fn due_clock(timer: &Timer, now: i64) -> String {
    let due = local(timer.due);
    let now = local(now);
    let seconds = timer.kind != TimerKind::Reminder
        && chrono::Timelike::second(&due) != 0
        && timer.due - timer.created < 3600;
    format::day_clock(&due, &now, seconds)
}

/// The live preview of `text` (`"Rings at 14:35 (in 10 min) — tea"`), or
/// the parse error to show instead.
pub fn preview(text: &str) -> Result<String, String> {
    let now = Local::now();
    parse::parse(&now, text)
        .map(|p| format::describe(&p.due, &now, &p.label))
        .map_err(|e| e.to_string())
}

/// Parse `text` and add the timer.
pub fn add(text: &str) -> Result<Timer, String> {
    let now = Local::now();
    let p = parse::parse(&now, text).map_err(|e| e.to_string())?;
    mutate(|b| b.add(p.due.timestamp(), now.timestamp(), p.kind, &p.label))
        .map_err(|e| e.to_string())
}

/// Add a timer `secs` from now (the menu's preset buttons).
pub fn add_after(secs: i64, label: &str) -> Result<Timer, String> {
    let now = now_unix();
    mutate(|b| b.add(now + secs.max(1), now, TimerKind::Timer, label)).map_err(|e| e.to_string())
}

/// Re-queue a rung timer `secs` from now (the notification's Snooze).
pub fn snooze(timer: &Timer, secs: i64) -> Result<Timer, String> {
    let now = now_unix();
    mutate(|b| b.add(now + secs.max(1), now, TimerKind::Snooze, &timer.label))
        .map_err(|e| e.to_string())
}

pub fn cancel(id: u32) -> Option<Timer> {
    mutate(|b| b.cancel(id))
}

pub fn clear() -> usize {
    mutate(TimerBook::clear)
}

/// The pending timers, soonest first.
pub fn list() -> Vec<Timer> {
    shared().book.timers().to_vec()
}

/// The soonest pending timer.
pub fn next() -> Option<Timer> {
    shared().book.next().cloned()
}

/// Whether the scheduler tick has a timer to wait for.
pub fn has_pending() -> bool {
    !shared().book.is_empty()
}

/// How long until a remaining-time readout of any pending timer changes
/// (aligned to the wall-clock second), `None` with nothing pending — so a
/// readout wakes once per visible change and never while idle.
pub fn next_readout_change() -> Option<std::time::Duration> {
    use std::time::{SystemTime, UNIX_EPOCH};
    let since = SystemTime::now().duration_since(UNIX_EPOCH).ok()?;
    let now = since.as_secs() as i64;
    let step = shared()
        .book
        .timers()
        .iter()
        .map(|t| format::pill_next_change(t.remaining(now)))
        .min()?;
    Some(readout_wait(step, since.subsec_nanos()))
}

/// `step` whole seconds of readout, starting from `subsec_nanos` into the
/// current second: wait to the boundary, plus a hair so it has rolled.
fn readout_wait(step: std::time::Duration, subsec_nanos: u32) -> std::time::Duration {
    use std::time::Duration;
    let into = Duration::from_nanos(u64::from(subsec_nanos));
    step.saturating_sub(into) + Duration::from_millis(5)
}

/// Take the timers due at `now` (Unix seconds) — the scheduler tick.
pub fn take_due(now: i64) -> Vec<Fired> {
    mutate(|b| b.take_due(now))
}

// ── mshellctl timer ─────────────────────────────────────────────────────

/// A `mshellctl timer` verb.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Add(String),
    List,
    Cancel(u32),
    Clear,
}

impl Command {
    /// Parse a verb + its argument. Unknown verbs are an error.
    pub fn parse(verb: &str, arg: &str) -> Result<Self, String> {
        match verb.trim().to_ascii_lowercase().as_str() {
            "add" if !arg.trim().is_empty() => Ok(Self::Add(arg.trim().to_string())),
            "add" => Err(parse::ParseError::Empty.to_string()),
            "list" | "ls" | "" => Ok(Self::List),
            "cancel" | "rm" | "remove" => arg
                .trim()
                .trim_start_matches('#')
                .parse::<u32>()
                .map(Self::Cancel)
                .map_err(|_| {
                    format!("cancel needs a timer id (see `mshellctl timer list`), got '{arg}'")
                }),
            "clear" => Ok(Self::Clear),
            other => Err(format!(
                "unknown timer action '{other}' (add|list|cancel|clear)"
            )),
        }
    }
}

/// Run a `mshellctl timer` verb; the text to print, or the error.
pub fn run(cmd: Command) -> Result<String, String> {
    let now = now_unix();
    match cmd {
        Command::Add(text) => {
            let t = add(&text)?;
            let when = format::describe(&local(t.due), &local(now), &t.label);
            Ok(format!("Timer {}: {when}", t.id))
        }
        Command::List => Ok(list()
            .iter()
            .map(|t| format::list_line(t, &due_clock(t, now), now))
            .collect::<Vec<_>>()
            .join("\n")),
        Command::Cancel(id) => match cancel(id) {
            Some(t) if t.label.is_empty() => Ok(format!("Cancelled timer {id}")),
            Some(t) => Ok(format!("Cancelled timer {id} ({})", t.label)),
            None => Err(format!(
                "No timer with id {id} (see `mshellctl timer list`)"
            )),
        },
        Command::Clear => {
            let n = clear();
            Ok(format!(
                "Cleared {n} timer{}",
                if n == 1 { "" } else { "s" }
            ))
        }
    }
}

// ── Menu tab request ────────────────────────────────────────────────────

thread_local! {
    static PENDING_TAB: Cell<bool> = const { Cell::new(false) };
}

/// Ask the Alarm Clock menu to open on its Timers tab next reveal.
pub(crate) fn request_tab() {
    PENDING_TAB.with(|p| p.set(true));
}

/// Consume the pending Timers-tab request.
pub(crate) fn take_tab_request() -> bool {
    PENDING_TAB.with(|p| p.replace(false))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The runtime state is process-global; serialise the tests that use it.
    static GLOBAL: Mutex<()> = Mutex::new(());

    fn reset() -> MutexGuard<'static, ()> {
        let guard = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
        *shared() = Shared::default();
        guard
    }

    #[test]
    fn verbs_parse_and_bad_ones_are_rejected() {
        assert_eq!(
            Command::parse("add", " 10m tea "),
            Ok(Command::Add("10m tea".into()))
        );
        assert!(Command::parse("add", "  ").is_err());
        assert_eq!(Command::parse("list", ""), Ok(Command::List));
        assert_eq!(Command::parse("", ""), Ok(Command::List));
        assert_eq!(Command::parse("cancel", "3"), Ok(Command::Cancel(3)));
        assert_eq!(Command::parse("rm", "#3"), Ok(Command::Cancel(3)));
        assert!(Command::parse("cancel", "tea").is_err());
        assert_eq!(Command::parse("CLEAR", ""), Ok(Command::Clear));
        assert!(Command::parse("snooze", "").is_err());
    }

    #[test]
    fn the_cli_adds_lists_cancels_and_clears() {
        let _g = reset();
        let out = run(Command::Add("10m tea".into())).expect("adds");
        assert!(out.starts_with("Timer 1: Rings at "), "{out}");
        assert!(out.ends_with("(in 10 min) — tea"), "{out}");
        run(Command::Add("1h".into())).expect("adds");
        let list = run(Command::List).expect("lists");
        let lines: Vec<&str> = list.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(
            lines[0].starts_with("1\t") && lines[0].ends_with("\ttea"),
            "{list}"
        );
        assert!(run(Command::Add("tea".into())).is_err());
        assert_eq!(
            run(Command::Cancel(1)),
            Ok("Cancelled timer 1 (tea)".into())
        );
        assert!(run(Command::Cancel(1)).is_err());
        assert_eq!(run(Command::Clear), Ok("Cleared 1 timer".into()));
        assert_eq!(run(Command::List), Ok(String::new()));
    }

    #[test]
    fn changes_are_published_and_drive_the_pending_test() {
        let _g = reset();
        let mut rx = changes();
        rx.borrow_and_update();
        assert!(!has_pending());
        let t = add_after(600, "tea").expect("adds");
        assert!(rx.has_changed().unwrap_or(false));
        let p = *rx.borrow_and_update();
        assert_eq!((p.next_due, p.count), (Some(t.due), 1));
        assert!(has_pending());
        // Taking nothing publishes nothing.
        assert!(take_due(t.due - 1).is_empty());
        assert!(!rx.has_changed().unwrap_or(true));
        let fired = take_due(t.due);
        assert_eq!(fired.len(), 1);
        assert!(!fired[0].missed);
        assert!(
            !has_pending(),
            "the tick may stop once the last timer fired"
        );
        assert_eq!(rx.borrow_and_update().count, 0);
    }

    #[test]
    fn snooze_requeues_the_label_as_a_snooze() {
        let _g = reset();
        let t = add_after(1, "tea").expect("adds");
        let s = snooze(&t, 300).expect("snoozes");
        assert_eq!((s.kind, s.label.as_str()), (TimerKind::Snooze, "tea"));
        assert_eq!(s.due - s.created, 300);
    }

    #[test]
    fn the_store_round_trips_through_a_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("margo/timers.json");
        assert_eq!(
            load_from(&path),
            (TimerBook::default(), None),
            "missing = empty"
        );
        let mut book = TimerBook::default();
        book.add(2_000, 1_000, TimerKind::Reminder, "standup")
            .expect("room");
        book.add(1_500, 1_000, TimerKind::Timer, "tea")
            .expect("room");
        save_to(&path, &book.to_json()).expect("saves");
        assert_eq!(load_from(&path), (book, None));
    }

    #[test]
    fn a_broken_file_is_kept_and_never_saved_over() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("timers.json");
        std::fs::write(&path, "{ not json").expect("write");
        let (book, broken) = load_from(&path);
        assert!(book.is_empty());
        assert!(broken.is_some());
        assert!(dir.path().join("timers.json.bak").exists());
        assert!(save_to(&path, &TimerBook::default().to_json()).is_err());
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "{ not json");
    }

    #[test]
    fn timers_added_before_the_file_loads_are_kept() {
        let mut loaded = TimerBook::default();
        loaded.add(500, 0, TimerKind::Timer, "saved").expect("room");
        let mut early = TimerBook::default();
        early.add(100, 0, TimerKind::Timer, "early").expect("room");
        let merged = merge_loaded(loaded, &early);
        let labels: Vec<&str> = merged.timers().iter().map(|t| t.label.as_str()).collect();
        assert_eq!(labels, vec!["early", "saved"]);
        let ids: Vec<u32> = merged.timers().iter().map(|t| t.id).collect();
        assert_eq!(ids, vec![2, 1]);
    }

    #[test]
    fn missed_timers_after_a_restart_fire_once_as_missed() {
        let _g = reset();
        // A file saved before a shutdown, read back an hour after one came due.
        let mut saved = TimerBook::default();
        saved
            .add(10_000, 9_400, TimerKind::Timer, "tea")
            .expect("room");
        saved
            .add(20_000, 9_400, TimerKind::Reminder, "later")
            .expect("room");
        shared().book = TimerBook::from_json(&saved.to_json()).expect("parses");
        let fired = take_due(10_000 + 3600);
        assert_eq!(fired.len(), 1);
        assert!(fired[0].missed);
        assert_eq!(fired[0].timer.label, "tea");
        assert!(take_due(10_000 + 3600).is_empty());
        assert!(has_pending());
    }

    #[test]
    fn readouts_wake_on_the_second_boundary() {
        use std::time::Duration;
        let ms = Duration::from_millis;
        assert_eq!(readout_wait(Duration::from_secs(1), 250_000_000), ms(755));
        assert_eq!(readout_wait(Duration::from_secs(60), 0), ms(60_005));
    }

    #[test]
    fn nothing_pending_means_no_readout_wake_up() {
        let _g = reset();
        assert_eq!(next_readout_change(), None);
        add_after(600, "").expect("adds");
        let wait = next_readout_change().expect("a timer runs");
        assert!(wait <= std::time::Duration::from_millis(1005), "{wait:?}");
    }

    #[test]
    fn the_presets_are_the_plugins_five() {
        let labels: Vec<&str> = PRESETS.iter().map(|(_, l)| *l).collect();
        assert_eq!(labels, vec!["+5m", "+10m", "+15m", "+25m", "+1h"]);
    }
}
