//! Settings → Users.
//!
//! A GNOME-style account manager. Lists the system's human accounts
//! (parsed from `/etc/passwd` + `/etc/group`) and lets you edit them —
//! avatar, full name, account type (Standard / Administrator), password —
//! plus add / remove users. Privileged changes run through `pkexec`, so the
//! margo polkit agent prompts for authentication (same UX as GNOME's
//! AccountsService route); avatars for the current user just write `~/.face`
//! and need no privilege.
//!
//! Not yet covered (needs subsystems margo doesn't expose yet): automatic
//! login (mlogind has no autologin setting) and fingerprint enrolment (no
//! fprintd on this build).

use relm4::gtk::prelude::*;
use relm4::gtk::{gio, glib};
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
    status: gtk::Label,
    add_entry: gtk::Entry,
    busy: bool,
}

#[derive(Debug)]
pub(crate) enum UsersSettingsInput {
    SaveFullName(String, String),
    SetAdmin(String, bool),
    PickAvatar(String, bool),
    ApplyAvatar(String, bool, PathBuf),
    ChangePassword(String, String, String),
    AddUser,
    RemoveUser(String),
}

#[derive(Debug)]
pub(crate) enum UsersSettingsOutput {}

pub(crate) struct UsersSettingsInit {}

#[derive(Debug)]
pub(crate) enum UsersSettingsCommandOutput {
    OpDone { ok: bool, msg: String },
}

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
                            set_label: "Manage the accounts on this system. Privileged changes ask for your password.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                #[local_ref]
                status_label -> gtk::Label {
                    add_css_class: "label-small",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_visible: false,
                },

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Accounts",
                    set_halign: gtk::Align::Start,
                },

                #[local_ref]
                list_box -> gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 10,
                },

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Add user",
                    set_halign: gtk::Align::Start,
                    set_margin_top: 8,
                },
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 8,
                    #[local_ref]
                    add_entry_widget -> gtk::Entry {
                        set_hexpand: true,
                        set_placeholder_text: Some("username (lowercase, no spaces)"),
                    },
                    gtk::Button {
                        add_css_class: "ok-button-surface",
                        set_label: "Create",
                        connect_clicked[sender] => move |_| {
                            sender.input(UsersSettingsInput::AddUser);
                        },
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
        let list_box = gtk::Box::new(gtk::Orientation::Vertical, 10);
        let status_label = gtk::Label::new(None);
        let add_entry_widget = gtk::Entry::new();
        let model = UsersSettingsModel {
            list: list_box.clone(),
            status: status_label.clone(),
            add_entry: add_entry_widget.clone(),
            busy: false,
        };
        let widgets = view_output!();
        let _ = root;
        rebuild(&model.list, &sender);
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            UsersSettingsInput::SaveFullName(user, name) => {
                self.run_priv(
                    &sender,
                    vec!["chfn".into(), "-f".into(), name, user],
                    "Full name updated",
                );
            }

            UsersSettingsInput::SetAdmin(user, make_admin) => {
                // Guard: never strip the last administrator.
                if !make_admin && admin_count() <= 1 {
                    self.set_status("Can't remove the last administrator.", false);
                    rebuild(&self.list, &sender); // reset the toggle
                    return;
                }
                let flag = if make_admin { "-a" } else { "-d" };
                self.run_priv(
                    &sender,
                    vec!["gpasswd".into(), flag.into(), user, "wheel".into()],
                    if make_admin {
                        "Now an administrator"
                    } else {
                        "Now a standard user"
                    },
                );
            }

            UsersSettingsInput::PickAvatar(user, is_current) => {
                let dialog = gtk::FileDialog::builder().title("Choose a picture").build();
                let filter = gtk::FileFilter::new();
                filter.set_name(Some("Images"));
                for ext in ["png", "jpg", "jpeg", "webp", "svg", "gif", "bmp"] {
                    filter.add_suffix(ext);
                }
                let filters = gio::ListStore::new::<gtk::FileFilter>();
                filters.append(&filter);
                dialog.set_filters(Some(&filters));
                let sender = sender.clone();
                dialog.open(None::<&gtk::Window>, gio::Cancellable::NONE, move |res| {
                    if let Ok(file) = res
                        && let Some(path) = file.path()
                    {
                        sender.input(UsersSettingsInput::ApplyAvatar(
                            user.clone(),
                            is_current,
                            path,
                        ));
                    }
                });
            }

            UsersSettingsInput::ApplyAvatar(user, is_current, path) => {
                if is_current {
                    // The current user owns ~/.face — no privilege needed.
                    match std::env::var_os("HOME") {
                        Some(home) => {
                            let dest = PathBuf::from(home).join(".face");
                            match std::fs::copy(&path, &dest) {
                                Ok(_) => {
                                    self.set_status("Picture updated.", true);
                                    rebuild(&self.list, &sender);
                                }
                                Err(e) => {
                                    self.set_status(&format!("Couldn't write ~/.face: {e}"), false)
                                }
                            }
                        }
                        None => self.set_status("HOME not set.", false),
                    }
                } else {
                    let dest = format!("/var/lib/AccountsService/icons/{user}");
                    self.run_priv(
                        &sender,
                        vec!["cp".into(), path.to_string_lossy().into_owned(), dest],
                        "Picture updated",
                    );
                }
            }

            UsersSettingsInput::ChangePassword(user, new, confirm) => {
                if new.is_empty() {
                    self.set_status("Password can't be empty.", false);
                    return;
                }
                if new != confirm {
                    self.set_status("Passwords don't match.", false);
                    return;
                }
                if self.busy {
                    return;
                }
                self.set_busy(true);
                let sender_cmd = sender.clone();
                sender.oneshot_command(async move {
                    let res = set_password(&user, &new).await;
                    let _ = &sender_cmd;
                    match res {
                        Ok(()) => UsersSettingsCommandOutput::OpDone {
                            ok: true,
                            msg: "Password changed.".into(),
                        },
                        Err(e) => UsersSettingsCommandOutput::OpDone { ok: false, msg: e },
                    }
                });
            }

            UsersSettingsInput::AddUser => {
                let name = self.add_entry.text().trim().to_string();
                if !valid_username(&name) {
                    self.set_status(
                        "Enter a valid username (lowercase letters, digits, - or _).",
                        false,
                    );
                    return;
                }
                self.add_entry.set_text("");
                self.run_priv(
                    &sender,
                    vec![
                        "useradd".into(),
                        "-m".into(),
                        "-s".into(),
                        "/bin/bash".into(),
                        name,
                    ],
                    "User created — set a password from their card",
                );
            }

            UsersSettingsInput::RemoveUser(user) => {
                // Guards: not yourself, not the last admin.
                let current = std::env::var("USER").unwrap_or_default();
                if user == current {
                    self.set_status("You can't remove the account you're signed into.", false);
                    return;
                }
                if is_admin(&user) && admin_count() <= 1 {
                    self.set_status("Can't remove the last administrator.", false);
                    return;
                }
                // `userdel` without -r keeps the home directory (safer default).
                self.run_priv(
                    &sender,
                    vec!["userdel".into(), user],
                    "User removed (home kept)",
                );
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
            UsersSettingsCommandOutput::OpDone { ok, msg } => {
                self.set_busy(false);
                self.set_status(&msg, ok);
                if ok {
                    rebuild(&self.list, &sender);
                }
            }
        }
    }
}

