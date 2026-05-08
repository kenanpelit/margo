use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use tracing::{error, warn};
use xkbcommon::xkb;

use crate::types::*;

// ── Public entry point ───────────────────────────────────────────────────────

/// Parse the user's mango config file (auto-discovers path if none given).
pub fn parse_config(path: Option<&Path>) -> Result<Config> {
    let resolved = resolve_config_path(path)?;
    let mut cfg = Config::default();
    parse_file(&mut cfg, &resolved, true)?;
    inject_default_chvt_bindings(&mut cfg);
    Ok(cfg)
}

// ── Path resolution ──────────────────────────────────────────────────────────

fn resolve_config_path(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = explicit {
        return Ok(p.to_path_buf());
    }
    let home = std::env::var("HOME").context("HOME not set")?;
    Ok(PathBuf::from(home).join(".config/margo/config.conf"))
}

fn resolve_include_path(include: &str, relative_to: &Path) -> PathBuf {
    if let Some(rel) = include.strip_prefix("./") {
        let dir = relative_to.parent().unwrap_or(Path::new("."));
        dir.join(rel)
    } else if let Some(rest) = include.strip_prefix("~/") {
        let home = std::env::var("HOME").unwrap_or_default();
        PathBuf::from(home).join(rest)
    } else {
        PathBuf::from(include)
    }
}

// ── File-level parsing ───────────────────────────────────────────────────────

fn parse_file(cfg: &mut Config, path: &Path, required: bool) -> Result<()> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            if required {
                bail!("cannot open config file {}: {}", path.display(), e);
            }
            return Ok(());
        }
    };

    for (lineno, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Err(e) = parse_line(cfg, line, path) {
            error!(
                "{}:{}: {} — {:?}",
                path.display(),
                lineno + 1,
                e,
                line
            );
        }
    }
    Ok(())
}

/// Strip an inline `#`-comment that begins at a whitespace boundary
/// (`<space|tab>#…` to end of line) — same convention as
/// sshd_config / sysctl.conf. A `#` flush-against text is preserved
/// so we don't eat hex colours (`0x1e1e2eff`), regex anchors
/// (`^foo#bar$`), or URL fragments embedded in spawn commands.
///
/// Whole-line comments (`#` at column 0) are filtered upstream in
/// `parse_file`; this helper handles the inline case only.
///
/// Caveat: a literal `<space>#` inside a single-line shell command
/// (`bind = …,spawn,sh -c 'echo # foo'`) would also be stripped.
/// In practice that pattern doesn't appear in real configs — shell
/// inline-comments-after-space are rare; users wanting them
/// literal can quote-escape (`#` → `\#` in their shell, which the
/// stripper leaves alone since it's not a config-level `#`) or use
/// a wrapper script.
fn strip_inline_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut prev_ws = false;
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'#' && prev_ws {
            return line[..i].trim_end();
        }
        prev_ws = b == b' ' || b == b'\t';
    }
    line
}

// ── Line-level dispatch ──────────────────────────────────────────────────────

fn parse_line(cfg: &mut Config, line: &str, origin: &Path) -> Result<()> {
    // Strip inline comments before parsing so users can write e.g.
    //   xkb_rules_options = ctrl:nocaps   # CapsLock → Ctrl
    // without the comment becoming part of the value.
    let line = strip_inline_comment(line);
    let (raw_key, raw_val) = split_kv(line).ok_or_else(|| anyhow!("invalid line format"))?;
    let key = raw_key.trim();
    let val = raw_val.trim();

    // include/source directive
    if key == "include" || key == "source" {
        let p = resolve_include_path(val, origin);
        return parse_file(cfg, &p, false);
    }

    // bind variants
    if is_bind_key(key) {
        return parse_bind(cfg, key, val);
    }

    match key {
        "mousebind" => return parse_mousebind(cfg, val),
        "axisbind" => return parse_axisbind(cfg, val),
        "switchbind" => return parse_switchbind(cfg, val),
        "gesturebind" => return parse_gesturebind(cfg, val),
        "touchgesturebind" => return parse_touchgesturebind(cfg, val),
        "windowrule" => return parse_windowrule(cfg, val),
        "monitorrule" => return parse_monitorrule(cfg, val),
        "tagrule" => return parse_tagrule(cfg, val),
        "layerrule" => return parse_layerrule(cfg, val),
        "env" => return parse_env(cfg, val),
        "exec" => {
            cfg.exec.push(val.to_string());
            return Ok(());
        }
        "exec-once" => {
            cfg.exec_once.push(val.to_string());
            return Ok(());
        }
        _ => {}
    }

    parse_option(cfg, key, val)
}

