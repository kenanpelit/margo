//! `bind`/`mousebind`/`axisbind`/`switchbind`/`gesturebind`/`touchgesturebind`
//! lines: modifier, key and button names, the action argument slots and the
//! default VT-switch bindings.

use anyhow::{Context, Result, bail};
use tracing::warn;
use xkbcommon::xkb;

use super::values::split_csv;
use crate::types::*;

// ── Bind parsing ─────────────────────────────────────────────────────────────

pub(super) fn parse_bind(cfg: &mut Config, key: &str, val: &str) -> Result<()> {
    let suffix = &key[4..];
    let release_apply = suffix.contains('r');
    let lock_apply = suffix.contains('l');
    let pass_apply = suffix.contains('p');
    let allow_conflict = suffix.contains('c');
    let key_type = if suffix.contains('s') {
        KeyType::Sym
    } else {
        KeyType::Code
    };

    let parts = split_csv(val, 8);
    if parts.len() < 3 {
        bail!("bind needs at least 3 fields: {}", val);
    }

    let modifiers = parse_modifiers(&parts[0])?;
    let key_sym_code = parse_key(&parts[1], key_type == KeyType::Sym)?;
    let action = parts[2].clone();
    if action.is_empty() {
        // A stray comma (`super,q,,killclient`): binding it would only
        // dispatch the empty action, a silent no-op.
        bail!("bind has an empty ACTION field: {}", val);
    }
    let args = build_arg(&parts[3..]);

    let mode = cfg.key_mode.clone();
    let is_common = mode == "common";
    let is_default = mode == "default";

    cfg.key_bindings.push(KeyBinding {
        modifiers,
        key: key_sym_code,
        action,
        arg: args,
        mode,
        is_common_mode: is_common,
        is_default_mode: is_default,
        lock_apply,
        release_apply,
        pass_apply,
        allow_conflict,
    });
    Ok(())
}

pub(super) fn parse_mousebind(cfg: &mut Config, val: &str) -> Result<()> {
    let parts = split_csv(val, 8);
    if parts.len() < 3 {
        bail!("mousebind needs at least 3 fields");
    }
    let modifiers = parse_modifiers(&parts[0])?;
    let button = parse_button(&parts[1]);
    let action = parts[2].clone();
    let arg = build_arg(&parts[3..]);
    cfg.mouse_bindings.push(MouseBinding {
        modifiers,
        button,
        action,
        arg,
    });
    Ok(())
}

pub(super) fn parse_axisbind(cfg: &mut Config, val: &str) -> Result<()> {
    let parts = split_csv(val, 8);
    if parts.len() < 3 {
        bail!("axisbind needs at least 3 fields");
    }
    let modifiers = parse_modifiers(&parts[0])?;
    let direction = parse_axis_direction(&parts[1]);
    let action = parts[2].clone();
    let arg = build_arg(&parts[3..]);
    cfg.axis_bindings.push(AxisBinding {
        modifiers,
        direction,
        action,
        arg,
    });
    Ok(())
}

pub(super) fn parse_switchbind(cfg: &mut Config, val: &str) -> Result<()> {
    let parts = split_csv(val, 8);
    if parts.len() < 2 {
        bail!("switchbind needs at least 2 fields");
    }
    let fold = parse_fold(&parts[0]);
    let action = parts[1].clone();
    let arg = build_arg(&parts[2..]);
    cfg.switch_bindings
        .push(SwitchBinding { fold, action, arg });
    Ok(())
}

/// Motion direction codes — matches mango C `enum { TOUCH_SWIPE_UP, ... }`
/// in `src/mango.c`. Keep numeric values stable.
fn parse_motion(s: &str) -> u32 {
    match s.trim().to_ascii_lowercase().as_str() {
        "up" => 0,
        "down" => 1,
        "right" => 2,
        "left" => 3,
        "up_right" | "up-right" | "upright" => 4,
        "up_left" | "up-left" | "upleft" => 5,
        "down_left" | "down-left" | "downleft" => 6,
        "down_right" | "down-right" | "downright" => 7,
        "none" => 8,
        // Numeric fallback for backward compat
        other => other.parse::<u32>().unwrap_or(0),
    }
}

