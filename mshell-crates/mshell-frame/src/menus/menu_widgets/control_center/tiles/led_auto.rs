//! Automatic ThinkPad logo LED behaviour (`control_center.logo_led`):
//! off while the screen is locked, off while Do Not Disturb is on, and a
//! short blink when a notification arrives.
//!
//! It writes `brightness` directly and runs only when the user can (a udev
//! rule, see docs/widgets.md): going through the polkit helper would mean
//! a password prompt on every lock or notification. Without write access
//! it stays inert and Settings says why.
//!
//! The decisions are pure ([`Driver`], [`blink_pattern`], [`arrivals`]);
//! [`configure`] runs them on the shell's tokio runtime against the
//! compositor's lock state, the DND change queue and the notification list.

use super::led::{LOGO_LED, LedState, probe};
use mshell_config::schema::config::LogoLedAutomation;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// On/off pairs in a notification blink.
const BLINKS: usize = 3;
/// Each blink half-period.
const BLINK_STEP: Duration = Duration::from_millis(220);

/// Whether the rules want the LED forced off now.
pub(crate) fn forced_off(rules: &LogoLedAutomation, locked: bool, dnd: bool) -> bool {
    (rules.off_while_locked && locked) || (rules.off_while_dnd && dnd)
}

/// Remembers the user's LED state across a forced-off period.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Driver {
    forced: bool,
    /// The LED's state when forcing began.
    base_on: bool,
}

impl Driver {
    /// The rules now want `want` (forced off or not); the LED reads
    /// `current_on`. Returns the state to write, if any.
    ///
    /// Leaving a forced period restores the earlier state, unless the LED
    /// is on now: then the user switched it on meanwhile, and that wins.
    pub(crate) fn update(&mut self, want: bool, current_on: bool) -> Option<bool> {
        match (want, self.forced) {
            (true, false) => {
                self.forced = true;
                self.base_on = current_on;
                current_on.then_some(false)
            }
            (false, true) => {
                self.forced = false;
                (!current_on && self.base_on).then_some(true)
            }
            _ => None,
        }
    }

    pub(crate) fn forced(&self) -> bool {
        self.forced
    }
}

/// The states to write for a blink that ends where it started.
pub(crate) fn blink_pattern(on: bool, times: usize) -> Vec<bool> {
    (0..times).flat_map(|_| [!on, on]).collect()
}

/// Whether `ids` holds a notification newer than `seen_max`, and the new
/// mark. The first look (`None`) only sets the mark: starting the shell
/// with notifications in the list is not an arrival.
pub(crate) fn arrivals(seen_max: Option<u32>, ids: &[u32]) -> (bool, Option<u32>) {
    let max = ids.iter().copied().max();
    match (seen_max, max) {
        (None, m) => (false, Some(m.unwrap_or(0))),
        (Some(s), Some(m)) if m > s => (true, Some(m)),
        (s, _) => (false, s),
    }
}

/// Why the automation can't run on this machine, if it can't.
pub(crate) fn blocker(led: &LedState) -> Option<&'static str> {
    if !led.available {
        Some("no ThinkPad logo LED")
    } else if !led.writable {
        Some("its brightness is not writable (add the udev rule)")
    } else {
        None
    }
}

fn write(on: bool, max: u32) {
    let path = Path::new("/sys/class/leds")
        .join(LOGO_LED)
        .join("brightness");
    let value = if on { max.max(1) } else { 0 };
    if let Err(e) = std::fs::write(&path, format!("{value}\n")) {
        tracing::warn!("logo LED: cannot write {}: {e}", path.display());
    }
}

/// The running automation's stop switch.
static RUNNING: Mutex<Option<CancellationToken>> = Mutex::new(None);

/// Start (or restart) the automation for `rules`, or stop it when every
/// rule is off or the LED isn't writable. Called from the config effect.
pub(crate) fn configure(rules: LogoLedAutomation) {
    let mut running = RUNNING.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(old) = running.take() {
        old.cancel();
    }
    let any = rules.off_while_locked || rules.off_while_dnd || rules.blink_on_notification;
    if !any {
        return;
    }
    let led = probe();
    if let Some(why) = blocker(&led) {
        tracing::info!("logo LED automation is off: {why}");
        return;
    }
    let token = CancellationToken::new();
    *running = Some(token.clone());
    mshell_services::tokio_rt_spawn(run(rules, token));
}

