//! Hidden Bar — a collapsible drawer pill (native port of the DMS
//! hidden-bar plugin).
//!
//! Renders the widgets listed in the bar's `hidden_widgets` config inside a
//! [`gtk::Revealer`] behind a trigger button. Hover (when `auto_expand`) or
//! left-click reveals the drawer; right-click pins it open; the pointer
//! leaving collapses it again after `collapse_delay_ms` (unless pinned).
//!
//! The child widget controllers are built by `bar.rs` (via the shared
//! `build_widget` builder) and handed in through [`HiddenBarInit`]; this model
//! keeps them alive for its lifetime and parents their roots in the revealer.

use mshell_common::dynamic_box::generic_widget_controller::GenericWidgetController;
use relm4::gtk::glib;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::Duration;

pub(crate) struct HiddenBarInit {
    pub orientation: gtk::Orientation,
    /// Root widgets of the hidden children, parented into the revealer.
    pub children: Vec<gtk::Widget>,
    /// The owning controllers — kept alive by the model.
    pub child_controllers: Vec<Box<dyn GenericWidgetController>>,
    pub start_expanded: bool,
    pub auto_expand: bool,
    pub hover_delay_ms: u32,
    pub auto_collapse: bool,
    pub collapse_delay_ms: u32,
}

pub(crate) struct HiddenBarModel {
    orientation: gtk::Orientation,
    expanded: bool,
    pinned: bool,
    auto_expand: bool,
    hover_delay_ms: u32,
    auto_collapse: bool,
    collapse_delay_ms: u32,
    hover_source: Option<glib::SourceId>,
    collapse_source: Option<glib::SourceId>,
    // Keep the child controllers alive; dropping them would tear down the
    // hidden widgets. Not read directly.
    _child_controllers: Vec<Box<dyn GenericWidgetController>>,
}

#[derive(Debug)]
pub(crate) enum HiddenBarInput {
    ToggleExpand,
    TogglePin,
    // IPC verbs (mshellctl hidden-bar …)
    Expand,
    Collapse,
    Pin,
    Unpin,
    // Internal hover / timer events
    HoverEnter,
    HoverLeave,
    RevealNow,
    CollapseNow,
}

#[derive(Debug)]
pub(crate) enum HiddenBarOutput {}

#[relm4::component(pub)]
impl Component for HiddenBarModel {
    type CommandOutput = ();
    type Input = HiddenBarInput;
    type Output = HiddenBarOutput;
    type Init = HiddenBarInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "hidden-bar",
            set_orientation: model.orientation,
            set_spacing: 4,

            add_controller = gtk::EventControllerMotion {
                connect_enter[sender] => move |_, _, _| {
                    sender.input(HiddenBarInput::HoverEnter);
                },
                connect_leave[sender] => move |_| {
                    sender.input(HiddenBarInput::HoverLeave);
                },
            },

            #[name = "trigger"]
            gtk::Button {
                add_css_class: "hidden-bar-trigger",
                add_css_class: "bar-button",
                #[watch]
                set_css_classes: if model.pinned {
                    &["hidden-bar-trigger", "bar-button", "pinned"]
                } else {
                    &["hidden-bar-trigger", "bar-button"]
                },
                #[watch]
                set_icon_name: if model.expanded {
                    "pan-end-symbolic"
                } else {
                    "view-more-horizontal-symbolic"
                },
                #[watch]
                set_tooltip_text: Some(if model.pinned {
                    "Hidden Bar (pinned) — left-click toggle · right-click unpin"
                } else {
                    "Hidden Bar — left-click toggle · right-click pin"
                }),
                connect_clicked[sender] => move |_| {
                    sender.input(HiddenBarInput::ToggleExpand);
                },

                add_controller = gtk::GestureClick {
                    set_button: 3,
                    connect_pressed[sender] => move |gesture, _, _, _| {
                        gesture.set_state(gtk::EventSequenceState::Claimed);
                        sender.input(HiddenBarInput::TogglePin);
                    },
                },
            },

            #[name = "revealer"]
            gtk::Revealer {
                #[watch]
                set_reveal_child: model.expanded,

                #[name = "child_box"]
                gtk::Box {
                    add_css_class: "hidden-bar-items",
                    set_orientation: model.orientation,
                    set_spacing: 4,
                },
            },
        }
    }

    fn init(
        init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = HiddenBarModel {
            orientation: init.orientation,
            expanded: init.start_expanded,
            pinned: false,
            auto_expand: init.auto_expand,
            hover_delay_ms: init.hover_delay_ms,
            auto_collapse: init.auto_collapse,
            collapse_delay_ms: init.collapse_delay_ms,
            hover_source: None,
            collapse_source: None,
            _child_controllers: init.child_controllers,
        };

        let widgets = view_output!();

        // Slide along the bar's axis.
        widgets
            .revealer
            .set_transition_type(if init.orientation == gtk::Orientation::Vertical {
                gtk::RevealerTransitionType::SlideUp
            } else {
                gtk::RevealerTransitionType::SlideRight
            });

        // Parent the hidden children inside the revealer.
        for child in &init.children {
            widgets.child_box.append(child);
        }

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            HiddenBarInput::ToggleExpand => {
                self.cancel_hover();
                if self.expanded {
                    self.collapse();
                } else {
                    self.expand();
                }
            }
            HiddenBarInput::TogglePin => {
                self.pinned = !self.pinned;
                if self.pinned {
                    self.cancel_collapse();
                    self.expand();
                }
            }
            HiddenBarInput::Expand => self.expand(),
            HiddenBarInput::Collapse => {
                if !self.pinned {
                    self.collapse();
                }
            }
            HiddenBarInput::Pin => {
                self.pinned = true;
                self.cancel_collapse();
                self.expand();
            }
            HiddenBarInput::Unpin => {
                self.pinned = false;
            }
            HiddenBarInput::HoverEnter => {
                self.cancel_collapse();
                if self.auto_expand && !self.expanded && self.hover_source.is_none() {
                    let s = sender.clone();
                    let id = glib::timeout_add_local_once(
                        Duration::from_millis(self.hover_delay_ms as u64),
                        move || s.input(HiddenBarInput::RevealNow),
                    );
                    self.hover_source = Some(id);
                }
            }
            HiddenBarInput::HoverLeave => {
                self.cancel_hover();
                if self.expanded
                    && self.auto_collapse
                    && !self.pinned
                    && self.collapse_source.is_none()
                {
                    let s = sender.clone();
                    let id = glib::timeout_add_local_once(
                        Duration::from_millis(self.collapse_delay_ms as u64),
                        move || s.input(HiddenBarInput::CollapseNow),
                    );
                    self.collapse_source = Some(id);
                }
            }
            HiddenBarInput::RevealNow => {
                self.hover_source = None;
                self.expand();
            }
            HiddenBarInput::CollapseNow => {
                self.collapse_source = None;
                if !self.pinned {
                    self.collapse();
                }
            }
        }
    }
}

impl HiddenBarModel {
    fn expand(&mut self) {
        self.expanded = true;
    }

    fn collapse(&mut self) {
        self.expanded = false;
    }

    fn cancel_hover(&mut self) {
        if let Some(s) = self.hover_source.take() {
            s.remove();
        }
    }

    fn cancel_collapse(&mut self) {
        if let Some(s) = self.collapse_source.take() {
            s.remove();
        }
    }
}
