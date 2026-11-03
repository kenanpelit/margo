use crossterm::event::{KeyCode, KeyModifiers};
use log::error;
use ratatui::style::{Color, Modifier};
use serde::{Deserialize, Serialize, Serializer, de::Error};
use std::fmt;
use std::process;
use toml::Value;

pub fn get_color(color: &str) -> Color {
    let resolved = if let Some(color) = str_to_color(color) {
        color
    } else {
        error!("Did not recognize the color '{}'", color);
        Color::White
    };
    // On the bare Linux VT this swaps truecolor for a reprogrammed palette
    // slot so login matches `--preview`; in preview it's a pass-through.
    crate::console_palette::map_color(resolved)
}

fn str_to_color(color: &str) -> Option<Color> {
    use Color::*;

    let c = color.to_lowercase();
    Some(match &c[..] {
        // TUI colors
        "black" => Black,
        "red" => Red,
        "green" => Green,
        "yellow" => Yellow,
        "blue" => Blue,
        "magenta" => Magenta,
        "cyan" => Cyan,
        "gray" => Gray,
        "dark gray" => DarkGray,
        "light red" => LightRed,
        "light green" => LightGreen,
        "light yellow" => LightYellow,
        "light blue" => LightBlue,
        "light magenta" => LightMagenta,
        "light cyan" => LightCyan,
        "white" => White,

        // Custom colors
        "orange" => Rgb(255, 127, 0),

        // Hex and unknown
        c => {
            if !c.starts_with('#') || c.len() != 7 {
                return None;
            }

            let r = &c[1..3];
            let g = &c[3..5];
            let b = &c[5..7];

            let r = u8::from_str_radix(r, 16).ok()?;
            let g = u8::from_str_radix(g, 16).ok()?;
            let b = u8::from_str_radix(b, 16).ok()?;

            Rgb(r, g, b)
        }
    })
}

fn get_modifier(modifier: &str) -> Option<Modifier> {
    let m = modifier.trim().to_lowercase();
    Some(match &m[..] {
        "bold" => Modifier::BOLD,
        "dim" => Modifier::DIM,
        "italic" => Modifier::ITALIC,
        "underlined" => Modifier::UNDERLINED,
        "slow blink" => Modifier::SLOW_BLINK,
        "rapid blink" => Modifier::RAPID_BLINK,
        "reversed" => Modifier::REVERSED,
        "crossed out" => Modifier::CROSSED_OUT,
        "hidden" => Modifier::HIDDEN,
        _ => return None,
    })
}

pub fn get_modifiers(modifiers: &str) -> Vec<Modifier> {
    let mut ms = Vec::new();

    for modifier in modifiers.split(',') {
        if let Some(modifier) = get_modifier(modifier) {
            ms.push(modifier);
        }
    }

    ms
}

pub fn get_function_key(key: &str) -> Option<KeyCode> {
    Some(match key.trim() {
        "F1" => KeyCode::F(1),
        "F2" => KeyCode::F(2),
        "F3" => KeyCode::F(3),
        "F4" => KeyCode::F(4),
        "F5" => KeyCode::F(5),
        "F6" => KeyCode::F(6),
        "F7" => KeyCode::F(7),
        "F8" => KeyCode::F(8),
        "F9" => KeyCode::F(9),
        "F10" => KeyCode::F(10),
        "F11" => KeyCode::F(11),
        "F12" => KeyCode::F(12),
        _ => return None,
    })
}

/// Parse a power-key spec such as `F1` or `Ctrl+Shift+F1` into the exact
/// modifier set and function key it names. Modifier names are
/// case-insensitive (`ctrl`/`control`, `shift`, `alt`, `super`/`meta`).
/// `None` for anything else, so a typo binds nothing rather than a bare key.
pub fn parse_key_spec(spec: &str) -> Option<(KeyModifiers, KeyCode)> {
    let mut parts: Vec<&str> = spec.split('+').map(str::trim).collect();
    let key = get_function_key(parts.pop()?)?;
    let mut mods = KeyModifiers::NONE;
    for m in parts {
        mods |= match m.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => KeyModifiers::CONTROL,
            "shift" => KeyModifiers::SHIFT,
            "alt" => KeyModifiers::ALT,
            "super" | "meta" => KeyModifiers::SUPER,
            _ => return None,
        };
    }
    Some((mods, key))
}

macro_rules! partial_struct_field {
    ($field_type:ty) => {
        $field_type
    };
    ($field_type:ty, $par_field_type:ty) => {
        $par_field_type
    };
}

macro_rules! merge_strategy {
    ($self:ident, $dest:ident, $src:expr) => {
        $self.$dest = $src
    };
    ($self:ident, $dest:ident, $src:expr, $_:ty) => {
        $self.$dest.merge_in_partial($src)
    };
}

macro_rules! var_replacement_strategy {
    ($vars:ident, $value:ident, $field_type:ty) => {
        <$field_type as VariableInsertable>::insert($value, $vars)?
    };
    ($vars:ident, $value:ident, $field_type:ty, $_:ty) => {
        $value.into_partial($vars)?
    };
}

