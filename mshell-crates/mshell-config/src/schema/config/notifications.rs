//! `notifications.*` — popup toasts, history, sounds, quiet hours.

use super::bars::default_true;
use crate::schema::position::NotificationPosition;
use reactive_stores::{KeyMap, Patch, PatchField, Store, StorePath};
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
    /// Per-app / per-contact sounds: the first rule whose pattern matches
    /// replaces the built-in chime (and an app-supplied sound) for that
    /// notification. The usual gates (master switch, quiet hours,
    /// per-urgency toggles, `suppress-sound`) still apply. Empty by default.
    #[serde(default)]
    pub sound_rules: Vec<SoundRule>,
}

/// Which notification text a [`SoundRule`] pattern is matched against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SoundRuleField {
    /// App name, title (summary) or body.
    #[default]
    Any,
    /// The sending app's name only (a mention in another app's text
    /// can't trigger it).
    App,
    /// The title (summary) — usually the contact / group name.
    Summary,
    /// The body text.
    Body,
}

impl SoundRuleField {
    pub fn all() -> &'static [Self] {
        &[Self::Any, Self::App, Self::Summary, Self::Body]
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Any => "App, title or body",
            Self::App => "App name",
            Self::Summary => "Title",
            Self::Body => "Body",
        }
    }
}

impl PatchField for SoundRuleField {
    fn patch_field(
        &mut self,
        new: Self,
        path: &StorePath,
        notify: &mut dyn FnMut(&StorePath),
        _keys: Option<&KeyMap>,
    ) {
        if *self != new {
            *self = new;
            notify(path);
        }
    }
}

/// One `notifications.sound_rules` entry.
#[derive(
    Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize, Store, Patch, JsonSchema,
)]
#[serde(default)]
pub struct SoundRule {
    /// Case-insensitive substring to look for. An empty pattern never
    /// matches.
    pub pattern: String,
    /// Where to look for `pattern`.
    pub field: SoundRuleField,
    /// Sound file to play, or a folder: one of its audio files
    /// (ogg / oga / mp3) is picked at random each time.
    pub sound: String,
}

impl SoundRule {
    /// Whether this rule applies to a notification.
    pub fn matches(&self, app: &str, summary: &str, body: &str) -> bool {
        let needle = self.pattern.trim().to_lowercase();
        if needle.is_empty() || self.sound.trim().is_empty() {
            return false;
        }
        let hit = |s: &str| s.to_lowercase().contains(&needle);
        match self.field {
            SoundRuleField::Any => hit(app) || hit(summary) || hit(body),
            SoundRuleField::App => hit(app),
            SoundRuleField::Summary => hit(summary),
            SoundRuleField::Body => hit(body),
        }
    }
}

/// The first rule (in list order) matching the notification.
pub fn first_sound_rule<'a>(
    rules: &'a [SoundRule],
    app: &str,
    summary: &str,
    body: &str,
) -> Option<&'a SoundRule> {
    rules.iter().find(|r| r.matches(app, summary, body))
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
            sound_rules: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(pattern: &str, field: SoundRuleField) -> SoundRule {
        SoundRule {
            pattern: pattern.into(),
            field,
            sound: "/tmp/a.ogg".into(),
        }
    }

    #[test]
    fn any_field_matches_app_title_or_body_case_insensitively() {
        let r = rule("alice", SoundRuleField::Any);
        assert!(r.matches("Valent", "Alice Smith", "hi"));
        assert!(r.matches("Valent", "Bob", "tell ALICE"));
        assert!(r.matches("alice-app", "", ""));
        assert!(!r.matches("Valent", "Bob", "hello"));
    }

    #[test]
    fn app_rule_ignores_mentions_in_text() {
        let r = rule("discord", SoundRuleField::App);
        assert!(r.matches("discord", "general", "x"));
        assert!(!r.matches("Firefox", "Discord is down", "discord"));
    }

    #[test]
    fn summary_and_body_rules_look_only_there() {
        assert!(rule("mom", SoundRuleField::Summary).matches("Valent", "Mom", ""));
        assert!(!rule("mom", SoundRuleField::Summary).matches("Valent", "x", "mom"));
        assert!(rule("urgent", SoundRuleField::Body).matches("Mail", "x", "Urgent!"));
        assert!(!rule("urgent", SoundRuleField::Body).matches("urgent", "urgent", ""));
    }

    #[test]
    fn empty_pattern_or_sound_never_matches() {
        assert!(!rule("  ", SoundRuleField::Any).matches("a", "b", "c"));
        let mut r = rule("a", SoundRuleField::Any);
        r.sound = String::new();
        assert!(!r.matches("a", "a", "a"));
    }

    #[test]
    fn first_matching_rule_wins_in_list_order() {
        let mut a = rule("alice", SoundRuleField::Summary);
        a.sound = "/a.ogg".into();
        let mut b = rule("valent", SoundRuleField::App);
        b.sound = "/b.ogg".into();
        let rules = vec![a, b];
        assert_eq!(
            first_sound_rule(&rules, "Valent", "Alice", "").map(|r| r.sound.as_str()),
            Some("/a.ogg")
        );
        assert_eq!(
            first_sound_rule(&rules, "Valent", "Bob", "").map(|r| r.sound.as_str()),
            Some("/b.ogg")
        );
        assert!(first_sound_rule(&rules, "Mail", "Bob", "").is_none());
    }

    #[test]
    fn sound_rules_default_to_empty_and_deserialize_from_yaml() {
        assert!(Notifications::default().sound_rules.is_empty());
        let n: Notifications = yaml_serde::from_str(
            "sound_rules:\n  - pattern: Alice\n    field: summary\n    sound: /s/alice.ogg\n",
        )
        .unwrap();
        assert_eq!(n.sound_rules[0].field, SoundRuleField::Summary);
        assert_eq!(n.sound_rules[0].sound, "/s/alice.ogg");
    }
}
