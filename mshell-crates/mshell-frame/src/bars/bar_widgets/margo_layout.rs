//! Layout switcher pill for the bar.
//!
//! Reads the active monitor's layout list from
//! `mshell-margo-client`'s state-snapshot view (which mirrors
//! margo's compositor state.json) so the menu always reflects the
//! 14 layouts the compositor actually knows about (tile / scroller
//! / grid / monocle / deck / center_tile / right_tile /
//! vertical_scroller / vertical_tile / vertical_grid / vertical_deck
//! / tgmix / canvas / dwindle) — not the four-Hyprland-strings
//! hard-coded list this widget shipped with previously.
//!
//! Clicking a menu item shells out to `mctl dispatch setlayout
//! <index>` with the layout's index in `state.layouts`. That goes
//! through the same dwl-ipc dispatch the rest of margo uses, so
//! the layout change is durable + survives a margo reload.

use mshell_margo_client::read_state_json;
use relm4::gtk::gio::prelude::ActionMapExt;
use relm4::gtk::glib::clone::Downgrade;
use relm4::gtk::glib::variant::ToVariant;
use relm4::gtk::prelude::{BoxExt, ButtonExt, PopoverExt, WidgetExt};
use relm4::gtk::{Orientation, gio};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use tracing::warn;

#[derive(Debug)]
pub(crate) struct MargoLayoutModel {
    orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum MargoLayoutInput {
    /// Switch to layout at this index in margo's `state.layouts`.
    SetLayoutIndex(usize),
}

#[derive(Debug)]
pub(crate) enum MargoLayoutOutput {}

pub(crate) struct MargoLayoutInit {
    pub(crate) orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum MargoLayoutCommandOutput {}

#[relm4::component(pub)]
impl Component for MargoLayoutModel {
    type CommandOutput = MargoLayoutCommandOutput;
    type Input = MargoLayoutInput;
    type Output = MargoLayoutOutput;
    type Init = MargoLayoutInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "margo-layout-bar-widget",
            set_hexpand: model.orientation == Orientation::Vertical,
            set_vexpand: model.orientation == Orientation::Horizontal,
            set_halign: gtk::Align::Center,
            set_valign: gtk::Align::Center,

            #[name = "menu_button"]
            gtk::MenuButton {
                set_css_classes: &["ok-button-surface", "ok-bar-widget"],
                set_hexpand: false,
                set_vexpand: false,
                set_icon_name: "layout-symbolic",
                set_always_show_arrow: false,
            }
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = MargoLayoutModel {
            orientation: params.orientation,
        };

        let widgets = view_output!();

        // Populate the menu from the live margo state.json. Falls
        // back to the wired-in name list if state.json isn't
        // readable yet (margo not started, or transient I/O error
        // during startup). The wired list is taken verbatim from
        // `margo-config::layouts::DEFAULT_LAYOUTS` and from
        // state.json runs against margo 0.4.x.
        let layout_names: Vec<String> = read_state_json()
            .map(|s| s.layouts)
            .unwrap_or_else(default_layout_names);

        let action_group = gio::SimpleActionGroup::new();
        let menu = gio::Menu::new();
        let mut buttons: Vec<(String, gtk::Button)> = Vec::with_capacity(layout_names.len());

        for (idx, name) in layout_names.iter().enumerate() {
            let pretty = pretty_layout_name(name);
            let icon = icon_for_layout(name);
            let button = Self::add_layout(&sender, &menu, &action_group, idx, name, &pretty, icon);
            buttons.push(button);
        }

        let popover = gtk::PopoverMenu::from_model_full(&menu, gtk::PopoverMenuFlags::NESTED);
        popover.set_has_arrow(false);

        for (custom_id, widget) in &buttons {
            popover.add_child(widget, custom_id);
        }

        widgets.menu_button.set_popover(Some(&popover));
        widgets
            .menu_button
            .insert_action_group("main", Some(&action_group));

