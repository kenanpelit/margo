//! Settings → Behaviour. Focus, drag, snap, hot corner, scroll, scratchpad +
//! an Advanced expander (sync / tearing / inhibitors) in margo's `config.conf`.

use crate::compositor_conf::{read_bool, read_f64, read_int, read_raw, set_and_reload};
use crate::row::Row;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};

#[derive(Debug)]
pub(crate) enum BehaviourInput {
    SetBool(&'static str, bool),
    SetInt(&'static str, i64),
    SetF(&'static str, f64, usize),
    /// A hot-corner action dropdown: (config key, selected index). Index 0 =
    /// None (empty action), 1 = toggle_overview, 2 = the preserved custom value.
    SetCorner(&'static str, u32),
}

#[derive(Debug)]
pub(crate) enum BehaviourOutput {}
#[derive(Debug)]
pub(crate) enum BehaviourCommandOutput {}
pub(crate) struct BehaviourInit {}

pub(crate) struct BehaviourModel {
    focus_on_activate: bool,
    focus_cross_monitor: bool,
    exchange_cross_monitor: bool,
    focus_cross_tag: bool,
    view_current_to_back: bool,
    sloppyfocus: bool,
    warpcursor: bool,
    cursor_hide_timeout: f64,
    xwayland_persistence: bool,
    drag_tile_to_tile: bool,
    drag_warp_cursor: bool,
    drag_tile_refresh_interval: f64,
    drag_floating_refresh_interval: f64,
    enable_floating_snap: bool,
    snap_distance: f64,
    hot_corner_dwell: f64,
    tl_list: gtk::StringList,
    tr_list: gtk::StringList,
    bl_list: gtk::StringList,
    br_list: gtk::StringList,
    tl_idx: u32,
    tr_idx: u32,
    bl_idx: u32,
    br_idx: u32,
    tl_custom: String,
    tr_custom: String,
    bl_custom: String,
    br_custom: String,
    axis_bind_apply_timeout: f64,
    axis_scroll_factor: f64,
    scratchpad_cross_monitor: bool,
    single_scratchpad: bool,
    syncobj_enable: bool,
    allow_shortcuts_inhibit: bool,
    idleinhibit_ignore_visible: bool,
    corners5: gtk::StringList,
    tearing: gtk::StringList,
    drag_corner_idx: u32,
    allow_tearing_idx: u32,
}

fn adj(value: f64, lo: f64, hi: f64, step: f64) -> gtk::Adjustment {
    gtk::Adjustment::new(value, lo, hi, step, step * 4.0, 0.0)
}

/// Build a hot-corner action dropdown from the current config value:
/// `(item list, selected index, preserved custom string)`. "None" = disabled
/// (empty action); an unrecognised non-empty action is kept as a "Custom: …"
/// entry (index 2) so a hand-edited dispatch isn't silently dropped.
fn corner_dd(key: &str) -> (gtk::StringList, u32, String) {
    match read_raw(key).unwrap_or_default().trim() {
        "" => (
            gtk::StringList::new(&["None", "Toggle overview"]),
            0,
            String::new(),
        ),
        "toggle_overview" => (
            gtk::StringList::new(&["None", "Toggle overview"]),
            1,
            String::new(),
        ),
        other => {
            let label = format!("Custom: {other}");
            (
                gtk::StringList::new(&["None", "Toggle overview", label.as_str()]),
                2,
                other.to_string(),
            )
        }
    }
}

#[relm4::component(pub)]
impl Component for BehaviourModel {
    type CommandOutput = BehaviourCommandOutput;
    type Input = BehaviourInput;
    type Output = BehaviourOutput;
    type Init = BehaviourInit;

    view! {
        gtk::ScrolledWindow {
            set_vscrollbar_policy: gtk::PolicyType::Automatic,
            set_hscrollbar_policy: gtk::PolicyType::Never,
            set_hexpand: true,
            set_vexpand: true,

            gtk::Box {
                add_css_class: "settings-page",
                set_orientation: gtk::Orientation::Vertical,
                set_hexpand: true,
                set_spacing: 16,

                gtk::Box {
                    add_css_class: "settings-hero",
                    set_spacing: 16,
                    gtk::Image {
                        add_css_class: "settings-hero-icon",
                        set_icon_name: Some("preferences-system-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label { add_css_class: "settings-hero-title", set_label: "Behaviour", set_halign: gtk::Align::Start },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "Focus, drag, snapping, hot corner, scroll and scratchpad. Applied live via mctl reload.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                gtk::Label { add_css_class: "label-large-bold", set_label: "Focus", set_halign: gtk::Align::Start },

                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    #[template] Row {
                        #[template_child] title { set_label: "Sloppy focus (focus follows cursor)" },
                        gtk::Switch { set_valign: gtk::Align::Center, set_active: model.sloppyfocus,
                            connect_active_notify[sender] => move |s| sender.input(BehaviourInput::SetBool("sloppyfocus", s.is_active())) } },
                    #[template] Row {
                        #[template_child] title { set_label: "Warp cursor to focused window" },
                        #[template_child] desc { set_label: "Avoid with sloppy focus on — they can ping-pong." },
                        gtk::Switch { set_valign: gtk::Align::Center, set_active: model.warpcursor,
                            connect_active_notify[sender] => move |s| sender.input(BehaviourInput::SetBool("warpcursor", s.is_active())) } },
                    #[template] Row {
                        #[template_child] title { set_label: "Focus a window on activation request" },
                        gtk::Switch { set_valign: gtk::Align::Center, set_active: model.focus_on_activate,
                            connect_active_notify[sender] => move |s| sender.input(BehaviourInput::SetBool("focus_on_activate", s.is_active())) } },
                    #[template] Row {
                        #[template_child] title { set_label: "Let focus cross the monitor edge" },
                        gtk::Switch { set_valign: gtk::Align::Center, set_active: model.focus_cross_monitor,
                            connect_active_notify[sender] => move |s| sender.input(BehaviourInput::SetBool("focus_cross_monitor", s.is_active())) } },
                    #[template] Row {
                        #[template_child] title { set_label: "Let window-exchange cross the monitor edge" },
                        gtk::Switch { set_valign: gtk::Align::Center, set_active: model.exchange_cross_monitor,
                            connect_active_notify[sender] => move |s| sender.input(BehaviourInput::SetBool("exchange_cross_monitor", s.is_active())) } },
                    #[template] Row {
                        #[template_child] title { set_label: "Let focus cross tags" },
                        gtk::Switch { set_valign: gtk::Align::Center, set_active: model.focus_cross_tag,
                            connect_active_notify[sender] => move |s| sender.input(BehaviourInput::SetBool("focus_cross_tag", s.is_active())) } },
                    #[template] Row {
                        #[template_child] title { set_label: "Super+N on the same tag returns to previous" },
                        gtk::Switch { set_valign: gtk::Align::Center, set_active: model.view_current_to_back,
                            connect_active_notify[sender] => move |s| sender.input(BehaviourInput::SetBool("view_current_to_back", s.is_active())) } },
                    #[template] Row {
                        #[template_child] title { set_label: "Hide cursor after inactivity (seconds, 0 = never)" },
                        gtk::SpinButton { set_valign: gtk::Align::Center, set_adjustment: &adj(model.cursor_hide_timeout, 0.0, 30.0, 1.0),
                            connect_value_changed[sender] => move |s| sender.input(BehaviourInput::SetInt("cursor_hide_timeout", s.value() as i64)) } },
                    #[template] Row {
                        #[template_child] title { set_label: "XWayland resize persistence (no flicker)" },
                        gtk::Switch { set_valign: gtk::Align::Center, set_active: model.xwayland_persistence,
                            connect_active_notify[sender] => move |s| sender.input(BehaviourInput::SetBool("xwayland_persistence", s.is_active())) } },
                },

