//! `key = value` lines: the option key→`Config` field dispatch, `env =`,
//! and the canonical `OPTION_KEYS` catalogue the validator checks against.

use anyhow::{Result, bail};
use tracing::warn;

use super::values::{
    clamp_keyword, parse_bezier, parse_bool, parse_color, parse_f32, parse_f64, parse_float_list,
    parse_hms_to_seconds, parse_i32, parse_u32,
};
use crate::types::*;

// ── Option key→field dispatch ────────────────────────────────────────────────

pub(super) fn parse_option(cfg: &mut Config, key: &str, val: &str) -> Result<()> {
    match key {
        // animations
        "animations" => cfg.animations = parse_bool(val),
        "layer_animations" => cfg.layer_animations = parse_bool(val),
        "animation_type_open" => cfg.animation_type_open = clamp_keyword(val, 9),
        "animation_type_close" => cfg.animation_type_close = clamp_keyword(val, 9),
        "layer_animation_type_open" => cfg.layer_animation_type_open = clamp_keyword(val, 9),
        "layer_animation_type_close" => cfg.layer_animation_type_close = clamp_keyword(val, 9),
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
        "animation_curve_move" => cfg.animation_curve_move = parse_bezier(val)?,
        "animation_curve_open" => cfg.animation_curve_open = parse_bezier(val)?,
        "animation_curve_tag" => cfg.animation_curve_tag = parse_bezier(val)?,
        "animation_curve_close" => cfg.animation_curve_close = parse_bezier(val)?,
        "animation_curve_focus" => cfg.animation_curve_focus = parse_bezier(val)?,
        "animation_curve_opafadein" => cfg.animation_curve_opafadein = parse_bezier(val)?,
        "animation_curve_opafadeout" => cfg.animation_curve_opafadeout = parse_bezier(val)?,
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
        "animation_spring_mass" => cfg.animation_spring_mass = val.trim().parse().unwrap_or(1.0),

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
        "edge_scroller_focus_allow_speed" => {
            cfg.edge_scroller_focus_allow_speed = parse_f32(val).clamp(0.0, 1000.0)
        }
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
        "color_management" => cfg.color_management = parse_bool(val),
        "default_layout" => cfg.default_layout = val.to_string(),
        // `taglayout = <tag>, <layout>` — repeatable per-tag default layout.
        "taglayout" => {
            if let Some((t, name)) = val.split_once(',')
                && let Ok(tag) = t.trim().parse::<u32>()
            {
                let name = name.trim().to_string();
                if tag >= 1 && !name.is_empty() {
                    // Last write wins for a given tag.
                    cfg.taglayouts.retain(|(existing, _)| *existing != tag);
                    cfg.taglayouts.push((tag, name));
                }
            }
        }
        "taglayout_force" => cfg.taglayout_force = parse_bool(val),
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
        "drag_tile_small" => cfg.drag_tile_small = parse_bool(val),
        "mosaic_auto_overflow_tag" => cfg.mosaic_auto_overflow_tag = parse_bool(val),
        "mosaic_overflow_stack" => cfg.mosaic_overflow_stack = parse_bool(val),
        "swipe_min_threshold" => cfg.swipe_min_threshold = parse_u32(val),
        "focused_opacity" => cfg.focused_opacity = parse_f32(val),
        "unfocused_opacity" => cfg.unfocused_opacity = parse_f32(val),
        "dim_enable" => cfg.dim_enable = parse_bool(val),
        "dim_focused_color" => cfg.dim_focused_color = parse_color(val)?,
        "dim_unfocused_color" => cfg.dim_unfocused_color = parse_color(val)?,

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
        // Legacy alias: 1 = the column (scroller) overview.
        "ov_tab_mode" => {
            cfg.ov_tab_mode = parse_u32(val);
            if cfg.ov_tab_mode == 1 {
                cfg.overview_style = crate::types::OverviewStyle::Scroller;
            }
        }
        "overviewgappi" => cfg.overview_gap_inner = parse_i32(val),
        "overviewgappo" => cfg.overview_gap_outer = parse_i32(val),
        "overview_zoom" => cfg.overview_zoom = parse_f32(val).clamp(0.1, 1.0),
        "overview_transition_ms" => cfg.overview_transition_ms = parse_u32(val),
        "warmup_hidden_ms" => cfg.warmup_hidden_ms = parse_u32(val),
        "overview_selected_border_multiplier" => {
            cfg.overview_selected_border_multiplier = parse_f32(val).clamp(1.0, 4.0)
        }
        "overview_dim_alpha" => cfg.overview_dim_alpha = parse_f32(val).clamp(0.1, 1.0),
        "overview_cycle_order" => {
            cfg.overview_cycle_order = match val.trim().to_ascii_lowercase().as_str() {
                "mru" => crate::types::OverviewCycleOrder::Mru,
                "tag" => crate::types::OverviewCycleOrder::Tag,
                "mixed" => crate::types::OverviewCycleOrder::Mixed,
                other => {
                    tracing::warn!(
                        "config: unknown overview_cycle_order={other:?}; keeping default (mru)"
                    );
                    crate::types::OverviewCycleOrder::Mru
                }
            }
        }
        "overview_style" => {
            cfg.overview_style = match val.trim().to_ascii_lowercase().as_str() {
                "grid" => crate::types::OverviewStyle::Grid,
                "scroller" => crate::types::OverviewStyle::Scroller,
                other => {
                    tracing::warn!(
                        "config: unknown overview_style={other:?}; keeping default (grid)"
                    );
                    crate::types::OverviewStyle::Grid
                }
            }
        }
        "scroller_overview_zoom" => cfg.scroller_overview_zoom = parse_f32(val).clamp(0.1, 1.0),
        "scroller_overview_gap" => cfg.scroller_overview_gap = parse_i32(val).max(0),
        "mru_thumb_height" => cfg.mru_thumb_height = parse_u32(val).clamp(60, 600),
        "mru_scope" => cfg.mru_scope = val.trim().to_lowercase(),
        "mru_filter" => cfg.mru_filter = val.trim().to_lowercase(),
        "mru_show_labels" => cfg.mru_show_labels = parse_bool(val),
        "mru_max" => cfg.mru_max = parse_u32(val).clamp(2, 20),
        "mru_thumb_gap" => cfg.mru_thumb_gap = parse_u32(val).clamp(0, 64),
        "mru_panel_padding" => cfg.mru_panel_padding = parse_u32(val).clamp(0, 80),
        "mru_dim_alpha" => cfg.mru_dim_alpha = parse_f32(val).clamp(0.0, 0.9),
        "mru_accent_selection" => cfg.mru_accent_selection = parse_bool(val),
        "scroller_overview_loop" => cfg.scroller_overview_loop = parse_bool(val),
        "overview_backdrop_color" => cfg.overview_backdrop_color = parse_color(val)?,
        "overview_backdrop_image" => {
            let trimmed = val.trim();
            cfg.overview_backdrop_image = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };
        }

        // ── Twilight (blue-light filter) ───────────────────────────
        "twilight" => cfg.twilight = parse_bool(val),
        "twilight_mode" => {
            cfg.twilight_mode = match val.trim().to_ascii_lowercase().as_str() {
                "geo" => crate::types::TwilightMode::Geo,
                "manual" => crate::types::TwilightMode::Manual,
                "static" => crate::types::TwilightMode::Static,
                "schedule" => crate::types::TwilightMode::Schedule,
                other => {
                    tracing::warn!(
                        "config: unknown twilight_mode={other:?}; keeping default (geo)"
                    );
                    crate::types::TwilightMode::Geo
                }
            }
        }
        "twilight_day_temp" => cfg.twilight_day_temp = parse_u32(val).clamp(1000, 25000),
        "twilight_night_temp" => cfg.twilight_night_temp = parse_u32(val).clamp(1000, 25000),
        "twilight_day_gamma" => cfg.twilight_day_gamma = parse_u32(val).clamp(10, 200),
        "twilight_night_gamma" => cfg.twilight_night_gamma = parse_u32(val).clamp(10, 200),
        "twilight_transition_s" => cfg.twilight_transition_s = parse_u32(val).clamp(30, 7200),
        "twilight_update_interval" => cfg.twilight_update_interval = parse_u32(val).clamp(10, 300),
        "twilight_latitude" => cfg.twilight_latitude = parse_f32(val).clamp(-90.0, 90.0),
        "twilight_longitude" => cfg.twilight_longitude = parse_f32(val).clamp(-180.0, 180.0),
        "twilight_sunrise" => cfg.twilight_sunrise_sec = parse_hms_to_seconds(val),
        "twilight_sunset" => cfg.twilight_sunset_sec = parse_hms_to_seconds(val),
        "twilight_static_temp" => cfg.twilight_static_temp = parse_u32(val).clamp(1000, 25000),
        "twilight_static_gamma" => cfg.twilight_static_gamma = parse_u32(val).clamp(10, 200),
        "twilight_schedule_dir" => cfg.twilight_schedule_dir = val.trim().to_string(),
        "hot_corner_top_left" => cfg.hot_corner_top_left = val.trim().to_string(),
        "hot_corner_top_right" => cfg.hot_corner_top_right = val.trim().to_string(),
        "hot_corner_bottom_left" => cfg.hot_corner_bottom_left = val.trim().to_string(),
        "hot_corner_bottom_right" => cfg.hot_corner_bottom_right = val.trim().to_string(),
        "hot_corner_dwell_ms" => cfg.hot_corner_dwell_ms = parse_u32(val),
        "session_end_grace_ms" => cfg.session_end_grace_ms = parse_u32(val),
        "session_end_countdown_ms" => cfg.session_end_countdown_ms = parse_u32(val),
        "hot_corner_disable_on_fullscreen" => {
            cfg.hot_corner_disable_on_fullscreen = parse_bool(val)
        }
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
        "monly" => cfg.monly = parse_bool(val),
        "gappih" => cfg.gappih = parse_u32(val),
        "gappiv" => cfg.gappiv = parse_u32(val),
        "gappoh" => cfg.gappoh = parse_u32(val),
        "gappov" => cfg.gappov = parse_u32(val),
        "borderpx" => cfg.borderpx = parse_u32(val),

        "tag_carousel" => cfg.tag_carousel = parse_bool(val),
        "tag_gather" => cfg.tag_gather = parse_bool(val),

        // scratchpad
        "scratchpad_width_ratio" => cfg.scratchpad_width_ratio = parse_f32(val),
        "scratchpad_height_ratio" => cfg.scratchpad_height_ratio = parse_f32(val),
        "single_scratchpad" => cfg.single_scratchpad = parse_bool(val),

        // wallpaper
        "wallpaper" => {
            let trimmed = val.trim();
            cfg.wallpaper = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };
        }
        "wallpaper_fit" => {
            use crate::types::WallpaperFit;
            cfg.wallpaper_fit = WallpaperFit::from_name(val).ok_or_else(|| {
                let names = WallpaperFit::names_joined();
                anyhow::anyhow!("wallpaper_fit must be one of {names}, got `{}`", val.trim())
            })?;
        }

        // colours
        "rootcolor" => cfg.rootcolor = parse_color(val)?,
        "bordercolor" => cfg.bordercolor = parse_color(val)?,
        "focuscolor" => cfg.focuscolor = parse_color(val)?,
        "maximizescreencolor" => cfg.maximizescreencolor = parse_color(val)?,
        "urgentcolor" => cfg.urgentcolor = parse_color(val)?,
        "scratchpadcolor" => cfg.scratchpadcolor = parse_color(val)?,
        "floatingcolor" => cfg.floatingcolor = Some(parse_color(val)?),
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

        // tabbed window groups
        "group_bar_height" => cfg.group_bar_height = parse_u32(val),
        "group_bar_gap" => cfg.group_bar_gap = parse_u32(val),
        "group_active_color" => cfg.group_active_color = parse_color(val)?,
        "group_inactive_color" => cfg.group_inactive_color = parse_color(val)?,

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
        // Legacy unified `accel_profile`: backport from mango 0.13
        // splits this into `mouse_accel_profile` + `trackpad_accel_profile`.
        // We accept the old name and write to BOTH new fields so
        // existing configs see no behaviour change. If the user
        // also sets a more specific key below, it wins (handled
        // by the parser visiting later keys after the legacy one).
        "accel_profile" => {
            let profile = match parse_u32(val) {
                0 => AccelProfile::None,
                1 => AccelProfile::Flat,
                _ => AccelProfile::Adaptive,
            };
            cfg.mouse_accel_profile = profile;
            cfg.trackpad_accel_profile = profile;
        }
        "accel_speed" => {
            let speed = parse_f64(val);
            cfg.mouse_accel_speed = speed;
            cfg.trackpad_accel_speed = speed;
        }
        // New 0.13-style explicit keys.
        "mouse_accel_profile" => {
            cfg.mouse_accel_profile = match parse_u32(val) {
                0 => AccelProfile::None,
                1 => AccelProfile::Flat,
                _ => AccelProfile::Adaptive,
            }
        }
        "mouse_accel_speed" => cfg.mouse_accel_speed = parse_f64(val),
        "trackpad_accel_profile" => {
            cfg.trackpad_accel_profile = match parse_u32(val) {
                0 => AccelProfile::None,
                1 => AccelProfile::Flat,
                _ => AccelProfile::Adaptive,
            }
        }
        "trackpad_accel_speed" => cfg.trackpad_accel_speed = parse_f64(val),
        "trackpad_scroll_factor" => cfg.trackpad_scroll_factor = parse_f64(val),
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
        "idle_inhibit_when_fullscreen" => cfg.idle_inhibit_when_fullscreen = parse_bool(val),
        "log_level" => cfg.log_level = parse_i32(val),
        // Deprecated no-op: never wired to anything (smithay starts Xwayland
        // once with a fixed lifecycle). Still accepted so old configs load
        // without an unknown-key warning.
        "xwayland_persistence" => {}
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
        "disable_middle_paste" => cfg.disable_middle_paste = parse_bool(val),
        "auto_reload_config" => cfg.auto_reload_config = parse_bool(val),
        "per_output_frame_clock" => cfg.per_output_frame_clock = parse_bool(val),
        "keymode" => cfg.key_mode = clamp_keyword(val, 27),

        // xkb
        "xkb_rules_rules" => cfg.xkb_rules.rules = val.to_string(),
        "xkb_rules_model" => cfg.xkb_rules.model = val.to_string(),
        "xkb_rules_layout" => cfg.xkb_rules.layout = val.to_string(),
        "xkb_rules_variant" => cfg.xkb_rules.variant = val.to_string(),
        "xkb_rules_options" => cfg.xkb_rules.options = val.to_string(),

        // logging (file logging to ~/.local/state/margo/logs, last N sessions)
        "log_to_file" => cfg.log_to_file = parse_bool(val),
        "log_file_level" => cfg.log_file_level = val.trim().to_ascii_lowercase(),
        "log_keep_sessions" => cfg.log_keep_sessions = parse_u32(val),

        other => {
            warn!("unknown config key: {}", other);
        }
    }
    Ok(())
}

