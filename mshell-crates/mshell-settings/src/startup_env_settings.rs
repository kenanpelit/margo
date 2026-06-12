//! Settings → Startup. The one place for everything that runs when you log in:
//!
//! 1. **Autostart scripts** — the shell's `>start` scripts, each with a
//!    run-at-startup toggle, a post-login delay, an Every-start / Login-only
//!    trigger, extra arguments, a working directory, drag-to-reorder, a
//!    Run-now button, and a found/missing-on-`$PATH` badge. Round-tripped
//!    through `config.launcher.autostart_scripts` (the same list the launcher's
//!    `>start` provider and the boot runner read).
//! 2. **Startup commands** — margo's `exec = …` lines in `config.conf`.
//! 3. **Environment variables** — margo's `env = KEY, VALUE` lines.

use std::collections::BTreeSet;
use std::rc::Rc;

use crate::compositor_conf::{read_block, write_block};
use crate::reorder_dnd::attach_grip_drag;
use crate::row::Row;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    AutostartTrigger, ConfigStoreFields, LauncherStoreFields, ScriptAutostart,
};
use reactive_graph::traits::GetUntracked;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};

#[derive(Debug)]
pub(crate) enum StartupEnvInput {
    // Autostart scripts
    AddScript,
    RemoveScript(String),
    SetAutostart(String, bool),
    SetDelay(String, u32),
    SetTrigger(String, u32),
    SetArgs(String, String),
    SetCwd(String, String),
    RunNow(String),
    MoveScript(usize, i32),
    // Startup commands (`exec`)
    SetExec(String),
    AddExec,
    RemoveExec(usize),
    // Environment (`env`)
    SetEnvKey(String),
    SetEnvVal(String),
    AddEnv,
    RemoveEnv(usize),
}

#[derive(Debug)]
pub(crate) enum StartupEnvOutput {}
#[derive(Debug)]
pub(crate) enum StartupEnvCommandOutput {}
pub(crate) struct StartupEnvInit {}

pub(crate) struct StartupEnvModel {
    scripts: Vec<ScriptAutostart>,
    scripts_box: gtk::Box,
    /// Executable names on `$PATH` — for the per-row found/missing badge.
    /// Snapshotted once at page open.
    path_exes: Rc<BTreeSet<String>>,
    /// Free-text Add field: a `start-*` script name or a full command.
    script_entry: gtk::Entry,
    exec_rules: Vec<String>,
    env_rules: Vec<String>,
    exec_list: gtk::ListBox,
    env_list: gtk::ListBox,
    f_exec: String,
    f_env_key: String,
    f_env_val: String,
}

/// Snapshot the user's autostart-script list from config.
fn read_autostart_scripts() -> Vec<ScriptAutostart> {
    config_manager()
        .config()
        .launcher()
        .autostart_scripts()
        .get_untracked()
}

/// Find-or-create the entry for `name`, apply `mutate`, persist.
fn upsert_autostart(name: &str, mutate: impl FnOnce(&mut ScriptAutostart)) {
    config_manager().update_config(|config| {
        if let Some(entry) = config
            .launcher
            .autostart_scripts
            .iter_mut()
            .find(|e| e.name == name)
        {
            mutate(entry);
        } else {
            let mut entry = ScriptAutostart {
                name: name.to_string(),
                enabled: true,
                delay_secs: 0,
                trigger: AutostartTrigger::LoginOnce,
                args: String::new(),
                working_dir: String::new(),
            };
            mutate(&mut entry);
            config.launcher.autostart_scripts.push(entry);
        }
    });
}

/// Every executable name reachable on `$PATH`, deduped + sorted.
fn scan_path_exes() -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let Some(path) = std::env::var_os("PATH") else {
        return out;
    };
    for dir in std::env::split_paths(&path) {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in rd.flatten() {
            if let Ok(ft) = entry.file_type()
                && (ft.is_file() || ft.is_symlink())
                && let Ok(name) = entry.file_name().into_string()
            {
                out.insert(name);
            }
        }
    }
    out
}

