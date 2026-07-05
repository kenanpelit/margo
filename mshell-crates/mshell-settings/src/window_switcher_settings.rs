//! Settings → Window Switcher.
//!
//! Configures the compositor's MRU (Super/Alt+Tab) most-recently-used window
//! switcher — thumbnail size, count, theming and default scope/filter. Like the
//! Animations / Behaviour pages these knobs live in margo's `config.conf` (not
//! the shell YAML); every control writes its key immediately and applies live
//! via `mctl reload`.

use crate::compositor_conf::{conf_path, set_and_reload};
use crate::row::Row;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};

/// The parsed compositor config (defaults filled in), for the initial control
/// values. Writes go back through [`set_and_reload`], not this.
fn read_config() -> margo_config::Config {
    margo_config::parse_config_with_defaults(Some(&conf_path())).unwrap_or_default()
}

fn adj(value: f64, lo: f64, hi: f64, step: f64) -> gtk::Adjustment {
    gtk::Adjustment::new(value, lo, hi, step, step * 4.0, 0.0)
}

/// Shared chrome for the two sliders — fixed width, right-aligned value, the
/// `.settings-slider` fill (see sound_settings for the same idiom).
fn slider(lo: f64, hi: f64, step: f64) -> gtk::Scale {
    let s = gtk::Scale::with_range(gtk::Orientation::Horizontal, lo, hi, step);
    s.set_width_request(240);
    s.set_halign(gtk::Align::End);
    s.set_valign(gtk::Align::Center);
    s.set_draw_value(true);
    s.set_value_pos(gtk::PositionType::Right);
    s.add_css_class("settings-slider");
    s
}

fn scope_idx(v: &str) -> u32 {
    match v.trim().to_ascii_lowercase().as_str() {
        "output" => 1,
        "workspace" => 2,
        _ => 0,
    }
}

fn scope_key(idx: u32) -> &'static str {
    match idx {
        1 => "output",
        2 => "workspace",
        _ => "all",
    }
}

fn filter_idx(v: &str) -> u32 {
    match v.trim().to_ascii_lowercase().as_str() {
        "appid" => 1,
        _ => 0,
    }
}

fn filter_key(idx: u32) -> &'static str {
    match idx {
        1 => "appid",
        _ => "all",
    }
}

