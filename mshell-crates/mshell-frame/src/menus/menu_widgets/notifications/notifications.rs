//! Notification history menu — a **virtualized** `gtk::ListView`.
//!
//! History is persisted and grows without bound over time, so the old
//! design (one heavy `NotificationModel` relm4 component per entry, the
//! whole list torn down + rebuilt on *every* change) made the panel
//! slower the more notifications accumulated. This mirrors the clipboard
//! menu: the list model holds a cheap [`NotifRow`] per entry and a
//! `SignalListItemFactory` materializes only the visible screenful,
//! recycling row widgets on scroll. A single incoming toast re-splices
//! lightweight model data — it never builds N widget trees.
//!
//! Grouping (on by default) is preserved by **flattening** groups into
//! the model: a group of two or more emits a `Header` row followed by
//! its `Notif` children *only while expanded*. Clicking a header toggles
//! expansion and re-splices the store (cheap; widgets stay virtualized).

use mshell_common::notification::{
    APP_ICON_SIZE, BODY_IMAGE_SIZE, apply_body_text, build_image, detect_code,
    format_notification_time,
};
use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    ConfigStoreFields, GeneralStoreFields, MenuStoreFields, MenusStoreFields,
    NotificationsStoreFields,
};
use mshell_services::notification_service;
use mshell_utils::notifications::{spawn_dnd_watcher, spawn_notifications_watcher};
use reactive_graph::traits::{Get, GetUntracked};
use relm4::gtk::prelude::*;
use relm4::gtk::{gio, glib};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use wayle_notification::core::notification::Notification;

/// Swipe distance (px) past which a drag dismisses the toast.
const SWIPE_DISMISS: f64 = 64.0;
/// Movement (px) under which a drag counts as a tap (→ default action).
const TAP_SLOP: f64 = 8.0;
/// Drag distance that maps to the maximum fade during a swipe.
const FADE_SPAN: f64 = 320.0;

const ROW_DATA_KEY: &str = "notif-row-widgets";

/// Lightweight per-row model data placed in the [`gio::ListStore`]
/// (wrapped in a [`glib::BoxedAnyObject`]). No widgets — the factory
/// builds those lazily for the visible rows only.
#[derive(Clone)]
enum NotifRow {
    /// A collapsible group header: app name + member count + whether the
    /// group is currently expanded.
    Header {
        app: String,
        count: usize,
        expanded: bool,
    },
    /// A single notification row. `child` = inside an expanded group
    /// (indented), so the styling can set it apart from a top-level one.
    Notif {
        notification: Arc<Notification>,
        time: String,
        child: bool,
    },
}

/// Recycled per-row widgets, stashed on the `ListItem` in `connect_setup`
/// and re-read in `connect_bind`. Each row can render *either* a group
/// header (`header_box`) or a notification card (`card`); bind shows one
/// and hides the other.
struct RowWidgets {
    // ── group-header variant ──
    header_box: gtk::Box,
    header_chevron: gtk::Image,
    header_label: gtk::Label,
    // ── notification-card variant ──
    card: gtk::Box,
    app_icon_box: gtk::Box,
    app_name: gtk::Label,
    time_label: gtk::Label,
    close_button: gtk::Button,
    content: gtk::Box,
    thumb_box: gtk::Box,
    summary: gtk::Label,
    body: gtk::Label,
    code_container: gtk::Box,
    actions_container: gtk::Box,
    /// Live state read by the (once-wired) gesture / click handlers so
    /// they act on whatever row is currently bound to this widget.
    ctx: Rc<RefCell<RowCtx>>,
}

/// What the row's handlers act on right now — updated on every bind.
#[derive(Default)]
struct RowCtx {
    /// Bound notification (for a `Notif` row).
    notification: Option<Arc<Notification>>,
    /// Default-action key of the bound notification (tap target).
    default_key: Option<String>,
    /// Bound app name (for a `Header` row's toggle).
    header_app: Option<String>,
}

