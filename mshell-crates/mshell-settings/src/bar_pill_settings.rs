//! Per-bar-pill info pages.
//!
//! Bar-only widgets (Active Window, Margo Tags, Battery, etc.)
//! don't have their own menu surface — they're just pills that
//! live in the bar. Their placement is driven by the Bar's
//! widget list (Bar → Top / Bottom widget arrays), not a
//! position/min-width pair like menu surfaces have.
//!
//! These pages surface the widget so the Settings sidebar is
//! complete, and point the user to the right place to edit its
//! placement. Widgets that DO have a menu surface (System
//! Updates, CPU Dashboard, Audio Route, …) are configured under
//! Widgets → Menus instead, not here.

use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    BarWidgetsStoreFields, BarsStoreFields, ConfigStoreFields, MargoTagsConfigStoreFields,
};
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::glib;
use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BarPillKind {
    ActiveWindow,
    AudioVisualizer,
    Countdown,
    DarkMode,
    KeyboardLayout,
    Keymode,
    // Lock has its own rich page (lock-screen background) — see
    // `lock_settings.rs` — so it's not a generic bar-pill info page.
    Logout,
    MargoTags,
    ScrollerRibbon,
    Scratchpads,
    Reboot,
    RecordingIndicator,
    Shutdown,
}

impl BarPillKind {
    pub(crate) fn display_name(self) -> &'static str {
        match self {
            Self::ActiveWindow => "Active Window",
            Self::AudioVisualizer => "Audio Visualizer",
            Self::Countdown => "Countdown",
            Self::DarkMode => "Dark Mode Toggle",
            Self::KeyboardLayout => "Keyboard Layout",
            Self::Keymode => "Keymode",
            Self::Logout => "Logout",
            Self::MargoTags => "Margo Tags",
            Self::ScrollerRibbon => "Scroller Ribbon",
            Self::Scratchpads => "Scratchpads",
            Self::Reboot => "Reboot",
            Self::RecordingIndicator => "Recording Indicator",
            Self::Shutdown => "Shutdown",
        }
    }

    /// One-line description for the page body. Cuts to the
    /// chase: what is this widget, and what makes it useful?
    fn description(self) -> &'static str {
        match self {
            Self::ActiveWindow => {
                "Shows the title of the currently focused window. Click to cycle through windows on the active tag."
            }
            Self::AudioVisualizer => {
                "Live audio spectrum — a strip of bars driven by the `cava` CLI (raw mode). Pulses with whatever is playing; sits as a flat resting strip on silence. Needs `cava` installed."
            }
            Self::Countdown => {
                "Shows the soonest enabled countdown from the Alarm Clock (a schedule/hourglass glyph + remaining time). Click opens the Alarm Clock menu on its Countdown tab. Hidden when no enabled, parseable target remains."
            }
            Self::DarkMode => {
                "One-click flip between Light and Dark matugen modes. Icon reflects the mode you'd switch *to*."
            }
            Self::KeyboardLayout => {
                "Shows the active xkb keyboard layout (e.g. US, TR). Click cycles to the next configured layout — set multiple via `xkb_rules_layout = tr,us` in config.conf."
            }
            Self::Keymode => {
                "Shows margo's active keymode (`setkeymode`, e.g. resize) and hides in the default mode. Click returns to the default keymode. Turn on Settings → Toasts → Keymode for a toast on every change."
            }
            Self::Logout => "Logs out of the session. Confirms with a dialog.",
            Self::MargoTags => {
                "1–9 tag pills with focus / occupied / urgent states. Click to switch tags, scroll to cycle. Optionally followed by the active tag's windows as clickable app icons."
            }
            Self::ScrollerRibbon => {
                "One small segment per column of the focused output's scroller strip: lit inside the viewport, accented for the focused column, dim when scrolled off, with ‹N / N› counts of the columns hidden on each side. Click a segment to focus that column; scroll over the pill to move focus left / right. Hidden outside the scroller layout."
            }
            Self::Scratchpads => {
                "One chip per scratchpad window (named or anonymous), labelled with its app. A scratchpad on screen is the filled accent capsule, a tucked-away one an accent label. Click a chip to show / hide it — the same toggle as its keybind. Hidden when there are no scratchpads."
            }
            Self::Reboot => "Reboots the system. Confirms with a dialog.",
            Self::RecordingIndicator => {
                "Lights up while a screen-recording is in progress. Click stops the recording."
            }
            Self::Shutdown => "Powers off the system. Confirms with a dialog.",
        }
    }
}

pub(crate) struct BarPillSettingsModel {
    kind: BarPillKind,
    /// `bars.widgets.margo_tags.window_icons` (Margo Tags page only).
    tag_window_icons: bool,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum BarPillSettingsInput {
    TagWindowIconsChanged(bool),
    TagWindowIconsEffect(bool),
}

#[derive(Debug)]
pub(crate) enum BarPillSettingsOutput {}

pub(crate) struct BarPillSettingsInit {
    pub(crate) kind: BarPillKind,
}

#[relm4::component(pub(crate))]
impl Component for BarPillSettingsModel {
    type CommandOutput = ();
    type Input = BarPillSettingsInput;
    type Output = BarPillSettingsOutput;
    type Init = BarPillSettingsInit;

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
                        set_icon_name: Some("view-grid-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "Bar pill",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "Per-pill placement and behaviour.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                gtk::Label {
                    add_css_class: "label-large-bold",
                    #[watch]
                    set_label: model.kind.display_name(),
                    set_halign: gtk::Align::Start,
                },

                gtk::Label {
                    add_css_class: "label-medium",
                    #[watch]
                    set_label: model.kind.description(),
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                },

                // Margo Tags' one behaviour switch (other pills have none).
                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,
                    set_visible: model.kind == BarPillKind::MargoTags,

                    gtk::Box {
                        add_css_class: "action-row",
                        set_widget_name: "setting:/bars/widgets/margo_tags/window_icons",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_halign: gtk::Align::Start,
                                set_label: "Window icons",
                                set_hexpand: true,
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "Show the active tag's windows as app icons after the tags. Click an icon to focus that window.",
                                set_hexpand: true,
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(tag_window_icons_handler)]
                            set_active: model.tag_window_icons,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(BarPillSettingsInput::TagWindowIconsChanged(v));
                                glib::Propagation::Proceed
                            } @tag_window_icons_handler,
                        },
                    },
                },

                gtk::Separator {},

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Placement",
                    set_halign: gtk::Align::Start,
                },

                gtk::Label {
                    add_css_class: "label-small",
                    set_label: "Bar pills don't have their own menu surface — they live in the bar itself. To change which side of the bar this widget appears on (start / center / end) or to add / remove it, head to Bar → Top or Bottom bar widget lists.",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                },
            }
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let mut effects = EffectScope::new();
        if params.kind == BarPillKind::MargoTags {
            let s = sender.clone();
            effects.push(move |_| {
                let v = config_manager()
                    .config()
                    .bars()
                    .widgets()
                    .margo_tags()
                    .window_icons()
                    .get();
                s.input(BarPillSettingsInput::TagWindowIconsEffect(v));
            });
        }
        let model = BarPillSettingsModel {
            kind: params.kind,
            tag_window_icons: config_manager()
                .config()
                .bars()
                .widgets()
                .margo_tags()
                .window_icons()
                .get_untracked(),
            _effects: effects,
        };
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            BarPillSettingsInput::TagWindowIconsChanged(v) => {
                config_manager().update_config(|c| c.bars.widgets.margo_tags.window_icons = v);
            }
            BarPillSettingsInput::TagWindowIconsEffect(v) => self.tag_window_icons = v,
        }
    }
}