/// Expand a leading `~` / `~/` in a working-dir string. `None` for empty.
fn expand_cwd(dir: &str) -> Option<String> {
    let dir = dir.trim();
    if dir.is_empty() {
        return None;
    }
    Some(if let Some(rest) = dir.strip_prefix("~/") {
        std::env::var("HOME")
            .map(|h| format!("{h}/{rest}"))
            .unwrap_or_else(|_| dir.to_string())
    } else if dir == "~" {
        std::env::var("HOME").unwrap_or_else(|_| dir.to_string())
    } else {
        dir.to_string()
    })
}

/// The shell command line for an entry: command/script name + optional args.
/// Run through `sh -c`, so it can be a bare `start-*` script or a full command
/// (pipes, `&&`, quotes). Mirrors `mshell-core`'s autostart runner.
fn entry_cmdline(entry: &ScriptAutostart) -> String {
    let mut line = entry.name.trim().to_string();
    let args = entry.args.trim();
    if !args.is_empty() {
        line.push(' ');
        line.push_str(args);
    }
    line
}

/// Run an entry now (Run button), via `sh -c` in a transient `systemd --user`
/// *scope* — detached from mshell's cgroup, so the test instance behaves like a
/// real autostart (a wrapper that launches an app and exits leaves it alive,
/// and a mshell restart won't kill it). Falls back to a direct `sh -c`.
fn spawn_script(entry: &ScriptAutostart) {
    let dir = expand_cwd(&entry.working_dir);
    let cmdline = entry_cmdline(entry);

    let mut sd = std::process::Command::new("systemd-run");
    sd.arg("--user")
        .arg("--scope")
        .arg("--quiet")
        .arg("--collect");
    if let Some(d) = &dir {
        sd.current_dir(d);
    }
    sd.arg("--").arg("sh").arg("-c").arg(&cmdline);
    if sd.spawn().is_ok() {
        mshell_launcher::notify::toast("Started", &entry.name);
        return;
    }

    // No systemd-run: direct `sh -c` (won't survive a mshell restart).
    let mut cmd = std::process::Command::new("sh");
    cmd.arg("-c").arg(&cmdline);
    if let Some(d) = &dir {
        cmd.current_dir(d);
    }
    match cmd.spawn() {
        Ok(_) => mshell_launcher::notify::toast("Started", &entry.name),
        Err(e) => mshell_launcher::notify::toast("Failed to start", format!("{}: {e}", entry.name)),
    }
}

