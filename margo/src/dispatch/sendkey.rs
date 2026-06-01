//! `sendkey` dispatch action: inject a synthetic key combo into the
//! focused window, optionally gated by the focused `app_id` with a
//! fallback action. margo owns the seat keyboard, so the events are
//! forwarded straight to the focused surface — no virtual-keyboard
//! protocol, no uinput, no external tool.

use crate::state::MargoState;
use margo_config::Arg;
use smithay::backend::input::KeyState;
use smithay::input::keyboard::FilterResult;
use smithay::utils::SERIAL_COUNTER;
use std::time::Duration;

/// A parsed key combo as **evdev** keycodes (modifiers + one key).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyCombo {
    pub mods: Vec<u32>,
    pub key: u32,
}

/// Modifier name → left-variant evdev keycode.
fn mod_to_evdev(name: &str) -> Option<u32> {
    match name.to_ascii_lowercase().as_str() {
        "ctrl" | "control" => Some(29),                // KEY_LEFTCTRL
        "shift" => Some(42),                           // KEY_LEFTSHIFT
        "alt" | "meta" => Some(56),                    // KEY_LEFTALT
        "super" | "logo" | "mod" | "win" => Some(125), // KEY_LEFTMETA
        _ => None,
    }
}

/// Layout-independent key name → evdev keycode. Covers what tab/window
/// navigation needs; letter keys (layout-dependent) are intentionally out
/// of scope. xkb-style names, case-tolerant.
pub fn key_name_to_evdev(name: &str) -> Option<u32> {
    let n = name.to_ascii_lowercase();
    let code = match n.as_str() {
        "tab" => 15,
        "return" | "enter" => 28,
        "escape" | "esc" => 1,
        "space" => 57,
        "backspace" => 14,
        "delete" | "del" => 111,
        "insert" | "ins" => 110,
        "page_up" | "pageup" | "prior" => 104,
        "page_down" | "pagedown" | "next" => 109,
        "home" => 102,
        "end" => 107,
        "left" => 105,
        "right" => 106,
        "up" => 103,
        "down" => 108,
        "f1" => 59,
        "f2" => 60,
        "f3" => 61,
        "f4" => 62,
        "f5" => 63,
        "f6" => 64,
        "f7" => 65,
        "f8" => 66,
        "f9" => 67,
        "f10" => 68,
        "f11" => 87,
        "f12" => 88,
        "1" => 2,
        "2" => 3,
        "3" => 4,
        "4" => 5,
        "5" => 6,
        "6" => 7,
        "7" => 8,
        "8" => 9,
        "9" => 10,
        "0" => 11,
        _ => return None,
    };
    Some(code)
}

/// Parse `ctrl+shift+Tab` → mods + key (evdev codes). `+`-joined; the last
/// token is the key, the rest are modifiers. Any unknown token → `None`.
pub fn parse_combo(spec: &str) -> Option<KeyCombo> {
    let toks: Vec<&str> = spec
        .split('+')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    let (key_tok, mod_toks) = toks.split_last()?;
    let key = key_name_to_evdev(key_tok)?;
    let mut mods = Vec::with_capacity(mod_toks.len());
    for m in mod_toks {
        mods.push(mod_to_evdev(m)?);
    }
    Some(KeyCombo { mods, key })
}

/// Does the focused `app_id` match `regex`? Bad regex / no focus → false.
pub fn appid_matches(focused: Option<&str>, regex: &str) -> bool {
    let Some(app) = focused else { return false };
    match regex::Regex::new(regex) {
        Ok(re) => re.is_match(app),
        Err(_) => false,
    }
}

/// Split a fallback spec `action[:arg]` → `(action, arg)` (arg `""` if none).
pub fn parse_fallback(spec: &str) -> Option<(&str, &str)> {
    let spec = spec.trim();
    if spec.is_empty() {
        return None;
    }
    Some(match spec.split_once(':') {
        Some((a, b)) => (a.trim(), b.trim()),
        None => (spec, ""),
    })
}

