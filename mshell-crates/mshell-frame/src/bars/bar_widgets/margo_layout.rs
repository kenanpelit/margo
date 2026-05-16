//! Margo layout switcher bar pill.
//!
//! Pure trigger button — clicking emits `MargoLayoutOutput::Clicked`
//! which the bar forwards through `BarOutput::MargoLayoutClicked` →
//! `FrameInput::ToggleMargoLayoutMenu`, opening the new in-frame
//! Margo Layout menu (see
//! `menus/menu_widgets/margo_layout/margo_layout_menu_widget.rs`).
//!
//! Replaces the legacy `gtk::PopoverMenu` implementation that
//! spawned an `xdg_popup` window detached from the bar. The new
//! menu lives in the frame's regular menu stack so it slides out
//! contiguous with every other menu surface.
//!
//! The icon reflects whichever layout is currently active on the
//! focused output (polled every 500 ms from state.json) so the
//! pill is glanceable: at-a-cursor you can read the layout
//! without opening the menu.

use mshell_margo_client::read_state_json;
use relm4::{
    Component, ComponentParts, ComponentSender, gtk,
    gtk::Orientation,
    gtk::glib,
    gtk::prelude::*,
};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

const ACTIVE_POLL_INTERVAL: Duration = Duration::from_millis(500);

pub(crate) struct MargoLayoutModel {
    /// Last-seen active layout index. Used by the poll tick to
    /// skip GTK calls when nothing actually changed.
    last_active: Rc<RefCell<Option<usize>>>,
    /// Keeps the periodic poll alive — drops with the controller.
    _timeout: Option<glib::SourceId>,
}

#[derive(Debug)]
pub(crate) enum MargoLayoutInput {
    /// Periodic refresh — recompute the active layout index and
    /// update the icon if it changed.
    Tick,
    /// User clicked the pill — propagate via output to the bar +
    /// frame, which opens / closes the layout menu.
    Clicked,
}

#[derive(Debug)]
pub(crate) enum MargoLayoutOutput {
    /// Bar listens for this and emits
    /// `BarOutput::MargoLayoutClicked`. Frame then toggles the
    /// in-stack `MARGO_LAYOUT_MENU` surface.
    Clicked,
}

pub(crate) struct MargoLayoutInit {
    /// Kept on the init for bar-side API parity (every bar widget
    /// receives the bar's orientation). The pill itself is a
    /// single icon button so orientation doesn't affect layout
    /// here.
    pub(crate) orientation: Orientation,
}

#[relm4::component(pub(crate))]
impl Component for MargoLayoutModel {
    type CommandOutput = ();
    type Input = MargoLayoutInput;
    type Output = MargoLayoutOutput;
    type Init = MargoLayoutInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "margo-layout-bar-widget",
            set_halign: gtk::Align::Center,
            set_valign: gtk::Align::Center,

            #[name = "button"]
            gtk::Button {
                set_css_classes: &["ok-button-surface", "ok-bar-widget"],
                set_hexpand: false,
                set_vexpand: false,
                set_icon_name: "view-list-symbolic",
                connect_clicked[sender] => move |_| {
                    sender.input(MargoLayoutInput::Clicked);
                },
            }
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let last_active_cell: Rc<RefCell<Option<usize>>> = Rc::new(RefCell::new(None));

        let widgets = view_output!();

        // Seed the initial icon from state.json before the first
        // poll tick fires so the pill never flashes the generic
        // `view-list-symbolic` placeholder when the real layout
        // is already known.
        let initial = current_active_layout_idx();
        *last_active_cell.borrow_mut() = initial;
        if let Some(idx) = initial {
            apply_active_icon(&widgets.button, idx);
        }

        // Per-output active-layout poller — 500 ms tick, fires
        // `Tick` if the live layout index differs from the last
        // observation. Cheap enough to leave on permanently:
        // state.json reads are <1 ms.
        let sender_tick = sender.clone();
        let timeout = glib::timeout_add_local(ACTIVE_POLL_INTERVAL, move || {
            sender_tick.input(MargoLayoutInput::Tick);
            glib::ControlFlow::Continue
        });

        // `params.orientation` is part of the bar-widget init
        // ABI but isn't used here — the pill is a single icon.
        let _ = params.orientation;
        let model = MargoLayoutModel {
            last_active: last_active_cell,
            _timeout: Some(timeout),
        };

        let _ = root;
        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            MargoLayoutInput::Tick => {
                let next = current_active_layout_idx();
                let mut last = self.last_active.borrow_mut();
                if *last != next {
                    *last = next;
                    if let Some(idx) = next {
                        apply_active_icon(&widgets.button, idx);
                    }
                }
            }
            MargoLayoutInput::Clicked => {
                let _ = sender.output(MargoLayoutOutput::Clicked);
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

/// Read the layout name at `idx` from state.json (or fall back to
/// the wired-in default list when state.json is unavailable) and
/// apply the matching symbolic icon to the bar button so the pill
/// always reflects the live layout.
fn apply_active_icon(button: &gtk::Button, idx: usize) {
    let name = read_state_json()
        .and_then(|s| s.layouts.get(idx).cloned())
        .unwrap_or_else(|| default_layout_names().get(idx).cloned().unwrap_or_default());
    button.set_icon_name(icon_for_layout(&name));
}

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
