//! Alarm scheduler: the on-demand one-second tick, snoozes and firing.

use super::*;
use mshell_process::desktop::{self, Notice, Urgency};

// ── Alarm scheduler (port of the DMS alarmClock plugin) ─────────────────────
// A single main-thread glib timer ticks each second, reads the alarms from the
// config (reactive reads are main-thread-only), and fires matching alarms once
// per clock minute. The ring tone + the Stop/Snooze notification run on
// workers; Snooze re-fire times go on a thread-safe queue the tick drains.
//
// The timer only exists while there is something to wait for. It used to be
// installed unconditionally at startup, so a session with zero alarms still
// woke the GTK main loop once a second forever — and a main-loop wakeup is not
// free here: it walks the whole GSource list. `ensure_running` installs it,
// the tick removes itself once no enabled alarm and no pending snooze remain,
// and both the config effect and the snooze path re-arm it.
pub(super) static ALARM_SNOOZES: std::sync::Mutex<Vec<std::time::SystemTime>> =
    std::sync::Mutex::new(Vec::new());

thread_local! {
    static ALARM_TIMER_RUNNING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Is there any reason for the per-second tick to exist?
///
/// Main-thread only: reads the reactive config.
pub(super) fn alarm_work_pending() -> bool {
    let snoozed = ALARM_SNOOZES.lock().map(|q| !q.is_empty()).unwrap_or(false);
    snoozed
        || config_manager()
            .config()
            .alarm()
            .alarms()
            .get_untracked()
            .iter()
            .any(|a| a.enabled)
}

/// Install the per-second tick if work is pending and it isn't already up.
///
/// Main-thread only. Idempotent.
pub(super) fn ensure_alarm_timer() {
    if ALARM_TIMER_RUNNING.with(|r| r.get()) || !alarm_work_pending() {
        return;
    }
    ALARM_TIMER_RUNNING.with(|r| r.set(true));
    let last_minute = std::rc::Rc::new(std::cell::Cell::new(i64::MIN));
    glib::timeout_add_seconds_local(1, move || {
        if !alarm_work_pending() {
            ALARM_TIMER_RUNNING.with(|r| r.set(false));
            // Returning `Break` removes the source. Never pair this with a
            // `SourceId::remove()` elsewhere — removing an already-finished
            // source aborts the process.
            return glib::ControlFlow::Break;
        }
        run_alarm_tick(&last_minute);
        glib::ControlFlow::Continue
    });
}

/// Re-arm the tick from any thread (the Snooze action runs on a worker).
pub(super) fn wake_alarm_scheduler() {
    glib::idle_add_once(ensure_alarm_timer);
}

/// Watch the alarm list so enabling an alarm brings the tick back.
pub(super) fn spawn_alarm_scheduler() {
    Effect::new(|_| {
        // Tracked read: re-runs whenever an alarm is added, enabled, or edited.
        let _alarms = config_manager().config().alarm().alarms().get();
        ensure_alarm_timer();
    });
}

pub(super) fn run_alarm_tick(last_minute: &std::rc::Rc<std::cell::Cell<i64>>) {
    let Some(now) = glib::DateTime::now_local().ok() else {
        return;
    };

    // Due snooze re-fires (every tick, independent of the per-minute gate).
    let sys_now = std::time::SystemTime::now();
    let due = {
        let mut q = ALARM_SNOOZES.lock().unwrap_or_else(|e| e.into_inner());
        let due = q.iter().filter(|t| **t <= sys_now).count();
        q.retain(|t| *t > sys_now);
        due
    };
    if due > 0 {
        fire_alarm("Snoozed alarm".to_string());
    }

    // Scheduled alarms — fire at most once per clock minute.
    let minute_key = now.to_unix() / 60;
    if minute_key == last_minute.get() {
        return;
    }
    last_minute.set(minute_key);

    let hour = now.hour() as u8;
    let minute = now.minute() as u8;
    let weekday = (now.day_of_week() % 7) as u8; // 0 = Sunday … 6 = Saturday

    let alarms = config_manager().config().alarm().alarms().get_untracked();
    for (i, alarm) in alarms.iter().enumerate() {
        if !alarm.enabled || alarm.hour != hour || alarm.minutes != minute {
            continue;
        }
        if alarm.repeat_mask != 0 && (alarm.repeat_mask & (1 << weekday)) == 0 {
            continue; // repeating, but not on today's weekday
        }
        let label = if alarm.name.trim().is_empty() {
            "Alarm".to_string()
        } else {
            alarm.name.clone()
        };
        fire_alarm(label);
        if alarm.repeat_mask == 0 {
            // One-shot: disable after it fires.
            config_manager().update_config(move |c| {
                if let Some(a) = c.alarm.alarms.get_mut(i) {
                    a.enabled = false;
                }
            });
        }
    }
}

/// Ring settings shared by alarms and timers, read on the main thread.
struct RingSettings {
    notifications: bool,
    snooze_secs: u64,
    urgency: Urgency,
}

fn ring_settings() -> RingSettings {
    let alarm = config_manager().config().alarm();
    RingSettings {
        notifications: alarm.clone().notifications().get_untracked(),
        snooze_secs: (alarm.clone().snooze_minutes().get_untracked().max(1) as u64) * 60,
        urgency: Urgency::from_name(&alarm.urgency().get_untracked()),
    }
}

/// The Stop / Snooze notification for a ringing alarm or timer. It never
/// expires on its own: the tone stops when the user answers it.
pub(super) fn ring_notice(summary: &str, body: &str, urgency: Urgency) -> Notice {
    Notice::new("Alarm Clock", summary, body)
        .icon("alarm-symbolic")
        .urgency(urgency)
        .timeout_ms(0)
        .action("stop", "Stop")
        .action("snooze", "Snooze")
        // The looping tone is the sound.
        .silent()
}

/// Ring the tone + (optionally) pop a Stop/Snooze notification. The
/// notification blocks a worker thread until the user acts; the tone stops on
/// any outcome, and Snooze re-queues a fire `snooze_minutes` later.
pub(super) fn fire_alarm(label: String) {
    play_alarm_loop();
    let settings = ring_settings();
    if !settings.notifications {
        return;
    }
    std::thread::spawn(move || {
        let action = desktop::ask(ring_notice(&label, "It's time.", settings.urgency));
        stop_alarm(); // stop on every outcome (Stop / Snooze / dismiss)
        if action.as_deref() == Some("snooze")
            && let Ok(mut q) = ALARM_SNOOZES.lock()
        {
            q.push(
                std::time::SystemTime::now() + std::time::Duration::from_secs(settings.snooze_secs),
            );
            drop(q);
            // The tick may have removed itself (a one-shot alarm disables
            // itself when it fires), and this runs on a worker. Re-arm it.
            wake_alarm_scheduler();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ring_notification_has_stop_and_snooze_and_never_expires() {
        let n = ring_notice("tea", "10 min timer is up.", Urgency::Critical);
        assert_eq!(n.app, "Alarm Clock");
        assert_eq!(n.timeout_ms, Some(0));
        let ids: Vec<&str> = n.actions.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, vec!["stop", "snooze"]);
        assert_eq!(n.urgency, Urgency::Critical);
    }
}