#[derive(Debug)]
pub(crate) enum WindowSwitcherInput {
    /// A `u32` knob (thumbnail size / max / gap / padding), already clamped.
    SetU32(&'static str, u32),
    /// A boolean knob written as `1`/`0`.
    SetBool(&'static str, bool),
    /// Backdrop dim — slider is 0..90 (%), stored as the 0.0..0.9 float.
    SetDim(f64),
    /// `mru_scope` — `all` / `output` / `workspace`.
    SetScope(&'static str),
    /// `mru_filter` — `all` / `appid`.
    SetFilter(&'static str),
}

#[derive(Debug)]
pub(crate) enum WindowSwitcherOutput {}
#[derive(Debug)]
pub(crate) enum WindowSwitcherCommandOutput {}
pub(crate) struct WindowSwitcherInit {}

pub(crate) struct WindowSwitcherModel {
    max: f64,
    thumb_gap: f64,
    panel_padding: f64,
    show_labels: bool,
    accent_selection: bool,
    scope_list: gtk::StringList,
    filter_list: gtk::StringList,
    scope_idx: u32,
    filter_idx: u32,
}

#[relm4::component(pub)]
impl Component for WindowSwitcherModel {
    type CommandOutput = WindowSwitcherCommandOutput;
    type Input = WindowSwitcherInput;
    type Output = WindowSwitcherOutput;
    type Init = WindowSwitcherInit;

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
                        set_icon_name: Some("view-grid-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label { add_css_class: "settings-hero-title", set_label: "Window Switcher", set_halign: gtk::Align::Start },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "The Super/Alt+Tab most-recently-used switcher — previews, theming and behaviour.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                // ── General ──
                gtk::Label { add_css_class: "label-large-bold", set_label: "General", set_halign: gtk::Align::Start },

                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    #[template] Row {
                        #[template_child] title { set_label: "Thumbnail size" },
                        #[template_child] desc { set_label: "Height of each window preview in the switcher strip." },
                        #[local_ref] thumb_scale -> gtk::Scale {},
                    },
                    #[template] Row {
                        #[template_child] title { set_label: "Max thumbnails" },
                        #[template_child] desc { set_label: "How many previews the strip shows at once (the cycle still walks every window)." },
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_adjustment: &adj(model.max, 2.0, 20.0, 1.0),
                            connect_value_changed[sender] => move |s| sender.input(WindowSwitcherInput::SetU32("mru_max", (s.value().round() as i64).clamp(2, 20) as u32)),
                        },
                    },
                    #[template] Row {
                        #[template_child] title { set_label: "Show app labels" },
                        #[template_child] desc { set_label: "Draw the app-id under each thumbnail." },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[block_signal(labels_h)]
                            set_active: model.show_labels,
                            connect_active_notify[sender] => move |s| sender.input(WindowSwitcherInput::SetBool("mru_show_labels", s.is_active())) @labels_h,
                        },
                    },
                },

                // ── Appearance ──
                gtk::Label { add_css_class: "label-large-bold", set_label: "Appearance", set_halign: gtk::Align::Start },

                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    #[template] Row {
                        #[template_child] title { set_label: "Accent-themed selection" },
                        #[template_child] desc { set_label: "Tint the selected thumbnail, title and label with the theme accent colour." },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[block_signal(accent_h)]
                            set_active: model.accent_selection,
                            connect_active_notify[sender] => move |s| sender.input(WindowSwitcherInput::SetBool("mru_accent_selection", s.is_active())) @accent_h,
                        },
                    },
                    #[template] Row {
                        #[template_child] title { set_label: "Backdrop dim" },
                        #[template_child] desc { set_label: "Darken the desktop behind the switcher (0 = off)." },
                        #[local_ref] dim_scale -> gtk::Scale {},
                    },
                    #[template] Row {
                        #[template_child] title { set_label: "Thumbnail gap" },
                        #[template_child] desc { set_label: "Space between previews (px)." },
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_adjustment: &adj(model.thumb_gap, 0.0, 64.0, 1.0),
                            connect_value_changed[sender] => move |s| sender.input(WindowSwitcherInput::SetU32("mru_thumb_gap", (s.value().round() as i64).clamp(0, 64) as u32)),
                        },
                    },
                    #[template] Row {
                        #[template_child] title { set_label: "Panel padding" },
                        #[template_child] desc { set_label: "Inner padding around the switcher panel (px)." },
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_adjustment: &adj(model.panel_padding, 0.0, 80.0, 1.0),
                            connect_value_changed[sender] => move |s| sender.input(WindowSwitcherInput::SetU32("mru_panel_padding", (s.value().round() as i64).clamp(0, 80) as u32)),
                        },
                    },
                },

                // ── Behaviour ──
                gtk::Label { add_css_class: "label-large-bold", set_label: "Behaviour", set_halign: gtk::Align::Start },

                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    #[template] Row {
                        #[template_child] title { set_label: "Default scope" },
                        #[template_child] desc { set_label: "Which windows the switcher cycles by default." },
                        gtk::DropDown {
                            set_valign: gtk::Align::Center,
                            set_width_request: 180,
                            set_model: Some(&model.scope_list),
                            #[block_signal(scope_h)]
                            set_selected: model.scope_idx,
                            connect_selected_notify[sender] => move |d| sender.input(WindowSwitcherInput::SetScope(scope_key(d.selected()))) @scope_h,
                        },
                    },
                    #[template] Row {
                        #[template_child] title { set_label: "Default filter" },
                        #[template_child] desc { set_label: "Cycle every window, or only windows of the active app." },
                        gtk::DropDown {
                            set_valign: gtk::Align::Center,
                            set_width_request: 180,
                            set_model: Some(&model.filter_list),
                            #[block_signal(filter_h)]
                            set_selected: model.filter_idx,
                            connect_selected_notify[sender] => move |d| sender.input(WindowSwitcherInput::SetFilter(filter_key(d.selected()))) @filter_h,
                        },
                    },
                },
            }
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let cfg = read_config();

        // The two sliders are built here (range + chrome) and mounted via
        // `#[local_ref]`; the initial value is set *before* the change handler
        // is connected so priming the control never fires a spurious write.
        let thumb_scale = slider(60.0, 600.0, 10.0);
        thumb_scale.set_format_value_func(|_, v| format!("{v:.0}"));
        thumb_scale.set_value(cfg.mru_thumb_height as f64);
        {
            let sender = sender.clone();
            thumb_scale.connect_value_changed(move |s| {
                let v = s.value().round().clamp(60.0, 600.0) as u32;
                sender.input(WindowSwitcherInput::SetU32("mru_thumb_height", v));
            });
        }

        let dim_scale = slider(0.0, 90.0, 1.0);
        dim_scale.set_format_value_func(|_, v| format!("{v:.0}%"));
        dim_scale.set_value((cfg.mru_dim_alpha as f64) * 100.0);
        {
            let sender = sender.clone();
            dim_scale.connect_value_changed(move |s| {
                sender.input(WindowSwitcherInput::SetDim(s.value()));
            });
        }

        let model = WindowSwitcherModel {
            max: cfg.mru_max as f64,
            thumb_gap: cfg.mru_thumb_gap as f64,
            panel_padding: cfg.mru_panel_padding as f64,
            show_labels: cfg.mru_show_labels,
            accent_selection: cfg.mru_accent_selection,
            scope_list: gtk::StringList::new(&["All windows", "This output", "This workspace"]),
            filter_list: gtk::StringList::new(&["All windows", "Same app only"]),
            scope_idx: scope_idx(&cfg.mru_scope),
            filter_idx: filter_idx(&cfg.mru_filter),
        };
        let widgets = view_output!();
        let _ = root;
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            WindowSwitcherInput::SetU32(k, v) => set_and_reload(k, v.to_string()),
            WindowSwitcherInput::SetBool(k, v) => {
                set_and_reload(k, if v { "1" } else { "0" }.to_string())
            }
            WindowSwitcherInput::SetDim(pct) => {
                let a = (pct / 100.0).clamp(0.0, 0.9);
                set_and_reload("mru_dim_alpha", format!("{a:.2}"));
            }
            WindowSwitcherInput::SetScope(s) => set_and_reload("mru_scope", s.to_string()),
            WindowSwitcherInput::SetFilter(s) => set_and_reload("mru_filter", s.to_string()),
        }
    }
}