macro_rules! toml_config_struct {
    ($struct_name:ident, $partial_struct_name:ident, $rough_name:ident, $($field_name:ident => $field_type:ty $([$par_field_type:ty, $rough_field_type:ty])?),+ $(,)?) => {
        #[derive(Debug, Clone, Deserialize)]
        struct $rough_name {
            $($field_name: Option<partial_struct_field!(PossibleVariable<$field_type>$(, $rough_field_type)?)>,)+
        }
        #[derive(Debug, Clone, Deserialize, Serialize)]
        pub struct $struct_name {
            $(pub $field_name: $field_type,)+
        }
        #[derive(Clone, Deserialize)]
        pub struct $partial_struct_name {
            $(pub $field_name: Option<partial_struct_field!($field_type$(, $par_field_type)?)>,)+
        }
        impl $struct_name {
            pub fn merge_in_partial(&mut self, partial: $partial_struct_name) {
                $(
                if let Some($field_name) = partial.$field_name {
                    merge_strategy!(self, $field_name, $field_name $(, $par_field_type)?);
                }
                )+
            }
        }

        impl $rough_name {
            pub fn into_partial(self, variables: &Variables) -> Result<$partial_struct_name, VariableInsertionError> {
                Ok($partial_struct_name {
                    $(
                    $field_name: match self.$field_name {
                        Some(value) => Some(
                            var_replacement_strategy!(variables, value, $field_type$(, $par_field_type)?)
                        ),
                        None => None,
                    },
                    )+
                })
            }
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(transparent)]
pub struct Variables(toml::value::Table);

toml_config_struct! { Config, PartialConfig, RoughConfig,
    tty => u8,

    main_log_path => String,
    client_log_path => String,
    cache_path => String,

    do_log => bool,

    pam_service => String,

    // The unprivileged system user the greeter (margo+mgreet, or cage+foot) runs
    // as. Its logind session is created either way; only the setuid is
    // conditional. Empty string → the greeter stays root. A user named here but
    // absent from /etc/passwd logs an error and the graphical greeter refuses to
    // start, so the daemon falls back to the TTY greeter: people can still log
    // in, and the GTK stack never runs as root by accident.
    greeter_user => String,
    // PAM stack used to open the greeter's logind session. Authenticates nothing
    // (pam_permit); pam_systemd in its session phase is the point.
    greeter_pam_service => String,

    system_shell => String,
    initial_path => String,

    shell_login_flag => ShellLoginFlag,

    focus_behaviour => FocusBehaviour,

    background => BackgroundConfig [PartialBackgroundConfig, RoughBackgroundConfig],

    power_controls => PowerControlConfig [PartialPowerControlConfig, RoughPowerControlConfig],
    environment_switcher => SwitcherConfig [PartialSwitcherConfig, RoughSwitcherConfig],
    username_field => UsernameFieldConfig [PartialUsernameFieldConfig, RoughUsernameFieldConfig],
    password_field => PasswordFieldConfig [PartialPasswordFieldConfig, RoughPasswordFieldConfig],

    x11 => X11Config [PartialX11Config, RoughX11Config],
    wayland => WaylandConfig [PartialWaylandConfig, RoughWaylandConfig],

    display => DisplayConfig [PartialDisplayConfig, RoughDisplayConfig],

    autologin => AutologinConfig [PartialAutologinConfig, RoughAutologinConfig],
}

toml_config_struct! { BackgroundStyleConfig, PartialBackgroundStyleConfig, RoughBackgroundStyleConfig,
    color => String,
    show_border => bool,
    border_color => String,
}

toml_config_struct! { BackgroundConfig, PartialBackgroundConfig, RoughBackgroundConfig,
    show_background => bool,
    style => BackgroundStyleConfig [PartialBackgroundStyleConfig, RoughBackgroundStyleConfig],
}

toml_config_struct! { PowerControlConfig, PartialPowerControlConfig, RoughPowerControlConfig,
    hint_margin => u16,
    // Run power actions even while another user is logged in or a `block`
    // inhibitor covers them. Off: the greeter refuses, like logind does for an
    // unprivileged caller (see runner::power_guard).
    ignore_inhibitors => bool,
    base_entries => PowerControlVec [PartialPowerControlVec, RoughPowerControlVec],
    entries => PowerControlVec [PartialPowerControlVec, RoughPowerControlVec],
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(transparent)]
#[repr(transparent)]
pub struct PowerControlVec(pub Vec<PowerControl>);
#[derive(Clone, Deserialize)]
#[serde(transparent)]
#[repr(transparent)]
pub struct PartialPowerControlVec(pub Vec<PartialPowerControl>);
#[derive(Clone, Debug, Deserialize)]
#[serde(transparent)]
#[repr(transparent)]
struct RoughPowerControlVec(pub Vec<RoughPowerControl>);

toml_config_struct! { PowerControl, PartialPowerControl, RoughPowerControl,
    hint => String,
    hint_color => String,
    hint_modifiers => String,
    key => String,
    cmd => String,
}

impl Default for PowerControl {
    fn default() -> Self {
        PowerControl {
            hint: "".to_string(),
            hint_color: "dark gray".to_string(),
            hint_modifiers: "".to_string(),
            key: "".to_string(),
            cmd: "true".to_string(),
        }
    }
}

toml_config_struct! { SwitcherConfig, PartialSwitcherConfig, RoughSwitcherConfig,
    switcher_visibility => SwitcherVisibility,
    toggle_hint => String,
    toggle_hint_color => String,
    toggle_hint_modifiers => String,

    include_tty_shell => bool,

    remember => bool,

    show_movers => bool,
    mover_color => String,
    mover_color_focused => String,

    mover_modifiers => String,
    mover_modifiers_focused => String,

    left_mover => String,
    right_mover => String,

    mover_margin => u16,

    selected_color => String,
    selected_color_focused => String,

    selected_modifiers => String,
    selected_modifiers_focused => String,

    show_neighbours => bool,
    neighbour_color => String,
    neighbour_color_focused => String,

    neighbour_modifiers => String,
    neighbour_modifiers_focused => String,

    neighbour_margin => u16,

    max_display_length => u16,

    no_envs_text => String,

    no_envs_color => String,
    no_envs_color_focused => String,

    no_envs_modifiers => String,
    no_envs_modifiers_focused => String,
}

toml_config_struct! { InputFieldStyle, PartialInputFieldStyle, RoughInputFieldStyle,
    show_title => bool,
    title => String,

    show_border => bool,

    title_color => String,
    title_color_focused => String,

    content_color => String,
    content_color_focused => String,

    border_color => String,
    border_color_focused => String,

    use_max_width => bool,
    max_width => u16,
}

toml_config_struct! { UsernameFieldConfig, PartialUsernameFieldConfig, RoughUsernameFieldConfig,
    remember => bool,
    style => InputFieldStyle [PartialInputFieldStyle, RoughInputFieldStyle],
}

toml_config_struct! { PasswordFieldConfig, PartialPasswordFieldConfig, RoughPasswordFieldConfig,
    content_replacement_character => char,
    style => InputFieldStyle [PartialInputFieldStyle, RoughInputFieldStyle],
}

toml_config_struct! { X11Config, PartialX11Config, RoughX11Config,
    x11_display => String,

    xserver_timeout_secs => u16,

    xserver_log_path => String,

    xserver_path => String,
    xauth_path => String,

    scripts_path => String,
    xsetup_path => String,
    xsessions_path => String,
}

toml_config_struct! { WaylandConfig, PartialWaylandConfig, RoughWaylandConfig,
    scripts_path => String,
    wayland_sessions_path => String,
}

// How the greeter reaches the screen.
//
//   host = "gui" — the greeter (`mgreet`, native GTK4) runs under a throwaway
//     root `margo` instance, so a login card lands on EVERY connected monitor
//     at its own native/preferred mode (cage can't — it has no layer-shell).
//     Needs `margo`, `mgreet`, `mctl`; falls back to "cage" then "tty" if margo
//     can't start. This is the shipped default.
//   host = "cage" — the greeter runs inside a tiny wlroots kiosk compositor
//     (`cage`) hosting a Wayland terminal (`foot`). cage drives KMS directly,
//     so every connected monitor lights up at its own native/preferred mode
//     (dynamic, EDID-derived — no hardcoded resolution), and VT switching
//     (Ctrl+Alt+F<n>) keeps working. The greeter emits the validated login to
//     the root orchestrator, which launches the session on the bare VT.
//   host = "tty" — the classic in-process greeter rendered straight to the
//     Linux VT. Shares one fbcon framebuffer, so multi-monitor falls back to a
//     single common mode. This is also the automatic fallback whenever cage or
//     foot is missing, or cage fails to initialise (never a lockout).
toml_config_struct! { DisplayConfig, PartialDisplayConfig, RoughDisplayConfig,
    host => String,
    // cage output mode (`-m`): "last" confines the greeter to a single output
    // (the last-connected one) so the terminal fills exactly one monitor and the
    // login block centres within it; "extend" spans every output as one logical
    // surface (which pushes the centred block to the monitor seam). "last" also
    // adapts to hotplug: unplug the external and the greeter follows to the
    // laptop panel.
    output_mode => String,
    // Seconds of no keyboard or pointer activity before the "gui" greeter paints
    // itself black and forgets anything typed into the password field. Any key,
    // click or movement brings it back, and that first key wakes the screen
    // rather than landing in the field. `0` disables it. The "cage" and "tty"
    // hosts have no such surface and ignore it.
    blank_timeout => u16,
    // Ask the kernel for the first free VT at startup (`VT_OPENQRY`) instead of
    // taking the top-level `tty` number as-is. `tty` stays the fallback when the
    // query fails, and an explicit `--tty N` on the command line always wins.
    // Off by default: a fixed, predictable VT is the right call on a machine
    // whose gettys are laid out around it.
    dynamic_vt => bool,
    // A directory of images the "gui" greeter draws one of at random, per
    // greeter start (atrium's idiom). The runner picks the file, so the
    // compositor wallpaper and mgreet's backdrop always agree. Empty (the
    // default), unreadable, or holding nothing decodable → the baked blurred
    // copy of the last user's wallpaper, as before.
    background_dir => String,
    // Absolute path to a CSS file layered over the "gui" greeter's palette
    // (baked Dracula, then matugen, then this). The natural content is
    // `:root { --primary: …; }` token overrides. Empty (the default) → off.
    greeter_css => String,
    // Float the mkeys on-screen keyboard over the "gui" greeter for touch
    // login. Needs `mkeys` installed; silently skipped when it is not.
    osk => bool,
}

// Log in one user automatically, once per boot, before any greeter appears.
// Both `user` and `session` non-empty → enabled. The session phase of
// `pam_service` must be a real login stack (logind session, loginuid); its
// auth phase is `pam_permit`, because nobody is at the keyboard to answer.
// When the autologin session ends — or fails to start — the normal greeter
// takes over, so logging out re-greets rather than looping straight back in.
toml_config_struct! { AutologinConfig, PartialAutologinConfig, RoughAutologinConfig,
    user => String,
    session => String,
    pam_service => String,
}

impl AutologinConfig {
    /// Autologin is opt-in twice over: a user AND a session must be named.
    pub fn enabled(&self) -> bool {
        !self.user.is_empty() && !self.session.is_empty()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum FocusBehaviour {
    #[serde(rename = "default")]
    FirstNonCached,
    #[serde(rename = "no-focus")]
    NoFocus,
    #[serde(rename = "environment")]
    Environment,
    #[serde(rename = "username")]
    Username,
    #[serde(rename = "password")]
    Password,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum ShellLoginFlag {
    #[serde(rename = "none")]
    None,
    #[serde(rename = "short")]
    Short,
    #[serde(rename = "long")]
    Long,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SwitcherVisibility {
    Visible,
    Hidden,
    Keybind(KeyCode),
}
impl Serialize for SwitcherVisibility {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match &self {
            SwitcherVisibility::Visible => serializer.serialize_str("visible"),
            SwitcherVisibility::Hidden => serializer.serialize_str("hidden"),
            SwitcherVisibility::Keybind(kc) => match kc {
                KeyCode::F(i) => serializer.serialize_str(format!("F{}", i).as_str()),
                _ => serializer.serialize_str("error"),
            },
        }
    }
}

/// Deserialise from a string of "visible", "hidden", or the keybind ("F1"-"F12")
impl<'de> Deserialize<'de> for SwitcherVisibility {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s: String = Deserialize::deserialize(deserializer)?;

        Ok(match s.as_str() {
            "visible" => Self::Visible,
            "hidden" => Self::Hidden,
            key => {
                let Some(keycode) = get_function_key(key) else {
                    return Err(D::Error::custom(
                        "Invalid key provided to toggle switcher visibility. Only F1-F12 are allowed",
                    ));
                };

                Self::Keybind(keycode)
            }
        })
    }
}

impl Default for Config {
    fn default() -> Config {
        // The baked theme + layout live entirely in-code: `extra/config.toml`
        // is the structural base (every field present, colours still carrying
        // their `$var` placeholders) and `extra/variables.toml` is margo's
        // baseline Dracula palette. We substitute the baked variables into the
        // baked config here so `Config::default()` is a *fully resolved* theme
        // — the greeter renders margo's palette even with no `/etc/mlogind`
        // files synced. A real `/etc` config/variables pair just overlays.
        let text = include_str!("../extra/config.toml");
        let mut config: Config = toml::from_str(text).unwrap_or_else(|e| {
            eprintln!("Default configuration file cannot be properly parsed: {e}");
            process::exit(1);
        });

        let variables = Variables::baked_default();
        match toml::from_str::<RoughConfig>(text).map(|r| r.into_partial(&variables)) {
            Ok(Ok(partial)) => config.merge_in_partial(partial),
            Ok(Err(e)) => eprintln!("Baked theme variables could not be applied: {e}"),
            Err(e) => eprintln!("Baked config could not be re-parsed for theming: {e}"),
        }

        config
    }
}

impl PartialConfig {
    /// Parse `config.toml` and substitute the first variable set that works.
    ///
    /// `candidates` runs from most to least specific — synced palette over the
    /// admin's variables, the admin's alone, the baked defaults. A substitution
    /// error (a wrongly typed value, an unset variable, a self-reference past the
    /// depth limit) then costs the palette, not the configuration: the old code
    /// gave up on `config.toml` altogether and fell back to the built-in config,
    /// silently losing the administrator's power keys, `pam_service`, autologin,
    /// TTY and greeter user. Returns the index of the candidate used. A file that
    /// is not valid TOML is still an error — no palette fixes that.
    pub fn from_str_with_variables(
        contents: &str,
        candidates: &[&Variables],
    ) -> Result<(PartialConfig, usize), Box<dyn std::error::Error>> {
        let mut last_err: Option<VariableInsertionError> = None;
        for (index, variables) in candidates.iter().enumerate() {
            let rough = toml::from_str::<RoughConfig>(contents)?;
            match rough.into_partial(variables) {
                Ok(partial) => return Ok((partial, index)),
                Err(err) => last_err = Some(err),
            }
        }
        match last_err {
            Some(err) => Err(err.into()),
            None => Ok((toml::from_str::<PartialConfig>(contents)?, 0)),
        }
    }
}

/// Colour keys a synced (user-written) palette may set: exactly the colours
/// `extra/variables.toml` defines and `extra/config.toml` references.
const SYNCED_COLOR_KEYS: &[&str] = &[
    "bg",
    "panel_border",
    "field_border",
    "text",
    "subtext",
    "muted",
    "accent",
    "accent_soft",
    "danger",
];
/// Text keys a synced palette may set. They become field titles on the login
/// screen, which is why they are held to [`is_plain_title`].
const SYNCED_TITLE_KEYS: &[&str] = &["login_title", "password_title"];
/// Long enough for "Benutzername" or "Contraseña", too short for a sentence
/// telling the next person at the screen what to type.
const MAX_TITLE_CHARS: usize = 16;

/// `#rrggbb` — the only form matugen writes and [`str_to_color`] parses as hex.
fn is_hex_color(value: &str) -> bool {
    value.len() == 7 && value.starts_with('#') && value[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

/// A short label: letters, digits, spaces and a little punctuation, no control
/// characters, no `$` (it would start a variable reference).
fn is_plain_title(value: &str) -> bool {
    let count = value.chars().count();
    (1..=MAX_TITLE_CHARS).contains(&count)
        && value.trim() == value
        && value
            .chars()
            .all(|c| c.is_alphanumeric() || c == ' ' || "-_.'()".contains(c))
}

/// The header margo's matugen wrote at the top of
/// `~/.config/margo/mlogind-variables.toml` while `mlogind sync-theme` still
/// copied that file byte for byte to `/etc/mlogind/variables.toml` (before
/// 4.5.1). Neither the shipped `extra/variables.toml` nor the current matugen
/// file (which names `/var/lib/mgreet`) carries it, and a file that does said
/// of itself "overwritten on every wallpaper/theme change", so an admin's edit
/// to it never survived the next sync. Its presence in `/etc` proves the old
/// sync put it there — with whatever the last user wrote.
const OLD_SYNC_HEADER: [&str; 3] = [
    "# Generated by margo's matugen — mlogind colour variables.",
    "# Overwritten on every wallpaper/theme change; `mlogind sync-theme`",
    "# copies this into /etc/mlogind/variables.toml for the greeter.",
];

/// Was this `/etc/mlogind/variables.toml` written by the old theme sync? Only
/// an exact [`OLD_SYNC_HEADER`] at the very top counts: anything else is the
/// administrator's.
pub fn written_by_old_sync(text: &str) -> bool {
    let mut lines = text.lines().map(|line| line.trim_end_matches('\r'));
    OLD_SYNC_HEADER
        .iter()
        .all(|header| lines.next() == Some(*header))
}

/// What the administrator's variables file turned out to be.
#[derive(Debug)]
pub enum AdminVariables {
    /// Theirs: trusted as written.
    Admin(Variables),
    /// A copy the old theme sync left behind. It is user-written, so it is
    /// not the admin's palette: what survives [`Variables::sanitize_synced`]
    /// (`None` when nothing does) stands in for the synced palette until the
    /// next sync publishes a real one.
    OldSync(Option<Variables>),
}

impl AdminVariables {
    /// Classify the text of `/etc/mlogind/variables.toml`.
    pub fn from_text(text: &str) -> Result<AdminVariables, toml::de::Error> {
        if written_by_old_sync(text) {
            return Ok(AdminVariables::OldSync(Variables::sanitize_synced(text)));
        }
        toml::from_str(text).map(AdminVariables::Admin)
    }
}

impl Variables {
    /// Keep the trustworthy part of a palette someone other than the
    /// administrator wrote — `~/.config/margo/mlogind-variables.toml`, copied
    /// out of the last user's home by `theme_sync`.
    ///
    /// Only the known keys survive, and only with a value of the right shape:
    /// a `#rrggbb` string for colours, a short plain title for the field
    /// titles. Anything else (`bg = 1`, `accent = "$accent"`, a key the config
    /// never references, a paragraph of text in `password_title`) is dropped,
    /// so substituting the result into `config.toml` cannot fail because of it
    /// and the greeter's labels can't be turned into instructions. `None` when
    /// the text is not TOML or nothing valid is left.
    pub fn sanitize_synced(text: &str) -> Option<Variables> {
        let table: toml::value::Table = toml::from_str(text).ok()?;
        let kept: toml::value::Table = table
            .into_iter()
            .filter(|(key, value)| {
                let Some(value) = value.as_str() else {
                    return false;
                };
                if SYNCED_COLOR_KEYS.contains(&key.as_str()) {
                    is_hex_color(value)
                } else if SYNCED_TITLE_KEYS.contains(&key.as_str()) {
                    is_plain_title(value)
                } else {
                    false
                }
            })
            .collect();
        (!kept.is_empty()).then_some(Variables(kept))
    }

    /// Serialise for publishing. Every value is a validated string, so this
    /// only fails on an allocation failure; an empty string is the safe answer.
    pub fn to_toml_string(&self) -> String {
        toml::to_string(&self.0).unwrap_or_default()
    }

    /// `other`'s keys win over ours.
    pub fn overlay(&mut self, other: Variables) {
        self.0.extend(other.0);
    }

    /// margo's baseline Dracula palette, baked into the binary from
    /// `extra/variables.toml`. Used to fully resolve the in-code default theme
    /// and as the fallback when no `/etc/mlogind/variables.toml` exists, so
    /// the greeter never falls back to the bare ratatui defaults.
    pub fn baked_default() -> Variables {
        toml::from_str(include_str!("../extra/variables.toml"))
            .expect("baked extra/variables.toml must parse")
    }
}

trait VariableInsertable: Sized {
    const DEPTH_LIMIT: u32 = 10;

    fn insert(
        possible: PossibleVariable<Self>,
        variables: &Variables,
    ) -> Result<Self, VariableInsertionError> {
        Self::insert_with_depth(possible, variables, 0)
    }
    fn insert_with_depth(
        value: PossibleVariable<Self>,
        variables: &Variables,
        depth: u32,
    ) -> Result<Self, VariableInsertionError>;
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum PossibleVariable<T> {
    Value(T),
    Variable(String),
}

impl<'de, T: Deserialize<'de>> TryFrom<toml::Value> for PossibleVariable<T> {
    type Error = &'static str;

    fn try_from(value: toml::Value) -> Result<Self, Self::Error> {
        if let Ok(i) = value.clone().try_into() {
            return Ok(Self::Value(i));
        }

        match value {
            Value::String(s) => Ok(PossibleVariable::Variable(s)),
            v => Err(v.type_str()),
        }
    }
}

#[derive(Debug)]
enum VariableInsertionError {
    ImpossibleVariableCast {
        var_ident: String,
        expected_type: &'static str,
    },
    UnsetVariable {
        var_ident: String,
    },
    DepthLimitReached,
    InvalidType {
        expected: &'static str,
        gotten: &'static str,
    },
    UnexpectedVariableType {
        var_ident: String,
        expected_type: &'static str,
    },
}

impl fmt::Display for VariableInsertionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VariableInsertionError::ImpossibleVariableCast {
                var_ident,
                expected_type,
            } => write!(
                f,
                "Impossible to use variable '{var_ident}' in string to cast to '{expected_type}'"
            ),
            VariableInsertionError::UnsetVariable { var_ident } => {
                write!(f, "Variable '{var_ident}' is not set")
            }
            VariableInsertionError::DepthLimitReached => {
                write!(f, "Variable evaluation reached the depth limit")
            }
            VariableInsertionError::InvalidType { expected, gotten } => {
                write!(f, "Expected type '{expected}'. Got type '{gotten}'.")
            }
            VariableInsertionError::UnexpectedVariableType {
                var_ident,
                expected_type,
            } => write!(
                f,
                "Needed to use variable '{var_ident}' as a '{expected_type}', but was unable to cast it as such."
            ),
        }
    }
}

impl PowerControlVec {
    pub fn merge_in_partial(&mut self, partial: PartialPowerControlVec) {
        *self = PowerControlVec(
            partial
                .0
                .into_iter()
                .map(|partial_elem| {
                    let mut elem = PowerControl::default();
                    elem.merge_in_partial(partial_elem);
                    elem
                })
                .collect::<Vec<PowerControl>>(),
        );
    }
}

impl RoughPowerControlVec {
    pub fn into_partial(
        self,
        variables: &Variables,
    ) -> Result<PartialPowerControlVec, VariableInsertionError> {
        self.0
            .into_iter()
            .map(|rough_elem| rough_elem.into_partial(variables))
            .collect::<Result<Vec<PartialPowerControl>, VariableInsertionError>>()
            .map(PartialPowerControlVec)
    }
}

impl std::error::Error for VariableInsertionError {}

macro_rules! non_string_var_insert {
    ($($type:ty [$type_str:literal]),+ $(,)?) => {
        $(
        impl VariableInsertable for $type {
            fn insert_with_depth(
                value: PossibleVariable<Self>,
                variables: &Variables,
                depth: u32,
            ) -> Result<Self, VariableInsertionError> {
                use VariableInsertionError as E;

                if depth == Self::DEPTH_LIMIT {
                    return Err(E::DepthLimitReached);
                }

                match value {
                    PossibleVariable::Variable(s) => {
                        // Ignore surrounding spaces
                        let s = s.trim();

                        let mut variter = VariableIterator::new(&s);

                        // No variable in string
                        let var = variter.next().ok_or(E::InvalidType {
                            expected: $type_str,
                            gotten: "string",
                        })?;

                        // Not whole string is variable
                        if var.span() != (0..s.len()) {
                            return Err(E::ImpossibleVariableCast {
                                var_ident: var.ident().to_string(),
                                expected_type: $type_str,
                            });
                        }

                        let value = <PossibleVariable<$type>>::try_from(
                            variables
                                .0
                                .get(var.ident())
                                .ok_or(E::UnsetVariable {
                                    var_ident: var.ident().to_string(),
                                })?
                                .clone(),
                        )
                        .map_err(|_| E::UnexpectedVariableType {
                            var_ident: var.ident().to_string(),
                            expected_type: $type_str,
                        })?;

                        Self::insert_with_depth(value, variables, depth + 1)
                    }
                    PossibleVariable::Value(b) => Ok(b),
                }
            }
        }
        )+
    };
}

non_string_var_insert! {
    bool ["boolean"],
    u8 ["unsigned 8-bit integer"],
    u16 ["unsigned 16-bit integer"],
    char ["character"],
    ShellLoginFlag ["shell login flag"],
    FocusBehaviour ["focus behavior"],
    SwitcherVisibility ["switcher visibility"],
}

impl VariableInsertable for String {
    fn insert_with_depth(
        value: PossibleVariable<Self>,
        variables: &Variables,
        depth: u32,
    ) -> Result<Self, VariableInsertionError> {
        use VariableInsertionError as E;

        if depth == Self::DEPTH_LIMIT {
            return Err(E::DepthLimitReached);
        }

        let mut s = match value {
            PossibleVariable::Value(s) | PossibleVariable::Variable(s) => s,
        };

        while let Some(var) = VariableIterator::new(&s).next() {
            let value = <PossibleVariable<String>>::try_from(
                variables
                    .0
                    .get(var.ident())
                    .ok_or(E::UnsetVariable {
                        var_ident: var.ident().to_string(),
                    })?
                    .clone(),
            )
            .map_err(|_| E::UnexpectedVariableType {
                var_ident: var.ident().to_string(),
                expected_type: "string",
            })?;

            let insertion = Self::insert_with_depth(value.clone(), variables, depth + 1)?;
            s.replace_range(var.span(), &insertion);
        }

        Ok(s)
    }
}

/// Iterator over variables in a given string
/// Assumes the presence of quotes
struct VariableIterator<'a> {
    inner: &'a str,
    offset: usize,
}

struct Variable<'a> {
    start: usize,
    ident: &'a str,
}

impl Variable<'_> {
    const START_SYMBOL: &'static str = "$";

    fn span(&self) -> std::ops::Range<usize> {
        self.start..self.start + Self::START_SYMBOL.len() + self.ident.len()
    }

    fn ident(&self) -> &str {
        self.ident
    }
}

impl<'a> VariableIterator<'a> {
    pub fn new(text: &'a str) -> Self {
        Self {
            inner: text,
            offset: 0,
        }
    }
}
impl<'a> Iterator for VariableIterator<'a> {
    type Item = Variable<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        let s = &self.inner[self.offset..];

        let start = s.find(Variable::START_SYMBOL)?;

        // skip the "$ pattern
        let s = &s[start + Variable::START_SYMBOL.len()..];

        // Find the first not variable token.
        let end = s
            .find(|c: char| !c.is_alphanumeric() && c != '_')
            .unwrap_or(s.len());

        let start = self.offset + start;
        self.offset = start + Variable::START_SYMBOL.len() + end;

        let ident = &s[..end];

        Some(Variable { start, ident })
    }
}

