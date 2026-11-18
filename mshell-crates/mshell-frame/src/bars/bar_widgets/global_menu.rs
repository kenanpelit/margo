//! Global menu — bar widget for [`crate::global_menu`]: the focused app's
//! name and one flat button per top-level menu of its exported dbusmenu.
//! A click asks the app to refresh that menu (`AboutToShow` + `GetLayout`)
//! and opens it as a nested `PopoverMenu` parented to the button; picking
//! an item sends `Event(id, "clicked")`. Menus past `global_menu.max_menus`
//! go under a » button. Hidden while `global_menu.enabled` is off or
//! nothing is focused; an app without an exported menu shows its name only.

use crate::global_menu::matcher::strip_mnemonic;
use crate::global_menu::{self, GlobalMenuState, RegistrarMode};
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, GlobalMenuConfig};
use reactive_graph::prelude::GetUntracked;
use relm4::gtk::Orientation;
use relm4::gtk::gio;
use relm4::gtk::glib;
use relm4::gtk::prelude::{
    ActionMapExt, AppInfoExt, BoxExt, ButtonExt, MenuModelExt, OrientableExt, PopoverExt, WidgetExt,
};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::sync::Arc;
use wayle_systray::types::menu::{MenuItem, MenuItemType, ToggleState, ToggleType};

/// The » button's key in `buttons` (dbusmenu ids are ≥ 0).
const OVERFLOW: i32 = -1;

pub(crate) struct GlobalMenuModel {
    state: GlobalMenuState,
    menus: gtk::Box,
    app_label: gtk::Label,
    popover: Option<gtk::PopoverMenu>,
    /// The bar buttons by top-level id (`OVERFLOW` for »).
    buttons: Vec<(i32, gtk::Button)>,
    _orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum GlobalMenuInput {
    /// Open the top-level menu `id` under its button.
    Open(i32),
    /// Open the overflow menu (top-level items from `index`) under ».
    Overflow(usize),
}

pub(crate) struct GlobalMenuInit {
    pub(crate) orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum GlobalMenuCommandOutput {
    State(GlobalMenuState),
    Submenu(Option<MenuItem>, i32),
}

/// Top-level entries shown in the bar: visible, labelled, non-separator.
pub(crate) fn top_level(menu: &MenuItem) -> Vec<&MenuItem> {
    menu.children
        .iter()
        .filter(|c| c.visible && c.item_type != MenuItemType::Separator)
        .filter(|c| c.label.as_deref().is_some_and(|l| !l.trim().is_empty()))
        .collect()
}

/// How many top-level menus fit `max` (0 = all) out of `n`.
pub(crate) fn shown(n: usize, max: u32) -> usize {
    if max == 0 { n } else { n.min(max as usize) }
}

/// The app's display name from its desktop entry, else the app id's
/// last segment.
fn app_name(app_id: &str) -> String {
    for id in [app_id.to_string(), app_id.to_lowercase()] {
        if let Some(i) = gio_unix::DesktopAppInfo::new(&format!("{id}.desktop")) {
            return i.display_name().to_string();
        }
    }
    app_id.rsplit('.').next().unwrap_or(app_id).to_string()
}

#[relm4::component(pub)]
impl Component for GlobalMenuModel {
    type CommandOutput = GlobalMenuCommandOutput;
    type Input = GlobalMenuInput;
    type Output = ();
    type Init = GlobalMenuInit;

