//! USB Drives menu — the panel content for `MenuType::Usb`. Ports the
//! noctalia `usb-manager` panel natively on top of
//! [`crate::removable_drives`] (udisks2 over D-Bus):
//!
//! * one content card per removable drive — name, size, and a
//!   **Remove safely** button (unmount → lock → power off / eject);
//! * one row per volume — label, filesystem, mount point — with
//!   **Open** (mounting first when needed), **Unmount**, and for LUKS
//!   volumes **Unlock** (inline passphrase entry) / **Lock**.
//!
//! The list is push-driven by the shared udisks watcher (mounts made
//! elsewhere show up too); nothing polls. Outcomes are reported as
//! toasts. udisks asks polkit, so the shell's polkit agent may prompt.

use crate::menus::reveal_poll::{RevealGate, watch_stream};
use crate::removable_drives::{self, Crypt, Drive, DrivesState, Volume};
use mshell_utils::units::bytes as format_size;
use relm4::gtk::prelude::{BoxExt, ButtonExt, EditableExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

pub(crate) struct UsbMenuWidgetModel {
    state: DrivesState,
    /// Volume (object path) whose passphrase entry is open.
    unlocking: Option<String>,
    status_label: gtk::Label,
    content: gtk::Box,
}

#[derive(Debug)]
pub(crate) enum UsbMenuWidgetInput {
    /// Mount (if needed) and open the volume in the file manager.
    Open(Volume),
    Unmount(Volume),
    /// Show / hide the passphrase entry under a locked volume.
    AskPassphrase(String),
    Unlock(Volume, String),
    Lock(Volume),
    RemoveDrive(Drive),
}

#[derive(Debug)]
pub(crate) enum UsbMenuWidgetOutput {}

pub(crate) struct UsbMenuWidgetInit {}

#[derive(Debug)]
pub(crate) enum UsbMenuWidgetCommandOutput {
    State(DrivesState),
}

#[relm4::component(pub(crate))]
impl Component for UsbMenuWidgetModel {
    type CommandOutput = UsbMenuWidgetCommandOutput;
    type Input = UsbMenuWidgetInput;
    type Output = UsbMenuWidgetOutput;
    type Init = UsbMenuWidgetInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "usb-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 12,

            // ── §12 panel header ──
            gtk::Box {
                add_css_class: "panel-header",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 12,
                gtk::Image {
                    add_css_class: "panel-header-icon",
                    set_valign: gtk::Align::Center,
                    set_icon_name: Some("drive-removable-media-symbolic"),
                },
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_hexpand: true,
                    gtk::Label {
                        add_css_class: "panel-title",
                        set_halign: gtk::Align::Start,
                        set_label: "USB Drives",
                    },
                    #[local_ref]
                    status_label_widget -> gtk::Label {
                        add_css_class: "label-small",
                        set_halign: gtk::Align::Start,
                        set_xalign: 0.0,
                    },
                },
            },

            #[name = "fill_list"]
            gtk::ScrolledWindow {
                add_css_class: "usb-scroller",
                set_hscrollbar_policy: gtk::PolicyType::Never,
                set_min_content_height: 0,
                set_max_content_height: 480,
                set_propagate_natural_height: true,

                #[local_ref]
                content -> gtk::Box {
                    add_css_class: "usb-drive-list",
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 8,
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let status_label_widget = gtk::Label::new(None);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 8);

        // Subscribed only while the menu is open: a closed menu keeps no
        // udisks watch alive (a USB pill still does).
        let open = RevealGate::follow_mapped(&root);
        sender.command(|out, shutdown| async move {
            let watch = open.stream_while_revealed(
                || watch_stream(removable_drives::subscribe()),
                |state| {
                    let _ = out.send(UsbMenuWidgetCommandOutput::State(state));
                },
            );
            tokio::select! {
                () = shutdown.wait() => {}
                () = watch => {}
            }
        });

        let model = UsbMenuWidgetModel {
            state: removable_drives::current(),
            unlocking: None,
            status_label: status_label_widget.clone(),
            content: content.clone(),
        };
        let widgets = view_output!();
        crate::menus::list_fit::mark_menu_list(&widgets.fill_list, 480);
        model.rebuild(&sender);

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            UsbMenuWidgetInput::Open(v) => {
                relm4::spawn(async move {
                    let mount_point = match v.mount_points.first() {
                        Some(mp) => Some(mp.clone()),
                        None => match &v.fs_path {
                            Some(fs) => match removable_drives::mount(fs).await {
                                Ok(mp) => Some(mp),
                                Err(e) => {
                                    removable_drives::report(
                                        &format!("Couldn't mount {}", v.title()),
                                        Err(removable_drives::error_text(&e)),
                                    );
                                    None
                                }
                            },
                            None => None,
                        },
                    };
                    if let Some(mp) = mount_point {
                        open_folder(&mp);
                    }
                });
            }
            UsbMenuWidgetInput::Unmount(v) => {
                relm4::spawn(async move {
                    let Some(fs) = v.fs_path.clone() else { return };
                    if let Err(e) = removable_drives::unmount(&fs).await {
                        removable_drives::report(
                            &format!("Couldn't unmount {}", v.title()),
                            Err(removable_drives::error_text(&e)),
                        );
                    }
                });
            }
            UsbMenuWidgetInput::AskPassphrase(path) => {
                self.unlocking = if self.unlocking.as_deref() == Some(path.as_str()) {
                    None
                } else {
                    Some(path)
                };
                self.rebuild(&sender);
            }
            UsbMenuWidgetInput::Unlock(v, passphrase) => {
                self.unlocking = None;
                relm4::spawn(async move {
                    match removable_drives::unlock_and_mount(&v.path, &passphrase).await {
                        Ok(Some(mp)) => open_folder(&mp),
                        Ok(None) => {}
                        Err(e) => removable_drives::report(
                            &format!("Couldn't unlock {}", v.title()),
                            Err(removable_drives::error_text(&e)),
                        ),
                    }
                });
                self.rebuild(&sender);
            }
            UsbMenuWidgetInput::Lock(v) => {
                relm4::spawn(async move {
                    if let Err(e) = removable_drives::lock(&v).await {
                        removable_drives::report(
                            &format!("Couldn't lock {}", v.title()),
                            Err(removable_drives::error_text(&e)),
                        );
                    }
                });
            }
            UsbMenuWidgetInput::RemoveDrive(d) => {
                removable_drives::spawn_remove_all(vec![d]);
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
            UsbMenuWidgetCommandOutput::State(state) => {
                if self.state != state {
                    self.state = state;
                    // Drop a passphrase entry whose volume went away.
                    if let Some(p) = &self.unlocking
                        && !self
                            .state
                            .drives
                            .iter()
                            .flat_map(|d| &d.volumes)
                            .any(|v| &v.path == p && v.crypt == Crypt::Locked)
                    {
                        self.unlocking = None;
                    }
                    self.rebuild(&sender);
                }
            }
        }
    }
}

