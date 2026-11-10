//! Mdash → Tabs settings section.
//!
//! Edits the `MenuWidget::Dashboard` entry of `mdash_menu`: tab order
//! (↑ / ↓), tab visibility and, per tab, which cards show. When the
//! user's `mdash_menu` is an own (pre-tabs) layout without a dashboard,
//! the section instead offers to switch to the tabbed dashboard; the
//! quick-action Buttons editor below (`mdash_buttons_settings`) then
//! edits that layout's button rows.
//!
//! The rows are rebuilt from config on every change (ours or an external
//! reload), so the page always shows what mdash will show.

use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, MenuStoreFields, MenusStoreFields};
use mshell_config::schema::dashboard::{DashboardConfig, DashboardTab};
use mshell_config::schema::menu_widgets::MenuWidget;
use reactive_graph::prelude::Get;
use relm4::gtk;
use relm4::gtk::prelude::{BoxExt, ButtonExt, CheckButtonExt, WidgetExt};

/// The dashboard entry of an `mdash_menu` widget list, if any.
pub(crate) fn dashboard_of(widgets: &[MenuWidget]) -> Option<DashboardConfig> {
    widgets.iter().find_map(|w| match w {
        MenuWidget::Dashboard(cfg) => Some(cfg.clone()),
        _ => None,
    })
}

/// Apply `edit` to the (first) dashboard entry of `mdash_menu`.
fn edit_dashboard(edit: impl FnOnce(&mut DashboardConfig) + 'static) {
    config_manager().update_config(move |c| {
        if let Some(cfg) = c.menus.mdash_menu.widgets.iter_mut().find_map(|w| match w {
            MenuWidget::Dashboard(cfg) => Some(cfg),
            _ => None,
        }) {
            edit(cfg);
        }
    });
}

/// Replace the user's own mdash layout with the default tabbed dashboard.
fn use_dashboard() {
    config_manager().update_config(|c| {
        c.menus.mdash_menu.widgets = vec![MenuWidget::Dashboard(DashboardConfig::default())];
    });
}

pub(crate) fn build_mdash_tabs_section() -> (gtk::Box, EffectScope) {
    let section = gtk::Box::new(gtk::Orientation::Vertical, 8);
    section.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

    let title = gtk::Label::new(Some("Tabs"));
    title.add_css_class("label-large-bold");
    title.set_halign(gtk::Align::Start);
    section.append(&title);
    let desc = gtk::Label::new(Some(
        "Order and show the dashboard's tabs and pick the cards each tab shows. \
         The other cards share the space of a hidden one. \
         `mshellctl menu mdash <tab>` opens mdash on a tab.",
    ));
    desc.add_css_class("label-small");
    desc.set_halign(gtk::Align::Start);
    desc.set_xalign(0.0);
    desc.set_wrap(true);
    section.append(&desc);

    let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
    section.append(&body);

    let mut effects = EffectScope::new();
    {
        let body = body.clone();
        effects.push(move |_| {
            let widgets = config_manager()
                .config()
                .menus()
                .mdash_menu()
                .widgets()
                .get();
            rebuild(&body, dashboard_of(&widgets));
        });
    }
    (section, effects)
}