pub(super) fn parse_gesturebind(cfg: &mut Config, val: &str) -> Result<()> {
    let parts = split_csv(val, 8);
    if parts.len() < 4 {
        bail!("gesturebind needs at least 4 fields");
    }
    let modifiers = parse_modifiers(&parts[0])?;
    let motion = parse_motion(&parts[1]);
    let fingers = parts[2].parse::<u32>().unwrap_or(0);
    let action = parts[3].clone();
    let arg = build_arg(&parts[4..]);
    cfg.gesture_bindings.push(GestureBinding {
        modifiers,
        motion,
        fingers,
        action,
        arg,
    });
    Ok(())
}

pub(super) fn parse_touchgesturebind(cfg: &mut Config, val: &str) -> Result<()> {
    let parts = split_csv(val, 8);
    if parts.len() < 5 {
        bail!("touchgesturebind needs at least 5 fields");
    }
    let swipe = parse_touch_swipe(&parts[0]);
    let edge = parse_edge(&parts[1]);
    let distance = parse_distance(&parts[2]);
    let fingers = parts[3].parse::<u32>().unwrap_or(0);
    let action = parts[4].clone();
    let arg = build_arg(&parts[5..]);
    cfg.touch_gesture_bindings.push(TouchGestureBinding {
        swipe,
        edge,
        distance,
        fingers,
        action,
        arg,
    });
    Ok(())
}

// ── Default ChVT bindings (Ctrl+Alt+F1…F12) ──────────────────────────────────

pub(super) fn inject_default_chvt_bindings(cfg: &mut Config) {
    for n in 1u32..=12 {
        let keysym_name = format!("XF86Switch_VT_{}", n);
        let keysym = xkb::keysym_from_name(&keysym_name, xkb::KEYSYM_NO_FLAGS);
        if keysym.raw() == 0u32 {
            continue;
        }
        cfg.key_bindings.push(KeyBinding {
            modifiers: Modifiers::CTRL | Modifiers::ALT,
            key: KeySymCode {
                keysym: keysym.raw(),
                keycode: MultiKeycode::default(),
                key_type: KeyType::Sym,
            },
            action: "chvt".to_string(),
            arg: Arg {
                ui: n,
                ..Default::default()
            },
            mode: "common".to_string(),
            is_common_mode: true,
            is_default_mode: false,
            lock_apply: true,
            release_apply: false,
            pass_apply: false,
            allow_conflict: false,
        });
    }
}

// ── Argument builder ─────────────────────────────────────────────────────────

fn build_arg(parts: &[String]) -> Arg {
    let mut arg = Arg::default();
    let get = |i: usize| parts.get(i).map(|s| s.as_str()).unwrap_or("0");

    // Try to fill numeric fields from available parts
    let s0 = get(0);
    let s1 = get(1);
    let s2 = get(2);
    // Prefer int first, fallback to float
    if let Ok(v) = s0.parse::<i32>() {
        arg.i = v;
        if v >= 0 {
            arg.ui = v as u32;
        }
        arg.f = v as f32;
    } else if let Ok(v) = s0.parse::<u32>() {
        arg.ui = v;
        arg.f = v as f32;
    } else if let Ok(v) = s0.parse::<f32>() {
        arg.f = v;
    } else if !s0.is_empty() {
        arg.v = Some(s0.to_string());
    }

    if let Ok(v) = s1.parse::<i32>() {
        arg.i2 = v;
        if v >= 0 {
            arg.ui2 = v as u32;
        }
        arg.f2 = v as f32;
    } else if let Ok(v) = s1.parse::<u32>() {
        arg.ui2 = v;
        arg.f2 = v as f32;
    } else if let Ok(v) = s1.parse::<f32>() {
        arg.f2 = v;
    } else if !s1.is_empty() {
        // An empty field (`…,spawn,` or `…,app,,cmd`) stays `None`, the
        // documented "`Some` only when non-empty" convention.
        arg.v2 = Some(s1.to_string());
    }

    if s2 != "0" && !s2.is_empty() {
        arg.v3 = Some(s2.to_string());
    }

    // 2024 edition: `let_chains` is stable, so collapse nested
    // `if let` into a single pattern.
    if let Some(s3) = parts.get(3) {
        if let Ok(v) = s3.parse::<u32>() {
            arg.ui = v;
        } else if !s3.is_empty() && s3 != "0" {
            // Non-numeric 4th field — the `summon` / `focusapp` match operator.
            arg.v4 = Some(s3.to_string());
        }
    }
    if let Some(s4) = parts.get(4)
        && let Ok(v) = s4.parse::<u32>()
    {
        arg.ui2 = v;
    }

    arg
}

