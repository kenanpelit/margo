//! Unified app launcher menu — search entry + result list backed
//! by a [`LauncherRuntime`].
//!
//! Replaces the legacy app-only widget: every result row (apps,
//! calculator, session actions, settings shortcuts, `>cmd`) flows
//! through the same provider trait + scoring + frecency pipeline.
//! The widget itself only owns UI state (filter string, selection
//! index) and delegates everything else to the runtime.
//!
//! ## Keyboard model
//!
//! Same as the old widget — readline / vim / emacs conventions:
//! `Down|Tab|Ctrl+N|Ctrl+J` → next, `Up|Shift+Tab|Ctrl+P|Ctrl+K` →
//! previous, `Enter` → activate, `Escape` → close. The search
//! entry receives focus on every open so users can start typing
//! immediately.

use crate::menus::menu_widgets::app_launcher::apps_provider::AppsProvider;
use crate::menus::menu_widgets::app_launcher::clipboard_provider::ClipboardProvider;
use crate::menus::menu_widgets::app_launcher::launcher_row::{
    LauncherRowInit, LauncherRowInput, LauncherRowModel, LauncherRowOutput,
};
use crate::menus::menu_widgets::app_launcher::windows_provider::WindowsProvider;
use gtk4_layer_shell::{KeyboardMode, LayerShell};
use mshell_common::dynamic_box::dynamic_box::{
    DynamicBoxFactory, DynamicBoxInit, DynamicBoxInput, DynamicBoxModel,
};
use mshell_common::dynamic_box::generic_widget_controller::{
    GenericWidgetController, GenericWidgetControllerExtSafe,
};
use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, IconsStoreFields, ThemeStoreFields};
use mshell_launcher::providers::{
    ArchLinuxPkgsProvider, BluetoothProvider, CalculatorProvider, CommandProvider, EmojiProvider,
    MctlProvider, PlayerctlProvider, ProviderListProvider, ScriptsProvider, SessionProvider,
    SettingsProvider, SymbolsProvider, WebsearchProvider, WireplumberProvider,
};
use mshell_launcher::{FrecencyStore, LauncherItem, LauncherRuntime};
use reactive_graph::traits::*;
use relm4::gtk::glib;
use relm4::gtk::prelude::*;
use relm4::gtk::{RevealerTransitionType, ScrolledWindow, gdk, gio};
use relm4::{
    Component, ComponentController, ComponentParts, ComponentSender, Controller, RelmWidgetExt, gtk,
};
use std::cell::RefCell;
use std::rc::Rc;

pub(crate) struct AppLauncherModel {
    dynamic_box: Controller<DynamicBoxModel<LauncherItem, String>>,
    /// Runtime owns the providers + frecency. Wrapped in
    /// `RefCell<Rc<>>` because the Apps provider needs to be
    /// mutated (toggle show_hidden) from within the widget's
    /// update path while the runtime simultaneously borrows
    /// providers immutably during `query()`. Two-borrow split via
    /// `Rc<AppsProvider>` on the side.
    runtime: RefCell<LauncherRuntime>,
    /// Shared handle to the Apps provider so we can toggle the
    /// `show_hidden` flag without going through the runtime
    /// (which only exposes immutable provider access).
    apps_provider: Rc<AppsProvider>,
    /// Closes the launcher menu. Set in init; called after every
    /// item activation. RefCell<Option<...>> so the closure can
    /// be stamped post-construction without an extra cell on the
    /// model type.
    close_sender: RefCell<Option<Box<dyn Fn() + 'static>>>,
    filter: String,
    results: Vec<LauncherItem>,
    /// Stable id of the currently-selected row. We store the id
    /// rather than an index so reordering between keystrokes
    /// doesn't strand the highlight on a different row.
    selected_id: Option<String>,
    is_revealed: bool,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum AppLauncherInput {
    FilterChanged(String),
    Activate,
    ParentRevealChanged(bool),
    DownPressed,
    UpPressed,
    /// User pressed Enter on a specific row (via mouse click or
    /// Tab+Enter on the focused row). Carries the row id.
    ActivateRow(String),
    ShowHiddenAppsChanged,
    ThemeChanged,
}

#[derive(Debug)]
pub(crate) enum AppLauncherOutput {
    CloseMenu,
}

pub(crate) struct AppLauncherInit {}

#[derive(Debug)]
pub(crate) enum AppLauncherCommandOutput {}

#[relm4::component(pub)]
impl Component for AppLauncherModel {
    type CommandOutput = AppLauncherCommandOutput;
    type Input = AppLauncherInput;
    type Output = AppLauncherOutput;
    type Init = AppLauncherInit;

