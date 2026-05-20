//! Valent Connect menu widget — the panel content for
//! `MenuType::Valent`. Ports the noctalia `valent-connect` Panel: a
//! header (title + refresh + device switcher), then a state card —
//! daemon-down / no-devices / unreachable / not-paired / connected.
//! The connected card shows the phone mock, battery / network /
//! signal stats, and the find / ping / browse / share actions.
//! Probing + actions live in [`crate::valent`].

use crate::valent::{self, Device, ValentReport};
use mshell_config::config_manager::config_manager;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};

pub(crate) struct ValentMenuWidgetModel {
    report: Option<ValentReport>,
    refreshing: bool,
    /// Device-switcher list is showing instead of the main card.
    switcher_open: bool,
}

#[derive(Debug)]
pub(crate) enum ValentMenuWidgetInput {
    /// Kick discovery + re-probe (header refresh button).
    Refresh,
    /// Re-probe only — used after an action / pair changes state.
    Reprobe,
    ToggleSwitcher,
    SelectDevice(String),
    FindMyPhone(String),
    Ping(String),
    Browse(String),
    /// Open a file chooser, then share the picked file.
    PickShare(String),
    Share(String, String),
    Pair(String),
    Unpair(String),
}

#[derive(Debug)]
pub(crate) enum ValentMenuWidgetOutput {
    CloseMenu,
}

pub(crate) struct ValentMenuWidgetInit {}

#[derive(Debug)]
pub(crate) enum ValentMenuWidgetCommandOutput {
    Loaded(ValentReport),
}

#[relm4::component(pub(crate))]
impl Component for ValentMenuWidgetModel {
    type CommandOutput = ValentMenuWidgetCommandOutput;
    type Input = ValentMenuWidgetInput;
    type Output = ValentMenuWidgetOutput;
    type Init = ValentMenuWidgetInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "valent-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 12,

            // Header: icon + title + switcher + refresh.
            gtk::Box {
                add_css_class: "valent-header",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 8,

                gtk::Image {
                    add_css_class: "valent-header-icon",
                    set_icon_name: Some("phone-symbolic"),
                },
                gtk::Label {
                    add_css_class: "label-medium-bold",
                    set_halign: gtk::Align::Start,
                    set_hexpand: true,
                    set_label: "Valent Connect",
                },

                gtk::Button {
                    add_css_class: "ok-button-surface",
                    set_valign: gtk::Align::Center,
                    set_tooltip_text: Some("Other devices"),
                    #[watch]
                    set_visible: model
                        .report
                        .as_ref()
                        .is_some_and(|r| r.devices.len() > 1),
                    connect_clicked[sender] => move |_| {
                        sender.input(ValentMenuWidgetInput::ToggleSwitcher);
                    },
                    gtk::Image { set_icon_name: Some("view-list-symbolic") },
                },

                gtk::Button {
                    add_css_class: "ok-button-surface",
                    set_valign: gtk::Align::Center,
                    set_tooltip_text: Some("Refresh"),
                    #[watch]
                    set_sensitive: !model.refreshing,
                    connect_clicked[sender] => move |_| {
                        sender.input(ValentMenuWidgetInput::Refresh);
                    },
                    gtk::Image { set_icon_name: Some("view-refresh-symbolic") },
                },
            },

