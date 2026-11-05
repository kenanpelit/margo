//! Frame layout: resize listeners, the side children, and where each menu
//! and plugin surface is positioned.

use super::*;

impl Frame {
    // Can't use sender for this.  Must queue redraw in the callback.  Otherwise, there is a slight
    // delay and the frame isn't draw immediately.
    pub(super) fn attach_resize_listeners(&self, widgets: &FrameWidgets) {
        // NB: neither the spacer's reserved height (layer-shell exclusive
        // zone) NOR the drawn frame band's top/bottom thickness is driven from
        // a `resized` stream any more. The Revealer fires "resized" 60×/s
        // during a bar slide (which re-tiled the compositor every frame and
        // tore window borders off the slide), and — worse — it reports the bar
        // container's *allocated* height, which exceeds the bar's *reserved*
        // height (`bar_center`'s natural measure). Driving the painted band
        // off that allocated height made the frame (TOP layer, above tiled
        // windows) bleed past `work_area` and paint over the lone window's
        // top/bottom border once `smartgaps` collapsed the gap. Both the
        // exclusive zone AND the band are now driven by
        // `BarOutput::ReserveHeight` (→ `FrameInput::SpacerReserve`), which
        // jumps straight to the target once and keeps hole edge == work_area
        // edge. The left/right thickness listeners below stay: side menus have
        // no spacer and their width legitimately tracks the live content.
        let frame_widget = widgets.frame_draw_widget.clone();
        widgets.left_bar_and_menu_container.connect_local(
            "resized",
            false,
            move |values: &[glib::Value]| {
                let width = signal_i32(values, 1);
                frame_widget.update_style(|s| s.left_thickness = width as f64);
                None
            },
        );
        // NOTE: Vertical Left / Right bars were removed; the
        // `left_bar_container` / `right_bar_container` widgets they
        // owned (with `left_spacer` / `right_spacer` listening for
        // resize) are gone with them. Menus that anchor to the side
        // edges still live inside `left_bar_and_menu_container` /
        // `right_bar_and_menu_container`, but they have no spacer —
        // their width is driven by the menu content directly.

        let frame_widget = widgets.frame_draw_widget.clone();
        widgets.right_bar_and_menu_container.connect_local(
            "resized",
            false,
            move |values: &[glib::Value]| {
                let width = signal_i32(values, 1);
                frame_widget.update_style(|s| s.right_thickness = width as f64);
                None
            },
        );

        let frame_widget = widgets.frame_draw_widget.clone();
        widgets.top_left_expander.connect_local(
            "resized",
            false,
            move |values: &[glib::Value]| {
                let height = signal_i32(values, 2);
                frame_widget.update_style(|s| s.left_top_expander_height = height as f64);
                None
            },
        );
        let frame_widget = widgets.frame_draw_widget.clone();
        widgets.top_right_expander.connect_local(
            "resized",
            false,
            move |values: &[glib::Value]| {
                let height = signal_i32(values, 2);
                frame_widget.update_style(|s| s.right_top_expander_height = height as f64);
                None
            },
        );
        let frame_widget = widgets.frame_draw_widget.clone();
        widgets.bottom_left_expander.connect_local(
            "resized",
            false,
            move |values: &[glib::Value]| {
                let height = signal_i32(values, 2);
                frame_widget.update_style(|s| s.left_bottom_expander_height = height as f64);
                None
            },
        );
        let frame_widget = widgets.frame_draw_widget.clone();
        widgets.bottom_right_expander.connect_local(
            "resized",
            false,
            move |values: &[glib::Value]| {
                let height = signal_i32(values, 2);
                frame_widget.update_style(|s| s.right_bottom_expander_height = height as f64);
                None
            },
        );
        let frame_widget = widgets.frame_draw_widget.clone();
        widgets.left_revealer_container.connect_local(
            "resized",
            false,
            move |values: &[glib::Value]| {
                let width = signal_i32(values, 1);
                frame_widget.update_style(|s| s.left_expander_width = width as f64);
                None
            },
        );
        let frame_widget = widgets.frame_draw_widget.clone();
        widgets.right_revealer_container.connect_local(
            "resized",
            false,
            move |values: &[glib::Value]| {
                let width = signal_i32(values, 1);
                frame_widget.update_style(|s| s.right_expander_width = width as f64);
                None
            },
        );

        let frame_widget = widgets.frame_draw_widget.clone();
        widgets.top_revealer_container.connect_local(
            "resized",
            false,
            move |values: &[glib::Value]| {
                let width = signal_i32(values, 1);
                let height = signal_i32(values, 2);
                frame_widget.update_style(|s| s.top_revealer_size = (width as f64, height as f64));
                None
            },
        );

        let frame_widget = widgets.frame_draw_widget.clone();
        widgets.top_left_revealer_container.connect_local(
            "resized",
            false,
            move |values: &[glib::Value]| {
                let width = signal_i32(values, 1);
                let height = signal_i32(values, 2);
                frame_widget
                    .update_style(|s| s.top_left_revealer_size = (width as f64, height as f64));
                None
            },
        );

        let frame_widget = widgets.frame_draw_widget.clone();
        widgets.top_right_revealer_container.connect_local(
            "resized",
            false,
            move |values: &[glib::Value]| {
                let width = signal_i32(values, 1);
                let height = signal_i32(values, 2);
                frame_widget
                    .update_style(|s| s.top_right_revealer_size = (width as f64, height as f64));
                None
            },
        );

        let frame_widget = widgets.frame_draw_widget.clone();
        widgets.bottom_revealer_container.connect_local(
            "resized",
            false,
            move |values: &[glib::Value]| {
                let width = signal_i32(values, 1);
                let height = signal_i32(values, 2);
                frame_widget
                    .update_style(|s| s.bottom_revealer_size = (width as f64, height as f64));
                None
            },
        );

        let frame_widget = widgets.frame_draw_widget.clone();
        widgets.bottom_left_revealer_container.connect_local(
            "resized",
            false,
            move |values: &[glib::Value]| {
                let width = signal_i32(values, 1);
                let height = signal_i32(values, 2);
                frame_widget
                    .update_style(|s| s.bottom_left_revealer_size = (width as f64, height as f64));
                None
            },
        );

        let frame_widget = widgets.frame_draw_widget.clone();
        widgets.bottom_right_revealer_container.connect_local(
            "resized",
            false,
            move |values: &[glib::Value]| {
                let width = signal_i32(values, 1);
                let height = signal_i32(values, 2);
                frame_widget
                    .update_style(|s| s.bottom_right_revealer_size = (width as f64, height as f64));
                None
            },
        );
    }

