//! Settings -> Guide -> Tools: a `gtk::ListBox` of companion binaries on
//! the left, a monospace `--help` dump on the right. Its own small Relm4
//! `Component` (unlike the other three tabs, which are plain
//! build-once-and-forget widgets) because tool selection and the
//! per-tool loading state genuinely change after the tab is first shown.

use super::tools::{self, HelpResult};
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::collections::HashMap;

#[derive(Debug, Clone)]
enum ToolState {
    Loading,
    Output(String),
    NotFound,
    TimedOut,
}

#[derive(Debug, Default)]
pub(crate) struct ToolsTabModel {
    selected: Option<&'static str>,
    cache: HashMap<&'static str, ToolState>,
}

#[derive(Debug)]
pub(crate) enum ToolsTabInput {
    Select(&'static str),
    HelpLoaded(&'static str, HelpResult),
}

#[derive(Debug)]
pub(crate) enum ToolsTabOutput {}

pub(crate) struct ToolsTabInit {}

#[derive(Debug)]
pub(crate) enum ToolsTabCommandOutput {}

#[relm4::component(pub(crate))]
impl Component for ToolsTabModel {
    type CommandOutput = ToolsTabCommandOutput;
    type Input = ToolsTabInput;
    type Output = ToolsTabOutput;
    type Init = ToolsTabInit;

    view! {
        #[root]
        gtk::Box {
            set_orientation: gtk::Orientation::Horizontal,
            set_hexpand: true,
            set_vexpand: true,
            set_spacing: 8,

            gtk::ScrolledWindow {
                set_width_request: 180,
                set_hscrollbar_policy: gtk::PolicyType::Never,
                #[name = "tool_list"]
                gtk::ListBox {
                    add_css_class: "guide-tools-list",
                    connect_row_selected[sender] => move |_, row| {
                        if let Some(row) = row {
                            let name = tools::TOOLS[row.index() as usize];
                            sender.input(ToolsTabInput::Select(name));
                        }
                    },
                },
            },

            gtk::ScrolledWindow {
                set_hexpand: true,
                #[name = "output_view"]
                gtk::TextView {
                    add_css_class: "guide-tools-output",
                    set_editable: false,
                    set_cursor_visible: false,
                    set_monospace: true,
                    set_wrap_mode: gtk::WrapMode::WordChar,
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = ToolsTabModel::default();
        let widgets = view_output!();
        for name in tools::TOOLS {
            let row = gtk::Label::new(Some(name));
            row.set_halign(gtk::Align::Start);
            row.set_margin_top(4);
            row.set_margin_bottom(4);
            row.set_margin_start(8);
            row.set_margin_end(8);
            widgets.tool_list.append(&row);
        }
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            ToolsTabInput::Select(name) => {
                self.selected = Some(name);
                if !self.cache.contains_key(name) {
                    self.cache.insert(name, ToolState::Loading);
                    let sender = sender.clone();
                    tools::spawn_help(name, move |result| {
                        sender.input(ToolsTabInput::HelpLoaded(name, result));
                    });
                }
            }
            ToolsTabInput::HelpLoaded(name, result) => {
                let mapped = match result {
                    HelpResult::Output(text) => ToolState::Output(text),
                    HelpResult::NotFound => ToolState::NotFound,
                    HelpResult::TimedOut => ToolState::TimedOut,
                };
                self.cache.insert(name, mapped);
            }
        }
    }

    fn post_view() {
        let buffer = output_view.buffer();
        let text = match model.selected.and_then(|name| model.cache.get(name)) {
            None => String::new(),
            Some(ToolState::Loading) => "Loading…".to_string(),
            Some(ToolState::Output(text)) => text.clone(),
            Some(ToolState::NotFound) => {
                format!("\"{}\" not found on PATH.", model.selected.unwrap_or(""))
            }
            Some(ToolState::TimedOut) => format!(
                "\"{} --help\" didn't respond within 2 seconds.",
                model.selected.unwrap_or("")
            ),
        };
        buffer.set_text(&text);
    }
}
