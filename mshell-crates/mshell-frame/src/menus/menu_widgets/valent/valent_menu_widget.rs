//! Valent Connect menu widget — the panel content for
//! `MenuType::Valent`. Ports the noctalia `valent-connect` Panel: a
//! header (title + settings + refresh + device switcher), then a state
//! card — daemon-down / no-devices / unreachable / not-paired /
//! connected. The connected card shows the phone mock, battery /
//! network / signal stats, quick actions (find / ping / browse /
//! share file / share text / clipboard push-pull), and inline
//! alias/avatar editing. Probing + actions live in [`crate::valent`].

use crate::valent::{self, Device, ValentReport};
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, ValentDeviceOverride, ValentStoreFields};
use reactive_graph::traits::GetUntracked;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};

pub(crate) struct ValentMenuWidgetModel {
    report: Option<ValentReport>,
    refreshing: bool,
    /// Device-switcher list is showing instead of the main card.
    switcher_open: bool,
    /// Inline "share text" entry row is showing under the connected card.
    share_text_open: bool,
    /// Device id currently being renamed inline (header shows an Entry
    /// instead of the name Label for this device).
    renaming_id: Option<String>,
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
    ClipboardPush(String),
    ClipboardPull(String),
    ToggleShareText,
    ShareTextSend(String, String),
    StartRename(String),
    CancelRename,
    CommitRename(String, String),
    /// Open a file chooser, then set the picked image as the device avatar.
    PickAvatar(String),
    AvatarChosen(String, String),
    PollIntervalChanged(u32),
    ShowBatteryPercentChanged(bool),
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

