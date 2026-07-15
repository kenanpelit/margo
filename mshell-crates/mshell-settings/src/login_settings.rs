//! Settings → Login Screen: a GUI over mlogind's `/etc/mlogind/config.toml`.
//!
//! The greeter knobs phase D added (background directory, admin CSS, OSK,
//! autologin, blank timeout) plus the host ladder — previously "edit a
//! root-owned TOML by hand". The page reads the file directly (it is 0644)
//! and writes through `sudo -n install` because it is root's: a login
//! manager's config must never be writable by an unprivileged user, or any
//! user could autologin themselves as anyone.
//!
//! Privilege policy is the DNS widget's, verbatim: passwordless `sudo -n`,
//! else an askpass via `sudo -A`, else a toast and nothing happens. NEVER
//! pkexec — the Settings panel holds an exclusive keyboard grab, so a polkit
//! dialog can never receive the password and the shell appears frozen.
//!
//! mlogind parses its config once, at daemon start, and the daemon is the
//! running session's ancestor — restarting it would kill the desktop. So the
//! page promises exactly what is true: changes apply from the next boot.

use relm4::gtk::glib;
use relm4::gtk::prelude::{BoxExt, ButtonExt, EditableExt, EntryExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

use crate::mlogind_conf::{self, Edit, LoginConf, Value};

const HOSTS: [&str; 3] = ["gui", "cage", "tty"];

#[derive(Debug)]
pub(crate) struct LoginSettingsModel {
    conf: LoginConf,
    /// Dropdown catalogues, index-aligned with the GTK `StringList`s.
    users: Vec<String>,
    sessions: Vec<String>,
    users_list: gtk::StringList,
    sessions_list: gtk::StringList,
    hosts_list: gtk::StringList,
    user_index: u32,
    session_index: u32,
    host_index: u32,
    autologin_enabled: bool,
    /// `/etc/pam.d/mlogind-autologin` exists — without it autologin fails
    /// (safely, to the greeter) at boot, so warn up front.
    pam_ok: bool,
    dirty: bool,
    busy: bool,
    status: String,
}

#[derive(Debug)]
pub(crate) enum LoginSettingsInput {
    BackgroundDirChanged(String),
    GreeterCssChanged(String),
    OskChanged(bool),
    BlankTimeoutChanged(u32),
    HostSelected(u32),
    AutologinEnabledChanged(bool),
    AutologinUserSelected(u32),
    AutologinSessionSelected(u32),
    Apply,
}

#[derive(Debug)]
pub(crate) enum LoginSettingsOutput {}

pub(crate) struct LoginSettingsInit {}

#[derive(Debug)]
pub(crate) enum LoginSettingsCommandOutput {
    Applied(Result<(), String>),
}

#[relm4::component(pub)]
impl Component for LoginSettingsModel {
    type CommandOutput = LoginSettingsCommandOutput;
    type Input = LoginSettingsInput;
    type Output = LoginSettingsOutput;
    type Init = LoginSettingsInit;

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
                            set_label: "Login Screen",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "The mlogind greeter — what the machine shows before anyone is logged in. Saving needs admin rights; changes take effect at the next boot.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                // ── Appearance ─────────────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Appearance",
                    set_halign: gtk::Align::Start,
                },

                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    gtk::Box {
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,

                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_halign: gtk::Align::Start,
                                set_label: "Background directory",
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "A folder of images; every login screen draws one at random, and the compositor wallpaper matches it. Empty shows a blurred copy of your desktop wallpaper.",
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },

                        gtk::Entry {
                            set_valign: gtk::Align::Center,
                            set_width_chars: 22,
                            set_placeholder_text: Some("/path/to/photos"),
                            set_text: &model.conf.background_dir,
                            connect_changed[sender] => move |e| {
                                sender.input(LoginSettingsInput::BackgroundDirChanged(e.text().to_string()));
                            },
                        },
                    },

                    gtk::Box {
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,

                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_halign: gtk::Align::Start,
                                set_label: "Theme CSS",
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "Absolute path to a CSS file layered over the greeter's palette (e.g. :root { --primary: #ff5555; }). Wins over the wallpaper theme.",
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },

                        gtk::Entry {
                            set_valign: gtk::Align::Center,
                            set_width_chars: 22,
                            set_placeholder_text: Some("/etc/mlogind/theme.css"),
                            set_text: &model.conf.greeter_css,
                            connect_changed[sender] => move |e| {
                                sender.input(LoginSettingsInput::GreeterCssChanged(e.text().to_string()));
                            },
                        },
                    },

                    gtk::Box {
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,

                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_halign: gtk::Align::Start,
                                set_label: "Blank after (seconds)",
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "Idle time before the login screen goes black and forgets a half-typed password. 0 never blanks.",
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },

                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_range: (0.0, 3600.0),
                            set_increments: (10.0, 60.0),
                            set_digits: 0,
                            set_value: model.conf.blank_timeout as f64,
                            connect_value_changed[sender] => move |s| {
                                sender.input(LoginSettingsInput::BlankTimeoutChanged(s.value() as u32));
                            },
                        },
                    },

                    gtk::Box {
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,

                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_halign: gtk::Align::Start,
                                set_label: "On-screen keyboard",
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "Float the mkeys virtual keyboard over the login card, for touch login.",
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },

                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            set_active: model.conf.osk,
                            connect_state_set[sender] => move |_, on| {
                                sender.input(LoginSettingsInput::OskChanged(on));
                                glib::Propagation::Proceed
                            },
                        },
                    },

                    gtk::Box {
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,

                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_halign: gtk::Align::Start,
                                set_label: "Greeter host",
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "gui — a login card on every monitor (recommended). cage — a kiosk terminal. tty — the plain console form. A broken host falls back to the next one on its own.",
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },

                        gtk::DropDown {
                            set_valign: gtk::Align::Center,
                            set_model: Some(&model.hosts_list),
                            set_selected: model.host_index,
                            connect_selected_notify[sender] => move |dd| {
                                sender.input(LoginSettingsInput::HostSelected(dd.selected()));
                            },
                        },
                    },
                },

                // ── Autologin ──────────────────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Automatic Login",
                    set_halign: gtk::Align::Start,
                },

                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    gtk::Box {
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,

                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_halign: gtk::Align::Start,
                                set_label: "Enabled",
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "Log this user in once per boot, with no login screen. Logging out still shows the greeter — it never loops back in.",
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                            },
                        },

                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            set_active: model.autologin_enabled,
                            connect_state_set[sender] => move |_, on| {
                                sender.input(LoginSettingsInput::AutologinEnabledChanged(on));
                                glib::Propagation::Proceed
                            },
                        },
                    },

                    gtk::Box {
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        #[watch]
                        set_sensitive: model.autologin_enabled,

                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_hexpand: true,
                            set_label: "User",
                        },

                        gtk::DropDown {
                            set_valign: gtk::Align::Center,
                            set_model: Some(&model.users_list),
                            set_selected: model.user_index,
                            connect_selected_notify[sender] => move |dd| {
                                sender.input(LoginSettingsInput::AutologinUserSelected(dd.selected()));
                            },
                        },
                    },

                    gtk::Box {
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        #[watch]
                        set_sensitive: model.autologin_enabled,

                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_hexpand: true,
                            set_label: "Session",
                        },

                        gtk::DropDown {
                            set_valign: gtk::Align::Center,
                            set_model: Some(&model.sessions_list),
                            set_selected: model.session_index,
                            connect_selected_notify[sender] => move |dd| {
                                sender.input(LoginSettingsInput::AutologinSessionSelected(dd.selected()));
                            },
                        },
                    },

                    gtk::Box {
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_visible: !model.pam_ok,

                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_label: "⚠ /etc/pam.d/mlogind-autologin is missing — autologin will fail (safely, to the login screen) until the margo package reinstalls it.",
                        },
                    },
                },

                // ── Apply ──────────────────────────────────────
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 12,

                    gtk::Label {
                        add_css_class: "label-small",
                        set_halign: gtk::Align::Start,
                        set_xalign: 0.0,
                        set_hexpand: true,
                        set_wrap: true,
                        #[watch]
                        set_label: &model.status,
                    },

                    gtk::Button {
                        add_css_class: "ok-button-primary",
                        set_valign: gtk::Align::Center,
                        set_halign: gtk::Align::End,
                        #[watch]
                        set_sensitive: model.dirty && !model.busy,
                        #[watch]
                        set_label: if model.busy { "Saving…" } else { "Save" },
                        connect_clicked[sender] => move |_| {
                            sender.input(LoginSettingsInput::Apply);
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
        let conf = mlogind_conf::load();

        let mut users = login_users();
        ensure_listed(&mut users, &conf.autologin_user);
        if users.is_empty() {
            // A machine with no regular users still needs a row to select.
            users.push(whoami());
        }
        let mut sessions = wayland_sessions();
        ensure_listed(&mut sessions, &conf.autologin_session);
        if sessions.is_empty() {
            sessions.push("Margo (UWSM)".to_string());
        }

        let default_user = whoami();
        let user_current = if conf.autologin_user.is_empty() {
            default_user.as_str()
        } else {
            conf.autologin_user.as_str()
        };
        let user_index = index_of(&users, user_current);
        let session_index = index_of(&sessions, &conf.autologin_session);
        let host_index = HOSTS.iter().position(|h| *h == conf.host).unwrap_or(0) as u32;

        let autologin_enabled =
            !conf.autologin_user.is_empty() && !conf.autologin_session.is_empty();

        let model = LoginSettingsModel {
            users_list: string_list(&users),
            sessions_list: string_list(&sessions),
            hosts_list: string_list(&HOSTS.map(str::to_string)),
            users,
            sessions,
            user_index,
            session_index,
            host_index,
            autologin_enabled,
            pam_ok: std::path::Path::new("/etc/pam.d/mlogind-autologin").is_file(),
            conf,
            dirty: false,
            busy: false,
            status: String::new(),
        };
        let widgets = view_output!();
        let _ = sender; // moved into the view handlers above

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            LoginSettingsInput::BackgroundDirChanged(v) => {
                self.conf.background_dir = v;
                self.touch();
            }
            LoginSettingsInput::GreeterCssChanged(v) => {
                self.conf.greeter_css = v;
                self.touch();
            }
            LoginSettingsInput::OskChanged(v) => {
                self.conf.osk = v;
                self.touch();
            }
            LoginSettingsInput::BlankTimeoutChanged(v) => {
                self.conf.blank_timeout = v;
                self.touch();
            }
            LoginSettingsInput::HostSelected(i) => {
                self.host_index = i.min(HOSTS.len() as u32 - 1);
                self.touch();
            }
            LoginSettingsInput::AutologinEnabledChanged(v) => {
                self.autologin_enabled = v;
                self.touch();
            }
            LoginSettingsInput::AutologinUserSelected(i) => {
                self.user_index = i;
                self.touch();
            }
            LoginSettingsInput::AutologinSessionSelected(i) => {
                self.session_index = i;
                self.touch();
            }
            LoginSettingsInput::Apply => {
                if self.busy {
                    return;
                }
                self.busy = true;
                self.status = String::new();
                let edits = self.edits();
                sender.oneshot_command(async move {
                    LoginSettingsCommandOutput::Applied(apply_to_disk(edits).await)
                });
            }
        }
    }

    fn update_cmd(
        &mut self,
        message: Self::CommandOutput,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        let LoginSettingsCommandOutput::Applied(result) = message;
        self.busy = false;
        match result {
            Ok(()) => {
                self.dirty = false;
                self.status = "Saved. Changes take effect at the next boot.".to_string();
            }
            Err(err) => {
                self.status = format!("Could not save: {err}");
                mshell_launcher::notify::toast("Login Screen", &self.status);
            }
        }
    }
}

