//! Settings → Users.
//!
//! A read-only view of the system's human accounts — username, full name,
//! administrator status, and avatar — parsed from `/etc/passwd` + `/etc/group`
//! (and `~/.face` / AccountsService icons). Account changes (password, avatar)
//! are deliberately left to the system tools; this page just surfaces who's
//! here and who's an admin, with no privileges required.

use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::path::PathBuf;

struct UserInfo {
    name: String,
    full_name: String,
    admin: bool,
    is_current: bool,
    avatar: Option<PathBuf>,
}

#[derive(Debug)]
pub(crate) struct UsersSettingsModel {
    list: gtk::Box,
}

#[derive(Debug)]
pub(crate) enum UsersSettingsInput {
    Refresh,
}

#[derive(Debug)]
pub(crate) enum UsersSettingsOutput {}

pub(crate) struct UsersSettingsInit {}

#[derive(Debug)]
pub(crate) enum UsersSettingsCommandOutput {}

#[relm4::component(pub)]
impl Component for UsersSettingsModel {
    type CommandOutput = UsersSettingsCommandOutput;
    type Input = UsersSettingsInput;
    type Output = UsersSettingsOutput;
    type Init = UsersSettingsInit;

    view! {
        #[root]
        gtk::ScrolledWindow {
            set_vscrollbar_policy: gtk::PolicyType::Automatic,
            set_hscrollbar_policy: gtk::PolicyType::Never,
            set_propagate_natural_height: false,
            set_propagate_natural_width: false,
            set_hexpand: true,
            set_vexpand: true,

            gtk::Box {
                add_css_class: "settings-page",
                set_orientation: gtk::Orientation::Vertical,
                set_hexpand: true,
                set_spacing: 16,

                gtk::Box {
                    add_css_class: "settings-hero",
                    set_orientation: gtk::Orientation::Horizontal,
                    set_halign: gtk::Align::Start,
                    set_spacing: 16,
                    gtk::Image {
                        add_css_class: "settings-hero-icon",
                        set_icon_name: Some("system-users-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "Users",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "The accounts on this system — read-only.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Accounts",
                    set_halign: gtk::Align::Start,
                },

                #[local_ref]
                list_box -> gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 8,
                },

                gtk::Label {
                    add_css_class: "label-small",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_margin_top: 8,
                    set_label: "To change your password run `passwd` in a terminal; avatars come from ~/.face. Account management (add / remove / promote) is a privileged system task.",
                },

                gtk::Button {
                    add_css_class: "ok-button-surface",
                    add_css_class: "ok-button-cell",
                    set_halign: gtk::Align::Start,
                    set_label: "Refresh",
                    connect_clicked[sender] => move |_| {
                        sender.input(UsersSettingsInput::Refresh);
                    },
                },
            }
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let _ = &sender;
        let list_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let model = UsersSettingsModel {
            list: list_box.clone(),
        };
        let widgets = view_output!();
        let _ = root;
        rebuild(&model.list);
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            UsersSettingsInput::Refresh => rebuild(&self.list),
        }
    }
}

/// Rebuild the account cards from the current /etc/passwd state.
fn rebuild(list: &gtk::Box) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    for u in read_users() {
        list.append(&user_card(&u));
    }
}

fn user_card(u: &UserInfo) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.add_css_class("ok-button-surface");
    row.add_css_class("ok-button-cell");

    let avatar = match u.avatar.as_deref().filter(|p| p.exists()) {
        Some(path) => gtk::Image::from_file(path),
        None => gtk::Image::from_icon_name("avatar-default-symbolic"),
    };
    avatar.set_pixel_size(40);
    avatar.set_valign(gtk::Align::Center);
    row.append(&avatar);

    let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    let title = if u.full_name.is_empty() || u.full_name == u.name {
        u.name.clone()
    } else {
        format!("{} ({})", u.full_name, u.name)
    };
    let name = gtk::Label::new(Some(&title));
    name.add_css_class("label-medium-bold");
    name.set_halign(gtk::Align::Start);
    name.set_xalign(0.0);
    text.append(&name);
    let role = gtk::Label::new(Some(if u.admin { "Administrator" } else { "Standard user" }));
    role.add_css_class("label-small");
    role.set_halign(gtk::Align::Start);
    role.set_xalign(0.0);
    text.append(&role);
    row.append(&text);

    if u.is_current {
        let chip = gtk::Label::new(Some("You"));
        chip.add_css_class("label-small-bold");
        chip.set_valign(gtk::Align::Center);
        row.append(&chip);
    }
    row
}

/// Human accounts from /etc/passwd (UID 1000–60000, real login shell),
/// current user first, with admin status from /etc/group.
fn read_users() -> Vec<UserInfo> {
    let admins = admin_members();
    let current = std::env::var("USER").unwrap_or_default();
    let Ok(passwd) = std::fs::read_to_string("/etc/passwd") else {
        return Vec::new();
    };
    let mut users: Vec<UserInfo> = passwd
        .lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split(':').collect();
            if f.len() < 7 {
                return None;
            }
            let (name, uid, gecos, shell) = (f[0], f[2].parse::<u32>().ok()?, f[4], f[6]);
            // Human accounts only: the regular UID range + a real shell.
            if !(1000..=60000).contains(&uid) {
                return None;
            }
            if shell.ends_with("nologin") || shell.ends_with("false") {
                return None;
            }
            let full_name = gecos.split(',').next().unwrap_or("").trim().to_string();
            Some(UserInfo {
                admin: admins.contains(&name.to_string()),
                is_current: name == current,
                avatar: avatar_for(name, name == current),
                full_name,
                name: name.to_string(),
            })
        })
        .collect();
    // Current user first, then alphabetical.
    users.sort_by(|a, b| b.is_current.cmp(&a.is_current).then(a.name.cmp(&b.name)));
    users
}

/// Members of the `wheel` / `sudo` groups (the usual admin groups).
fn admin_members() -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(group) = std::fs::read_to_string("/etc/group") {
        for line in group.lines() {
            let f: Vec<&str> = line.split(':').collect();
            if f.len() >= 4 && (f[0] == "wheel" || f[0] == "sudo") {
                out.extend(f[3].split(',').filter(|m| !m.is_empty()).map(String::from));
            }
        }
    }
    out
}

/// Avatar path: `~/.face` for the current user, else the AccountsService icon.
fn avatar_for(name: &str, is_current: bool) -> Option<PathBuf> {
    if is_current
        && let Some(home) = std::env::var_os("HOME")
    {
        let face = PathBuf::from(home).join(".face");
        if face.exists() {
            return Some(face);
        }
    }
    let asvc = PathBuf::from(format!("/var/lib/AccountsService/icons/{name}"));
    asvc.exists().then_some(asvc)
}