fn is_bind_key(k: &str) -> bool {
    if !k.starts_with("bind") {
        return false;
    }
    k[4..].chars().all(|c| matches!(c, 's' | 'l' | 'r' | 'p'))
}

fn split_kv(line: &str) -> Option<(&str, &str)> {
    let pos = line.find('=')?;
    Some((&line[..pos], &line[pos + 1..]))
}

// ── Option key→field dispatch ────────────────────────────────────────────────

fn parse_option(cfg: &mut Config, key: &str, val: &str) -> Result<()> {
    match key {
        // animations
        "animations" => cfg.animations = parse_bool(val),
        "layer_animations" => cfg.layer_animations = parse_bool(val),
        "animation_type_open" => cfg.animation_type_open = val[..val.len().min(9)].to_string(),
        "animation_type_close" => cfg.animation_type_close = val[..val.len().min(9)].to_string(),
        "layer_animation_type_open" => {
            cfg.layer_animation_type_open = val[..val.len().min(9)].to_string()
        }
        "layer_animation_type_close" => {
            cfg.layer_animation_type_close = val[..val.len().min(9)].to_string()
        }
        "animation_fade_in" => cfg.animation_fade_in = parse_bool(val),
        "animation_fade_out" => cfg.animation_fade_out = parse_bool(val),
        "tag_animation_direction" => {
            cfg.tag_animation_direction = if val.trim() == "1" {
                TagAnimDirection::Vertical
            } else {
                TagAnimDirection::Horizontal
            }
        }
        "zoom_initial_ratio" => cfg.zoom_initial_ratio = parse_f32(val),
        "zoom_end_ratio" => cfg.zoom_end_ratio = parse_f32(val),
        "fadein_begin_opacity" => cfg.fadein_begin_opacity = parse_f32(val),
        "fadeout_begin_opacity" => cfg.fadeout_begin_opacity = parse_f32(val),
        "animation_duration_move" => cfg.animation_duration_move = parse_u32(val),
        "animation_duration_open" => cfg.animation_duration_open = parse_u32(val),
        "animation_duration_tag" => cfg.animation_duration_tag = parse_u32(val),
        "animation_duration_close" => cfg.animation_duration_close = parse_u32(val),
        "animation_duration_focus" => cfg.animation_duration_focus = parse_u32(val),
        "animation_duration_canvas_pan" => cfg.animation_duration_canvas_pan = parse_u32(val),
        "animation_duration_canvas_zoom" => cfg.animation_duration_canvas_zoom = parse_u32(val),
        "animation_curve_move" => cfg.animation_curve_move = parse_bezier(val)?,
        "animation_curve_open" => cfg.animation_curve_open = parse_bezier(val)?,
        "animation_curve_tag" => cfg.animation_curve_tag = parse_bezier(val)?,
        "animation_curve_close" => cfg.animation_curve_close = parse_bezier(val)?,
        "animation_curve_focus" => cfg.animation_curve_focus = parse_bezier(val)?,
        "animation_curve_opafadein" => cfg.animation_curve_opafadein = parse_bezier(val)?,
        "animation_curve_opafadeout" => cfg.animation_curve_opafadeout = parse_bezier(val)?,
        "animation_curve_canvas_pan" => cfg.animation_curve_canvas_pan = parse_bezier(val)?,
        "animation_curve_canvas_zoom" => cfg.animation_curve_canvas_zoom = parse_bezier(val)?,
        "animation_clock_move" => cfg.animation_clock_move = val.trim().to_lowercase(),
        "animation_clock_open" => cfg.animation_clock_open = val.trim().to_lowercase(),
        "animation_clock_close" => cfg.animation_clock_close = val.trim().to_lowercase(),
        "animation_clock_tag" => cfg.animation_clock_tag = val.trim().to_lowercase(),
        "animation_clock_focus" => cfg.animation_clock_focus = val.trim().to_lowercase(),
        "animation_clock_layer" => cfg.animation_clock_layer = val.trim().to_lowercase(),
        "animation_spring_stiffness" => {
            cfg.animation_spring_stiffness = val.trim().parse().unwrap_or(800.0)
        }
        "animation_spring_damping_ratio" => {
            cfg.animation_spring_damping_ratio = val.trim().parse().unwrap_or(1.0)
        }
        "animation_spring_mass" => {
            cfg.animation_spring_mass = val.trim().parse().unwrap_or(1.0)
        }

        // scroller
        "scroller_structs" => cfg.scroller_structs = parse_i32(val),
        "scroller_default_proportion" => cfg.scroller_default_proportion = parse_f32(val),
        "scroller_default_proportion_single" => {
            cfg.scroller_default_proportion_single = parse_f32(val)
        }
        "scroller_ignore_proportion_single" => {
            cfg.scroller_ignore_proportion_single = parse_bool(val)
        }
        "scroller_focus_center" => cfg.scroller_focus_center = parse_bool(val),
        "scroller_prefer_center" => cfg.scroller_prefer_center = parse_bool(val),
        "scroller_prefer_overspread" => cfg.scroller_prefer_overspread = parse_bool(val),
        "edge_scroller_pointer_focus" => cfg.edge_scroller_pointer_focus = parse_bool(val),
        "scroller_proportion_preset" => {
            cfg.scroller_proportion_presets = parse_float_list(val)
                .into_iter()
                .map(|v| v.clamp(0.1, 1.0))
                .collect()
        }
        "circle_layout" => {
            cfg.circle_layouts = val.split(',').map(|s| s.trim().to_string()).collect()
        }

        // focus / layout
        "new_is_master" => cfg.new_is_master = parse_bool(val),
        "default_layout" => cfg.default_layout = val.to_string(),
        "default_mfact" => cfg.default_mfact = parse_f32(val),
        "default_nmaster" => cfg.default_nmaster = parse_u32(val),
        "center_master_overspread" => cfg.center_master_overspread = parse_bool(val),
        "center_when_single_stack" => cfg.center_when_single_stack = parse_bool(val),
        "auto_layout" => cfg.auto_layout = parse_bool(val),
        "focus_cross_monitor" => cfg.focus_cross_monitor = parse_bool(val),
        "exchange_cross_monitor" => cfg.exchange_cross_monitor = parse_bool(val),
        "scratchpad_cross_monitor" => cfg.scratchpad_cross_monitor = parse_bool(val),
        "focus_cross_tag" => cfg.focus_cross_tag = parse_bool(val),
        "view_current_to_back" => cfg.view_current_to_back = parse_bool(val),
        "no_border_when_single" => cfg.no_border_when_single = parse_bool(val),
        "no_radius_when_single" => cfg.no_radius_when_single = parse_bool(val),
        "snap_distance" => cfg.snap_distance = parse_i32(val),
        "enable_floating_snap" => cfg.enable_floating_snap = parse_bool(val),
        "drag_tile_to_tile" => cfg.drag_tile_to_tile = parse_bool(val),
        "swipe_min_threshold" => cfg.swipe_min_threshold = parse_u32(val),
        "focused_opacity" => cfg.focused_opacity = parse_f32(val),
        "unfocused_opacity" => cfg.unfocused_opacity = parse_f32(val),

        // hotarea
        "hotarea_size" => cfg.hotarea_size = parse_u32(val),
        "hotarea_corner" => {
            cfg.hotarea_corner = match parse_i32(val) {
                0 => HotareaCorner::TopLeft,
                1 => HotareaCorner::TopRight,
                2 => HotareaCorner::BottomLeft,
                3 => HotareaCorner::BottomRight,
                _ => HotareaCorner::BottomLeft,
            }
        }
        "enable_hotarea" => cfg.enable_hotarea = parse_bool(val),

        // overview
        "ov_tab_mode" => cfg.ov_tab_mode = parse_u32(val),
        "overviewgappi" => cfg.overview_gap_inner = parse_i32(val),
        "overviewgappo" => cfg.overview_gap_outer = parse_i32(val),
        "cursor_hide_timeout" => cfg.cursor_hide_timeout = parse_u32(val),

        // gaps / borders
        "enable_gaps" | "gaps_enabled" => cfg.enable_gaps = parse_bool(val),
        "gap" | "gaps" => {
            let gap = parse_u32(val);
            cfg.gappih = gap;
            cfg.gappiv = gap;
            cfg.gappoh = gap;
            cfg.gappov = gap;
        }
        "gappi" | "gaps_in" | "gapsin" | "inner_gaps" | "window_gap" => {
            let gap = parse_u32(val);
            cfg.gappih = gap;
            cfg.gappiv = gap;
        }
        "gappo" | "gaps_out" | "gapsout" | "outer_gaps" | "monitor_gap" => {
            let gap = parse_u32(val);
            cfg.gappoh = gap;
            cfg.gappov = gap;
        }
        "smartgaps" => cfg.smartgaps = parse_bool(val),
        "gappih" => cfg.gappih = parse_u32(val),
        "gappiv" => cfg.gappiv = parse_u32(val),
        "gappoh" => cfg.gappoh = parse_u32(val),
        "gappov" => cfg.gappov = parse_u32(val),
        "borderpx" => cfg.borderpx = parse_u32(val),

        // canvas
        "canvas_tiling" => cfg.canvas_tiling = parse_bool(val),
        "canvas_tiling_gap" => cfg.canvas_tiling_gap = parse_i32(val),
        "canvas_pan_on_kill" => cfg.canvas_pan_on_kill = parse_bool(val),
        "canvas_anchor_animate" => cfg.canvas_anchor_animate = parse_bool(val),
        "tag_carousel" => cfg.tag_carousel = parse_bool(val),

        // scratchpad
        "scratchpad_width_ratio" => cfg.scratchpad_width_ratio = parse_f32(val),
        "scratchpad_height_ratio" => cfg.scratchpad_height_ratio = parse_f32(val),
        "single_scratchpad" => cfg.single_scratchpad = parse_bool(val),

        // colours
        "rootcolor" => cfg.rootcolor = parse_color(val)?,
        "bordercolor" => cfg.bordercolor = parse_color(val)?,
        "focuscolor" => cfg.focuscolor = parse_color(val)?,
        "maximizescreencolor" => cfg.maximizescreencolor = parse_color(val)?,
        "urgentcolor" => cfg.urgentcolor = parse_color(val)?,
        "scratchpadcolor" => cfg.scratchpadcolor = parse_color(val)?,
        "globalcolor" => cfg.globalcolor = parse_color(val)?,
        "overlaycolor" => cfg.overlaycolor = parse_color(val)?,
        "shadowscolor" => cfg.shadowscolor = parse_color(val)?,

        // blur / visual
        "blur" => cfg.blur = parse_bool(val),
        "blur_layer" => cfg.blur_layer = parse_bool(val),
        "blur_optimized" => cfg.blur_optimized = parse_bool(val),
        "border_radius" => cfg.border_radius = parse_i32(val),
        "blur_params_num_passes" => cfg.blur_params.num_passes = parse_i32(val),
        "blur_params_radius" => cfg.blur_params.radius = parse_i32(val),
        "blur_params_noise" => cfg.blur_params.noise = parse_f32(val),
        "blur_params_brightness" => cfg.blur_params.brightness = parse_f32(val),
        "blur_params_contrast" => cfg.blur_params.contrast = parse_f32(val),
        "blur_params_saturation" => cfg.blur_params.saturation = parse_f32(val),
        "shadows" => cfg.shadows = parse_bool(val),
        "shadow_only_floating" => cfg.shadow_only_floating = parse_bool(val),
        "layer_shadows" => cfg.layer_shadows = parse_bool(val),
        "shadows_size" => cfg.shadows_size = parse_u32(val),
        "shadows_blur" => cfg.shadows_blur = parse_f32(val),
        "shadows_position_x" => cfg.shadows_position_x = parse_i32(val),
        "shadows_position_y" => cfg.shadows_position_y = parse_i32(val),

        // input
        "repeat_rate" => cfg.repeat_rate = parse_i32(val),
        "repeat_delay" => cfg.repeat_delay = parse_i32(val),
        "numlockon" => cfg.numlock_on = parse_bool(val),
        "capslock" => cfg.capslock = parse_bool(val),
        "disable_trackpad" => cfg.disable_trackpad = parse_bool(val),
        "tap_to_click" => cfg.tap_to_click = parse_bool(val),
        "tap_and_drag" => cfg.tap_and_drag = parse_bool(val),
        "drag_lock" => cfg.drag_lock = parse_bool(val),
        "mouse_natural_scrolling" => cfg.mouse_natural_scrolling = parse_bool(val),
        "trackpad_natural_scrolling" => cfg.trackpad_natural_scrolling = parse_bool(val),
        "disable_while_typing" => cfg.disable_while_typing = parse_bool(val),
        "left_handed" => cfg.left_handed = parse_bool(val),
        "middle_button_emulation" => cfg.middle_button_emulation = parse_bool(val),
        "accel_profile" => {
            cfg.accel_profile = match parse_u32(val) {
                0 => AccelProfile::None,
                1 => AccelProfile::Flat,
                _ => AccelProfile::Adaptive,
            }
        }
        "accel_speed" => cfg.accel_speed = parse_f64(val),
        "scroll_method" => {
            cfg.scroll_method = match parse_u32(val) {
                0 => ScrollMethod::NoScroll,
                1 => ScrollMethod::TwoFinger,
                2 => ScrollMethod::Edge,
                3 => ScrollMethod::OnButtonDown,
                _ => ScrollMethod::TwoFinger,
            }
        }
        "scroll_button" => cfg.scroll_button = parse_u32(val),
        "click_method" => {
            cfg.click_method = match parse_u32(val) {
                0 => ClickMethod::None,
                1 => ClickMethod::ButtonAreas,
                2 => ClickMethod::Clickfinger,
                _ => ClickMethod::ButtonAreas,
            }
        }
        "send_events_mode" => cfg.send_events_mode = parse_u32(val),
        "button_map" => cfg.button_map = parse_u32(val),
        "axis_scroll_factor" => cfg.axis_scroll_factor = parse_f64(val),
        "axis_bind_apply_timeout" => cfg.axis_bind_apply_timeout = parse_u32(val),
        "tablet_map_to_mon" => cfg.tablet_map_to_mon = Some(val.to_string()),

        // touch
        "touch_distance_threshold" => cfg.touch_distance_threshold = parse_f64(val),
        "touch_degrees_leniency" => cfg.touch_degrees_leniency = parse_f64(val),
        "touch_timeoutms" => cfg.touch_timeout_ms = parse_u32(val),
        "touch_edge_size_left" => cfg.touch_edge_size_left = parse_f64(val),
        "touch_edge_size_top" => cfg.touch_edge_size_top = parse_f64(val),
        "touch_edge_size_right" => cfg.touch_edge_size_right = parse_f64(val),
        "touch_edge_size_bottom" => cfg.touch_edge_size_bottom = parse_f64(val),

        // misc
        "sloppyfocus" => cfg.sloppyfocus = parse_bool(val),
        "sloppyfocus_arrange" => cfg.sloppyfocus_arrange = parse_bool(val),
        "warpcursor" => cfg.warpcursor = parse_bool(val),
        "drag_corner" => cfg.drag_corner = parse_i32(val),
        "drag_warp_cursor" => cfg.drag_warp_cursor = parse_bool(val),
        "cursor_theme" => cfg.cursor_theme = Some(val.to_string()),
        "cursor_size" => cfg.cursor_size = parse_u32(val),
        "focus_on_activate" => cfg.focus_on_activate = parse_bool(val),
        "idleinhibit_ignore_visible" => cfg.idleinhibit_ignore_visible = parse_bool(val),
        "log_level" => cfg.log_level = parse_i32(val),
        "xwayland_persistence" => cfg.xwayland_persistence = parse_bool(val),
        "syncobj_enable" => cfg.syncobj_enable = parse_bool(val),
        "drag_tile_refresh_interval" => cfg.drag_tile_refresh_interval = parse_f32(val),
        "drag_floating_refresh_interval" => cfg.drag_floating_refresh_interval = parse_f32(val),
        "allow_tearing" => {
            cfg.allow_tearing = match parse_i32(val) {
                1 => TearingMode::WindowHint,
                2 => TearingMode::Always,
                _ => TearingMode::Disabled,
            }
        }
        "allow_shortcuts_inhibit" => {
            cfg.allow_shortcuts_inhibit = match parse_i32(val) {
                0 => ShortcutsInhibit::Disable,
                2 => ShortcutsInhibit::DenyNew,
                _ => ShortcutsInhibit::Enable,
            }
        }
        "allow_lock_transparent" => cfg.allow_lock_transparent = parse_bool(val),
        "keymode" => cfg.key_mode = val[..val.len().min(27)].to_string(),

        // xkb
        "xkb_rules_rules" => cfg.xkb_rules.rules = val.to_string(),
        "xkb_rules_model" => cfg.xkb_rules.model = val.to_string(),
        "xkb_rules_layout" => cfg.xkb_rules.layout = val.to_string(),
        "xkb_rules_variant" => cfg.xkb_rules.variant = val.to_string(),
        "xkb_rules_options" => cfg.xkb_rules.options = val.to_string(),

        other => {
            warn!("unknown config key: {}", other);
        }
    }
    Ok(())
}