/// Repaint the autostart-script list — one card per entry.
fn rebuild_scripts(
    scripts_box: &gtk::Box,
    scripts: &[ScriptAutostart],
    path_exes: &BTreeSet<String>,
    sender: &ComponentSender<StartupEnvModel>,
) {
    while let Some(child) = scripts_box.first_child() {
        scripts_box.remove(&child);
    }
    if scripts.is_empty() {
        let empty = gtk::Label::builder()
            .label("No autostart scripts yet. Add one below.")
            .halign(gtk::Align::Start)
            .xalign(0.0)
            .wrap(true)
            .build();
        empty.add_css_class("label-small");
        scripts_box.append(&empty);
        return;
    }

    for (i, entry) in scripts.iter().enumerate() {
        let name = entry.name.clone();

        let card = gtk::Box::new(gtk::Orientation::Vertical, 4);
        card.add_css_class("launcher-script-row");

        // ── Line 1: grip · name · status · run · enable · remove ──
        let line1 = gtk::Box::new(gtk::Orientation::Horizontal, 8);

        let grip = gtk::Image::from_icon_name("list-drag-handle-symbolic");
        grip.set_tooltip_text(Some("Drag to reorder"));
        line1.append(&grip);

        let label = gtk::Label::builder()
            .label(&name)
            .halign(gtk::Align::Start)
            .hexpand(true)
            .xalign(0.0)
            .build();
        label.add_css_class("label-medium");
        line1.append(&label);

        // Read-only status (not a button): flag an entry whose first word
        // doesn't resolve to an executable on $PATH — a typo, or one that
        // isn't installed yet. Resolved entries show nothing. We check only
        // the first token so full commands (`foo --bar`, `sh -c '…'`) work.
        let first_token = name.split_whitespace().next().unwrap_or("");
        if !path_exes.contains(first_token) {
            let badge = gtk::Label::new(Some("not on PATH"));
            badge.add_css_class("label-small");
            badge.add_css_class("startup-script-missing");
            badge.set_valign(gtk::Align::Center);
            badge.set_tooltip_text(Some(
                "This name doesn't match any executable on $PATH — check the spelling, or install it.",
            ));
            line1.append(&badge);
        }

        let run = gtk::Button::from_icon_name("media-playback-start-symbolic");
        run.add_css_class("flat");
        run.set_valign(gtk::Align::Center);
        run.set_tooltip_text(Some("Run now"));
        {
            let s = sender.clone();
            let n = name.clone();
            run.connect_clicked(move |_| s.input(StartupEnvInput::RunNow(n.clone())));
        }
        line1.append(&run);

        let enable = gtk::Switch::new();
        enable.set_valign(gtk::Align::Center);
        enable.set_tooltip_text(Some("Run at startup"));
        enable.set_active(entry.enabled);
        {
            let s = sender.clone();
            let n = name.clone();
            enable.connect_active_notify(move |sw| {
                s.input(StartupEnvInput::SetAutostart(n.clone(), sw.is_active()))
            });
        }
        line1.append(&enable);

        let remove = gtk::Button::from_icon_name("user-trash-symbolic");
        remove.add_css_class("flat");
        remove.set_valign(gtk::Align::Center);
        remove.set_tooltip_text(Some("Remove"));
        {
            let s = sender.clone();
            let n = name.clone();
            remove.connect_clicked(move |_| s.input(StartupEnvInput::RemoveScript(n.clone())));
        }
        line1.append(&remove);

        card.append(&line1);

        // ── Line 2: delay · trigger · args · working dir ──
        let line2 = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        line2.set_margin_start(28);

        let after = gtk::Label::new(Some("after"));
        after.add_css_class("label-small");
        let delay = gtk::SpinButton::with_range(0.0, 3600.0, 1.0);
        delay.set_digits(0);
        delay.set_valign(gtk::Align::Center);
        delay.set_tooltip_text(Some("Seconds after startup before this runs"));
        delay.set_value(entry.delay_secs as f64);
        {
            let s = sender.clone();
            let n = name.clone();
            delay.connect_value_changed(move |sp| {
                s.input(StartupEnvInput::SetDelay(
                    n.clone(),
                    sp.value().max(0.0) as u32,
                ))
            });
        }
        let secs = gtk::Label::new(Some("s"));
        secs.add_css_class("label-small");
        line2.append(&after);
        line2.append(&delay);
        line2.append(&secs);

        let trigger = gtk::DropDown::from_strings(&["Every start", "Login only"]);
        trigger.set_valign(gtk::Align::Center);
        trigger.set_tooltip_text(Some(
            "Every start: also on mshell restart. Login only: first start per login.",
        ));
        trigger.set_selected(match entry.trigger {
            AutostartTrigger::EveryStart => 0,
            AutostartTrigger::LoginOnce => 1,
        });
        {
            let s = sender.clone();
            let n = name.clone();
            trigger.connect_selected_notify(move |d| {
                s.input(StartupEnvInput::SetTrigger(n.clone(), d.selected()))
            });
        }
        line2.append(&trigger);

        let args = gtk::Entry::new();
        args.set_hexpand(true);
        args.set_valign(gtk::Align::Center);
        args.set_placeholder_text(Some("arguments (optional)"));
        args.set_text(&entry.args);
        {
            let s = sender.clone();
            let n = name.clone();
            args.connect_changed(move |e| {
                s.input(StartupEnvInput::SetArgs(n.clone(), e.text().to_string()))
            });
        }
        line2.append(&args);

        let cwd = gtk::Entry::new();
        cwd.set_hexpand(true);
        cwd.set_valign(gtk::Align::Center);
        cwd.set_placeholder_text(Some("working dir, e.g. ~/projects (optional)"));
        cwd.set_text(&entry.working_dir);
        {
            let s = sender.clone();
            let n = name.clone();
            cwd.connect_changed(move |e| {
                s.input(StartupEnvInput::SetCwd(n.clone(), e.text().to_string()))
            });
        }
        line2.append(&cwd);

        card.append(&line2);

        // Drag-to-reorder from the grip.
        {
            let s = sender.clone();
            attach_grip_drag(&grip, &card, move |delta| {
                s.input(StartupEnvInput::MoveScript(i, delta));
            });
        }

        scripts_box.append(&card);
    }
}

