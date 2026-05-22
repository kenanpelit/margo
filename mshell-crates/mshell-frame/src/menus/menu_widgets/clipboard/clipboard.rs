use crate::menus::menu_widgets::clipboard::clipboard_item::ClipboardItemModel;
use mshell_clipboard::{ClipCategory, ClipboardEntry, ClipboardHistory, clipboard_service};
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentController, ComponentParts, ComponentSender, Controller, gtk};
use std::cell::Cell;
use tokio::sync::broadcast;
use tracing::{error, warn};

thread_local! {
    /// True while the `/` filter field is open on the focused
    /// clipboard menu. Lives at module scope (not in the model) so
    /// the frame's Esc handler — which owns the Escape keybind on the
    /// layer surface — can check it and route Esc to "exit search"
    /// instead of "close menu". Only the keyboard-focused surface
    /// receives Esc, and only an open clipboard sets this, so the
    /// flag unambiguously refers to the menu the user is in.
    static SEARCH_ACTIVE: Cell<bool> = const { Cell::new(false) };
}

/// Whether the clipboard `/` filter is currently open. Read by the
/// frame's Esc handler (see `frame.rs`).
pub(crate) fn search_is_active() -> bool {
    SEARCH_ACTIVE.with(|c| c.get())
}

fn set_search_active(active: bool) {
    SEARCH_ACTIVE.with(|c| c.set(active));
}

/// Clipboard menu type tabs. `All` shows the full history; the three
/// type tabs filter by content category; `Favorites` shows pinned
/// entries of any type. Number keys 1–5 jump; Tab cycles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ClipTab {
    All,
    Text,
    Images,
    Files,
    Favorites,
}

impl ClipTab {
    const ALL: [ClipTab; 5] = [
        ClipTab::All,
        ClipTab::Text,
        ClipTab::Images,
        ClipTab::Files,
        ClipTab::Favorites,
    ];

    /// Index into [`Self::ALL`] — also the 1-based number-key slot.
    fn index(self) -> usize {
        Self::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }

    /// Next tab, wrapping — drives the Tab key.
    fn next(self) -> ClipTab {
        Self::ALL[(self.index() + 1) % Self::ALL.len()]
    }

    /// Short label without the count (the count is appended live).
    fn base_label(self) -> &'static str {
        match self {
            ClipTab::All => "All",
            ClipTab::Text => "Text",
            ClipTab::Images => "Images",
            ClipTab::Files => "Files",
            ClipTab::Favorites => "★",
        }
    }

    /// Does this entry belong in this tab?
    fn matches(self, e: &ClipboardEntry) -> bool {
        match self {
            ClipTab::All => true,
            ClipTab::Text => e.category() == ClipCategory::Text,
            ClipTab::Images => e.category() == ClipCategory::Image,
            ClipTab::Files => e.category() == ClipCategory::File,
            ClipTab::Favorites => e.pinned,
        }
    }
}

/// CSS classes for a tab button — `active` when it's the selected tab.
fn tab_classes(active: bool) -> &'static [&'static str] {
    if active {
        &["clipboard-tab", "active"]
    } else {
        &["clipboard-tab"]
    }
}

/// Tab button label: base name + live count, e.g. `Text 12`. The ★
/// favorites tab keeps just its glyph + count.
fn tab_label(tab: ClipTab, counts: &[usize; 5]) -> String {
    format!("{} {}", tab.base_label(), counts[tab.index()])
}

pub(crate) struct ClipboardModel {
    list_box: gtk::ListBox,
    /// Row item controllers, kept alive (their widgets live in the
    /// ListBox). Index-aligned with `items`.
    rows: Vec<Controller<ClipboardItemModel>>,
    /// Currently-displayed entries, in row order — index maps a
    /// selected ListBox row back to a clipboard id.
    items: Vec<ClipboardEntry>,
    history: ClipboardHistory,
    delete_button_visible: bool,
    /// Active type tab.
    active_tab: ClipTab,
    /// Per-tab entry counts (index-aligned with `ClipTab::ALL`),
    /// recomputed from the full history on every rebuild.
    tab_counts: [usize; 5],
    /// Current `/` filter query (lower-cased substring match). The
    /// open/closed state lives in the [`SEARCH_ACTIVE`] thread-local
    /// so the frame's Esc handler can read it.
    search_query: String,
}