    view! {
        #[root]
        #[name = "root"]
        gtk::Box {
            add_css_class: "app-launcher-menu-widget",
            set_orientation: gtk::Orientation::Vertical,

            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_margin_bottom: 8,

                gtk::Image {
                    add_css_class: "app-launcher-search-icon",
                    set_icon_name: Some("system-search-symbolic"),
                },

                #[name = "search_entry"]
                gtk::Entry {
                    add_css_class: "ok-entry",
                    set_placeholder_text: Some("Search apps, calc, > commands…"),
                    set_hexpand: true,
                    connect_changed[sender] => move |entry| {
                        sender.input(AppLauncherInput::FilterChanged(entry.text().to_string()));
                    },
                    connect_activate[sender] => move |_| {
                        sender.input(AppLauncherInput::Activate);
                    },
                },

                gtk::Button {
                    add_css_class: "ok-button-surface",
                    set_hexpand: false,
                    set_vexpand: false,
                    connect_clicked[sender] => move |_| {
                        sender.input(AppLauncherInput::ShowHiddenAppsChanged);
                    },

                    #[name = "image"]
                    gtk::Image {
                        set_hexpand: true,
                        set_vexpand: true,
                        set_halign: gtk::Align::Center,
                        set_valign: gtk::Align::Center,
                        #[watch]
                        set_icon_name: if model.apps_provider.show_hidden() {
                            Some("eye-symbolic")
                        } else {
                            Some("eye-off-symbolic")
                        },
                    }
                }
            },

            #[name = "scrolled_window"]
            ScrolledWindow {
                set_vscrollbar_policy: gtk::PolicyType::Automatic,
                set_propagate_natural_height: true,

                #[name = "apps_box"]
                gtk::Box {},
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        // Build provider stack. Order matters for command-mode
        // dispatch — first matching wins — and for the empty-query
        // browse list (concatenated in registration order).
        let apps_provider = Rc::new(AppsProvider::new());

        // Apps provider: launch happens in the activate closure;
        // closing the launcher is handled centrally in
        // `activate_id` so we don't wire a per-launch callback.

        // Settings provider: route through the in-process
        // section-backend bridge so the panel opens AND the
        // sidebar jumps to the selected section. The chain is
        // `open_settings_at_section` (mshell-settings static) →
        // registered closure in mshell-core/relm_app.rs →
        // tokio mpsc → ShellInput::OpenSettingsAtSection →
        // FrameInput::OpenSettingsAtSection →
        // SettingsWindowInput::ActivateSection. We avoid going
        // through `mshellctl` here so the launcher doesn't
        // depend on a CLI being on $PATH for what's really an
        // in-process navigation.
        let settings_provider = SettingsProvider::new(Rc::new(move |section_id| {
            mshell_settings::open_settings_at_section(section_id);
        }));

        let mut runtime = LauncherRuntime::new(FrecencyStore::load());
        // Order matters for empty-query browse — first-registered
        // providers' results show first when the user opens the
        // launcher without typing. Apps go first (the most common
        // browse target), Windows second (alt-tab replacement
        // for `Open Apps Switcher` muscle memory), then the
        // typed-only providers in roughly "expected frequency".
        // ProviderListProvider needs a callback that rewrites
        // the launcher's search entry — registers BEFORE Apps so
        // its `;` palette intercepts before the apps fuzzy
        // matcher gets a chance.
        let sender_for_search = sender.clone();
        let provider_list_provider = ProviderListProvider::new(Rc::new(move |text: &str| {
            // Channel-back to the widget so it can call
            // `search_entry.set_text(...)` from the GTK main
            // thread.
            let _ = sender_for_search.input_sender().send(
                AppLauncherInput::FilterChanged(text.to_string()),
            );
        }));

        runtime.register(Box::new(AppsProviderHandle(apps_provider.clone())));
        runtime.register(Box::new(WindowsProvider::new()));
        runtime.register(Box::new(CalculatorProvider::new()));
        runtime.register(Box::new(SessionProvider::new()));
        runtime.register(Box::new(MctlProvider::new()));
        runtime.register(Box::new(settings_provider));
        runtime.register(Box::new(ClipboardProvider::new()));
        runtime.register(Box::new(ScriptsProvider::new()));
        runtime.register(Box::new(SymbolsProvider::new()));
        runtime.register(Box::new(EmojiProvider::new()));
        runtime.register(Box::new(WebsearchProvider::new()));
        runtime.register(Box::new(provider_list_provider));
        runtime.register(Box::new(PlayerctlProvider::new()));
        runtime.register(Box::new(ArchLinuxPkgsProvider::new()));
        runtime.register(Box::new(WireplumberProvider::new()));
        runtime.register(Box::new(BluetoothProvider::new()));
        runtime.register(Box::new(CommandProvider::new()));

        // Sender clone for the dynamic-box factory's per-row
        // controller wiring — we need to keep the original sender
        // for the rest of init.
        let sender_for_rows = sender.clone();
        let factory = DynamicBoxFactory::<LauncherItem, String> {
            id: Box::new(|item| item.id.clone()),
            create: Box::new(move |item| {
                // Pass the whole LauncherItem by value into the
                // row component. Item itself isn't cloneable
                // (Rc<dyn Fn()> is, but the runtime moved the
                // owned value to us already) so we move it.
                let controller: Controller<LauncherRowModel> = LauncherRowModel::builder()
                    .launch(LauncherRowInit {
                        item: clone_launcher_item(item),
                    })
                    .forward(sender_for_rows.input_sender(), |out| match out {
                        LauncherRowOutput::Activated(id) => AppLauncherInput::ActivateRow(id),
                    });
                Box::new(controller) as Box<dyn GenericWidgetController>
            }),
            update: None,
        };

        let dynamic: Controller<DynamicBoxModel<LauncherItem, String>> = DynamicBoxModel::builder()
            .launch(DynamicBoxInit {
                factory,
                orientation: gtk::Orientation::Vertical,
                spacing: 10,
                transition_type: RevealerTransitionType::SlideDown,
                transition_duration_ms: 0,
                reverse: false,
                retain_entries: true,
                allow_drag_and_drop: false,
            })
            .detach();

        // Keyboard navigation. See module-level docs for the full
        // list of accepted bindings.
        let key_controller = gtk::EventControllerKey::new();
        let sender_clone = sender.clone();
        key_controller.connect_key_pressed(move |_, key, _, modifier| {
            let ctrl = modifier.contains(gdk::ModifierType::CONTROL_MASK);
            let shift = modifier.contains(gdk::ModifierType::SHIFT_MASK);
            let is_down = matches!(key, gdk::Key::Down)
                || (matches!(key, gdk::Key::Tab) && !shift)
                || (ctrl && matches!(key, gdk::Key::n | gdk::Key::N | gdk::Key::j | gdk::Key::J));
            let is_up = matches!(key, gdk::Key::Up | gdk::Key::ISO_Left_Tab)
                || (matches!(key, gdk::Key::Tab) && shift)
                || (ctrl && matches!(key, gdk::Key::p | gdk::Key::P | gdk::Key::k | gdk::Key::K));
            if is_down {
                sender_clone.input(AppLauncherInput::DownPressed);
                glib::Propagation::Stop
            } else if is_up {
                sender_clone.input(AppLauncherInput::UpPressed);
                glib::Propagation::Stop
            } else if matches!(key, gdk::Key::Escape) {
                let _ = sender_clone.output(AppLauncherOutput::CloseMenu);
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });

        let mut effect_scope = EffectScope::new();

        // Re-render results when the icon theme changes — apps
        // need their matugen-filtered icons re-applied.
        let sender_clone = sender.clone();
        effect_scope.push(move |_| {
            let _ = config_manager()
                .config()
                .theme()
                .icons()
                .app_icon_theme()
                .get();
            let _ = config_manager()
                .config()
                .theme()
                .icons()
                .apply_theme_filter()
                .get();
            let _ = config_manager().config().theme().theme().get();
            let _ = config_manager()
                .config()
                .theme()
                .icons()
                .filter_strength()
                .get();
            let _ = config_manager()
                .config()
                .theme()
                .icons()
                .monochrome_strength()
                .get();
            let _ = config_manager()
                .config()
                .theme()
                .icons()
                .contrast_strength()
                .get();
            sender_clone.input(AppLauncherInput::ThemeChanged);
        });

        // Refresh apps when desktop entries change (install /
        // uninstall while the launcher is open).
        let sender_clone = sender.clone();
        let monitor = gio::AppInfoMonitor::get();
        monitor.connect_changed(move |_| {
            sender_clone.input(AppLauncherInput::FilterChanged(String::new()));
        });

        // Initial population — runtime.on_opened triggers Apps
        // provider refresh, FilterChanged populates the list.
        runtime.on_opened();

        // Close-sender wired after construction so activate_id
        // can fire a CloseMenu output without going through the
        // input loop (which would race the row-controller
        // teardown).
        let close_sender_cell: RefCell<Option<Box<dyn Fn() + 'static>>> = RefCell::new(None);
        let sender_for_close = sender.clone();
        *close_sender_cell.borrow_mut() = Some(Box::new(move || {
            let _ = sender_for_close.output(AppLauncherOutput::CloseMenu);
        }));

        let model = AppLauncherModel {
            dynamic_box: dynamic,
            runtime: RefCell::new(runtime),
            apps_provider,
            close_sender: close_sender_cell,
            filter: String::new(),
            results: Vec::new(),
            selected_id: None,
            is_revealed: false,
            _effects: effect_scope,
        };

        let widgets = view_output!();
        widgets.apps_box.append(model.dynamic_box.widget());
        widgets.root.add_controller(key_controller);

        sender.input(AppLauncherInput::FilterChanged(String::new()));

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
            AppLauncherInput::FilterChanged(filter) => {
                self.filter = filter;
                self.recompute_results();
                self.push_results_to_dynamic_box();
                self.broadcast_selection();
            }
            AppLauncherInput::Activate => {
                // Enter on the search entry — activate whatever
                // row is currently selected.
                tracing::info!(target: "mshell::launcher", "Activate input, selected_id={:?}", self.selected_id);
                if let Some(id) = &self.selected_id.clone() {
                    self.activate_id(id);
                }
            }
            AppLauncherInput::ActivateRow(id) => {
                tracing::info!(target: "mshell::launcher", "ActivateRow input id={id}");
                self.activate_id(&id);
            }
            AppLauncherInput::ShowHiddenAppsChanged => {
                let new_state = !self.apps_provider.show_hidden();
                self.apps_provider.set_show_hidden(new_state);
                self.recompute_results();
                self.push_results_to_dynamic_box();
                self.broadcast_selection();
            }
            AppLauncherInput::ParentRevealChanged(revealed) => {
                if revealed && !self.is_revealed {
                    if let Some(window) = widgets.apps_box.toplevel_window() {
                        window.set_keyboard_mode(KeyboardMode::Exclusive);
                    }
                    self.runtime.borrow_mut().on_opened();
                    self.filter.clear();
                    widgets.search_entry.set_text("");
                    widgets.search_entry.grab_focus();
                    self.recompute_results();
                    self.push_results_to_dynamic_box();
                    self.broadcast_selection();
                } else if !revealed && self.is_revealed {
                    if let Some(window) = widgets.apps_box.toplevel_window() {
                        window.set_keyboard_mode(KeyboardMode::None);
                    }
                    self.runtime.borrow_mut().on_closed();
                    self.runtime.borrow_mut().flush();
                }
                self.is_revealed = revealed;
            }
            AppLauncherInput::DownPressed => {
                self.move_selection(1);
                self.broadcast_selection();
                self.ensure_selected_visible(&widgets.scrolled_window);
            }
            AppLauncherInput::UpPressed => {
                self.move_selection(-1);
                self.broadcast_selection();
                self.ensure_selected_visible(&widgets.scrolled_window);
            }
            AppLauncherInput::ThemeChanged => {
                // Force a full re-render so app icons pick up the
                // new matugen filter values.
                self.push_results_to_dynamic_box();
                self.broadcast_selection();
            }
        }

        self.update_view(widgets, sender);
    }
}

impl AppLauncherModel {
    fn recompute_results(&mut self) {
        self.results = self.runtime.borrow().query(&self.filter);
        // Keep selection on the first row by default. The runtime
        // already sorts so [0] is the best match.
        self.selected_id = self.results.first().map(|i| i.id.clone());
    }