pub(super) fn parse_env(cfg: &mut Config, val: &str) -> Result<()> {
    let mut parts = val.splitn(2, ',');
    let name = parts.next().unwrap_or("").trim().to_string();
    let value = parts.next().unwrap_or("").trim().to_string();
    if name.is_empty() {
        bail!("env directive missing name");
    }
    let home = std::env::var("HOME").ok();
    cfg.envs
        .push((name, expand_tilde_in_env(&value, home.as_deref())));
    Ok(())
}

/// Replace `~/` with `$HOME/` in an `env` value, at the start or after a
/// `:`, `=` or whitespace (so `PATH`-style lists and `--flag=~/x` work,
/// while a stray `a~/b` is left alone). With no `$HOME` the value is kept
/// verbatim rather than failing the whole config load. mango 0.16.2.
fn expand_tilde_in_env(value: &str, home: Option<&str>) -> String {
    let Some(home) = home.filter(|h| !h.is_empty()) else {
        return value.to_string();
    };
    let home = home.trim_end_matches('/');
    let mut out = String::with_capacity(value.len());
    let mut prev: Option<char> = None;
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        let at_word_start = prev.is_none_or(|p| p == ':' || p == '=' || p.is_whitespace());
        if c == '~' && at_word_start && chars.peek() == Some(&'/') {
            out.push_str(home);
        } else {
            out.push(c);
        }
        prev = Some(c);
    }
    out
}

