//! Scratchpad chips — one chip per scratchpad window, click to toggle it.
//!
//! Lists every scratchpad (named or anonymous, shown or tucked away) as a
//! short app-name chip in the same capsule language as the tag row: a
//! scratchpad on screen is the filled accent capsule, a hidden one an
//! accent-tinted label. Click a chip to show / hide that scratchpad
//! (`togglescratchpadid <id>`, the same show path as its keybind). The
//! whole pill hides when there are no scratchpads. Port of the
//! special-workspaces plugin onto margo scratchpads.
//!
//! Source: `margo_service().scratchpads`, derived from the `scratchpad` /
//! `scratchpad_shown` client fields margo pushes in `watch state`.

use futures::StreamExt;
use mshell_margo_client::indicators::{ScratchpadChip, chip_label};
use mshell_services::margo_service;
use relm4::gtk::Orientation;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};

pub(crate) struct ScratchpadsModel {
    chips: Vec<ScratchpadChip>,
    orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum ScratchpadsInput {}

#[derive(Debug)]
pub(crate) enum ScratchpadsOutput {}

pub(crate) struct ScratchpadsInit {
    pub(crate) orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum ScratchpadsCommandOutput {
    Chips(Vec<ScratchpadChip>),
}

#[relm4::component(pub)]
impl Component for ScratchpadsModel {
    type CommandOutput = ScratchpadsCommandOutput;
    type Input = ScratchpadsInput;
    type Output = ScratchpadsOutput;
    type Init = ScratchpadsInit;

    view! {
        #[root]
        #[name = "root"]
        gtk::Box {
            add_css_class: "scratchpads-bar-widget",
            set_orientation: model.orientation,
            set_hexpand: model.orientation == Orientation::Vertical,
            set_vexpand: model.orientation == Orientation::Horizontal,
            set_halign: gtk::Align::Center,
            set_valign: gtk::Align::Center,
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
            let mut stream = margo_service().scratchpads.watch();
            loop {
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    next = stream.next() => match next {
                        Some(chips) => {
                            let _ = out.send(ScratchpadsCommandOutput::Chips(chips));
                        }
                        None => break,
                    },
                }
            }
        });

        let model = ScratchpadsModel {
            chips: margo_service().scratchpads.get(),
            orientation: params.orientation,
        };
        let widgets = view_output!();
        render(&widgets.root, &model.chips);
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
            ScratchpadsCommandOutput::Chips(chips) => {
                if chips == self.chips {
                    return;
                }
                self.chips = chips;
                render(&widgets.root, &self.chips);
            }
        }
    }
}

/// Rebuild the chip row. A handful of scratchpads at most, so a full
/// rebuild per change is simpler than diffing and costs nothing.
fn render(root: &gtk::Box, chips: &[ScratchpadChip]) {
    while let Some(child) = root.first_child() {
        root.remove(&child);
    }
    root.set_visible(!chips.is_empty());
    for chip in chips {
        // The tag pill's classes, so a chip reads exactly like a tag:
        // occupied tint always (a scratchpad is never empty), the filled
        // accent capsule while it is on screen.
        let mut classes = vec!["margo-tag", "tag-has-windows", "scratchpad-chip"];
        if chip.shown {
            classes.push("tag-active");
        }
        let label = gtk::Label::new(Some(&chip_label(&chip.app_id, &chip.title)));
        label.add_css_class("margo-tag-label");
        label.set_halign(gtk::Align::Center);
        label.set_valign(gtk::Align::Center);
        let button = gtk::Button::new();
        button.set_css_classes(&classes);
        button.set_child(Some(&label));
        let state = if chip.shown { "shown" } else { "hidden" };
        let what = if chip.app_id.is_empty() {
            chip.title.as_str()
        } else {
            chip.app_id.as_str()
        };
        button.set_tooltip_text(Some(&format!(
            "Scratchpad: {what} ({state})\n{}\n\nClick: {}",
            chip.title,
            if chip.shown { "hide" } else { "show" }
        )));
        mshell_common::a11y::name(&button, &format!("Toggle scratchpad {what}"));
        let id = chip.id.to_string();
        button.connect_clicked(move |_| {
            mshell_margo_client::ctl::dispatch_detached("togglescratchpadid", &[&id]);
        });
        root.append(&button);
    }
}