#[cfg(test)]
mod tests {
    use super::{AdminVariables, PartialConfig, VariableIterator, Variables, written_by_old_sync};

    const BAKED_CONFIG: &str = include_str!("../extra/config.toml");

    fn vars(text: &str) -> Variables {
        toml::from_str(text).expect("test variables parse")
    }

    /// The file the old sync left in /etc: the matugen header, then whatever
    /// the user's `mlogind-variables.toml` said.
    fn old_sync_file(body: &str) -> String {
        format!(
            "# Generated by margo's matugen — mlogind colour variables.\n\
             # Overwritten on every wallpaper/theme change; `mlogind sync-theme`\n\
             # copies this into /etc/mlogind/variables.toml for the greeter.\n\
             {body}"
        )
    }

    #[test]
    fn the_old_sync_header_marks_a_file_it_wrote() {
        assert!(written_by_old_sync(&old_sync_file("bg = \"#000000\"\n")));
        assert!(written_by_old_sync(&old_sync_file("")));
        // CRLF endings do not hide it.
        assert!(written_by_old_sync(
            &old_sync_file("").replace('\n', "\r\n")
        ));
    }

    #[test]
    fn admin_files_are_never_taken_for_the_old_sync() {
        // The shipped default, and the current matugen file an admin copied in
        // by hand on purpose.
        assert!(!written_by_old_sync(include_str!(
            "../extra/variables.toml"
        )));
        assert!(!written_by_old_sync(
            "# Generated by margo's matugen — mlogind colour variables.\n\
             # Overwritten on every wallpaper/theme change; `mlogind sync-theme`\n\
             # publishes a validated copy as /var/lib/mgreet/variables.toml.\n\
             bg = \"#000000\"\n"
        ));
        // The header anywhere but the top, or only part of it.
        assert!(!written_by_old_sync(&format!(
            "# mine\n{}",
            old_sync_file("")
        )));
        assert!(!written_by_old_sync(
            "# Generated by margo's matugen — mlogind colour variables.\nbg = \"#000000\"\n"
        ));
        assert!(!written_by_old_sync(""));
    }