            // State card — rebuilt imperatively per report.
            #[name = "content"]
            gtk::Box {
                add_css_class: "valent-content",
                set_orientation: gtk::Orientation::Vertical,
                set_vexpand: true,
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = ValentMenuWidgetModel {
            report: None,
            refreshing: true,
            switcher_open: false,
        };
        let widgets = view_output!();
        sender.input(ValentMenuWidgetInput::Reprobe);
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
            ValentMenuWidgetInput::Refresh => {
                self.refreshing = true;
                sender.oneshot_command(async {
                    valent::refresh_discovery().await;
                    let report = valent::probe().await;
                    ValentMenuWidgetCommandOutput::Loaded(report)
                });
            }
            ValentMenuWidgetInput::Reprobe => {
                self.refreshing = true;
                sender.oneshot_command(async {
                    ValentMenuWidgetCommandOutput::Loaded(valent::probe().await)
                });
            }
            ValentMenuWidgetInput::ToggleSwitcher => {
                self.switcher_open = !self.switcher_open;
                rebuild_content(
                    &widgets.content,
                    self.report.as_ref(),
                    self.switcher_open,
                    &sender,
                );
            }
            ValentMenuWidgetInput::SelectDevice(id) => {
                let stored = id.clone();
                config_manager().update_config(move |c| {
                    c.valent.main_device_id = stored;
                });
                self.switcher_open = false;
                rebuild_content(
                    &widgets.content,
                    self.report.as_ref(),
                    self.switcher_open,
                    &sender,
                );
            }
            ValentMenuWidgetInput::FindMyPhone(id) => {
                relm4::spawn(async move { valent::find_my_phone(id).await });
            }
            ValentMenuWidgetInput::Ping(id) => {
                relm4::spawn(async move { valent::ping(id).await });
            }
            ValentMenuWidgetInput::Browse(id) => {
                relm4::spawn(async move { valent::browse_files(id).await });
                // Browsing hands off to the file manager — close the
                // panel so it doesn't linger over it.
                let _ = sender.output(ValentMenuWidgetOutput::CloseMenu);
            }
            ValentMenuWidgetInput::PickShare(id) => {
                // Parent must be `None`: a layer-shell surface has no
                // xdg_toplevel, so handing it to the file-chooser
                // portal as a parent aborts GTK (crashing the shell).
                // The wallpaper menu picks folders the same way.
                let dialog = gtk::FileDialog::builder()
                    .title("Send file to phone")
                    .modal(true)
                    .build();
                let sender = sender.clone();
                dialog.open(gtk::Window::NONE, gtk::gio::Cancellable::NONE, move |res| {
                    if let Ok(file) = res {
                        if let Some(path) = file.path() {
                            sender.input(ValentMenuWidgetInput::Share(
                                id.clone(),
                                path.to_string_lossy().into_owned(),
                            ));
                        }
                    }
                });
            }
            ValentMenuWidgetInput::Share(id, path) => {
                relm4::spawn(async move { valent::share_file(id, path).await });
            }
            ValentMenuWidgetInput::Pair(id) => {
                relm4::spawn(async move { valent::pair(id).await });
                sender.input(ValentMenuWidgetInput::Reprobe);
            }
            ValentMenuWidgetInput::Unpair(id) => {
                relm4::spawn(async move { valent::unpair(id).await });
                sender.input(ValentMenuWidgetInput::Reprobe);
            }
        }
        self.update_view(widgets, sender);
    }

    fn update_cmd_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            ValentMenuWidgetCommandOutput::Loaded(report) => {
                self.refreshing = false;
                self.report = Some(report);
                rebuild_content(
                    &widgets.content,
                    self.report.as_ref(),
                    self.switcher_open,
                    &sender,
                );
            }
        }
        self.update_view(widgets, sender);
    }
}

fn preferred_id() -> String {
    use mshell_config::schema::config::{ConfigStoreFields, ValentStoreFields};
    use reactive_graph::traits::GetUntracked;
    config_manager()
        .config()
        .valent()
        .main_device_id()
        .get_untracked()
}

/// Clear + repaint the state card for the current report.
fn rebuild_content(
    container: &gtk::Box,
    report: Option<&ValentReport>,
    switcher_open: bool,
    sender: &ComponentSender<ValentMenuWidgetModel>,
) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }

    let Some(report) = report else {
        container.append(&info_card("dialog-information-symbolic", "Checking for devices…"));
        return;
    };

    if !report.daemon_available {
        container.append(&info_card(
            "dialog-warning-symbolic",
            "Valent daemon isn't running. Start it with\n`systemctl --user start valent`.",
        ));
        return;
    }

    if switcher_open && report.devices.len() > 1 {
        container.append(&switcher_card(report, sender));
        return;
    }

    let Some(device) = report.main_device(&preferred_id()) else {
        container.append(&info_card(
            "phone-disconnected-symbolic",
            "No devices found. Pair the KDE Connect app on your phone over the same network.",
        ));
        return;
    };

    if !device.reachable {
        container.append(&unreachable_card(device, sender));
    } else if !device.paired {
        container.append(&pairing_card(device, sender));
    } else {
        container.append(&connected_card(device, sender));
    }
}

// ── Cards ───────────────────────────────────────────────────────