    view! {
        #[root]
        gtk::Box {
            set_css_classes: &["global-menu-bar-widget", "ok-bar-widget"],
            set_orientation: Orientation::Horizontal,
            set_spacing: 2,
            set_valign: gtk::Align::Center,
            set_visible: false,

            #[local_ref]
            app_label -> gtk::Label {
                add_css_class: "global-menu-app",
            },
            #[local_ref]
            menus -> gtk::Box {
                set_orientation: Orientation::Horizontal,
                set_spacing: 0,
            },
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let mut rx = global_menu::subscribe();
        sender.command(|out, shutdown| async move {
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            loop {
                let state = rx.borrow_and_update().clone();
                let _ = out.send(GlobalMenuCommandOutput::State(state));
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    r = rx.changed() => if r.is_err() { break },
                }
            }
        });
        let app_label = gtk::Label::new(None);
        let menus = gtk::Box::new(Orientation::Horizontal, 0);
        let model = GlobalMenuModel {
            state: GlobalMenuState::default(),
            menus: menus.clone(),
            app_label: app_label.clone(),
            popover: None,
            buttons: Vec::new(),
            _orientation: params.orientation,
        };
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        let Some(service) = self.state.service.clone() else {
            return;
        };
        match msg {
            GlobalMenuInput::Open(id) => {
                sender.oneshot_command(async move {
                    let sub = global_menu::open_submenu(service, id).await;
                    GlobalMenuCommandOutput::Submenu(sub, id)
                });
            }
            GlobalMenuInput::Overflow(from) => {
                let Some(button) = self.button(OVERFLOW) else {
                    return;
                };
                if let Some(menu) = &self.state.menu {
                    let mut rest = (**menu).clone();
                    let keep: Vec<i32> = top_level(menu).iter().skip(from).map(|m| m.id).collect();
                    rest.children.retain(|c| keep.contains(&c.id));
                    self.popup(&rest, &button, &service);
                }
            }
        }
    }

    fn update_cmd(
        &mut self,
        msg: Self::CommandOutput,
        sender: ComponentSender<Self>,
        root: &Self::Root,
    ) {
        match msg {
            GlobalMenuCommandOutput::State(s) => {
                let rebuild = s.menu.as_ref().map(Arc::as_ptr)
                    != self.state.menu.as_ref().map(Arc::as_ptr)
                    || s.app_id != self.state.app_id;
                self.state = s;
                if rebuild {
                    self.render(root, &sender);
                }
            }
            GlobalMenuCommandOutput::Submenu(sub, id) => {
                if let (Some(sub), Some(service), Some(button)) =
                    (sub, self.state.service.clone(), self.button(id))
                {
                    self.popup(&sub, &button, &service);
                }
            }
        }
    }

    fn shutdown(&mut self, _widgets: &mut Self::Widgets, _output: relm4::Sender<Self::Output>) {
        if let Some(p) = self.popover.take() {
            p.unparent();
        }
    }
}

impl GlobalMenuModel {
    fn render(&mut self, root: &gtk::Box, sender: &ComponentSender<Self>) {
        let cfg: GlobalMenuConfig = config_manager().config().global_menu().get_untracked();
        while let Some(c) = self.menus.first_child() {
            self.menus.remove(&c);
        }
        self.buttons.clear();
        let s = &self.state;
        let visible =
            s.enabled && !s.app_id.is_empty() && self._orientation == Orientation::Horizontal;
        root.set_visible(visible);
        if !visible {
            return;
        }
        self.app_label.set_label(&app_name(&s.app_id));
        self.app_label.set_visible(cfg.show_app_name);
        root.set_tooltip_text(Some(&match (&s.menu, s.mode) {
            (Some(_), _) => format!("{} — menu from {}", s.app_id, s.service.as_ref().map_or("", |x| x.0.as_str())),
            (None, RegistrarMode::Failed) => "Global menu: no session bus".to_string(),
            (None, _) => format!(
                "{} exports no menu ({} registered). See the Global Menu docs for the appmenu modules apps need.",
                s.app_id, s.registered
            ),
        }));
        let Some(menu) = &s.menu else { return };
        let tops = top_level(menu);
        let n = shown(tops.len(), cfg.max_menus);
        for m in tops.iter().take(n) {
            let b = gtk::Button::with_label(&strip_mnemonic(m.label.as_deref().unwrap_or("")));
            b.set_css_classes(&["ok-button-flat", "global-menu-item"]);
            b.set_sensitive(m.enabled);
            let (id, s2) = (m.id, sender.clone());
            b.connect_clicked(move |_| s2.input(GlobalMenuInput::Open(id)));
            self.menus.append(&b);
            self.buttons.push((id, b));
        }
        if n < tops.len() {
            let b = gtk::Button::with_label("»");
            b.set_css_classes(&["ok-button-flat", "global-menu-item"]);
            b.set_tooltip_text(Some("More menus"));
            let s2 = sender.clone();
            b.connect_clicked(move |_| s2.input(GlobalMenuInput::Overflow(n)));
            self.menus.append(&b);
            self.buttons.push((OVERFLOW, b));
        }
    }