            // Header: icon + title + settings + switcher + refresh.
            gtk::Box {
                add_css_class: "valent-header",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 8,

                gtk::Image {
                    add_css_class: "panel-header-icon",
                    set_icon_name: Some("phone-symbolic"),
                    set_valign: gtk::Align::Center,
                },
                gtk::Label {
                    add_css_class: "panel-title",
                    set_halign: gtk::Align::Start,
                    set_hexpand: true,
                    set_label: "Valent Connect",
                },

                #[name = "settings_button"]
                gtk::MenuButton {
                    add_css_class: "ok-button-surface",
                    set_valign: gtk::Align::Center,
                    set_tooltip_text: Some("Valent settings"),
                    set_icon_name: "emblem-system-symbolic",

                    #[wrap(Some)]
                    set_popover = &gtk::Popover {
                        gtk::Box {
                            add_css_class: "valent-settings-popover",
                            set_orientation: gtk::Orientation::Vertical,
                            set_spacing: 10,
                            set_margin_start: 10,
                            set_margin_end: 10,
                            set_margin_top: 10,
                            set_margin_bottom: 10,

                            gtk::Box {
                                set_orientation: gtk::Orientation::Horizontal,
                                set_spacing: 8,
                                gtk::Label {
                                    add_css_class: "label-small",
                                    set_label: "Poll interval (s)",
                                    set_halign: gtk::Align::Start,
                                    set_hexpand: true,
                                },
                                #[name = "poll_interval_spin"]
                                gtk::SpinButton {
                                    set_valign: gtk::Align::Center,
                                    set_adjustment: &gtk::Adjustment::new(5.0, 5.0, 300.0, 5.0, 5.0, 0.0),
                                    connect_value_changed[sender] => move |s| {
                                        sender.input(ValentMenuWidgetInput::PollIntervalChanged(
                                            s.value().round() as u32,
                                        ));
                                    },
                                },
                            },

                            gtk::Box {
                                set_orientation: gtk::Orientation::Horizontal,
                                set_spacing: 8,
                                gtk::Label {
                                    add_css_class: "label-small",
                                    set_label: "Show battery %",
                                    set_halign: gtk::Align::Start,
                                    set_hexpand: true,
                                },
                                #[name = "battery_percent_switch"]
                                gtk::Switch {
                                    set_valign: gtk::Align::Center,
                                    connect_active_notify[sender] => move |s| {
                                        sender.input(ValentMenuWidgetInput::ShowBatteryPercentChanged(
                                            s.is_active(),
                                        ));
                                    },
                                },
                            },
                        },
                    },
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
            share_text_open: false,
            renaming_id: None,
        };
        let widgets = view_output!();

        // Pre-fill the settings popover from config each time it opens —
        // it isn't reactive (relm4 doesn't watch a plain gtk::Popover), so
        // this is the read side; the SpinButton/Switch signals above are
        // the write side (live, not commit-on-close — cheap scalar writes).
        if let Some(popover) = widgets.settings_button.popover() {
            let spin = widgets.poll_interval_spin.clone();
            let switch = widgets.battery_percent_switch.clone();
            popover.connect_show(move |_| {
                let v = config_manager()
                    .config()
                    .valent()
                    .poll_interval_secs()
                    .get_untracked();
                spin.set_value(v as f64);
                let show = config_manager()
                    .config()
                    .valent()
                    .show_battery_percent()
                    .get_untracked();
                switch.set_active(show);
            });
        }

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
                self.rebuild(widgets, &sender);
            }
            ValentMenuWidgetInput::SelectDevice(id) => {
                let stored = id.clone();
                config_manager().update_config(move |c| {
                    c.valent.main_device_id = stored;
                });
                self.switcher_open = false;
                self.rebuild(widgets, &sender);
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
                    if let Ok(file) = res
                        && let Some(path) = file.path()
                    {
                        sender.input(ValentMenuWidgetInput::Share(
                            id.clone(),
                            path.to_string_lossy().into_owned(),
                        ));
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
            ValentMenuWidgetInput::ClipboardPush(id) => {
                relm4::spawn(async move { valent::clipboard_push(id).await });
            }
            ValentMenuWidgetInput::ClipboardPull(id) => {
                relm4::spawn(async move { valent::clipboard_pull(id).await });
            }
            ValentMenuWidgetInput::ToggleShareText => {
                self.share_text_open = !self.share_text_open;
                self.rebuild(widgets, &sender);
            }
            ValentMenuWidgetInput::ShareTextSend(id, text) => {
                relm4::spawn(async move { valent::share_text(id, text).await });
                self.share_text_open = false;
                self.rebuild(widgets, &sender);
            }
            ValentMenuWidgetInput::StartRename(id) => {
                self.renaming_id = Some(id);
                self.rebuild(widgets, &sender);
            }
            ValentMenuWidgetInput::CancelRename => {
                self.renaming_id = None;
                self.rebuild(widgets, &sender);
            }
            ValentMenuWidgetInput::CommitRename(id, alias) => {
                set_device_alias(&id, alias.trim().to_string());
                self.renaming_id = None;
                self.rebuild(widgets, &sender);
            }
            ValentMenuWidgetInput::PickAvatar(id) => {
                // Same parent=None rationale as PickShare above.
                let dialog = gtk::FileDialog::builder()
                    .title("Choose a device image")
                    .modal(true)
                    .build();
                let sender = sender.clone();
                dialog.open(gtk::Window::NONE, gtk::gio::Cancellable::NONE, move |res| {
                    if let Ok(file) = res
                        && let Some(path) = file.path()
                    {
                        sender.input(ValentMenuWidgetInput::AvatarChosen(
                            id.clone(),
                            path.to_string_lossy().into_owned(),
                        ));
                    }
                });
            }
            ValentMenuWidgetInput::AvatarChosen(id, path) => {
                set_device_image(&id, path);
                self.rebuild(widgets, &sender);
            }
            ValentMenuWidgetInput::PollIntervalChanged(secs) => {
                config_manager().update_config(move |c| {
                    c.valent.poll_interval_secs = secs.clamp(5, 300);
                });
            }
            ValentMenuWidgetInput::ShowBatteryPercentChanged(show) => {
                config_manager().update_config(move |c| {
                    c.valent.show_battery_percent = show;
                });
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
                self.rebuild(widgets, &sender);
            }
        }
        self.update_view(widgets, sender);
    }
}

impl ValentMenuWidgetModel {
    /// Clear + repaint the state card for the current model state.
    fn rebuild(&self, widgets: &<Self as Component>::Widgets, sender: &ComponentSender<Self>) {
        rebuild_content(
            &widgets.content,
            self.report.as_ref(),
            self.switcher_open,
            self.share_text_open,
            self.renaming_id.as_deref(),
            sender,
        );
    }
}

fn preferred_id() -> String {
    config_manager()
        .config()
        .valent()
        .main_device_id()
        .get_untracked()
}

// ── Per-device overrides (alias / avatar) ──────────────────────────
// Purely local cosmetics — Valent's own D-Bus surface has no concept
// of either, so these never leave `Valent::devices` in the shell's own
// config profile.

fn device_override(device_id: &str) -> Option<ValentDeviceOverride> {
    config_manager()
        .config()
        .valent()
        .devices()
        .get_untracked()
        .into_iter()
        .find(|d| d.device_id == device_id)
}

fn display_name(device: &Device) -> String {
    device_override(&device.id)
        .map(|o| o.alias)
        .filter(|a| !a.is_empty())
        .unwrap_or_else(|| device.name.clone())
}

