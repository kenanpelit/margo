use crate::bar_settings::bar_widget_section::{
    BarSection, WidgetSectionInit, WidgetSectionInput, WidgetSectionModel,
};
use crate::bar_settings::monitor_chip::{MonitorChipModel, MonitorChipOutput};
use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::bar_widgets::BarWidget;
use mshell_config::schema::config::{
    BarsStoreFields, ConfigStoreFields, FrameStoreFields, HorizontalBarStoreFields,
    SizingStoreFields, ThemeAttributesStoreFields, ThemeStoreFields,
};
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::factory::{DynamicIndex, FactoryVecDeque};
use relm4::gtk::prelude::*;
use relm4::gtk::{gdk, gio, glib};
use relm4::{Component, ComponentController, ComponentParts, ComponentSender, Controller, gtk};

/// `gdk::RGBA` → CSS hex `#rrggbbaa`, the form injected as `--frame-bg` /
/// `--frame-border` (GTK4 + the frame-draw widget both parse 8-digit hex).
fn rgba_to_css(c: gdk::RGBA) -> String {
    let q = |f: f32| (f.clamp(0.0, 1.0) * 255.0).round() as u32;
    format!(
        "#{:02x}{:02x}{:02x}{:02x}",
        q(c.red()),
        q(c.green()),
        q(c.blue()),
        q(c.alpha())
    )
}

#[derive(Debug)]
pub(crate) struct BarSettingsModel {
    enable_frame: bool,
    islands: bool,
    /// Manual frame colour override on? Derived from a non-empty
    /// `sizing.frame_color`. When off the frame follows the matugen palette.
    frame_color_custom: bool,
    /// Current frame fill / border colours shown in the pickers.
    frame_color: gdk::RGBA,
    frame_border_color: gdk::RGBA,
    /// Manual separator colour override on? + the colour shown in the picker.
    separator_color_custom: bool,
    separator_color: gdk::RGBA,
    /// Bar show/hide slide animation duration (ms); `bars.slide_duration_ms`.
    slide_duration: i32,
    chips: FactoryVecDeque<MonitorChipModel>,
    available_monitors: Vec<String>,
    selected_monitors: Vec<String>,
    // Vertical Left / Right bar surfaces were removed; the
    // corresponding `_controller` slots and `_min_width` /
    // `_reveal_by_default` flags are gone with them. Only the
    // horizontal Top / Bottom bars remain editable.
    top_bar_start_controller: Controller<WidgetSectionModel>,
    top_bar_center_controller: Controller<WidgetSectionModel>,
    top_bar_end_controller: Controller<WidgetSectionModel>,
    bottom_bar_start_controller: Controller<WidgetSectionModel>,
    bottom_bar_center_controller: Controller<WidgetSectionModel>,
    bottom_bar_end_controller: Controller<WidgetSectionModel>,
    top_enabled: bool,
    bottom_enabled: bool,
    top_min_height: i32,
    bottom_min_height: i32,
    top_reveal_by_default: bool,
    bottom_reveal_by_default: bool,
    top_auto_hide_delay: i32,
    bottom_auto_hide_delay: i32,
    /// Hover tint strength (%) shared by every bar pill.
    bar_hover_strength: i32,
    /// Debounce handles for the two `min_height` spin buttons.
    ///
    /// Each click of the SpinButton's up / down arrow fires
    /// `value_changed`, and `update_config` writes to disk +
    /// triggers a `notify` reload + re-runs every effect bound to
    /// the bars store. Dragging through ten values in a second
    /// turned that into a write storm that occasionally took
    /// mshell down — the bar tree was being rebuilt while still
    /// inside an earlier rebuild. We coalesce here: stage the
    /// value into model state for immediate UI feedback, but only
    /// persist (and re-apply) once the value has settled for
    /// `MIN_HEIGHT_DEBOUNCE_MS`.
    top_min_height_debounce: Option<glib::JoinHandle<()>>,
    bottom_min_height_debounce: Option<glib::JoinHandle<()>>,
    top_auto_hide_delay_debounce: Option<glib::JoinHandle<()>>,
    bottom_auto_hide_delay_debounce: Option<glib::JoinHandle<()>>,
    _effects: EffectScope,
}

/// How long the spin button has to stay still before we commit
/// the value through `update_config`.
const MIN_HEIGHT_DEBOUNCE_MS: u64 = 350;

#[derive(Debug)]
pub(crate) enum BarSettingsInput {
    EnableFrameToggled(bool),
    EnableFrameChanged(bool),
    FrameColorCustomToggled(bool),
    FrameFillColorSet(gdk::RGBA),
    FrameBorderColorSet(gdk::RGBA),
    SeparatorColorCustomToggled(bool),
    SeparatorColorSet(gdk::RGBA),
    IslandsToggled(bool),
    IslandsChanged(bool),
    /// SpinButton edited → write `bars.slide_duration_ms`.
    SlideDurationSet(i32),
    /// Config changed elsewhere → mirror into the SpinButton.
    SlideDurationChanged(i32),
    AddMonitor(String),
    RemoveMonitor(DynamicIndex),
    AvailableMonitorsChanged(Vec<String>),
    SelectedMonitorsChanged(Vec<String>),
    TopMinHeightChanged(i32),
    BottomMinHeightChanged(i32),
    /// Debounced commit — actually persist the staged
    /// `top_min_height` to config.
    CommitTopMinHeight,
    /// Debounced commit — actually persist the staged
    /// `bottom_min_height` to config.
    CommitBottomMinHeight,
    TopRevealByDefaultChanged(bool),
    BottomRevealByDefaultChanged(bool),
    TopEnabledToggled(bool),
    BottomEnabledToggled(bool),
    TopAutoHideDelayChanged(i32),
    BottomAutoHideDelayChanged(i32),
    CommitTopAutoHideDelay,
    CommitBottomAutoHideDelay,
    BarHoverStrengthChanged(i32),
    BarHoverStrengthEffect(i32),

