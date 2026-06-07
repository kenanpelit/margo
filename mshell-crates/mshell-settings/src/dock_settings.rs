//! Settings → Widgets → mdock.
//!
//! Tunables for mdock (the pinned/running app dock). Two modes — a bar-widget
//! pill (`in_bar`) and a standalone per-output layer-shell surface
//! (`standalone`, with Always / Auto-hide / Toggle behaviour). All live in the
//! shell config (`config.dock`); the dock + surface watch the same store and
//! re-apply live. Pins live in the `pinned_apps_store` cache (pin from the
//! dock's right-click menu), not here.

use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    ConfigStoreFields, DockBehavior, DockPosition, DockStoreFields,
};
use reactive_graph::traits::GetUntracked;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};

fn behavior_index(b: DockBehavior) -> u32 {
    match b {
        DockBehavior::Always => 0,
        DockBehavior::AutoHide => 1,
        DockBehavior::Toggle => 2,
    }
}

fn behavior_from_index(i: u32) -> DockBehavior {
    match i {
        0 => DockBehavior::Always,
        2 => DockBehavior::Toggle,
        _ => DockBehavior::AutoHide,
    }
}

fn position_index(p: DockPosition) -> u32 {
    match p {
        DockPosition::Top => 0,
        DockPosition::Bottom => 1,
        DockPosition::Left => 2,
        DockPosition::Right => 3,
    }
}

fn position_from_index(i: u32) -> DockPosition {
    match i {
        0 => DockPosition::Top,
        2 => DockPosition::Left,
        3 => DockPosition::Right,
        _ => DockPosition::Bottom,
    }
}

#[derive(Debug)]
pub(crate) struct DockSettingsModel {
    icon_size: i32,
    spacing: i32,
    show_tooltips: bool,
    hover_preview: bool,
    show_running: bool,
    in_bar: bool,
    standalone: bool,
    separator: bool,
    launcher_enabled: bool,
    behavior_idx: u32,
    position_idx: u32,
    behaviors: gtk::StringList,
    positions: gtk::StringList,
}

#[derive(Debug)]
pub(crate) enum DockSettingsInput {
    SetIconSize(i32),
    SetSpacing(i32),
    SetShowTooltips(bool),
    SetHoverPreview(bool),
    SetShowRunning(bool),
    SetInBar(bool),
    SetStandalone(bool),
    SetSeparator(bool),
    SetLauncherEnabled(bool),
    SetBehavior(u32),
    SetPosition(u32),
}

#[derive(Debug)]
pub(crate) enum DockSettingsOutput {}

pub(crate) struct DockSettingsInit {}

#[derive(Debug)]
pub(crate) enum DockSettingsCommandOutput {}

#[relm4::component(pub)]
impl Component for DockSettingsModel {
    type CommandOutput = DockSettingsCommandOutput;
    type Input = DockSettingsInput;
    type Output = DockSettingsOutput;
    type Init = DockSettingsInit;