    pub(super) fn apply_left_and_right_side_children(&self, widgets: &FrameWidgets) {
        // Snapshot which child each region's stack currently shows. The
        // `remove_all` + re-add below would otherwise leave every stack
        // defaulting to its first-added child (clock) — which flashes the
        // clock menu over whatever menu is actually open whenever a widget's
        // position/config change triggers a restack. We restore these at the
        // end so an open menu keeps showing the right child.
        let prev_visible = [
            widgets.left_stack.visible_child_name(),
            widgets.right_stack.visible_child_name(),
            widgets.top_stack.visible_child_name(),
            widgets.top_left_stack.visible_child_name(),
            widgets.top_right_stack.visible_child_name(),
            widgets.bottom_stack.visible_child_name(),
            widgets.bottom_left_stack.visible_child_name(),
            widgets.bottom_right_stack.visible_child_name(),
        ];

        widgets.left_stack.remove_all();
        widgets.right_stack.remove_all();
        widgets.top_stack.remove_all();
        widgets.top_left_stack.remove_all();
        widgets.top_right_stack.remove_all();
        widgets.bottom_stack.remove_all();
        widgets.bottom_left_stack.remove_all();
        widgets.bottom_right_stack.remove_all();

        // Re-attach every built lazy menu at its configured position. Unbuilt
        // (never-opened) menus stay absent until `ensure_menu_built` adds them
        // on first open; a later reposition then re-places them here. Iteration
        // order is irrelevant — an open stack's visible child is restored below
        // and a closed stack's default child is never shown.
        for (name, controller) in &self.menus {
            let widget: Widget = controller.widget().clone().upcast();
            Self::add_to_stack(widgets, &widget, name, &menu_position(name));
        }
        // Eager surfaces: screenshare + the plugin panel (bespoke open paths)
        // and the lazily-built Settings panel (its own model, attached iff built).
        let screenshare_widget: Widget = self.screenshare_menu.widget().clone().upcast();
        Self::add_to_stack(
            widgets,
            &screenshare_widget,
            SCREENSHARE_MENU,
            &menu_position(SCREENSHARE_MENU),
        );
        let plugin_panel_widget: Widget = self.plugin_panel_menu.widget().clone().upcast();
        Self::add_to_stack(
            widgets,
            &plugin_panel_widget,
            NPLUGIN_PANEL_MENU,
            &menu_position(NPLUGIN_PANEL_MENU),
        );
        if let Some(controller) = self.settings_menu.as_ref() {
            let settings_widget: Widget = controller.widget().clone().upcast();
            Self::add_to_stack(
                widgets,
                &settings_widget,
                SETTINGS_MENU,
                &menu_position(SETTINGS_MENU),
            );
        }

        // Restore the pre-restack visible child for each region (see the
        // `prev_visible` snapshot above) so a re-add doesn't flash the
        // first-added menu (clock) over the menu that's actually open.
        let stacks = [
            &widgets.left_stack,
            &widgets.right_stack,
            &widgets.top_stack,
            &widgets.top_left_stack,
            &widgets.top_right_stack,
            &widgets.bottom_stack,
            &widgets.bottom_left_stack,
            &widgets.bottom_right_stack,
        ];
        for (stack, prev) in stacks.iter().zip(prev_visible.iter()) {
            if let Some(name) = prev
                && stack.child_by_name(name.as_str()).is_some()
            {
                stack.set_visible_child_full(name.as_str(), gtk::StackTransitionType::None);
            }
        }
    }

