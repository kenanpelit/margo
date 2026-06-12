//! Settings → Logging. File logging for both the shell (`mshell`) and the
//! compositor (`margo`) — enable + level, applied live.
//!
//! Logs live flat in `~/.local/state/margo/logs` (`mshell-*.log` / `margo-*.log`,
//! last few sessions kept). The shell half writes the mshell YAML config and
//! retunes the running logger in-process; the compositor half patches margo's
//! `config.conf` and `mctl reload` re-applies it live.

use crate::compositor_conf::{read_bool, read_raw, set_and_reload};
use crate::row::Row;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::ConfigStoreFields;
use reactive_graph::traits::GetUntracked;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};

/// The level ladder, lowest → highest verbosity. Index = dropdown position.
const LEVELS: [&str; 5] = ["error", "warn", "info", "debug", "trace"];

fn level_idx(level: &str) -> u32 {
    LEVELS
        .iter()
        .position(|l| l.eq_ignore_ascii_case(level))
        .unwrap_or(2) as u32 // default: info
}

#[derive(Debug)]
pub(crate) enum LoggingInput {
    SetShellEnabled(bool),
    SetShellLevel(u32),
    SetCompEnabled(bool),
    SetCompLevel(u32),
    OpenFolder,
}

#[derive(Debug)]
pub(crate) enum LoggingOutput {}
#[derive(Debug)]
pub(crate) enum LoggingCommandOutput {}
pub(crate) struct LoggingInit {}

pub(crate) struct LoggingModel {
    levels: gtk::StringList,
    shell_enabled: bool,
    shell_level_idx: u32,
    comp_enabled: bool,
    comp_level_idx: u32,
}

#[relm4::component(pub)]
impl Component for LoggingModel {
    type CommandOutput = LoggingCommandOutput;
    type Input = LoggingInput;
    type Output = LoggingOutput;
    type Init = LoggingInit;

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
                        set_icon_name: Some("text-x-generic-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label { add_css_class: "settings-hero-title", set_label: "Logging", set_halign: gtk::Align::Start },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "File logs in ~/.local/state/margo/logs — the last few sessions of each are kept for diagnosis. Level changes apply live.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                gtk::Label { add_css_class: "label-large-bold", set_label: "Shell (mshell)", set_halign: gtk::Align::Start },

                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    #[template] Row {
                        #[template_child] title { set_label: "Write shell log files" },
                        #[template_child] desc { set_label: "mshell-*.log" },
                        gtk::Switch { set_valign: gtk::Align::Center, set_active: model.shell_enabled,
                            connect_active_notify[sender] => move |s| sender.input(LoggingInput::SetShellEnabled(s.is_active())) } },
                    #[template] Row {
                        #[template_child] title { set_label: "Shell log level" },
                        #[template_child] desc { set_label: "debug / trace = deeper diagnostics" },
                        gtk::DropDown { set_valign: gtk::Align::Center, set_width_request: 160,
                            set_model: Some(&model.levels),
                            #[block_signal(shell_level_h)]
                            set_selected: model.shell_level_idx,
                            connect_selected_notify[sender] => move |d| sender.input(LoggingInput::SetShellLevel(d.selected())) @shell_level_h } },
                },

                gtk::Label { add_css_class: "label-large-bold", set_label: "Compositor (margo)", set_halign: gtk::Align::Start, set_margin_top: 8 },

                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    #[template] Row {
                        #[template_child] title { set_label: "Write compositor log files" },
                        #[template_child] desc { set_label: "margo-*.log — applied via mctl reload" },
                        gtk::Switch { set_valign: gtk::Align::Center, set_active: model.comp_enabled,
                            connect_active_notify[sender] => move |s| sender.input(LoggingInput::SetCompEnabled(s.is_active())) } },
                    #[template] Row {
                        #[template_child] title { set_label: "Compositor log level" },
                        #[template_child] desc { set_label: "debug / trace = deeper diagnostics" },
                        gtk::DropDown { set_valign: gtk::Align::Center, set_width_request: 160,
                            set_model: Some(&model.levels),
                            #[block_signal(comp_level_h)]
                            set_selected: model.comp_level_idx,
                            connect_selected_notify[sender] => move |d| sender.input(LoggingInput::SetCompLevel(d.selected())) @comp_level_h } },
                },

                gtk::Button {
                    add_css_class: "ok-button-surface",
                    set_halign: gtk::Align::Start,
                    set_valign: gtk::Align::Center,
                    set_margin_top: 12,
                    set_label: "Open log folder",
                    connect_clicked[sender] => move |_| sender.input(LoggingInput::OpenFolder),
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
        let shell = config_manager().config().logging().get_untracked();
        let model = LoggingModel {
            levels: gtk::StringList::new(&["Error", "Warn", "Info", "Debug", "Trace"]),
            shell_enabled: shell.enabled,
            shell_level_idx: level_idx(&shell.level),
            comp_enabled: read_bool("log_to_file", true),
            comp_level_idx: level_idx(&read_raw("log_file_level").unwrap_or_else(|| "info".into())),
        };
        let widgets = view_output!();
        let _ = root;
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            LoggingInput::SetShellEnabled(v) => {
                self.shell_enabled = v;
                config_manager().update_config(|c| c.logging.enabled = v);
                let _ = mshell_logging::set_enabled(v);
            }
            LoggingInput::SetShellLevel(idx) => {
                let level = LEVELS[idx.min(4) as usize];
                self.shell_level_idx = idx;
                config_manager().update_config(|c| c.logging.level = level.to_string());
                let _ = mshell_logging::set_level(level);
            }
            LoggingInput::SetCompEnabled(v) => {
                self.comp_enabled = v;
                set_and_reload("log_to_file", if v { "1" } else { "0" }.to_string());
            }
            LoggingInput::SetCompLevel(idx) => {
                let level = LEVELS[idx.min(4) as usize];
                self.comp_level_idx = idx;
                set_and_reload("log_file_level", level.to_string());
            }
            LoggingInput::OpenFolder => {
                let dir = margo_logging::logs_dir();
                let _ = std::fs::create_dir_all(&dir);
                let _ = std::process::Command::new("xdg-open").arg(&dir).spawn();
            }
        }
    }
}