fn device_image_path(device_id: &str) -> Option<String> {
    device_override(device_id)
        .map(|o| o.image_path)
        .filter(|p| !p.is_empty())
}

fn upsert_device_override(device_id: &str, f: impl FnOnce(&mut ValentDeviceOverride)) {
    let device_id = device_id.to_string();
    config_manager().update_config(move |c| {
        match c
            .valent
            .devices
            .iter_mut()
            .find(|d| d.device_id == device_id)
        {
            Some(entry) => f(entry),
            None => {
                let mut entry = ValentDeviceOverride {
                    device_id: device_id.clone(),
                    ..Default::default()
                };
                f(&mut entry);
                c.valent.devices.push(entry);
            }
        }
        // An override that's back to fully-empty is just clutter.
        c.valent
            .devices
            .retain(|d| !d.alias.is_empty() || !d.image_path.is_empty());
    });
}

fn set_device_alias(device_id: &str, alias: String) {
    upsert_device_override(device_id, |entry| entry.alias = alias);
}

fn set_device_image(device_id: &str, path: String) {
    upsert_device_override(device_id, |entry| entry.image_path = path);
}

// ── Cards ───────────────────────────────────────────────────────

/// Clear + repaint the state card for the current report.
fn rebuild_content(
    container: &gtk::Box,
    report: Option<&ValentReport>,
    switcher_open: bool,
    share_text_open: bool,
    renaming_id: Option<&str>,
    sender: &ComponentSender<ValentMenuWidgetModel>,
) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }

    let Some(report) = report else {
        container.append(&info_card(
            "dialog-information-symbolic",
            "Checking for devices…",
        ));
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
            "phone-symbolic",
            "No devices found. Pair the KDE Connect app on your phone over the same network.",
        ));
        return;
    };

    if !device.reachable {
        container.append(&unreachable_card(device, sender));
    } else if !device.paired {
        container.append(&pairing_card(device, sender));
    } else {
        container.append(&connected_card(
            device,
            share_text_open,
            renaming_id,
            sender,
        ));
    }
}