    #[test]
    fn an_old_sync_copy_is_not_the_admins_palette() {
        let text = old_sync_file(
            "bg = \"#101010\"\n\
             password_title = \"Type your root password here\"\n\
             pam_service = \"evil\"\n",
        );
        match AdminVariables::from_text(&text).unwrap() {
            AdminVariables::OldSync(Some(v)) => {
                // Only what a synced palette may carry survives.
                assert_eq!(v.0.len(), 1, "{:?}", v.0);
                assert_eq!(v.0["bg"].as_str(), Some("#101010"));
            }
            other => panic!("expected an old-sync copy, got {other:?}"),
        }
        assert!(matches!(
            AdminVariables::from_text(&old_sync_file("bg = 1\n")).unwrap(),
            AdminVariables::OldSync(None)
        ));
    }

    #[test]
    fn an_admin_file_is_trusted_as_written() {
        let text = "password_title = \"Enter your password, please\"\n";
        match AdminVariables::from_text(text).unwrap() {
            AdminVariables::Admin(v) => assert_eq!(
                v.0["password_title"].as_str(),
                Some("Enter your password, please")
            ),
            other => panic!("expected the admin's file, got {other:?}"),
        }
        assert!(AdminVariables::from_text("not = = toml").is_err());
    }

    #[test]
    fn a_synced_palette_keeps_only_known_keys_with_valid_values() {
        let v = Variables::sanitize_synced(
            "bg = \"#112233\"\n\
             accent = \"#ABCDEF\"\n\
             text = 1\n\
             muted = \"$accent\"\n\
             danger = \"red; drop\"\n\
             pam_service = \"evil\"\n\
             login_title = \"Kullanıcı\"\n",
        )
        .expect("two colours and a title are valid");
        let table = &v.0;
        assert_eq!(table.len(), 3, "{table:?}");
        assert_eq!(table["bg"].as_str(), Some("#112233"));
        assert_eq!(table["accent"].as_str(), Some("#ABCDEF"));
        assert_eq!(table["login_title"].as_str(), Some("Kullanıcı"));
    }