#[derive(Debug)]
pub(crate) enum ClipboardInput {
    Refresh,
    DeleteAllClicked,
    /// Jump to a specific type tab (number keys 1–5 / clicks).
    SetTab(ClipTab),
    /// Tab key — cycle to the next type tab.
    CycleTab,
    SelectNext,
    SelectPrev,
    CopySelected,
    DeleteSelected,
    /// Pin / unpin the selected entry (Ctrl+P).
    PinSelected,
    /// The frame's clipboard menu was revealed (`true`) or hidden
    /// (`false`). On reveal we pull keyboard focus into the list so
    /// Tab / Ctrl+n/k / Enter work immediately — without first
    /// needing a mouse click to put focus in the surface.
    ParentRevealChanged(bool),
    /// `/` pressed — open the vim-style filter field and focus it.
    EnterSearch,
    /// Esc while searching — clear the filter and return to the list.
    ExitSearch,
    /// The filter text changed — re-filter the list live.
    SearchChanged(String),
}

#[derive(Debug)]
pub(crate) enum ClipboardOutput {
    CloseMenu,
}

pub(crate) struct ClipboardInit {}

#[derive(Debug)]
pub(crate) enum ClipboardCommandOutput {}

#[relm4::component(pub)]
impl Component for ClipboardModel {
    type CommandOutput = ClipboardCommandOutput;
    type Input = ClipboardInput;
    type Output = ClipboardOutput;
    type Init = ClipboardInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "clipboard-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 12,

            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,

                gtk::Label {
                    add_css_class: "label-medium-bold",
                    set_halign: gtk::Align::Start,
                    set_label: "Clipboard History",
                    set_hexpand: true,
                },

