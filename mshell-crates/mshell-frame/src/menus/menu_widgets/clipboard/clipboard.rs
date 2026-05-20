use crate::menus::menu_widgets::clipboard::clipboard_item::ClipboardItemModel;
use mshell_clipboard::{ClipboardEntry, ClipboardHistory, clipboard_service};
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentController, ComponentParts, ComponentSender, Controller, gtk};
use tokio::sync::broadcast;
use tracing::{error, warn};

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
    /// Active tab: false = All (unpinned), true = Favorites (pinned).
    show_pinned_only: bool,
}

#[derive(Debug)]
pub(crate) enum ClipboardInput {
    Refresh,
    DeleteAllClicked,
    /// Switch tab. `true` = Favorites (pinned), `false` = All.
    SetPinnedFilter(bool),
    /// Tab key — flip between All and Favorites.
    ToggleTab,
    SelectNext,
    SelectPrev,
    CopySelected,
    DeleteSelected,
    /// Pin / unpin the selected entry (Ctrl+P).
    PinSelected,
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

            // Tab strip — All (unpinned) vs Favorites (pinned). Tab
            // key toggles; the active tab is highlighted.
            gtk::Box {
                add_css_class: "clipboard-tabs",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 4,
                set_halign: gtk::Align::Start,

                gtk::Button {
                    #[watch]
                    set_css_classes: if model.show_pinned_only {
                        &["clipboard-tab"]
                    } else {
                        &["clipboard-tab", "active"]
                    },
                    set_label: "All",
                    connect_clicked[sender] => move |_| {
                        sender.input(ClipboardInput::SetPinnedFilter(false));
                    },
                },
                gtk::Button {
                    #[watch]
                    set_css_classes: if model.show_pinned_only {
                        &["clipboard-tab", "active"]
                    } else {
                        &["clipboard-tab"]
                    },
                    set_label: "★ Favorites",
                    connect_clicked[sender] => move |_| {
                        sender.input(ClipboardInput::SetPinnedFilter(true));
                    },
                },
            },

            gtk::Label {
                add_css_class: "label-small",
                set_halign: gtk::Align::Start,
                set_label: "Tab: switch · Ctrl+n/k: move · Enter: copy · Ctrl+p: pin · Delete: remove",
                set_xalign: 0.0,
            },

            gtk::Label {
                add_css_class: "label-medium",
                #[watch]
                set_visible: !model.delete_button_visible,
                set_label: if model.show_pinned_only { "No favorites yet" } else { "Empty" },
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
        // to remove. Arrow keys are handled natively by the ListBox.
        let key_sender = sender.clone();
        let key = gtk::EventControllerKey::new();
        key.connect_key_pressed(move |_, keyval, _, modifier| {
            let ctrl = modifier.contains(gtk::gdk::ModifierType::CONTROL_MASK);
            match keyval {
                gtk::gdk::Key::Tab | gtk::gdk::Key::ISO_Left_Tab => {
                    key_sender.input(ClipboardInput::ToggleTab);
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
                gtk::gdk::Key::p if ctrl => {
                    key_sender.input(ClipboardInput::PinSelected);
                    gtk::glib::Propagation::Stop
                }
                gtk::gdk::Key::Delete | gtk::gdk::Key::BackSpace => {
                    key_sender.input(ClipboardInput::DeleteSelected);
                    gtk::glib::Propagation::Stop
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
            show_pinned_only: false,
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
            ClipboardInput::SetPinnedFilter(pinned_only) => {
                self.show_pinned_only = pinned_only;
                self.rebuild_rows();
            }
            ClipboardInput::ToggleTab => {
                self.show_pinned_only = !self.show_pinned_only;
                self.rebuild_rows();
            }
            ClipboardInput::SelectNext => self.move_selection(1),
            ClipboardInput::SelectPrev => self.move_selection(-1),
            ClipboardInput::CopySelected => {
                if let Some(id) = self.selected_id() {
                    clipboard_service().copy_entry(id);
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
                    clipboard_service().toggle_pin(id);
                    // broadcast → Refresh; the entry hops between
                    // the All and Favorites tabs accordingly.
                }
            }
            ClipboardInput::DeleteAllClicked => {
                clipboard_service().clear_history();
                let _ = sender.output(ClipboardOutput::CloseMenu);
            }
        }

        self.update_view(widgets, sender);
    }
}

impl ClipboardModel {
    /// Rebuild the ListBox rows from the (filtered) history.
    fn rebuild_rows(&mut self) {
        // All = unpinned only; Favorites = pinned only. Favourites
        // live solely under their own tab so the All view stays the
        // rolling history without the pinned ones doubling up.
        let items: Vec<ClipboardEntry> = self
            .history
            .entries()
            .into_iter()
            .filter(|e| e.pinned == self.show_pinned_only)
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
        // / Delete land here as soon as the menu is open.
        if let Some(row) = self.list_box.row_at_index(0) {
            self.list_box.select_row(Some(&row));
            if self.list_box.is_mapped() {
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
