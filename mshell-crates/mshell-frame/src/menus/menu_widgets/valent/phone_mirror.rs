//! "Screen mirroring" card at the bottom of the Valent menu: adb devices
//! with a Mirror / Stop button each (scrcpy), wireless-debugging pair and
//! connect forms, and a screen-off switch. Valent itself has no screen
//! mirroring, so this talks to adb directly — see [`crate::valent::scrcpy`].
//! Built like the rest of the Valent menu (`valent-card`, label tiers,
//! `ok-button-*`).

use crate::valent::scrcpy::{self, AdbDevice, AdbState};
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, ValentStoreFields};
use reactive_graph::traits::GetUntracked;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};

pub(crate) struct PhoneMirrorModel {
    /// `adb` and `scrcpy` are both on PATH.
    tools: bool,
    devices: Option<Result<Vec<AdbDevice>, String>>,
    /// Last pair / connect / mirror outcome shown under the forms.
    status: Option<Result<String, String>>,
    list: gtk::Box,
}

#[derive(Debug)]
pub(crate) enum PhoneMirrorInput {
    Refresh,
    Mirror(String),
    Stop(String),
    Disconnect(String),
    Pair(String, String),
    Connect(String),
    ScreenOffChanged(bool),
}

#[derive(Debug)]
pub(crate) enum PhoneMirrorCommandOutput {
    Devices(Result<Vec<AdbDevice>, String>),
    /// An adb action finished; re-list devices afterwards.
    Action(Result<String, String>),
}

#[relm4::component(pub(crate))]
impl Component for PhoneMirrorModel {
    type CommandOutput = PhoneMirrorCommandOutput;
    type Input = PhoneMirrorInput;
    type Output = ();
    type Init = ();

    view! {
        #[root]
        gtk::Box {
            add_css_class: "valent-card",
            add_css_class: "valent-mirror",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 10,

            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 8,
                gtk::Label {
                    add_css_class: "label-medium-bold",
                    set_label: "Screen mirroring",
                    set_halign: gtk::Align::Start,
                    set_hexpand: true,
                },
                #[name = "refresh_button"]
                gtk::Button {
                    add_css_class: "ok-button-surface",
                    set_valign: gtk::Align::Center,
                    set_tooltip_text: Some("Refresh devices"),
                    set_icon_name: "view-refresh-symbolic",
                    #[watch]
                    set_visible: model.tools,
                    connect_clicked[sender] => move |_| sender.input(PhoneMirrorInput::Refresh),
                },
            },

            #[local_ref]
            list -> gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 6,
            },

            gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 6,
                #[watch]
                set_visible: model.tools,

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 8,
                    gtk::Label {
                        add_css_class: "label-small",
                        set_label: "Keep the phone screen off",
                        set_halign: gtk::Align::Start,
                        set_hexpand: true,
                    },
                    #[name = "screen_off_switch"]
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        connect_active_notify[sender] => move |s| {
                            sender.input(PhoneMirrorInput::ScreenOffChanged(s.is_active()));
                        },
                    },
                },

                gtk::Label {
                    add_css_class: "label-small",
                    add_css_class: "valent-mirror-caption",
                    set_label: "Wireless debugging — pair once with the code from the phone's Developer options, then connect.",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 8,
                    #[name = "pair_addr"]
                    gtk::Entry {
                        set_hexpand: true,
                        set_placeholder_text: Some("IP:pairing port"),
                    },
                    #[name = "pair_code"]
                    gtk::Entry {
                        set_width_chars: 7,
                        set_max_length: 6,
                        set_input_purpose: gtk::InputPurpose::Digits,
                        set_placeholder_text: Some("Code"),
                    },
                    #[name = "pair_button"]
                    gtk::Button {
                        add_css_class: "ok-button-surface",
                        set_label: "Pair",
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 8,
                    #[name = "connect_addr"]
                    gtk::Entry {
                        set_hexpand: true,
                        set_placeholder_text: Some("IP:port"),
                    },
                    #[name = "connect_button"]
                    gtk::Button {
                        add_css_class: "ok-button-surface",
                        set_label: "Connect",
                    },
                },
            },

            gtk::Label {
                add_css_class: "label-small",
                add_css_class: "valent-mirror-status",
                set_halign: gtk::Align::Start,
                set_xalign: 0.0,
                set_wrap: true,
                #[watch]
                set_visible: model.status.is_some(),
                #[watch]
                set_label: match &model.status {
                    Some(Ok(s)) | Some(Err(s)) => s.as_str(),
                    None => "",
                },
                #[watch]
                set_css_classes: if matches!(model.status, Some(Err(_))) {
                    &["label-small", "valent-mirror-status", "danger"]
                } else {
                    &["label-small", "valent-mirror-status"]
                },
            },
        }
    }

    fn init(_: (), root: Self::Root, sender: ComponentSender<Self>) -> ComponentParts<Self> {
        let list = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let model = PhoneMirrorModel {
            tools: margo_fs::path::on_path("adb") && margo_fs::path::on_path("scrcpy"),
            devices: None,
            status: None,
            list: list.clone(),
        };
        let widgets = view_output!();
        mshell_common::a11y::name(&widgets.refresh_button, "Refresh devices");
        widgets.screen_off_switch.set_active(
            config_manager()
                .config()
                .valent()
                .scrcpy_screen_off()
                .get_untracked(),
        );

        let pair = {
            let (addr, code, s) = (
                widgets.pair_addr.clone(),
                widgets.pair_code.clone(),
                sender.clone(),
            );
            move || {
                s.input(PhoneMirrorInput::Pair(
                    addr.text().into(),
                    code.text().into(),
                ))
            }
        };
        {
            let pair = pair.clone();
            widgets.pair_button.connect_clicked(move |_| pair());
        }
        widgets.pair_code.connect_activate(move |_| pair());
        let connect = {
            let (addr, s) = (widgets.connect_addr.clone(), sender.clone());
            move || s.input(PhoneMirrorInput::Connect(addr.text().into()))
        };
        {
            let connect = connect.clone();
            widgets.connect_button.connect_clicked(move |_| connect());
        }
        widgets.connect_addr.connect_activate(move |_| connect());

        // List devices whenever the menu (and so this card) is shown.
        {
            let sender = sender.clone();
            root.connect_map(move |_| sender.input(PhoneMirrorInput::Refresh));
        }
        model.render(&sender);
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        if !self.tools {
            return;
        }
        match message {
            PhoneMirrorInput::Refresh => {
                sender.oneshot_command(async {
                    PhoneMirrorCommandOutput::Devices(scrcpy::devices().await)
                });
            }
            PhoneMirrorInput::Mirror(serial) => {
                let cfg = config_manager().config().valent();
                let screen_off = cfg.clone().scrcpy_screen_off().get_untracked();
                let extra = cfg.scrcpy_args().get_untracked();
                self.status = scrcpy::start_mirror(&serial, screen_off, &extra)
                    .err()
                    .map(Err);
                self.render(&sender);
            }
            PhoneMirrorInput::Stop(serial) => {
                scrcpy::stop_mirror(&serial);
                self.render(&sender);
            }
            PhoneMirrorInput::Disconnect(serial) => {
                scrcpy::stop_mirror(&serial);
                sender.oneshot_command(async move {
                    PhoneMirrorCommandOutput::Action(scrcpy::disconnect(&serial).await)
                });
            }
            PhoneMirrorInput::Pair(addr, code) => {
                if !scrcpy::valid_host_port(&addr) {
                    self.status = Some(Err("Enter the pairing address as IP:port".into()));
                } else if !scrcpy::valid_pair_code(&code) {
                    self.status = Some(Err("The pairing code has 6 digits".into()));
                } else {
                    self.status = Some(Ok("Pairing…".into()));
                    sender.oneshot_command(async move {
                        PhoneMirrorCommandOutput::Action(scrcpy::pair(&addr, &code).await)
                    });
                }
            }
            PhoneMirrorInput::Connect(addr) => {
                if scrcpy::valid_host_port(&addr) {
                    self.status = Some(Ok("Connecting…".into()));
                    sender.oneshot_command(async move {
                        PhoneMirrorCommandOutput::Action(scrcpy::connect(&addr).await)
                    });
                } else {
                    self.status = Some(Err("Enter the address as IP:port".into()));
                }
            }
            PhoneMirrorInput::ScreenOffChanged(v) => {
                config_manager().update_config(move |c| c.valent.scrcpy_screen_off = v);
            }
        }
    }

    fn update_cmd(
        &mut self,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            PhoneMirrorCommandOutput::Devices(d) => self.devices = Some(d),
            PhoneMirrorCommandOutput::Action(r) => {
                self.status = Some(r);
                sender.input(PhoneMirrorInput::Refresh);
            }
        }
        self.render(&sender);
    }
}