                gtk::Button {
                    add_css_class: "ok-button-surface",
                    set_valign: gtk::Align::Center,
                    connect_clicked[sender] => move |_| {
                        sender.input(ClipboardInput::DeleteAllClicked);
                    },

                    gtk::Label {
                        add_css_class: "label-small",
                        set_label: "Clear all",
                    },
                },
            },

            // Type tabs — All · Text · Images · Files · ★ (favorites),
            // each with a live count. Number keys 1–5 jump; Tab cycles.
            gtk::Box {
                add_css_class: "clipboard-tabs",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 4,
                set_halign: gtk::Align::Start,

                gtk::Button {
                    #[watch]
                    set_css_classes: tab_classes(model.active_tab == ClipTab::All),
                    #[watch]
                    set_label: &tab_label(ClipTab::All, &model.tab_counts),
                    connect_clicked[sender] => move |_| {
                        sender.input(ClipboardInput::SetTab(ClipTab::All));
                    },
                },
                gtk::Button {
                    #[watch]
                    set_css_classes: tab_classes(model.active_tab == ClipTab::Text),
                    #[watch]
                    set_label: &tab_label(ClipTab::Text, &model.tab_counts),
                    connect_clicked[sender] => move |_| {
                        sender.input(ClipboardInput::SetTab(ClipTab::Text));
                    },
                },
                gtk::Button {
                    #[watch]
                    set_css_classes: tab_classes(model.active_tab == ClipTab::Images),
                    #[watch]
                    set_label: &tab_label(ClipTab::Images, &model.tab_counts),
                    connect_clicked[sender] => move |_| {
                        sender.input(ClipboardInput::SetTab(ClipTab::Images));
                    },
                },
                gtk::Button {
                    #[watch]
                    set_css_classes: tab_classes(model.active_tab == ClipTab::Files),
                    #[watch]
                    set_label: &tab_label(ClipTab::Files, &model.tab_counts),
                    connect_clicked[sender] => move |_| {
                        sender.input(ClipboardInput::SetTab(ClipTab::Files));
                    },
                },
                gtk::Button {
                    #[watch]
                    set_css_classes: tab_classes(model.active_tab == ClipTab::Favorites),
                    #[watch]
                    set_label: &tab_label(ClipTab::Favorites, &model.tab_counts),
                    connect_clicked[sender] => move |_| {
                        sender.input(ClipboardInput::SetTab(ClipTab::Favorites));
                    },
                },
            },

            // Vim-style filter field — hidden until `/` is pressed,
            // then slides down and takes focus. Live-filters the list.
            #[name = "search_revealer"]
            gtk::Revealer {
                set_transition_type: gtk::RevealerTransitionType::SlideDown,
                set_transition_duration: 120,

                #[name = "search_entry"]
                gtk::Entry {
                    add_css_class: "ok-entry",
                    add_css_class: "clipboard-search",
                    set_placeholder_text: Some("Filter clipboard…  (Esc to exit)"),
                    set_hexpand: true,
                    connect_changed[sender] => move |entry| {
                        sender.input(ClipboardInput::SearchChanged(entry.text().to_string()));
                    },
                    connect_activate[sender] => move |_| {
                        sender.input(ClipboardInput::CopySelected);
                    },
                },
            },

            gtk::Label {
                add_css_class: "label-small",
                add_css_class: "clipboard-hint",
                set_halign: gtk::Align::Start,
                set_label: "/: search · 1-5/Tab: tabs · Ctrl+n/k: move · Enter: copy · Ctrl+p: pin · Delete: remove",
                set_xalign: 0.0,
            },

            gtk::Label {
                add_css_class: "label-medium",
                #[watch]
                set_visible: !model.delete_button_visible,
                #[watch]
                set_label: match model.active_tab {
                    ClipTab::Favorites => "No favorites yet",
                    _ => "Empty",
                },
            },

            gtk::ScrolledWindow {
                set_vscrollbar_policy: gtk::PolicyType::Automatic,
                set_hscrollbar_policy: gtk::PolicyType::Never,
                set_propagate_natural_height: true,
                set_propagate_natural_width: false,
                set_vexpand: true,

                #[local_ref]
                list_box -> gtk::ListBox {
                    add_css_class: "clipboard-list",
                    set_selection_mode: gtk::SelectionMode::Single,
                    connect_row_activated[sender] => move |_, _| {
                        sender.input(ClipboardInput::CopySelected);
                    },
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let service = clipboard_service();
        let history = service.history().clone();

        // Live refresh on any clipboard event.
        let event_sender = sender.clone();
        sender.command(move |_out, shutdown| async move {
            let service = clipboard_service();
            let mut rx = service.subscribe();
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);

            loop {
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    result = rx.recv() => {
                        match result {
                            Ok(_) => event_sender.input(ClipboardInput::Refresh),
                            Err(broadcast::error::RecvError::Lagged(n)) => {
                                warn!("Clipboard panel missed {n} events, refreshing");
                                event_sender.input(ClipboardInput::Refresh);
                            }
                            Err(broadcast::error::RecvError::Closed) => {
                                error!("Clipboard broadcast channel closed");
                                break;
                            }
                        }
                    }
                }
            }
        });

        let list_box = gtk::ListBox::new();

        // Keyboard control on the menu root (clipse-style): Tab to
        // switch tabs, Ctrl+n / Ctrl+k to move, Enter to copy, Delete
        // to remove, `/` to open the filter (vim). Arrow keys are
        // handled natively by the ListBox in normal mode.
        //
        // The controller runs in the *Capture* phase so it sees keys
        // before the focused search entry. That lets nav shortcuts
        // (Ctrl+n/k, Enter, …) keep working while typing a filter,
        // while plain characters fall through (`Proceed`) into the
        // entry. `search_active` decides the few keys whose meaning
        // flips between modes: `/`, Esc and Backspace/Delete.
        let key_sender = sender.clone();
        let key = gtk::EventControllerKey::new();
        key.set_propagation_phase(gtk::PropagationPhase::Capture);
        key.connect_key_pressed(move |_, keyval, _, modifier| {
            let ctrl = modifier.contains(gtk::gdk::ModifierType::CONTROL_MASK);
            let searching = search_is_active();
            match keyval {
                // `/` opens the filter — but only in normal mode; while
                // searching it's a literal character for the entry.
                gtk::gdk::Key::slash if !searching && !ctrl => {
                    key_sender.input(ClipboardInput::EnterSearch);
                    gtk::glib::Propagation::Stop
                }
                // Esc closes the filter while searching; otherwise let
                // the frame handle it (close the menu).
                gtk::gdk::Key::Escape => {
                    if searching {
                        key_sender.input(ClipboardInput::ExitSearch);
                        gtk::glib::Propagation::Stop
                    } else {
                        gtk::glib::Propagation::Proceed
                    }
                }
                gtk::gdk::Key::Tab | gtk::gdk::Key::ISO_Left_Tab => {
                    key_sender.input(ClipboardInput::CycleTab);
                    gtk::glib::Propagation::Stop
                }
                // Number keys 1–5 jump straight to a tab — but only in
                // normal mode (while searching they're literal digits).
                gtk::gdk::Key::_1 | gtk::gdk::Key::_2 | gtk::gdk::Key::_3
                | gtk::gdk::Key::_4 | gtk::gdk::Key::_5
                    if !searching && !ctrl =>
                {
                    let idx = (keyval.to_unicode().and_then(|c| c.to_digit(10)).unwrap_or(1)
                        as usize)
                        .saturating_sub(1);
                    if let Some(tab) = ClipTab::ALL.get(idx) {
                        key_sender.input(ClipboardInput::SetTab(*tab));
                    }
                    gtk::glib::Propagation::Stop
                }
                gtk::gdk::Key::n if ctrl => {
                    key_sender.input(ClipboardInput::SelectNext);
                    gtk::glib::Propagation::Stop
                }
                gtk::gdk::Key::k if ctrl => {
                    key_sender.input(ClipboardInput::SelectPrev);
                    gtk::glib::Propagation::Stop
                }
                // Arrow keys drive the list explicitly while searching
                // (the focused entry would otherwise eat them as cursor
                // moves). In normal mode the ListBox handles them.
                gtk::gdk::Key::Down if searching => {
                    key_sender.input(ClipboardInput::SelectNext);
                    gtk::glib::Propagation::Stop
                }
                gtk::gdk::Key::Up if searching => {
                    key_sender.input(ClipboardInput::SelectPrev);
                    gtk::glib::Propagation::Stop
                }
                gtk::gdk::Key::p if ctrl => {
                    key_sender.input(ClipboardInput::PinSelected);
                    gtk::glib::Propagation::Stop
                }
                // Backspace/Delete remove an entry in normal mode, but
                // edit the query while searching.
                gtk::gdk::Key::Delete | gtk::gdk::Key::BackSpace => {
                    if searching {
                        gtk::glib::Propagation::Proceed
                    } else {
                        key_sender.input(ClipboardInput::DeleteSelected);
                        gtk::glib::Propagation::Stop
                    }
                }
                gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter => {
                    key_sender.input(ClipboardInput::CopySelected);
                    gtk::glib::Propagation::Stop
                }
                _ => gtk::glib::Propagation::Proceed,
            }
        });
        root.add_controller(key);

        let model = ClipboardModel {
            list_box: list_box.clone(),
            rows: Vec::new(),
            items: Vec::new(),
            history,
            delete_button_visible: false,
            active_tab: ClipTab::All,
            tab_counts: [0; 5],
            search_query: String::new(),
        };

        let widgets = view_output!();

        // Populate immediately so the list + active tab reflect
        // current history on first open, not just after an event.
        sender.input(ClipboardInput::Refresh);

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
            ClipboardInput::Refresh => self.rebuild_rows(),
            ClipboardInput::SetTab(tab) => {
                self.active_tab = tab;
                self.rebuild_rows();
            }
            ClipboardInput::CycleTab => {
                self.active_tab = self.active_tab.next();
                self.rebuild_rows();
            }
            ClipboardInput::SelectNext => self.move_selection(1),
            ClipboardInput::SelectPrev => self.move_selection(-1),
            ClipboardInput::CopySelected => {
                if let Some(id) = self.selected_id() {
                    let kind = self
                        .items
                        .iter()
                        .find(|e| e.id == id)
                        .map(|e| e.category());
                    clipboard_service().copy_entry(id);
                    let body = match kind {
                        Some(ClipCategory::Image) => "Image copied to clipboard",
                        Some(ClipCategory::File) => "File copied to clipboard",
                        _ => "Text copied to clipboard",
                    };
                    mshell_launcher::notify::toast("Copied", body);
                    let _ = sender.output(ClipboardOutput::CloseMenu);
                }
            }
            ClipboardInput::DeleteSelected => {
                if let Some(id) = self.selected_id() {
                    clipboard_service().delete_entry(id);
                    // broadcast → Refresh rebuilds the list.
                }
            }
            ClipboardInput::PinSelected => {
                if let Some(id) = self.selected_id() {
                    let was_pinned = self
                        .items
                        .iter()
                        .find(|e| e.id == id)
                        .map(|e| e.pinned)
                        .unwrap_or(false);
                    clipboard_service().toggle_pin(id);
                    if was_pinned {
                        mshell_launcher::notify::toast("Unpinned", "Removed from favorites");
                    } else {
                        mshell_launcher::notify::toast("Pinned", "Added to favorites");
                    }
                    // broadcast → Refresh; the entry hops between tabs.
                }
            }
            ClipboardInput::DeleteAllClicked => {
                clipboard_service().clear_history();
                mshell_launcher::notify::toast("Clipboard cleared", "All entries removed");
                let _ = sender.output(ClipboardOutput::CloseMenu);
            }
            ClipboardInput::EnterSearch => {
                set_search_active(true);
                widgets.search_revealer.set_reveal_child(true);
                // Defer the focus grab so the just-revealed entry has
                // mapped/allocated before we focus it.
                let entry = widgets.search_entry.clone();
                gtk::glib::idle_add_local_once(move || {
                    entry.grab_focus();
                });
            }
            ClipboardInput::ExitSearch => {
                set_search_active(false);
                self.search_query.clear();
                widgets.search_entry.set_text("");
                widgets.search_revealer.set_reveal_child(false);
                // Rebuild with the cleared filter; search_active is now
                // false so this re-grabs focus into the list.
                self.rebuild_rows();
            }
            ClipboardInput::SearchChanged(text) => {
                self.search_query = text;
                self.rebuild_rows();
            }
            ClipboardInput::ParentRevealChanged(revealed) => {
                if revealed {
                    // Open fresh: any stale filter from a previous
                    // session is cleared so the full history shows.
                    set_search_active(false);
                    self.search_query.clear();
                    widgets.search_entry.set_text("");
                    widgets.search_revealer.set_reveal_child(false);
                    // Re-sync to current history, then defer the focus
                    // grab to an idle tick so the layer surface has
                    // finished mapping/allocating — grabbing focus on a
                    // not-yet-mapped row silently no-ops, which is the
                    // exact "keys dead until I click" symptom.
                    self.rebuild_rows();
                    let list_box = self.list_box.clone();
                    gtk::glib::idle_add_local_once(move || {
                        if let Some(row) = list_box.row_at_index(0) {
                            list_box.select_row(Some(&row));
                            row.grab_focus();
                        }
                    });
                }
            }
        }

        self.update_view(widgets, sender);
    }
}