// ── Bind parsing ─────────────────────────────────────────────────────────────

fn parse_bind(cfg: &mut Config, key: &str, val: &str) -> Result<()> {
    let suffix = &key[4..];
    let release_apply = suffix.contains('r');
    let lock_apply = suffix.contains('l');
    let pass_apply = suffix.contains('p');
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
    });
    Ok(())
}

fn parse_mousebind(cfg: &mut Config, val: &str) -> Result<()> {
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

fn parse_axisbind(cfg: &mut Config, val: &str) -> Result<()> {
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

fn parse_switchbind(cfg: &mut Config, val: &str) -> Result<()> {
    let parts = split_csv(val, 8);
    if parts.len() < 2 {
        bail!("switchbind needs at least 2 fields");
    }
    let fold = parse_fold(&parts[0]);
    let action = parts[1].clone();
    let arg = build_arg(&parts[2..]);
    cfg.switch_bindings.push(SwitchBinding { fold, action, arg });
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

fn parse_gesturebind(cfg: &mut Config, val: &str) -> Result<()> {
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

fn parse_touchgesturebind(cfg: &mut Config, val: &str) -> Result<()> {
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

// ── Rule parsing ─────────────────────────────────────────────────────────────

fn parse_windowrule(cfg: &mut Config, val: &str) -> Result<()> {
    let mut rule = WindowRule::default();
    for part in split_csv_colon(val) {
        let (k, v) = part;
        match k.as_str() {
            "appid" | "app_id" => rule.id = Some(v),
            "title" => rule.title = Some(v),
            // Niri-style exclude clauses — invert the match.
            "exclude_appid" | "exclude_app_id" | "not_appid" => {
                rule.exclude_id = Some(v)
            }
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
            "tags" => rule.tags = 1 << (v.parse::<u32>().unwrap_or(1).saturating_sub(1)),
            "monitor" => rule.monitor = Some(v),
            "offsetx" | "offset_x" => rule.offset_x = parse_i32_s(&v),
            "offsety" | "offset_y" => rule.offset_y = parse_i32_s(&v),
            "width" => rule.width = parse_i32_s(&v),
            "height" => rule.height = parse_i32_s(&v),
            "isfloating" | "floating" | "is_floating" => {
                rule.is_floating = Some(parse_bool_s(&v))
            }
            "isfullscreen" | "fullscreen" | "is_fullscreen" => {
                rule.is_fullscreen = Some(parse_bool_s(&v))
            }
            "isfakefullscreen" | "fakefullscreen" | "is_fake_fullscreen" => {
                rule.is_fake_fullscreen = Some(parse_bool_s(&v))
            }
            "scroller_proportion" => rule.scroller_proportion = Some(parse_f32_s(&v)),
            "scroller_proportion_single" => {
                rule.scroller_proportion_single = Some(parse_f32_s(&v))
            }
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
            "isoverlay" => rule.is_overlay = Some(parse_bool_s(&v)),
            "allow_shortcuts_inhibit" => rule.allow_shortcuts_inhibit = Some(parse_bool_s(&v)),
            "ignore_maximize" => rule.ignore_maximize = Some(parse_bool_s(&v)),
            "ignore_minimize" => rule.ignore_minimize = Some(parse_bool_s(&v)),
            "isnosizehint" => rule.no_size_hint = Some(parse_bool_s(&v)),
            "indleinhibit_when_focus" => rule.idle_inhibit_when_focus = Some(parse_bool_s(&v)),
            "nofocus" => rule.no_focus = Some(parse_bool_s(&v)),
            "nofadein" => rule.no_fade_in = Some(parse_bool_s(&v)),
            "nofadeout" => rule.no_fade_out = Some(parse_bool_s(&v)),
            "no_force_center" => rule.no_force_center = Some(parse_bool_s(&v)),
            "isterm" => rule.is_term = Some(parse_bool_s(&v)),
            "allow_csd" => rule.allow_csd = Some(parse_bool_s(&v)),
            "force_fakemaximize" => rule.force_fake_maximize = Some(parse_bool_s(&v)),
            "force_tiled_state" => rule.force_tiled_state = Some(parse_bool_s(&v)),
            "force_tearing" => rule.force_tearing = Some(parse_bool_s(&v)),
            "noswallow" => rule.no_swallow = Some(parse_bool_s(&v)),
            "noblur" => rule.no_blur = Some(parse_bool_s(&v)),
            "canvas_notile" => rule.canvas_no_tile = Some(parse_bool_s(&v)),
            "focused_opacity" => rule.focused_opacity = Some(parse_f32_s(&v)),
            "unfocused_opacity" => rule.unfocused_opacity = Some(parse_f32_s(&v)),
            other => warn!("unknown windowrule option: {}", other),
        }
    }
    cfg.window_rules.push(rule);
    Ok(())
}

fn parse_monitorrule(cfg: &mut Config, val: &str) -> Result<()> {
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
    if rule.name.is_none() && rule.make.is_none() && rule.model.is_none() && rule.serial.is_none()
    {
        bail!("monitorrule must specify at least one of: name, make, model, serial");
    }
    cfg.monitor_rules.push(rule);
    Ok(())
}

fn parse_tagrule(cfg: &mut Config, val: &str) -> Result<()> {
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
            other => warn!("unknown tagrule option: {}", other),
        }
    }
    cfg.tag_rules.push(rule);
    Ok(())
}

fn parse_layerrule(cfg: &mut Config, val: &str) -> Result<()> {
    let mut rule = LayerRule::default();
    for (k, v) in split_csv_colon(val) {
        match k.as_str() {
            "layer_name" => rule.layer_name = Some(v),
            "animation_type_open" => rule.animation_type_open = Some(v),
            "animation_type_close" => rule.animation_type_close = Some(v),
            "noblur" => rule.no_blur = parse_bool_s(&v),
            "noanim" => rule.no_anim = parse_bool_s(&v),
            "noshadow" => rule.no_shadow = parse_bool_s(&v),
            other => warn!("unknown layerrule option: {}", other),
        }
    }
    cfg.layer_rules.push(rule);
    Ok(())
}

fn parse_env(cfg: &mut Config, val: &str) -> Result<()> {
    let mut parts = val.splitn(2, ',');
    let name = parts.next().unwrap_or("").trim().to_string();
    let value = parts.next().unwrap_or("").trim().to_string();
    if name.is_empty() {
        bail!("env directive missing name");
    }
    cfg.envs.push((name, value));
    Ok(())
}

// ── Default ChVT bindings (Ctrl+Alt+F1…F12) ──────────────────────────────────

fn inject_default_chvt_bindings(cfg: &mut Config) {
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
    } else if s0 != "0" {
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
    } else if s1 != "0" {
        arg.v2 = Some(s1.to_string());
    }

    if s2 != "0" && !s2.is_empty() {
        arg.v3 = Some(s2.to_string());
    }

    if let Some(s3) = parts.get(3) {
        if let Ok(v) = s3.parse::<u32>() {
            arg.ui = v;
        }
    }
    if let Some(s4) = parts.get(4) {
        if let Ok(v) = s4.parse::<u32>() {
            arg.ui2 = v;
        }
    }

    arg
}

// ── Modifier parsing ─────────────────────────────────────────────────────────

fn parse_modifiers(s: &str) -> Result<Modifiers> {
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

fn parse_key(s: &str, prefer_sym: bool) -> Result<KeySymCode> {
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

    let flags = if prefer_sym {
        xkb::KEYSYM_NO_FLAGS
    } else {
        xkb::KEYSYM_CASE_INSENSITIVE
    };
    let keysym = xkb::keysym_from_name(s, flags);
    if keysym.raw() == 0u32 {
        bail!("unknown keysym: {}", s);
    }
    Ok(KeySymCode {
        keysym: keysym.raw(),
        keycode: MultiKeycode::default(),
        key_type: KeyType::Sym,
    })
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

// ── Colour parsing ───────────────────────────────────────────────────────────

fn parse_color(s: &str) -> Result<Rgba> {
    let s = s.trim().trim_start_matches("0x").trim_start_matches('#');
    let hex = u32::from_str_radix(s, 16).context("invalid color")?;
    // format: RRGGBBAA (8 digits) or RRGGBB (6 digits)
    let (r, g, b, a) = if s.len() >= 8 {
        (
            ((hex >> 24) & 0xff) as f32 / 255.0,
            ((hex >> 16) & 0xff) as f32 / 255.0,
            ((hex >> 8) & 0xff) as f32 / 255.0,
            (hex & 0xff) as f32 / 255.0,
        )
    } else {
        (
            ((hex >> 16) & 0xff) as f32 / 255.0,
            ((hex >> 8) & 0xff) as f32 / 255.0,
            (hex & 0xff) as f32 / 255.0,
            1.0,
        )
    };
    Ok(Rgba([r, g, b, a]))
}

// ── Bezier curve parsing ─────────────────────────────────────────────────────

fn parse_bezier(s: &str) -> Result<BezierCurve> {
    let nums: Vec<f64> = s
        .split(',')
        .map(|t| t.trim().parse::<f64>())
        .collect::<std::result::Result<_, _>>()
        .context("invalid bezier curve")?;
    if nums.len() != 4 {
        bail!("bezier curve must have 4 values, got {}", nums.len());
    }
    Ok(BezierCurve([nums[0], nums[1], nums[2], nums[3]]))
}

// ── CSV helpers ──────────────────────────────────────────────────────────────

fn split_csv(s: &str, max: usize) -> Vec<String> {
    s.splitn(max, ',')
        .map(|p| p.trim().to_string())
        .collect()
}

/// Split comma-separated `key:value` pairs.
fn split_csv_colon(s: &str) -> Vec<(String, String)> {
    s.split(',')
        .filter_map(|part| {
            let mut it = part.splitn(2, ':');
            let k = it.next()?.trim().to_string();
            let v = it.next().unwrap_or("").trim().to_string();
            Some((k, v))
        })
        .collect()
}

fn parse_float_list(s: &str) -> Vec<f32> {
    s.split(',')
        .filter_map(|t| t.trim().parse().ok())
        .collect()
}

// ── Primitive parsers ────────────────────────────────────────────────────────

fn parse_bool(s: &str) -> bool {
    matches!(s.trim(), "1" | "true" | "yes" | "on")
}
fn parse_i32(s: &str) -> i32 {
    s.trim().parse().unwrap_or(0)
}
fn parse_u32(s: &str) -> u32 {
    s.trim().parse().unwrap_or(0)
}
fn parse_f32(s: &str) -> f32 {
    s.trim().parse().unwrap_or(0.0)
}
fn parse_f64(s: &str) -> f64 {
    s.trim().parse().unwrap_or(0.0)
}
fn parse_bool_s(s: &str) -> bool {
    parse_bool(s)
}

#[cfg(test)]
mod tests {
    use super::{parse_config, strip_inline_comment};

    #[test]
    fn inline_comments_after_whitespace_are_stripped() {
        // Common case the bug surfaced for: option list with trailing comment.
        assert_eq!(
            strip_inline_comment("xkb_rules_options = ctrl:nocaps   # CapsLock → Ctrl"),
            "xkb_rules_options = ctrl:nocaps"
        );
        // Tab before `#` should also count.
        assert_eq!(
            strip_inline_comment("repeat_rate = 35\t# faster keys"),
            "repeat_rate = 35"
        );
    }

    #[test]
    fn flush_hash_is_preserved() {
        // Regex `#` flush against neighbour char — common in
        // window-rule title patterns. The space before `=` is
        // followed by `title:^foo#bar$`, no whitespace right
        // before the `#`, so it stays.
        assert_eq!(
            strip_inline_comment("windowrule = title:^foo#bar$"),
            "windowrule = title:^foo#bar$"
        );
        // URL fragment in spawn argument.
        assert_eq!(
            strip_inline_comment("bind = super,d,spawn,xdg-open https://x.org/page#anchor"),
            "bind = super,d,spawn,xdg-open https://x.org/page#anchor"
        );
    }

    #[test]
    fn no_hash_means_unchanged() {
        assert_eq!(
            strip_inline_comment("default_layout = scroller"),
            "default_layout = scroller"
        );
    }

    #[test]
    fn parses_source_and_unsigned_bind_args() {
        let dir = std::env::temp_dir().join(format!(
            "margo-config-test-{}",
            std::process::id()
        ));
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
        assert!(cfg
            .key_bindings
            .iter()
            .any(|bind| bind.action == "view" && bind.arg.ui == u32::MAX), "view args: {view_args:?}");
        assert!(cfg
            .key_bindings
            .iter()
            .any(|bind| bind.action == "view" && bind.arg.ui == 1), "view args: {view_args:?}");

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn parses_floating_windowrule_with_aliases() {
        let dir = std::env::temp_dir().join(format!(
            "margo-windowrule-test-{}",
            std::process::id()
        ));
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
            Some(r"^(com\.github\.hluk\.copyq|copyq|wiremix|org\.pulseaudio\.pavucontrol|io\.ente\.auth)$")
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn parses_gap_aliases() {
        let dir = std::env::temp_dir().join(format!(
            "margo-gaps-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let main = dir.join("config.conf");

        std::fs::write(
            &main,
            "enable_gaps=1\ngaps=4\ngaps_in=6\ngaps_out=8\nsmartgaps=1\n",
        )
        .unwrap();

        let cfg = parse_config(Some(&main)).unwrap();
        assert!(cfg.enable_gaps);
        assert!(cfg.smartgaps);
        assert_eq!(cfg.gappih, 6);
        assert_eq!(cfg.gappiv, 6);
        assert_eq!(cfg.gappoh, 8);
        assert_eq!(cfg.gappov, 8);

        let _ = std::fs::remove_dir_all(dir);
    }
}
fn parse_i32_s(s: &str) -> i32 {
    parse_i32(s)
}
fn parse_f32_s(s: &str) -> f32 {
    parse_f32(s)
}
fn parse_u32_s(s: &str) -> u32 {
    parse_u32(s)
}