impl LoginSettingsModel {
    fn touch(&mut self) {
        self.dirty = true;
        self.status.clear();
    }

    /// The full managed-key set, always written together — idempotent, and a
    /// hand-deleted key comes back rather than silently diverging from what
    /// the page shows.
    fn edits(&self) -> Vec<Edit> {
        let user = if self.autologin_enabled {
            self.users
                .get(self.user_index as usize)
                .cloned()
                .unwrap_or_default()
        } else {
            // Disabling clears the user; the session survives as a
            // convenience for the next time the switch comes back on.
            String::new()
        };
        let session = self
            .sessions
            .get(self.session_index as usize)
            .cloned()
            .unwrap_or_default();
        let host = HOSTS
            .get(self.host_index as usize)
            .copied()
            .unwrap_or("gui");

        vec![
            Edit {
                section: "display",
                key: "host",
                value: Value::Str(host.to_string()),
            },
            Edit {
                section: "display",
                key: "background_dir",
                value: Value::Str(self.conf.background_dir.trim().to_string()),
            },
            Edit {
                section: "display",
                key: "greeter_css",
                value: Value::Str(self.conf.greeter_css.trim().to_string()),
            },
            Edit {
                section: "display",
                key: "osk",
                value: Value::Bool(self.conf.osk),
            },
            Edit {
                section: "display",
                key: "blank_timeout",
                value: Value::Int(i64::from(self.conf.blank_timeout)),
            },
            Edit {
                section: "autologin",
                key: "user",
                value: Value::Str(user),
            },
            Edit {
                section: "autologin",
                key: "session",
                value: Value::Str(session),
            },
        ]
    }
}

