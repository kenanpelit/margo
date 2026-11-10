//! Mdash → Buttons settings section.
//!
//! Embeds the shared QuickActions add / remove / reorder editor and maps
//! the single flat button list onto `mdash_menu`'s `QuickActions` rows
//! (split as evenly as possible). Rendered as a sub-section inside the
//! generic `WidgetMenuSettingsModel` page when `kind == MenuKind::Mdash`.
//!
//! A user's own (pre-tabs) mdash layout lays its buttons out in dense
//! rows; here the user edits one logical list and we redistribute it
//! across whatever `QuickActions` slots the menu has, leaving every other
//! widget (header, calendar, weather, …) untouched.
//!
//! The tabbed dashboard (the default) has no button rows — its toggles are
//! Controls tiles, its launchers the Tools tab, power the header — so the
//! section only shows while the menu has `QuickActions` rows
//! (`mdash_tabs_settings` edits the dashboard).

use crate::menu_settings::quick_actions_list::{
    QuickActionListInput, QuickActionListModel, QuickActionListOutput,
};
use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, MenuStoreFields, MenusStoreFields};
use mshell_config::schema::menu_widgets::{MenuWidget, QuickActionWidget, QuickActionsConfig};
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::prelude::{BoxExt, WidgetExt};
use relm4::{Component, ComponentController, ComponentParts, ComponentSender, Controller, gtk};

/// Flatten every button in `mdash_menu`'s `QuickActions` rows, in order.
fn flatten(widgets: Vec<MenuWidget>) -> Vec<QuickActionWidget> {
    widgets
        .into_iter()
        .filter_map(|w| match w {
            MenuWidget::QuickActions(c) => Some(c.widgets),
            _ => None,
        })
        .flatten()
        .collect()
}

fn read_buttons_untracked() -> Vec<QuickActionWidget> {
    flatten(
        config_manager()
            .config()
            .menus()
            .mdash_menu()
            .widgets()
            .get_untracked(),
    )
}

/// The buttons, and whether the menu has any button row at all.
fn read_buttons_tracked() -> (Vec<QuickActionWidget>, bool) {
    let widgets = config_manager()
        .config()
        .menus()
        .mdash_menu()
        .widgets()
        .get();
    let rows = has_button_rows(&widgets);
    (flatten(widgets), rows)
}

/// The section only applies to a layout with `QuickActions` rows, or to
/// one without a tabbed dashboard (a row gets added there).
fn has_button_rows(widgets: &[MenuWidget]) -> bool {
    widgets
        .iter()
        .any(|w| matches!(w, MenuWidget::QuickActions(_)))
        || !widgets
            .iter()
            .any(|w| matches!(w, MenuWidget::Dashboard(_)))
}

/// Split `items` into `n` near-equal chunks (front chunks take the remainder).
fn split_even(items: Vec<QuickActionWidget>, n: usize) -> Vec<Vec<QuickActionWidget>> {
    let n = n.max(1);
    let total = items.len();
    let base = total / n;
    let extra = total % n;
    let mut it = items.into_iter();
    (0..n)
        .map(|i| {
            let take = base + usize::from(i < extra);
            (&mut it).take(take).collect()
        })
        .collect()
}

/// Write a flat button list back into `mdash_menu`'s `QuickActions` rows,
/// distributing it evenly and preserving all other widgets.
fn write_buttons(buttons: Vec<QuickActionWidget>) {
    config_manager().update_config(move |c| {
        let slots: Vec<usize> = c
            .menus
            .mdash_menu
            .widgets
            .iter()
            .enumerate()
            .filter(|(_, w)| matches!(w, MenuWidget::QuickActions(_)))
            .map(|(i, _)| i)
            .collect();

        if slots.is_empty() {
            // Never bolt a button row onto the tabbed dashboard: that
            // would turn it into a mixed layout.
            if c.menus
                .mdash_menu
                .widgets
                .iter()
                .any(|w| matches!(w, MenuWidget::Dashboard(_)))
            {
                return;
            }
            c.menus
                .mdash_menu
                .widgets
                .push(MenuWidget::QuickActions(QuickActionsConfig {
                    widgets: buttons,
                }));
            return;
        }

        for (slot, chunk) in slots.iter().zip(split_even(buttons, slots.len())) {
            if let MenuWidget::QuickActions(cfg) = &mut c.menus.mdash_menu.widgets[*slot] {
                cfg.widgets = chunk;
            }
        }
    });
}