fn connected_card(
    device: &Device,
    sender: &ComponentSender<ValentMenuWidgetModel>,
) -> gtk::Box {
    let card = card_box("valent-card");

    // Header row: name + action buttons.
    let head = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .build();
    let name = gtk::Label::builder()
        .label(&device.name)
        .halign(gtk::Align::Start)
        .hexpand(true)
        .xalign(0.0)
        .build();
    name.add_css_class("label-medium-bold");
    head.append(&name);

    head.append(&action_button(
        "edit-find-symbolic",
        "Find my phone",
        device.id.clone(),
        sender,
        ValentMenuWidgetInput::FindMyPhone,
    ));
    head.append(&action_button(
        "mail-send-symbolic",
        "Send a ping",
        device.id.clone(),
        sender,
        ValentMenuWidgetInput::Ping,
    ));
    head.append(&action_button(
        "folder-remote-symbolic",
        "Browse files (SFTP)",
        device.id.clone(),
        sender,
        ValentMenuWidgetInput::Browse,
    ));
    head.append(&action_button(
        "document-send-symbolic",
        "Send a file",
        device.id.clone(),
        sender,
        ValentMenuWidgetInput::PickShare,
    ));
    card.append(&head);

    // Stats: battery, network type, signal.
    let stats = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .build();
    stats.append(&stat_row(
        battery_icon(device.battery_charge, device.battery_charging),
        "Battery",
        &device
            .battery_charge
            .map(|c| format!("{c}%"))
            .unwrap_or_else(|| "Unknown".into()),
    ));

    // Cellular stats only show when the phone's connectivity report
    // plugin actually sends data — otherwise both rows just read
    // "Unknown", which looks broken. A muted hint explains why.
    let has_connectivity =
        device.network_strength >= 0 || !device.network_type.is_empty();
    if has_connectivity {
        stats.append(&stat_row(
            network_type_icon(&device.network_type),
            "Network",
            if device.network_type.is_empty() {
                "Unknown"
            } else {
                &device.network_type
            },
        ));
        stats.append(&stat_row(
            signal_icon(device.network_strength),
            "Signal",
            signal_text(device.network_strength),
        ));
    } else {
        let hint = gtk::Label::builder()
            .label(
                "Cellular report unavailable — enable the \
                 \"Connectivity report\" plugin in the KDE Connect \
                 app on your phone.",
            )
            .halign(gtk::Align::Start)
            .xalign(0.0)
            .wrap(true)
            .build();
        hint.add_css_class("label-small");
        stats.append(&hint);
    }
    card.append(&stats);

    card
}

fn pairing_card(
    device: &Device,
    sender: &ComponentSender<ValentMenuWidgetModel>,
) -> gtk::Box {
    let card = card_box("valent-card");

    let name = gtk::Label::builder()
        .label(&device.name)
        .halign(gtk::Align::Start)
        .xalign(0.0)
        .build();
    name.add_css_class("label-medium-bold");
    card.append(&name);

    let hint = gtk::Label::builder()
        .label(if device.pair_requested {
            "Pairing request sent — accept it on your phone."
        } else {
            "This device isn't paired yet."
        })
        .halign(gtk::Align::Start)
        .xalign(0.0)
        .wrap(true)
        .build();
    hint.add_css_class("label-small");
    card.append(&hint);

    let pair = gtk::Button::with_label("Pair");
    pair.add_css_class("ok-button-primary");
    pair.set_halign(gtk::Align::Start);
    pair.set_sensitive(!device.pair_requested);
    {
        let id = device.id.clone();
        let sender = sender.clone();
        pair.connect_clicked(move |_| {
            sender.input(ValentMenuWidgetInput::Pair(id.clone()));
        });
    }
    card.append(&pair);

    card
}

fn unreachable_card(
    device: &Device,
    sender: &ComponentSender<ValentMenuWidgetModel>,
) -> gtk::Box {
    let card = info_card(
        "phone-disconnected-symbolic",
        &format!("{} is paired but not reachable.", device.name),
    );
    let unpair = gtk::Button::with_label("Unpair");
    unpair.add_css_class("ok-button-surface");
    unpair.set_halign(gtk::Align::Center);
    {
        let id = device.id.clone();
        let sender = sender.clone();
        unpair.connect_clicked(move |_| {
            sender.input(ValentMenuWidgetInput::Unpair(id.clone()));
        });
    }
    card.append(&unpair);
    card
}

