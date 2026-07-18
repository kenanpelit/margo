//! Compositor half of the xdg-desktop-portal **GlobalShortcuts**
//! backend (`org.freedesktop.impl.portal.GlobalShortcuts`).
//!
//! `margo-portal` owns the D-Bus surface; margo owns the keys. The
//! portal registers an app's shortcut session over the control socket
//!
//! ```text
//! dispatch global_shortcuts_bind <session> <id>:<TRIGGER>,<id>:<TRIGGER>,…
//! dispatch global_shortcuts_unbind <session>
//! watch shortcuts        # activation event stream (one JSON per line)
//! ```
//!
//! and receives `{"shortcut":{"session":…,"id":…,"state":"activated"|
//! "deactivated","timestamp_ms":…}}` frames when the user hits a
//! trigger. Ids are percent-encoded by the portal so they stay
//! whitespace/comma-free on the wire.
//!
//! Trigger grammar is the portal convention: `CTRL+SHIFT+t`,
//! `LOGO+F5`, `ALT+Print` — modifier tokens joined with `+`, final
//! token an xkb keysym name (case-insensitive fallback). Config binds
//! always win: the input path only consults this registry after
//! `find_keybinding` misses, so an app can never shadow a user bind.

use std::collections::HashMap;

use margo_config::Modifiers;
use smithay::input::keyboard::xkb;

pub struct RegisteredShortcut {
    pub id: String,
    pub mods: Modifiers,
    pub keysym: u32,
    /// Verbatim trigger string, echoed back over the `shortcuts`
    /// topic so the portal can answer `ListShortcuts`.
    pub trigger: String,
}

#[derive(Default)]
pub struct GlobalShortcutsRegistry {
    /// Portal session handle → its bound shortcuts.
    pub sessions: HashMap<String, Vec<RegisteredShortcut>>,
    /// Currently-held shortcut `(session, id, keycode)` — the release
    /// of that keycode fires the `deactivated` event.
    pub active: Option<(String, String, u32)>,
}

impl GlobalShortcutsRegistry {
    /// Find the shortcut matching a pressed key. First match wins;
    /// sessions iterate in arbitrary order but real-world triggers
    /// rarely collide across apps (and config binds already won).
    pub fn match_press(&self, mods: Modifiers, keysym: u32) -> Option<(String, String)> {
        for (session, shortcuts) in &self.sessions {
            for sc in shortcuts {
                if sc.mods == mods && sc.keysym == keysym {
                    return Some((session.clone(), sc.id.clone()));
                }
            }
        }
        None
    }

    pub fn summary(&self) -> serde_json::Value {
        let sessions: serde_json::Map<String, serde_json::Value> = self
            .sessions
            .iter()
            .map(|(session, shortcuts)| {
                (
                    session.clone(),
                    serde_json::Value::Array(
                        shortcuts
                            .iter()
                            .map(|sc| {
                                serde_json::json!({
                                    "id": sc.id,
                                    "trigger": sc.trigger,
                                })
                            })
                            .collect(),
                    ),
                )
            })
            .collect();
        serde_json::json!({ "global_shortcuts": sessions })
    }
}

/// Parse a portal trigger description (`CTRL+SHIFT+t`, `LOGO+F5`) into
/// margo's modifier mask + keysym. `None` when the key token is
/// unknown or missing — the shortcut is then registered trigger-less
/// (listed, never fired) exactly like portals with no rebind UI do.
pub fn parse_trigger(s: &str) -> Option<(Modifiers, u32)> {
    let mut mods = Modifiers::empty();
    let mut keysym = 0u32;
    for token in s.split('+') {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        match token.to_ascii_uppercase().as_str() {
            "CTRL" | "CONTROL" => mods |= Modifiers::CTRL,
            "SHIFT" => mods |= Modifiers::SHIFT,
            "ALT" | "MOD1" => mods |= Modifiers::ALT,
            "LOGO" | "SUPER" | "META" | "MOD4" => mods |= Modifiers::LOGO,
            _ => {
                let mut raw = xkb::keysym_from_name(token, xkb::KEYSYM_NO_FLAGS).raw();
                if raw == 0 {
                    raw = xkb::keysym_from_name(token, xkb::KEYSYM_CASE_INSENSITIVE).raw();
                }
                if raw == 0 {
                    return None;
                }
                keysym = raw;
            }
        }
    }
    (keysym != 0).then_some((mods, keysym))
}

/// Minimal %XX decode for the portal-encoded id/session tokens.
pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

