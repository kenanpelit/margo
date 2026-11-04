//! USB Drives — bar pill for removable drives (udisks2).
//!
//! A USB glyph + the number of plugged-in removable drives. The icon
//! tints `--primary` while any volume is mounted (DESIGN.md §3). The pill
//! hides itself while nothing is plugged in (or udisks2 isn't running).
//! Left click opens the drive panel; right click safely removes every
//! drive (unmount, lock, power off / eject). State comes from the single
//! udisks watcher in [`crate::removable_drives`] — no polling.

use crate::removable_drives::{self, DrivesState};
use relm4::gtk::Orientation;
use relm4::gtk::prelude::{BoxExt, ButtonExt, GestureSingleExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

pub(crate) struct UsbModel {
    state: DrivesState,
    _orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum UsbInput {
    /// Left click — open the drive panel.
    Clicked,
    /// Right click — remove every drive safely.
    RemoveAll,
}

#[derive(Debug)]
pub(crate) enum UsbOutput {
    Clicked,
}

pub(crate) struct UsbInit {
    pub(crate) orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum UsbCommandOutput {
    State(DrivesState),
}

#[relm4::component(pub)]
impl Component for UsbModel {
    type CommandOutput = UsbCommandOutput;
    type Input = UsbInput;
    type Output = UsbOutput;
    type Init = UsbInit;

    view! {
        #[root]
        #[name = "root"]
        gtk::Box {
            #[watch]
            set_css_classes: if model.state.drives.iter().any(|d| d.any_mounted()) {
                &["usb-bar-widget", "active"]
            } else {
                &["usb-bar-widget"]
            },
            #[watch]
            set_visible: model.state.available && !model.state.drives.is_empty(),
            set_hexpand: model._orientation == Orientation::Vertical,
            set_vexpand: model._orientation == Orientation::Horizontal,
            set_halign: gtk::Align::Center,
            set_valign: gtk::Align::Center,
            set_has_tooltip: true,
            #[watch]
            set_tooltip_text: Some(&removable_drives::tooltip(&model.state.drives)),

            gtk::Button {
                set_css_classes: &["ok-button-surface", "ok-bar-widget"],
                set_hexpand: false,
                set_vexpand: false,
                connect_clicked[sender] => move |_| {
                    sender.input(UsbInput::Clicked);
                },

                gtk::Box {
                    set_orientation: Orientation::Horizontal,
                    set_spacing: 4,
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,

                    gtk::Image {
                        set_icon_name: Some("drive-removable-media-symbolic"),
                    },
                    gtk::Label {
                        add_css_class: "usb-bar-label",
                        #[watch]
                        set_label: &model.state.drives.len().to_string(),
                        #[watch]
                        set_visible: model.state.drives.len() > 1,
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
        let mut rx = removable_drives::subscribe();
        sender.command(|out, shutdown| async move {
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            loop {
                let state = rx.borrow_and_update().clone();
                let _ = out.send(UsbCommandOutput::State(state));
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    r = rx.changed() => if r.is_err() { break },
                }
            }
        });

        let model = UsbModel {
            state: removable_drives::current(),
            _orientation: params.orientation,
        };
        let widgets = view_output!();

        // Right-click → safe-remove every drive.
        let gesture = gtk::GestureClick::new();
        gesture.set_button(gtk::gdk::BUTTON_SECONDARY);
        let remove_sender = sender.clone();
        gesture.connect_pressed(move |_, _, _, _| {
            remove_sender.input(UsbInput::RemoveAll);
        });
        widgets.root.add_controller(gesture);

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            UsbInput::Clicked => {
                let _ = sender.output(UsbOutput::Clicked);
            }
            UsbInput::RemoveAll => {
                removable_drives::spawn_remove_all(self.state.drives.clone());
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
            UsbCommandOutput::State(state) => {
                self.state = state;
            }
        }
    }
}