    TopStartEffect(Vec<BarWidget>),
    TopCenterEffect(Vec<BarWidget>),
    TopEndEffect(Vec<BarWidget>),
    BottomStartEffect(Vec<BarWidget>),
    BottomCenterEffect(Vec<BarWidget>),
    BottomEndEffect(Vec<BarWidget>),
    TopMinHeightEffect(i32),
    BottomMinHeightEffect(i32),
    TopRevealByDefaultEffect(bool),
    BottomRevealByDefaultEffect(bool),
    TopEnabledEffect(bool),
    BottomEnabledEffect(bool),
    TopAutoHideDelayEffect(i32),
    BottomAutoHideDelayEffect(i32),
}

#[derive(Debug)]
pub(crate) enum BarSettingsOutput {}

pub(crate) struct BarSettingsInit {}

#[derive(Debug)]
pub(crate) enum BarSettingsCommandOutput {}

#[relm4::component(pub)]
impl Component for BarSettingsModel {
    type CommandOutput = BarSettingsCommandOutput;
    type Input = BarSettingsInput;
    type Output = BarSettingsOutput;
    type Init = BarSettingsInit;

    view! {
        #[root]
        gtk::ScrolledWindow {
            set_vscrollbar_policy: gtk::PolicyType::Automatic,
            set_hscrollbar_policy: gtk::PolicyType::Never,
            set_propagate_natural_height: false,
            set_propagate_natural_width: false,
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
                        set_icon_name: Some("computer-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "Bar",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "Frame anchoring, density, pill order, monitor visibility.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Frame",
                    set_halign: gtk::Align::Start,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    gtk::Label {
                        add_css_class: "label-small",
                        set_halign: gtk::Align::Start,
                        set_label: "Enable frame drawing.",
                        set_hexpand: true,
                    },

                    gtk::Switch {
                        #[watch]
                        #[block_signal(enable_frame_handler)]
                        set_active: model.enable_frame,
                        connect_state_set[sender] => move |_, enabled| {
                            sender.input(BarSettingsInput::EnableFrameToggled(enabled));
                            glib::Propagation::Proceed
                        } @enable_frame_handler,
                    }
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    gtk::Label {
                        add_css_class: "label-small",
                        set_halign: gtk::Align::Start,
                        set_label: "Custom frame colour — override the wallpaper-derived (matugen) frame fill + border with fixed colours. Off = follow the theme.",
                        set_hexpand: true,
                        set_xalign: 0.0,
                        set_wrap: true,
                        set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                    },

                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        #[watch]
                        #[block_signal(frame_color_custom_handler)]
                        set_active: model.frame_color_custom,
                        connect_state_set[sender] => move |_, enabled| {
                            sender.input(BarSettingsInput::FrameColorCustomToggled(enabled));
                            glib::Propagation::Proceed
                        } @frame_color_custom_handler,
                    }
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 8,
                    #[watch]
                    set_sensitive: model.frame_color_custom,

                    gtk::Label {
                        add_css_class: "label-small",
                        set_label: "Frame fill / border",
                        set_halign: gtk::Align::Start,
                        set_hexpand: true,
                        set_xalign: 0.0,
                    },
                    gtk::Label {
                        add_css_class: "label-small",
                        set_label: "Fill",
                        set_halign: gtk::Align::End,
                    },
                    gtk::ColorDialogButton {
                        set_valign: gtk::Align::Center,
                        set_dialog: &gtk::ColorDialog::builder().with_alpha(true).build(),
                        #[watch]
                        set_rgba: &model.frame_color,
                        connect_rgba_notify[sender] => move |b| {
                            sender.input(BarSettingsInput::FrameFillColorSet(b.rgba()));
                        },
                    },
                    gtk::Label {
                        add_css_class: "label-small",
                        set_label: "Border",
                        set_halign: gtk::Align::End,
                    },
                    gtk::ColorDialogButton {
                        set_valign: gtk::Align::Center,
                        set_dialog: &gtk::ColorDialog::builder().with_alpha(true).build(),
                        #[watch]
                        set_rgba: &model.frame_border_color,
                        connect_rgba_notify[sender] => move |b| {
                            sender.input(BarSettingsInput::FrameBorderColorSet(b.rgba()));
                        },
                    },
                },

                // ── Separator colour ────────────────────────────────────
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    gtk::Label {
                        add_css_class: "label-small",
                        set_halign: gtk::Align::Start,
                        set_label: "Custom separator colour — the thin divider pills (Separator widget). Off = follow the theme (matugen outline).",
                        set_hexpand: true,
                        set_xalign: 0.0,
                        set_wrap: true,
                        set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                    },

                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        #[watch]
                        #[block_signal(separator_color_custom_handler)]
                        set_active: model.separator_color_custom,
                        connect_state_set[sender] => move |_, enabled| {
                            sender.input(BarSettingsInput::SeparatorColorCustomToggled(enabled));
                            glib::Propagation::Proceed
                        } @separator_color_custom_handler,
                    }
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 8,
                    #[watch]
                    set_sensitive: model.separator_color_custom,