impl ClipboardModel {
    /// Rebuild the ListBox rows from the (filtered) history.
    fn rebuild_rows(&mut self) {
        // The active type tab selects which entries show; the `/`
        // filter then narrows them by a case-insensitive substring of
        // the entry's full content. Per-tab counts come from the full
        // history (search-independent), so the tab strip always shows
        // how much each category holds.
        let all = self.history.entries();
        let mut counts = [0usize; ClipTab::ALL.len()];
        for e in &all {
            for (i, tab) in ClipTab::ALL.iter().enumerate() {
                if tab.matches(e) {
                    counts[i] += 1;
                }
            }
        }
        self.tab_counts = counts;

        let query = self.search_query.to_lowercase();
        let active_tab = self.active_tab;
        let items: Vec<ClipboardEntry> = all
            .into_iter()
            .filter(|e| active_tab.matches(e))
            .filter(|e| query.is_empty() || e.search_haystack().contains(&query))
            .collect();

        // Tear down old rows.
        while let Some(child) = self.list_box.first_child() {
            self.list_box.remove(&child);
        }
        self.rows.clear();

        for item in &items {
            let controller: Controller<ClipboardItemModel> =
                ClipboardItemModel::builder().launch(item.clone()).detach();
            self.list_box.append(controller.widget());
            self.rows.push(controller);
        }

        self.delete_button_visible = !items.is_empty();
        self.items = items;

        // Keep a selection so keyboard control has an anchor, and
        // pull focus into the list (when mapped) so Ctrl+n/k / Enter
        // / Delete land here as soon as the menu is open. While the
        // `/` filter is active, DON'T grab focus — that would steal
        // it from the search entry mid-keystroke. The selection still
        // tracks row 0 so Ctrl+n/k / Enter operate on the top match.
        if let Some(row) = self.list_box.row_at_index(0) {
            self.list_box.select_row(Some(&row));
            if self.list_box.is_mapped() && !search_is_active() {
                row.grab_focus();
            }
        }
    }

    /// Clipboard id of the currently-selected row, if any.
    fn selected_id(&self) -> Option<u64> {
        let idx = self.list_box.selected_row()?.index();
        if idx < 0 {
            return None;
        }
        self.items.get(idx as usize).map(|e| e.id)
    }

    /// Move selection by `delta` rows, clamped to the list bounds,
    /// and scroll it into view.
    fn move_selection(&self, delta: i32) {
        let len = self.items.len() as i32;
        if len == 0 {
            return;
        }
        let cur = self
            .list_box
            .selected_row()
            .map(|r| r.index())
            .unwrap_or(0);
        let next = (cur + delta).clamp(0, len - 1);
        if let Some(row) = self.list_box.row_at_index(next) {
            self.list_box.select_row(Some(&row));
            row.grab_focus();
        }
    }
}