impl UsbMenuWidgetModel {
    fn rebuild(&self, sender: &ComponentSender<Self>) {
        self.status_label.set_label(&status_line(&self.state));
        while let Some(child) = self.content.first_child() {
            self.content.remove(&child);
        }
        for d in &self.state.drives {
            self.content
                .append(&drive_card(d, self.unlocking.as_deref(), sender));
        }
    }
}

/// Header status line.
pub(crate) fn status_line(state: &DrivesState) -> String {
    if !state.available {
        return "udisks2 isn't running — install / start udisks2".to_string();
    }
    let n = state.drives.len();
    let mounted = state
        .drives
        .iter()
        .flat_map(|d| &d.volumes)
        .filter(|v| v.is_mounted())
        .count();
    match (n, mounted) {
        (0, _) => "No removable drives".to_string(),
        (1, 0) => "1 drive".to_string(),
        (n, 0) => format!("{n} drives"),
        (1, m) => format!("1 drive · {m} mounted"),
        (n, m) => format!("{n} drives · {m} mounted"),
    }
}

/// Volume subtitle: `vfat · /run/media/k/STICK`, `ext4 · locked`, …
pub(crate) fn volume_subtitle(v: &Volume) -> String {
    let state = match (&v.crypt, v.mount_points.first()) {
        (Crypt::Locked, _) => "Encrypted · locked".to_string(),
        (_, Some(mp)) => mp.clone(),
        (Crypt::Unlocked(_), None) => "Unlocked · not mounted".to_string(),
        (Crypt::None, None) => "Not mounted".to_string(),
    };
    let fs = if v.fs_type.is_empty() || v.crypt == Crypt::Locked {
        format_size(v.size)
    } else {
        format!("{} {}", format_size(v.size), v.fs_type)
    };
    format!("{fs}  ·  {state}")
}

