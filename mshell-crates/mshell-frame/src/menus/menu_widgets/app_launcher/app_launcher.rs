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
//! Power-user bindings inspired by walker (quick-activate) and
//! noctalia (Tab cycle, Delete, …):
//!
//! | Key                  | Action                              |
//! |---|---|
//! | `↓` / `Ctrl+N` / `Ctrl+J` | select next                    |
//! | `↑` / `Ctrl+P` / `Ctrl+K` | select previous                |
//! | `PageDown`           | jump 10 rows forward                |
//! | `PageUp`             | jump 10 rows backward               |
//! | `Tab`                | next provider category              |
//! | `Shift+Tab`          | previous provider category          |
//! | `Enter`              | activate selected                   |
//! | `Ctrl+Enter`         | provider's alt action (if any)      |
//! | `Ctrl+1` .. `Ctrl+9` | activate the Nth result             |
//! | `Ctrl+Shift+P`       | toggle pin on selected (★)          |
//! | `Delete`             | delete frecency/history entry       |
//! | `Ctrl+E`             | toggle fuzzy / exact-substring mode |
//! | `Ctrl+R`             | resume last query                   |
//! | `Esc`                | close                               |
//!
//! Ctrl+N rather than Alt+N for quick activate because margo's
//! compositor config uses Alt+N for user-defined dispatches; the
//! launcher would otherwise have to fight the compositor's bind.
//!
//! Note: we use **Ctrl+Shift+P** for pin (instead of plain Ctrl+P)
//! because Ctrl+P is the historical "previous selection" emacs
//! binding the launcher already exposes — overloading it would
//! break a long-standing muscle-memory shortcut.

use crate::menus::menu_widgets::app_launcher::apps_provider::AppsProvider;
use crate::menus::menu_widgets::app_launcher::clipboard_provider::ClipboardProvider;
use crate::menus::menu_widgets::app_launcher::launcher_row::{
    LauncherRowInit, LauncherRowInput, LauncherRowModel, LauncherRowOutput,
};
use crate::menus::menu_widgets::app_launcher::tags_provider::TagsProvider;
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
use mshell_config::schema::config::{
    ConfigStoreFields, IconsStoreFields, LauncherStoreFields, PassStoreFields, ThemeStoreFields,
};
use mshell_launcher::providers::{
    ArchLinuxPkgsProvider, BluetoothProvider, CalculatorProvider, CommandProvider, EmojiProvider,
    MctlProvider, PassProvider, PlayerctlProvider, ProviderListProvider, ScriptsProvider,
    SessionProvider, SettingsProvider, SshProvider, SymbolsProvider, WebsearchProvider,
    WireplumberProvider,
};
use mshell_launcher::{DisplayItem, FrecencyStore, LauncherItem, LauncherRuntime};
use reactive_graph::traits::*;
use relm4::gtk::glib;
use relm4::gtk::prelude::*;
use relm4::gtk::{RevealerTransitionType, ScrolledWindow, gdk, gio, pango};
use relm4::{
    Component, ComponentController, ComponentParts, ComponentSender, Controller, RelmWidgetExt, gtk,
};
use std::cell::RefCell;
use std::rc::Rc;