// ── Modifier parsing ─────────────────────────────────────────────────────────

/// Parse a bind's MODS field (`super+shift`, `NONE`, `code:133`, …) the way
/// margo does. Unknown names are skipped with a warning; a non-empty field with
/// no name margo knows is an error — margo drops that bind. Public so editors
/// of the config text agree with the compositor on what a bind's modifiers are.
pub fn parse_modifiers(s: &str) -> Result<Modifiers> {
    if s.is_empty() || s.eq_ignore_ascii_case("none") {
        return Ok(Modifiers::empty());
    }
    let mut mods = Modifiers::empty();
    let mut matched = false;
    for token in s.split('+') {
        let t = token.trim().to_ascii_lowercase();
        if t.is_empty() {
            continue;
        }
        if let Some(code_str) = t.strip_prefix("code:") {
            let code: u32 = code_str.trim().parse().unwrap_or(0);
            match code {
                133 | 134 => mods |= Modifiers::LOGO,
                37 | 105 => mods |= Modifiers::CTRL,
                50 | 62 => mods |= Modifiers::SHIFT,
                64 | 108 => mods |= Modifiers::ALT,
                _ => warn!("unknown modifier keycode: {}", code),
            }
            matched = true;
            continue;
        }
        match t.as_str() {
            "super" | "super_l" | "super_r" => {
                mods |= Modifiers::LOGO;
                matched = true;
            }
            "ctrl" | "ctrl_l" | "ctrl_r" => {
                mods |= Modifiers::CTRL;
                matched = true;
            }
            "shift" | "shift_l" | "shift_r" => {
                mods |= Modifiers::SHIFT;
                matched = true;
            }
            "alt" | "alt_l" | "alt_r" => {
                mods |= Modifiers::ALT;
                matched = true;
            }
            "hyper" | "hyper_l" | "hyper_r" => {
                mods |= Modifiers::MOD3;
                matched = true;
            }
            "none" => {
                matched = true;
            }
            other => warn!("unknown modifier: {}", other),
        }
    }
    if !matched {
        bail!("no valid modifier in: {}", s);
    }
    Ok(mods)
}

// ── Key parsing ──────────────────────────────────────────────────────────────

/// xkbcommon's `keysym_from_name` as a raw keysym, 0 (`NoSymbol`) when the
/// name is unknown. Use this instead of calling xkbcommon directly on text
/// from a config file or a client: xkbcommon (0.8 and 0.9) builds a CString with
/// `unwrap()`, so a name containing a NUL byte panics; here it is simply not
/// a key name.
pub fn keysym_raw_from_name(name: &str, case_insensitive: bool) -> u32 {
    if name.contains('\0') {
        return 0;
    }
    let flags = if case_insensitive {
        xkb::KEYSYM_CASE_INSENSITIVE
    } else {
        xkb::KEYSYM_NO_FLAGS
    };
    xkb::keysym_from_name(name, flags).raw()
}