    /// `FrameInput::SetDrawFrame`: show or hide the drawn frame and re-inset
    /// the bars to match.
    pub(super) fn set_draw_frame(&self, widgets: &FrameWidgets, draw_frame: bool) {
        widgets
            .frame_draw_widget
            .update_style(|s| s.draw_frame = draw_frame);
        // The frame_draw_widget is the bars' + menus' only
        // background (they sit transparent on top of it). With
        // it off they'd vanish, so flip `.frame-disabled` on the
        // shared overlay — SCSS then gives each bar/menu its own
        // opaque standalone surface (see `_frame_fallback.scss`).
        if draw_frame {
            widgets.overlay.remove_css_class("frame-disabled");
        } else {
            widgets.overlay.add_css_class("frame-disabled");
        }
        self.strips.set_draw_frame(draw_frame);
        // Toggling `.frame-disabled` changes each bar's natural height
        // (the frameless bar adds an inset floating panel). Push the
        // inset (margin = `frameless_gap`, or 0 when the frame is back
        // on) to both bars now: that re-measures them and refreshes the
        // layer-shell exclusive zone immediately, so windows follow the
        // toggle instead of staying tiled under the old reserve. (relm4
        // queues these, so they run after the CSS class is applied.)
        let gap = config_manager()
            .config()
            .bars()
            .frame()
            .frameless_gap()
            .get_untracked();
        let disabled = !draw_frame;
        self.top_bar
            .sender()
            .emit(BarInput::SetFrameInset { disabled, gap });
        self.bottom_bar
            .sender()
            .emit(BarInput::SetFrameInset { disabled, gap });
    }

    /// `FrameInput::SpacerReserve`: a bar's reserved height changed.
    pub(super) fn spacer_reserve(&self, widgets: &FrameWidgets, is_top: bool, height: i32) {
        let spacer = if is_top {
            &self.top_spacer
        } else {
            &self.bottom_spacer
        };
        let _ = spacer
            .sender()
            .send(FrameSpacerInput::HeightUpdated(height));
        // Lock the drawn frame band to the SAME height as the layer-
        // shell exclusive zone. The band (`top/bottom_thickness`) is
        // otherwise driven off the bar container's *allocated* height
        // (the `resized` listeners below), while the exclusive zone is
        // the bar's *reserved* height (`BarOutput::ReserveHeight`,
        // measured off `bar_center`). When the two differ the frame —
        // which paints on the TOP layer, above tiled windows — bleeds
        // past `work_area` by that difference, painting over the
        // window's top/bottom border. With gaps that overhang hides in
        // the gap; with `smartgaps` (gap → 0) the lone window sits
        // flush against `work_area` and the overhang eats its top and
        // bottom border. Driving the band from the reserve guarantees
        // hole edge == work_area edge, so the border always shows.
        if is_top {
            widgets
                .frame_draw_widget
                .update_style(|s| s.top_thickness = height as f64);
        } else {
            widgets
                .frame_draw_widget
                .update_style(|s| s.bottom_thickness = height as f64);
        }
        self.strips.set_band(is_top, f64::from(height));
    }
}