pub(crate) struct NotificationsModel {
    /// The virtualized history list. Only the visible rows are ever
    /// materialized; the rest live as cheap [`NotifRow`] data in `store`.
    list_view: gtk::ListView,
    /// Flattened rows (headers + notifications), newest first.
    store: gio::ListStore,
    empty_label_visible: bool,
    dnd: bool,
    /// App names whose group is expanded. Persists across store rebuilds.
    expanded: Rc<RefCell<HashSet<String>>>,
    /// Configured inner-list max height (px); 0 → no cap. Lives on the
    /// inner scroller (not the menu) so the header stays fixed and the
    /// bounded viewport lets the ListView virtualize.
    list_max_height: i32,
    /// Whether this menu is currently revealed. Every monitor hosts a
    /// notifications menu, so an un-gated rebuild-on-notification was N
    /// store rebuilds per incoming toast. We rebuild once, on the next
    /// reveal, if dirty.
    revealed: bool,
    /// A notification (or grouping/format) change landed while hidden —
    /// rebuild on next reveal.
    dirty: bool,
    /// Whether the store has been built at least once. Lets a reveal skip
    /// the (O(history)) rebuild when nothing changed since the last open.
    built: bool,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum NotificationsInput {
    ClearAllClicked,
    DndClicked,
    /// `group_notifications` or the clock format flipped — rebuild rows.
    RebuildRequested,
    /// A group header was clicked — toggle its expansion + re-splice.
    ToggleGroup(String),
    /// Inner-list max-height config changed.
    SetMaxHeight(i32),
    /// The frame's notifications menu was revealed (`true`) or hidden
    /// (`false`). Rebuilds happen only while revealed; a notification
    /// arriving while hidden just flips `dirty`.
    ParentRevealChanged(bool),
}

#[derive(Debug)]
pub(crate) enum NotificationsOutput {
    CloseMenu,
}

pub(crate) struct NotificationsInit {}

#[derive(Debug)]
pub(crate) enum NotificationsCommandOutput {
    NotificationsChanged,
    DndChanged,
}

#[relm4::component(pub)]
impl Component for NotificationsModel {
    type CommandOutput = NotificationsCommandOutput;
    type Input = NotificationsInput;
    type Output = NotificationsOutput;
    type Init = NotificationsInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "notifications-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 12,

            // ── §12 panel header ──
            gtk::Box {
                add_css_class: "panel-header",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 12,

                gtk::Button {
                    add_css_class: "panel-action-btn",
                    set_valign: gtk::Align::Center,
                    set_tooltip_text: Some("Do Not Disturb"),
                    connect_clicked[sender] => move |_| {
                        sender.input(NotificationsInput::DndClicked);
                    },

                    gtk::Image {
                        #[watch]
                        set_icon_name: if model.dnd {
                            Some("notification-disabled-symbolic")
                        } else {
                            Some("notification-symbolic")
                        },
                    },
                },

                gtk::Label {
                    add_css_class: "panel-title",
                    set_halign: gtk::Align::Start,
                    set_label: "Notification History",
                    set_hexpand: true,
                },

                gtk::Button {
                    add_css_class: "panel-action-btn",
                    set_valign: gtk::Align::Center,
                    set_tooltip_text: Some("Clear all"),
                    connect_clicked[sender] => move |_| {
                        sender.input(NotificationsInput::ClearAllClicked);
                    },

                    gtk::Image {
                        set_icon_name: Some("edit-clear-all-symbolic"),
                    },
                },
            },

            gtk::Label {
                add_css_class: "label-medium",
                #[watch]
                set_visible: model.empty_label_visible,
                set_label: "Empty",
            },

