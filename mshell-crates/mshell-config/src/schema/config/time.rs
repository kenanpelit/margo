//! The time widgets: the Alarm Clock's alarms and countdowns, and the
//! Pomodoro cycle (Alarm Clock menu's Pomodoro tab + the `Pomodoro` bar
//! pill + `mshellctl pomodoro`), and the 20-20-20 eye-care reminder
//! (`EyeCare` bar pill + `mshellctl eyecare`).

use super::*;

/// One configured alarm. `repeat_mask` bit `i` (0 = Sunday … 6 = Saturday)
/// marks a repeating weekday; mask 0 = a one-shot alarm that disables itself
/// after firing. Snooze state is runtime-only (not persisted).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct Alarm {
    pub hour: u8,
    pub minutes: u8,
    pub name: String,
    pub enabled: bool,
    pub repeat_mask: u8,
}

impl Default for Alarm {
    fn default() -> Self {
        Self {
            hour: 7,
            minutes: 0,
            name: String::new(),
            enabled: false,
            repeat_mask: 0,
        }
    }
}

/// One countdown target — a passive "time until / since a date" entry
/// surfaced by the Countdown tab of the Alarm Clock menu and the
/// `countdown` bar pill. Unlike an [`Alarm`] it never rings; it is a
/// display only. `unit` is a validated string (`hours` | `days` |
/// `weeks` | `months`) parsed by the shell's `countdown` calc layer,
/// mirroring how [`AlarmConfig::urgency`] is stored.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct Countdown {
    /// Target date, `YYYY-MM-DD [HH:MM]` (time optional → midnight).
    pub target: String,
    /// `hours` | `days` | `weeks` | `months`.
    pub unit: String,
    /// Suffix shown after the unit; empty → "remaining" (or "overdue").
    pub label: String,
    pub enabled: bool,
}

impl Default for Countdown {
    fn default() -> Self {
        Self {
            target: String::new(),
            unit: "days".to_string(),
            label: String::new(),
            enabled: true,
        }
    }
}

/// Alarm-clock widget config: the alarm list plus ring behaviour, and
/// the (ring-less) countdown list shared with the Countdown tab + pill.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct AlarmConfig {
    pub alarms: Vec<Alarm>,
    /// Countdown targets (Countdown tab + `countdown` bar pill). Passive
    /// displays; no scheduler or tone.
    pub countdowns: Vec<Countdown>,
    /// Minutes to snooze when the alarm notification's Snooze action is hit.
    pub snooze_minutes: u32,
    /// Pop a desktop notification (with Stop / Snooze actions) when ringing.
    pub notifications: bool,
    /// Notification urgency: `low` | `normal` | `critical`.
    pub urgency: String,
    /// While a one-shot timer / reminder (`mshellctl timer`, the Timers
    /// tab, the launcher's `timer` prefix) is pending, the AlarmClock bar
    /// pill shows the soonest one's time left. With none pending the pill
    /// is unchanged, so this is on by default.
    pub show_timer_on_pill: bool,
}

impl Default for AlarmConfig {
    fn default() -> Self {
        Self {
            alarms: Vec::new(),
            countdowns: Vec::new(),
            snooze_minutes: 5,
            notifications: true,
            urgency: "normal".to_string(),
            show_timer_on_pill: true,
        }
    }
}

/// Pomodoro cycle settings. Runtime state (phase, time left, sessions done)
/// lives in the shell and is never persisted; these are only the rules.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct Pomodoro {
    /// Length of a focus (work) session in minutes.
    pub work_minutes: u32,
    /// Length of a short break in minutes.
    pub short_break_minutes: u32,
    /// Length of the long break in minutes.
    pub long_break_minutes: u32,
    /// Work sessions per round; the long break follows the last one.
    pub sessions_before_long_break: u32,
    /// Start each break by itself when a work session ends.
    pub auto_start_breaks: bool,
    /// Start the next work session by itself when a break ends.
    pub auto_start_work: bool,
    /// Pop a desktop notification when a phase ends.
    pub notifications: bool,
    /// Play the timer chime when a phase ends.
    pub sound: bool,
}

impl Default for Pomodoro {
    fn default() -> Self {
        Self {
            work_minutes: 25,
            short_break_minutes: 5,
            long_break_minutes: 15,
            sessions_before_long_break: 4,
            auto_start_breaks: false,
            auto_start_work: false,
            notifications: true,
            sound: true,
        }
    }
}

/// 20-20-20 eye-care reminder: after `active_minutes` of screen time, look
/// at something ~6 m (20 ft) away for `break_seconds`. Only time at the
/// keyboard counts; `away_seconds` without input is a natural break and
/// resets the count. Off by default — nothing runs until it is enabled
/// (Settings → Widgets → Eye Care, the pill, or `mshellctl eyecare on`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct EyeCare {
    /// Run the reminder.
    pub enabled: bool,
    /// Minutes of active screen time between breaks.
    pub active_minutes: u32,
    /// Length of a break in seconds.
    pub break_seconds: u32,
    /// Seconds without keyboard / mouse input after which you count as
    /// away (never shorter than a break).
    pub away_seconds: u32,
    /// Pop a desktop notification when a break starts and ends.
    pub notifications: bool,
    /// Chime when a break starts and ends (skipped under Do Not Disturb).
    pub sound: bool,
}

impl Default for EyeCare {
    fn default() -> Self {
        Self {
            enabled: false,
            active_minutes: 20,
            break_seconds: 20,
            away_seconds: 120,
            notifications: true,
            sound: true,
        }
    }
}