impl MargoState {
    /// `app_id` of the focused client, if any.
    fn focused_app_id(&self) -> Option<String> {
        self.focused_client_idx()
            .map(|i| self.clients[i].app_id.clone())
    }

    /// Dispatch entry for the `sendkey` action.
    pub fn send_key(&mut self, arg: &Arg) {
        let combo = match arg.v.as_deref().and_then(parse_combo) {
            Some(c) => c,
            None => {
                tracing::warn!(combo = ?arg.v, "sendkey: unparseable combo");
                return;
            }
        };

        // Optional app-id gate.
        if let Some(re) = arg.v2.as_deref().filter(|s| !s.is_empty())
            && !appid_matches(self.focused_app_id().as_deref(), re)
        {
            // Run the fallback action, if any.
            if let Some((action, farg)) = arg.v3.as_deref().and_then(parse_fallback)
                && action != "sendkey"
            {
                let sub = Arg {
                    v: (!farg.is_empty()).then(|| farg.to_string()),
                    ..Arg::default()
                };
                crate::dispatch::dispatch_action(self, action, &sub);
            }
            return;
        }

        self.inject_combo(&combo);
    }

    /// Forward a synthetic press/release sequence for `combo` to the
    /// focused surface (mods down → key down → key up → mods up).
    fn inject_combo(&mut self, combo: &KeyCombo) {
        let Some(kb) = self.seat.get_keyboard() else {
            return;
        };
        // (evdev, state) sequence.
        let mut seq: Vec<(u32, KeyState)> = Vec::with_capacity(combo.mods.len() * 2 + 2);
        for &m in &combo.mods {
            seq.push((m, KeyState::Pressed));
        }
        seq.push((combo.key, KeyState::Pressed));
        seq.push((combo.key, KeyState::Released));
        for &m in combo.mods.iter().rev() {
            seq.push((m, KeyState::Released));
        }

        for (evdev, key_state) in seq {
            let serial = SERIAL_COUNTER.next_serial();
            let time = Duration::from(self.clock.now()).as_millis() as u32;
            // +8: evdev → xkb keycode. Forward so the client receives it and
            // margo's own keybindings don't re-trigger.
            kb.input::<(), _>(
                self,
                (evdev + 8).into(),
                key_state,
                serial,
                time,
                |_, _, _| FilterResult::Forward,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_combo_mods_and_key() {
        assert_eq!(
            parse_combo("ctrl+Tab"),
            Some(KeyCombo {
                mods: vec![29],
                key: 15
            })
        );
        assert_eq!(
            parse_combo("ctrl+shift+Tab"),
            Some(KeyCombo {
                mods: vec![29, 42],
                key: 15
            })
        );
        assert_eq!(
            parse_combo("Tab"),
            Some(KeyCombo {
                mods: vec![],
                key: 15
            })
        );
        // case-insensitive
        assert_eq!(
            parse_combo("CTRL+tab"),
            Some(KeyCombo {
                mods: vec![29],
                key: 15
            })
        );
        // unknown key / mod
        assert_eq!(parse_combo("ctrl+bogus"), None);
        assert_eq!(parse_combo("hyper+Tab"), None);
    }

    #[test]
    fn key_names() {
        assert_eq!(key_name_to_evdev("Page_Up"), Some(104));
        assert_eq!(key_name_to_evdev("Prior"), Some(104));
        assert_eq!(key_name_to_evdev("F5"), Some(63));
        assert_eq!(key_name_to_evdev("nope"), None);
    }

    #[test]
    fn appid_match() {
        assert!(appid_matches(Some("Kenp"), "^(Kenp|Ai)$"));
        assert!(!appid_matches(Some("kitty"), "^Kenp$"));
        assert!(!appid_matches(None, "^x$"));
        assert!(!appid_matches(Some("x"), "^(unterminated"));
    }

    #[test]
    fn fallback_split() {
        assert_eq!(parse_fallback("focusdir:up"), Some(("focusdir", "up")));
        assert_eq!(parse_fallback("zoom"), Some(("zoom", "")));
        assert_eq!(parse_fallback(""), None);
    }
}
