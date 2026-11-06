//! Valent (KDE Connect) integration settings.

use super::*;

/// Per-device cosmetic override, edited inline in the Valent panel
/// (click the device name to rename, click the avatar to pick an
/// image). Purely local — Valent's own D-Bus surface has no concept
/// of an alias or avatar, so these never leave this config file.
#[derive(
    Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize, Store, Patch, JsonSchema,
)]
#[serde(default)]
pub struct ValentDeviceOverride {
    pub device_id: String,
    /// Display name shown instead of the D-Bus-reported name. Empty
    /// means "use the reported name".
    pub alias: String,
    /// Filesystem path to a custom avatar image. Empty means "use the
    /// generic phone icon".
    pub image_path: String,
}

/// Valent (KDE Connect) integration settings. `main_device_id` is the
/// sticky device the bar pill + panel default to when several phones
/// are paired; empty means "auto-pick the first reachable one".
///
/// Deliberately a hand-written `impl Default` rather than
/// `#[derive(Default)]`: a profile baked before this section existed
/// (or one that never touched Valent settings) has no top-level
/// `valent:` key at all, so `Config`'s own `#[serde(default)]` falls
/// back to `Valent::default()` for the *whole struct* — the per-field
/// `#[serde(default = "…")]` attributes below only apply when the
/// `valent:` block is present but an individual field is missing.
/// A derived `Default` would silently give `poll_interval_secs: 0` and
/// `show_battery_percent: false` in that whole-block-missing case
/// (see the mdash/lyrics precedent in git history for the same trap).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Store, Patch, JsonSchema)]
#[serde(default)]
pub struct Valent {
    pub main_device_id: String,
    /// Seconds between device polls (battery/connectivity refresh).
    /// The bar pill's first probe always lands ~4s after launch
    /// regardless of this value.
    #[serde(default = "default_valent_poll_interval_secs")]
    pub poll_interval_secs: u32,
    /// Show the battery percentage label next to the bar pill's icon.
    #[serde(default = "default_true")]
    pub show_battery_percent: bool,
    /// Per-device alias/avatar overrides, keyed by matching `device_id`.
    pub devices: Vec<ValentDeviceOverride>,
    /// Screen mirroring (scrcpy): keep the phone's own screen off while
    /// mirroring. Off by default.
    pub scrcpy_screen_off: bool,
    /// Extra scrcpy arguments (whitespace-separated, no shell), e.g.
    /// `--max-size=1280 --stay-awake`. Empty by default.
    pub scrcpy_args: String,
}

impl Default for Valent {
    fn default() -> Self {
        Self {
            main_device_id: String::new(),
            poll_interval_secs: default_valent_poll_interval_secs(),
            show_battery_percent: true,
            devices: Vec::new(),
            scrcpy_screen_off: false,
            scrcpy_args: String::new(),
        }
    }
}

fn default_valent_poll_interval_secs() -> u32 {
    5
}