fn rebuild(body: &gtk::Box, cfg: Option<DashboardConfig>) {
    while let Some(child) = body.first_child() {
        body.remove(&child);
    }
    let Some(cfg) = cfg else {
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        list.add_css_class("boxed-list");
        let row = row(
            "setting:/mdash/use_dashboard",
            "Your own mdash layout is in use",
            "Switch to the tabbed dashboard (Today, Controls, System, Tools). Your layout is replaced.",
        );
        let button = gtk::Button::with_label("Use the tabbed dashboard");
        button.add_css_class("ok-button-primary");
        button.set_valign(gtk::Align::Center);
        button.connect_clicked(|_| use_dashboard());
        row.append(&button);
        list.append(&row);
        body.append(&list);
        return;
    };
    let tabs = cfg.normalised();
    let shown = tabs.iter().filter(|t| t.visible).count();
    let last = tabs.len() - 1;
    for (i, entry) in tabs.into_iter().enumerate() {
        let tab = entry.tab;
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        list.add_css_class("boxed-list");

        let head = row(
            &format!("setting:/mdash/tab/{}", tab.token()),
            tab.label(),
            tab_blurb(tab),
        );
        let up = gtk::Button::from_icon_name("go-up-symbolic");
        up.add_css_class("flat");
        up.set_valign(gtk::Align::Center);
        up.set_sensitive(i > 0);
        up.set_tooltip_text(Some("Move up"));
        mshell_common::a11y::name(&up, &format!("Move {} up", tab.label()));
        up.connect_clicked(move |_| edit_dashboard(move |c| c.move_tab(tab, -1)));
        let down = gtk::Button::from_icon_name("go-down-symbolic");
        down.add_css_class("flat");
        down.set_valign(gtk::Align::Center);
        down.set_sensitive(i < last);
        down.set_tooltip_text(Some("Move down"));
        mshell_common::a11y::name(&down, &format!("Move {} down", tab.label()));
        down.connect_clicked(move |_| edit_dashboard(move |c| c.move_tab(tab, 1)));
        let switch = gtk::Switch::new();
        switch.set_valign(gtk::Align::Center);
        switch.set_active(entry.visible);
        // The last shown tab can't be hidden.
        switch.set_sensitive(!(entry.visible && shown == 1));
        mshell_common::a11y::name(&switch, &format!("Show the {} tab", tab.label()));
        switch.connect_active_notify(move |s| {
            let v = s.is_active();
            edit_dashboard(move |c| {
                c.set_tab_visible(tab, v);
            });
        });
        head.append(&up);
        head.append(&down);
        head.append(&switch);
        list.append(&head);

        // Per-card checks (a tab with one card has nothing to pick).
        if tab.cards().len() > 1 && entry.visible {
            let cards = gtk::FlowBox::new();
            cards.add_css_class("action-row");
            cards.set_selection_mode(gtk::SelectionMode::None);
            cards.set_column_spacing(12);
            cards.set_max_children_per_line(5);
            cards.set_homogeneous(true);
            for card in tab.cards() {
                let card = *card;
                let check = gtk::CheckButton::with_label(card.label());
                check.set_active(entry.shows(card));
                check.connect_toggled(move |c| {
                    let v = c.is_active();
                    edit_dashboard(move |cfg| cfg.set_card_shown(card, v));
                });
                cards.append(&check);
            }
            list.append(&cards);
        }
        body.append(&list);
    }
}

fn tab_blurb(tab: DashboardTab) -> &'static str {
    match tab {
        DashboardTab::Today => "Calendar and agenda, weather, media, timers and alarms.",
        DashboardTab::Controls => "Volume, mic and brightness sliders and every quick setting.",
        DashboardTab::System => {
            "CPU, memory, temperature, disk, battery, network, updates and AI limits."
        }
        DashboardTab::Tools => "Shortcuts to the shell's menus.",
    }
}

/// An Adwaita action row: title + subtitle on the left; the caller appends
/// the controls.
fn row(tag: &str, title: &str, subtitle: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.add_css_class("action-row");
    row.set_widget_name(tag);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
    text.set_valign(gtk::Align::Center);
    text.set_hexpand(true);
    let t = gtk::Label::new(Some(title));
    t.add_css_class("label-medium-bold");
    t.set_halign(gtk::Align::Start);
    let s = gtk::Label::new(Some(subtitle));
    s.add_css_class("label-small");
    s.set_halign(gtk::Align::Start);
    s.set_xalign(0.0);
    s.set_wrap(true);
    s.set_natural_wrap_mode(gtk::NaturalWrapMode::None);
    text.append(&t);
    text.append(&s);
    row.append(&text);
    row
}

#[cfg(test)]
mod tests {
    use super::*;
    use mshell_config::schema::dashboard::DashboardCard;

    #[test]
    fn the_dashboard_entry_is_found_in_a_mixed_list() {
        assert_eq!(dashboard_of(&[MenuWidget::Weather]), None);
        let cfg = DashboardConfig::default();
        assert_eq!(
            dashboard_of(&[MenuWidget::Weather, MenuWidget::Dashboard(cfg.clone())]),
            Some(cfg)
        );
    }

    #[test]
    fn every_card_has_a_label_for_its_check() {
        for tab in DashboardTab::ALL {
            assert!(!tab_blurb(tab).is_empty());
            for card in tab.cards() {
                assert!(!DashboardCard::label(*card).is_empty());
            }
        }
    }
}
