//! Settings → Widgets → AI Agents — the `bars.widgets.agents` knobs:
//! session notifications, extra Claude Code accounts and the currency of
//! the cost estimates. Composed in `widget_pages.rs` with the generic
//! per-menu geometry page, the same way Lyrics is.

use crate::row::Row;
use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    AgentsConfig, AgentsCurrency, BarWidgetsStoreFields, BarsStoreFields, ConfigStoreFields,
};
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::glib;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};

pub(crate) struct AgentsSettingsModel {
    cfg: AgentsConfig,
    currencies: gtk::StringList,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum AgentsSettingsInput {
    NotifyWaiting(bool),
    NotifyFinished(bool),
    ConfigDirs(String),
    Currency(u32),
    Effect(AgentsConfig),
}

pub(crate) struct AgentsSettingsInit {}

/// `"~/.claude-work, /srv/c"` → `["~/.claude-work", "/srv/c"]`.
pub(crate) fn parse_dirs(text: &str) -> Vec<String> {
    text.split([',', '\n'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

const CURRENCIES: [(AgentsCurrency, &str); 2] = [
    (AgentsCurrency::Usd, "USD ($)"),
    (AgentsCurrency::Eur, "EUR (€)"),
];

#[relm4::component(pub(crate))]
impl Component for AgentsSettingsModel {
    type CommandOutput = ();
    type Input = AgentsSettingsInput;
    type Output = ();
    type Init = AgentsSettingsInit;

    view! {
        #[root]
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

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Claude Code sessions",
                    set_halign: gtk::Align::Start,
                },

                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Notify when a session needs you" },
                        #[template_child] desc { set_label: "A desktop notification when a session starts waiting for a permission or an answer. The bar icon turns into a red bell either way." },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(waiting_handler)]
                            set_active: model.cfg.notify_waiting,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(AgentsSettingsInput::NotifyWaiting(v));
                                glib::Propagation::Proceed
                            } @waiting_handler,
                        },
                    },

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Notify when a long turn finishes" },
                        #[template_child] desc { set_label: "A desktop notification when a session goes from working to idle after two minutes or more." },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(finished_handler)]
                            set_active: model.cfg.notify_finished,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(AgentsSettingsInput::NotifyFinished(v));
                                glib::Propagation::Proceed
                            } @finished_handler,
                        },
                    },

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Extra Claude config dirs" },
                        #[template_child] desc { set_label: "CLAUDE_CONFIG_DIR of other accounts to always show, comma-separated (e.g. ~/.claude-work). Dirs of running sessions are found on their own." },
                        gtk::Entry {
                            set_valign: gtk::Align::Center,
                            set_width_request: 220,
                            set_placeholder_text: Some("~/.claude-work"),
                            set_text: &model.cfg.claude_config_dirs.join(", "),
                            connect_changed[sender] => move |e| {
                                sender.input(AgentsSettingsInput::ConfigDirs(e.text().to_string()));
                            },
                        },
                    },

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Cost currency" },
                        #[template_child] desc { set_label: "Estimated costs use LiteLLM's public model prices; EUR converts at the day's ECB rate." },
                        gtk::DropDown {
                            set_valign: gtk::Align::Center,
                            set_model: Some(&model.currencies),
                            #[watch]
                            #[block_signal(currency_handler)]
                            set_selected: CURRENCIES.iter().position(|(c, _)| *c == model.cfg.currency).unwrap_or(0) as u32,
                            connect_selected_notify[sender] => move |d| {
                                sender.input(AgentsSettingsInput::Currency(d.selected()));
                            } @currency_handler,
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
        let mut effects = EffectScope::new();
        let sc = sender.clone();
        effects.push(move |_| {
            let c = config_manager().config().bars().widgets().agents().get();
            sc.input(AgentsSettingsInput::Effect(c));
        });
        let currencies = gtk::StringList::new(&CURRENCIES.map(|(_, l)| l));
        let model = AgentsSettingsModel {
            cfg: config_manager()
                .config()
                .bars()
                .widgets()
                .agents()
                .get_untracked(),
            currencies,
            _effects: effects,
        };
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {
        let set = |f: Box<dyn FnOnce(&mut AgentsConfig)>| {
            config_manager().update_config(move |c| f(&mut c.bars.widgets.agents));
        };
        match message {
            AgentsSettingsInput::NotifyWaiting(v) => set(Box::new(move |a| a.notify_waiting = v)),
            AgentsSettingsInput::NotifyFinished(v) => {
                set(Box::new(move |a| a.notify_finished = v));
            }
            AgentsSettingsInput::ConfigDirs(text) => {
                let dirs = parse_dirs(&text);
                if dirs != self.cfg.claude_config_dirs {
                    set(Box::new(move |a| a.claude_config_dirs = dirs));
                }
            }
            AgentsSettingsInput::Currency(i) => {
                if let Some((c, _)) = CURRENCIES.get(i as usize).copied() {
                    set(Box::new(move |a| a.currency = c));
                }
            }
            AgentsSettingsInput::Effect(c) => self.cfg = c,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dirs_split_on_commas_and_lines() {
        assert_eq!(
            parse_dirs(" ~/.claude-work, /srv/c ,\n,"),
            ["~/.claude-work", "/srv/c"]
        );
        assert!(parse_dirs("  ").is_empty());
    }
}