            gtk::ScrolledWindow {
                set_vscrollbar_policy: gtk::PolicyType::Automatic,
                // `External` (not `Never`): lets the scroller shrink
                // horizontally so the configured menu width governs,
                // while the ListView ellipsizes rather than clips. The
                // height cap below gives the bounded viewport that lets
                // the list virtualize.
                set_hscrollbar_policy: gtk::PolicyType::External,
                set_min_content_width: 0,
                set_propagate_natural_width: false,
                set_propagate_natural_height: true,
                #[watch]
                set_max_content_height: if model.list_max_height > 0 {
                    model.list_max_height
                } else {
                    -1
                },

                #[local_ref]
                list_view -> gtk::ListView {
                    add_css_class: "notifications-list",
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        spawn_notifications_watcher(&sender, || NotificationsCommandOutput::NotificationsChanged);
        spawn_dnd_watcher(&sender, || NotificationsCommandOutput::DndChanged);

        // Rebuild the history live when the grouping toggle or the clock
        // format flips (the row time strings are baked at build time).
        let mut effects = EffectScope::new();
        {
            let eff_sender = sender.clone();
            effects.push(move |_| {
                // Each store accessor consumes the handle, so re-read the
                // root per field (matches the pattern elsewhere).
                let _ = config_manager()
                    .config()
                    .notifications()
                    .group_notifications()
                    .get();
                let _ = config_manager()
                    .config()
                    .general()
                    .clock_format_24_h()
                    .get();
                eff_sender.input(NotificationsInput::RebuildRequested);
            });
        }
        // Track the inner-list max height (Settings → Notifications).
        {
            let eff_sender = sender.clone();
            effects.push(move |_| {
                let h = config_manager()
                    .config()
                    .menus()
                    .notification_menu()
                    .maximum_height()
                    .get();
                eff_sender.input(NotificationsInput::SetMaxHeight(h));
            });
        }

        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let selection = gtk::NoSelection::new(Some(store.clone()));
        let expanded: Rc<RefCell<HashSet<String>>> = Rc::new(RefCell::new(HashSet::new()));

        let factory = gtk::SignalListItemFactory::new();
        Self::wire_factory(&factory, &sender);

        let list_view = gtk::ListView::new(
            None::<gtk::NoSelection>,
            None::<gtk::SignalListItemFactory>,
        );
        list_view.set_model(Some(&selection));
        list_view.set_factory(Some(&factory));
        list_view.set_single_click_activate(false);

        let model = NotificationsModel {
            list_view: list_view.clone(),
            store,
            empty_label_visible: true,
            dnd: false,
            expanded,
            list_max_height: 0,
            revealed: false,
            dirty: false,
            built: false,
            _effects: effects,
        };

        let list_view = &model.list_view;
        let widgets = view_output!();

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
            NotificationsInput::ClearAllClicked => {
                tokio::spawn(async move {
                    let _ = notification_service().dismiss_all().await;
                });
                let _ = sender.output(NotificationsOutput::CloseMenu);
            }
            NotificationsInput::DndClicked => {
                let service = notification_service();
                let dnd = service.dnd.get();
                service.set_dnd(!dnd);
            }
            NotificationsInput::RebuildRequested => {
                if self.revealed {
                    self.refresh();
                } else {
                    self.dirty = true;
                }
            }
            NotificationsInput::ToggleGroup(app) => {
                {
                    let mut set = self.expanded.borrow_mut();
                    if !set.remove(&app) {
                        set.insert(app);
                    }
                }
                if self.revealed {
                    self.refresh();
                }
            }
            NotificationsInput::SetMaxHeight(h) => {
                self.list_max_height = h;
            }
            NotificationsInput::ParentRevealChanged(revealed) => {
                self.revealed = revealed;
                // Only rebuild on open if something changed since last time
                // (or the very first open) — reopening an unchanged history
                // shouldn't re-splice the whole list model.
                if revealed && (self.dirty || !self.built) {
                    self.dirty = false;
                    self.refresh();
                }
            }
        }

        self.update_view(widgets, sender);
    }