// ── Public key catalogue ─────────────────────────────────────────────────────
//
// Canonical list of recognised top-level option keys, mirrored from
// `parse_option`'s match arms. Used by `crate::validator` so it doesn't
// have to re-implement the dispatch tree. Kept in sync with the parser
// by the `option_keys_match_parser` test below: any key the parser
// accepts but the slice forgets — or vice-versa — flips the test red.
//
// Adding a new option: append the key here AND add the match arm in
// `parse_option`. The test catches drift either direction.

pub const OPTION_KEYS: &[&str] = &[
    "accel_profile",
    "accel_speed",
    "mouse_accel_profile",
    "mouse_accel_speed",
    "trackpad_accel_profile",
    "trackpad_accel_speed",
    "trackpad_scroll_factor",
    "allow_lock_transparent",
    "allow_shortcuts_inhibit",
    "allow_tearing",
    "disable_middle_paste",
    "auto_reload_config",
    "per_output_frame_clock",
    "animation_clock_close",
    "animation_clock_focus",
    "animation_clock_layer",
    "animation_clock_move",
    "animation_clock_open",
    "animation_clock_tag",
    "animation_curve_close",
    "animation_curve_focus",
    "animation_curve_move",
    "animation_curve_opafadein",
    "animation_curve_opafadeout",
    "animation_curve_open",
    "animation_curve_tag",
    "animation_duration_close",
    "animation_duration_focus",
    "animation_duration_move",
    "animation_duration_open",
    "animation_duration_tag",
    "animation_fade_in",
    "animation_fade_out",
    "animations",
    "animation_spring_damping_ratio",
    "animation_spring_mass",
    "animation_spring_stiffness",
    "animation_type_close",
    "animation_type_open",
    "auto_layout",
    "axis_bind_apply_timeout",
    "axis_scroll_factor",
    "blur",
    "blur_layer",
    "blur_optimized",
    "blur_params_brightness",
    "blur_params_contrast",
    "blur_params_noise",
    "blur_params_num_passes",
    "blur_params_radius",
    "blur_params_saturation",
    "bordercolor",
    "borderpx",
    "border_radius",
    "button_map",
    "capslock",
    "center_master_overspread",
    "center_when_single_stack",
    "circle_layout",
    "click_method",
    "color_management",
    "cursor_hide_timeout",
    "cursor_size",
    "cursor_theme",
    "default_layout",
    "taglayout",
    "taglayout_force",
    "default_mfact",
    "default_nmaster",
    "dim_enable",
    "dim_focused_color",
    "dim_unfocused_color",
    "disable_trackpad",
    "disable_while_typing",
    "drag_corner",
    "drag_floating_refresh_interval",
    "drag_lock",
    "drag_tile_refresh_interval",
    "drag_tile_small",
    "drag_tile_to_tile",
    "drag_warp_cursor",
    "mosaic_auto_overflow_tag",
    "mosaic_overflow_stack",
    "edge_scroller_pointer_focus",
    "edge_scroller_focus_allow_speed",
    "enable_floating_snap",
    "enable_hotarea",
    "exchange_cross_monitor",
    "fadein_begin_opacity",
    "fadeout_begin_opacity",
    "floatingcolor",
    "focuscolor",
    "focus_cross_monitor",
    "focus_cross_tag",
    "focused_opacity",
    "focus_on_activate",
    "gappih",
    "gappiv",
    "gappoh",
    "gappov",
    "globalcolor",
    "group_active_color",
    "group_bar_gap",
    "group_bar_height",
    "group_inactive_color",
    "hotarea_corner",
    "hotarea_size",
    "hot_corner_bottom_left",
    "hot_corner_bottom_right",
    "hot_corner_disable_on_fullscreen",
    "hot_corner_dwell_ms",
    "session_end_grace_ms",
    "session_end_countdown_ms",
    "hot_corner_top_left",
    "hot_corner_top_right",
    "idleinhibit_ignore_visible",
    "idle_inhibit_when_fullscreen",
    "keymode",
    "layer_animations",
    "layer_animation_type_close",
    "layer_animation_type_open",
    "layer_shadows",
    "left_handed",
    "log_file_level",
    "log_keep_sessions",
    "log_level",
    "log_to_file",
    "maximizescreencolor",
    "middle_button_emulation",
    "mouse_natural_scrolling",
    "new_is_master",
    "no_border_when_single",
    "no_radius_when_single",
    "numlockon",
    "overlaycolor",
    "overview_cycle_order",
    "overview_dim_alpha",
    "overviewgappi",
    "overviewgappo",
    "overview_selected_border_multiplier",
    "overview_style",
    "overview_transition_ms",
    "warmup_hidden_ms",
    "overview_zoom",
    "ov_tab_mode",
    "scroller_overview_gap",
    "mru_thumb_height",
    "mru_scope",
    "mru_filter",
    "mru_show_labels",
    "mru_max",
    "mru_thumb_gap",
    "mru_panel_padding",
    "mru_dim_alpha",
    "mru_accent_selection",
    "scroller_overview_loop",
    "scroller_overview_zoom",
    "overview_backdrop_color",
    "overview_backdrop_image",
    "repeat_delay",
    "repeat_rate",
    "rootcolor",
    "wallpaper",
    "wallpaper_fit",
    "scratchpadcolor",
    "scratchpad_cross_monitor",
    "scratchpad_height_ratio",
    "scratchpad_width_ratio",
    "scroll_button",
    "scroller_default_proportion",
    "scroller_default_proportion_single",
    "scroller_focus_center",
    "scroller_ignore_proportion_single",
    "scroller_prefer_center",
    "scroller_prefer_overspread",
    "scroller_proportion_preset",
    "scroller_structs",
    "scroll_method",
    "send_events_mode",
    "shadow_only_floating",
    "shadows",
    "shadows_blur",
    "shadowscolor",
    "shadows_position_x",
    "shadows_position_y",
    "shadows_size",
    "single_scratchpad",
    "sloppyfocus",
    "sloppyfocus_arrange",
    "smartgaps",
    "monly",
    "enable_gaps",
    "gaps_enabled",
    // Gap-size aliases handled by parse_option's `gap*`/`gaps_*`
    // arms — must be listed here or the validator wrongly warns
    // W001 (unknown key) on a value the parser happily accepts.
    "gap",
    "gaps",
    "gappi",
    "gaps_in",
    "gapsin",
    "inner_gaps",
    "window_gap",
    "gappo",
    "gaps_out",
    "gapsout",
    "outer_gaps",
    "monitor_gap",
    "snap_distance",
    "swipe_min_threshold",
    "syncobj_enable",
    "tablet_map_to_mon",
    "tag_animation_direction",
    "tag_carousel",
    "tag_gather",
    "tap_and_drag",
    "tap_to_click",
    "touch_degrees_leniency",
    "touch_distance_threshold",
    "touch_edge_size_bottom",
    "touch_edge_size_left",
    "touch_edge_size_right",
    "touch_edge_size_top",
    "touch_timeoutms",
    "trackpad_natural_scrolling",
    "twilight",
    "twilight_day_gamma",
    "twilight_day_temp",
    "twilight_latitude",
    "twilight_longitude",
    "twilight_mode",
    "twilight_night_gamma",
    "twilight_night_temp",
    "twilight_schedule_dir",
    "twilight_static_gamma",
    "twilight_static_temp",
    "twilight_sunrise",
    "twilight_sunset",
    "twilight_transition_s",
    "twilight_update_interval",
    "unfocused_opacity",
    "urgentcolor",
    "view_current_to_back",
    "warpcursor",
    "xkb_rules_layout",
    "xkb_rules_model",
    "xkb_rules_options",
    "xkb_rules_rules",
    "xkb_rules_variant",
    "xwayland_persistence",
    "zoom_end_ratio",
    "zoom_initial_ratio",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::tests::parse_conf;
    use crate::parser::{parse_config, parse_config_str};

    #[test]
    fn env_tilde_expands_at_word_starts_only() {
        let home = Some("/home/kenan");
        assert_eq!(expand_tilde_in_env("~/bin", home), "/home/kenan/bin");
        assert_eq!(
            expand_tilde_in_env("~/bin:~/.local/bin:/usr/bin", home),
            "/home/kenan/bin:/home/kenan/.local/bin:/usr/bin"
        );
        assert_eq!(
            expand_tilde_in_env("--config=~/x --log ~/y", home),
            "--config=/home/kenan/x --log /home/kenan/y"
        );
        // Not at a word start, or not followed by `/`: untouched.
        assert_eq!(expand_tilde_in_env("a~/b", home), "a~/b");
        assert_eq!(expand_tilde_in_env("~user/x", home), "~user/x");
        assert_eq!(expand_tilde_in_env("~", home), "~");
        // A trailing slash on $HOME doesn't double up.
        assert_eq!(
            expand_tilde_in_env("~/x", Some("/home/kenan/")),
            "/home/kenan/x"
        );
    }

    #[test]
    fn dim_knobs_default_off_and_parse_rrggbbaa() {
        let d = parse_config_str("", None).unwrap();
        assert!(!d.dim_enable);
        assert_eq!(d.dim_focused_color.0, [0.0, 0.0, 0.0, 0.0]);
        assert_eq!(d.dim_unfocused_color.0, [0.0, 0.0, 0.0, 85.0 / 255.0]);

        let c = parse_config_str(
            "dim_enable = 1\ndim_focused_color = 0x00000000\ndim_unfocused_color = 0x0000004d\n",
            None,
        )
        .unwrap();
        assert!(c.dim_enable);
        assert_eq!(c.dim_unfocused_color.0, [0.0, 0.0, 0.0, 77.0 / 255.0]);
    }

    #[test]
    fn env_tilde_without_home_is_verbatim() {
        assert_eq!(expand_tilde_in_env("~/bin", None), "~/bin");
        assert_eq!(expand_tilde_in_env("~/bin", Some("")), "~/bin");
    }

    /// Drift guard: every scalar key `parse_option` handles must appear
    /// in `OPTION_KEYS`, or the validator emits a bogus W001 ("unknown
    /// key") for a value the parser silently accepts. The list is
    /// hand-maintained, so scrape `parse_option`'s own source at test
    /// time and diff it against the const rather than trusting a human
    /// to keep two lists in sync.
    #[test]
    fn option_keys_match_parser_arms() {
        let src = include_str!("options.rs");
        let lines: Vec<&str> = src.lines().collect();
        let start = lines
            .iter()
            .position(|l| l.trim_start().starts_with("pub(super) fn parse_option"))
            .expect("parse_option fn");
        // The catch-all arm that warns on an unrecognised key marks the
        // end of the real key list; inner `other =>` arms of nested
        // matches are indented deeper and precede their own warns.
        let end = (start..lines.len())
            .find(|&i| {
                lines[i].trim_start().starts_with("other =>")
                    && lines
                        .get(i + 1)
                        .is_some_and(|n| n.contains("unknown config key"))
            })
            .expect("parse_option catch-all");

        // Top-level match arms sit at exactly 8 spaces of indent; nested
        // match arms (e.g. inside `overview_style`) are deeper, so an
        // indent test cleanly separates keys from enum-variant strings.
        let mut arm_keys = std::collections::BTreeSet::new();
        for &l in &lines[start..end] {
            if !(l.starts_with("        \"") || l.starts_with("        | \"")) {
                continue;
            }
            let pat = l.split("=>").next().unwrap_or(l);
            let mut rest = pat;
            while let Some(a) = rest.find('"') {
                let after = &rest[a + 1..];
                let Some(b) = after.find('"') else { break };
                arm_keys.insert(after[..b].to_string());
                rest = &after[b + 1..];
            }
        }

        let known: std::collections::BTreeSet<String> =
            super::OPTION_KEYS.iter().map(|s| s.to_string()).collect();
        let missing: Vec<&String> = arm_keys.difference(&known).collect();
        assert!(
            missing.is_empty(),
            "parse_option handles keys absent from OPTION_KEYS \
             (the validator would wrongly warn W001 on them): {missing:?}"
        );
    }

    #[test]
    fn parses_taglayouts_and_force() {
        let dir = std::env::temp_dir().join(format!("margo-tl-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let main = dir.join("config.conf");
        std::fs::write(
            &main,
            "taglayout = 1, tile\n\
             taglayout = 2, scroller\n\
             taglayout = 2, grid\n\
             taglayout_force = true\n",
        )
        .unwrap();
        let cfg = parse_config(Some(&main)).unwrap();
        // Last write wins for tag 2 (grid), tag 1 stays tile.
        assert_eq!(
            cfg.taglayouts,
            vec![(1, "tile".to_string()), (2, "grid".to_string())]
        );
        assert!(cfg.taglayout_force);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_tag_carousel_and_edge_scroller_speed() {
        let dir = std::env::temp_dir().join(format!("margo-knobs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let main = dir.join("config.conf");
        std::fs::write(
            &main,
            "tag_carousel = 1\n\
             edge_scroller_focus_allow_speed = 40\n",
        )
        .unwrap();
        let cfg = parse_config(Some(&main)).unwrap();
        assert!(cfg.tag_carousel);
        assert_eq!(cfg.edge_scroller_focus_allow_speed, 40.0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_tag_gather() {
        let dir = std::env::temp_dir().join(format!("margo-tag-gather-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let main = dir.join("config.conf");
        std::fs::write(&main, "tag_gather = 1\n").unwrap();
        let cfg = parse_config(Some(&main)).unwrap();
        assert!(cfg.tag_gather);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_hidden_warmup_ms() {
        let cfg = parse_conf("warmup", "warmup_hidden_ms = 2500\n");
        assert_eq!(cfg.warmup_hidden_ms, 2500);
    }

    #[test]
    fn edge_scroller_speed_is_clamped() {
        let dir = std::env::temp_dir().join(format!("margo-clamp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let main = dir.join("config.conf");
        // Out-of-range values clamp into 0..=1000; negatives floor to 0.
        std::fs::write(&main, "edge_scroller_focus_allow_speed = 99999\n").unwrap();
        assert_eq!(
            parse_config(Some(&main))
                .unwrap()
                .edge_scroller_focus_allow_speed,
            1000.0
        );
        std::fs::write(&main, "edge_scroller_focus_allow_speed = -5\n").unwrap();
        assert_eq!(
            parse_config(Some(&main))
                .unwrap()
                .edge_scroller_focus_allow_speed,
            0.0
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_gap_aliases() {
        let dir = std::env::temp_dir().join(format!("margo-gaps-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let main = dir.join("config.conf");

        std::fs::write(
            &main,
            "enable_gaps=1\ngaps=4\ngaps_in=6\ngaps_out=8\nsmartgaps=1\nmonly=1\n",
        )
        .unwrap();

        let cfg = parse_config(Some(&main)).unwrap();
        assert!(cfg.enable_gaps);
        assert!(cfg.smartgaps);
        assert!(cfg.monly);
        assert_eq!(cfg.gappih, 6);
        assert_eq!(cfg.gappiv, 6);
        assert_eq!(cfg.gappoh, 8);
        assert_eq!(cfg.gappov, 8);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn parses_group_scalar_knobs() {
        // Drift guard: the four `group_*` scalar keys must parse into
        // their Config fields (and not warn as unknown).
        let cfg = parse_conf(
            "grp",
            "group_bar_height = 24\ngroup_bar_gap = 6\n\
             group_active_color = 0x11223344\ngroup_inactive_color = 0x55667788\n",
        );
        assert_eq!(cfg.group_bar_height, 24);
        assert_eq!(cfg.group_bar_gap, 6);
        assert_eq!(
            cfg.group_active_color.0,
            super::Rgba::from_hex(0x11223344).0
        );
        assert_eq!(
            cfg.group_inactive_color.0,
            super::Rgba::from_hex(0x55667788).0
        );
    }

    #[test]
    fn group_defaults_off() {
        // Additivity: with no group_* keys, the strip is hidden (0) so
        // groups stay invisible until explicitly opted into.
        let cfg = parse_conf("grpdef", "borderpx = 2\n");
        assert_eq!(cfg.group_bar_height, 0);
    }

    #[test]
    fn ov_tab_mode_one_selects_the_scroller_overview() {
        let cfg = parse_config_str("ov_tab_mode = 1\n", None).unwrap();
        assert_eq!(cfg.overview_style, crate::types::OverviewStyle::Scroller);
        let cfg = parse_config_str("overview_style = scroller\nov_tab_mode = 0\n", None).unwrap();
        assert_eq!(cfg.overview_style, crate::types::OverviewStyle::Scroller);
    }
}