    fn push_results_to_dynamic_box(&self) {
        // Materialise items by cloning each one (Rc<dyn Fn> is
        // cheap to clone); the dynamic box keeps controllers
        // keyed by id and replaces a row's controller only when
        // its key changes — so re-sending the same items just
        // re-runs the factory once per visible row.
        let cloned: Vec<LauncherItem> = self.results.iter().map(clone_launcher_item).collect();
        let _ = self
            .dynamic_box
            .sender()
            .send(DynamicBoxInput::SetItems(cloned));
    }

    fn broadcast_selection(&self) {
        let selected = self.selected_id.clone().unwrap_or_default();
        self.dynamic_box.model().for_each_entry(|_, entry| {
            if let Some(ctrl) = entry
                .controller
                .as_ref()
                .downcast_ref::<Controller<LauncherRowModel>>()
            {
                let _ = ctrl
                    .sender()
                    .send(LauncherRowInput::SelectionChanged(selected.clone()));
            }
        });
    }

    fn move_selection(&mut self, delta: isize) {
        if self.results.is_empty() {
            self.selected_id = None;
            return;
        }
        let current = self
            .selected_id
            .as_ref()
            .and_then(|id| self.results.iter().position(|i| &i.id == id))
            .unwrap_or(0);
        let target = (current as isize + delta).clamp(0, self.results.len() as isize - 1) as usize;
        self.selected_id = Some(self.results[target].id.clone());
    }

