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
    Component, ComponentParts, ComponentSender, gtk, gtk::gdk, gtk::glib, gtk::prelude::*,
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
    /// `buttons[i]`'s canonical layout index (its position in
    /// `state.layouts` — what `mctl layout <N>` wants). Rows are
    /// displayed in `circle_layout` order, so a row's screen position
    /// no longer equals its layout index.
    row_canonical: Vec<usize>,
    /// Last-seen active layout index. Kept across ticks so the
    /// poller only fires a re-render when the value actually
    /// changes (avoids per-frame churn while the menu is open).
    last_active: Rc<RefCell<Option<usize>>>,
    /// Cleanup handle: when the controller drops, the timer
    /// closure is released and the periodic tick stops.
    _timeout: Option<glib::SourceId>,
    /// Row index currently held by the keyboard focus-walk. Tracked
    /// in the model (not GTK) so Tab / Ctrl+N wrap deterministically
    /// even when no row has grabbed focus yet — same scheme as the
    /// session menu.
    focused: usize,
}

#[derive(Debug)]
pub(crate) enum MargoLayoutMenuWidgetInput {
    /// Set the layout on the focused output via `mctl layout <idx>`,
    /// where `idx` is the *canonical* layout index (`state.layouts`
    /// position), not the row's on-screen position. Triggered by a
    /// row click.
    Activate(usize),
    /// Live update from the poll-tick — refresh the `.selected`
    /// class on each row based on the new index.
    LayoutChanged(Option<usize>),
    /// Move keyboard focus to the next row (Tab / Down / Ctrl+N /
    /// Ctrl+J), wrapping at the end.
    FocusNext,
    /// Move keyboard focus to the previous row (Shift+Tab / Up /
    /// Ctrl+P / Ctrl+K), wrapping at the start.
    FocusPrev,
    /// The host menu was revealed / hidden. On reveal we grab keyboard
    /// focus onto a row so the focus-walk controller starts receiving
    /// Tab / Ctrl+N — without a focused descendant the root controller
    /// stays dormant. Mirrors the session menu.
    ParentRevealChanged(bool),
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

