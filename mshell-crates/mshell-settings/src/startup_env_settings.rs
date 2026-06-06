//! Settings → Startup & Environment. Two list editors over margo's
//! `exec = …` (startup commands) and `env = KEY, VALUE` lines in `config.conf`.

use crate::compositor_conf::{read_block, write_block};
use crate::row::Row;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};

#[derive(Debug)]
pub(crate) enum StartupEnvInput {
    SetExec(String),
    AddExec,
    RemoveExec(usize),
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
    exec_rules: Vec<String>,
    env_rules: Vec<String>,
    exec_list: gtk::ListBox,
    env_list: gtk::ListBox,
    f_exec: String,
    f_env_key: String,
    f_env_val: String,
}

fn rebuild(
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
        // Break mid-token (regex / paths have no spaces) + report the
        // wrapped width as natural, so a long payload doesn't force the
        // Settings panel wider than its configured size on this page.
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
    rebuild(
        &model.exec_list,
        &model.exec_rules,
        "No startup commands yet.",
        StartupEnvInput::RemoveExec,
        sender,
    );
    rebuild(
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
                        gtk::Label { add_css_class: "settings-hero-title", set_label: "Startup & Environment", set_halign: gtk::Align::Start },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "Commands run when margo starts, and environment variables exported to the session. Take effect on the next start (reload re-reads, but env/exec only apply at launch).",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                gtk::Label { add_css_class: "label-large-bold", set_label: "Startup commands", set_halign: gtk::Align::Start },

                #[template] Row {
                    #[template_child] title { set_label: "Command" },
                    gtk::Entry {
                        set_valign: gtk::Align::Center,
                        set_hexpand: true,
                        set_placeholder_text: Some("e.g. wl-paste --watch cliphist store"),
                        connect_changed[sender] => move |e| sender.input(StartupEnvInput::SetExec(e.text().to_string())),
                    },
                },
                gtk::Button {
                    set_halign: gtk::Align::Start,
                    add_css_class: "suggested-action",
                    set_label: "Add command",
                    connect_clicked[sender] => move |_| sender.input(StartupEnvInput::AddExec),
                },
                #[local_ref]
                exec_list -> gtk::ListBox {
                    add_css_class: "boxed-list",
                    set_selection_mode: gtk::SelectionMode::None,
                },

                gtk::Label { add_css_class: "label-large-bold", set_label: "Environment variables", set_halign: gtk::Align::Start, set_margin_top: 8 },

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
                gtk::Button {
                    set_halign: gtk::Align::Start,
                    add_css_class: "suggested-action",
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
        let model = StartupEnvModel {
            exec_rules: read_block("exec"),
            env_rules: read_block("env"),
            exec_list: gtk::ListBox::new(),
            env_list: gtk::ListBox::new(),
            f_exec: String::new(),
            f_env_key: String::new(),
            f_env_val: String::new(),
        };
        let exec_list = model.exec_list.clone();
        let env_list = model.env_list.clone();
        let widgets = view_output!();
        rebuild_all(&model, &sender);
        let _ = root;
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            StartupEnvInput::SetExec(v) => self.f_exec = v,
            StartupEnvInput::SetEnvKey(v) => self.f_env_key = v,
            StartupEnvInput::SetEnvVal(v) => self.f_env_val = v,
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