/// Rewrite `/etc/mlogind/config.toml` with `edits` applied, as root.
///
/// The edit happens in user space (read → surgical patch → temp file); only
/// the final `install -m644` runs privileged, so the sudo surface is one
/// fixed argv, not file content squeezed through a shell.
async fn apply_to_disk(edits: Vec<Edit>) -> Result<(), String> {
    let current = tokio::fs::read_to_string(mlogind_conf::CONFIG_PATH)
        .await
        .unwrap_or_default();
    let next = mlogind_conf::apply_edits(&current, &edits);

    let tmp = std::env::temp_dir().join(format!("mshell-mlogind-{}.toml", std::process::id()));
    tokio::fs::write(&tmp, &next)
        .await
        .map_err(|e| format!("temp file: {e}"))?;

    let tmp_str = tmp.to_string_lossy().into_owned();
    let result = run_privileged(&["install", "-m644", &tmp_str, mlogind_conf::CONFIG_PATH]).await;
    let _ = tokio::fs::remove_file(&tmp).await;
    result
}

/// The DNS widget's privilege policy (see `dns_menu_widget.rs`): passwordless
/// `sudo -n`, else `sudo -A` with an askpass, else a clear failure. Never
/// pkexec — its dialog can't take keyboard under the panel's exclusive grab.
async fn run_privileged(args: &[&str]) -> Result<(), String> {
    let have_sudo_n = tokio::process::Command::new("sudo")
        .args(["-n", "true"])
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false);

    let mut cmd = tokio::process::Command::new("sudo");
    if have_sudo_n {
        cmd.arg("-n");
    } else if let Some(askpass) = askpass_helper() {
        cmd.arg("-A");
        cmd.env("SUDO_ASKPASS", askpass);
    } else {
        return Err("needs passwordless sudo (or an askpass helper on PATH)".to_string());
    }

    let status = cmd
        .args(args)
        .status()
        .await
        .map_err(|e| format!("sudo spawn: {e}"))?;
    if !status.success() {
        return Err(format!("sudo {} exited {status}", args.join(" ")));
    }
    Ok(())
}