pub(crate) fn parse_key(s: &str, prefer_sym: bool) -> Result<KeySymCode> {
    let s = s.trim();

    // "code:NNN" form
    if let Some(rest) = s.strip_prefix("code:") {
        let code: u32 = rest.trim().parse().context("invalid keycode")?;
        return Ok(KeySymCode {
            keysym: 0,
            keycode: MultiKeycode {
                code1: code,
                ..Default::default()
            },
            key_type: KeyType::Code,
        });
    }

    // Exact (case-sensitive) match first — XF86 keysyms are canonically
    // mixed-case — then a case-insensitive pass (unless the caller prefers the
    // symbol) so plain names like `return` / `escape` / `f1` parse in any case.
    let mut raw = keysym_raw_from_name(s, false);
    if raw == 0u32 && !prefer_sym {
        raw = keysym_raw_from_name(s, true);
    }
    if raw == 0u32 {
        // Some valid XF86 keysyms are missing from xkbcommon's name table in
        // certain versions (notably `XF86AudioPlayPause`, which headsets emit
        // for the combined play/pause key). Resolve those by their canonical
        // raw value (X11/XF86keysym.h) so the bind still works.
        raw = keysym_alias(s).with_context(|| format!("unknown keysym: {s}"))?;
    }
    Ok(KeySymCode {
        keysym: raw,
        keycode: MultiKeycode::default(),
        key_type: KeyType::Sym,
    })
}

/// Fallback for keysym names absent from xkbcommon's table. Matched
/// case-insensitively. Values are the canonical X11/XF86keysym.h constants.
fn keysym_alias(name: &str) -> Option<u32> {
    const ALIASES: &[(&str, u32)] = &[("XF86AudioPlayPause", 0x1008_FF32)];
    ALIASES
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, v)| *v)
}

// ── Button name → evdev code ─────────────────────────────────────────────────

fn parse_button(s: &str) -> u32 {
    match s.trim().to_ascii_lowercase().as_str() {
        "lmb" | "mouse:272" => 272, // BTN_LEFT
        "rmb" | "mouse:273" => 273, // BTN_RIGHT
        "mmb" | "mouse:274" => 274, // BTN_MIDDLE
        "mouse:275" => 275,
        "mouse:276" => 276,
        other => other
            .strip_prefix("mouse:")
            .and_then(|n| n.parse().ok())
            .or_else(|| other.parse().ok())
            .unwrap_or(u32::MAX),
    }
}

// ── Direction helpers ────────────────────────────────────────────────────────

fn parse_axis_direction(s: &str) -> u32 {
    match s.trim().to_ascii_lowercase().as_str() {
        "up" => 0,
        "down" => 1,
        "left" => 2,
        "right" => 3,
        _ => 0,
    }
}

fn parse_touch_swipe(s: &str) -> TouchSwipe {
    match s.trim().to_ascii_lowercase().as_str() {
        "up" => TouchSwipe::Up,
        "down" => TouchSwipe::Down,
        "left" => TouchSwipe::Left,
        "right" => TouchSwipe::Right,
        "up_left" => TouchSwipe::UpLeft,
        "up_right" => TouchSwipe::UpRight,
        "down_left" => TouchSwipe::DownLeft,
        "down_right" => TouchSwipe::DownRight,
        _ => TouchSwipe::None,
    }
}

fn parse_edge(s: &str) -> EdgeOrCorner {
    match s.trim().to_ascii_lowercase().as_str() {
        "none" => EdgeOrCorner::None,
        "left" => EdgeOrCorner::Left,
        "right" => EdgeOrCorner::Right,
        "top" => EdgeOrCorner::Top,
        "bottom" => EdgeOrCorner::Bottom,
        "top_left" => EdgeOrCorner::TopLeft,
        "top_right" => EdgeOrCorner::TopRight,
        "bottom_left" => EdgeOrCorner::BottomLeft,
        "bottom_right" => EdgeOrCorner::BottomRight,
        _ => EdgeOrCorner::Any,
    }
}

fn parse_distance(s: &str) -> Distance {
    match s.trim().to_ascii_lowercase().as_str() {
        "short" => Distance::Short,
        "medium" => Distance::Medium,
        "long" => Distance::Long,
        _ => Distance::Any,
    }
}