/// A menu's configured screen anchor, read fresh from the config store by its
/// stable menu-name. The single source of truth for where
/// [`Frame::ensure_menu_built`] first attaches a menu and where
/// [`Frame::apply_left_and_right_side_children`] re-attaches it on a reposition
/// — the two used to duplicate these per-menu `.position().get()` reads.
/// `wizard` shares the settings slot (both are mutually-exclusive centre
/// panels). The fallback is unreachable: callers only pass known menu names.
pub(super) fn menu_position(name: &str) -> Position {
    macro_rules! pos {
        ($menu:ident) => {
            mshell_config::config_manager::config_manager()
                .config()
                .menus()
                .$menu()
                .position()
                .get()
        };
    }
    match name {
        CLOCK_MENU => pos!(clock_menu),
        CLIPBOARD_MENU => pos!(clipboard_menu),
        NOTIFICATION_MENU => pos!(notification_menu),
        SCREENSHOT_MENU => pos!(screenshot_menu),
        APP_LAUNCHER_MENU => pos!(app_launcher_menu),
        WALLPAPER_MENU => pos!(wallpaper_menu),
        SCREENSHARE_MENU => pos!(screenshare_menu),
        // The wizard shares the settings slot/position (both centre panels).
        WIZARD_MENU | SETTINGS_MENU => pos!(settings_menu),
        NUFW_MENU => pos!(ufw_menu),
        PRIVACY_MENU => pos!(privacy_menu),
        BLUETOOTH_MENU => pos!(bluetooth_menu),
        CPU_DASHBOARD_MENU => pos!(cpu_dashboard_menu),
        AUDIO_DASHBOARD_MENU => pos!(audio_dashboard_menu),
        SYSTEM_UPDATE_MENU => pos!(system_update_menu),
        VALENT_MENU => pos!(valent_menu),
        WEATHER_MENU => pos!(weather_menu),
        KEEP_AWAKE_MENU => pos!(keep_awake_menu),
        TWILIGHT_MENU => pos!(twilight_menu),
        KEYBINDS_MENU => pos!(keybinds_menu),
        ALARMCLOCK_MENU => pos!(alarmclock_menu),
        DOCK_MENU => pos!(dock_menu),
        CONTROL_CENTER_MENU => pos!(control_center_menu),
        SSH_MENU => pos!(ssh_menu),
        USB_MENU => pos!(usb_menu),
        DISCORD_MENU => pos!(discord_voice_menu),
        CRASHES_MENU => pos!(crashes_menu),
        NDNS_MENU => pos!(dns_menu),
        NVPN_MENU => pos!(vpn_menu),
        NAI_MENU => pos!(ai_menu),
        NPODMAN_MENU => pos!(podman_menu),
        NGITHUB_MENU => pos!(github_menu),
        NAGENTS_MENU => pos!(agents_menu),
        NNOTES_MENU => pos!(notes_menu),
        NIP_MENU => pos!(ip_menu),
        NVPN_INDICATOR_MENU => pos!(vpn_indicator_menu),
        NNETWORK_MENU => pos!(network_menu),
        NPOWER_MENU => pos!(power_menu),
        MEDIA_PLAYER_MENU => pos!(media_player_menu),
        MTUNE_MENU => pos!(mtune_menu),
        LYRICS_MENU => pos!(lyrics_menu),
        SESSION_MENU => pos!(session_menu),
        MDASH_MENU => pos!(mdash_menu),
        MARGO_LAYOUT_MENU => pos!(margo_layout_menu),
        COLOR_PICKER_MENU => pos!(color_picker_menu),
        NPLUGIN_PANEL_MENU => pos!(plugin_panel_menu),
        _ => Position::TopRight,
    }
}

/// The plugin's composite key from a derived widget name. Names look like
/// `plugin:<composite-key>:<widget-key>` (the composite key may itself contain
/// a `:` for custom sources), so take everything between the prefix and the
/// last `:`.
pub(super) fn plugin_key_from_widget(name: &str) -> Option<String> {
    let rest = name.strip_prefix("plugin:")?;
    let (key, _widget) = rest.rsplit_once(':')?;
    Some(key.to_string())
}

/// Map a stored kebab position string to a menu anchor (default top-right).
pub(super) fn position_from_kebab(s: &str) -> Position {
    match s.trim() {
        "left" => Position::Left,
        "right" => Position::Right,
        "top" => Position::Top,
        "top-left" => Position::TopLeft,
        "bottom" => Position::Bottom,
        "bottom-left" => Position::BottomLeft,
        "bottom-right" => Position::BottomRight,
        _ => Position::TopRight,
    }
}