/// Rebuild a raw-line list (exec / env) into `list_box`.
fn rebuild_raw(
    list_box: &gtk::ListBox,
    rules: &[String],
    empty_msg: &str,
    ctor: fn(usize) -> StartupEnvInput,
    sender: &ComponentSender<StartupEnvModel>,
) {
    while let Some(child) = list_box.first_child() {
        list_box.remove(&child);
    }
    if rules.is_empty() {
        let row = gtk::ListBoxRow::new();
        row.set_selectable(false);
        let lbl = gtk::Label::new(Some(empty_msg));
        lbl.add_css_class("label-small");
        lbl.set_halign(gtk::Align::Start);
        lbl.set_margin_top(8);
        lbl.set_margin_bottom(8);
        lbl.set_margin_start(8);
        row.set_child(Some(&lbl));
        list_box.append(&row);
        return;
    }
    for (i, payload) in rules.iter().enumerate() {
        let row = gtk::ListBoxRow::new();
        row.set_selectable(false);
        let hbox = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        hbox.set_margin_top(6);
        hbox.set_margin_bottom(6);
        hbox.set_margin_start(8);
        hbox.set_margin_end(8);
        let lbl = gtk::Label::new(Some(payload));
        lbl.set_halign(gtk::Align::Start);
        lbl.set_hexpand(true);
        lbl.set_xalign(0.0);
        lbl.set_wrap(true);
        lbl.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        lbl.set_natural_wrap_mode(gtk::NaturalWrapMode::None);
        lbl.set_selectable(true);
        lbl.add_css_class("label-medium");
        hbox.append(&lbl);
        let btn = gtk::Button::from_icon_name("user-trash-symbolic");
        btn.add_css_class("flat");
        btn.set_valign(gtk::Align::Center);
        let s = sender.clone();
        btn.connect_clicked(move |_| s.input(ctor(i)));
        hbox.append(&btn);
        row.set_child(Some(&hbox));
        list_box.append(&row);
    }
}

fn rebuild_all(model: &StartupEnvModel, sender: &ComponentSender<StartupEnvModel>) {
    rebuild_scripts(&model.scripts_box, &model.scripts, &model.path_exes, sender);
    rebuild_raw(
        &model.exec_list,
        &model.exec_rules,
        "No startup commands yet.",
        StartupEnvInput::RemoveExec,
        sender,
    );
    rebuild_raw(
        &model.env_list,
        &model.env_rules,
        "No environment variables set.",
        StartupEnvInput::RemoveEnv,
        sender,
    );
}

#[relm4::component(pub)]
impl Component for StartupEnvModel {
    type CommandOutput = StartupEnvCommandOutput;
    type Input = StartupEnvInput;
    type Output = StartupEnvOutput;
    type Init = StartupEnvInit;