fn drive_card(
    d: &Drive,
    unlocking: Option<&str>,
    sender: &ComponentSender<UsbMenuWidgetModel>,
) -> gtk::Box {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
    card.add_css_class("usb-drive-card");

    // Header: name + size, Remove safely.
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    head.add_css_class("usb-drive-header");
    let icon = gtk::Image::from_icon_name("drive-removable-media-symbolic");
    icon.set_valign(gtk::Align::Center);
    head.append(&icon);
    let texts = gtk::Box::new(gtk::Orientation::Vertical, 0);
    texts.set_hexpand(true);
    let name = gtk::Label::new(Some(&d.name));
    name.add_css_class("label-medium-bold");
    name.set_xalign(0.0);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    texts.append(&name);
    let sub = gtk::Label::new(Some(&format_size(d.size)));
    sub.set_css_classes(&["label-small", "dim-label"]);
    sub.set_xalign(0.0);
    texts.append(&sub);
    head.append(&texts);
    let remove = gtk::Button::with_label("Remove safely");
    remove.set_css_classes(&["ok-button-surface", "usb-remove-btn"]);
    remove.set_valign(gtk::Align::Center);
    remove.set_tooltip_text(Some(d.removal_hint()));
    {
        let s = sender.clone();
        let drive = d.clone();
        remove.connect_clicked(move |_| s.input(UsbMenuWidgetInput::RemoveDrive(drive.clone())));
    }
    head.append(&remove);
    card.append(&head);

    for v in &d.volumes {
        card.append(&volume_row(v, sender));
        if unlocking == Some(v.path.as_str()) {
            card.append(&passphrase_row(v, sender));
        }
    }
    card
}

fn volume_row(v: &Volume, sender: &ComponentSender<UsbMenuWidgetModel>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.add_css_class("usb-volume-row");
    if v.is_mounted() {
        row.add_css_class("mounted");
    }
    let texts = gtk::Box::new(gtk::Orientation::Vertical, 0);
    texts.set_hexpand(true);
    texts.set_valign(gtk::Align::Center);
    let title = gtk::Label::new(Some(&v.title()));
    title.add_css_class("label-medium");
    title.set_xalign(0.0);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    texts.append(&title);
    let sub = gtk::Label::new(Some(&volume_subtitle(v)));
    sub.set_css_classes(&["label-small", "dim-label"]);
    sub.set_xalign(0.0);
    sub.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    texts.append(&sub);
    row.append(&texts);

    match &v.crypt {
        Crypt::Locked => {
            let path = v.path.clone();
            row.append(&icon_button(
                "changes-allow-symbolic",
                "Unlock",
                sender,
                move || UsbMenuWidgetInput::AskPassphrase(path.clone()),
            ));
        }
        crypt => {
            if v.fs_path.is_some() {
                let open = v.clone();
                row.append(&icon_button(
                    "folder-symbolic",
                    if v.is_mounted() {
                        "Open in file manager"
                    } else {
                        "Mount and open"
                    },
                    sender,
                    move || UsbMenuWidgetInput::Open(open.clone()),
                ));
            }
            if v.is_mounted() {
                let um = v.clone();
                row.append(&icon_button(
                    "media-eject-symbolic",
                    "Unmount",
                    sender,
                    move || UsbMenuWidgetInput::Unmount(um.clone()),
                ));
            }
            if matches!(crypt, Crypt::Unlocked(_)) {
                let lk = v.clone();
                row.append(&icon_button(
                    "changes-prevent-symbolic",
                    "Lock",
                    sender,
                    move || UsbMenuWidgetInput::Lock(lk.clone()),
                ));
            }
        }
    }
    row
}

