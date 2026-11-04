//! `notifications.*` — popup toasts, history, sounds, quiet hours.

use super::bars::default_true;
use crate::schema::position::NotificationPosition;
use reactive_stores::{Patch, Store};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct Notifications {
    pub notification_position: NotificationPosition,
    /// App names (substring match, case-insensitive) whose
    /// notifications are silently dropped — the per-app mute list.
    /// Applied to the wayle service's blocklist on startup and
    /// whenever it changes.
    #[serde(default)]
    pub blocklist: Vec<String>,
    /// Show the small close (✕) button in each notification's header.
    /// On by default — it's the primary manual dismiss (a horizontal
    /// swipe also dismisses). Set `false` for a cleaner toast.
    pub show_close_button: bool,
    /// Show the app-provided action buttons (View / Open / Reply / …).
    /// Off by default — these are the large buttons that clutter toasts;
    /// turn on if you act on notifications straight from the popup.
    pub show_action_buttons: bool,
    /// Group the notification history by app: two or more notifications
    /// from the same app collapse into an expandable "App (N)" header.
    /// On by default. Set `false` for a flat, chronological list.
    pub group_notifications: bool,
    /// Width (px) of the corner popup toasts. Independent of the
    /// notification *history* menu width (that lives in
    /// `menus.notification_menu.minimum_width`); this is the
    /// transient toast surface anchored to a screen corner.
    pub popup_width: i32,
    /// Max number of (most-recent) notifications the history menu renders.
    /// Persisted history can grow into the hundreds; rendering all of them
    /// rebuilds a large list model on every open. 0 = unlimited.
    #[serde(default = "default_history_limit")]
    pub history_limit: u32,
    /// Show a shrinking bar across the top of each popup toast counting
    /// down its remaining on-screen time. On by default.
    #[serde(default = "default_true")]
    pub show_timeout_bar: bool,
    /// How long (ms) a popup toast stays on screen before auto-dismiss
    /// (an app-supplied `expire_timeout` shorter than this still wins).
    /// Also the duration the timeout bar animates over.
    #[serde(default = "default_popup_duration_ms")]
    pub popup_duration_ms: u32,
    /// Render a reply entry on popups whose notification carries an
    /// `"inline-reply"` action (KDE-style; Valent / KDE apps / chat
    /// clients). Sending emits `NotificationReplied(id, text)` back to
    /// the app. On by default — it only appears when an app asks for it.
    #[serde(default = "default_true")]
    pub inline_reply: bool,
    /// Master switch for notification sounds (played when a popup toast
    /// appears; DND already suppresses the popups themselves). Off by
    /// default to preserve the silent out-of-the-box behaviour.
    #[serde(default)]
    pub sound_enabled: bool,
    /// Play a sound for low-urgency notifications.
    #[serde(default)]
    pub sound_low: bool,
    /// Play a sound for normal-urgency notifications.
    #[serde(default = "default_true")]
    pub sound_normal: bool,
    /// Play a sound for critical-urgency notifications (brighter tone).
    #[serde(default = "default_true")]
    pub sound_critical: bool,
    /// Honour the spec's `sound-file` hint: when an app supplies its own
    /// sound, play that instead of the built-in chime. The
    /// `suppress-sound` hint is always honoured regardless.
    #[serde(default = "default_true")]
    pub sound_from_client: bool,
    /// Quiet hours: suppress notification sounds (popups still show)
    /// inside the window below.
    #[serde(default)]
    pub quiet_hours_enabled: bool,
    /// Quiet-hours window start, `HH:MM` (24-hour).
    #[serde(default = "default_quiet_start")]
    pub quiet_hours_start: String,
    /// Quiet-hours window end, `HH:MM` (24-hour). An end before the
    /// start wraps past midnight (22:00–08:00).
    #[serde(default = "default_quiet_end")]
    pub quiet_hours_end: String,
    /// Render a progress bar on notifications carrying the spec's
    /// `value` hint (downloads, file transfers, backups).
    #[serde(default = "default_true")]
    pub show_progress: bool,
    /// Turn Do-Not-Disturb on automatically while any screencast /
    /// screen-share is running (the same PipeWire detection the privacy
    /// pill uses) and restore the previous state when the last one ends.
    /// If DND was already on when sharing started it is left alone.
    /// Off by default.
    #[serde(default)]
    pub auto_dnd_screen_share: bool,
    /// How long (ms) sharing must stay stopped before auto-DND releases,
    /// so a stop/start pair (switching the shared window) doesn't flap.
    #[serde(default = "default_auto_dnd_release_ms")]
    pub auto_dnd_release_ms: u32,
}

fn default_auto_dnd_release_ms() -> u32 {
    1000
}

fn default_quiet_start() -> String {
    "22:00".to_string()
}

fn default_quiet_end() -> String {
    "08:00".to_string()
}

fn default_history_limit() -> u32 {
    200
}

fn default_popup_duration_ms() -> u32 {
    5000
}

impl Default for Notifications {
    fn default() -> Self {
        Self {
            notification_position: NotificationPosition::Right,
            blocklist: Vec::new(),
            show_close_button: true,
            show_action_buttons: false,
            group_notifications: true,
            popup_width: 460,
            history_limit: default_history_limit(),
            show_timeout_bar: true,
            popup_duration_ms: default_popup_duration_ms(),
            inline_reply: true,
            sound_enabled: false,
            sound_low: false,
            sound_normal: true,
            sound_critical: true,
            sound_from_client: true,
            quiet_hours_enabled: false,
            quiet_hours_start: default_quiet_start(),
            quiet_hours_end: default_quiet_end(),
            show_progress: true,
            auto_dnd_screen_share: false,
            auto_dnd_release_ms: default_auto_dnd_release_ms(),
        }
    }
}
