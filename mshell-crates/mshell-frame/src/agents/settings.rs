//! The AI Agents config (`bars.widgets.agents`), mirrored for the poll
//! threads: the reactive config store lives on the GTK thread, the polls
//! run on blocking workers. The pill and the menu each hold a
//! [`watch_settings`] scope that copies every change here.

use super::pricing::Currency;
use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    AgentsConfig, AgentsCurrency, BarWidgetsStoreFields, BarsStoreFields, ConfigStoreFields,
};
use reactive_graph::prelude::Get;
use std::sync::{LazyLock, RwLock};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct AgentSettings {
    pub(crate) notify_waiting: bool,
    pub(crate) notify_finished: bool,
    pub(crate) currency: Currency,
}

static SETTINGS: LazyLock<RwLock<AgentSettings>> = LazyLock::new(RwLock::default);

pub(crate) fn current() -> AgentSettings {
    SETTINGS.read().map(|s| s.clone()).unwrap_or_default()
}

pub(crate) fn currency() -> Currency {
    current().currency
}

fn apply(c: &AgentsConfig) {
    if let Ok(mut s) = SETTINGS.write() {
        *s = AgentSettings {
            notify_waiting: c.notify_waiting,
            notify_finished: c.notify_finished,
            currency: match c.currency {
                AgentsCurrency::Usd => Currency::Usd,
                AgentsCurrency::Eur => Currency::Eur,
            },
        };
    }
    super::accounts::set_configured(c.claude_config_dirs.clone());
}

/// Track the config live; keep the returned scope alive with the widget.
pub(crate) fn watch_settings() -> EffectScope {
    let mut effects = EffectScope::new();
    effects.push(move |_| {
        let c = config_manager().config().bars().widgets().agents().get();
        apply(&c);
    });
    effects
}
