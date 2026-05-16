//! Margo layout switcher rendered as an in-frame menu widget.
//!
//! The previous bar widget (`bar_widgets/margo_layout.rs`) used a
//! `gtk::PopoverMenu`, which under Wayland creates its own
//! `xdg_popup` surface — visually that reads as a separate window
//! detached from the bar, not the contiguous slide-out drawer the
//! rest of mshell's menus produce. This widget is the same layout
//! list, but rendered as plain GTK content suitable to be embedded
//! in the frame's menu stack (under `MenuType::MargoLayout`), so
//! it slides out alongside the bar like Clock / Session /
//! Notifications do.
//!
//! Content: a vertical list of layout rows (icon + display name),
//! with the row matching the focused output's `layout_idx`
//! marked `.selected`. The list is sourced from `state.json` so
//! custom layout sets are honoured; a wired-in fallback covers
//! the brief cold-start window before margo writes its first
//! state file. Clicking a row spawns `mctl layout <idx>` (the
//! dispatch path that actually flips the focused output's
//! layout) and emits `CloseMenu` so the drawer collapses.

use mshell_margo_client::read_state_json;
use relm4::{
    Component, ComponentParts, ComponentSender, gtk,
    gtk::glib,
    gtk::prelude::*,
};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;
use tracing::warn;

const ACTIVE_POLL_INTERVAL: Duration = Duration::from_millis(500);

pub(crate) struct MargoLayoutMenuWidgetModel {
    /// Bare button refs so the poll-tick handler can flip CSS
    /// classes by index without walking the GTK widget tree.
    buttons: Rc<RefCell<Vec<gtk::Button>>>,
    /// Last-seen active layout index. Kept across ticks so the
    /// poller only fires a re-render when the value actually
    /// changes (avoids per-frame churn while the menu is open).
    last_active: Rc<RefCell<Option<usize>>>,
    /// Cleanup handle: when the controller drops, the timer
    /// closure is released and the periodic tick stops.
    _timeout: Option<glib::SourceId>,
}

#[derive(Debug)]
pub(crate) enum MargoLayoutMenuWidgetInput {
    /// Set the layout on the focused output via `mctl layout
    /// <idx>`. Triggered by a row click.
    Activate(usize),
    /// Live update from the poll-tick — refresh the `.selected`
    /// class on each row based on the new index.
    LayoutChanged(Option<usize>),
}

#[derive(Debug)]
pub(crate) enum MargoLayoutMenuWidgetOutput {
    /// Tell the host menu to collapse — fired after a successful
    /// row click so the user gets a clean "tap and the menu
    /// closes" UX.
    CloseMenu,
}

pub(crate) struct MargoLayoutMenuWidgetInit {}

#[relm4::component(pub(crate))]
impl Component for MargoLayoutMenuWidgetModel {
    type CommandOutput = ();
    type Input = MargoLayoutMenuWidgetInput;
    type Output = MargoLayoutMenuWidgetOutput;
    type Init = MargoLayoutMenuWidgetInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "margo-layout-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 8,

            gtk::Label {
                add_css_class: "label-large-bold",
                set_label: "Layout",
                set_xalign: 0.0,
            },

            #[name = "row_box"]
            gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 4,
                add_css_class: "margo-layout-menu-widget-rows",
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let buttons_cell: Rc<RefCell<Vec<gtk::Button>>> = Rc::new(RefCell::new(Vec::new()));
        let last_active_cell: Rc<RefCell<Option<usize>>> = Rc::new(RefCell::new(None));

        // Build the row list from state.json so user-customised
        // layout vectors are honoured. Falls back to the wired-in
        // default list when margo hasn't started writing
        // state.json yet (transient on cold session start).
        let layout_names: Vec<String> = read_state_json()
            .map(|s| s.layouts)
            .filter(|v| !v.is_empty())
            .unwrap_or_else(default_layout_names);

        let widgets = view_output!();

        let mut button_vec: Vec<gtk::Button> = Vec::with_capacity(layout_names.len());
        for (idx, name) in layout_names.iter().enumerate() {
            let pretty = pretty_layout_name(name);
            let icon_name = icon_for_layout(name);
            let row = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .spacing(12)
                .build();
            let icon = gtk::Image::from_icon_name(icon_name);
            icon.add_css_class("margo-layout-menu-icon");
            row.append(&icon);
            let label = gtk::Label::new(Some(&pretty));
            label.add_css_class("margo-layout-menu-label");
            label.set_xalign(0.0);
            label.set_hexpand(true);
            row.append(&label);

            let btn = gtk::Button::builder()
                .child(&row)
                .css_classes(["margo-layout-menu-row"])
                .build();
            let s = sender.clone();
            btn.connect_clicked(move |_| {
                s.input(MargoLayoutMenuWidgetInput::Activate(idx));
            });
            widgets.row_box.append(&btn);
            button_vec.push(btn);
        }
        *buttons_cell.borrow_mut() = button_vec;

        // Initial highlight + poll tick.
        let initial = current_active_layout_idx();
        *last_active_cell.borrow_mut() = initial;
        apply_active_class(&buttons_cell.borrow(), initial);

        let sender_tick = sender.clone();
        let last_active_tick = last_active_cell.clone();
        let timeout = glib::timeout_add_local(ACTIVE_POLL_INTERVAL, move || {
            let next = current_active_layout_idx();
            let mut last = last_active_tick.borrow_mut();
            if *last != next {
                *last = next;
                sender_tick.input(MargoLayoutMenuWidgetInput::LayoutChanged(next));
            }
            glib::ControlFlow::Continue
        });

        let model = MargoLayoutMenuWidgetModel {
            buttons: buttons_cell,
            last_active: last_active_cell,
            _timeout: Some(timeout),
        };

        let _ = root; // keep `root` alive in the macro-expanded view
        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            MargoLayoutMenuWidgetInput::Activate(idx) => {
                // Optimistic highlight: paint the new row selected
                // immediately so the click feels snappy. The poll
                // tick will reconcile if margo rejects the dispatch.
                *self.last_active.borrow_mut() = Some(idx);
                apply_active_class(&self.buttons.borrow(), Some(idx));
                tokio::spawn(async move {
                    let mut command = tokio::process::Command::new("mctl");
                    command.arg("layout").arg(idx.to_string());
                    match command.status().await {
                        Ok(s) if s.success() => {}
                        Ok(s) => warn!(?s, idx, "mctl layout returned non-zero"),
                        Err(e) => warn!(error = %e, idx, "mctl layout spawn failed"),
                    }
                });
                let _ = sender.output(MargoLayoutMenuWidgetOutput::CloseMenu);
            }
            MargoLayoutMenuWidgetInput::LayoutChanged(idx) => {
                apply_active_class(&self.buttons.borrow(), idx);
            }
        }
    }
}

/// Read the focused output's `layout_idx` from state.json. Returns
/// `None` when state.json is missing or the index is past the
/// layouts list (transient during config reload).
fn current_active_layout_idx() -> Option<usize> {
    let state = read_state_json()?;
    let focused = state
        .outputs
        .iter()
        .find(|o| o.name == state.active_output)?;
    let idx = focused.layout_idx;
    if idx < state.layouts.len() { Some(idx) } else { None }
}

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
/// (margo not running yet, IPC socket transient). Mirrors the
/// list used by the bar-widget popover variant.
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

/// Per-layout icon hint. `view-list-symbolic` is the generic
/// fallback for layouts whose dedicated icon hasn't been
/// packaged in MargoMaterial / Adwaita.
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
        _ => "view-list-symbolic",
    }
}