fn askpass_helper() -> Option<std::ffi::OsString> {
    if let Some(ap) = std::env::var_os("SUDO_ASKPASS") {
        return Some(ap);
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join("askpass"))
        .find(|p| p.is_file())
        .map(|p| p.into_os_string())
}

fn string_list(items: &[String]) -> gtk::StringList {
    let refs: Vec<&str> = items.iter().map(String::as_str).collect();
    gtk::StringList::new(&refs)
}

fn index_of(items: &[String], value: &str) -> u32 {
    items.iter().position(|i| i == value).unwrap_or(0) as u32
}

/// Keep whatever the config currently names selectable, even if it is not in
/// the enumerated list — saving must not silently rewrite a value just
/// because the page could not list it.
fn ensure_listed(items: &mut Vec<String>, current: &str) {
    if !current.is_empty() && !items.iter().any(|i| i == current) {
        items.push(current.to_string());
    }
}

fn whoami() -> String {
    std::env::var("USER").unwrap_or_else(|_| "root".to_string())
}

/// Regular login accounts from `/etc/passwd`: human uid range, a real shell.
fn login_users() -> Vec<String> {
    let Ok(text) = std::fs::read_to_string("/etc/passwd") else {
        return Vec::new();
    };
    let mut users: Vec<String> = text.lines().filter_map(parse_passwd_line).collect();
    users.sort();
    users
}

fn parse_passwd_line(line: &str) -> Option<String> {
    let mut fields = line.split(':');
    let name = fields.next()?;
    let _password = fields.next()?;
    let uid: u32 = fields.next()?.parse().ok()?;
    let _gid = fields.next()?;
    let _gecos = fields.next()?;
    let _home = fields.next()?;
    let shell = fields.next().unwrap_or("");
    ((1000..60000).contains(&uid) && !shell.ends_with("nologin") && !shell.ends_with("false"))
        .then(|| name.to_string())
}

/// The session names mlogind's switcher offers, in its order — `Request::…`
/// and `[autologin] session` both resolve against this list. `mlogind envs`
/// derives it from the same config + session dirs the daemon uses, so asking
/// the binary beats re-implementing its scan.
fn wayland_sessions() -> Vec<String> {
    let Ok(output) = std::process::Command::new("mlogind").arg("envs").output() else {
        return Vec::new();
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwd_filtering_keeps_humans_and_drops_daemons() {
        assert_eq!(
            parse_passwd_line("kenan:x:1000:1000::/home/kenan:/bin/zsh"),
            Some("kenan".to_string())
        );
        assert_eq!(parse_passwd_line("root:x:0:0::/root:/bin/bash"), None);
        assert_eq!(
            parse_passwd_line("mlogind-greeter:x:937:937::/:/usr/bin/nologin"),
            None
        );
        assert_eq!(
            parse_passwd_line("nobody:x:65534:65534::/:/usr/bin/nologin"),
            None
        );
        assert_eq!(parse_passwd_line("garbage"), None);
    }

    #[test]
    fn the_current_config_value_stays_selectable() {
        let mut items = vec!["Margo (UWSM)".to_string()];
        ensure_listed(&mut items, "Sway");
        assert!(items.iter().any(|i| i == "Sway"));
        // Empty and already-listed values add nothing.
        ensure_listed(&mut items, "");
        ensure_listed(&mut items, "Sway");
        assert_eq!(items.len(), 2);
    }
}