fn parse_fold(s: &str) -> FoldState {
    match s.trim().to_ascii_lowercase().as_str() {
        "fold" => FoldState::Fold,
        "unfold" => FoldState::Unfold,
        _ => FoldState::Invalid,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::{parse_config, parse_config_str};
    use crate::types::Arg;

    /// Regression (cargo-fuzz `parse_config_str`): a NUL byte in a bind's key
    /// name reached xkbcommon's `keysym_from_name`, whose CString conversion
    /// unwraps and panicked — a stray NUL in config.conf took margo down at
    /// load / `mctl reload`. It must be an ordinary bad line instead.
    #[test]
    fn nul_byte_in_bind_key_is_rejected_without_panicking() {
        let c = parse_config_str("bind = SUPER,a\0b,spawn,kitty\nborderpx = 3\n", None)
            .expect("parse returns");
        assert_eq!(c.borderpx, 3, "later lines still parse");
        assert!(parse_key("a\0b", false).is_err());
        assert!(parse_key("\0", true).is_err());
    }

    #[test]
    fn xf86_media_keysyms_resolve() {
        // Regression: case-insensitive lookup lowercases XF86 names and fails.
        // These must parse (exact, case-sensitive) via the NO_FLAGS first pass.
        for name in [
            "XF86AudioPlayPause",
            "XF86AudioPlay",
            "XF86AudioNext",
            "XF86AudioPrev",
            "XF86AudioRaiseVolume",
            "XF86MonBrightnessUp",
        ] {
            let k = parse_key(name, false).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_ne!(k.keysym, 0, "{name} resolved to NoSymbol");
        }
        // Plain names still parse case-insensitively.
        assert_ne!(parse_key("return", false).unwrap().keysym, 0);
        assert_ne!(parse_key("ESCAPE", false).unwrap().keysym, 0);
    }

    #[test]
    fn parses_source_and_unsigned_bind_args() {
        let dir = std::env::temp_dir().join(format!("margo-config-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let main = dir.join("config.conf");
        let extra = dir.join("extra.conf");

        std::fs::write(
            &main,
            "source=./extra.conf\ndefault_layout=scroller\nbind=super,code:65,view,4294967295\n",
        )
        .unwrap();
        std::fs::write(&extra, "bind=super,code:10,view,1\n").unwrap();

        let cfg = parse_config(Some(&main)).unwrap();
        let view_args: Vec<u32> = cfg
            .key_bindings
            .iter()
            .filter(|bind| bind.action == "view")
            .map(|bind| bind.arg.ui)
            .collect();
        assert_eq!(cfg.default_layout, "scroller");
        assert!(
            cfg.key_bindings
                .iter()
                .any(|bind| bind.action == "view" && bind.arg.ui == u32::MAX),
            "view args: {view_args:?}"
        );
        assert!(
            cfg.key_bindings
                .iter()
                .any(|bind| bind.action == "view" && bind.arg.ui == 1),
            "view args: {view_args:?}"
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    // Table tests for the bind argument mapping (`bind = MODS,KEY,ACTION,args…`
    // → `Arg`), the part of a bind the dispatcher actually reads.
    fn arg(parts: &[&str]) -> Arg {
        build_arg(&parts.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn no_args_is_the_default_arg() {
        assert_eq!(arg(&[]), Arg::default());
    }

    #[test]
    fn first_slot_numbers() {
        let a = arg(&["4"]);
        assert_eq!((a.i, a.ui, a.f, a.v.as_deref()), (4, 4, 4.0, None));
        let a = arg(&["0"]);
        assert_eq!((a.i, a.ui, a.f, a.v.as_deref()), (0, 0, 0.0, None));
        let a = arg(&["+3"]);
        assert_eq!((a.i, a.ui), (3, 3));
        // Negative ints keep `ui` at 0 rather than wrapping.
        let a = arg(&["-1"]);
        assert_eq!((a.i, a.ui, a.f), (-1, 0, -1.0));
        // Tag masks above i32::MAX land in `ui` only (`tagall`-style masks).
        let a = arg(&["4294967295"]);
        assert_eq!((a.i, a.ui, a.f), (0, u32::MAX, u32::MAX as f32));
        let a = arg(&["i32-overflow-free"]);
        assert_eq!(a.v.as_deref(), Some("i32-overflow-free"));
    }

    #[test]
    fn first_slot_floats_and_strings() {
        let a = arg(&["0.05"]);
        assert_eq!((a.i, a.ui, a.f, a.v.as_deref()), (0, 0, 0.05, None));
        let a = arg(&["-0.1"]);
        assert_eq!(a.f, -0.1);
        let a = arg(&["1e3"]);
        assert_eq!(a.f, 1000.0);
        let a = arg(&["firefox"]);
        assert_eq!((a.i, a.f, a.v.as_deref()), (0, 0.0, Some("firefox")));
        // Too big for u32 and not an i32: parsed as a float.
        let a = arg(&["4294967296"]);
        assert_eq!((a.ui, a.f, a.v.as_deref()), (0, 4294967296.0, None));
    }

    #[test]
    fn second_slot_mirrors_the_first() {
        let a = arg(&["1", "2"]);
        assert_eq!((a.i, a.i2, a.ui2, a.f2), (1, 2, 2, 2.0));
        let a = arg(&["1", "-0.5"]);
        assert_eq!((a.i2, a.ui2, a.f2), (0, 0, -0.5));
        let a = arg(&["kitty", "dropdown"]);
        assert_eq!(
            (a.v.as_deref(), a.v2.as_deref()),
            (Some("kitty"), Some("dropdown"))
        );
        let a = arg(&["x", "4294967295"]);
        assert_eq!((a.ui2, a.v2.as_deref()), (u32::MAX, None));
    }

    #[test]
    fn third_to_fifth_slots() {
        let a = arg(&["app", "none", "kitty --class app"]);
        assert_eq!(a.v3.as_deref(), Some("kitty --class app"));
        assert_eq!(arg(&["a", "b", "0"]).v3, None);
        assert_eq!(arg(&["a", "b", ""]).v3, None);
        // A numeric 4th slot overrides `ui` from slot 1; a word is the
        // summon/focusapp match operator.
        let a = arg(&["5", "0", "0", "7"]);
        assert_eq!((a.i, a.ui, a.v4.as_deref()), (5, 7, None));
        let a = arg(&["app", "title", "cmd", "or"]);
        assert_eq!(a.v4.as_deref(), Some("or"));
        assert_eq!(arg(&["a", "b", "c", "0"]).v4, None);
        let a = arg(&["1", "2", "c", "3", "9"]);
        assert_eq!((a.ui, a.ui2), (3, 9));
        assert_eq!(
            arg(&["1", "2", "c", "3", "x"]).ui2,
            2,
            "non-numeric 5th is ignored"
        );
    }

    /// `Arg::v`/`v2` are documented as `Some` only when non-empty (the
    /// convention the IPC side follows too). A trailing comma or an empty
    /// field must not produce `Some("")`.
    #[test]
    fn empty_fields_are_none_not_empty_strings() {
        let a = arg(&[""]);
        assert_eq!(a, Arg::default());
        let a = arg(&["", ""]);
        assert_eq!((a.v.as_deref(), a.v2.as_deref()), (None, None));
        let a = arg(&["kitty", ""]);
        assert_eq!((a.v.as_deref(), a.v2.as_deref()), (Some("kitty"), None));
        let a = arg(&["", "title"]);
        assert_eq!((a.v.as_deref(), a.v2.as_deref()), (None, Some("title")));

        let cfg = parse_config_str("bind=SUPER,q,killclient,\n", None).unwrap();
        let bind = cfg
            .key_bindings
            .iter()
            .find(|b| b.action == "killclient")
            .unwrap();
        assert_eq!(bind.arg.v, None);
    }

    #[test]
    fn binds_carry_their_args() {
        let cfg = parse_config_str(
            "bind=SUPER,Return,spawn,foot\n\
             bind=SUPER,1,view,1\n\
             bind=SUPER+SHIFT,minus,setmfact,-0.05\n",
            None,
        )
        .unwrap();
        let find = |action: &str| {
            cfg.key_bindings
                .iter()
                .find(|b| b.action == action)
                .unwrap_or_else(|| panic!("no {action} bind"))
                .arg
                .clone()
        };
        assert_eq!(find("spawn").v.as_deref(), Some("foot"));
        assert_eq!(find("view").ui, 1);
        assert_eq!(find("setmfact").f, -0.05);
    }
}