async fn run(rules: LogoLedAutomation, stop: CancellationToken) {
    use futures::StreamExt;
    let service = mshell_services::notification_service();
    let mut locked_rx = mshell_session::session_lock::compositor_locked();
    let mut dnd_rx = service.as_ref().map(|s| s.dnd_changes());
    let mut dnd = service.as_ref().map(|s| s.dnd.get()).unwrap_or(false);
    let mut notes = service
        .as_ref()
        .filter(|_| rules.blink_on_notification)
        .map(|s| Box::pin(s.notifications.watch()));
    let mut seen: Option<u32> = None;
    let mut driver = Driver::default();

    let locked_now = |rx: &tokio::sync::watch::Receiver<Option<bool>>| rx.borrow().unwrap_or(false);
    let apply = |driver: &mut Driver, locked: bool, dnd: bool| {
        let led = probe();
        if !led.writable {
            return;
        }
        if let Some(on) = driver.update(forced_off(&rules, locked, dnd), led.on) {
            write(on, led.max);
        }
    };
    apply(&mut driver, locked_now(&locked_rx), dnd);

    loop {
        tokio::select! {
            () = stop.cancelled() => break,
            changed = locked_rx.changed() => {
                if changed.is_err() {
                    break;
                }
                apply(&mut driver, locked_now(&locked_rx), dnd);
            }
            v = async {
                match dnd_rx.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                let Some(v) = v else { dnd_rx = None; continue };
                dnd = v;
                apply(&mut driver, locked_now(&locked_rx), dnd);
            }
            list = async {
                match notes.as_mut() {
                    Some(s) => s.next().await,
                    None => std::future::pending().await,
                }
            } => {
                let Some(list) = list else { notes = None; continue };
                let ids: Vec<u32> = list.iter().map(|n| n.id).collect();
                let (arrived, mark) = arrivals(seen, &ids);
                seen = mark;
                if arrived && !driver.forced() && !dnd && !locked_now(&locked_rx) {
                    let led = probe();
                    for on in blink_pattern(led.on, BLINKS) {
                        write(on, led.max);
                        tokio::time::sleep(BLINK_STEP).await;
                    }
                }
            }
        }
    }
    // Stopping mid-force (rules changed): give the user their LED back.
    let led = probe();
    if led.writable
        && let Some(on) = driver.update(false, led.on)
    {
        write(on, led.max);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(locked: bool, dnd: bool) -> LogoLedAutomation {
        LogoLedAutomation {
            off_while_locked: locked,
            off_while_dnd: dnd,
            blink_on_notification: false,
        }
    }

    #[test]
    fn forced_off_follows_the_enabled_rules() {
        assert!(forced_off(&rules(true, false), true, false));
        assert!(!forced_off(&rules(true, false), false, true));
        assert!(forced_off(&rules(false, true), false, true));
        assert!(forced_off(&rules(true, true), true, true));
        assert!(!forced_off(&rules(false, false), true, true));
    }

    #[test]
    fn lock_turns_it_off_and_unlock_restores_it() {
        let mut d = Driver::default();
        assert_eq!(d.update(false, true), None, "nothing to do unforced");
        assert_eq!(d.update(true, true), Some(false), "locked: off");
        assert_eq!(d.update(true, false), None, "still locked");
        assert_eq!(d.update(false, false), Some(true), "unlocked: back on");
    }

    #[test]
    fn an_led_that_was_off_stays_off() {
        let mut d = Driver::default();
        assert_eq!(d.update(true, false), None);
        assert_eq!(d.update(false, false), None);
    }

    #[test]
    fn the_user_switching_it_on_while_forced_wins() {
        let mut d = Driver::default();
        assert_eq!(d.update(true, false), None);
        // DND still on, user turns the LED on from the tile.
        assert_eq!(d.update(false, true), None, "left on");
        let mut d = Driver::default();
        d.update(true, true);
        assert_eq!(d.update(false, true), None, "already on: no write");
    }

    #[test]
    fn blinks_end_where_they_started() {
        assert_eq!(blink_pattern(false, 2), [true, false, true, false]);
        assert_eq!(blink_pattern(true, 1), [false, true]);
        assert!(blink_pattern(true, 0).is_empty());
    }

    #[test]
    fn only_newer_notifications_are_arrivals() {
        assert_eq!(arrivals(None, &[4, 7]), (false, Some(7)), "startup");
        assert_eq!(arrivals(None, &[]), (false, Some(0)));
        assert_eq!(arrivals(Some(7), &[4, 7]), (false, Some(7)));
        assert_eq!(arrivals(Some(7), &[4, 7, 8]), (true, Some(8)));
        // Dismissals shrink the list: not an arrival, mark kept.
        assert_eq!(arrivals(Some(8), &[4]), (false, Some(8)));
        assert_eq!(arrivals(Some(8), &[]), (false, Some(8)));
    }

    #[test]
    fn blockers_name_the_missing_piece() {
        let none = LedState::default();
        assert_eq!(blocker(&none), Some("no ThinkPad logo LED"));
        let ro = LedState {
            available: true,
            ..LedState::default()
        };
        assert!(blocker(&ro).unwrap().contains("udev rule"));
        let rw = LedState {
            available: true,
            writable: true,
            ..LedState::default()
        };
        assert_eq!(blocker(&rw), None);
    }
}