impl PhoneMirrorModel {
    fn render(&self, sender: &ComponentSender<Self>) {
        let list = &self.list;
        while let Some(child) = list.first_child() {
            list.remove(&child);
        }
        if !self.tools {
            hint(
                list,
                "Install scrcpy and android-tools (adb) to mirror and control your phone.",
            );
            return;
        }
        match &self.devices {
            None => hint(list, "Looking for devices…"),
            Some(Err(e)) => hint(list, e),
            Some(Ok(d)) if d.is_empty() => hint(
                list,
                "No phone found. Plug it in with USB debugging on, or pair over Wi-Fi below.",
            ),
            Some(Ok(d)) => {
                let live = scrcpy::mirroring();
                for dev in d {
                    list.append(&device_row(dev, live.contains(&dev.serial), sender));
                }
            }
        }
    }
}

fn hint(list: &gtk::Box, text: &str) {
    let l = gtk::Label::new(Some(text));
    l.add_css_class("label-small");
    l.set_halign(gtk::Align::Start);
    l.set_xalign(0.0);
    l.set_wrap(true);
    list.append(&l);
}

fn device_row(dev: &AdbDevice, live: bool, sender: &ComponentSender<PhoneMirrorModel>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.add_css_class("valent-mirror-row");

    let icon = gtk::Image::from_icon_name(if dev.wireless() {
        "network-wireless-symbolic"
    } else {
        "phone-symbolic"
    });
    icon.set_valign(gtk::Align::Center);
    row.append(&icon);

    let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
    text.set_hexpand(true);
    let name = gtk::Label::new(Some(dev.display_name()));
    name.add_css_class("label-medium-bold");
    name.set_xalign(0.0);
    text.append(&name);
    let sub = gtk::Label::new(Some(&dev.status_text()));
    sub.add_css_class("label-small");
    sub.set_xalign(0.0);
    sub.set_wrap(true);
    text.append(&sub);
    row.append(&text);

    if dev.state == AdbState::Device {
        let btn = gtk::Button::with_label(if live { "Stop" } else { "Mirror" });
        btn.add_css_class(if live {
            "ok-button-surface"
        } else {
            "ok-button-primary"
        });
        btn.set_valign(gtk::Align::Center);
        let serial = dev.serial.clone();
        let s = sender.clone();
        btn.connect_clicked(move |_| {
            s.input(if live {
                PhoneMirrorInput::Stop(serial.clone())
            } else {
                PhoneMirrorInput::Mirror(serial.clone())
            });
        });
        row.append(&btn);
    }
    if dev.wireless() {
        let btn = gtk::Button::from_icon_name("window-close-symbolic");
        btn.add_css_class("ok-button-surface");
        btn.set_valign(gtk::Align::Center);
        btn.set_tooltip_text(Some("Disconnect"));
        mshell_common::a11y::name(&btn, "Disconnect");
        let serial = dev.serial.clone();
        let s = sender.clone();
        btn.connect_clicked(move |_| s.input(PhoneMirrorInput::Disconnect(serial.clone())));
        row.append(&btn);
    }
    row
}