impl UsersSettingsModel {
    fn set_status(&self, msg: &str, ok: bool) {
        self.status.set_label(msg);
        self.status.set_visible(!msg.is_empty());
        if ok {
            self.status.remove_css_class("status-error");
        } else {
            self.status.add_css_class("status-error");
        }
    }

    fn set_busy(&mut self, busy: bool) {
        self.busy = busy;
    }

    /// Run a privileged `pkexec <args…>` action, then report + refresh.
    fn run_priv(&mut self, sender: &ComponentSender<Self>, args: Vec<String>, ok_msg: &str) {
        if self.busy {
            return;
        }
        self.set_busy(true);
        self.set_status("Waiting for authorization…", true);
        let ok_msg = ok_msg.to_string();
        sender.oneshot_command(async move {
            match run_pkexec(&args).await {
                Ok(()) => UsersSettingsCommandOutput::OpDone {
                    ok: true,
                    msg: ok_msg,
                },
                Err(e) => UsersSettingsCommandOutput::OpDone { ok: false, msg: e },
            }
        });
    }
}

// ── Privileged helpers ───────────────────────────────────────────────────────

/// Run a privileged `<args…>` action. Prefers silent `sudo -n`, falls back to
/// the polkit agent — see [`crate::sys::privileged`].
async fn run_pkexec(args: &[String]) -> Result<(), String> {
    let argv: Vec<&str> = args.iter().map(String::as_str).collect();
    crate::sys::privileged::run(&argv).await
}

/// Set a user's password via `chpasswd` (password fed on stdin so it never
/// lands in the process arguments).
async fn set_password(user: &str, password: &str) -> Result<(), String> {
    let line = format!("{user}:{password}\n");
    crate::sys::privileged::run_with_stdin(&["chpasswd"], line.as_bytes()).await
}

// ── UI building ──────────────────────────────────────────────────────────────

fn rebuild(list: &gtk::Box, sender: &ComponentSender<UsersSettingsModel>) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    for u in read_users() {
        list.append(&user_card(&u, sender));
    }
}

