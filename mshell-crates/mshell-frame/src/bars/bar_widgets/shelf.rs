//! Shelf — bar pill for the file shelf ([`crate::shelf`]).
//!
//! A tray glyph and, once something is on the shelf, how many files it
//! holds (tinted `--primary`, DESIGN.md §3). Files dropped on the pill go
//! straight onto the shelf without opening anything; a click opens the
//! Shelf menu, where they can be dragged out into other apps.

use crate::menus::menu_widgets::shelf::dnd;
use crate::shelf::{self, ShelfState};
use relm4::gtk::Orientation;
use relm4::gtk::prelude::{BoxExt, ButtonExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, RelmWidgetExt, gtk};

pub(crate) struct ShelfModel {
    count: usize,
    _orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum ShelfInput {
    Clicked,
}

#[derive(Debug)]
pub(crate) enum ShelfOutput {
    Clicked,
}

pub(crate) struct ShelfInit {
    pub(crate) orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum ShelfCommandOutput {
    State(ShelfState),
}

/// The pill tooltip.
fn tooltip(count: usize) -> String {
    let head = match count {
        0 => "Shelf: empty".to_string(),
        1 => "Shelf: 1 file".to_string(),
        n => format!("Shelf: {n} files"),
    };
    format!("{head}\n\nDrop files here to hold them · click: open the shelf")
}

#[relm4::component(pub)]
impl Component for ShelfModel {
    type CommandOutput = ShelfCommandOutput;
    type Input = ShelfInput;
    type Output = ShelfOutput;
    type Init = ShelfInit;

    view! {
        #[root]
        gtk::Box {
            set_css_classes: &["shelf-bar-widget", "ok-button-surface", "ok-bar-widget"],
            #[watch]
            set_class_active: ("active", model.count > 0),
            set_hexpand: model._orientation == Orientation::Vertical,
            set_vexpand: model._orientation == Orientation::Horizontal,
            set_halign: gtk::Align::Center,
            set_valign: gtk::Align::Center,
            set_has_tooltip: true,
            #[watch]
            set_tooltip_text: Some(&tooltip(model.count)),

            gtk::Button {
                set_css_classes: &["ok-button-flat"],
                set_hexpand: false,
                set_vexpand: false,
                connect_clicked[sender] => move |_| {
                    sender.input(ShelfInput::Clicked);
                },

                gtk::Box {
                    set_orientation: Orientation::Horizontal,
                    set_spacing: 4,
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,

                    gtk::Image {
                        set_icon_name: Some("folder-download-symbolic"),
                    },
                    gtk::Label {
                        add_css_class: "shelf-bar-label",
                        #[watch]
                        set_label: &model.count.to_string(),
                        #[watch]
                        set_visible: model.count > 0,
                    },
                },
            }
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let mut rx = shelf::subscribe();
        sender.command(|out, shutdown| async move {
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            loop {
                let state = rx.borrow_and_update().clone();
                let _ = out.send(ShelfCommandOutput::State(state));
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    r = rx.changed() => if r.is_err() { break },
                }
            }
        });
        let model = ShelfModel {
            count: 0,
            _orientation: params.orientation,
        };
        let widgets = view_output!();
        dnd::attach_drop_target(&root);
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            ShelfInput::Clicked => {
                let _ = sender.output(ShelfOutput::Clicked);
            }
        }
    }

    fn update_cmd(
        &mut self,
        message: Self::CommandOutput,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            ShelfCommandOutput::State(s) => self.count = s.items.len(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tooltip_counts_files() {
        assert!(tooltip(0).starts_with("Shelf: empty"));
        assert!(tooltip(1).starts_with("Shelf: 1 file\n"));
        assert!(tooltip(3).starts_with("Shelf: 3 files"));
    }
}
