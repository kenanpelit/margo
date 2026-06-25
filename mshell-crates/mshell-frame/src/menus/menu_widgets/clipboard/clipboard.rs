use mshell_clipboard::{
    ClipCategory, ClipboardHistory, EntryPreview, EntryView, clipboard_service,
};
use mshell_config::config_manager::config_manager;
use mshell_config::schema::clipboard::{ClipboardDensity, ClipboardStoreFields};
use mshell_config::schema::config::{ConfigStoreFields, MenuStoreFields, MenusStoreFields};
use reactive_graph::traits::GetUntracked;
use relm4::gtk::prelude::*;
use relm4::gtk::{gdk, gio, glib};
use relm4::{Component, ComponentParts, ComponentSender, RelmWidgetExt, gtk};

use crate::menus::menu_widgets::app_launcher::launcher_row::{
    match_accent_value, resolve_primary_var, set_match_accent,
};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use time::OffsetDateTime;
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

    /// Does a row with this category / pin state belong in this tab?
    fn matches_cat(self, cat: ClipCategory, pinned: bool) -> bool {
        match self {
            ClipTab::All => true,
            // Text tab = the whole text family (plain text + the
            // refined URL / colour / code / email categories).
            ClipTab::Text => matches!(
                cat,
                ClipCategory::Text
                    | ClipCategory::Url
                    | ClipCategory::Color
                    | ClipCategory::Code
                    | ClipCategory::Email
            ),
            ClipTab::Images => cat == ClipCategory::Image,
            ClipTab::Files => cat == ClipCategory::File,
            ClipTab::Favorites => pinned,
        }
    }

    /// Symbolic icon name for a content category — drives the per-row
    /// type glyph.
    pub(crate) fn category_icon(cat: ClipCategory) -> &'static str {
        match cat {
            ClipCategory::Url => "web-browser-symbolic",
            ClipCategory::Color => "color-select-symbolic",
            ClipCategory::Code => "text-x-script-symbolic",
            ClipCategory::Email => "mail-message-symbolic",
            ClipCategory::Image => "image-x-generic-symbolic",
            ClipCategory::File => "text-x-generic-symbolic",
            ClipCategory::Text => "edit-paste-symbolic",
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

/// Toggle the `compact` row-density class on the menu root from the
/// configured `clipboard.density`. Read on init + each reveal so a
/// Settings change applies on the next open without a restart.
fn apply_density(root: &gtk::Box) {
    let compact = config_manager()
        .config()
        .clipboard()
        .density()
        .get_untracked()
        == ClipboardDensity::Compact;
    if compact {
        root.add_css_class("compact");
    } else {
        root.remove_css_class("compact");
    }
}

/// Configured clipboard-menu max height (px), from
/// Settings → Clipboard ("Max height", `menus.clipboard_menu.maximum_height`).
/// 0 = no cap. We apply it to the *inner list* scroller (not the whole
/// menu) so the header + tabs stay fixed and only the history scrolls —
/// and so the bounded viewport lets the ListView virtualize when capped.
fn configured_list_max_height() -> i32 {
    config_manager()
        .config()
        .menus()
        .clipboard_menu()
        .maximum_height()
        .get_untracked()
}

/// Per-row model data placed in the [`gio::ListStore`] (wrapped in a
/// [`glib::BoxedAnyObject`]) is [`EntryView`] — a lightweight
/// projection that carries previews + metadata + search haystack but
/// **never** the entry's raw `data` payload, so the list model holds
/// no clipboard bytes. Built under the history lock via
/// [`ClipboardHistory::views`].
///
/// Sub-widgets of a recycled list row, stashed on the `ListItem` in
/// `connect_setup` and re-read in `connect_bind` to repaint for the
/// newly-bound [`ClipRow`].
struct RowWidgets {
    title: gtk::Label,
    type_icon: gtk::Image,
    preview_box: gtk::Box,
    pin_button: gtk::Button,
    pin_image: gtk::Image,
}

const ROW_DATA_KEY: &str = "clip-row-widgets";

pub(crate) struct ClipboardModel {
    /// The virtualized list. Only the visible screenful of rows is ever
    /// materialized; the rest live as cheap [`ClipRow`] model data in
    /// `store`. Recycled via the factory's setup/bind/unbind (the
    /// copyq QListView pattern).
    list_view: gtk::ListView,
    /// Full set of rows (all categories), newest first. Tab + `/`
    /// filtering happens in `filter`, never by rebuilding this.
    store: gio::ListStore,
    /// Tab + search predicate. Re-evaluated lazily by GTK when
    /// `filter.changed(..)` is called on a tab/search change.
    filter: gtk::CustomFilter,
    filter_model: gtk::FilterListModel,
    selection: gtk::SingleSelection,
    /// Shared with the `filter` closure: the active tab + lower-cased
    /// query it reads on each evaluation.
    tab_state: Rc<Cell<ClipTab>>,
    query_state: Rc<RefCell<String>>,
    history: ClipboardHistory,
    delete_button_visible: bool,
    /// Active type tab (source of truth; mirrored into `tab_state`).
    active_tab: ClipTab,
    /// Per-tab entry counts (index-aligned with `ClipTab::ALL`),
    /// recomputed from the full history on every populate.
    tab_counts: [usize; 5],
    /// Current `/` filter query (raw text). The lower-cased copy the
    /// filter reads lives in `query_state`; the open/closed state lives
    /// in the [`SEARCH_ACTIVE`] thread-local for the frame's Esc handler.
    search_query: String,
    /// Whether this menu is currently revealed. Clipboard events that
    /// arrive while hidden only flip `dirty` instead of repopulating —
    /// every monitor hosts a clipboard menu, so an un-gated
    /// repopulate-on-copy was N populates per copy.
    revealed: bool,
    /// A clipboard event landed while hidden — repopulate on next reveal.
    dirty: bool,
    /// Monotonic generation for the `/` filter debounce. Each keystroke
    /// bumps it and schedules an `ApplySearch(gen)`; only the firing
    /// whose gen still matches re-filters, so fast typing coalesces.
    search_gen: u64,
    /// Configured max height (px) for the history scroller, re-read from
    /// Settings → Clipboard on each reveal. 0 = grow to fit (no cap).
    list_max_height: i32,
}

#[derive(Debug)]
pub(crate) enum ClipboardInput {
    Refresh,
    DeleteAllClicked,
    /// The header gear — open Settings on the Widgets page (where the
    /// clipboard's own settings live) and close this panel.
    OpenSettings,
    /// Jump to a specific type tab (number keys 1–5 / clicks).
    SetTab(ClipTab),
    /// Tab key — cycle to the next type tab.
    CycleTab,
    SelectNext,
    SelectPrev,
    /// Copy the selected row (Enter).
    CopySelected,
    /// Copy a specific row by id (row activation / single click).
    CopyId(u64),
    DeleteSelected,
    /// Pin / unpin the selected entry (Ctrl+P).
    PinSelected,
    /// The frame's clipboard menu was revealed (`true`) or hidden
    /// (`false`). On reveal we repopulate (if dirty) and pull keyboard
    /// focus into the list so Tab / Ctrl+n/k / Enter work immediately.
    ParentRevealChanged(bool),
    /// `/` pressed — open the vim-style filter field and focus it.
    EnterSearch,
    /// Esc while searching — clear the filter and return to the list.
    ExitSearch,
    /// The filter text changed — re-filter the list live (debounced).
    SearchChanged(String),
    /// Debounce fire for the `/` filter — re-filters only if `gen` is
    /// still the latest keystroke's generation.
    ApplySearch(u64),
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

            // Panel header (DESIGN.md §12): leading clipboard glyph +
            // a display-size SemiBold title, with circular icon action
            // buttons (clear-all / settings) trailing.
            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 12,

                gtk::Image {
                    add_css_class: "clipboard-header-icon",
                    set_valign: gtk::Align::Center,
                    set_icon_name: Some("edit-paste-symbolic"),
                },

                gtk::Label {
                    add_css_class: "clipboard-title",
                    set_halign: gtk::Align::Start,
                    set_valign: gtk::Align::Center,
                    set_label: "Clipboard History",
                    set_hexpand: true,
                },

                gtk::Button {
                    add_css_class: "clipboard-action-btn",
                    set_valign: gtk::Align::Center,
                    set_icon_name: "trash-symbolic",
                    set_tooltip_text: Some("Clear all"),
                    connect_clicked[sender] => move |_| {
                        sender.input(ClipboardInput::DeleteAllClicked);
                    },
                },

                gtk::Button {
                    add_css_class: "clipboard-action-btn",
                    set_valign: gtk::Align::Center,
                    set_icon_name: "settings-symbolic",
                    set_tooltip_text: Some("Clipboard settings"),
                    connect_clicked[sender] => move |_| {
                        sender.input(ClipboardInput::OpenSettings);
                    },
                },
            },

            // Type tabs — All · Text · Images · Files · ★ (favorites),
            // each with a live count. Number keys 1–5 jump; Tab cycles.
            gtk::Box {
                add_css_class: "clipboard-tabs",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 4,
                // Centre the tab pill in the panel regardless of width
                // (was Start = left-aligned).
                set_halign: gtk::Align::Center,

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
                // Wrap (don't impose the full one-line width as a minimum):
                // this long hint was the panel's real width floor, so the
                // configured `minimum_width` couldn't shrink it below ~550px.
                set_wrap: true,
                set_wrap_mode: gtk::pango::WrapMode::WordChar,
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
                // `External`, not `Never`, on the horizontal axis: a
                // `Never` hscrollbar makes the scroller demand its
                // child's *minimum* width, and a GtkListView reports the
                // widest row's width there — which propagated up to the
                // menu's outer scroller and stopped the configured
                // `minimum_width` from shrinking the panel (height worked
                // because it's capped on this inner scroller; width is
                // pinned on the outer one). `External` shows no scrollbar
                // yet lets this scroller shrink horizontally, and since
                // the ListView is itself scrollable its rows are still
                // laid out at the viewport width (they ellipsize, never
                // clip). `min_content_width: 0` lets it shrink freely so
                // the outer `width_request = minimum_width` governs.
                set_hscrollbar_policy: gtk::PolicyType::External,
                set_min_content_width: 0,
                set_propagate_natural_width: false,
                // Grow to fit the history, capped at the configured
                // "Max height" (Settings → Clipboard). The cap lives on
                // THIS inner scroller — not the whole menu — so the
                // header + tabs stay fixed and only the list scrolls;
                // and a bounded viewport is exactly what lets the
                // ListView virtualize (only the visible rows are built).
                // 0 = no cap (grow to fit). `propagate_natural_height`
                // gives the menu its height from the (extrapolated) list
                // size without realizing every row.
                set_propagate_natural_height: true,
                #[watch]
                set_max_content_height: if model.list_max_height > 0 {
                    model.list_max_height
                } else {
                    -1
                },

                #[local_ref]
                list_view -> gtk::ListView {
                    add_css_class: "clipboard-list",
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

        // --- Virtualized list plumbing (mirrors the wallpaper grid). ---
        let store = gio::ListStore::new::<glib::BoxedAnyObject>();

        let tab_state = Rc::new(Cell::new(ClipTab::All));
        let query_state = Rc::new(RefCell::new(String::new()));

        let filter_tab = tab_state.clone();
        let filter_query = query_state.clone();
        let filter = gtk::CustomFilter::new(move |obj| {
            let Some(bo) = obj.downcast_ref::<glib::BoxedAnyObject>() else {
                return false;
            };
            let row = bo.borrow::<EntryView>();
            if !filter_tab.get().matches_cat(row.category, row.pinned) {
                return false;
            }
            let query = filter_query.borrow();
            query.is_empty() || row.haystack.contains(query.as_str())
        });
        let filter_model = gtk::FilterListModel::new(Some(store.clone()), Some(filter.clone()));
        let selection = gtk::SingleSelection::new(Some(filter_model.clone()));
        selection.set_autoselect(false);
        selection.set_can_unselect(true);

        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(move |_, list_item| {
            let list_item = list_item.downcast_ref::<gtk::ListItem>().unwrap();

            // Row content: title (relative time) + a preview container
            // filled on bind. The whole row is the activatable surface
            // (single-click / Enter → copy), so this is not a button.
            let content = gtk::Box::new(gtk::Orientation::Vertical, 4);
            content.add_css_class("clipboard-item");

            // Header: per-content-type icon + relative-time title.
            let type_icon = gtk::Image::new();
            type_icon.add_css_class("clipboard-item-type");
            type_icon.set_valign(gtk::Align::Center);

            let title = gtk::Label::new(None);
            title.add_css_class("clipboard-item-title");
            title.set_hexpand(true);
            title.set_halign(gtk::Align::Start);

            let header = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            header.append(&type_icon);
            header.append(&title);
            content.append(&header);

            let preview_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
            preview_box.set_valign(gtk::Align::Center);
            content.append(&preview_box);

            // Action cluster — top-right: pin then trash. They act on
            // the *currently bound* row, read live from `list_item.item()`.
            let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            actions.set_halign(gtk::Align::End);
            actions.set_valign(gtk::Align::Start);
            actions.set_margin_all(8);

            let pin_button = gtk::Button::new();
            pin_button.add_css_class("ok-button-surface");
            pin_button.add_css_class("clipboard-pin-button");
            let pin_image = gtk::Image::new();
            pin_image.set_halign(gtk::Align::Center);
            pin_image.set_valign(gtk::Align::Center);
            pin_button.set_child(Some(&pin_image));

            let trash_button = gtk::Button::new();
            trash_button.add_css_class("ok-button-surface");
            trash_button.add_css_class("clipboard-trash-button");
            let trash_image = gtk::Image::from_icon_name("trash-symbolic");
            trash_image.set_halign(gtk::Align::Center);
            trash_image.set_valign(gtk::Align::Center);
            trash_button.set_child(Some(&trash_image));

            actions.append(&pin_button);
            actions.append(&trash_button);

            let li_pin = list_item.downgrade();
            pin_button.connect_clicked(move |_| {
                if let Some(li) = li_pin.upgrade()
                    && let Some(id) = row_id_of(&li)
                {
                    clipboard_service().toggle_pin(id);
                }
            });
            let li_trash = list_item.downgrade();
            trash_button.connect_clicked(move |_| {
                if let Some(li) = li_trash.upgrade()
                    && let Some(id) = row_id_of(&li)
                {
                    clipboard_service().delete_entry(id);
                }
            });

            let overlay = gtk::Overlay::new();
            overlay.set_child(Some(&content));
            overlay.add_overlay(&actions);
            list_item.set_child(Some(&overlay));

            let rw = RowWidgets {
                title,
                type_icon,
                preview_box,
                pin_button,
                pin_image,
            };
            unsafe { list_item.set_data(ROW_DATA_KEY, rw) };
        });

        let bind_query = query_state.clone();
        factory.connect_bind(move |_, list_item| {
            let list_item = list_item.downcast_ref::<gtk::ListItem>().unwrap();
            let Some(rw) = (unsafe { list_item.data::<RowWidgets>(ROW_DATA_KEY) }) else {
                return;
            };
            let rw = unsafe { rw.as_ref() };
            let Some(obj) = list_item.item() else { return };
            let Ok(bo) = obj.downcast::<glib::BoxedAnyObject>() else {
                return;
            };
            let row = bo.borrow::<EntryView>();

            rw.title.set_label(&relative_time(row.timestamp));
            rw.type_icon
                .set_icon_name(Some(ClipTab::category_icon(row.category)));
            if row.pinned {
                rw.pin_button.add_css_class("pinned");
                rw.pin_image.set_icon_name(Some("starred-symbolic"));
            } else {
                rw.pin_button.remove_css_class("pinned");
                rw.pin_image.set_icon_name(Some("non-starred-symbolic"));
            }
            clear_box(&rw.preview_box);
            let query = bind_query.borrow();
            build_preview(&rw.preview_box, &row.preview, &query);
        });

        factory.connect_unbind(move |_, list_item| {
            let list_item = list_item.downcast_ref::<gtk::ListItem>().unwrap();
            if let Some(rw) = unsafe { list_item.data::<RowWidgets>(ROW_DATA_KEY) } {
                // Drop the preview (and its texture) so a recycled slot
                // doesn't pin off-screen image memory.
                clear_box(&unsafe { rw.as_ref() }.preview_box);
            }
        });

        let list_view = gtk::ListView::new(
            None::<gtk::SingleSelection>,
            None::<gtk::SignalListItemFactory>,
        );
        list_view.set_model(Some(&selection));
        list_view.set_factory(Some(&factory));
        // Single click on a row copies it (matches the old copy-button).
        list_view.set_single_click_activate(true);
        let activate_selection = selection.clone();
        let activate_sender = sender.clone();
        list_view.connect_activate(move |_, position| {
            if let Some(obj) = activate_selection.item(position)
                && let Ok(bo) = obj.downcast::<glib::BoxedAnyObject>()
            {
                let id = bo.borrow::<EntryView>().id;
                activate_sender.input(ClipboardInput::CopyId(id));
            }
        });

        // Keyboard control on the menu root (clipse-style): Tab to
        // switch tabs, Ctrl+n / Ctrl+k to move, Enter to copy, Delete
        // to remove, `/` to open the filter (vim). Arrow keys are
        // handled natively by the ListView in normal mode.
        //
        // The controller runs in the *Capture* phase so it sees keys
        // before the focused search entry. That lets nav shortcuts keep
        // working while typing a filter; plain characters fall through.
        let key_sender = sender.clone();
        let key = gtk::EventControllerKey::new();
        key.set_propagation_phase(gtk::PropagationPhase::Capture);
        key.connect_key_pressed(move |_, keyval, _, modifier| {
            let ctrl = modifier.contains(gtk::gdk::ModifierType::CONTROL_MASK);
            let searching = search_is_active();
            match keyval {
                gtk::gdk::Key::slash if !searching && !ctrl => {
                    key_sender.input(ClipboardInput::EnterSearch);
                    gtk::glib::Propagation::Stop
                }
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
                gtk::gdk::Key::_1
                | gtk::gdk::Key::_2
                | gtk::gdk::Key::_3
                | gtk::gdk::Key::_4
                | gtk::gdk::Key::_5
                    if !searching && !ctrl =>
                {
                    let idx = (keyval
                        .to_unicode()
                        .and_then(|c| c.to_digit(10))
                        .unwrap_or(1) as usize)
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
            list_view: list_view.clone(),
            store,
            filter,
            filter_model,
            selection,
            tab_state,
            query_state,
            history,
            delete_button_visible: false,
            active_tab: ClipTab::All,
            tab_counts: [0; 5],
            search_query: String::new(),
            revealed: false,
            dirty: false,
            search_gen: 0,
            list_max_height: configured_list_max_height(),
        };

        let widgets = view_output!();

        // Apply the configured row density to the root.
        apply_density(&root);

        // Populate immediately so the list + counts reflect current
        // history on first open (the reveal re-syncs anyway, but this
        // keeps a warm model).
        sender.input(ClipboardInput::Refresh);

        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::Input,
        sender: ComponentSender<Self>,
        root: &Self::Root,
    ) {
        match message {
            ClipboardInput::Refresh => {
                // Live clipboard event. Repopulate only if we're the
                // visible menu; otherwise defer to the next reveal so a
                // copy doesn't repopulate every monitor's hidden panel.
                if self.revealed {
                    self.populate();
                } else {
                    self.dirty = true;
                }
            }
            ClipboardInput::SetTab(tab) => {
                self.active_tab = tab;
                self.tab_state.set(tab);
                self.refilter();
            }
            ClipboardInput::CycleTab => {
                let tab = self.active_tab.next();
                self.active_tab = tab;
                self.tab_state.set(tab);
                self.refilter();
            }
            ClipboardInput::SelectNext => self.move_selection(1),
            ClipboardInput::SelectPrev => self.move_selection(-1),
            ClipboardInput::CopySelected => {
                if let Some(id) = self.selected_id() {
                    sender.input(ClipboardInput::CopyId(id));
                }
            }
            ClipboardInput::CopyId(id) => {
                let body = match self.history.get(id).map(|e| e.category()) {
                    Some(ClipCategory::Image) => "Image copied to clipboard",
                    Some(ClipCategory::File) => "File copied to clipboard",
                    _ => "Text copied to clipboard",
                };
                clipboard_service().copy_entry(id);
                mshell_launcher::notify::toast("Copied", body);
                let _ = sender.output(ClipboardOutput::CloseMenu);
            }
            ClipboardInput::DeleteSelected => {
                if let Some(id) = self.selected_id() {
                    clipboard_service().delete_entry(id);
                    // broadcast → Refresh repopulates the list.
                }
            }
            ClipboardInput::PinSelected => {
                if let Some(id) = self.selected_id() {
                    let was_pinned = self.history.get(id).map(|e| e.pinned).unwrap_or(false);
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
            ClipboardInput::OpenSettings => {
                mshell_settings::open_settings_at_section("widgets/clipboard");
            }
            ClipboardInput::EnterSearch => {
                set_search_active(true);
                widgets.search_revealer.set_reveal_child(true);
                let entry = widgets.search_entry.clone();
                gtk::glib::idle_add_local_once(move || {
                    entry.grab_focus();
                });
            }
            ClipboardInput::ExitSearch => {
                set_search_active(false);
                self.search_query.clear();
                *self.query_state.borrow_mut() = String::new();
                widgets.search_entry.set_text("");
                widgets.search_revealer.set_reveal_child(false);
                self.refilter();
                self.focus_list();
            }
            ClipboardInput::SearchChanged(text) => {
                // Store the query now (cheap) but defer the re-filter
                // ~70 ms so a fast typist doesn't re-filter on every
                // keystroke. The gen check drops all but the last.
                self.search_query = text;
                self.search_gen = self.search_gen.wrapping_add(1);
                let generation = self.search_gen;
                let debounce_sender = sender.clone();
                gtk::glib::timeout_add_local_once(
                    std::time::Duration::from_millis(70),
                    move || {
                        debounce_sender.input(ClipboardInput::ApplySearch(generation));
                    },
                );
            }
            ClipboardInput::ApplySearch(generation) => {
                if generation == self.search_gen {
                    *self.query_state.borrow_mut() = self.search_query.to_lowercase();
                    // Rebuild the model so every visible row re-binds and picks
                    // up the new highlight: a FilterListModel leaves surviving
                    // rows untouched, which would freeze a stale highlight from
                    // the previous keystroke.
                    self.populate();
                }
            }
            ClipboardInput::ParentRevealChanged(revealed) => {
                self.revealed = revealed;
                if revealed {
                    // Pick up Settings changes without a restart.
                    apply_density(root);
                    self.list_max_height = configured_list_max_height();
                    // Open fresh: any stale filter from a previous
                    // session is cleared so the full history shows.
                    set_search_active(false);
                    self.search_query.clear();
                    *self.query_state.borrow_mut() = String::new();
                    self.active_tab = ClipTab::All;
                    self.tab_state.set(ClipTab::All);
                    widgets.search_entry.set_text("");
                    widgets.search_revealer.set_reveal_child(false);
                    // Always re-sync to current history on open (cheap:
                    // it's model data, not widgets) and clear `dirty`.
                    self.dirty = false;
                    // Resolve the matugen accent from a realized widget so the
                    // search highlight matches the theme (refreshed each open →
                    // picks up theme changes between opens).
                    set_match_accent(resolve_primary_var(&widgets.search_entry));
                    self.populate();
                    self.focus_list();
                }
            }
        }

        self.update_view(widgets, sender);
    }
}

impl ClipboardModel {
    /// Rebuild the *model* (not widgets) from the full history: refill
    /// the store with lightweight [`ClipRow`]s and recompute per-tab
    /// counts. The virtualized view materializes only visible rows from
    /// this on its own. Search-independent (the `/` filter narrows the
    /// view, the counts always reflect the whole history).
    fn populate(&mut self) {
        // Lightweight views (no raw `data` clone, category computed
        // once) — see [`ClipboardHistory::views`].
        let views = self.history.views();
        let mut counts = [0usize; ClipTab::ALL.len()];

        self.store.remove_all();
        for view in views {
            for (i, tab) in ClipTab::ALL.iter().enumerate() {
                if tab.matches_cat(view.category, view.pinned) {
                    counts[i] += 1;
                }
            }
            self.store.append(&glib::BoxedAnyObject::new(view));
        }

        self.tab_counts = counts;
        self.refilter();
    }

    /// Re-evaluate the tab + search filter against the current store
    /// (lazy — GTK only re-tests materialized rows), refresh the
    /// empty-state flag, and anchor the selection at the top match.
    fn refilter(&mut self) {
        self.filter.changed(gtk::FilterChange::Different);
        let n = self.filter_model.n_items();
        self.delete_button_visible = n > 0;
        if n > 0 {
            self.selection.set_selected(0);
        }
    }

    /// Defer to an idle tick so the layer surface has finished
    /// mapping/allocating, then select the top row and pull keyboard
    /// focus into the list (grabbing focus on a not-yet-mapped view
    /// silently no-ops — the "keys dead until I click" symptom).
    fn focus_list(&self) {
        let list_view = self.list_view.clone();
        let selection = self.selection.clone();
        gtk::glib::idle_add_local_once(move || {
            if selection.n_items() > 0 {
                selection.set_selected(0);
            }
            list_view.grab_focus();
        });
    }

    /// Clipboard id of the currently-selected (filtered) row, if any.
    fn selected_id(&self) -> Option<u64> {
        let pos = self.selection.selected();
        if pos == gtk::INVALID_LIST_POSITION {
            return None;
        }
        let obj = self.selection.item(pos)?;
        let bo = obj.downcast::<glib::BoxedAnyObject>().ok()?;
        let id = bo.borrow::<EntryView>().id;
        Some(id)
    }

    /// Move selection by `delta` rows within the filtered view, clamped,
    /// and scroll + focus it into view.
    fn move_selection(&self, delta: i32) {
        let n = self.selection.n_items() as i32;
        if n == 0 {
            return;
        }
        let cur = self.selection.selected();
        let cur = if cur == gtk::INVALID_LIST_POSITION {
            0
        } else {
            cur as i32
        };
        let next = (cur + delta).clamp(0, n - 1) as u32;
        self.selection.set_selected(next);
        self.list_view
            .scroll_to(next, gtk::ListScrollFlags::FOCUS, None);
    }
}

/// Clipboard id of the row currently bound to a `ListItem`, read live
/// so a recycled row's overlay buttons always act on what they show.
fn row_id_of(list_item: &gtk::ListItem) -> Option<u64> {
    let obj = list_item.item()?;
    let bo = obj.downcast::<glib::BoxedAnyObject>().ok()?;
    let id = bo.borrow::<EntryView>().id;
    Some(id)
}

/// Remove every child of a box (used to clear a recycled preview slot).
fn clear_box(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

/// Fill a recycled row's preview container for the given entry preview.
/// Build Pango markup for `text` with every case-insensitive occurrence of
/// `needle_lower` accent-coloured (`needle_lower` is already lower-cased).
/// `None` when the needle is empty or doesn't occur in `text` (the caller then
/// keeps the plain label — e.g. the match was in the truncated-off tail).
/// Char-aligned and Pango-escaped.
fn highlight_substring_markup(text: &str, needle_lower: &str, accent: &str) -> Option<String> {
    if needle_lower.is_empty() {
        return None;
    }
    let needle: Vec<char> = needle_lower.chars().collect();
    let chars: Vec<char> = text.chars().collect();
    // 1:1 lower-cased view so matched-char indices line up with `chars`.
    let lower: Vec<char> = chars
        .iter()
        .map(|c| c.to_lowercase().next().unwrap_or(*c))
        .collect();

    let mut matched = vec![false; chars.len()];
    let mut any = false;
    let mut i = 0;
    while i + needle.len() <= lower.len() {
        if lower[i..i + needle.len()] == needle[..] {
            for m in matched.iter_mut().skip(i).take(needle.len()) {
                *m = true;
            }
            any = true;
            i += needle.len();
        } else {
            i += 1;
        }
    }
    if !any {
        return None;
    }

    let mut markup = String::new();
    for (idx, ch) in chars.iter().enumerate() {
        let esc = gtk::glib::markup_escape_text(&ch.to_string());
        if matched[idx] {
            markup.push_str("<span foreground=\"");
            markup.push_str(accent);
            markup.push_str("\">");
            markup.push_str(esc.as_str());
            markup.push_str("</span>");
        } else {
            markup.push_str(esc.as_str());
        }
    }
    Some(markup)
}

fn build_preview(preview_box: &gtk::Box, preview: &EntryPreview, query_lower: &str) {
    match preview {
        EntryPreview::Text(text) => {
            let label = gtk::Label::builder()
                .label(text)
                .halign(gtk::Align::Fill)
                .hexpand(true)
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .lines(2)
                .wrap(true)
                .wrap_mode(gtk::pango::WrapMode::WordChar)
                .build();
            label.add_css_class("label-medium-bold");
            // fzf-style: accent-colour the matched query substring(s) when the
            // `/` filter is active. Falls back to the plain `.label(text)`
            // above when there's no query, no accent, or the match is in the
            // truncated-off tail.
            if let Some(accent) = match_accent_value()
                && let Some(markup) = highlight_substring_markup(text, query_lower, &accent)
            {
                label.set_markup(&markup);
            }
            preview_box.append(&label);
        }
        EntryPreview::Image {
            rgba,
            width,
            height,
        } => {
            let bytes = glib::Bytes::from(&rgba[..]);
            let texture = gdk::MemoryTexture::new(
                *width as i32,
                *height as i32,
                gdk::MemoryFormat::R8g8b8a8,
                &bytes,
                (*width * 4) as usize,
            );
            let picture = gtk::Picture::for_paintable(&texture);
            picture.set_content_fit(gtk::ContentFit::Cover);
            picture.set_hexpand(true);

            let frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
            frame.set_overflow(gtk::Overflow::Hidden);
            frame.set_height_request(200);
            frame.set_hexpand(true);
            frame.append(&picture);
            frame.add_css_class("clipboard-item-image");
            preview_box.append(&frame);
        }
        EntryPreview::Binary { mime_type, size } => {
            let label = gtk::Label::builder()
                .label(format!("{mime_type}  ({})", format_size(*size)))
                .halign(gtk::Align::Start)
                .build();
            label.add_css_class("label-small-bold");
            preview_box.append(&label);
        }
    }
}

/// Relative "captured at" label for a row's title line — "just now",
/// "5m ago", "3h ago", "yesterday", "4d ago".
fn relative_time(timestamp: OffsetDateTime) -> String {
    let then = timestamp.unix_timestamp();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(then);
    let diff = (now - then).max(0);
    if diff < 45 {
        "just now".to_string()
    } else if diff < 3600 {
        format!("{}m ago", (diff / 60).max(1))
    } else if diff < 86_400 {
        format!("{}h ago", diff / 3600)
    } else if diff < 172_800 {
        "yesterday".to_string()
    } else {
        format!("{}d ago", diff / 86_400)
    }
}

fn format_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}
