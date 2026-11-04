//! Scroller ribbon — a slim viewport indicator for margo's scroller layout.
//!
//! One small segment per column of the focused output's scroller strip,
//! in strip order: columns inside the viewport are lit, the focused one
//! carries the accent, columns scrolled off either edge are dim, and a
//! "‹N" / "N›" count names how many sit hidden on each side. Click a
//! segment to focus (and scroll to) that column (`focuswindowid`); scroll
//! over the pill to move focus one column left / right (`focusdir`).
//!
//! The geometry is exact, not estimated: margo publishes each column's
//! target rect relative to the work area in the output's `scroller`
//! object (`watch state`). The pill hides itself outside the scroller
//! layout and with fewer than two columns (nothing to orient by).
//!
//! Follows the focused output like the tag row and layout pill do (bar
//! widgets carry no output of their own).

use futures::StreamExt;
use mshell_margo_client::indicators::{ColumnPlace, Ribbon, ribbon};
use mshell_margo_client::state_json::RawScroller;
use mshell_services::margo_service;
use mshell_utils::hover_scroll::attach_hover_scroll;
use relm4::gtk::Orientation;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};

pub(crate) struct ScrollerRibbonModel {
    ribbon: Option<Ribbon>,
    orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum ScrollerRibbonInput {}

#[derive(Debug)]
pub(crate) enum ScrollerRibbonOutput {}

pub(crate) struct ScrollerRibbonInit {
    pub(crate) orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum ScrollerRibbonCommandOutput {
    View(Option<RawScroller>),
}

#[relm4::component(pub)]
impl Component for ScrollerRibbonModel {
    type CommandOutput = ScrollerRibbonCommandOutput;
    type Input = ScrollerRibbonInput;
    type Output = ScrollerRibbonOutput;
    type Init = ScrollerRibbonInit;

    view! {
        #[root]
        #[name = "root"]
        gtk::Box {
            set_css_classes: &["scroller-ribbon-bar-widget", "ok-button-surface", "ok-bar-widget"],
            set_hexpand: false,
            set_vexpand: false,
            set_halign: gtk::Align::Center,
            set_valign: gtk::Align::Center,

            #[name = "strip"]
            gtk::Box {
                set_orientation: model.orientation,
                set_spacing: 4,
                set_halign: gtk::Align::Center,
                set_valign: gtk::Align::Center,
            },
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        // `watch()` yields the current value first, then every change.
        sender.command(|out, shutdown| async move {
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            let mut stream = margo_service().scroller.watch();
            loop {
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    next = stream.next() => match next {
                        Some(view) => {
                            let _ = out.send(ScrollerRibbonCommandOutput::View(view));
                        }
                        None => break,
                    },
                }
            }
        });

        let model = ScrollerRibbonModel {
            ribbon: ribbon(margo_service().scroller.get().as_ref()),
            orientation: params.orientation,
        };
        let widgets = view_output!();

        // Scroll moves focus one column; the scroller follows focus.
        let _handles = attach_hover_scroll(&widgets.root, move |dx, dy, _hovered, _| {
            let step = if dy != 0.0 { dy } else { dx };
            if step < 0.0 {
                mshell_margo_client::ctl::dispatch_detached("focusdir", &["left"]);
            } else if step > 0.0 {
                mshell_margo_client::ctl::dispatch_detached("focusdir", &["right"]);
            }
        });

        render(&widgets, &model);
        ComponentParts { model, widgets }
    }

    fn update_cmd_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::CommandOutput,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            ScrollerRibbonCommandOutput::View(view) => {
                let next = ribbon(view.as_ref());
                if next == self.ribbon {
                    return;
                }
                self.ribbon = next;
                render(widgets, self);
            }
        }
    }
}

/// Rebuild the strip from the ribbon model. A strip holds a handful of
/// columns, so a full rebuild per change is cheaper than diffing.
fn render(widgets: &ScrollerRibbonModelWidgets, model: &ScrollerRibbonModel) {
    let strip = &widgets.strip;
    while let Some(child) = strip.first_child() {
        strip.remove(&child);
    }
    let Some(r) = &model.ribbon else {
        widgets.root.set_visible(false);
        return;
    };
    widgets.root.set_visible(true);
    widgets.root.set_tooltip_text(Some(&tooltip(r)));

    if r.hidden_left > 0 {
        strip.append(&count_label(&format!("‹{}", r.hidden_left)));
    }
    for seg in &r.segments {
        let mut classes = vec!["scroller-ribbon-segment"];
        match seg.place {
            ColumnPlace::Visible => classes.push("in-view"),
            ColumnPlace::Left | ColumnPlace::Right => {}
        }
        if seg.focused {
            classes.push("focused");
        }
        let button = gtk::Button::new();
        button.set_css_classes(&classes);
        button.set_valign(gtk::Align::Center);
        button.set_halign(gtk::Align::Center);
        let name = if seg.app_id.is_empty() {
            "Focus column".to_string()
        } else {
            format!("Focus {}", seg.app_id)
        };
        mshell_common::a11y::name(&button, &name);
        let id = seg.id.to_string();
        button.connect_clicked(move |_| {
            mshell_margo_client::ctl::dispatch_detached("focuswindowid", &[&id]);
        });
        strip.append(&button);
    }
    if r.hidden_right > 0 {
        strip.append(&count_label(&format!("{}›", r.hidden_right)));
    }
}

fn count_label(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.add_css_class("scroller-ribbon-count");
    label.set_valign(gtk::Align::Center);
    label
}

fn tooltip(r: &Ribbon) -> String {
    format!(
        "Scroller: {} columns · {} hidden left · {} hidden right\n\nClick a column: focus it  ·  Scroll: move focus",
        r.segments.len(),
        r.hidden_left,
        r.hidden_right
    )
}