                gtk::Label { add_css_class: "label-large-bold", set_label: "Drag to rearrange", set_halign: gtk::Align::Start },

                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    #[template] Row {
                        #[template_child] title { set_label: "Drag a tile onto another to swap" },
                        gtk::Switch { set_valign: gtk::Align::Center, set_active: model.drag_tile_to_tile,
                            connect_active_notify[sender] => move |s| sender.input(BehaviourInput::SetBool("drag_tile_to_tile", s.is_active())) } },
                    #[template] Row {
                        #[template_child] title { set_label: "Grab corner" },
                        gtk::DropDown { set_valign: gtk::Align::Center, set_width_request: 160,
                            set_model: Some(&model.corners5),
                            #[block_signal(drag_corner_h)]
                            set_selected: model.drag_corner_idx,
                            connect_selected_notify[sender] => move |d| sender.input(BehaviourInput::SetInt("drag_corner", d.selected() as i64)) @drag_corner_h } },
                    #[template] Row {
                        #[template_child] title { set_label: "Warp cursor while dragging" },
                        gtk::Switch { set_valign: gtk::Align::Center, set_active: model.drag_warp_cursor,
                            connect_active_notify[sender] => move |s| sender.input(BehaviourInput::SetBool("drag_warp_cursor", s.is_active())) } },
                    #[template] Row {
                        #[template_child] title { set_label: "Tile drag refresh interval (ms)" },
                        gtk::SpinButton { set_valign: gtk::Align::Center, set_digits: 1, set_adjustment: &adj(model.drag_tile_refresh_interval, 1.0, 60.0, 0.5),
                            connect_value_changed[sender] => move |s| sender.input(BehaviourInput::SetF("drag_tile_refresh_interval", s.value(), 1)) } },
                    #[template] Row {
                        #[template_child] title { set_label: "Floating drag refresh interval (ms)" },
                        gtk::SpinButton { set_valign: gtk::Align::Center, set_digits: 1, set_adjustment: &adj(model.drag_floating_refresh_interval, 1.0, 60.0, 0.5),
                            connect_value_changed[sender] => move |s| sender.input(BehaviourInput::SetF("drag_floating_refresh_interval", s.value(), 1)) } },
                },

                gtk::Label { add_css_class: "label-large-bold", set_label: "Snapping", set_halign: gtk::Align::Start },

                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    #[template] Row {
                        #[template_child] title { set_label: "Floating window snapping" },
                        gtk::Switch { set_valign: gtk::Align::Center, set_active: model.enable_floating_snap,
                            connect_active_notify[sender] => move |s| sender.input(BehaviourInput::SetBool("enable_floating_snap", s.is_active())) } },
                    #[template] Row {
                        #[template_child] title { set_label: "Snap distance (px)" },
                        gtk::SpinButton { set_valign: gtk::Align::Center, set_adjustment: &adj(model.snap_distance, 0.0, 128.0, 1.0),
                            connect_value_changed[sender] => move |s| sender.input(BehaviourInput::SetInt("snap_distance", s.value() as i64)) } },
                },

                gtk::Label { add_css_class: "label-large-bold", set_label: "Hot corners", set_halign: gtk::Align::Start },

                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    #[template] Row {
                        #[template_child] title { set_label: "Top-left corner" },
                        gtk::DropDown { set_valign: gtk::Align::Center, set_width_request: 180,
                            set_model: Some(&model.tl_list),
                            #[block_signal(tl_h)]
                            set_selected: model.tl_idx,
                            connect_selected_notify[sender] => move |d| sender.input(BehaviourInput::SetCorner("hot_corner_top_left", d.selected())) @tl_h } },
                    #[template] Row {
                        #[template_child] title { set_label: "Top-right corner" },
                        gtk::DropDown { set_valign: gtk::Align::Center, set_width_request: 180,
                            set_model: Some(&model.tr_list),
                            #[block_signal(tr_h)]
                            set_selected: model.tr_idx,
                            connect_selected_notify[sender] => move |d| sender.input(BehaviourInput::SetCorner("hot_corner_top_right", d.selected())) @tr_h } },
                    #[template] Row {
                        #[template_child] title { set_label: "Bottom-left corner" },
                        gtk::DropDown { set_valign: gtk::Align::Center, set_width_request: 180,
                            set_model: Some(&model.bl_list),
                            #[block_signal(bl_h)]
                            set_selected: model.bl_idx,
                            connect_selected_notify[sender] => move |d| sender.input(BehaviourInput::SetCorner("hot_corner_bottom_left", d.selected())) @bl_h } },
                    #[template] Row {
                        #[template_child] title { set_label: "Bottom-right corner" },
                        gtk::DropDown { set_valign: gtk::Align::Center, set_width_request: 180,
                            set_model: Some(&model.br_list),
                            #[block_signal(br_h)]
                            set_selected: model.br_idx,
                            connect_selected_notify[sender] => move |d| sender.input(BehaviourInput::SetCorner("hot_corner_bottom_right", d.selected())) @br_h } },
                    #[template] Row {
                        #[template_child] title { set_label: "Dwell before firing (ms)" },
                        #[template_child] desc { set_label: "How long the cursor must rest in a corner before its action fires." },
                        gtk::SpinButton { set_valign: gtk::Align::Center, set_adjustment: &adj(model.hot_corner_dwell, 0.0, 1000.0, 10.0),
                            connect_value_changed[sender] => move |s| sender.input(BehaviourInput::SetInt("hot_corner_dwell_ms", s.value() as i64)) } },
                },

                gtk::Label { add_css_class: "label-large-bold", set_label: "Scroll & scratchpad", set_halign: gtk::Align::Start },

                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    #[template] Row {
                        #[template_child] title { set_label: "Axis-bind apply timeout (ms)" },
                        gtk::SpinButton { set_valign: gtk::Align::Center, set_adjustment: &adj(model.axis_bind_apply_timeout, 0.0, 1000.0, 10.0),
                            connect_value_changed[sender] => move |s| sender.input(BehaviourInput::SetInt("axis_bind_apply_timeout", s.value() as i64)) } },
                    #[template] Row {
                        #[template_child] title { set_label: "Scroll factor" },
                        gtk::SpinButton { set_valign: gtk::Align::Center, set_digits: 2, set_adjustment: &adj(model.axis_scroll_factor, 0.1, 5.0, 0.05),
                            connect_value_changed[sender] => move |s| sender.input(BehaviourInput::SetF("axis_scroll_factor", s.value(), 2)) } },
                    #[template] Row {
                        #[template_child] title { set_label: "Scratchpad follows cursor across monitors" },
                        gtk::Switch { set_valign: gtk::Align::Center, set_active: model.scratchpad_cross_monitor,
                            connect_active_notify[sender] => move |s| sender.input(BehaviourInput::SetBool("scratchpad_cross_monitor", s.is_active())) } },
                    #[template] Row {
                        #[template_child] title { set_label: "Auto-hide other scratchpads" },
                        gtk::Switch { set_valign: gtk::Align::Center, set_active: model.single_scratchpad,
                            connect_active_notify[sender] => move |s| sender.input(BehaviourInput::SetBool("single_scratchpad", s.is_active())) } },
                },

                gtk::Expander {
                    set_label: Some("Advanced — sync, tearing, inhibitors"),
                    set_margin_top: 8,

                    gtk::Box {
                        add_css_class: "boxed-list",
                        set_orientation: gtk::Orientation::Vertical,
                        set_margin_top: 8,

                        #[template] Row {
                            #[template_child] title { set_label: "Explicit sync (syncobj)" },
                            #[template_child] desc { set_label: "Set on for DXVK/Vulkan." },
                            gtk::Switch { set_valign: gtk::Align::Center, set_active: model.syncobj_enable,
                                connect_active_notify[sender] => move |s| sender.input(BehaviourInput::SetBool("syncobj_enable", s.is_active())) } },
                        #[template] Row {
                            #[template_child] title { set_label: "Tearing" },
                            gtk::DropDown { set_valign: gtk::Align::Center, set_width_request: 160,
                                set_model: Some(&model.tearing),
                                #[block_signal(tearing_h)]
                                set_selected: model.allow_tearing_idx,
                                connect_selected_notify[sender] => move |d| sender.input(BehaviourInput::SetInt("allow_tearing", d.selected() as i64)) @tearing_h } },
                        #[template] Row {
                            #[template_child] title { set_label: "Allow apps to inhibit shortcuts" },
                            gtk::Switch { set_valign: gtk::Align::Center, set_active: model.allow_shortcuts_inhibit,
                                connect_active_notify[sender] => move |s| sender.input(BehaviourInput::SetBool("allow_shortcuts_inhibit", s.is_active())) } },
                        #[template] Row {
                            #[template_child] title { set_label: "Idle inhibitor ignores window visibility" },
                            gtk::Switch { set_valign: gtk::Align::Center, set_active: model.idleinhibit_ignore_visible,
                                connect_active_notify[sender] => move |s| sender.input(BehaviourInput::SetBool("idleinhibit_ignore_visible", s.is_active())) } },
                    }
                },
            }
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let _ = &sender;
        let (tl_list, tl_idx, tl_custom) = corner_dd("hot_corner_top_left");
        let (tr_list, tr_idx, tr_custom) = corner_dd("hot_corner_top_right");
        let (bl_list, bl_idx, bl_custom) = corner_dd("hot_corner_bottom_left");
        let (br_list, br_idx, br_custom) = corner_dd("hot_corner_bottom_right");
        let model = BehaviourModel {
            focus_on_activate: read_bool("focus_on_activate", true),
            focus_cross_monitor: read_bool("focus_cross_monitor", false),
            exchange_cross_monitor: read_bool("exchange_cross_monitor", false),
            focus_cross_tag: read_bool("focus_cross_tag", false),
            view_current_to_back: read_bool("view_current_to_back", true),
            sloppyfocus: read_bool("sloppyfocus", true),
            warpcursor: read_bool("warpcursor", false),
            cursor_hide_timeout: read_int("cursor_hide_timeout", 3) as f64,
            xwayland_persistence: read_bool("xwayland_persistence", true),
            drag_tile_to_tile: read_bool("drag_tile_to_tile", true),
            drag_warp_cursor: read_bool("drag_warp_cursor", true),
            drag_tile_refresh_interval: read_f64("drag_tile_refresh_interval", 8.0),
            drag_floating_refresh_interval: read_f64("drag_floating_refresh_interval", 8.0),
            enable_floating_snap: read_bool("enable_floating_snap", true),
            snap_distance: read_int("snap_distance", 30) as f64,
            hot_corner_dwell: read_int("hot_corner_dwell_ms", 100) as f64,
            tl_list,
            tr_list,
            bl_list,
            br_list,
            tl_idx,
            tr_idx,
            bl_idx,
            br_idx,
            tl_custom,
            tr_custom,
            bl_custom,
            br_custom,
            axis_bind_apply_timeout: read_int("axis_bind_apply_timeout", 100) as f64,
            axis_scroll_factor: read_f64("axis_scroll_factor", 1.0),
            scratchpad_cross_monitor: read_bool("scratchpad_cross_monitor", true),
            single_scratchpad: read_bool("single_scratchpad", true),
            syncobj_enable: read_bool("syncobj_enable", false),
            allow_shortcuts_inhibit: read_bool("allow_shortcuts_inhibit", true),
            idleinhibit_ignore_visible: read_bool("idleinhibit_ignore_visible", false),
            corners5: gtk::StringList::new(&[
                "Top-left",
                "Top-right",
                "Bottom-left",
                "Bottom-right",
                "Automatic",
            ]),
            tearing: gtk::StringList::new(&["Off", "On", "Rule-only"]),
            drag_corner_idx: read_int("drag_corner", 4).clamp(0, 4) as u32,
            allow_tearing_idx: read_int("allow_tearing", 2).clamp(0, 2) as u32,
        };
        let widgets = view_output!();
        let _ = root;
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            BehaviourInput::SetBool(k, v) => {
                set_and_reload(k, if v { "1" } else { "0" }.to_string())
            }
            BehaviourInput::SetInt(k, v) => set_and_reload(k, v.to_string()),
            BehaviourInput::SetF(k, v, d) => set_and_reload(k, format!("{:.*}", d, v)),
            BehaviourInput::SetCorner(key, idx) => {
                let action = match idx {
                    1 => "toggle_overview".to_string(),
                    2 => match key {
                        "hot_corner_top_right" => self.tr_custom.clone(),
                        "hot_corner_bottom_left" => self.bl_custom.clone(),
                        "hot_corner_bottom_right" => self.br_custom.clone(),
                        _ => self.tl_custom.clone(),
                    },
                    _ => String::new(),
                };
                set_and_reload(key, action);
            }
        }
    }
}