    fn activate_id(&mut self, id: &str) {
        let Some(item) = self.results.iter().find(|i| i.id == id) else {
            tracing::warn!(target: "mshell::launcher", "activate_id: no item with id={id} in {} results", self.results.len());
            return;
        };
        if let Some(key) = &item.usage_key {
            self.runtime.borrow_mut().record_usage(key);
        }
        (item.on_activate)();
        // Most activations want the launcher dismissed after
        // they run — Apps, Calc, Cmd, Session, Mctl, Scripts,
        // Windows, Clipboard all expect the panel to go away.
        //
        // Settings is the exception: its on_activate kicks off a
        // tokio-bridge chain that ends with `toggle_menu(SETTINGS_MENU)`
        // making Settings the visible stack child (auto-hiding
        // the launcher via stack-swap). If we also fired
        // `CloseMenus` here the two messages would race in the
        // tokio scheduler — sometimes the close-all arrives
        // *after* the settings-open and slams Settings back off
        // half a frame after it appears. Skipping the post-close
        // for `settings:*` items lets the section-nav chain own
        // the visibility transition unambiguously.
        let auto_close = !id.starts_with("settings:");
        if auto_close {
            let _ = self.close_sender.borrow().as_ref().map(|s| s());
        }
    }

    fn ensure_selected_visible(&self, scrolled_window: &ScrolledWindow) {
        let vadj = scrolled_window.vadjustment();
        let Some(selected_key) = self.selected_id.clone() else {
            return;
        };
        let container = self.dynamic_box.widget().clone().upcast::<gtk::Widget>();
        for key in self.dynamic_box.model().order.iter() {
            if key != &selected_key {
                continue;
            }
            if let Some(entry) = self.dynamic_box.model().entries.get(key) {
                if !entry.revealer.is_visible() {
                    return;
                }
                let Some(bounds) = entry.revealer.compute_bounds(&container) else {
                    return;
                };
                let y = bounds.y() as f64;
                let height = bounds.height() as f64;
                let view_start = vadj.value();
                let view_end = view_start + vadj.page_size();
                if y < view_start {
                    vadj.set_value(y);
                } else if y + height > view_end {
                    vadj.set_value((y + height - vadj.page_size()).max(0.0));
                }
                return;
            }
        }
    }
}

/// `LauncherItem` is intentionally not `Clone` (the runtime hands
/// ownership to the UI in one go) but the UI does need to clone
/// items: once for the dynamic box, once for the runtime's
/// internal sort buffer. This helper centralises the field-wise
/// copy so future fields don't get forgotten.
fn clone_launcher_item(src: &LauncherItem) -> LauncherItem {
    LauncherItem {
        id: src.id.clone(),
        name: src.name.clone(),
        description: src.description.clone(),
        icon: src.icon.clone(),
        icon_is_path: src.icon_is_path,
        score: src.score,
        provider_name: src.provider_name.clone(),
        usage_key: src.usage_key.clone(),
        on_activate: src.on_activate.clone(),
    }
}

/// Helper a future PR will call from notify-style providers; for
/// now lives next to the widget so we don't have to grow the
/// launcher crate with notify-rust dep before there's a real
/// caller.
#[allow(dead_code)]
fn _placeholder_notify_helper() {}

/// Thin newtype wrapper that lets us hand a shared `Rc<AppsProvider>`
/// to the runtime (which wants `Box<dyn Provider>`) while keeping
/// our own clone for `show_hidden` toggling.
struct AppsProviderHandle(Rc<AppsProvider>);

impl mshell_launcher::Provider for AppsProviderHandle {
    fn name(&self) -> &str {
        self.0.name()
    }

    fn handles_search(&self) -> bool {
        self.0.handles_search()
    }

    fn handles_command(&self, q: &str) -> bool {
        self.0.handles_command(q)
    }

    fn commands(&self) -> Vec<LauncherItem> {
        self.0.commands()
    }

    fn search(&self, q: &str) -> Vec<LauncherItem> {
        self.0.search(q)
    }

    fn on_opened(&mut self) {
        // AppsProvider uses interior mutability (RefCell) so we
        // can call its mutable-looking ops through &self.
        self.0.refresh();
    }
}