    view! {
        #[root]
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
                    set_orientation: gtk::Orientation::Horizontal,
                    set_halign: gtk::Align::Start,
                    set_spacing: 16,
                    gtk::Image {
                        add_css_class: "settings-hero-icon",
                        set_icon_name: Some("view-grid-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "mdock",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "The pinned / running app dock — as a bar pill and/or a standalone surface. Click an icon to focus that app's window; hover to preview; right-click to pin.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                // ── Modes ───────────────────────────────────────────
                gtk::Label { add_css_class: "label-large-bold", set_label: "Modes", set_halign: gtk::Align::Start },

                #[template] DockRow {
                    #[template_child] title { set_label: "Show in the bar" },
                    #[template_child] desc { set_label: "The classic dock pill inside an mshell bar." },
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        #[block_signal(in_bar_h)]
                        set_active: model.in_bar,
                        connect_active_notify[sender] => move |s| sender.input(DockSettingsInput::SetInBar(s.is_active())) @in_bar_h,
                    },
                },
                #[template] DockRow {
                    #[template_child] title { set_label: "Standalone dock surface" },
                    #[template_child] desc { set_label: "A floating dock window on its own (independent of the bar)." },
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        #[block_signal(standalone_h)]
                        set_active: model.standalone,
                        connect_active_notify[sender] => move |s| sender.input(DockSettingsInput::SetStandalone(s.is_active())) @standalone_h,
                    },
                },
                #[template] DockRow {
                    #[template_child] title { set_label: "Standalone behaviour" },
                    #[template_child] desc { set_label: "Always visible, auto-hide on the edge, or toggle on demand (mshellctl dock toggle)." },
                    gtk::DropDown {
                        set_valign: gtk::Align::Center,
                        set_width_request: 160,
                        set_model: Some(&model.behaviors),
                        #[block_signal(behavior_h)]
                        set_selected: model.behavior_idx,
                        connect_selected_notify[sender] => move |d| sender.input(DockSettingsInput::SetBehavior(d.selected())) @behavior_h,
                    },
                },
                #[template] DockRow {
                    #[template_child] title { set_label: "Standalone position" },
                    #[template_child] desc { set_label: "Which screen edge the standalone dock anchors to." },
                    gtk::DropDown {
                        set_valign: gtk::Align::Center,
                        set_width_request: 160,
                        set_model: Some(&model.positions),
                        #[block_signal(position_h)]
                        set_selected: model.position_idx,
                        connect_selected_notify[sender] => move |d| sender.input(DockSettingsInput::SetPosition(d.selected())) @position_h,
                    },
                },

                // ── Contents ────────────────────────────────────────
                gtk::Label { add_css_class: "label-large-bold", set_label: "Contents", set_halign: gtk::Align::Start, set_margin_top: 12 },

                #[template] DockRow {
                    #[template_child] title { set_label: "Show running apps" },
                    #[template_child] desc { set_label: "Include running apps that aren't pinned. Off = a pinned-only launcher dock." },
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        #[block_signal(show_running_h)]
                        set_active: model.show_running,
                        connect_active_notify[sender] => move |s| sender.input(DockSettingsInput::SetShowRunning(s.is_active())) @show_running_h,
                    },
                },
                #[template] DockRow {
                    #[template_child] title { set_label: "App-launcher button" },
                    #[template_child] desc { set_label: "A launcher button at the end of the dock." },
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        #[block_signal(launcher_h)]
                        set_active: model.launcher_enabled,
                        connect_active_notify[sender] => move |s| sender.input(DockSettingsInput::SetLauncherEnabled(s.is_active())) @launcher_h,
                    },
                },
                #[template] DockRow {
                    #[template_child] title { set_label: "Separator" },
                    #[template_child] desc { set_label: "A divider between the apps and the launcher button." },
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        #[block_signal(separator_h)]
                        set_active: model.separator,
                        connect_active_notify[sender] => move |s| sender.input(DockSettingsInput::SetSeparator(s.is_active())) @separator_h,
                    },
                },

                // ── Appearance / hover ──────────────────────────────
                gtk::Label { add_css_class: "label-large-bold", set_label: "Appearance", set_halign: gtk::Align::Start, set_margin_top: 12 },

                #[template] DockRow {
                    #[template_child] title { set_label: "Icon size" },
                    #[template_child] desc { set_label: "App-icon pixel size in the dock." },
                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_range: (16.0, 96.0),
                        set_increments: (2.0, 8.0),
                        set_digits: 0,
                        #[block_signal(icon_size_h)]
                        set_value: model.icon_size as f64,
                        connect_value_changed[sender] => move |s| sender.input(DockSettingsInput::SetIconSize(s.value() as i32)) @icon_size_h,
                    },
                },
                #[template] DockRow {
                    #[template_child] title { set_label: "Item spacing" },
                    #[template_child] desc { set_label: "Pixels between dock items." },
                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_range: (0.0, 32.0),
                        set_increments: (1.0, 4.0),
                        set_digits: 0,
                        #[block_signal(spacing_h)]
                        set_value: model.spacing as f64,
                        connect_value_changed[sender] => move |s| sender.input(DockSettingsInput::SetSpacing(s.value() as i32)) @spacing_h,
                    },
                },
                #[template] DockRow {
                    #[template_child] title { set_label: "Hover preview" },
                    #[template_child] desc { set_label: "A rich hover card: app name + its open window titles." },
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        #[block_signal(hover_h)]
                        set_active: model.hover_preview,
                        connect_active_notify[sender] => move |s| sender.input(DockSettingsInput::SetHoverPreview(s.is_active())) @hover_h,
                    },
                },
                #[template] DockRow {
                    #[template_child] title { set_label: "Window tooltips" },
                    #[template_child] desc { set_label: "Plain hover tooltip listing window titles (used when Hover preview is off)." },
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        #[block_signal(show_tooltips_h)]
                        set_active: model.show_tooltips,
                        connect_active_notify[sender] => move |s| sender.input(DockSettingsInput::SetShowTooltips(s.is_active())) @show_tooltips_h,
                    },
                },

                // ── Ignore + icon overrides (profile YAML) ──────────
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 4,
                    set_margin_top: 12,
                    gtk::Label { add_css_class: "label-medium-bold", set_halign: gtk::Align::Start, set_label: "Ignore list & icon overrides" },
                    gtk::Label {
                        add_css_class: "label-small",
                        set_halign: gtk::Align::Start,
                        set_label: "Hide specific apps, or map a synthetic --class to an icon, in your profile YAML:",
                        set_xalign: 0.0,
                        set_wrap: true,
                        set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                    },
                    gtk::Label {
                        add_css_class: "label-small",
                        set_halign: gtk::Align::Start,
                        set_xalign: 0.0,
                        set_selectable: true,
                        set_label: "dock:\n  ignore:\n    - Steam\n  icon_overrides:\n    - class: Ai\n      icon: helium",
                    },
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let d = config_manager().config().dock().get_untracked();
        let model = DockSettingsModel {
            icon_size: d.icon_size as i32,
            spacing: d.spacing as i32,
            show_tooltips: d.show_tooltips,
            hover_preview: d.hover_preview,
            show_running: d.show_running,
            in_bar: d.in_bar,
            standalone: d.standalone,
            separator: d.separator,
            launcher_enabled: d.launcher_enabled,
            behavior_idx: behavior_index(d.behavior),
            position_idx: position_index(d.position),
            behaviors: gtk::StringList::new(&["Always", "Auto-hide", "Toggle"]),
            positions: gtk::StringList::new(&["Top", "Bottom", "Left", "Right"]),
        };
        let widgets = view_output!();
        let _ = root;
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            DockSettingsInput::SetIconSize(v) => {
                let v = v.clamp(8, 256);
                self.icon_size = v;
                config_manager().update_config(move |c| c.dock.icon_size = v as u32);
            }
            DockSettingsInput::SetSpacing(v) => {
                let v = v.clamp(0, 64);
                self.spacing = v;
                config_manager().update_config(move |c| c.dock.spacing = v as u32);
            }
            DockSettingsInput::SetShowTooltips(v) => {
                self.show_tooltips = v;
                config_manager().update_config(move |c| c.dock.show_tooltips = v);
            }
            DockSettingsInput::SetHoverPreview(v) => {
                self.hover_preview = v;
                config_manager().update_config(move |c| c.dock.hover_preview = v);
            }
            DockSettingsInput::SetShowRunning(v) => {
                self.show_running = v;
                config_manager().update_config(move |c| c.dock.show_running = v);
            }
            DockSettingsInput::SetInBar(v) => {
                self.in_bar = v;
                config_manager().update_config(move |c| c.dock.in_bar = v);
            }
            DockSettingsInput::SetStandalone(v) => {
                self.standalone = v;
                config_manager().update_config(move |c| c.dock.standalone = v);
            }
            DockSettingsInput::SetSeparator(v) => {
                self.separator = v;
                config_manager().update_config(move |c| c.dock.separator = v);
            }
            DockSettingsInput::SetLauncherEnabled(v) => {
                self.launcher_enabled = v;
                config_manager().update_config(move |c| c.dock.launcher_enabled = v);
            }
            DockSettingsInput::SetBehavior(i) => {
                self.behavior_idx = i;
                let b = behavior_from_index(i);
                config_manager().update_config(move |c| c.dock.behavior = b);
            }
            DockSettingsInput::SetPosition(i) => {
                self.position_idx = i;
                let p = position_from_index(i);
                config_manager().update_config(move |c| c.dock.position = p);
            }
        }
    }
}

/// A settings row: a left-hand title + description, with a trailing control.
#[relm4::widget_template(pub)]
impl relm4::WidgetTemplate for DockRow {
    view! {
        gtk::Box {
            set_orientation: gtk::Orientation::Horizontal,
            set_spacing: 20,
            gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_hexpand: true,
                #[name = "title"]
                gtk::Label {
                    add_css_class: "label-medium-bold",
                    set_halign: gtk::Align::Start,
                    set_hexpand: true,
                },
                #[name = "desc"]
                gtk::Label {
                    add_css_class: "label-small",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                },
            },
        }
    }
}