    view! {
        gtk::ScrolledWindow {
            set_vscrollbar_policy: gtk::PolicyType::Automatic,
            set_hscrollbar_policy: gtk::PolicyType::Never,
            set_hexpand: true,
            set_vexpand: true,

            gtk::Box {
                add_css_class: "settings-page",
                set_orientation: gtk::Orientation::Vertical,
                set_hexpand: true,
                set_spacing: 16,

                gtk::Box {
                    add_css_class: "settings-hero",
                    set_spacing: 16,
                    gtk::Image {
                        add_css_class: "settings-hero-icon",
                        set_icon_name: Some("system-run-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label { add_css_class: "settings-hero-title", set_label: "Startup", set_halign: gtk::Align::Start },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "Scripts, commands, and environment variables that run when you log in. Scripts apply on the next shell start; exec / env apply on the next compositor start.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                // ════════ Autostart ════════
                gtk::Label { add_css_class: "label-large-bold", set_label: "Autostart", set_halign: gtk::Align::Start },
                gtk::Label {
                    add_css_class: "label-small",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                    set_label: "A `start-*` script or any command (pipes, &&, quotes — run via sh -c). Per entry: run-at-startup toggle, a delay, Every-start (also on mshell restart) vs Login-only, extra arguments, and a working directory. Drag to reorder, or hit Run to test. The badge flags a name that isn't on $PATH.",
                },

                #[local_ref]
                scripts_box -> gtk::Box {
                    add_css_class: "settings-boxed-list",
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 6,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 8,
                    #[local_ref]
                    script_entry -> gtk::Entry {
                        set_hexpand: true,
                        set_placeholder_text: Some("script or command, e.g. start-foo  /  wl-paste --watch cliphist store"),
                        connect_activate[sender] => move |_| sender.input(StartupEnvInput::AddScript),
                    },
                    gtk::Button {
                        add_css_class: "ok-button-primary",
                        set_label: "Add",
                        connect_clicked[sender] => move |_| sender.input(StartupEnvInput::AddScript),
                    },
                },

                // ════════ Startup commands ════════
                gtk::Label { add_css_class: "label-large-bold", set_label: "Startup commands", set_halign: gtk::Align::Start, set_margin_top: 8 },
                gtk::Label {
                    add_css_class: "label-small",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                    set_label: "Compositor `exec` — run once when margo launches, before the shell. For a delay or a Login-only / Every-start trigger, use Autostart above instead.",
                },

                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    #[template] Row {
                        #[template_child] title { set_label: "Command" },
                        gtk::Entry {
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            set_placeholder_text: Some("e.g. wl-paste --watch cliphist store"),
                            connect_changed[sender] => move |e| sender.input(StartupEnvInput::SetExec(e.text().to_string())),
                        },
                    },
                },
                gtk::Button {
                    set_halign: gtk::Align::Start,
                    add_css_class: "ok-button-primary",
                    set_label: "Add command",
                    connect_clicked[sender] => move |_| sender.input(StartupEnvInput::AddExec),
                },
                #[local_ref]
                exec_list -> gtk::ListBox {
                    add_css_class: "boxed-list",
                    set_selection_mode: gtk::SelectionMode::None,
                },

                // ════════ Environment variables ════════
                gtk::Label { add_css_class: "label-large-bold", set_label: "Environment variables", set_halign: gtk::Align::Start, set_margin_top: 8 },

                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    #[template] Row {
                        #[template_child] title { set_label: "Name" },
                        gtk::Entry {
                            set_valign: gtk::Align::Center,
                            set_placeholder_text: Some("e.g. GDK_BACKEND"),
                            connect_changed[sender] => move |e| sender.input(StartupEnvInput::SetEnvKey(e.text().to_string())),
                        },
                    },
                    #[template] Row {
                        #[template_child] title { set_label: "Value" },
                        gtk::Entry {
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            set_placeholder_text: Some("e.g. wayland,x11"),
                            connect_changed[sender] => move |e| sender.input(StartupEnvInput::SetEnvVal(e.text().to_string())),
                        },
                    },
                },
                gtk::Button {
                    set_halign: gtk::Align::Start,
                    add_css_class: "ok-button-primary",
                    set_label: "Add variable",
                    connect_clicked[sender] => move |_| sender.input(StartupEnvInput::AddEnv),
                },
                #[local_ref]
                env_list -> gtk::ListBox {
                    add_css_class: "boxed-list",
                    set_selection_mode: gtk::SelectionMode::None,
                },
            }
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let path_exes = Rc::new(scan_path_exes());
        let script_entry = gtk::Entry::new();

        let model = StartupEnvModel {
            scripts: read_autostart_scripts(),
            scripts_box: gtk::Box::new(gtk::Orientation::Vertical, 6),
            path_exes,
            script_entry,
            exec_rules: read_block("exec"),
            env_rules: read_block("env"),
            exec_list: gtk::ListBox::new(),
            env_list: gtk::ListBox::new(),
            f_exec: String::new(),
            f_env_key: String::new(),
            f_env_val: String::new(),
        };
        let scripts_box = model.scripts_box.clone();
        let script_entry = model.script_entry.clone();
        let exec_list = model.exec_list.clone();
        let env_list = model.env_list.clone();
        let widgets = view_output!();