    fn update_cmd_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            NotificationsCommandOutput::NotificationsChanged => {
                // The hot path: a notification arrived/cleared. Re-splice
                // the visible menu's lightweight model; hidden ones defer.
                if self.revealed {
                    self.refresh();
                } else {
                    self.dirty = true;
                }
            }
            NotificationsCommandOutput::DndChanged => {
                self.dnd = notification_service().dnd.get();
            }
        }

        self.update_view(widgets, sender);
    }
}

impl NotificationsModel {
    /// Pull current history, refresh the empty-state flag, and re-splice
    /// the lightweight row model. The single rebuild entry point.
    fn refresh(&mut self) {
        let notifications = notification_service().notifications.get();
        self.empty_label_visible = notifications.is_empty();
        self.rebuild_store(&notifications);
        self.built = true;
    }

    /// Flatten the history into [`NotifRow`]s and splice them into the
    /// store in one shot. No widgets are built here — the factory does
    /// that lazily for visible rows only.
    fn rebuild_store(&self, notifications: &[Arc<Notification>]) {
        let group = config_manager()
            .config()
            .notifications()
            .group_notifications()
            .get_untracked();
        let fmt24 = config_manager()
            .config()
            .general()
            .clock_format_24_h()
            .get_untracked();

        // Render only the most-recent `history_limit` entries (0 = all) so a
        // 500-entry persisted history doesn't rebuild a 500-row model on
        // every open. The slice is the top of the menu's natural order.
        let limit = config_manager()
            .config()
            .notifications()
            .history_limit()
            .get_untracked() as usize;
        let notifications: &[Arc<Notification>] = if limit > 0 && notifications.len() > limit {
            &notifications[..limit]
        } else {
            notifications
        };

        let mut rows: Vec<NotifRow> = Vec::with_capacity(notifications.len());

        if !group {
            for n in notifications {
                rows.push(NotifRow::Notif {
                    notification: n.clone(),
                    time: format_notification_time(n, fmt24),
                    child: false,
                });
            }
        } else {
            // Group by app, preserving first-seen order.
            let mut order: Vec<String> = Vec::new();
            let mut groups: HashMap<String, Vec<Arc<Notification>>> = HashMap::new();
            for n in notifications {
                let app = n.app_name.get().unwrap_or_default();
                if !groups.contains_key(&app) {
                    order.push(app.clone());
                }
                groups.entry(app).or_default().push(n.clone());
            }

            // Prune the expanded set to apps that still have a group, so
            // it can't grow unbounded as apps come and go.
            {
                let mut exp = self.expanded.borrow_mut();
                exp.retain(|app| groups.get(app).is_some_and(|v| v.len() >= 2));
            }
            let expanded = self.expanded.borrow();

            for app in order {
                let items = groups.remove(&app).unwrap_or_default();
                if items.len() == 1 {
                    rows.push(NotifRow::Notif {
                        notification: items[0].clone(),
                        time: format_notification_time(&items[0], fmt24),
                        child: false,
                    });
                } else {
                    let is_expanded = expanded.contains(&app);
                    rows.push(NotifRow::Header {
                        app: app.clone(),
                        count: items.len(),
                        expanded: is_expanded,
                    });
                    if is_expanded {
                        for n in &items {
                            rows.push(NotifRow::Notif {
                                notification: n.clone(),
                                time: format_notification_time(n, fmt24),
                                child: true,
                            });
                        }
                    }
                }
            }
        }

        let objs: Vec<glib::BoxedAnyObject> =
            rows.into_iter().map(glib::BoxedAnyObject::new).collect();
        let prev = self.store.n_items();
        self.store.splice(0, prev, &objs);
    }

