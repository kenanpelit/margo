//! `windowrule`/`monitorrule`/`tagrule`/`layerrule` lines and the per-kind
//! option tables the validator checks rule lines against.

use anyhow::{Result, bail};
use tracing::warn;

use super::values::{
    parse_bool_s, parse_f32_s, parse_i32_s, parse_percent_fraction, parse_u32_s, split_csv_colon,
};
use crate::types::*;

// ── Rule parsing ─────────────────────────────────────────────────────────────
//
// Every option each rule kind accepts, mirrored from the match arms below
// (`rule_key_tables_match_parser_arms` keeps them in sync). The validator
// checks rule lines against these, so an option the parser would only log
// is reported to the user as W008.

/// Options `windowrule` / `windowrule-once` accept.
pub const WINDOWRULE_KEYS: &[&str] = &[
    "appid",
    "app_id",
    "title",
    "exclude_appid",
    "exclude_app_id",
    "not_appid",
    "exclude_title",
    "not_title",
    "min_width",
    "min_height",
    "max_width",
    "max_height",
    "open_focused",
    "block_out_from_screencast",
    "blockout",
    "tags",
    "monitor",
    "monitor_name",
    "offsetx",
    "offset_x",
    "offsety",
    "offset_y",
    "width",
    "height",
    "isfloating",
    "floating",
    "is_floating",
    "isfullscreen",
    "fullscreen",
    "is_fullscreen",
    "isfakefullscreen",
    "fakefullscreen",
    "is_fake_fullscreen",
    "scroller_proportion",
    "scroller_proportion_single",
    "animation_type_open",
    "animation_type_close",
    "layer_animation_type_open",
    "layer_animation_type_close",
    "isnoborder",
    "isnoshadow",
    "isnoradius",
    "isnoanimation",
    "borderpx",
    "border_width",
    "isopensilent",
    "istagsilent",
    "isnamedscratchpad",
    "isunglobal",
    "isglobal",
    "isoverlay",
    "is_overlay",
    "overlay",
    "allow_shortcuts_inhibit",
    "ignore_maximize",
    "ignore_minimize",
    "isnosizehint",
    "idle_inhibit_when_focus",
    "indleinhibit_when_focus",
    "nofocus",
    "focus_on_xdg_activate",
    "nofadein",
    "nofadeout",
    "no_force_center",
    "isterm",
    "allow_csd",
    "force_fakemaximize",
    "force_tiled_state",
    "force_tearing",
    "noswallow",
    "confine_pointer",
    "noblur",
    "group",
    "isgroup",
    "focused_opacity",
    "unfocused_opacity",
];

/// The `windowrule` options that select windows. A rule with none of them
/// applies to every window.
pub const WINDOWRULE_MATCHERS: &[&str] = &[
    "appid",
    "app_id",
    "title",
    "exclude_appid",
    "exclude_app_id",
    "not_appid",
    "exclude_title",
    "not_title",
];

/// Options `monitorrule` accepts.
pub const MONITORRULE_KEYS: &[&str] = &[
    "name", "make", "model", "serial", "rr", "scale", "x", "y", "width", "height", "refresh",
    "vrr", "custom",
];

/// Options `tagrule` accepts.
pub const TAGRULE_KEYS: &[&str] = &[
    "id",
    "layout_name",
    "monitor_name",
    "monitor_make",
    "monitor_model",
    "monitor_serial",
    "mfact",
    "nmaster",
    "no_render_border",
    "open_as_floating",
    "no_hide",
    "wallpaper",
];

/// Options `layerrule` accepts.
pub const LAYERRULE_KEYS: &[&str] = &[
    "layer_name",
    "animation_type_open",
    "animation_type_close",
    "noblur",
    "noanim",
    "noshadow",
];

/// The `layerrule` options that select layer surfaces.
pub const LAYERRULE_MATCHERS: &[&str] = &["layer_name"];