    #[test]
    fn a_synced_title_must_be_a_short_plain_label() {
        for bad in [
            "Enter your root password below",
            "Pass\u{1b}[2J",
            "Line\nbreak",
            "",
            " Padded",
            "$password_title",
        ] {
            let text = format!("password_title = {}", toml::Value::from(bad));
            assert!(
                Variables::sanitize_synced(&text).is_none(),
                "{bad:?} must be refused"
            );
        }
        assert!(Variables::sanitize_synced("password_title = \"Şifre\"").is_some());
    }

    #[test]
    fn a_synced_palette_that_is_not_toml_or_has_nothing_valid_is_none() {
        assert!(Variables::sanitize_synced("not = = toml").is_none());
        assert!(Variables::sanitize_synced("bg = 1").is_none());
        assert!(Variables::sanitize_synced("").is_none());
    }

    #[test]
    fn the_sanitised_palette_round_trips_through_its_serialisation() {
        let v = Variables::sanitize_synced("bg = \"#000000\"\npassword_title = \"PIN\"").unwrap();
        let again = Variables::sanitize_synced(&v.to_toml_string()).unwrap();
        assert_eq!(v.0, again.0);
    }

    #[test]
    fn a_broken_palette_costs_the_palette_not_the_configuration() {
        // `bg = 1` used to make `config.toml` fail to load, and mlogind then
        // ran on the built-in config: the admin's pam_service, power keys,
        // autologin, tty and greeter user silently gone.
        let config = format!("pam_service = \"site-login\"\n{BAKED_CONFIG}").replacen(
            "pam_service = \"mlogind\"\n",
            "",
            1,
        );
        let mut broken = Variables::baked_default();
        broken.overlay(vars("bg = 1"));
        let baked = Variables::baked_default();

        let (partial, used) = PartialConfig::from_str_with_variables(&config, &[&broken, &baked])
            .expect("the baked variables still substitute");
        assert_eq!(used, 1, "fell back to the second candidate");
        assert_eq!(partial.pam_service.as_deref(), Some("site-login"));
    }