                    gtk::Label {
                        add_css_class: "label-small",
                        set_label: "Separator",
                        set_halign: gtk::Align::Start,
                        set_hexpand: true,
                        set_xalign: 0.0,
                    },
                    gtk::ColorDialogButton {
                        set_valign: gtk::Align::Center,
                        set_dialog: &gtk::ColorDialog::builder().with_alpha(true).build(),
                        #[watch]
                        set_rgba: &model.separator_color,
                        connect_rgba_notify[sender] => move |b| {
                            sender.input(BarSettingsInput::SeparatorColorSet(b.rgba()));
                        },
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    gtk::Label {
                        add_css_class: "label-small",
                        set_halign: gtk::Align::Start,
                        set_label: "Islands: a transparent bar where each pill floats as its own rounded surface (instead of one continuous strip). Applies immediately.",
                        set_hexpand: true,
                        set_xalign: 0.0,
                        set_wrap: true,
                        set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                    },

                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        #[watch]
                        #[block_signal(islands_handler)]
                        set_active: model.islands,
                        connect_state_set[sender] => move |_, enabled| {
                            sender.input(BarSettingsInput::IslandsToggled(enabled));
                            glib::Propagation::Proceed
                        } @islands_handler,
                    }
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    gtk::Label {
                        add_css_class: "label-small",
                        set_halign: gtk::Align::Start,
                        set_label: "Bar slide animation (ms). Match the compositor's window move-animation (margo animation_duration_move) so a bar toggle stays glued to the windows. 0 = instant. Applies immediately.",
                        set_hexpand: true,
                        set_xalign: 0.0,
                        set_wrap: true,
                        set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                    },

                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_range: (0.0, 2000.0),
                        set_increments: (50.0, 100.0),
                        #[watch]
                        #[block_signal(slide_duration_handler)]
                        set_value: model.slide_duration as f64,
                        connect_value_changed[sender] => move |s| {
                            sender.input(BarSettingsInput::SlideDurationSet(s.value() as i32));
                        } @slide_duration_handler,
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,

                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Monitors",
                            set_hexpand: true,
                        },

                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Monitors to show the frame on. If empty, a frame will show on all monitors.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },

                        // Empty state
                        gtk::Label {
                            #[watch]
                            set_visible: model.selected_monitors.is_empty(),
                            set_label: "All monitors",
                            set_halign: gtk::Align::Start,
                            set_css_classes: &["monitor-chip-empty", "label-small-primary"],
                        },

                        // Chips
                        #[local_ref]
                        chip_box -> gtk::FlowBox {
                            set_selection_mode: gtk::SelectionMode::None,
                            set_row_spacing: 4,
                            set_column_spacing: 4,
                            set_homogeneous: false,
                            #[watch]
                            set_visible: !model.selected_monitors.is_empty(),
                        },
                    },

                    #[name = "add_monitor_button"]
                    gtk::MenuButton {
                        set_label: "Add monitor",
                        set_vexpand: false,
                        set_hexpand: false,
                        set_halign: gtk::Align::Start,
                        set_valign: gtk::Align::Start,
                        #[watch]
                        set_sensitive: model.has_unselected_monitors(),
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_hexpand: true,
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Hover strength (%)",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Tint opacity of every bar pill's hover — one value for all widgets so they highlight identically.",
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_range: (0.0, 60.0),
                        set_increments: (1.0, 5.0),
                        #[watch]
                        #[block_signal(bar_hover_handler)]
                        set_value: model.bar_hover_strength as f64,
                        connect_value_changed[sender] => move |s| {
                            sender.input(BarSettingsInput::BarHoverStrengthChanged(s.value() as i32));
                        } @bar_hover_handler,
                    },
                },

                gtk::Separator {},

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Top Bar",
                    set_halign: gtk::Align::Start,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Show this bar",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Off turns the bar off entirely — it never appears, not even on hover.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        #[watch]
                        #[block_signal(top_enabled_handler)]
                        set_active: model.top_enabled,
                        connect_state_set[sender] => move |_, enabled| {
                            sender.input(BarSettingsInput::TopEnabledToggled(enabled));
                            glib::Propagation::Proceed
                        } @top_enabled_handler,
                    }
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    gtk::Label {
                        add_css_class: "label-small",
                        set_halign: gtk::Align::Start,
                        set_label: "Minimum Height",
                        set_hexpand: true,
                    },

                    gtk::SpinButton {
                        set_range: (0.0, 500.0),
                        set_increments: (1.0, 10.0),
                        #[watch]
                        #[block_signal(top_min_handler)]
                        set_value: model.top_min_height as f64,
                        connect_value_changed[sender] => move |s| {
                            sender.input(BarSettingsInput::TopMinHeightChanged(s.value() as i32));
                        } @top_min_handler,
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,

                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Auto-hide",
                            set_hexpand: true,
                        },

                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Hide the bar; it slides in when you move the pointer to its screen edge.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        // Auto-hide is the inverse of reveal_by_default.
                        #[watch]
                        #[block_signal(top_reveal_by_default_handler)]
                        set_active: !model.top_reveal_by_default,
                        connect_state_set[sender] => move |_, auto_hide| {
                            sender.input(BarSettingsInput::TopRevealByDefaultChanged(!auto_hide));
                            glib::Propagation::Proceed
                        } @top_reveal_by_default_handler,
                    }
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    #[watch]
                    set_sensitive: !model.top_reveal_by_default,

                    gtk::Label {
                        add_css_class: "label-small",
                        set_halign: gtk::Align::Start,
                        set_label: "Auto-hide delay (ms)",
                        set_hexpand: true,
                    },

                    gtk::SpinButton {
                        set_range: (0.0, 5000.0),
                        set_increments: (50.0, 250.0),
                        #[watch]
                        #[block_signal(top_auto_hide_delay_handler)]
                        set_value: model.top_auto_hide_delay as f64,
                        connect_value_changed[sender] => move |s| {
                            sender.input(BarSettingsInput::TopAutoHideDelayChanged(s.value() as i32));
                        } @top_auto_hide_delay_handler,
                    },
                },

                model.top_bar_start_controller.widget().clone() {},
                model.top_bar_center_controller.widget().clone() {},
                model.top_bar_end_controller.widget().clone() {},

                gtk::Separator {},

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Bottom Bar",
                    set_halign: gtk::Align::Start,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Show this bar",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Off turns the bar off entirely — it never appears, not even on hover.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        #[watch]
                        #[block_signal(bottom_enabled_handler)]
                        set_active: model.bottom_enabled,
                        connect_state_set[sender] => move |_, enabled| {
                            sender.input(BarSettingsInput::BottomEnabledToggled(enabled));
                            glib::Propagation::Proceed
                        } @bottom_enabled_handler,
                    }
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    gtk::Label {
                        add_css_class: "label-small",
                        set_halign: gtk::Align::Start,
                        set_label: "Minimum Height",
                        set_hexpand: true,
                    },

                    gtk::SpinButton {
                        set_range: (0.0, 500.0),
                        set_increments: (1.0, 10.0),
                        #[watch]
                        #[block_signal(bottom_min_handler)]
                        set_value: model.bottom_min_height as f64,
                        connect_value_changed[sender] => move |s| {
                            sender.input(BarSettingsInput::BottomMinHeightChanged(s.value() as i32));
                        } @bottom_min_handler,
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,

                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Auto-hide",
                            set_hexpand: true,
                        },

                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Hide the bar; it slides in when you move the pointer to its screen edge.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        // Auto-hide is the inverse of reveal_by_default.
                        #[watch]
                        #[block_signal(bottom_reveal_by_default_handler)]
                        set_active: !model.bottom_reveal_by_default,
                        connect_state_set[sender] => move |_, auto_hide| {
                            sender.input(BarSettingsInput::BottomRevealByDefaultChanged(!auto_hide));
                            glib::Propagation::Proceed
                        } @bottom_reveal_by_default_handler,
                    }
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    #[watch]
                    set_sensitive: !model.bottom_reveal_by_default,

                    gtk::Label {
                        add_css_class: "label-small",
                        set_halign: gtk::Align::Start,
                        set_label: "Auto-hide delay (ms)",
                        set_hexpand: true,
                    },

                    gtk::SpinButton {
                        set_range: (0.0, 5000.0),
                        set_increments: (50.0, 250.0),
                        #[watch]
                        #[block_signal(bottom_auto_hide_delay_handler)]
                        set_value: model.bottom_auto_hide_delay as f64,
                        connect_value_changed[sender] => move |s| {
                            sender.input(BarSettingsInput::BottomAutoHideDelayChanged(s.value() as i32));
                        } @bottom_auto_hide_delay_handler,
                    },
                },

                model.bottom_bar_start_controller.widget().clone() {},
                model.bottom_bar_center_controller.widget().clone() {},
                model.bottom_bar_end_controller.widget().clone() {},

            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let chips = FactoryVecDeque::builder()
            .launch(gtk::FlowBox::default())
            .forward(sender.input_sender(), |output| match output {
                MonitorChipOutput::Remove(index) => BarSettingsInput::RemoveMonitor(index),
            });

        let mut effects = EffectScope::new();

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let config = config_manager().config();
            let enabled = config.bars().frame().enable_frame().get();
            sender_clone.input(BarSettingsInput::EnableFrameChanged(enabled));
        });

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let config = config_manager().config();
            let islands = config.bars().islands().get();
            sender_clone.input(BarSettingsInput::IslandsChanged(islands));
        });

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let ms = config_manager().config().bars().slide_duration_ms().get();
            sender_clone.input(BarSettingsInput::SlideDurationChanged(ms as i32));
        });

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let config = config_manager().config();
            let monitors = config.bars().frame().monitor_filter().get();
            sender_clone.input(BarSettingsInput::SelectedMonitorsChanged(monitors));
        });

        let sender_clone = sender.clone();
        if let Some(display) = gdk::Display::default() {
            let monitors = display.monitors();
            let names: Vec<String> = (0..monitors.n_items())
                .filter_map(|i| monitors.item(i))
                .filter_map(|obj| obj.downcast::<gdk::Monitor>().ok())
                .filter_map(|m| m.connector().map(|c| c.to_string()))
                .collect();
            sender_clone.input(BarSettingsInput::AvailableMonitorsChanged(names));

            // Also listen for monitor changes
            let sender_clone2 = sender.clone();
            display.connect_notify(Some("monitors"), move |display, _| {
                let monitors = display.monitors();
                let names: Vec<String> = (0..monitors.n_items())
                    .filter_map(|i| monitors.item(i))
                    .filter_map(|obj| obj.downcast::<gdk::Monitor>().ok())
                    .filter_map(|m| m.connector().map(|c| c.to_string()))
                    .collect();
                sender_clone2.input(BarSettingsInput::AvailableMonitorsChanged(names));
            });
        }

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let config = config_manager().config();
            let value = config.bars().top_bar().minimum_height().get();
            sender_clone.input(BarSettingsInput::TopMinHeightEffect(value));
        });

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let config = config_manager().config();
            let value = config.bars().top_bar().reveal_by_default().get();
            sender_clone.input(BarSettingsInput::TopRevealByDefaultEffect(value));
        });

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let value = config_manager()
                .config()
                .theme()
                .attributes()
                .sizing()
                .bar_hover_strength()
                .get();
            sender_clone.input(BarSettingsInput::BarHoverStrengthEffect(value));
        });

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let config = config_manager().config();
            let value = config.bars().top_bar().left_widgets().get();
            sender_clone.input(BarSettingsInput::TopStartEffect(value));
        });

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let config = config_manager().config();
            let value = config.bars().top_bar().center_widgets().get();
            sender_clone.input(BarSettingsInput::TopCenterEffect(value));
        });

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let config = config_manager().config();
            let value = config.bars().top_bar().right_widgets().get();
            sender_clone.input(BarSettingsInput::TopEndEffect(value));
        });

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let config = config_manager().config();
            let value = config.bars().bottom_bar().minimum_height().get();
            sender_clone.input(BarSettingsInput::BottomMinHeightEffect(value));
        });

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let config = config_manager().config();
            let value = config.bars().bottom_bar().reveal_by_default().get();
            sender_clone.input(BarSettingsInput::BottomRevealByDefaultEffect(value));
        });

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let value = config_manager().config().bars().top_bar().enabled().get();
            sender_clone.input(BarSettingsInput::TopEnabledEffect(value));
        });
        let sender_clone = sender.clone();
        effects.push(move |_| {
            let value = config_manager()
                .config()
                .bars()
                .bottom_bar()
                .enabled()
                .get();
            sender_clone.input(BarSettingsInput::BottomEnabledEffect(value));
        });
        let sender_clone = sender.clone();
        effects.push(move |_| {
            let value = config_manager()
                .config()
                .bars()
                .top_bar()
                .auto_hide_delay_ms()
                .get();
            sender_clone.input(BarSettingsInput::TopAutoHideDelayEffect(value));
        });
        let sender_clone = sender.clone();
        effects.push(move |_| {
            let value = config_manager()
                .config()
                .bars()
                .bottom_bar()
                .auto_hide_delay_ms()
                .get();
            sender_clone.input(BarSettingsInput::BottomAutoHideDelayEffect(value));
        });

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let config = config_manager().config();
            let value = config.bars().bottom_bar().left_widgets().get();
            sender_clone.input(BarSettingsInput::BottomStartEffect(value));
        });

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let config = config_manager().config();
            let value = config.bars().bottom_bar().center_widgets().get();
            sender_clone.input(BarSettingsInput::BottomCenterEffect(value));
        });

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let config = config_manager().config();
            let value = config.bars().bottom_bar().right_widgets().get();
            sender_clone.input(BarSettingsInput::BottomEndEffect(value));
        });

        // NOTE: Vertical Left / Right bar surface effects are gone —
        // the corresponding 10 effect blocks would have pushed
        // `LeftMinWidthEffect` / `LeftRevealByDefaultEffect` /
        // `LeftStartEffect` / `LeftCenterEffect` / `LeftEndEffect`
        // (× 2 for Right). The settings UI no longer surfaces those
        // panels.

        let top_bar_start_controller = WidgetSectionModel::builder()
            .launch(WidgetSectionInit {
                bar_section: BarSection::Start,
                location: crate::bar_settings::bar_widget_factory::BarListLocation::TopStart,
                widgets: config_manager()
                    .config()
                    .bars()
                    .top_bar()
                    .left_widgets()
                    .get_untracked(),
            })
            .detach();

        let top_bar_center_controller = WidgetSectionModel::builder()
            .launch(WidgetSectionInit {
                bar_section: BarSection::Center,
                location: crate::bar_settings::bar_widget_factory::BarListLocation::TopCenter,
                widgets: config_manager()
                    .config()
                    .bars()
                    .top_bar()
                    .center_widgets()
                    .get_untracked(),
            })
            .detach();

        let top_bar_end_controller = WidgetSectionModel::builder()
            .launch(WidgetSectionInit {
                bar_section: BarSection::End,
                location: crate::bar_settings::bar_widget_factory::BarListLocation::TopEnd,
                widgets: config_manager()
                    .config()
                    .bars()
                    .top_bar()
                    .right_widgets()
                    .get_untracked(),
            })
            .detach();

        // Vertical Left / Right bar widget-section controllers are gone.

        let bottom_bar_start_controller = WidgetSectionModel::builder()
            .launch(WidgetSectionInit {
                bar_section: BarSection::Start,
                location: crate::bar_settings::bar_widget_factory::BarListLocation::BottomStart,
                widgets: config_manager()
                    .config()
                    .bars()
                    .bottom_bar()
                    .left_widgets()
                    .get_untracked(),
            })
            .detach();

        let bottom_bar_center_controller = WidgetSectionModel::builder()
            .launch(WidgetSectionInit {
                bar_section: BarSection::Center,
                location: crate::bar_settings::bar_widget_factory::BarListLocation::BottomCenter,
                widgets: config_manager()
                    .config()
                    .bars()
                    .bottom_bar()
                    .center_widgets()
                    .get_untracked(),
            })
            .detach();

        let bottom_bar_end_controller = WidgetSectionModel::builder()
            .launch(WidgetSectionInit {
                bar_section: BarSection::End,
                location: crate::bar_settings::bar_widget_factory::BarListLocation::BottomEnd,
                widgets: config_manager()
                    .config()
                    .bars()
                    .bottom_bar()
                    .right_widgets()
                    .get_untracked(),
            })
            .detach();

        let init_frame_fill = config_manager()
            .config()
            .theme()
            .attributes()
            .sizing()
            .frame_color()
            .get_untracked();
        let init_frame_border = config_manager()
            .config()
            .theme()
            .attributes()
            .sizing()
            .frame_border_color()
            .get_untracked();
        let init_separator = config_manager()
            .config()
            .theme()
            .attributes()
            .sizing()
            .separator_color()
            .get_untracked();
        let model = BarSettingsModel {
            enable_frame: false,
            islands: false,
            frame_color_custom: !init_frame_fill.trim().is_empty(),
            frame_color: gdk::RGBA::parse(init_frame_fill.trim())
                .unwrap_or_else(|_| gdk::RGBA::new(0.12, 0.12, 0.18, 1.0)),
            frame_border_color: gdk::RGBA::parse(init_frame_border.trim())
                .unwrap_or_else(|_| gdk::RGBA::new(0.19, 0.20, 0.27, 1.0)),
            separator_color_custom: !init_separator.trim().is_empty(),
            separator_color: gdk::RGBA::parse(init_separator.trim())
                .unwrap_or_else(|_| gdk::RGBA::new(0.27, 0.28, 0.35, 1.0)),
            slide_duration: config_manager()
                .config()
                .bars()
                .slide_duration_ms()
                .get_untracked() as i32,
            chips,
            available_monitors: Vec::new(),
            selected_monitors: Vec::new(),
            top_bar_start_controller,
            top_bar_center_controller,
            top_bar_end_controller,
            bottom_bar_start_controller,
            bottom_bar_center_controller,
            bottom_bar_end_controller,
            top_min_height: config_manager()
                .config()
                .bars()
                .top_bar()
                .minimum_height()
                .get_untracked(),
            bottom_min_height: config_manager()
                .config()
                .bars()
                .bottom_bar()
                .minimum_height()
                .get_untracked(),
            top_reveal_by_default: config_manager()
                .config()
                .bars()
                .top_bar()
                .reveal_by_default()
                .get_untracked(),
            bottom_reveal_by_default: config_manager()
                .config()
                .bars()
                .bottom_bar()
                .reveal_by_default()
                .get_untracked(),
            top_enabled: config_manager()
                .config()
                .bars()
                .top_bar()
                .enabled()
                .get_untracked(),
            bottom_enabled: config_manager()
                .config()
                .bars()
                .bottom_bar()
                .enabled()
                .get_untracked(),
            top_auto_hide_delay: config_manager()
                .config()
                .bars()
                .top_bar()
                .auto_hide_delay_ms()
                .get_untracked(),
            bottom_auto_hide_delay: config_manager()
                .config()
                .bars()
                .bottom_bar()
                .auto_hide_delay_ms()
                .get_untracked(),
            bar_hover_strength: config_manager()
                .config()
                .theme()
                .attributes()
                .sizing()
                .bar_hover_strength()
                .get_untracked(),
            top_min_height_debounce: None,
            bottom_min_height_debounce: None,
            top_auto_hide_delay_debounce: None,
            bottom_auto_hide_delay_debounce: None,
            _effects: effects,
        };

        let chip_box = model.chips.widget();

        let widgets = view_output!();

        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            BarSettingsInput::EnableFrameToggled(enabled) => {
                let config_manager = config_manager();
                config_manager.update_config(|config| {
                    config.bars.frame.enable_frame = enabled;
                });
            }
            BarSettingsInput::EnableFrameChanged(enable) => {
                self.enable_frame = enable;
            }
            BarSettingsInput::FrameColorCustomToggled(on) => {
                self.frame_color_custom = on;
                let (fill, border) = if on {
                    (
                        rgba_to_css(self.frame_color),
                        rgba_to_css(self.frame_border_color),
                    )
                } else {
                    (String::new(), String::new())
                };
                config_manager().update_config(|config| {
                    config.theme.attributes.sizing.frame_color = fill;
                    config.theme.attributes.sizing.frame_border_color = border;
                });
            }
            BarSettingsInput::FrameFillColorSet(rgba) => {
                self.frame_color = rgba;
                if self.frame_color_custom {
                    let hex = rgba_to_css(rgba);
                    config_manager().update_config(|config| {
                        config.theme.attributes.sizing.frame_color = hex;
                    });
                }
            }
            BarSettingsInput::FrameBorderColorSet(rgba) => {
                self.frame_border_color = rgba;
                if self.frame_color_custom {
                    let hex = rgba_to_css(rgba);
                    config_manager().update_config(|config| {
                        config.theme.attributes.sizing.frame_border_color = hex;
                    });
                }
            }
            BarSettingsInput::SeparatorColorCustomToggled(on) => {
                self.separator_color_custom = on;
                let val = if on {
                    rgba_to_css(self.separator_color)
                } else {
                    String::new()
                };
                config_manager().update_config(|config| {
                    config.theme.attributes.sizing.separator_color = val;
                });
            }
            BarSettingsInput::SeparatorColorSet(rgba) => {
                self.separator_color = rgba;
                if self.separator_color_custom {
                    let hex = rgba_to_css(rgba);
                    config_manager().update_config(|config| {
                        config.theme.attributes.sizing.separator_color = hex;
                    });
                }
            }
            BarSettingsInput::IslandsToggled(enabled) => {
                config_manager().update_config(|config| {
                    config.bars.islands = enabled;
                });
            }
            BarSettingsInput::IslandsChanged(enabled) => {
                self.islands = enabled;
            }
            BarSettingsInput::SlideDurationSet(ms) => {
                let ms = ms.max(0) as u32;
                config_manager().update_config(|config| {
                    config.bars.slide_duration_ms = ms;
                });
            }
            BarSettingsInput::SlideDurationChanged(ms) => {
                self.slide_duration = ms;
            }
            BarSettingsInput::AddMonitor(name) => {
                if !self.selected_monitors.contains(&name) {
                    self.selected_monitors.push(name.clone());
                    self.chips.guard().push_back(name);
                    config_manager().update_config(|config| {
                        config.bars.frame.monitor_filter = self.selected_monitors.clone();
                    });
                }
                self.rebuild_menu(widgets, &sender);
            }
            BarSettingsInput::RemoveMonitor(index) => {
                let idx = index.current_index();
                if idx < self.selected_monitors.len() {
                    self.selected_monitors.remove(idx);
                    self.chips.guard().remove(idx);
                    config_manager().update_config(|config| {
                        config.bars.frame.monitor_filter = self.selected_monitors.clone();
                    });
                }
                self.rebuild_menu(widgets, &sender);
            }
            BarSettingsInput::AvailableMonitorsChanged(monitors) => {
                self.available_monitors = monitors;
                self.rebuild_menu(widgets, &sender);
            }
            BarSettingsInput::SelectedMonitorsChanged(monitors) => {
                self.selected_monitors = monitors.clone();
                let mut guard = self.chips.guard();
                guard.clear();
                for name in monitors {
                    guard.push_back(name);
                }
                drop(guard);
                self.rebuild_menu(widgets, &sender);
            }
            BarSettingsInput::TopMinHeightChanged(min) => {
                // Stage the value immediately so the UI reflects the
                // current spin-button position, but defer persisting
                // it until the user stops scrubbing. See
                // `MIN_HEIGHT_DEBOUNCE_MS` for the rationale.
                self.top_min_height = min;
                if let Some(h) = self.top_min_height_debounce.take() {
                    h.abort();
                }
                let sender_clone = sender.clone();
                self.top_min_height_debounce = Some(glib::spawn_future_local(async move {
                    glib::timeout_future(std::time::Duration::from_millis(MIN_HEIGHT_DEBOUNCE_MS))
                        .await;
                    sender_clone.input(BarSettingsInput::CommitTopMinHeight);
                }));
            }
            BarSettingsInput::BottomMinHeightChanged(min) => {
                self.bottom_min_height = min;
                if let Some(h) = self.bottom_min_height_debounce.take() {
                    h.abort();
                }
                let sender_clone = sender.clone();
                self.bottom_min_height_debounce = Some(glib::spawn_future_local(async move {
                    glib::timeout_future(std::time::Duration::from_millis(MIN_HEIGHT_DEBOUNCE_MS))
                        .await;
                    sender_clone.input(BarSettingsInput::CommitBottomMinHeight);
                }));
            }
            BarSettingsInput::CommitTopMinHeight => {
                self.top_min_height_debounce = None;
                let min = self.top_min_height;
                config_manager().update_config(|config| {
                    config.bars.top_bar.minimum_height = min;
                });
            }
            BarSettingsInput::CommitBottomMinHeight => {
                self.bottom_min_height_debounce = None;
                let min = self.bottom_min_height;
                config_manager().update_config(|config| {
                    config.bars.bottom_bar.minimum_height = min;
                });
            }
            BarSettingsInput::TopRevealByDefaultChanged(reveal) => {
                self.top_reveal_by_default = reveal;
                config_manager().update_config(|config| {
                    config.bars.top_bar.reveal_by_default = reveal;
                });
            }
            BarSettingsInput::BottomRevealByDefaultChanged(reveal) => {
                self.bottom_reveal_by_default = reveal;
                config_manager().update_config(|config| {
                    config.bars.bottom_bar.reveal_by_default = reveal;
                });
            }
            BarSettingsInput::TopEnabledToggled(enabled) => {
                self.top_enabled = enabled;
                config_manager().update_config(move |config| {
                    config.bars.top_bar.enabled = enabled;
                });
            }
            BarSettingsInput::BottomEnabledToggled(enabled) => {
                self.bottom_enabled = enabled;
                config_manager().update_config(move |config| {
                    config.bars.bottom_bar.enabled = enabled;
                });
            }
            BarSettingsInput::TopAutoHideDelayChanged(ms) => {
                // Same debounce as min-height: stage now, persist once settled.
                self.top_auto_hide_delay = ms;
                if let Some(h) = self.top_auto_hide_delay_debounce.take() {
                    h.abort();
                }
                let sender_clone = sender.clone();
                self.top_auto_hide_delay_debounce = Some(glib::spawn_future_local(async move {
                    glib::timeout_future(std::time::Duration::from_millis(MIN_HEIGHT_DEBOUNCE_MS))
                        .await;
                    sender_clone.input(BarSettingsInput::CommitTopAutoHideDelay);
                }));
            }
            BarSettingsInput::BottomAutoHideDelayChanged(ms) => {
                self.bottom_auto_hide_delay = ms;
                if let Some(h) = self.bottom_auto_hide_delay_debounce.take() {
                    h.abort();
                }
                let sender_clone = sender.clone();
                self.bottom_auto_hide_delay_debounce = Some(glib::spawn_future_local(async move {
                    glib::timeout_future(std::time::Duration::from_millis(MIN_HEIGHT_DEBOUNCE_MS))
                        .await;
                    sender_clone.input(BarSettingsInput::CommitBottomAutoHideDelay);
                }));
            }
            BarSettingsInput::CommitTopAutoHideDelay => {
                self.top_auto_hide_delay_debounce = None;
                let ms = self.top_auto_hide_delay;
                config_manager().update_config(move |config| {
                    config.bars.top_bar.auto_hide_delay_ms = ms;
                });
            }
            BarSettingsInput::CommitBottomAutoHideDelay => {
                self.bottom_auto_hide_delay_debounce = None;
                let ms = self.bottom_auto_hide_delay;
                config_manager().update_config(move |config| {
                    config.bars.bottom_bar.auto_hide_delay_ms = ms;
                });
            }
            BarSettingsInput::BarHoverStrengthChanged(v) => {
                self.bar_hover_strength = v;
                config_manager().update_config(move |config| {
                    config.theme.attributes.sizing.bar_hover_strength = v;
                });
            }
            BarSettingsInput::BarHoverStrengthEffect(v) => {
                self.bar_hover_strength = v;
            }
            BarSettingsInput::TopStartEffect(widgets) => {
                self.top_bar_start_controller
                    .emit(WidgetSectionInput::SetWidgetsEffect(widgets));
            }
            BarSettingsInput::TopCenterEffect(widgets) => {
                self.top_bar_center_controller
                    .emit(WidgetSectionInput::SetWidgetsEffect(widgets));
            }
            BarSettingsInput::TopEndEffect(widgets) => {
                self.top_bar_end_controller
                    .emit(WidgetSectionInput::SetWidgetsEffect(widgets));
            }
            BarSettingsInput::BottomStartEffect(widgets) => {
                self.bottom_bar_start_controller
                    .emit(WidgetSectionInput::SetWidgetsEffect(widgets));
            }
            BarSettingsInput::BottomCenterEffect(widgets) => {
                self.bottom_bar_center_controller
                    .emit(WidgetSectionInput::SetWidgetsEffect(widgets));
            }
            BarSettingsInput::BottomEndEffect(widgets) => {
                self.bottom_bar_end_controller
                    .emit(WidgetSectionInput::SetWidgetsEffect(widgets));
            }
            BarSettingsInput::TopMinHeightEffect(height) => {
                self.top_min_height = height;
            }
            BarSettingsInput::BottomMinHeightEffect(height) => {
                self.bottom_min_height = height;
            }
            BarSettingsInput::TopRevealByDefaultEffect(reveal) => {
                self.top_reveal_by_default = reveal;
            }
            BarSettingsInput::BottomRevealByDefaultEffect(reveal) => {
                self.bottom_reveal_by_default = reveal;
            }
            BarSettingsInput::TopEnabledEffect(enabled) => {
                self.top_enabled = enabled;
            }
            BarSettingsInput::BottomEnabledEffect(enabled) => {
                self.bottom_enabled = enabled;
            }
            BarSettingsInput::TopAutoHideDelayEffect(ms) => {
                self.top_auto_hide_delay = ms;
            }
            BarSettingsInput::BottomAutoHideDelayEffect(ms) => {
                self.bottom_auto_hide_delay = ms;
            }
        }

        self.update_view(widgets, sender);
    }
}

impl BarSettingsModel {
    fn has_unselected_monitors(&self) -> bool {
        self.available_monitors
            .iter()
            .any(|m| !self.selected_monitors.contains(m))
    }

    fn rebuild_menu(&self, widgets: &<Self as Component>::Widgets, sender: &ComponentSender<Self>) {
        let menu = gio::Menu::new();
        let action_group = gio::SimpleActionGroup::new();

        for name in &self.available_monitors {
            if self.selected_monitors.contains(name) {
                continue;
            }

            let action_name = format!("add-{}", name.replace(' ', "-"));
            let action = gio::SimpleAction::new(&action_name, None);

            let sender = sender.input_sender().clone();
            let monitor_name = name.clone();
            action.connect_activate(move |_, _| {
                sender.emit(BarSettingsInput::AddMonitor(monitor_name.clone()));
            });

            action_group.add_action(&action);
            menu.append(Some(name.as_str()), Some(&format!("monitor.{action_name}")));
        }

        widgets
            .add_monitor_button
            .insert_action_group("monitor", Some(&action_group));
        widgets.add_monitor_button.set_menu_model(Some(&menu));
    }
}