impl crate::state::MargoState {
    /// `dispatch global_shortcuts_bind <session> <id>:<TRIG>,<id>:<TRIG>,…`
    pub fn global_shortcuts_bind(&mut self, arg: &str) {
        let mut parts = arg.splitn(2, ' ');
        let Some(session) = parts.next().filter(|s| !s.is_empty()) else {
            tracing::warn!("global_shortcuts_bind: missing session");
            return;
        };
        let session = percent_decode(session);
        let mut shortcuts = Vec::new();
        for entry in parts.next().unwrap_or("").split(',') {
            let entry = entry.trim();
            if entry.is_empty() {
                continue;
            }
            let (id, trigger) = match entry.split_once(':') {
                Some((id, trig)) => (percent_decode(id), percent_decode(trig)),
                None => (percent_decode(entry), String::new()),
            };
            let parsed = parse_trigger(&trigger);
            if parsed.is_none() && !trigger.is_empty() {
                tracing::warn!(id = %id, trigger = %trigger, "global shortcut: unparseable trigger, registering unbound");
            }
            let (mods, keysym) = parsed.unwrap_or((Modifiers::empty(), 0));
            shortcuts.push(RegisteredShortcut {
                id,
                mods,
                keysym,
                trigger,
            });
        }
        tracing::info!(
            session = %session,
            count = shortcuts.len(),
            "global shortcuts bound"
        );
        self.global_shortcuts.sessions.insert(session, shortcuts);
    }

    pub fn global_shortcuts_unbind(&mut self, arg: &str) {
        let session = percent_decode(arg.trim());
        if self.global_shortcuts.sessions.remove(&session).is_some() {
            tracing::info!(session = %session, "global shortcuts session unbound");
        }
    }

    /// Called from the key-press path AFTER config binds and modal
    /// keys pass. Returns true when a shortcut fired (key swallowed).
    pub fn global_shortcut_press(&mut self, mods: Modifiers, keysym: u32, keycode: u32) -> bool {
        if keysym == 0 {
            return false;
        }
        let Some((session, id)) = self.global_shortcuts.match_press(mods, keysym) else {
            return false;
        };
        self.global_shortcuts.active = Some((session.clone(), id.clone(), keycode));
        self.push_global_shortcut_event(&session, &id, true);
        true
    }

    /// Called on every key release; fires `deactivated` when the held
    /// shortcut's keycode is let go. Returns true when swallowed.
    pub fn global_shortcut_release(&mut self, keycode: u32) -> bool {
        let Some((session, id, kc)) = self.global_shortcuts.active.take() else {
            return false;
        };
        if kc != keycode {
            self.global_shortcuts.active = Some((session, id, kc));
            return false;
        }
        self.push_global_shortcut_event(&session, &id, false);
        true
    }

    /// Fan an activation frame out to every `watch shortcuts`
    /// subscriber (the portal daemon; anyone else scripting against
    /// the socket gets it too).
    fn push_global_shortcut_event(&mut self, session: &str, id: &str, activated: bool) {
        let payload = serde_json::json!({
            "shortcut": {
                "session": session,
                "id": id,
                "state": if activated { "activated" } else { "deactivated" },
                "timestamp_ms": crate::utils::now_ms(),
            }
        });
        let subs: Vec<u32> = self
            .ipc_watches
            .watches
            .iter()
            .filter(|w| w.topic == "shortcuts")
            .map(|w| w.token)
            .collect();
        for token in subs {
            self.ipc_send(token, &payload);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trigger_grammar_parses_portal_conventions() {
        let (mods, sym) = parse_trigger("CTRL+SHIFT+t").expect("parses");
        assert_eq!(mods, Modifiers::CTRL | Modifiers::SHIFT);
        assert_eq!(sym, xkb::keysym_from_name("t", xkb::KEYSYM_NO_FLAGS).raw());

        let (mods, _) = parse_trigger("LOGO+F5").expect("parses");
        assert_eq!(mods, Modifiers::LOGO);

        assert!(parse_trigger("CTRL+").is_none(), "no key token");
        assert!(parse_trigger("CTRL+NotAKeyName42x").is_none());
    }

    #[test]
    fn percent_decode_round_trips_reserved_bytes() {
        assert_eq!(percent_decode("a%3Ab%2Cc%20d"), "a:b,c d");
        assert_eq!(percent_decode("plain"), "plain");
        assert_eq!(percent_decode("dangling%2"), "dangling%2");
    }

    #[test]
    fn match_press_requires_exact_modifier_set() {
        let mut reg = GlobalShortcutsRegistry::default();
        let (mods, sym) = parse_trigger("CTRL+SHIFT+t").unwrap();
        reg.sessions.insert(
            "s1".into(),
            vec![RegisteredShortcut {
                id: "toggle".into(),
                mods,
                keysym: sym,
                trigger: "CTRL+SHIFT+t".into(),
            }],
        );
        assert!(reg.match_press(mods, sym).is_some());
        assert!(reg.match_press(Modifiers::CTRL, sym).is_none());
        assert!(reg.match_press(mods, sym + 1).is_none());
    }
}