    fn button(&self, id: i32) -> Option<gtk::Button> {
        self.buttons
            .iter()
            .find(|(i, _)| *i == id)
            .map(|(_, b)| b.clone())
    }

    fn popup(&mut self, menu: &MenuItem, button: &gtk::Button, service: &(String, String)) {
        if let Some(old) = self.popover.take() {
            old.unparent();
        }
        let model = gio::Menu::new();
        let actions = gio::SimpleActionGroup::new();
        append_items(&menu.children, &model, &actions, service);
        let popover = gtk::PopoverMenu::from_model_full(&model, gtk::PopoverMenuFlags::NESTED);
        popover.insert_action_group("gm", Some(&actions));
        popover.set_parent(button);
        popover.popup();
        self.popover = Some(popover);
    }
}

/// Items into `target`, separators starting new sections (as GTK draws them).
fn append_items(
    items: &[MenuItem],
    target: &gio::Menu,
    actions: &gio::SimpleActionGroup,
    service: &(String, String),
) {
    let mut section = gio::Menu::new();
    for item in items.iter().filter(|i| i.visible) {
        if item.item_type == MenuItemType::Separator {
            if section.n_items() > 0 {
                target.append_section(None, &section);
                section = gio::Menu::new();
            }
            continue;
        }
        let label = strip_mnemonic(item.label.as_deref().unwrap_or(""));
        if item.has_children() {
            let sub = gio::Menu::new();
            append_items(&item.children, &sub, actions, service);
            section.append_submenu(Some(&label), &sub);
            continue;
        }
        let name = format!("i{}", item.id);
        let action = match item.toggle_type {
            ToggleType::Checkmark | ToggleType::Radio => gio::SimpleAction::new_stateful(
                &name,
                None,
                &glib::Variant::from(item.toggle_state == ToggleState::Checked),
            ),
            _ => gio::SimpleAction::new(&name, None),
        };
        let (svc, id) = (service.clone(), item.id);
        action.connect_activate(move |_, _| global_menu::activate(svc.clone(), id));
        action.set_enabled(item.enabled);
        actions.add_action(&action);
        let mi = gio::MenuItem::new(Some(&label), Some(&format!("gm.{name}")));
        if let Some(accel) = item.shortcut.as_deref().and_then(accelerator) {
            mi.set_attribute_value("accel", Some(&glib::Variant::from(accel.as_str())));
        }
        section.append_item(&mi);
    }
    if section.n_items() > 0 {
        target.append_section(None, &section);
    }
}

/// dbusmenu `shortcut` (`[["Control","S"]]`) → a GTK accelerator string
/// (shown next to the item; GTK does not bind it).
pub(crate) fn accelerator(shortcut: &[Vec<String>]) -> Option<String> {
    let (key, mods) = shortcut.first()?.split_last()?;
    let mut s: String = mods
        .iter()
        .filter_map(|m| match m.as_str() {
            "Control" => Some("<Control>"),
            "Shift" => Some("<Shift>"),
            "Alt" => Some("<Alt>"),
            "Super" => Some("<Super>"),
            _ => None,
        })
        .collect();
    s.push_str(key);
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overflow_counts_and_accelerators() {
        assert_eq!(shown(7, 0), 7);
        assert_eq!(shown(7, 4), 4);
        assert_eq!(shown(2, 4), 2);
        let sc = vec![vec![
            "Control".to_string(),
            "Shift".to_string(),
            "S".to_string(),
        ]];
        assert_eq!(accelerator(&sc).as_deref(), Some("<Control><Shift>S"));
        assert_eq!(accelerator(&[]), None);
    }
}