/// Inline passphrase entry for a locked LUKS volume. The text goes
/// straight to udisks; it is never logged or kept.
fn passphrase_row(v: &Volume, sender: &ComponentSender<UsbMenuWidgetModel>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.add_css_class("usb-passphrase-row");
    let entry = gtk::PasswordEntry::new();
    entry.set_show_peek_icon(true);
    entry.set_hexpand(true);
    entry.set_placeholder_text(Some("Passphrase"));
    mshell_common::a11y::name(&entry, &format!("Passphrase for {}", v.title()));
    let unlock = gtk::Button::with_label("Unlock");
    unlock.set_css_classes(&["ok-button-primary"]);
    let submit = {
        let s = sender.clone();
        let vol = v.clone();
        let entry = entry.clone();
        move || {
            let text = entry.text().to_string();
            if !text.is_empty() {
                entry.set_text("");
                s.input(UsbMenuWidgetInput::Unlock(vol.clone(), text));
            }
        }
    };
    {
        let submit = submit.clone();
        entry.connect_activate(move |_| submit());
    }
    unlock.connect_clicked(move |_| submit());
    row.append(&entry);
    row.append(&unlock);
    entry.grab_focus();
    row
}

/// Icon-only flat row action (podman-style) with an accessible name.
fn icon_button(
    icon: &str,
    label: &str,
    sender: &ComponentSender<UsbMenuWidgetModel>,
    msg: impl Fn() -> UsbMenuWidgetInput + 'static,
) -> gtk::Button {
    let btn = gtk::Button::from_icon_name(icon);
    btn.add_css_class("ok-button-flat");
    btn.set_valign(gtk::Align::Center);
    btn.set_tooltip_text(Some(label));
    mshell_common::a11y::name(&btn, label);
    let s = sender.clone();
    btn.connect_clicked(move |_| s.input(msg()));
    btn
}

/// Open a mount point in the user's file manager.
fn open_folder(path: &str) {
    if let Err(e) = mshell_process::desktop::open(path) {
        removable_drives::report("Couldn't open the folder", Err(format!("xdg-open: {e}")));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vol(crypt: Crypt, fs: &str, mounts: &[&str]) -> Volume {
        Volume {
            path: "/b/sdb1".into(),
            device: "/dev/sdb1".into(),
            label: String::new(),
            fs_type: fs.into(),
            size: 16_000_000_000,
            crypt,
            fs_path: Some("/b/sdb1".into()),
            mount_points: mounts.iter().map(|m| m.to_string()).collect(),
        }
    }

    #[test]
    fn subtitles() {
        assert_eq!(
            volume_subtitle(&vol(Crypt::None, "vfat", &["/run/media/k/S"])),
            "14.9 GiB vfat  ·  /run/media/k/S"
        );
        assert_eq!(
            volume_subtitle(&vol(Crypt::Locked, "crypto_LUKS", &[])),
            "14.9 GiB  ·  Encrypted · locked"
        );
        assert_eq!(
            volume_subtitle(&vol(Crypt::Unlocked("/b/dm".into()), "ext4", &[])),
            "14.9 GiB ext4  ·  Unlocked · not mounted"
        );
        assert_eq!(
            volume_subtitle(&vol(Crypt::None, "", &[])),
            "14.9 GiB  ·  Not mounted"
        );
    }

    #[test]
    fn status_lines() {
        assert_eq!(
            status_line(&DrivesState::default()),
            "udisks2 isn't running — install / start udisks2"
        );
        let mut s = DrivesState {
            available: true,
            drives: vec![],
        };
        assert_eq!(status_line(&s), "No removable drives");
        s.drives.push(Drive {
            path: "/d".into(),
            name: "X".into(),
            size: 1,
            can_power_off: true,
            ejectable: false,
            volumes: vec![vol(Crypt::None, "vfat", &["/m"])],
        });
        assert_eq!(status_line(&s), "1 drive · 1 mounted");
        s.drives.push(s.drives[0].clone());
        s.drives[1].volumes.clear();
        assert_eq!(status_line(&s), "2 drives · 1 mounted");
    }
}