    #[test]
    fn a_working_palette_is_used_first() {
        let synced = Variables::baked_default();
        let baked = Variables::baked_default();
        let (_, used) =
            PartialConfig::from_str_with_variables(BAKED_CONFIG, &[&synced, &baked]).unwrap();
        assert_eq!(used, 0);
    }

    #[test]
    fn an_unset_variable_in_every_candidate_is_an_error() {
        let empty = vars("");
        assert!(PartialConfig::from_str_with_variables(BAKED_CONFIG, &[&empty]).is_err());
    }

    #[test]
    fn a_config_that_is_not_toml_is_an_error_whatever_the_palette() {
        let baked = Variables::baked_default();
        assert!(PartialConfig::from_str_with_variables("tty = = 2", &[&baked]).is_err());
    }

    #[test]
    fn test_variable_iterator() {
        macro_rules! assert_var_iter {
            (
                $s:literal,
                ($($ident:literal),*)
            ) => {
                let variables: Vec<String> = VariableIterator::new($s).map(|v| v.ident().to_string()).collect();
                let idents: &[&str] = &[$($ident),*];

                eprintln!("variables = {variables:?}");
                eprintln!("ident = {idents:?}");

                assert_eq!(
                    &variables,
                    idents,
                );
            };
        }

        assert_var_iter!("", ());
        assert_var_iter!("abcdef", ());
        assert_var_iter!("$a", ("a"));
        assert_var_iter!("$a$b", ("a", "b"));
        assert_var_iter!("$a_c$b", ("a_c", "b"));
        assert_var_iter!("$a()$b", ("a", "b"));
        assert_var_iter!("$0    $1", ("0", "1"));
        assert_var_iter!("$var1    $var2    $var3  ", ("var1", "var2", "var3"));
    }
}