fn switcher_card(
    report: &ValentReport,
    sender: &ComponentSender<ValentMenuWidgetModel>,
) -> gtk::Box {
    let card = card_box("valent-card");
    let current = preferred_id();
    for dev in &report.devices {
        let btn = gtk::Button::with_label(&dev.name);
        btn.add_css_class(if dev.id == current {
            "ok-button-primary"
        } else {
            "ok-button-surface"
        });
        btn.set_halign(gtk::Align::Fill);
        {
            let id = dev.id.clone();
            let sender = sender.clone();
            btn.connect_clicked(move |_| {
                sender.input(ValentMenuWidgetInput::SelectDevice(id.clone()));
            });
        }
        card.append(&btn);
    }
    card
}

// ── Small builders ──────────────────────────────────────────────

fn card_box(class: &str) -> gtk::Box {
    let b = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(14)
        .build();
    b.add_css_class(class);
    b
}

fn info_card(icon: &str, text: &str) -> gtk::Box {
    let card = card_box("valent-card");
    card.set_valign(gtk::Align::Center);
    let img = gtk::Image::from_icon_name(icon);
    img.add_css_class("valent-info-icon");
    img.set_pixel_size(48);
    card.append(&img);
    let label = gtk::Label::builder()
        .label(text)
        .justify(gtk::Justification::Center)
        .wrap(true)
        .build();
    label.add_css_class("label-small");
    card.append(&label);
    card
}

fn action_button(
    icon: &str,
    tooltip: &str,
    id: String,
    sender: &ComponentSender<ValentMenuWidgetModel>,
    make: fn(String) -> ValentMenuWidgetInput,
) -> gtk::Button {
    let btn = gtk::Button::from_icon_name(icon);
    btn.add_css_class("ok-button-surface");
    btn.set_tooltip_text(Some(tooltip));
    btn.set_valign(gtk::Align::Center);
    let sender = sender.clone();
    btn.connect_clicked(move |_| {
        sender.input(make(id.clone()));
    });
    btn
}

fn stat_row(icon: &str, label: &str, value: &str) -> gtk::Box {
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(12)
        .build();
    row.add_css_class("valent-stat");

    let img = gtk::Image::from_icon_name(icon);
    img.add_css_class("valent-stat-icon");
    img.set_pixel_size(28);
    row.append(&img);

    let col = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .build();
    let cap = gtk::Label::builder().label(label).halign(gtk::Align::Start).xalign(0.0).build();
    cap.add_css_class("valent-stat-caption");
    let val = gtk::Label::builder().label(value).halign(gtk::Align::Start).xalign(0.0).build();
    val.add_css_class("valent-stat-value");
    col.append(&cap);
    col.append(&val);
    row.append(&col);

    row
}

// ── Icon / label maps ───────────────────────────────────────────

fn battery_icon(charge: Option<i32>, charging: bool) -> &'static str {
    if charging {
        return "battery-full-charging-symbolic";
    }
    match charge.unwrap_or(-1) {
        c if c < 0 => "battery-missing-symbolic",
        c if c < 10 => "battery-empty-symbolic",
        c if c < 30 => "battery-caution-symbolic",
        c if c < 55 => "battery-low-symbolic",
        c if c < 80 => "battery-good-symbolic",
        _ => "battery-full-symbolic",
    }
}

fn network_type_icon(t: &str) -> &'static str {
    match t {
        "5G" => "network-cellular-5g-symbolic",
        "LTE" | "4G" => "network-cellular-4g-symbolic",
        "HSPA" | "UMTS" | "3G" | "CDMA" | "CDMA2000" => "network-cellular-3g-symbolic",
        "EDGE" | "GPRS" | "GSM" | "2G" | "iDEN" => "network-cellular-2g-symbolic",
        "" => "network-cellular-offline-symbolic",
        _ => "network-cellular-symbolic",
    }
}

fn signal_icon(strength: i32) -> &'static str {
    match strength {
        0 => "network-cellular-signal-none-symbolic",
        1 => "network-cellular-signal-weak-symbolic",
        2 => "network-cellular-signal-ok-symbolic",
        3 => "network-cellular-signal-good-symbolic",
        4 => "network-cellular-signal-excellent-symbolic",
        _ => "network-cellular-signal-disabled-symbolic",
    }
}

fn signal_text(strength: i32) -> &'static str {
    match strength {
        0 => "Very weak",
        1 => "Weak",
        2 => "Fair",
        3 => "Good",
        4 => "Excellent",
        _ => "Unknown",
    }
}