pub(crate) struct AppLauncherModel {
    dynamic_box: Controller<DynamicBoxModel<DisplayItem, String>>,
    /// Runtime owns the providers + frecency + pins. Wrapped in
    /// `RefCell` because the Apps provider needs to be mutated
    /// (toggle show_hidden) from within the widget's update path
    /// while the runtime simultaneously borrows providers
    /// immutably during `query()`. Two-borrow split via
    /// `Rc<AppsProvider>` on the side.
    runtime: RefCell<LauncherRuntime>,
    /// Shared handle to the Apps provider so we can toggle the
    /// `show_hidden` flag without going through the runtime.
    apps_provider: Rc<AppsProvider>,
    /// Closes the launcher menu. Set in init; called after every
    /// item activation. RefCell<Option<...>> so the closure can
    /// be stamped post-construction without an extra cell on the
    /// model type.
    close_sender: RefCell<Option<Box<dyn Fn() + 'static>>>,
    filter: String,
    results: Vec<DisplayItem>,
    /// Stable id of the currently-selected row. We store the id
    /// rather than an index so reordering between keystrokes
    /// doesn't strand the highlight on a different row.
    selected_id: Option<String>,
    /// Cached label for the active provider category — drives the
    /// tab-strip highlight without re-asking the runtime on every
    /// repaint.
    active_category: String,
    is_revealed: bool,
    /// Detail for the selected result (right preview pane). `None`
    /// hides the pane so the list fills the width.
    current_preview: Option<mshell_launcher::LauncherPreview>,
    /// Per-widget provider that paints the colour swatch's background;
    /// reloaded with the selected colour in `refresh_preview`.
    swatch_provider: gtk::CssProvider,
    /// Last CSS loaded into `swatch_provider`. Loading CSS triggers a
    /// display-wide restyle, so we skip the reload when the swatch
    /// hasn't changed — otherwise every Ctrl+N/K keystroke paid for a
    /// global restyle and the list crawled.
    swatch_css: String,
    /// Settings → Launcher knobs, mirrored live from the config store.
    show_preview: bool,
    compact_rows: bool,
    large_app_icons: bool,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum AppLauncherInput {
    FilterChanged(String),
    /// Plain Enter on the search entry — activate the currently-
    /// selected row.
    Activate,
    /// Ctrl+Enter — activate the selected row's *alt* action (the
    /// provider's `alt_action`, if any; otherwise falls back to
    /// the regular activation).
    AltActivate,
    ParentRevealChanged(bool),
    DownPressed,
    UpPressed,
    /// PageDown — jump 10 rows forward.
    PageDownPressed,
    /// PageUp — jump 10 rows backward.
    PageUpPressed,
    /// User pressed Enter on a specific row (via mouse click or
    /// Tab+Enter on the focused row). Carries the row id.
    ActivateRow(String),
    /// Alt+N — activate the row with quick_key = N. Carries the
    /// digit (1..=9) so the handler can find the matching row.
    QuickActivate(u8),
    /// Tab — cycle to the next provider category (Apps → Insert →
    /// Compositor → All → …). `delta` is +1 or -1.
    CycleCategory(i32),
    /// Direct-jump to a named category. Fired by mouse-click on a
    /// category pill in the tab strip.
    SelectCategory(String),
    /// Ctrl+Shift+P — toggle pin on the selected item.
    TogglePin,
    /// Delete — remove the selected item from its provider's
    /// frecency / history store (provider must opt in via
    /// `can_delete`).
    DeleteEntry,
    /// Ctrl+E — toggle fuzzy ↔ exact-substring matching.
    ToggleExactSearch,
    /// Ctrl+R — repopulate the search entry with the last query
    /// the launcher saw before closing.
    ResumeLastQuery,
    /// Programmatic search-text swap — used by the
    /// `ProviderListProvider` cheatsheet to drop the chosen
    /// prefix's example query into the search entry so the user
    /// can refine + Enter from there. Distinguished from
    /// `FilterChanged` because we *also* need to update the
    /// `gtk::Entry`'s visible text, not just the internal
    /// filter state.
    SetSearchText(String),
    ShowHiddenAppsChanged,
    ThemeChanged,
    /// Settings → Launcher toggle changed (preview / density / icons).
    LauncherConfigChanged,
    /// Right-click context menu → Pin/Unpin. Carries the
    /// item's `usage_key` so the runtime can persist the toggle
    /// regardless of which row is currently keyboard-selected.
    TogglePinFromRow(String),
    /// Right-click context menu → Hide/Unhide.
    ToggleHiddenFromRow(String),
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
            // Base class + Settings-driven modifiers (density / icon size).
            #[watch]
            set_css_classes: &{
                let mut v = vec!["app-launcher-menu-widget"];
                if model.compact_rows {
                    v.push("app-launcher-compact");
                }
                if model.large_app_icons {
                    v.push("launcher-large-icons");
                }
                v
            },
            set_orientation: gtk::Orientation::Vertical,
            // Fill the parent menu surface's allocation — the
            // outer width is **owned by the config** (via
            // `config.menus.app_launcher_menu.minimum_width`,
            // which the user can tweak from Settings → Menus).
            // No hardcoded `set_width_request` here: an earlier
            // pass pinned the root at 370 px to chase a jitter
            // bug, but that floor silently overrode any value the
            // user picked smaller than the parent allocation, so
            // the Settings spinner became read-only in practice.
            //
            // Width stability across category cycles / pin
            // toggles is guaranteed upstream by removing the
            // natural-width fluctuation sources (the selected
            // pill's font-weight bump, the contextual chip
            // count, the Pin/Unpin label flip) — see the
            // `rebuild_binds_strip` and category-pill SCSS for
            // the details.
            set_hexpand: true,
            set_halign: gtk::Align::Fill,

            gtk::Box {
                add_css_class: "app-launcher-search-row",
                set_orientation: gtk::Orientation::Horizontal,

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

                // Exact-search indicator — small "≈/=" pill next to
                // the eye toggle that tells the user whether fuzzy
                // (~) or exact (=) matching is active. CSS handles
                // the visual style; the label text is the only
                // model-bound thing.
                #[name = "exact_indicator"]
                gtk::Label {
                    add_css_class: "app-launcher-exact-mode",
                    #[watch]
                    set_label: if model.runtime.borrow().is_exact_search() { "=" } else { "~" },
                    set_margin_start: 6,
                    set_margin_end: 6,
                    set_valign: gtk::Align::Center,
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

            // Category tab strip — noctalia-style. One pill per
            // distinct provider category in registration order
            // (with implicit "All" first). The selected pill gets
            // the `selected` CSS class. Tab / Shift+Tab cycle
            // through them; clicking a pill jumps directly. Rebuilt
            // any time recompute_results runs so a freshly-cycled
            // category swaps highlight without a full re-render.
            #[name = "category_strip"]
            gtk::Box {
                add_css_class: "app-launcher-category-strip",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 3,
                set_margin_bottom: 8,
                set_halign: gtk::Align::Start,
                set_hexpand: true,
            },

            // Two-zone content: result list (left) + preview pane
            // (right). The preview hides when the selection yields no
            // detail, so the list reclaims the full width.
            gtk::Box {
                add_css_class: "app-launcher-content",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 12,
                set_vexpand: true,

                #[name = "scrolled_window"]
                ScrolledWindow {
                    set_vscrollbar_policy: gtk::PolicyType::Automatic,
                    set_hscrollbar_policy: gtk::PolicyType::Never,
                    set_propagate_natural_height: true,
                    // Don't let an extra-long row name push the
                    // launcher wider than the parent's allocation —
                    // labels inside each row already ellipsize, so
                    // capping the natural width here is what makes
                    // the cap actually fire.
                    set_propagate_natural_width: false,
                    set_hexpand: true,

                    #[name = "apps_box"]
                    gtk::Box {
                        set_hexpand: true,
                    },
                },

                gtk::Box {
                    add_css_class: "app-launcher-preview",
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 6,
                    set_width_request: 220,
                    #[watch]
                    set_visible: model.show_preview && model.current_preview.is_some(),

                    gtk::Label {
                        add_css_class: "app-launcher-preview-title",
                        set_halign: gtk::Align::Start,
                        set_xalign: 0.0,
                        set_wrap: true,
                        #[watch]
                        set_label: model
                            .current_preview
                            .as_ref()
                            .map(|p| p.title.as_str())
                            .unwrap_or(""),
                    },

                    #[name = "preview_swatch"]
                    gtk::Box {
                        add_css_class: "app-launcher-preview-swatch",
                        set_height_request: 44,
                        #[watch]
                        set_visible: model
                            .current_preview
                            .as_ref()
                            .map(|p| matches!(p.kind, mshell_launcher::PreviewKind::Color))
                            .unwrap_or(false),
                    },

                    gtk::Label {
                        set_halign: gtk::Align::Start,
                        set_xalign: 0.0,
                        set_wrap: true,
                        set_vexpand: true,
                        set_valign: gtk::Align::Start,
                        #[watch]
                        set_css_classes: if matches!(
                            model.current_preview.as_ref().map(|p| &p.kind),
                            Some(mshell_launcher::PreviewKind::Mono)
                        ) {
                            &["app-launcher-preview-body", "mono"]
                        } else {
                            &["app-launcher-preview-body"]
                        },
                        #[watch]
                        set_label: model
                            .current_preview
                            .as_ref()
                            .map(|p| p.body.as_str())
                            .unwrap_or(""),
                    },
                },
            },

            // Keybind hint strip — walker-style footer listing the
            // currently-relevant shortcuts. A FlowBox (not a single-
            // line Box) so chips **wrap** onto a second line and each
            // shows its FULL label — no ellipsis. The FlowBox's
            // minimum width is just the widest single chip, so it
            // never forces the panel wider; it simply reflows within
            // whatever width the panel has.
            #[name = "binds_strip"]
            gtk::FlowBox {
                add_css_class: "app-launcher-binds-strip",
                set_orientation: gtk::Orientation::Horizontal,
                set_selection_mode: gtk::SelectionMode::None,
                set_column_spacing: 8,
                set_row_spacing: 4,
                set_max_children_per_line: 24,
                set_min_children_per_line: 1,
                set_homogeneous: false,
                set_margin_top: 10,
                set_halign: gtk::Align::Fill,
                set_hexpand: true,
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let apps_provider = Rc::new(AppsProvider::new());

        let settings_provider = SettingsProvider::new(Rc::new(move |section_id| {
            mshell_settings::open_settings_at_section(section_id);
        }));

        let mut runtime = LauncherRuntime::new(FrecencyStore::load());
        let sender_for_search = sender.clone();
        let provider_list_provider = ProviderListProvider::new(Rc::new(move |text: &str| {
            let _ = sender_for_search
                .input_sender()
                .send(AppLauncherInput::SetSearchText(text.to_string()));
        }));

        runtime.register(Box::new(AppsProviderHandle(apps_provider.clone())));
        runtime.register(Box::new(WindowsProvider::new()));
        runtime.register(Box::new(TagsProvider::new()));
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
        runtime.register(Box::new(SshProvider::new()));
        // pass: store path follows Settings → Launcher (else
        // $PASSWORD_STORE_DIR / ~/.password-store). Resolver re-reads on
        // each open, so the setting applies without a shell restart.
        runtime.register(Box::new(PassProvider::with_resolver(Box::new(|| {
            let configured = config_manager()
                .config()
                .pass()
                .store_path()
                .get_untracked();
            let trimmed = configured.trim();
            if trimmed.is_empty() {
                mshell_launcher::providers::pass::default_store_dir()
            } else if let Some(rest) = trimmed.strip_prefix("~/") {
                std::env::var_os("HOME")
                    .map(|h| std::path::PathBuf::from(h).join(rest))
                    .unwrap_or_else(|| std::path::PathBuf::from(trimmed))
            } else {
                std::path::PathBuf::from(trimmed)
            }
        }))));
        runtime.register(Box::new(CommandProvider::new()));

        let sender_for_rows = sender.clone();
        let factory = DynamicBoxFactory::<DisplayItem, String> {
            id: Box::new(|d| d.item.id.clone()),
            create: Box::new(move |d| {
                let controller: Controller<LauncherRowModel> = LauncherRowModel::builder()
                    .launch(LauncherRowInit {
                        display: clone_display_item(d),
                    })
                    .forward(sender_for_rows.input_sender(), |out| match out {
                        LauncherRowOutput::Activated(id) => AppLauncherInput::ActivateRow(id),
                        LauncherRowOutput::TogglePin(key) => {
                            AppLauncherInput::TogglePinFromRow(key)
                        }
                        LauncherRowOutput::ToggleHidden(key) => {
                            AppLauncherInput::ToggleHiddenFromRow(key)
                        }
                    });
                Box::new(controller) as Box<dyn GenericWidgetController>
            }),
            update: None,
        };

        let dynamic: Controller<DynamicBoxModel<DisplayItem, String>> = DynamicBoxModel::builder()
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

        // Keyboard navigation — the full power-user binding set. See
        // module-level docs for the table. Capture phase so our
        // handler runs *before* the search entry's default Tab
        // behaviour (which moves focus to the next widget and would
        // otherwise swallow Tab before our category-cycle code
        // could see it).
        let key_controller = gtk::EventControllerKey::new();
        key_controller.set_propagation_phase(gtk::PropagationPhase::Capture);
        let sender_clone = sender.clone();
        key_controller.connect_key_pressed(move |_, key, _, modifier| {
            let ctrl = modifier.contains(gdk::ModifierType::CONTROL_MASK);
            let shift = modifier.contains(gdk::ModifierType::SHIFT_MASK);
            let alt = modifier.contains(gdk::ModifierType::ALT_MASK);

            // Ctrl+1..Ctrl+9 → quick activate. Handled first so the
            // digit keys don't fall through to the search entry as
            // typed input. Bound to Ctrl rather than Alt because
            // margo's compositor config already uses Alt+N for
            // user-defined dispatches; the launcher's keyboard
            // grab catches Ctrl+digit cleanly without colliding
            // with the compositor binds.
            if ctrl && !alt {
                let digit = match key {
                    gdk::Key::_1 => Some(1u8),
                    gdk::Key::_2 => Some(2),
                    gdk::Key::_3 => Some(3),
                    gdk::Key::_4 => Some(4),
                    gdk::Key::_5 => Some(5),
                    gdk::Key::_6 => Some(6),
                    gdk::Key::_7 => Some(7),
                    gdk::Key::_8 => Some(8),
                    gdk::Key::_9 => Some(9),
                    _ => None,
                };
                if let Some(n) = digit {
                    sender_clone.input(AppLauncherInput::QuickActivate(n));
                    return glib::Propagation::Stop;
                }
            }

            // Tab / Shift+Tab → category cycle. Replaces the old
            // "Tab = Down" binding — that's still available via
            // arrow keys / Ctrl+N. The category cycle is the
            // noctalia pattern and lets the user sweep through
            // provider buckets when the empty-browse list is too
            // long to scroll.
            if matches!(key, gdk::Key::Tab) && !ctrl && !alt {
                let delta = if shift { -1 } else { 1 };
                sender_clone.input(AppLauncherInput::CycleCategory(delta));
                return glib::Propagation::Stop;
            }
            if matches!(key, gdk::Key::ISO_Left_Tab) && !ctrl && !alt {
                sender_clone.input(AppLauncherInput::CycleCategory(-1));
                return glib::Propagation::Stop;
            }

            // Ctrl+Shift+P → toggle pin. Single Ctrl+P stays
            // bound to "previous selection" (emacs muscle memory).
            if ctrl && shift && matches!(key, gdk::Key::P | gdk::Key::p) {
                sender_clone.input(AppLauncherInput::TogglePin);
                return glib::Propagation::Stop;
            }

            // Ctrl+E → toggle exact-substring mode.
            if ctrl && !shift && !alt && matches!(key, gdk::Key::E | gdk::Key::e) {
                sender_clone.input(AppLauncherInput::ToggleExactSearch);
                return glib::Propagation::Stop;
            }

            // Ctrl+R → resume last query.
            if ctrl && !shift && !alt && matches!(key, gdk::Key::R | gdk::Key::r) {
                sender_clone.input(AppLauncherInput::ResumeLastQuery);
                return glib::Propagation::Stop;
            }

            // Delete → remove the selected frecency / history entry.
            if matches!(key, gdk::Key::Delete) && !ctrl && !alt {
                sender_clone.input(AppLauncherInput::DeleteEntry);
                return glib::Propagation::Stop;
            }

            // PageUp / PageDown → 10-row jump.
            if matches!(key, gdk::Key::Page_Down) {
                sender_clone.input(AppLauncherInput::PageDownPressed);
                return glib::Propagation::Stop;
            }
            if matches!(key, gdk::Key::Page_Up) {
                sender_clone.input(AppLauncherInput::PageUpPressed);
                return glib::Propagation::Stop;
            }

            // Ctrl+Enter → alt action.
            if ctrl && !shift && matches!(key, gdk::Key::Return | gdk::Key::KP_Enter) {
                sender_clone.input(AppLauncherInput::AltActivate);
                return glib::Propagation::Stop;
            }

            // Existing navigation: Down / Up / Ctrl+N|J / Ctrl+P|K.
            // (Ctrl+P pinning lives behind the *Shift* combo handled
            // above so the bare Ctrl+P emacs binding is preserved.)
            let is_down = matches!(key, gdk::Key::Down)
                || (ctrl
                    && !shift
                    && matches!(key, gdk::Key::n | gdk::Key::N | gdk::Key::j | gdk::Key::J));
            let is_up = matches!(key, gdk::Key::Up)
                || (ctrl
                    && !shift
                    && matches!(key, gdk::Key::p | gdk::Key::P | gdk::Key::k | gdk::Key::K));
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

        // Live-track the Settings → Launcher toggles so changes apply
        // without reopening the launcher.
        let sender_clone = sender.clone();
        effect_scope.push(move |_| {
            let _ = config_manager().config().launcher().show_preview().get();
            let _ = config_manager().config().launcher().compact_rows().get();
            let _ = config_manager().config().launcher().large_app_icons().get();
            sender_clone.input(AppLauncherInput::LauncherConfigChanged);
        });

        let sender_clone = sender.clone();
        let monitor = gio::AppInfoMonitor::get();
        monitor.connect_changed(move |_| {
            sender_clone.input(AppLauncherInput::FilterChanged(String::new()));
        });

        runtime.on_opened();

        let close_sender_cell: RefCell<Option<Box<dyn Fn() + 'static>>> = RefCell::new(None);
        let sender_for_close = sender.clone();
        *close_sender_cell.borrow_mut() = Some(Box::new(move || {
            let _ = sender_for_close.output(AppLauncherOutput::CloseMenu);
        }));

        let active_category = runtime.active_category_label();

        let model = AppLauncherModel {
            dynamic_box: dynamic,
            runtime: RefCell::new(runtime),
            apps_provider,
            close_sender: close_sender_cell,
            filter: String::new(),
            results: Vec::new(),
            selected_id: None,
            active_category,
            is_revealed: false,
            current_preview: None,
            swatch_provider: gtk::CssProvider::new(),
            swatch_css: String::new(),
            show_preview: config_manager()
                .config()
                .launcher()
                .show_preview()
                .get_untracked(),
            compact_rows: config_manager()
                .config()
                .launcher()
                .compact_rows()
                .get_untracked(),
            large_app_icons: config_manager()
                .config()
                .launcher()
                .large_app_icons()
                .get_untracked(),
            _effects: effect_scope,
        };

        let widgets = view_output!();
        widgets.apps_box.append(model.dynamic_box.widget());
        widgets.root.add_controller(key_controller);

        // The colour-swatch background is painted by a dedicated
        // CssProvider (reloaded per selection in `refresh_preview`).
        // Registered display-wide — the `.app-launcher-preview-swatch`
        // selector scopes it — because the per-widget
        // `StyleContext::add_provider` is deprecated in GTK4.
        if let Some(display) = gtk::gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                &model.swatch_provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }

        // Initial category strip + binds strip + result render.
        // All three depend on the runtime being constructed so we
        // run them after the model is built but before the first
        // frame.
        rebuild_category_strip(&widgets.category_strip, &model, &sender);
        rebuild_binds_strip(&widgets.binds_strip, &model);

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
                if let Some(id) = &self.selected_id.clone() {
                    self.activate_id(id, /*alt=*/ false);
                }
            }
            AppLauncherInput::AltActivate => {
                if let Some(id) = &self.selected_id.clone() {
                    self.activate_id(id, /*alt=*/ true);
                }
            }
            AppLauncherInput::ActivateRow(id) => {
                self.activate_id(&id, /*alt=*/ false);
            }
            AppLauncherInput::QuickActivate(n) => {
                // Activate the row whose quick_key matches N (1..9).
                let target = self
                    .results
                    .iter()
                    .find(|d| d.quick_key == n.to_string())
                    .map(|d| d.item.id.clone());
                if let Some(id) = target {
                    self.activate_id(&id, /*alt=*/ false);
                }
            }
            AppLauncherInput::CycleCategory(delta) => {
                let new_label = self.runtime.borrow_mut().cycle_category(delta);
                self.active_category = new_label;
                rebuild_category_strip(&widgets.category_strip, self, &sender);
                self.recompute_results();
                self.push_results_to_dynamic_box();
                self.broadcast_selection();
            }
            AppLauncherInput::SelectCategory(label) => {
                let new_label = self.runtime.borrow_mut().select_category(&label);
                self.active_category = new_label;
                rebuild_category_strip(&widgets.category_strip, self, &sender);
                self.recompute_results();
                self.push_results_to_dynamic_box();
                self.broadcast_selection();
            }
            AppLauncherInput::TogglePin => {
                // Pin operates on the selected item's usage_key —
                // if the item has none (calculator results, command
                // palette entries) the action is silently a no-op.
                let key = self
                    .selected_id
                    .as_ref()
                    .and_then(|id| self.results.iter().find(|d| &d.item.id == id))
                    .and_then(|d| d.item.usage_key.clone());
                if let Some(k) = key {
                    let _ = self.runtime.borrow_mut().toggle_pin(&k);
                    self.recompute_results();
                    self.push_results_to_dynamic_box();
                    self.broadcast_selection();
                    self.broadcast_pin_state();
                }
            }
            AppLauncherInput::DeleteEntry => {
                // Snapshot the selected item; ask the runtime
                // whether the owning provider claims it; delete +
                // re-query.
                let selected_item = self
                    .selected_id
                    .as_ref()
                    .and_then(|id| self.results.iter().find(|d| &d.item.id == id))
                    .map(|d| clone_launcher_item(&d.item));
                if let Some(item) = selected_item
                    && self.runtime.borrow().can_delete(&item)
                {
                    self.runtime.borrow_mut().delete_item(&item);
                    self.recompute_results();
                    self.push_results_to_dynamic_box();
                    self.broadcast_selection();
                }
            }
            AppLauncherInput::ToggleExactSearch => {
                let _ = self.runtime.borrow_mut().toggle_exact_search();
                // Re-run the current query under the new matcher.
                self.recompute_results();
                self.push_results_to_dynamic_box();
                self.broadcast_selection();
            }
            AppLauncherInput::ResumeLastQuery => {
                let last = self.runtime.borrow().last_query().to_string();
                if !last.is_empty() {
                    widgets.search_entry.set_text(&last);
                    widgets.search_entry.set_position(-1);
                }
            }
            AppLauncherInput::SetSearchText(text) => {
                widgets.search_entry.set_text(&text);
                widgets.search_entry.set_position(-1);
                widgets.search_entry.grab_focus();
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
                    // Snapshot the current query for Ctrl+R the
                    // next time the launcher opens.
                    {
                        let mut rt = self.runtime.borrow_mut();
                        rt.remember_query(&self.filter);
                        rt.on_closed();
                        rt.flush();
                    }
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
            AppLauncherInput::PageDownPressed => {
                self.move_selection(10);
                self.broadcast_selection();
                self.ensure_selected_visible(&widgets.scrolled_window);
            }
            AppLauncherInput::PageUpPressed => {
                self.move_selection(-10);
                self.broadcast_selection();
                self.ensure_selected_visible(&widgets.scrolled_window);
            }
            AppLauncherInput::ThemeChanged => {
                self.push_results_to_dynamic_box();
                self.broadcast_selection();
            }
            AppLauncherInput::LauncherConfigChanged => {
                let cfg = config_manager();
                self.show_preview = cfg.config().launcher().show_preview().get_untracked();
                self.compact_rows = cfg.config().launcher().compact_rows().get_untracked();
                self.large_app_icons = cfg.config().launcher().large_app_icons().get_untracked();
                // The `#[watch]` bindings on the root classes + preview
                // pane re-apply after this update returns.
            }
            AppLauncherInput::TogglePinFromRow(key) => {
                if !key.is_empty() {
                    let _ = self.runtime.borrow_mut().toggle_pin(&key);
                    self.recompute_results();
                    self.push_results_to_dynamic_box();
                    self.broadcast_selection();
                    self.broadcast_pin_state();
                }
            }
            AppLauncherInput::ToggleHiddenFromRow(key) => {
                if !key.is_empty() {
                    let _ = self.runtime.borrow_mut().toggle_hidden(&key);
                    // Hidden items vanish from browse mode on
                    // requery (empty filter); when the user has a
                    // non-empty filter the row stays visible but
                    // the context-menu label flips.
                    self.recompute_results();
                    self.push_results_to_dynamic_box();
                    self.broadcast_selection();
                }
            }
        }

        // Keep the bind-hint footer in sync with the current
        // selection / category / pin / exact state. Cheap (≤ 7
        // chips) and the strip is highly contextual, so a refresh
        // after every input keeps the displayed shortcuts honest.
        rebuild_binds_strip(&widgets.binds_strip, self);

        self.update_view(widgets, sender);
    }
}

impl AppLauncherModel {
    fn recompute_results(&mut self) {
        self.results = self.runtime.borrow().query(&self.filter);
        self.selected_id = self.results.first().map(|d| d.item.id.clone());
        self.refresh_preview();
    }

    fn push_results_to_dynamic_box(&self) {
        let cloned: Vec<DisplayItem> = self.results.iter().map(clone_display_item).collect();
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

    /// Send each row its fresh pin state. Called after a pin toggle
    /// — the DynamicBox reconciler keeps existing controllers
    /// (factory has `update: None`), so without this broadcast the
    /// ★ glyph would be stuck on the pre-toggle state until the
    /// row is rebuilt for some other reason. Maps by row id, so
    /// rows whose pin state didn't change still get the same value
    /// sent back to them (cheap and idempotent).
    fn broadcast_pin_state(&self) {
        // Build an id → pinned lookup once instead of an O(n²)
        // scan inside the per-entry callback.
        let pinned_by_id: std::collections::HashMap<String, bool> = self
            .results
            .iter()
            .map(|d| (d.item.id.clone(), d.pinned))
            .collect();
        self.dynamic_box.model().for_each_entry(|key, entry| {
            if let Some(pinned) = pinned_by_id.get(key)
                && let Some(ctrl) = entry
                    .controller
                    .as_ref()
                    .downcast_ref::<Controller<LauncherRowModel>>()
            {
                let _ = ctrl.sender().send(LauncherRowInput::PinChanged(*pinned));
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
            .and_then(|id| self.results.iter().position(|d| &d.item.id == id))
            .unwrap_or(0);
        let target = (current as isize + delta).clamp(0, self.results.len() as isize - 1) as usize;
        self.selected_id = Some(self.results[target].item.id.clone());
        self.refresh_preview();
    }

    /// The currently highlighted result, if any.
    fn selected_display_item(&self) -> Option<&DisplayItem> {
        let id = self.selected_id.as_ref()?;
        self.results.iter().find(|d| &d.item.id == id)
    }

    /// Recompute the preview pane for the current selection. Asks the
    /// owning provider via the runtime; paints the colour swatch when
    /// the preview is a `Color`.
    fn refresh_preview(&mut self) {
        self.current_preview = self
            .selected_display_item()
            .and_then(|d| self.runtime.borrow().preview_for(&d.item));
        let swatch = self.current_preview.as_ref().and_then(|p| p.swatch.clone());
        let css = match swatch {
            Some(hex) => format!(".app-launcher-preview-swatch {{ background-color: {hex}; }}"),
            None => String::new(),
        };
        // Only reload when the swatch actually changed — `load_from_string`
        // forces a display-wide restyle, far too expensive to run on every
        // keyboard move.
        if css != self.swatch_css {
            self.swatch_provider.load_from_string(&css);
            self.swatch_css = css;
        }
    }

    fn activate_id(&mut self, id: &str, alt: bool) {
        let Some(display) = self.results.iter().find(|d| d.item.id == id) else {
            return;
        };
        // Snapshot the bits we need before borrowing the runtime
        // mutably — `record_usage` takes &mut.
        let usage_key = display.item.usage_key.clone();
        let item_clone = clone_launcher_item(&display.item);
        if let Some(key) = &usage_key {
            self.runtime.borrow_mut().record_usage(key);
        }
        if alt {
            // Provider's alt action if any; otherwise fall through
            // to the regular on_activate so Ctrl+Enter never feels
            // dead.
            let alt_fn = self.runtime.borrow().alt_action(&item_clone);
            match alt_fn {
                Some(f) => f(),
                None => (item_clone.on_activate)(),
            }
        } else {
            (item_clone.on_activate)();
        }
        // Same auto-close-skip as before: Settings owns its own
        // visibility transition via the section-nav chain.
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

/// One keybind hint = visible key chip + caption + whether it
/// applies to the current selection. Always-rendered hints have
/// `applicable: true` regardless of state; contextual hints (Pin /
/// Remove / Alt) flip their `applicable` flag based on the
/// selected row's capabilities.
///
/// We render *every* hint every time — the inapplicable ones go
/// to `opacity: 0` instead of being removed. That keeps the
/// strip's natural width identical across selections so the
/// launcher menu doesn't slide sideways when navigation toggles
/// the contextual chips.
struct BindHint {
    key: &'static str,
    label: &'static str,
    applicable: bool,
}

/// (Re)render the keybind hint strip at the bottom of the launcher.
/// Walker-style: each hint is a small chip showing the key combo
/// followed by what it does. The chip set is **fixed** in size —
/// always 9 chips, in a stable order — so the strip's intrinsic
/// width never changes as the selection moves. Contextual chips
/// (Alt action / Pin·Unpin / Remove) fade to `opacity: 0` when
/// they don't apply rather than being removed; the slot is still
/// allocated.
fn rebuild_binds_strip(strip: &gtk::FlowBox, model: &AppLauncherModel) {
    // Snapshot the selected row's capabilities so we can flip
    // contextual chips on/off without holding two runtime borrows.
    let selected = model
        .selected_id
        .as_ref()
        .and_then(|id| model.results.iter().find(|d| &d.item.id == id));
    let (has_alt, has_pin, has_delete) = if let Some(d) = selected {
        let item = &d.item;
        let rt = model.runtime.borrow();
        (
            rt.alt_action(item).is_some(),
            item.usage_key.is_some(),
            rt.can_delete(item),
        )
    } else {
        (false, false, false)
    };

    // Static caption — toggling between "Pin" and "Unpin" would
    // change the chip's natural width and slide every chip to its
    // right. The row's ★ glyph already shows whether the item is
    // pinned right now; the chip just labels what the keybind does
    // (toggle pin state).
    let pin_label: &'static str = "Pin";

    // Stable order matches the keyboard mental model: activation
    // first, then power keys, then panel-level binds (Exact /
    // Last / Close). Inapplicable chips reserve their slot via
    // opacity 0 so the strip's natural width is identical for
    // every selection.
    let hints: [BindHint; 9] = [
        BindHint {
            key: "↵",
            label: "Activate",
            applicable: true,
        },
        BindHint {
            key: "Ctrl ↵",
            label: "Alt action",
            applicable: has_alt,
        },
        BindHint {
            key: "Ctrl 1-9",
            label: "Quick",
            applicable: true,
        },
        BindHint {
            key: "Tab",
            label: "Categories",
            applicable: true,
        },
        BindHint {
            key: "Ctrl ⇧ P",
            label: pin_label,
            applicable: has_pin,
        },
        BindHint {
            key: "Del",
            label: "Remove",
            applicable: has_delete,
        },
        BindHint {
            key: "Ctrl E",
            label: "Exact",
            applicable: true,
        },
        BindHint {
            key: "Ctrl R",
            label: "Last",
            applicable: true,
        },
        BindHint {
            key: "Esc",
            label: "Close",
            applicable: true,
        },
    ];

    // Tear down old chips. GTK4 has no `clear()`.
    while let Some(child) = strip.first_child() {
        strip.remove(&child);
    }

    for hint in &hints {
        // Skip inapplicable contextual chips entirely. The FlowBox
        // reflows on its own, so we no longer need opacity-0 spacer
        // chips to keep a single-line strip's width stable.
        if !hint.applicable {
            continue;
        }

        let chip = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        chip.add_css_class("app-launcher-bind-chip");

        let key_lbl = gtk::Label::new(Some(hint.key));
        key_lbl.add_css_class("app-launcher-bind-key");
        chip.append(&key_lbl);

        // Full label, no ellipsize — the FlowBox wraps chips that
        // don't fit onto the next line instead of truncating them,
        // and a single chip's natural width never forces the panel
        // wider than its own ~one-chip minimum.
        let cap_lbl = gtk::Label::new(Some(hint.label));
        cap_lbl.add_css_class("app-launcher-bind-label");
        chip.append(&cap_lbl);

        strip.append(&chip);
    }
}

/// Icon name for a category pill. Maps each category label to a
/// themed icon that hints at the category's purpose without
/// committing to a text label. The pill renders the icon followed
/// by the label (icon-led layout) so the strip stays scannable
/// without giving up the textual hint.
///
/// Falls back to `"view-list-symbolic"` for unknown labels — that
/// way a future provider that lands with a fresh category still
/// gets a sensible default until someone adds a mapping here.
fn category_icon(label: &str) -> &'static str {
    match label {
        "All" => "view-grid-symbolic",
        "Apps" => "view-app-grid-symbolic",
        "Compositor" => "preferences-desktop-display-symbolic",
        "System" => "preferences-system-symbolic",
        "Run" => "utilities-terminal-symbolic",
        // input-keyboard-symbolic is the cross-theme name that
        // exists in MargoMaterial, kora, breeze, and Adwaita —
        // semantically a good fit for "type a character to
        // insert" (symbols / emoji / clipboard paste). The
        // previously-used "format-text-symbolic" doesn't exist in
        // MargoMaterial and rendered as a missing-icon glyph.
        "Insert" => "input-keyboard-symbolic",
        "Search" => "system-search-symbolic",
        "Connect" => "network-server-symbolic",
        _ => "view-list-symbolic",
    }
}

/// (Re)render the category tab strip. Clears the existing children
/// and stamps one button per category from the runtime, with the
/// active one carrying the `selected` CSS class. Each pill is an
/// `icon + label` pair — icon for at-a-glance recognition, label
/// for clarity. Clicking a pill jumps directly to that category.
fn rebuild_category_strip(
    strip: &gtk::Box,
    model: &AppLauncherModel,
    sender: &ComponentSender<AppLauncherModel>,
) {
    // Tear down old buttons. GTK4 doesn't ship a `clear()` so we
    // walk children until first_child is None.
    while let Some(child) = strip.first_child() {
        strip.remove(&child);
    }
    let categories = model.runtime.borrow().categories();
    let active = model.active_category.clone();
    for cat in categories {
        let btn = gtk::Button::new();
        let mut classes: Vec<&str> = vec!["app-launcher-category-pill"];
        if cat.label == active {
            classes.push("selected");
        }
        btn.set_css_classes(&classes);
        btn.set_tooltip_text(Some(&cat.label));

        // Pill content: icon + label in a small horizontal box. The
        // box's spacing handles the icon→label gap; the SCSS adds
        // breathing room on the outer pill.
        let inner = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        inner.add_css_class("app-launcher-category-pill-content");

        let img = gtk::Image::from_icon_name(category_icon(&cat.label));
        img.add_css_class("app-launcher-category-pill-icon");
        inner.append(&img);

        let lbl = gtk::Label::new(Some(&cat.label));
        lbl.add_css_class("app-launcher-category-pill-label");
        // Allow the pill caption to ellipsize so the strip's
        // *minimum* width per pill drops from its full text natural
        // to icon + ellipsis (~50 px). The natural is unchanged so
        // wide launchers still render every full category name in
        // a single row. Tooltip carries the full text for the
        // abbreviated state at very narrow widths. See the matching
        // comment in `rebuild_binds_strip` for the underlying GTK4
        // sizing reason (outer menu ScrolledWindow's hscrollbar
        // policy makes `min/max_content_width` no-ops, so the only
        // way to honour a smaller `minimum_width` is to reduce the
        // child tree's measured minimum).
        lbl.set_ellipsize(pango::EllipsizeMode::End);
        inner.append(&lbl);

        btn.set_child(Some(&inner));
        let sender_clone = sender.clone();
        let label_str = cat.label.clone();
        btn.connect_clicked(move |_| {
            let _ = sender_clone
                .input_sender()
                .send(AppLauncherInput::SelectCategory(label_str.clone()));
        });
        strip.append(&btn);
    }
}

/// Clone helper for the LauncherItem field set.
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

fn clone_display_item(src: &DisplayItem) -> DisplayItem {
    DisplayItem {
        item: clone_launcher_item(&src.item),
        pinned: src.pinned,
        quick_key: src.quick_key.clone(),
        hidden: src.hidden,
    }
}

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

    fn category(&self) -> &str {
        "Apps"
    }
}