        for (custom_id, button) in &buttons {
            popover.add_child(button, custom_id);
            let popover_weak = popover.downgrade();
            button.connect_clicked(move |_| {
                if let Some(p) = popover_weak.upgrade() {
                    p.popdown();
                }
            });
        }

        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        _widgets: &mut Self::Widgets,
        message: Self::Input,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            MargoLayoutInput::SetLayoutIndex(idx) => {
                // mctl dispatch setlayout <idx>. Margo reads
                // `arg.i` (i32) from the first dispatch slot, so
                // pass the index as the only argument. The previous
                // version of this widget called
                // `MargoService::eval("hl.workspace_rule(...)")`
                // which is a hyprland string the margo client
                // never grew an action for — that's the bug the
                // user observed as "layout switcher does nothing."
                tokio::spawn(async move {
                    let mut command = tokio::process::Command::new("mctl");
                    command
                        .arg("dispatch")
                        .arg("setlayout")
                        .arg(idx.to_string());
                    match command.status().await {
                        Ok(status) if status.success() => {}
                        Ok(status) => warn!(
                            ?status,
                            idx, "mctl dispatch setlayout returned non-zero"
                        ),
                        Err(e) => warn!(
                            error = %e,
                            idx,
                            "mctl dispatch setlayout spawn failed"
                        ),
                    }
                });
            }
        }
    }
}

impl MargoLayoutModel {
    fn add_layout(
        sender: &ComponentSender<Self>,
        menu: &gio::Menu,
        action_group: &gio::SimpleActionGroup,
        idx: usize,
        action_id: &str,
        display_name: &str,
        icon_name: &str,
    ) -> (String, gtk::Button) {
        let action = gio::SimpleAction::new(action_id, None);
        let sender_clone = sender.clone();
        action.connect_activate(move |_, _| {
            let _ = sender_clone.input(MargoLayoutInput::SetLayoutIndex(idx));
        });
        action_group.add_action(&action);

        let custom_id = format!("layout-{}", action_id);

        let item = gio::MenuItem::new(Some(display_name), Some(&format!("main.{}", action_id)));
        item.set_attribute_value("custom", Some(&custom_id.to_variant()));
        menu.append_item(&item);

        let row = gtk::Box::builder()
            .orientation(Orientation::Horizontal)
            .spacing(8)
            .build();
        row.append(&gtk::Image::from_icon_name(icon_name));
        row.append(&gtk::Label::new(Some(display_name)));

        let button = gtk::Button::builder()
            .child(&row)
            .action_name(&format!("main.{}", action_id))
            .css_classes(["ok-button-surface"])
            .build();

        (custom_id, button)
    }
}

/// Wired-in fallback for when `read_state_json()` returns `None`
/// (margo not running yet, IPC socket transient). Matches margo's
/// `config.layouts` default list as of mshell-port branch.
fn default_layout_names() -> Vec<String> {
    [
        "tile",
        "scroller",
        "grid",
        "monocle",
        "deck",
        "center_tile",
        "right_tile",
        "vertical_scroller",
        "vertical_tile",
        "vertical_grid",
        "vertical_deck",
        "tgmix",
        "canvas",
        "dwindle",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// Display name for the layout dropdown — snake_case identifiers
/// get title-cased and the leading "vertical_" namespace folded
/// into a " (Vertical)" suffix so the list reads cleanly.
fn pretty_layout_name(id: &str) -> String {
    if let Some(stem) = id.strip_prefix("vertical_") {
        return format!("{} (Vertical)", title_case_snake(stem));
    }
    title_case_snake(id)
}

fn title_case_snake(s: &str) -> String {
    s.split('_')
        .map(|chunk| {
            let mut chars = chunk.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().chain(chars).collect::<String>(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Icon-name hint for each layout. Falls back to the generic
/// `layout-symbolic` so the menu doesn't render blank rows for
/// layouts whose dedicated icon hasn't been packaged yet.
fn icon_for_layout(id: &str) -> &'static str {
    match id {
        "tile" => "layout-tile-symbolic",
        "scroller" | "vertical_scroller" => "layout-scrolling-symbolic",
        "grid" | "vertical_grid" => "layout-grid-symbolic",
        "monocle" => "layout-monocle-symbolic",
        "deck" | "vertical_deck" => "layout-deck-symbolic",
        "center_tile" => "layout-center-symbolic",
        "right_tile" => "layout-right-symbolic",
        "vertical_tile" => "layout-tile-vertical-symbolic",
        "tgmix" => "layout-mix-symbolic",
        "canvas" => "layout-canvas-symbolic",
        "dwindle" => "layout-dwindle-symbolic",
        _ => "layout-symbolic",
    }
}