        rebuild_all(&model, &sender);
        let _ = root;
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            // ── Autostart scripts ──
            StartupEnvInput::AddScript => {
                let name = self.script_entry.text().trim().to_string();
                if name.is_empty() || self.scripts.iter().any(|e| e.name == name) {
                    return;
                }
                upsert_autostart(&name, |_| {});
                self.script_entry.set_text("");
                self.scripts = read_autostart_scripts();
                rebuild_scripts(&self.scripts_box, &self.scripts, &self.path_exes, &sender);
            }
            StartupEnvInput::RemoveScript(name) => {
                config_manager().update_config(|config| {
                    config.launcher.autostart_scripts.retain(|e| e.name != name);
                });
                self.scripts = read_autostart_scripts();
                rebuild_scripts(&self.scripts_box, &self.scripts, &self.path_exes, &sender);
            }
            StartupEnvInput::SetAutostart(name, enabled) => {
                upsert_autostart(&name, |e| e.enabled = enabled);
                self.scripts = read_autostart_scripts();
            }
            StartupEnvInput::SetDelay(name, secs) => {
                upsert_autostart(&name, |e| e.delay_secs = secs);
                self.scripts = read_autostart_scripts();
            }
            StartupEnvInput::SetTrigger(name, idx) => {
                let trigger = if idx == 1 {
                    AutostartTrigger::LoginOnce
                } else {
                    AutostartTrigger::EveryStart
                };
                upsert_autostart(&name, |e| e.trigger = trigger);
                self.scripts = read_autostart_scripts();
            }
            StartupEnvInput::SetArgs(name, args) => {
                upsert_autostart(&name, |e| e.args = args.trim().to_string());
                self.scripts = read_autostart_scripts();
            }
            StartupEnvInput::SetCwd(name, dir) => {
                upsert_autostart(&name, |e| e.working_dir = dir.trim().to_string());
                self.scripts = read_autostart_scripts();
            }
            StartupEnvInput::RunNow(name) => {
                if let Some(entry) = self.scripts.iter().find(|e| e.name == name) {
                    spawn_script(entry);
                }
            }
            StartupEnvInput::MoveScript(from, delta) => {
                if from >= self.scripts.len() || delta == 0 {
                    return;
                }
                let to = (from as i32 + delta).clamp(0, self.scripts.len() as i32 - 1) as usize;
                if to == from {
                    return;
                }
                let entry = self.scripts.remove(from);
                self.scripts.insert(to, entry);
                let new_order = self.scripts.clone();
                config_manager().update_config(move |config| {
                    config.launcher.autostart_scripts = new_order;
                });
                rebuild_scripts(&self.scripts_box, &self.scripts, &self.path_exes, &sender);
            }
            // ── Startup commands ──
            StartupEnvInput::SetExec(v) => self.f_exec = v,
            StartupEnvInput::AddExec => {
                let cmd = self.f_exec.trim();
                if cmd.is_empty() {
                    return;
                }
                self.exec_rules.push(cmd.to_string());
                write_block("exec", &self.exec_rules);
                rebuild_all(self, &sender);
            }
            StartupEnvInput::RemoveExec(i) => {
                if i < self.exec_rules.len() {
                    self.exec_rules.remove(i);
                    write_block("exec", &self.exec_rules);
                    rebuild_all(self, &sender);
                }
            }
            // ── Environment ──
            StartupEnvInput::SetEnvKey(v) => self.f_env_key = v,
            StartupEnvInput::SetEnvVal(v) => self.f_env_val = v,
            StartupEnvInput::AddEnv => {
                let key = self.f_env_key.trim();
                if key.is_empty() {
                    return;
                }
                self.env_rules
                    .push(format!("{key}, {}", self.f_env_val.trim()));
                write_block("env", &self.env_rules);
                rebuild_all(self, &sender);
            }
            StartupEnvInput::RemoveEnv(i) => {
                if i < self.env_rules.len() {
                    self.env_rules.remove(i);
                    write_block("env", &self.env_rules);
                    rebuild_all(self, &sender);
                }
            }
        }
    }
}