fn connected_card(
    device: &Device,
    share_text_open: bool,
    renaming_id: Option<&str>,
    sender: &ComponentSender<ValentMenuWidgetModel>,
) -> gtk::Box {
    let card = card_box("valent-card");

    // Header row: avatar + name (or inline rename entry) + rename toggle.
    card.append(&name_row(
        device,
        renaming_id == Some(device.id.as_str()),
        sender,
    ));

    // Quick actions: find / ping / browse.
    let quick = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .build();
    quick.append(&action_button(
        "edit-find-symbolic",
        "Find my phone",
        device.id.clone(),
        sender,
        ValentMenuWidgetInput::FindMyPhone,
    ));
    quick.append(&action_button(
        "mail-send-symbolic",
        "Send a ping",
        device.id.clone(),
        sender,
        ValentMenuWidgetInput::Ping,
    ));
    quick.append(&action_button(
        "folder-remote-symbolic",
        "Browse files (SFTP)",
        device.id.clone(),
        sender,
        ValentMenuWidgetInput::Browse,
    ));
    card.append(&quick);

    // Sharing actions: share file / share text / clipboard push-pull.
    let sharing = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .build();
    sharing.append(&action_button(
        "document-send-symbolic",
        "Send a file",
        device.id.clone(),
        sender,
        ValentMenuWidgetInput::PickShare,
    ));
    let share_text_btn = gtk::Button::from_icon_name("insert-text-symbolic");
    share_text_btn.add_css_class("ok-button-surface");
    share_text_btn.set_tooltip_text(Some("Share text"));
    share_text_btn.set_valign(gtk::Align::Center);
    {
        let sender = sender.clone();
        share_text_btn.connect_clicked(move |_| {
            sender.input(ValentMenuWidgetInput::ToggleShareText);
        });
    }
    sharing.append(&share_text_btn);
    sharing.append(&action_button(
        "edit-copy-symbolic",
        "Send clipboard to phone",
        device.id.clone(),
        sender,
        ValentMenuWidgetInput::ClipboardPush,
    ));
    sharing.append(&action_button(
        "edit-paste-symbolic",
        "Get phone's clipboard",
        device.id.clone(),
        sender,
        ValentMenuWidgetInput::ClipboardPull,
    ));
    card.append(&sharing);

    if share_text_open {
        card.append(&share_text_row(&device.id, sender));
    }

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
    let has_connectivity = device.network_strength >= 0 || !device.network_type.is_empty();
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

/// Header row: avatar (click → pick image) + name (click pencil → inline
/// rename) or, while `renaming`, an Entry + save/cancel buttons.
fn name_row(
    device: &Device,
    renaming: bool,
    sender: &ComponentSender<ValentMenuWidgetModel>,
) -> gtk::Box {
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();

    row.append(&avatar_button(&device.id, sender));

    if renaming {
        let entry = gtk::Entry::builder().hexpand(true).build();
        entry.set_text(
            &device_override(&device.id)
                .map(|o| o.alias)
                .unwrap_or_default(),
        );

        let commit = gtk::Button::from_icon_name("object-select-symbolic");
        commit.add_css_class("ok-button-surface");
        commit.set_tooltip_text(Some("Save"));
        {
            let id = device.id.clone();
            let sender = sender.clone();
            let entry = entry.clone();
            commit.connect_clicked(move |_| {
                sender.input(ValentMenuWidgetInput::CommitRename(
                    id.clone(),
                    entry.text().to_string(),
                ));
            });
        }
        {
            let id = device.id.clone();
            let sender = sender.clone();
            entry.connect_activate(move |e| {
                sender.input(ValentMenuWidgetInput::CommitRename(
                    id.clone(),
                    e.text().to_string(),
                ));
            });
        }

        let cancel = gtk::Button::from_icon_name("process-stop-symbolic");
        cancel.add_css_class("ok-button-surface");
        cancel.set_tooltip_text(Some("Cancel"));
        {
            let sender = sender.clone();
            cancel.connect_clicked(move |_| {
                sender.input(ValentMenuWidgetInput::CancelRename);
            });
        }

        row.append(&entry);
        row.append(&commit);
        row.append(&cancel);
    } else {
        let name = gtk::Label::builder()
            .label(display_name(device))
            .halign(gtk::Align::Start)
            .hexpand(true)
            .xalign(0.0)
            .build();
        name.add_css_class("label-medium-bold");
        row.append(&name);

        let rename_btn = gtk::Button::from_icon_name("document-edit-symbolic");
        rename_btn.add_css_class("ok-button-surface");
        rename_btn.set_tooltip_text(Some("Rename this device"));
        {
            let id = device.id.clone();
            let sender = sender.clone();
            rename_btn.connect_clicked(move |_| {
                sender.input(ValentMenuWidgetInput::StartRename(id.clone()));
            });
        }
        row.append(&rename_btn);
    }

    row
}

fn avatar_button(device_id: &str, sender: &ComponentSender<ValentMenuWidgetModel>) -> gtk::Button {
    let btn = gtk::Button::new();
    btn.add_css_class("ok-button-flat");
    btn.add_css_class("valent-avatar-button");
    btn.set_valign(gtk::Align::Center);
    btn.set_tooltip_text(Some("Change device image"));

    let img = match device_image_path(device_id) {
        Some(path) => gtk::Image::from_file(&path),
        None => gtk::Image::from_icon_name("phone-symbolic"),
    };
    img.add_css_class("valent-avatar");
    img.set_pixel_size(32);
    btn.set_child(Some(&img));

    {
        let id = device_id.to_string();
        let sender = sender.clone();
        btn.connect_clicked(move |_| {
            sender.input(ValentMenuWidgetInput::PickAvatar(id.clone()));
        });
    }
    btn
}

fn share_text_row(device_id: &str, sender: &ComponentSender<ValentMenuWidgetModel>) -> gtk::Box {
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .build();
    row.add_css_class("valent-share-text-row");

    let entry = gtk::Entry::builder()
        .hexpand(true)
        .placeholder_text("Type a message…")
        .build();

    let send = gtk::Button::from_icon_name("mail-send-symbolic");
    send.add_css_class("ok-button-primary");
    send.set_tooltip_text(Some("Send"));

    {
        let id = device_id.to_string();
        let sender = sender.clone();
        let entry = entry.clone();
        send.connect_clicked(move |_| {
            let text = entry.text().to_string();
            if !text.trim().is_empty() {
                sender.input(ValentMenuWidgetInput::ShareTextSend(id.clone(), text));
            }
        });
    }
    {
        let id = device_id.to_string();
        let sender = sender.clone();
        entry.connect_activate(move |e| {
            let text = e.text().to_string();
            if !text.trim().is_empty() {
                sender.input(ValentMenuWidgetInput::ShareTextSend(id.clone(), text));
            }
        });
    }

    row.append(&entry);
    row.append(&send);
    row
}

fn pairing_card(device: &Device, sender: &ComponentSender<ValentMenuWidgetModel>) -> gtk::Box {
    let card = card_box("valent-card");

    let name = gtk::Label::builder()
        .label(display_name(device))
        .halign(gtk::Align::Start)
        .xalign(0.0)
        .build();
    name.add_css_class("label-medium-bold");
    card.append(&name);

    // Valent exposes only `pair` / `unpair` — there is no separate
    // accept/reject GAction. While a pairing request is incoming we
    // relabel the same two actions as Accept/Reject, which is the only
    // meaningful mapping onto the existing verbs.
    if device.pair_incoming {
        let hint = gtk::Label::builder()
            .label("Your phone wants to pair.")
            .halign(gtk::Align::Start)
            .xalign(0.0)
            .wrap(true)
            .build();
        hint.add_css_class("label-small");
        card.append(&hint);

        let buttons = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(8)
            .build();

        let accept = gtk::Button::with_label("Accept");
        accept.add_css_class("ok-button-primary");
        accept.add_css_class("ok-button-cell");
        {
            let id = device.id.clone();
            let sender = sender.clone();
            accept.connect_clicked(move |_| {
                sender.input(ValentMenuWidgetInput::Pair(id.clone()));
            });
        }
        buttons.append(&accept);

        let reject = gtk::Button::with_label("Reject");
        reject.add_css_class("ok-button-surface");
        reject.add_css_class("ok-button-cell");
        {
            let id = device.id.clone();
            let sender = sender.clone();
            reject.connect_clicked(move |_| {
                sender.input(ValentMenuWidgetInput::Unpair(id.clone()));
            });
        }
        buttons.append(&reject);

        card.append(&buttons);
        return card;
    }

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
    pair.add_css_class("ok-button-cell");
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

fn unreachable_card(device: &Device, sender: &ComponentSender<ValentMenuWidgetModel>) -> gtk::Box {
    let card = info_card(
        "phone-symbolic",
        &format!("{} is paired but not reachable.", display_name(device)),
    );
    let unpair = gtk::Button::with_label("Unpair");
    unpair.add_css_class("ok-button-surface");
    unpair.add_css_class("ok-button-cell");
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
        let row = gtk::Button::new();
        row.add_css_class(if dev.id == current {
            "ok-button-primary"
        } else {
            "ok-button-surface"
        });
        row.set_halign(gtk::Align::Fill);

        let inner = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(8)
            .build();
        let img = match device_image_path(&dev.id) {
            Some(path) => gtk::Image::from_file(&path),
            None => gtk::Image::from_icon_name("phone-symbolic"),
        };
        img.set_pixel_size(20);
        inner.append(&img);
        inner.append(&gtk::Label::new(Some(&display_name(dev))));
        row.set_child(Some(&inner));

        {
            let id = dev.id.clone();
            let sender = sender.clone();
            row.connect_clicked(move |_| {
                sender.input(ValentMenuWidgetInput::SelectDevice(id.clone()));
            });
        }
        card.append(&row);
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
    let cap = gtk::Label::builder()
        .label(label)
        .halign(gtk::Align::Start)
        .xalign(0.0)
        .build();
    cap.add_css_class("valent-stat-caption");
    let val = gtk::Label::builder()
        .label(value)
        .halign(gtk::Align::Start)
        .xalign(0.0)
        .build();
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
    // KDE Connect passes Android's network-type string through verbatim,
    // and it varies by ROM / Android version — 5G can arrive as "5G",
    // "5G NR", "NR", "NRNSA", "NR_NSA", "5G+", in any case. Match on the
    // key token (case-insensitively) rather than an exact string, so a
    // 5G phone doesn't fall through to the 4G / generic branch. Order
    // matters: most-recent generation first.
    let t = t.trim().to_ascii_uppercase();
    if t.is_empty() {
        "network-cellular-offline-symbolic"
    } else if t.contains("5G") || t.contains("NR") {
        "network-cellular-5g-symbolic"
    } else if t.contains("LTE") || t.contains("4G") {
        "network-cellular-4g-symbolic"
    } else if t.contains("3G")
        || t.contains("HSPA")
        || t.contains("HSDPA")
        || t.contains("HSUPA")
        || t.contains("UMTS")
        || t.contains("WCDMA")
        || t.contains("EVDO")
        || t.contains("CDMA")
    {
        "network-cellular-3g-symbolic"
    } else if t.contains("2G")
        || t.contains("EDGE")
        || t.contains("GPRS")
        || t.contains("GSM")
        || t.contains("IDEN")
    {
        "network-cellular-2g-symbolic"
    } else {
        "network-cellular-symbolic"
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