    /// Wire the factory's setup / bind / unbind once. `setup` builds the
    /// reusable row skeleton and its (live-reading) handlers; `bind`
    /// repaints it from the bound [`NotifRow`]; `unbind` releases the
    /// row's dynamic children (images, action buttons) so a recycled
    /// slot doesn't pin off-screen memory.
    fn wire_factory(factory: &gtk::SignalListItemFactory, sender: &ComponentSender<Self>) {
        // ── setup ──
        {
            let sender = sender.clone();
            factory.connect_setup(move |_, list_item| {
                let list_item = list_item.downcast_ref::<gtk::ListItem>().unwrap();
                let ctx: Rc<RefCell<RowCtx>> = Rc::new(RefCell::new(RowCtx::default()));

                // Group-header variant.
                let header_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                header_box.add_css_class("notification-group-header");
                let header_chevron = gtk::Image::from_icon_name("go-next-symbolic");
                header_chevron.add_css_class("notification-group-chevron");
                let header_label = gtk::Label::new(None);
                header_label.add_css_class("notification-group-title");
                header_label.set_halign(gtk::Align::Start);
                header_label.set_hexpand(true);
                header_box.append(&header_chevron);
                header_box.append(&header_label);
                {
                    let ctx = ctx.clone();
                    let sender = sender.clone();
                    let click = gtk::GestureClick::new();
                    click.connect_released(move |_, _, _, _| {
                        let app = ctx.borrow().header_app.clone();
                        if let Some(app) = app {
                            sender.input(NotificationsInput::ToggleGroup(app));
                        }
                    });
                    header_box.add_controller(click);
                }

                // Notification-card variant.
                let card = gtk::Box::new(gtk::Orientation::Vertical, 8);
                card.add_css_class("notification");
                card.set_hexpand(true);

                let header_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                let app_icon_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                let app_name = gtk::Label::new(None);
                app_name.add_css_class("label-small-bold-variant");
                app_name.set_hexpand(true);
                app_name.set_xalign(0.0);
                let time_label = gtk::Label::new(None);
                time_label.add_css_class("label-small");
                let close_button = gtk::Button::new();
                close_button.add_css_class("ok-button-surface");
                close_button.set_margin_start(4);
                let close_img = gtk::Image::from_icon_name("close-symbolic");
                close_img.set_halign(gtk::Align::Center);
                close_img.set_valign(gtk::Align::Center);
                close_button.set_child(Some(&close_img));
                header_row.append(&app_icon_box);
                header_row.append(&app_name);
                header_row.append(&time_label);
                header_row.append(&close_button);
                {
                    let ctx = ctx.clone();
                    close_button.connect_clicked(move |_| {
                        if let Some(n) = ctx.borrow().notification.clone() {
                            n.dismiss();
                        }
                    });
                }

                let content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
                let thumb_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
                thumb_box.set_valign(gtk::Align::Start);
                let text_col = gtk::Box::new(gtk::Orientation::Vertical, 4);
                text_col.set_hexpand(true);
                let summary = gtk::Label::new(None);
                summary.add_css_class("label-medium-bold");
                summary.set_xalign(0.0);
                summary.set_wrap(true);
                summary.set_wrap_mode(gtk::pango::WrapMode::WordChar);
                summary.set_width_chars(20);
                summary.set_max_width_chars(44);
                let body = gtk::Label::new(None);
                body.add_css_class("label-small");
                body.set_xalign(0.0);
                body.set_wrap(true);
                body.set_wrap_mode(gtk::pango::WrapMode::WordChar);
                body.set_width_chars(20);
                body.set_max_width_chars(44);
                text_col.append(&summary);
                text_col.append(&body);
                content.append(&thumb_box);
                content.append(&text_col);

                // One GestureDrag drives tap → default action and
                // horizontal swipe → dismiss, fading the card while
                // dragging. Reads the live ctx so recycling is safe.
                {
                    let drag = gtk::GestureDrag::new();
                    let card_fade = card.downgrade();
                    drag.connect_drag_update(move |_, off_x, _| {
                        if let Some(card) = card_fade.upgrade() {
                            let fade = (off_x.abs() / FADE_SPAN).min(0.6);
                            card.set_opacity(1.0 - fade);
                        }
                    });
                    let ctx = ctx.clone();
                    let card_end = card.downgrade();
                    let sender = sender.clone();
                    drag.connect_drag_end(move |_, off_x, off_y| {
                        let (notif, key) = {
                            let c = ctx.borrow();
                            (c.notification.clone(), c.default_key.clone())
                        };
                        if off_x.abs() > SWIPE_DISMISS && off_x.abs() > off_y.abs() {
                            if let Some(n) = notif {
                                n.dismiss();
                                let _ = sender.output(NotificationsOutput::CloseMenu);
                            }
                        } else if off_x.abs() < TAP_SLOP && off_y.abs() < TAP_SLOP {
                            if let Some(card) = card_end.upgrade() {
                                card.set_opacity(1.0);
                            }
                            if let (Some(n), Some(key)) = (notif, key) {
                                let sender = sender.clone();
                                tokio::spawn(async move {
                                    let _ = n.invoke(&key).await;
                                    let _ = sender.output(NotificationsOutput::CloseMenu);
                                });
                            }
                        } else if let Some(card) = card_end.upgrade() {
                            card.set_opacity(1.0);
                        }
                    });
                    content.add_controller(drag);
                }

                let code_container = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                let actions_container = gtk::Box::new(gtk::Orientation::Vertical, 4);

                card.append(&header_row);
                card.append(&content);
                card.append(&code_container);
                card.append(&actions_container);

                let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
                root.append(&header_box);
                root.append(&card);
                list_item.set_child(Some(&root));

                let rw = RowWidgets {
                    header_box,
                    header_chevron,
                    header_label,
                    card,
                    app_icon_box,
                    app_name,
                    time_label,
                    close_button,
                    content,
                    thumb_box,
                    summary,
                    body,
                    code_container,
                    actions_container,
                    ctx,
                };
                unsafe { list_item.set_data(ROW_DATA_KEY, rw) };
            });
        }

        // ── bind ──
        {
            let sender = sender.clone();
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
                let row = bo.borrow::<NotifRow>();

                match &*row {
                    NotifRow::Header {
                        app,
                        count,
                        expanded,
                    } => {
                        rw.header_box.set_visible(true);
                        rw.card.set_visible(false);
                        rw.header_chevron.set_icon_name(Some(if *expanded {
                            "go-down-symbolic"
                        } else {
                            "go-next-symbolic"
                        }));
                        rw.header_label.set_label(&format!("{app}  ({count})"));
                        let mut c = rw.ctx.borrow_mut();
                        c.header_app = Some(app.clone());
                        c.notification = None;
                        c.default_key = None;
                    }
                    NotifRow::Notif {
                        notification,
                        time,
                        child,
                    } => {
                        rw.header_box.set_visible(false);
                        rw.card.set_visible(true);
                        bind_card(rw, notification, time, *child, &sender);
                    }
                }
            });
        }

        // ── unbind ──
        factory.connect_unbind(move |_, list_item| {
            let list_item = list_item.downcast_ref::<gtk::ListItem>().unwrap();
            if let Some(rw) = unsafe { list_item.data::<RowWidgets>(ROW_DATA_KEY) } {
                let rw = unsafe { rw.as_ref() };
                clear_box(&rw.app_icon_box);
                clear_box(&rw.thumb_box);
                clear_box(&rw.code_container);
                clear_box(&rw.actions_container);
                rw.card.set_opacity(1.0);
                let mut c = rw.ctx.borrow_mut();
                c.notification = None;
                c.default_key = None;
                c.header_app = None;
            }
        });
    }
}