fn user_card(u: &UserInfo, sender: &ComponentSender<UsersSettingsModel>) -> gtk::Box {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
    card.add_css_class("ok-button-surface");
    card.add_css_class("ok-button-cell");

    // ── Header row (avatar + name + "You" chip) ──
    let expander = gtk::Expander::new(None);
    expander.set_expanded(u.is_current);

    let header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let avatar = match u.avatar.as_deref().filter(|p| p.exists()) {
        Some(path) => gtk::Image::from_file(path),
        None => gtk::Image::from_icon_name("avatar-default-symbolic"),
    };
    avatar.set_pixel_size(40);
    avatar.set_valign(gtk::Align::Center);
    header.append(&avatar);

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
    let role = gtk::Label::new(Some(if u.admin {
        "Administrator"
    } else {
        "Standard user"
    }));
    role.add_css_class("label-small");
    role.set_halign(gtk::Align::Start);
    role.set_xalign(0.0);
    text.append(&role);
    header.append(&text);
    if u.is_current {
        let chip = gtk::Label::new(Some("You"));
        chip.add_css_class("label-small-bold");
        chip.set_valign(gtk::Align::Center);
        header.append(&chip);
    }
    expander.set_label_widget(Some(&header));
    card.append(&expander);

    // ── Detail controls (inside the expander) ──
    let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
    body.set_margin_top(12);
    body.set_margin_start(4);
    body.set_margin_end(4);

    // Change picture
    {
        let btn = gtk::Button::with_label("Change picture…");
        btn.add_css_class("ok-button-surface");
        btn.set_halign(gtk::Align::Start);
        let s = sender.clone();
        let user = u.name.clone();
        let is_current = u.is_current;
        btn.connect_clicked(move |_| {
            s.input(UsersSettingsInput::PickAvatar(user.clone(), is_current));
        });
        body.append(&btn);
    }

    // Full name
    {
        let row = labeled_row("Full name");
        let entry = gtk::Entry::new();
        entry.set_text(&u.full_name);
        entry.set_hexpand(true);
        let save = gtk::Button::with_label("Save");
        save.add_css_class("ok-button-surface");
        let s = sender.clone();
        let user = u.name.clone();
        let entry_c = entry.clone();
        save.connect_clicked(move |_| {
            s.input(UsersSettingsInput::SaveFullName(
                user.clone(),
                entry_c.text().trim().to_string(),
            ));
        });
        row.append(&entry);
        row.append(&save);
        body.append(&row);
    }

    // Account type (Administrator switch)
    {
        let row = labeled_row("Administrator");
        let sw = gtk::Switch::new();
        sw.set_valign(gtk::Align::Center);
        sw.set_halign(gtk::Align::End);
        sw.set_hexpand(true);
        sw.set_active(u.admin); // set before connecting → no spurious signal
        let s = sender.clone();
        let user = u.name.clone();
        sw.connect_state_set(move |_, state| {
            s.input(UsersSettingsInput::SetAdmin(user.clone(), state));
            glib::Propagation::Proceed
        });
        row.append(&sw);
        body.append(&row);
    }

    // Change password
    {
        let new = gtk::PasswordEntry::new();
        new.set_show_peek_icon(true);
        new.set_hexpand(true);
        let confirm = gtk::PasswordEntry::new();
        confirm.set_show_peek_icon(true);
        confirm.set_hexpand(true);
        let set = gtk::Button::with_label("Set password");
        set.add_css_class("ok-button-surface");
        let s = sender.clone();
        let user = u.name.clone();
        let new_c = new.clone();
        let confirm_c = confirm.clone();
        set.connect_clicked(move |_| {
            s.input(UsersSettingsInput::ChangePassword(
                user.clone(),
                new_c.text().to_string(),
                confirm_c.text().to_string(),
            ));
            new_c.set_text("");
            confirm_c.set_text("");
        });

        let new_row = labeled_row("New password");
        new_row.append(&new);
        body.append(&new_row);

        let confirm_row = labeled_row("Confirm");
        confirm_row.append(&confirm);
        confirm_row.append(&set);
        body.append(&confirm_row);
    }

    // Remove user (two-click confirm; never for the current user)
    if !u.is_current {
        let btn = gtk::Button::with_label("Remove user…");
        btn.add_css_class("ok-button-surface");
        btn.add_css_class("destructive-action");
        btn.set_halign(gtk::Align::Start);
        let s = sender.clone();
        let user = u.name.clone();
        let armed = std::rc::Rc::new(std::cell::Cell::new(false));
        btn.connect_clicked(move |b| {
            if armed.get() {
                s.input(UsersSettingsInput::RemoveUser(user.clone()));
            } else {
                armed.set(true);
                b.set_label("Click again to remove");
            }
        });
        body.append(&btn);
    }

    expander.set_child(Some(&body));
    card
}

/// A `[label][…]` horizontal row with a fixed-width caption.
fn labeled_row(caption: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let label = gtk::Label::new(Some(caption));
    label.add_css_class("label-small");
    label.set_width_chars(12);
    label.set_xalign(0.0);
    row.append(&label);
    row
}

// ── System reads ─────────────────────────────────────────────────────────────

fn valid_username(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 32
        && name.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

fn is_admin(user: &str) -> bool {
    admin_members().iter().any(|m| m == user)
}

fn admin_count() -> usize {
    // Distinct human accounts that are admins.
    read_users().iter().filter(|u| u.admin).count()
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
    if is_current && let Some(home) = std::env::var_os("HOME") {
        let face = PathBuf::from(home).join(".face");
        if face.exists() {
            return Some(face);
        }
    }
    let asvc = PathBuf::from(format!("/var/lib/AccountsService/icons/{name}"));
    asvc.exists().then_some(asvc)
}