/// A rule that names no selector but carried an unknown option almost always
/// had its selector misspelled (`class:` for `appid:`, `namespace:` for
/// `layer_name:`). Applying it would hit every window or layer surface, so
/// the rule is dropped instead. A rule with no selector and no unknown option
/// is a deliberate catch-all and is kept.
fn check_rule_selector(kind: &str, has_matcher: bool, unknown: &[String]) -> Result<()> {
    if has_matcher || unknown.is_empty() {
        return Ok(());
    }
    bail!(
        "{kind} has no recognised selector (unknown option{} {}) — ignored instead of \
         applying to everything",
        if unknown.len() == 1 { "" } else { "s" },
        unknown
            .iter()
            .map(|k| format!("`{k}`"))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

pub(super) fn parse_windowrule(cfg: &mut Config, val: &str, once: bool) -> Result<()> {
    let mut rule = WindowRule {
        once,
        ..WindowRule::default()
    };
    let mut has_matcher = false;
    let mut unknown = Vec::new();
    for part in split_csv_colon(val) {
        let (k, v) = part;
        has_matcher |= WINDOWRULE_MATCHERS.contains(&k.as_str());
        match k.as_str() {
            "appid" | "app_id" => rule.id = Some(v),
            "title" => rule.title = Some(v),
            // Niri-style exclude clauses — invert the match.
            "exclude_appid" | "exclude_app_id" | "not_appid" => rule.exclude_id = Some(v),
            "exclude_title" | "not_title" => rule.exclude_title = Some(v),
            // Niri-style size constraints. Apply to both tiled (clamps the
            // computed geometry) and floating windows.
            "min_width" => rule.min_width = parse_i32_s(&v).max(0),
            "min_height" => rule.min_height = parse_i32_s(&v).max(0),
            "max_width" => rule.max_width = parse_i32_s(&v).max(0),
            "max_height" => rule.max_height = parse_i32_s(&v).max(0),
            "open_focused" => rule.open_focused = Some(parse_bool_s(&v)),
            "block_out_from_screencast" | "blockout" => {
                rule.block_out_from_screencast = Some(parse_bool_s(&v))
            }
            // Tag numbers are 1..=32 → bit 0..=31. Clamp the shift amount:
            // an out-of-range `tags:33` (typo) would otherwise overflow the
            // shift and panic in release (overflow-checks = true).
            "tags" => rule.tags = 1u32 << v.parse::<u32>().unwrap_or(1).saturating_sub(1).min(31),
            // `monitor:eDP-1` (windowrule context) pins a window to the
            // named output. `monitor_name` is the tagrule key spelling
            // for the same concept — accept it as an alias here so a
            // user writing `monitor_name:eDP-1` on a windowrule (likely
            // from misreading the docs that mix the two contexts) gets
            // the expected behaviour instead of silent drop.
            "monitor" | "monitor_name" => rule.monitor = Some(v),
            "offsetx" | "offset_x" => rule.offset_x = parse_i32_s(&v),
            "offsety" | "offset_y" => rule.offset_y = parse_i32_s(&v),
            // Window-rule width/height accept either an absolute
            // pixel value (`width:600`) or a monitor-relative
            // fraction (`width:50%`). The fraction form sets
            // `width_fraction`; the absolute form sets `width`.
            // Consumers prefer the fraction when both are set —
            // mirrors mango 0.13's flexible-window-rules behaviour.
            "width" => {
                if let Some(frac) = parse_percent_fraction(&v) {
                    rule.width_fraction = Some(frac);
                } else {
                    rule.width = parse_i32_s(&v);
                }
            }
            "height" => {
                if let Some(frac) = parse_percent_fraction(&v) {
                    rule.height_fraction = Some(frac);
                } else {
                    rule.height = parse_i32_s(&v);
                }
            }
            "isfloating" | "floating" | "is_floating" => rule.is_floating = Some(parse_bool_s(&v)),
            "isfullscreen" | "fullscreen" | "is_fullscreen" => {
                rule.is_fullscreen = Some(parse_bool_s(&v))
            }
            "isfakefullscreen" | "fakefullscreen" | "is_fake_fullscreen" => {
                rule.is_fake_fullscreen = Some(parse_bool_s(&v))
            }
            "scroller_proportion" => rule.scroller_proportion = Some(parse_f32_s(&v)),
            "scroller_proportion_single" => rule.scroller_proportion_single = Some(parse_f32_s(&v)),
            "animation_type_open" => rule.animation_type_open = Some(v),
            "animation_type_close" => rule.animation_type_close = Some(v),
            "layer_animation_type_open" => rule.layer_animation_type_open = Some(v),
            "layer_animation_type_close" => rule.layer_animation_type_close = Some(v),
            "isnoborder" => rule.no_border = Some(parse_bool_s(&v)),
            "isnoshadow" => rule.no_shadow = Some(parse_bool_s(&v)),
            "isnoradius" => rule.no_radius = Some(parse_bool_s(&v)),
            "isnoanimation" => rule.no_animation = Some(parse_bool_s(&v)),
            "borderpx" | "border_width" => rule.border_width = Some(parse_u32_s(&v)),
            "isopensilent" => rule.open_silent = Some(parse_bool_s(&v)),
            "istagsilent" => rule.tag_silent = Some(parse_bool_s(&v)),
            "isnamedscratchpad" => rule.is_named_scratchpad = Some(parse_bool_s(&v)),
            "isunglobal" => rule.is_unglobal = Some(parse_bool_s(&v)),
            "isglobal" => rule.is_global = Some(parse_bool_s(&v)),
            // Accept the same three spellings as `isfloating` so
            // user typos don't silently drop on the floor.
            "isoverlay" | "is_overlay" | "overlay" => rule.is_overlay = Some(parse_bool_s(&v)),
            "allow_shortcuts_inhibit" => rule.allow_shortcuts_inhibit = Some(parse_bool_s(&v)),
            "ignore_maximize" => rule.ignore_maximize = Some(parse_bool_s(&v)),
            "ignore_minimize" => rule.ignore_minimize = Some(parse_bool_s(&v)),
            "isnosizehint" => rule.no_size_hint = Some(parse_bool_s(&v)),
            // "indleinhibit_when_focus" was a typo'd key from the original
            // port that nothing ever consumed (MargoClient had no matching
            // field) — kept as a legacy alias now that it does something.
            "idle_inhibit_when_focus" | "indleinhibit_when_focus" => {
                rule.idle_inhibit_when_focus = Some(parse_bool_s(&v))
            }
            "nofocus" => rule.no_focus = Some(parse_bool_s(&v)),
            "focus_on_xdg_activate" => rule.focus_on_xdg_activate = Some(parse_bool_s(&v)),
            "nofadein" => rule.no_fade_in = Some(parse_bool_s(&v)),
            "nofadeout" => rule.no_fade_out = Some(parse_bool_s(&v)),
            "no_force_center" => rule.no_force_center = Some(parse_bool_s(&v)),
            "isterm" => rule.is_term = Some(parse_bool_s(&v)),
            "allow_csd" => rule.allow_csd = Some(parse_bool_s(&v)),
            "force_fakemaximize" => rule.force_fake_maximize = Some(parse_bool_s(&v)),
            "force_tiled_state" => rule.force_tiled_state = Some(parse_bool_s(&v)),
            "force_tearing" => rule.force_tearing = Some(parse_bool_s(&v)),
            "noswallow" => rule.no_swallow = Some(parse_bool_s(&v)),
            "confine_pointer" => rule.confine_pointer = Some(parse_bool_s(&v)),
            "noblur" => rule.no_blur = Some(parse_bool_s(&v)),
            "group" | "isgroup" => rule.group = Some(parse_bool_s(&v)),
            "focused_opacity" => rule.focused_opacity = Some(parse_f32_s(&v)),
            "unfocused_opacity" => rule.unfocused_opacity = Some(parse_f32_s(&v)),
            other => {
                warn!("unknown windowrule option: {}", other);
                if !other.is_empty() {
                    unknown.push(other.to_string());
                }
            }
        }
    }
    check_rule_selector("windowrule", has_matcher, &unknown)?;
    cfg.window_rules.push(rule);
    Ok(())
}

pub(super) fn parse_monitorrule(cfg: &mut Config, val: &str) -> Result<()> {
    let mut rule = MonitorRule {
        scale: 1.0,
        x: i32::MAX,
        y: i32::MAX,
        width: -1,
        height: -1,
        ..Default::default()
    };
    for (k, v) in split_csv_colon(val) {
        match k.as_str() {
            "name" => rule.name = Some(v),
            "make" => rule.make = Some(v),
            "model" => rule.model = Some(v),
            "serial" => rule.serial = Some(v),
            "rr" => rule.transform = parse_i32_s(&v).clamp(0, 7),
            "scale" => rule.scale = parse_f32_s(&v).clamp(0.001, 1000.0),
            "x" => rule.x = parse_i32_s(&v),
            "y" => rule.y = parse_i32_s(&v),
            "width" => rule.width = parse_i32_s(&v).max(1),
            "height" => rule.height = parse_i32_s(&v).max(1),
            "refresh" => rule.refresh = parse_f32_s(&v).clamp(0.001, 1000.0),
            "vrr" => rule.vrr = parse_bool_s(&v),
            "custom" => rule.custom_mode = parse_bool_s(&v),
            other => warn!("unknown monitorrule option: {}", other),
        }
    }
    if rule.name.is_none() && rule.make.is_none() && rule.model.is_none() && rule.serial.is_none() {
        bail!("monitorrule must specify at least one of: name, make, model, serial");
    }
    cfg.monitor_rules.push(rule);
    Ok(())
}

pub(super) fn parse_tagrule(cfg: &mut Config, val: &str) -> Result<()> {
    let mut rule = TagRule::default();
    for (k, v) in split_csv_colon(val) {
        match k.as_str() {
            "id" => rule.id = parse_i32_s(&v),
            "layout_name" => rule.layout_name = Some(v),
            "monitor_name" => rule.monitor_name = Some(v),
            "monitor_make" => rule.monitor_make = Some(v),
            "monitor_model" => rule.monitor_model = Some(v),
            "monitor_serial" => rule.monitor_serial = Some(v),
            "mfact" => rule.mfact = parse_f32_s(&v),
            "nmaster" => rule.nmaster = parse_i32_s(&v),
            "no_render_border" => rule.no_render_border = parse_bool_s(&v),
            "open_as_floating" => rule.open_as_floating = parse_bool_s(&v),
            "no_hide" => rule.no_hide = parse_bool_s(&v),
            "wallpaper" => rule.wallpaper = Some(v),
            other => warn!("unknown tagrule option: {}", other),
        }
    }
    cfg.tag_rules.push(rule);
    Ok(())
}

pub(super) fn parse_layerrule(cfg: &mut Config, val: &str) -> Result<()> {
    let mut rule = LayerRule::default();
    let mut has_matcher = false;
    let mut unknown = Vec::new();
    for (k, v) in split_csv_colon(val) {
        has_matcher |= LAYERRULE_MATCHERS.contains(&k.as_str());
        match k.as_str() {
            "layer_name" => rule.layer_name = Some(v),
            "animation_type_open" => rule.animation_type_open = Some(v),
            "animation_type_close" => rule.animation_type_close = Some(v),
            "noblur" => rule.no_blur = parse_bool_s(&v),
            "noanim" => rule.no_anim = parse_bool_s(&v),
            "noshadow" => rule.no_shadow = parse_bool_s(&v),
            other => {
                warn!("unknown layerrule option: {}", other);
                if !other.is_empty() {
                    unknown.push(other.to_string());
                }
            }
        }
    }
    check_rule_selector("layerrule", has_matcher, &unknown)?;
    cfg.layer_rules.push(rule);
    Ok(())
}

#[cfg(test)]
mod tests {

    use crate::parser::tests::parse_conf;
    use crate::parser::{parse_config, parse_config_str};

    #[test]
    fn out_of_range_tag_number_does_not_overflow() {
        // Regression: `tags:33` (bit 33) overflowed `1 << n` and panicked in
        // release (overflow-checks = true). The shift amount is clamped to 31.
        let dir = std::env::temp_dir().join(format!("margo-tag-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let main = dir.join("config.conf");
        std::fs::write(&main, "windowrule = tags:33,appid:foo\n").unwrap();
        // Must not panic while parsing.
        let cfg = parse_config(Some(&main)).unwrap();
        assert_eq!(cfg.window_rules.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_floating_windowrule_with_aliases() {
        let dir =
            std::env::temp_dir().join(format!("margo-windowrule-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let main = dir.join("config.conf");

        std::fs::write(
            &main,
            r#"windowrule = is_floating:1,width:600,height:1080,offset_x:95,offset_y:-15,appid:^(com\.github\.hluk\.copyq|copyq|wiremix|org\.pulseaudio\.pavucontrol|io\.ente\.auth)$
"#,
        )
        .unwrap();

        let cfg = parse_config(Some(&main)).unwrap();
        let rule = cfg.window_rules.first().expect("windowrule parsed");
        assert_eq!(rule.is_floating, Some(true));
        assert_eq!(rule.width, 600);
        assert_eq!(rule.height, 1080);
        assert_eq!(rule.offset_x, 95);
        assert_eq!(rule.offset_y, -15);
        assert_eq!(
            rule.id.as_deref(),
            Some(
                r"^(com\.github\.hluk\.copyq|copyq|wiremix|org\.pulseaudio\.pavucontrol|io\.ente\.auth)$"
            )
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn windowrule_percent_sets_fractional_size() {
        // `width:50%` / `height:25%` → fractions, not absolute pixels.
        let cfg = parse_conf("frac", "windowrule = width:50%,height:25%,appid:^foo$\n");
        let rule = cfg.window_rules.first().expect("rule parsed");
        assert_eq!(rule.width_fraction, Some(0.5));
        assert_eq!(rule.height_fraction, Some(0.25));
    }

    #[test]
    fn windowrule_clamps_oversized_percent() {
        // A typo like `200%` pins to full size rather than overflowing.
        let cfg = parse_conf("fracclamp", "windowrule = width:200%,appid:^foo$\n");
        assert_eq!(cfg.window_rules[0].width_fraction, Some(1.0));
    }

    #[test]
    fn windowrule_absolute_size_leaves_fraction_unset() {
        let cfg = parse_conf("abssize", "windowrule = width:600,height:480,appid:^foo$\n");
        let rule = &cfg.window_rules[0];
        assert_eq!(rule.width, 600);
        assert_eq!(rule.height, 480);
        assert_eq!(rule.width_fraction, None);
        assert_eq!(rule.height_fraction, None);
    }

    #[test]
    fn windowrule_monitor_and_monitor_name_are_aliases() {
        let cfg = parse_conf(
            "mon",
            "windowrule = monitor:DP-3,appid:^a$\nwindowrule = monitor_name:eDP-1,appid:^b$\n",
        );
        assert_eq!(cfg.window_rules[0].monitor.as_deref(), Some("DP-3"));
        assert_eq!(cfg.window_rules[1].monitor.as_deref(), Some("eDP-1"));
    }

    #[test]
    fn windowrule_idle_inhibit_when_focus_aliases() {
        // The correct spelling and the original typo'd key (never
        // consumed until MargoClient grew the matching field) both work.
        let cfg = parse_conf(
            "idl",
            "windowrule = idle_inhibit_when_focus:1,appid:^a$\nwindowrule = indleinhibit_when_focus:1,appid:^b$\n",
        );
        assert_eq!(cfg.window_rules.len(), 2);
        for r in &cfg.window_rules {
            assert_eq!(
                r.idle_inhibit_when_focus,
                Some(true),
                "idle_inhibit_when_focus not set for {:?}",
                r.id
            );
        }
    }

    #[test]
    fn windowrule_overlay_aliases_all_set_is_overlay() {
        // isoverlay / is_overlay / overlay are equivalent.
        let cfg = parse_conf(
            "ovl",
            "windowrule = isoverlay:1,appid:^a$\nwindowrule = is_overlay:1,appid:^b$\nwindowrule = overlay:1,appid:^c$\n",
        );
        assert_eq!(cfg.window_rules.len(), 3);
        for r in &cfg.window_rules {
            assert_eq!(r.is_overlay, Some(true), "overlay not set for {:?}", r.id);
        }
    }

    #[test]
    fn windowrule_group_aliases() {
        let cfg = parse_conf(
            "wrgrp",
            "windowrule = group:1,appid:^a$\nwindowrule = isgroup:1,appid:^b$\n",
        );
        assert_eq!(cfg.window_rules.len(), 2);
        assert_eq!(cfg.window_rules[0].group, Some(true));
        assert_eq!(cfg.window_rules[1].group, Some(true));
    }

    /// A rule whose only selector was misspelled must not widen to every
    /// window/layer: margo drops it. A rule that never had a selector and
    /// has no unknown key is a deliberate catch-all and stays.
    #[test]
    fn rule_without_a_recognised_matcher_but_an_unknown_key_is_dropped() {
        let cfg = parse_config_str(
            "windowrule = isfloating:1, class:^pavucontrol$\n\
             windowrule = opacity:1.0\n\
             layerrule = noanim:1, namespace:^waybar$\n\
             windowrule = isfloating:1\n\
             windowrule = isfloating:1, appid:^a$, bogus:1\n\
             layerrule = noblur:1\n",
            None,
        )
        .unwrap();
        assert_eq!(cfg.window_rules.len(), 2, "{:?}", cfg.window_rules);
        assert!(cfg.window_rules[0].id.is_none());
        assert_eq!(cfg.window_rules[1].id.as_deref(), Some("^a$"));
        assert_eq!(cfg.layer_rules.len(), 1);
        assert!(cfg.layer_rules[0].no_blur);
    }

    /// Drift guard for the rule-option tables the validator checks against:
    /// scrape each rule parser's match arms and compare both ways.
    #[test]
    fn rule_key_tables_match_parser_arms() {
        let src = include_str!("rules.rs");
        let lines: Vec<&str> = src.lines().collect();
        for (fn_name, table) in [
            ("pub(super) fn parse_windowrule", super::WINDOWRULE_KEYS),
            ("pub(super) fn parse_monitorrule", super::MONITORRULE_KEYS),
            ("pub(super) fn parse_tagrule", super::TAGRULE_KEYS),
            ("pub(super) fn parse_layerrule", super::LAYERRULE_KEYS),
        ] {
            let start = lines
                .iter()
                .position(|l| l.trim_start().starts_with(fn_name))
                .expect(fn_name);
            let end = (start..lines.len())
                .find(|&i| lines[i].trim_start().starts_with("other =>"))
                .expect("catch-all arm");
            let mut arms = std::collections::BTreeSet::new();
            for &l in &lines[start..end] {
                let t = l.trim_start();
                if !t.starts_with('"') {
                    continue;
                }
                let pat = t.split("=>").next().unwrap_or(t);
                let mut rest = pat;
                while let Some(a) = rest.find('"') {
                    let after = &rest[a + 1..];
                    let Some(b) = after.find('"') else { break };
                    arms.insert(after[..b].to_string());
                    rest = &after[b + 1..];
                }
            }
            let table: std::collections::BTreeSet<String> =
                table.iter().map(|s| s.to_string()).collect();
            assert_eq!(arms, table, "{fn_name}: match arms and key table differ");
        }
        for m in super::WINDOWRULE_MATCHERS {
            assert!(super::WINDOWRULE_KEYS.contains(m), "{m}");
        }
        for m in super::LAYERRULE_MATCHERS {
            assert!(super::LAYERRULE_KEYS.contains(m), "{m}");
        }
    }
}