/// Repaint a recycled row's notification card from `notification`.
/// Dynamic children (app icon, thumbnail, OTP chip, action buttons) are
/// cleared and rebuilt so a recycled slot never shows a previous row's
/// content.
fn bind_card(
    rw: &RowWidgets,
    notification: &Arc<Notification>,
    time: &str,
    child: bool,
    sender: &ComponentSender<NotificationsModel>,
) {
    let show_close = config_manager()
        .config()
        .notifications()
        .show_close_button()
        .get_untracked();
    let show_actions = config_manager()
        .config()
        .notifications()
        .show_action_buttons()
        .get_untracked();

    if child {
        rw.card.add_css_class("notification-group-child");
    } else {
        rw.card.remove_css_class("notification-group-child");
    }

    rw.app_name
        .set_label(notification.app_name.get().unwrap_or_default().as_str());
    rw.time_label.set_label(time);
    rw.close_button.set_visible(show_close);
    rw.summary.set_label(notification.summary.get().as_str());

    let body = notification.body.get().unwrap_or_default();
    apply_body_text(&rw.body, &body);

    // App icon (header leading glyph).
    clear_box(&rw.app_icon_box);
    if let Some(icon) = notification.app_icon.get() {
        let icon = icon.trim();
        if !icon.is_empty() {
            let img = build_image(icon, APP_ICON_SIZE);
            img.add_css_class("notification-app-icon");
            rw.app_icon_box.append(&img);
        }
    }

    // Body thumbnail / album art.
    clear_box(&rw.thumb_box);
    if let Some(path) = notification.image_path.get() {
        let path = path.trim();
        if !path.is_empty() {
            let img = build_image(path, BODY_IMAGE_SIZE);
            img.set_valign(gtk::Align::Start);
            img.add_css_class("notification-image");
            rw.thumb_box.append(&img);
        }
    }

    // Default-action clickability (the GestureDrag tap target).
    let default_key = notification.default_action.get().map(|a| a.id.clone());
    if default_key.is_some() {
        rw.content.add_css_class("notification-clickable");
    } else {
        rw.content.remove_css_class("notification-clickable");
    }

    // 2FA / OTP one-click copy chip.
    clear_box(&rw.code_container);
    let haystack = format!("{} {}", notification.summary.get(), body);
    if let Some(code) = detect_code(&haystack) {
        let btn = gtk::Button::new();
        btn.add_css_class("notification-code-copy");
        let inner = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        inner.append(&gtk::Image::from_icon_name("edit-copy-symbolic"));
        inner.append(&gtk::Label::new(Some(&format!("Copy code  {code}"))));
        btn.set_child(Some(&inner));
        let code_for_click = code.clone();
        btn.connect_clicked(move |b| {
            b.clipboard().set_text(&code_for_click);
        });
        rw.code_container.append(&btn);
    }

    // Explicit action buttons.
    clear_box(&rw.actions_container);
    let action_icons = notification.action_icons.get();
    let actions = notification.actions.get();
    if show_actions && !actions.is_empty() {
        for action in &actions {
            let btn = if action_icons {
                let b = gtk::Button::new();
                b.set_child(Some(&gtk::Image::from_icon_name(&action.id)));
                b.set_tooltip_text(Some(&action.label));
                b
            } else {
                gtk::Button::with_label(&action.label)
            };
            btn.add_css_class("ok-button-primary");

            let notification = notification.clone();
            let key = action.id.clone();
            let sender = sender.clone();
            btn.connect_clicked(move |_| {
                let notification = notification.clone();
                let key = key.clone();
                let sender = sender.clone();
                tokio::spawn(async move {
                    let _ = notification.invoke(&key).await;
                    let _ = sender.output(NotificationsOutput::CloseMenu);
                });
            });
            rw.actions_container.append(&btn);
        }
    }

    // Update the live handler context.
    let mut c = rw.ctx.borrow_mut();
    c.notification = Some(notification.clone());
    c.default_key = default_key;
    c.header_app = None;
}

/// Remove every child of a box (used to clear recycled row content).
fn clear_box(b: &gtk::Box) {
    while let Some(child) = b.first_child() {
        b.remove(&child);
    }
}