pub(crate) struct MdashButtonsSettingsModel {
    list: Controller<QuickActionListModel>,
    /// Last button list we read or wrote. Dedupes our own config writes
    /// from the reactive effect so an external edit re-seeds the editor
    /// while our own edit does not loop back.
    last: Vec<QuickActionWidget>,
    _effects: EffectScope,
}

impl std::fmt::Debug for MdashButtonsSettingsModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MdashButtonsSettingsModel")
            .field("last", &self.last)
            .finish()
    }
}

#[derive(Debug)]
pub(crate) enum MdashButtonsSettingsInput {
    /// The embedded editor changed (user add / remove / reorder).
    Edited(Vec<QuickActionWidget>),
    /// `mdash_menu` changed in config (e.g. external reload) — re-seed;
    /// `rows` = the menu has button rows (else the section hides).
    External(Vec<QuickActionWidget>, bool),
}

#[derive(Debug)]
pub(crate) enum MdashButtonsSettingsOutput {}

pub(crate) struct MdashButtonsSettingsInit {}

impl Component for MdashButtonsSettingsModel {
    type CommandOutput = ();
    type Input = MdashButtonsSettingsInput;
    type Output = MdashButtonsSettingsOutput;
    type Init = MdashButtonsSettingsInit;
    type Root = gtk::Box;
    type Widgets = ();

    fn init_root() -> Self::Root {
        let section = gtk::Box::new(gtk::Orientation::Vertical, 12);
        section.set_hexpand(true);
        section
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

        let header = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let title = gtk::Label::new(Some("Buttons"));
        title.add_css_class("label-large-bold");
        title.set_halign(gtk::Align::Start);
        let desc = gtk::Label::new(Some(
            "Choose which quick-action buttons appear in mdash and reorder them. The list is split evenly across mdash's button rows. Changes take effect immediately.",
        ));
        desc.add_css_class("label-small");
        desc.set_halign(gtk::Align::Start);
        desc.set_xalign(0.0);
        desc.set_wrap(true);
        header.append(&title);
        header.append(&desc);
        root.append(&header);

        let initial = read_buttons_untracked();
        let list = QuickActionListModel::builder()
            .launch(initial.clone())
            .forward(sender.input_sender(), |out| match out {
                QuickActionListOutput::Changed(v) => MdashButtonsSettingsInput::Edited(v),
            });
        root.append(list.widget());

        let mut effects = EffectScope::new();
        {
            let s = sender.clone();
            effects.push(move |_| {
                let (buttons, rows) = read_buttons_tracked();
                s.input(MdashButtonsSettingsInput::External(buttons, rows));
            });
        }

        let model = MdashButtonsSettingsModel {
            list,
            last: initial,
            _effects: effects,
        };
        ComponentParts { model, widgets: () }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>, root: &Self::Root) {
        match message {
            MdashButtonsSettingsInput::Edited(v) => {
                if v != self.last {
                    self.last = v.clone();
                    write_buttons(v);
                }
            }
            MdashButtonsSettingsInput::External(v, rows) => {
                root.set_visible(rows);
                if v != self.last {
                    self.last = v.clone();
                    self.list.emit(QuickActionListInput::ReplaceAll(v));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mshell_config::schema::dashboard::DashboardConfig;

    #[test]
    fn the_buttons_editor_shows_only_for_button_layouts() {
        let dash = MenuWidget::Dashboard(DashboardConfig::default());
        let row = MenuWidget::QuickActions(QuickActionsConfig {
            widgets: vec![QuickActionWidget::Lock],
        });
        assert!(!has_button_rows(std::slice::from_ref(&dash)));
        assert!(has_button_rows(&[dash, row.clone()]));
        assert!(has_button_rows(&[MenuWidget::Weather, row]));
        assert!(has_button_rows(&[MenuWidget::Weather]));
    }

    #[test]
    fn split_even_spreads_the_remainder_to_the_front() {
        use QuickActionWidget as Q;
        let rows = split_even(vec![Q::Lock, Q::Logout, Q::Reboot], 2);
        assert_eq!(rows, vec![vec![Q::Lock, Q::Logout], vec![Q::Reboot]]);
    }
}
