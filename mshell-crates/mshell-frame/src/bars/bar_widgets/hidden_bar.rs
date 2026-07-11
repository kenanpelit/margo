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
use mshell_common::hidden_bar::HiddenBarVerb;
use relm4::gtk::glib;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

pub(crate) struct HiddenBarInit {
    /// Drawer name. Empty for the bar's default drawer; set for a named
    /// drawer. A CLI verb with a target name only acts on the matching
    /// drawer (see [`HiddenBarInput::Ipc`]).
    pub name: String,
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
    name: String,
    orientation: gtk::Orientation,
    expanded: bool,
    pinned: bool,
    auto_expand: bool,
    hover_delay_ms: u32,
    auto_collapse: bool,
    collapse_delay_ms: u32,
    // Shared slots (app_launcher pattern): the one-shot clears its own slot
    // *before* dispatching, so cancel never calls `remove()` on a source
    // that already fired — that panics in glib and, from a main-loop
    // callback, aborts the whole shell.
    hover_source: Rc<Cell<Option<glib::SourceId>>>,
    collapse_source: Rc<Cell<Option<glib::SourceId>>>,
    // Keep the child controllers alive; dropping them would tear down the
    // hidden widgets. Not read directly.
    _child_controllers: Vec<Box<dyn GenericWidgetController>>,
}

#[derive(Debug)]
pub(crate) enum HiddenBarInput {
    ToggleExpand,
    TogglePin,
    // IPC verb (mshellctl hidden-bar …). The optional target name filters
    // which drawer reacts: `None` = every drawer, `Some(name)` = only the
    // drawer whose `name` matches.
    Ipc(HiddenBarVerb, Option<String>),
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
                // Canonical bar-pill classes (DESIGN.md §4): the transparent
                // surface + 14%-primary hover wash comes from `.ok-bar-widget`.
                #[watch]
                set_css_classes: if model.pinned {
                    &["ok-button-surface", "ok-bar-widget", "hidden-bar-trigger", "pinned"]
                } else {
                    &["ok-button-surface", "ok-bar-widget", "hidden-bar-trigger"]
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
            name: init.name,
            orientation: init.orientation,
            expanded: init.start_expanded,
            pinned: false,
            auto_expand: init.auto_expand,
            hover_delay_ms: init.hover_delay_ms,
            auto_collapse: init.auto_collapse,
            collapse_delay_ms: init.collapse_delay_ms,
            hover_source: Rc::new(Cell::new(None)),
            collapse_source: Rc::new(Cell::new(None)),
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
            HiddenBarInput::Ipc(verb, target) => {
                // A targeted verb only acts on the matching drawer; an
                // untargeted one (`mshellctl hidden-bar <verb>` with no name)
                // reaches every drawer.
                if let Some(name) = target.as_deref()
                    && name != self.name
                {
                    return;
                }
                match verb {
                    HiddenBarVerb::Toggle => {
                        self.cancel_hover();
                        if self.expanded {
                            self.collapse();
                        } else {
                            self.expand();
                        }
                    }
                    HiddenBarVerb::Expand => self.expand(),
                    HiddenBarVerb::Collapse => {
                        if !self.pinned {
                            self.collapse();
                        }
                    }
                    HiddenBarVerb::Pin => {
                        self.pinned = true;
                        self.cancel_collapse();
                        self.expand();
                    }
                    HiddenBarVerb::Unpin => {
                        self.pinned = false;
                    }
                }
            }
            HiddenBarInput::HoverEnter => {
                self.cancel_collapse();
                if self.auto_expand && !self.expanded {
                    // Keep an already-counting reveal timer instead of
                    // restarting the delay.
                    let pending = self.hover_source.take();
                    if pending.is_some() {
                        self.hover_source.set(pending);
                    } else {
                        let s = sender.clone();
                        let slot = self.hover_source.clone();
                        let id = glib::timeout_add_local_once(
                            Duration::from_millis(self.hover_delay_ms as u64),
                            move || {
                                // Clear the slot *before* dispatching so a
                                // cancel racing the fired timer never sees a
                                // dead SourceId; fallible send because the
                                // drawer can be torn down (bar rebuild) while
                                // the timer is pending — `input()` would abort.
                                slot.set(None);
                                let _ = s.input_sender().send(HiddenBarInput::RevealNow);
                            },
                        );
                        self.hover_source.set(Some(id));
                    }
                }
            }
            HiddenBarInput::HoverLeave => {
                self.cancel_hover();
                if self.expanded && self.auto_collapse && !self.pinned {
                    let pending = self.collapse_source.take();
                    if pending.is_some() {
                        self.collapse_source.set(pending);
                    } else {
                        let s = sender.clone();
                        let slot = self.collapse_source.clone();
                        let id = glib::timeout_add_local_once(
                            Duration::from_millis(self.collapse_delay_ms as u64),
                            move || {
                                slot.set(None);
                                let _ = s.input_sender().send(HiddenBarInput::CollapseNow);
                            },
                        );
                        self.collapse_source.set(Some(id));
                    }
                }
            }
            HiddenBarInput::RevealNow => {
                // The one-shot already cleared its own slot when it fired.
                self.expand();
            }
            HiddenBarInput::CollapseNow => {
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
        // `take()` yields `None` once the one-shot has fired (it clears the
        // slot before dispatching), so this never removes a dead source.
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
