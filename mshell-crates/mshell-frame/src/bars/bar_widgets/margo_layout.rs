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
//! Clicking a menu item shells out to `mctl layout <index>`. The
//! dedicated `mctl layout` subcommand is the well-tested path —
//! `mctl dispatch setlayout <name>` exists too but requires the
//! name in slot 4 (`mctl dispatch setlayout "" "" "" <name>`),
//! which is fiddly enough that the previous version of this file
//! got the wire format wrong and clicks silently no-op'd. The
//! index API is order-coupled to the compositor's layout list but
//! we read it from the same state.json so the mapping is stable.
//!
//! The currently-active layout (per the focused output's
//! `layout_idx`) gets the `.selected` class so the matugen primary
//! accent paints it green / blue / whatever the active scheme is.
//! A 500 ms poll loop checks `read_state_json()` and emits
//! `LayoutChanged` only when the focused output's layout actually
//! moves — no event in `MargoEvent` fires for a layout-only switch
//! (tag-switch fires `WorkspaceV2`, layout switch is silent on the
//! event bus), so polling is the only path that catches all four
//! triggers (mctl layout, dispatch setlayout, switch_layout, and
//! direct keybind).

use mshell_margo_client::read_state_json;
use relm4::gtk::gio::prelude::ActionMapExt;
use relm4::gtk::glib::clone::Downgrade;
use relm4::gtk::glib::variant::ToVariant;
use relm4::gtk::prelude::{BoxExt, ButtonExt, PopoverExt, WidgetExt};
use relm4::gtk::{Orientation, gio};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;
use tracing::warn;

/// Polling interval for active-layout detection. Matches the upper
/// bound of `mshell_margo_client::sync::POLL_INTERVAL` (250 ms) so
/// the highlight lags at most one tick behind the rest of the bar.
const ACTIVE_POLL_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Debug)]
pub(crate) struct MargoLayoutModel {
    orientation: Orientation,
    /// Index → button mapping so `LayoutChanged` can flip the
    /// `.selected` class without rebuilding the popover. Held in
    /// `Rc<RefCell<…>>` because the periodic glib timeout also
    /// needs to reach in.
    buttons: Rc<RefCell<Vec<gtk::Button>>>,
    _timeout: Option<gtk::glib::SourceId>,
}

#[derive(Debug)]
pub(crate) enum MargoLayoutInput {
    /// Switch to layout at this index. Index maps to the list in
    /// `state.layouts` (and the wired-in fallback below).
    SetLayoutIndex(usize),
    /// Re-evaluate which button should carry the `.selected` class.
    /// Fired by the active-layout poll loop below.
    LayoutChanged(Option<usize>),
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
        let buttons_cell: Rc<RefCell<Vec<gtk::Button>>> = Rc::new(RefCell::new(Vec::new()));
        let active_cell: Rc<RefCell<Option<usize>>> = Rc::new(RefCell::new(None));

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

        // Save the bare buttons so the LayoutChanged handler can
        // flip CSS classes by index.
        {
            let mut sink = buttons_cell.borrow_mut();
            sink.extend(buttons.iter().map(|(_, b)| b.clone()));
        }

        // Start the active-layout poller. glib's timeout_add_local
        // runs on the GTK main loop so we can touch widgets from
        // its closure directly (no Send/Sync gymnastics).
        let sender_poll = sender.clone();
        let active_cell_poll = active_cell.clone();
        let timeout = gtk::glib::timeout_add_local(ACTIVE_POLL_INTERVAL, move || {
            let next = current_active_layout_idx();
            let mut last = active_cell_poll.borrow_mut();
            if *last != next {
                *last = next;
                sender_poll.input(MargoLayoutInput::LayoutChanged(next));
            }
            gtk::glib::ControlFlow::Continue
        });

        // Fire the initial highlight synchronously so the popover
        // never flashes a stale row before the first poll-tick.
        let initial_active = current_active_layout_idx();
        *active_cell.borrow_mut() = initial_active;
        apply_active_class(&buttons_cell.borrow(), initial_active);

        // active_cell stays alive via the timeout closure that
        // captured a clone — once `timeout` is dropped in Drop, the
        // closure releases its clone and the cell is freed.
        let _ = active_cell;

        let model = MargoLayoutModel {
            orientation: params.orientation,
            buttons: buttons_cell,
            _timeout: Some(timeout),
        };

        let widgets = view_output!();

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

    fn update(
        &mut self,
        message: Self::Input,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            MargoLayoutInput::SetLayoutIndex(idx) => {
                // `mctl layout <idx>` is the dedicated subcommand —
                // simpler than `mctl dispatch setlayout` which
                // requires the layout NAME in slot 4 (positions 1-3
                // empty) because margo reads `arg.v` from there. An
                // earlier version of this widget passed the idx as
                // slot 1 (arg.i), which `setlayout` ignores; that's
                // why clicks silently no-op'd.
                tokio::spawn(async move {
                    let mut command = tokio::process::Command::new("mctl");
                    command.arg("layout").arg(idx.to_string());
                    match command.status().await {
                        Ok(status) if status.success() => {}
                        Ok(status) => warn!(
                            ?status,
                            idx, "mctl layout returned non-zero"
                        ),
                        Err(e) => warn!(
                            error = %e,
                            idx,
                            "mctl layout spawn failed"
                        ),
                    }
                });
            }
            MargoLayoutInput::LayoutChanged(idx) => {
                apply_active_class(&self.buttons.borrow(), idx);
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

/// Read the focused output's `layout_idx` from state.json. Returns
/// `None` when state.json is missing, no output is currently
/// focused, or the index is past the layouts list (transient).
fn current_active_layout_idx() -> Option<usize> {
    let state = read_state_json()?;
    let focused = state.outputs.iter().find(|o| o.active)?;
    let idx = focused.layout_idx;
    if idx < state.layouts.len() { Some(idx) } else { None }
}

/// Apply the `.selected` class to the button at `idx`, clear it
/// on every other. The SCSS rule `.ok-button-surface.selected`
/// (`03-primitives/_buttons.scss`) paints it `var(--primary)` /
/// `var(--on-primary)`, so the popover row tracks the matugen
/// scheme for free.
fn apply_active_class(buttons: &[gtk::Button], active: Option<usize>) {
    for (i, button) in buttons.iter().enumerate() {
        if Some(i) == active {
            button.add_css_class("selected");
        } else {
            button.remove_css_class("selected");
        }
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