            // ── §12 panel header ──
            gtk::Box {
                add_css_class: "panel-header",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 12,
                gtk::Image {
                    add_css_class: "panel-header-icon",
                    set_valign: gtk::Align::Center,
                    set_icon_name: Some("layout-symbolic"),
                },
                gtk::Label {
                    add_css_class: "panel-title",
                    set_label: "Layout",
                    set_halign: gtk::Align::Start,
                    set_hexpand: true,
                    set_valign: gtk::Align::Center,
                },
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
        // layout vectors are honoured, ordered by the compositor's
        // `circle_layout` so the menu matches the layout-cycle
        // keybind. Falls back to the wired-in default list (canonical
        // order) when margo hasn't answered `get state` yet —
        // transient on cold session start.
        let (layout_names, circle) = read_state_json()
            .map(|s| (s.layouts, s.circle_layouts))
            .filter(|(v, _)| !v.is_empty())
            .unwrap_or_else(|| (default_layout_names(), Vec::new()));
        let ordered = ordered_layouts(&layout_names, &circle);

        let widgets = view_output!();

        let mut button_vec: Vec<gtk::Button> = Vec::with_capacity(ordered.len());
        let mut row_canonical: Vec<usize> = Vec::with_capacity(ordered.len());
        for (name, canonical_idx) in &ordered {
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
            let ci = *canonical_idx;
            btn.connect_clicked(move |_| {
                s.input(MargoLayoutMenuWidgetInput::Activate(ci));
            });
            widgets.row_box.append(&btn);
            button_vec.push(btn);
            row_canonical.push(*canonical_idx);
        }
        *buttons_cell.borrow_mut() = button_vec;

        // Initial highlight + poll tick.
        let initial = current_active_layout_idx();
        *last_active_cell.borrow_mut() = initial;
        apply_active_class(&buttons_cell.borrow(), &row_canonical, initial);

        // The send must be fallible and must stop the source on failure.
        // Holding the `SourceId` only stops the timer when the model is
        // explicitly dropped, but a relm4 controller teardown doesn't
        // guarantee Drop-then-next-tick ordering, and an in-flight tick on a
        // half-dropped controller would abort mshell with "The runtime of the
        // component was shutdown" — taking the whole shell with it. The bar
        // twin of this poller (bars/bar_widgets/margo_layout.rs) does the same.
        let sender_tick = sender.clone();
        let last_active_tick = last_active_cell.clone();
        let timeout = glib::timeout_add_local(ACTIVE_POLL_INTERVAL, move || {
            let next = current_active_layout_idx();
            let mut last = last_active_tick.borrow_mut();
            if *last != next
                && sender_tick
                    .input_sender()
                    .send(MargoLayoutMenuWidgetInput::LayoutChanged(next))
                    .is_err()
            {
                return glib::ControlFlow::Break;
            }
            *last = next;
            glib::ControlFlow::Continue
        });

        // Keyboard focus-walk (Tab / Shift+Tab, the arrow keys, Ctrl+N /
        // Ctrl+P, Ctrl+J / Ctrl+K) — via a Capture-phase ShortcutController,
        // the same mechanism the session menu uses for its number keys. An
        // EventControllerKey only fires while a *descendant* of its widget
        // holds keyboard focus; on a freshly-revealed layer-shell menu that
        // grab is fragile (focus can sit on the menu's ScrolledWindow instead
        // of a row), leaving the controller dormant. A ShortcutController's
        // KeyvalTrigger instead fires as long as the layer *surface* holds
        // keyboard focus — which `sync_keyboard_mode` guarantees (Exclusive)
        // while any menu is revealed — so it doesn't depend on the grab
        // landing. Capture phase so Tab preempts GTK's built-in focus-move.
        {
            let nav = |key: gdk::Key, mods: gdk::ModifierType, next: bool| {
                let s = sender.clone();
                gtk::Shortcut::builder()
                    .trigger(&gtk::KeyvalTrigger::new(key, mods))
                    .action(&gtk::CallbackAction::new(move |_, _| {
                        s.input(if next {
                            MargoLayoutMenuWidgetInput::FocusNext
                        } else {
                            MargoLayoutMenuWidgetInput::FocusPrev
                        });
                        glib::Propagation::Stop
                    }))
                    .build()
            };
            let sc = gtk::ShortcutController::new();
            sc.set_scope(gtk::ShortcutScope::Local);
            sc.set_propagation_phase(gtk::PropagationPhase::Capture);
            let e = gdk::ModifierType::empty();
            let ctrl = gdk::ModifierType::CONTROL_MASK;
            let shift = gdk::ModifierType::SHIFT_MASK;
            sc.add_shortcut(nav(gdk::Key::Tab, e, true));
            sc.add_shortcut(nav(gdk::Key::Down, e, true));
            sc.add_shortcut(nav(gdk::Key::n, ctrl, true));
            sc.add_shortcut(nav(gdk::Key::j, ctrl, true));
            sc.add_shortcut(nav(gdk::Key::ISO_Left_Tab, e, false));
            sc.add_shortcut(nav(gdk::Key::ISO_Left_Tab, shift, false));
            sc.add_shortcut(nav(gdk::Key::Tab, shift, false));
            sc.add_shortcut(nav(gdk::Key::Up, e, false));
            sc.add_shortcut(nav(gdk::Key::p, ctrl, false));
            sc.add_shortcut(nav(gdk::Key::k, ctrl, false));
            root.add_controller(sc);
        }

        let model = MargoLayoutMenuWidgetModel {
            buttons: buttons_cell,
            row_canonical,
            last_active: last_active_cell,
            _timeout: Some(timeout),
            focused: 0,
        };

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            MargoLayoutMenuWidgetInput::Activate(idx) => {
                // `idx` is the canonical layout index — what `mctl
                // layout` wants, and what the poll tick reports back.
                // Optimistic highlight: paint the matching row selected
                // immediately so the click feels snappy. The poll tick
                // will reconcile if margo rejects the dispatch.
                *self.last_active.borrow_mut() = Some(idx);
                apply_active_class(&self.buttons.borrow(), &self.row_canonical, Some(idx));
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
                apply_active_class(&self.buttons.borrow(), &self.row_canonical, idx);
            }
            MargoLayoutMenuWidgetInput::FocusNext => {
                let buttons = self.buttons.borrow();
                if !buttons.is_empty() {
                    self.focused = (self.focused + 1) % buttons.len();
                    buttons[self.focused].grab_focus();
                }
            }
            MargoLayoutMenuWidgetInput::FocusPrev => {
                let buttons = self.buttons.borrow();
                if !buttons.is_empty() {
                    self.focused = (self.focused + buttons.len() - 1) % buttons.len();
                    buttons[self.focused].grab_focus();
                }
            }
            MargoLayoutMenuWidgetInput::ParentRevealChanged(revealed) => {
                if revealed {
                    self.focused = 0;
                    // The layer-shell surface only takes keyboard focus after
                    // the frame's `sync_keyboard_mode` debounce; grabbing a row
                    // synchronously here sets the window focus pointer but it
                    // doesn't stick. Re-grab once the surface is actually
                    // keyboard-focused so a row shows the focus ring and Enter
                    // activates it (the ShortcutController above handles the
                    // walk regardless). Mirrors the session menu.
                    if let Some(first) = self.buttons.borrow().first().cloned() {
                        glib::timeout_add_local_once(Duration::from_millis(160), move || {
                            first.grab_focus();
                        });
                    }
                }
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
    if idx < state.layouts.len() {
        Some(idx)
    } else {
        None
    }
}

/// Highlight the row whose *canonical* layout index matches `active`.
/// Rows are shown in `circle_layout` order, so `row_canonical[i]` — not
/// `i` — is the layout each row stands for.
fn apply_active_class(buttons: &[gtk::Button], row_canonical: &[usize], active: Option<usize>) {
    for (i, button) in buttons.iter().enumerate() {
        if active.is_some() && row_canonical.get(i).copied() == active {
            button.add_css_class("selected");
        } else {
            button.remove_css_class("selected");
        }
    }
}

/// Row order for the menu: `circle_layout` order first — each layout name
/// paired with its canonical index in `layouts` (the number `mctl layout
/// <N>` expects) — then any layout not named in `circle_layout`, in
/// canonical order. An empty `circle` (the knob unset) leaves the
/// canonical order untouched. A `circle` entry naming a layout that
/// doesn't exist is skipped.
fn ordered_layouts(layouts: &[String], circle: &[String]) -> Vec<(String, usize)> {
    let mut out: Vec<(String, usize)> = Vec::with_capacity(layouts.len());
    let mut placed = vec![false; layouts.len()];
    for name in circle {
        if let Some(idx) = layouts.iter().position(|l| l == name)
            && !placed[idx]
        {
            placed[idx] = true;
            out.push((layouts[idx].clone(), idx));
        }
    }
    for (idx, name) in layouts.iter().enumerate() {
        if !placed[idx] {
            out.push((name.clone(), idx));
        }
    }
    out
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
        "tgmix",
        "dwindle",
        "floating",
        "mosaic",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

fn pretty_layout_name(id: &str) -> String {
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
        "scroller" => "layout-scrolling-symbolic",
        "grid" => "layout-grid-symbolic",
        "monocle" => "layout-monocle-symbolic",
        "deck" => "layout-deck-symbolic",
        "center_tile" => "layout-center-symbolic",
        "right_tile" => "layout-right-symbolic",
        "tgmix" => "layout-mix-symbolic",
        "dwindle" => "layout-dwindle-symbolic",
        // No dedicated icon packaged for "floating" or "mosaic" yet —
        // both fall through to the generic fallback above.
        _ => "view-list-symbolic",
    }
}

#[cfg(test)]
mod tests {
    use super::ordered_layouts;

    fn v(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn empty_circle_keeps_canonical_order() {
        let layouts = v(&["tile", "scroller", "grid"]);
        assert_eq!(
            ordered_layouts(&layouts, &[]),
            vec![
                ("tile".into(), 0),
                ("scroller".into(), 1),
                ("grid".into(), 2)
            ]
        );
    }

    #[test]
    fn circle_order_wins_and_carries_the_canonical_index() {
        let layouts = v(&["tile", "scroller", "grid", "monocle"]);
        let circle = v(&["grid", "tile", "monocle", "scroller"]);
        assert_eq!(
            ordered_layouts(&layouts, &circle),
            vec![
                ("grid".into(), 2),
                ("tile".into(), 0),
                ("monocle".into(), 3),
                ("scroller".into(), 1),
            ]
        );
    }

    #[test]
    fn layouts_missing_from_circle_are_appended_in_canonical_order() {
        let layouts = v(&["tile", "scroller", "grid", "monocle", "deck"]);
        let circle = v(&["deck", "tile"]);
        assert_eq!(
            ordered_layouts(&layouts, &circle),
            vec![
                ("deck".into(), 4),
                ("tile".into(), 0),
                ("scroller".into(), 1),
                ("grid".into(), 2),
                ("monocle".into(), 3),
            ]
        );
    }

    #[test]
    fn unknown_or_duplicate_circle_entries_are_ignored() {
        let layouts = v(&["tile", "scroller"]);
        let circle = v(&["bogus", "scroller", "scroller", "tile"]);
        assert_eq!(
            ordered_layouts(&layouts, &circle),
            vec![("scroller".into(), 1), ("tile".into(), 0)]
        );
    }
}
