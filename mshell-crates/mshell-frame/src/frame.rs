use crate::bars::bar::{BarInit, BarInput, BarModel, BarOutput, BarType};
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use mshell_common::box_with_resize::BoxWithResize;
use mshell_common::diagonal_revealer::DiagonalRevealer;
use mshell_common::motion::MENU_REVEAL_MS;
use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::*;
use mshell_config::schema::position::Position;
use reactive_graph::traits::*;
use relm4::RelmRemoveAllExt;
use relm4::gtk::{self, Widget, gdk, glib, prelude::*};
use relm4::prelude::*;
use tracing::info;

use crate::frame_draw_widget::FrameDrawWidget;
use crate::frame_spacer::{FrameSpacerInit, FrameSpacerInput, FrameSpacerModel};
use crate::menus::menu::MenuInput::ForwardHyprlandScreenshareReply;
use crate::menus::menu::{MenuInit, MenuInput, MenuModel, MenuOutput, MenuType};
use mshell_config::schema::config::CustomMenuRow;

mod bar_visibility;
/// Self-less controller/content builders + GTK-stack attach/detach helpers.
mod build;
mod effects;
mod hooks;
mod input;
mod keys;
mod layout;
mod menu_id;
mod menu_state;
mod pieces;
mod plugins;
mod strips;
pub use input::*;
use layout::*;
pub use menu_id::*;

/// Read an `i32` out of a `connect_local` signal argument list.
///
/// The `resized` signal always delivers `(widget, width: i32, height: i32)`, so
/// a missing or wrongly-typed slot would be a GTK bug rather than bad input.
/// Twenty call sites each `expect`ed it, which put twenty abort points inside
/// the shell's signal handlers for one read that cannot fail. Falling back to
/// `0` degrades a bar edge to zero thickness — a cosmetic glitch, not a dead
/// session.
fn signal_i32(values: &[glib::Value], index: usize) -> i32 {
    values
        .get(index)
        .and_then(|v| v.get::<i32>().ok())
        .unwrap_or(0)
}

pub struct Frame {
    // Margo's mshell ships only horizontal bars — vertical Left /
    // Right bar surfaces were removed because they conflict with
    // the scroller-default column flow. The `left_menu_*` /
    // `right_menu_*` fields below are unrelated: they control
    // menus that anchor to the screen's left / right edges.
    top_bar: Controller<BarModel>,
    bottom_bar: Controller<BarModel>,
    left_menu_expansion_type: VerticalMenuExpansion,
    right_menu_expansion_type: VerticalMenuExpansion,
    // `*_revealed` flags drive the GtkRevealer that owns the
    // corresponding menu stack — NOT a bar surface. Naming is by
    // anchor edge:
    //   top_revealed / bottom_revealed              — center menus
    //   left_revealed / right_revealed              — side menus
    //   top_left_revealed / top_right_revealed
    //   bottom_left_revealed / bottom_right_revealed — corner menus
    // Vertical Left / Right BAR surfaces were removed; these
    // flags refer to menus that anchor to those screen edges
    // (e.g. the quick-settings menu slides in from the side).
    left_revealed: bool,
    right_revealed: bool,
    top_revealed: bool,
    top_left_revealed: bool,
    top_right_revealed: bool,
    bottom_revealed: bool,
    bottom_left_revealed: bool,
    bottom_right_revealed: bool,
    /// Last menu-position snapshot applied by `RepositionMenus`, so the
    /// effect (which re-fires on every coarse config write) can skip the
    /// destructive restack when nothing moved. `None` until the first
    /// reposition.
    last_menu_positions: Option<Vec<Position>>,
    /// This Frame's monitor, kept so the lazily-built Settings panel
    /// (`ensure_settings_built`) can pass it to `SettingsWindowModel`.
    monitor: gdk::Monitor,
    top_spacer: Controller<FrameSpacerModel>,
    bottom_spacer: Controller<FrameSpacerModel>,
    /// The bar and border strip surfaces around this output's edges. The
    /// root window (the overlay) keeps only the menus; see `pieces`.
    strips: std::rc::Rc<strips::EdgeStrips>,
    /// The pending overlay hide (`sync_overlay`), `None` when none is armed.
    overlay_hide: std::rc::Rc<std::cell::RefCell<Option<glib::SourceId>>>,
    /// Every uniform [`MenuId`] menu, built on first open by
    /// [`ensure_menu_built`](Self::ensure_menu_built) and keyed by its stable
    /// menu-name. Each menu shell is a relm4 `Controller` plus ~3 config
    /// effects, and one `Frame` exists per monitor, so building all 36 eagerly
    /// in `init` attached ~108 GLib sources per monitor to the main loop for
    /// surfaces most sessions never open. A name is absent until first toggled.
    /// Screenshare + the plugin panel below stay eager (bespoke open paths).
    menus: std::collections::HashMap<&'static str, Controller<MenuModel>>,
    screenshare_menu: Controller<MenuModel>,
    /// First-class menu hosting whichever plugin WASM panel is opened.
    plugin_panel_menu: Controller<MenuModel>,
    /// Which screen edge the plugin-panel menu is currently anchored to, so a
    /// per-plugin position change can re-anchor it (move it between stacks).
    plugin_panel_position: Position,
    /// WASM runtime + the live panel per plugin key, kept alive here so a
    /// panel's event loop (and chat state) persists across opens. Only built
    /// with the `wasm-plugins` feature.
    #[cfg(feature = "wasm-plugins")]
    plugin_panel_runtime: mshell_plugin_ui::PluginRuntime,
    #[cfg(feature = "wasm-plugins")]
    plugin_panels: std::collections::HashMap<String, mshell_plugin_ui::PluginPanel>,
    /// Settings panel — uses its own dedicated model (not
    /// `MenuModel`) because its content is a custom sidebar +
    /// stack rather than the generic menu-widget pipeline.
    ///
    /// Built lazily on first open (`ensure_settings_built`): the panel
    /// launches ~48 page controllers, and one Frame exists per monitor, so
    /// building it eagerly cost ~48×N page-tree builds on the GTK main
    /// thread at login for a surface most sessions never open. `None`
    /// until first toggled.
    settings_menu: Option<Controller<mshell_settings::SettingsWindowModel>>,
    /// Pending keyboard-mode switch held inside the 90 ms debounce
    /// window. Replaced on every `sync_keyboard_mode` call; the
    /// timer reads whatever value was last written.
    pending_kbd_mode: std::rc::Rc<std::cell::RefCell<Option<gtk4_layer_shell::KeyboardMode>>>,
    pending_kbd_mode_timeout: std::rc::Rc<std::cell::RefCell<Option<gtk::glib::SourceId>>>,
    _effects: EffectScope,
}

#[relm4::component(pub)]
impl Component for Frame {
    type CommandOutput = ();
    type Input = FrameInput;
    type Output = FrameOutput;
    type Init = FrameInit;

    view! {
        gtk::Window {
            set_css_classes: &["frame-window", "window-opacity"],

            // Fill the window; draw happens in the DrawingArea
            // OkPanel required a hacky fix for fractional scaling.  Might need to do that here at some point.
            #[name = "overlay"]
            gtk::Overlay {
                set_hexpand: true,
                set_vexpand: true,

                add_overlay = &gtk::Box {
                    set_vexpand: true,
                    set_hexpand: true,
                    set_orientation: gtk::Orientation::Vertical,

                    // The bars live on their own strip surfaces (see
                    // `strips`); these placeholders keep the bars' height so
                    // the menus below sit exactly where they did.
                    #[name = "top_bar_placeholder"]
                    gtk::Box {
                        set_hexpand: true,
                        set_can_target: false,
                    },

                    gtk::Box {
                        set_vexpand: true,
                        set_hexpand: true,
                        set_orientation: gtk::Orientation::Horizontal,

                        #[name = "left_bar_and_menu_container"]
                        BoxWithResize::new() -> BoxWithResize {

                            // NOTE: Vertical Left bar surface has been removed
                            // (margo's scroller-default layout claims the
                            // horizontal real estate). Menus that anchor to
                            // the screen's left edge still live inside this
                            // container — they slide in / out via the
                            // `left_revealer` below.

                            #[name = "left_revealer_container"]
                            append = &BoxWithResize::new() -> BoxWithResize {

                                #[name = "left_revealer"]
                                append = &gtk::Revealer {
                                    set_transition_type: gtk::RevealerTransitionType::SlideRight,
                                    set_transition_duration: MENU_REVEAL_MS,
                                    #[watch]
                                    set_reveal_child: model.left_revealed,

                                    gtk::Box {
                                        set_orientation: gtk::Orientation::Vertical,
                                        set_vexpand: true,
                                        set_hexpand: false,

                                        #[name = "top_left_expander"]
                                        BoxWithResize::new() -> BoxWithResize {
                                            set_hexpand: true,
                                            #[watch]
                                            set_vexpand: match model.left_menu_expansion_type {
                                                VerticalMenuExpansion::AlwaysExpanded => {false}
                                                VerticalMenuExpansion::ExpandBothWays => {true}
                                                VerticalMenuExpansion::ExpandDown => {false}
                                                VerticalMenuExpansion::ExpandUp => {true}
                                            },
                                        },

                                        #[name = "left_stack"]
                                        gtk::Stack {
                                            set_transition_type: gtk::StackTransitionType::Crossfade,
                                            set_transition_duration: 200,
                                            set_vhomogeneous: false,
                                            set_hhomogeneous: false,
                                        },

                                        #[name = "bottom_left_expander"]
                                        BoxWithResize::new() -> BoxWithResize {
                                            set_hexpand: true,
                                            #[watch]
                                            set_vexpand: match model.left_menu_expansion_type {
                                                VerticalMenuExpansion::AlwaysExpanded => {false}
                                                VerticalMenuExpansion::ExpandBothWays => {true}
                                                VerticalMenuExpansion::ExpandDown => {true}
                                                VerticalMenuExpansion::ExpandUp => {false}
                                            },
                                        },
                                    },
                                },
                            },
                        },

                        gtk::Box {
                            set_orientation: gtk::Orientation::Horizontal,
                            set_hexpand: true,

                            gtk::Box {
                                set_orientation: gtk::Orientation::Vertical,
                                set_hexpand: false,

                                // This box is required to prevent the top and bottom menus
                                // from matching each other's width if one is larger than the other
                                gtk::Box {

                                    #[name = "top_left_revealer_container"]
                                    BoxWithResize::new() -> BoxWithResize {

                                        #[name = "top_left_revealer"]
                                        append = &DiagonalRevealer::new() {
                                            #[watch]
                                            set_revealed: model.top_left_revealed,

                                            #[wrap(Some)]
                                            #[name = "top_left_stack"]
                                            set_child = &gtk::Stack {
                                                set_transition_type: gtk::StackTransitionType::Crossfade,
                                                set_transition_duration: 200,
                                                set_vhomogeneous: false,
                                                set_hhomogeneous: false,
                                            },
                                        },
                                    },

                                    gtk::Box {
                                        set_hexpand: true,
                                    },
                                },

                                gtk::Box {
                                    set_height_request: 200,
                                    set_vexpand: true,
                                },

                                // This box is required to prevent the top and bottom menus
                                // from matching each other's width if one is larger than the other
                                gtk::Box {

                                    #[name = "bottom_left_revealer_container"]
                                    BoxWithResize::new() -> BoxWithResize {

                                        #[name = "bottom_left_revealer"]
                                        append = &DiagonalRevealer::new() {
                                            #[watch]
                                            set_revealed: model.bottom_left_revealed,

                                            #[wrap(Some)]
                                            #[name = "bottom_left_stack"]
                                            set_child = &gtk::Stack {
                                                set_transition_type: gtk::StackTransitionType::Crossfade,
                                                set_transition_duration: 200,
                                                set_vhomogeneous: false,
                                                set_hhomogeneous: false,
                                            },
                                        },
                                    },

                                    gtk::Box {
                                        set_hexpand: true,
                                    },
                                },
                            },

                            gtk::Box {
                                set_hexpand: true,
                            },

                            gtk::Box {
                                set_orientation: gtk::Orientation::Vertical,
                                set_hexpand: false,

                                // This box is required to prevent the top and bottom menus
                                // from matching each other's width if one is larger than the other
                                gtk::Box {

                                    gtk::Box {
                                        set_hexpand: true,
                                    },

                                    #[name = "top_revealer_container"]
                                    BoxWithResize::new() -> BoxWithResize {

                                        #[name = "top_revealer"]
                                        append = &DiagonalRevealer::new() {
                                            #[watch]
                                            set_revealed: model.top_revealed,

                                            #[wrap(Some)]
                                            #[name = "top_stack"]
                                            set_child = &gtk::Stack {
                                                set_transition_type: gtk::StackTransitionType::Crossfade,
                                                set_transition_duration: 200,
                                                set_vhomogeneous: false,
                                                set_hhomogeneous: false,
                                            },
                                        },
                                    },

                                    gtk::Box {
                                        set_hexpand: true,
                                    },
                                },

                                gtk::Box {
                                    set_height_request: 200,
                                    set_vexpand: true,
                                },

                                // This box is required to prevent the top and bottom menus
                                // from matching each other's width if one is larger than the other
                                gtk::Box {

                                    gtk::Box {
                                        set_hexpand: true,
                                    },

                                    #[name = "bottom_revealer_container"]
                                    BoxWithResize::new() -> BoxWithResize {

                                        #[name = "bottom_revealer"]
                                        append = &DiagonalRevealer::new() {
                                            #[watch]
                                            set_revealed: model.bottom_revealed,

                                            #[wrap(Some)]
                                            #[name = "bottom_stack"]
                                            set_child = &gtk::Stack {
                                                set_transition_type: gtk::StackTransitionType::Crossfade,
                                                set_transition_duration: 200,
                                                set_vhomogeneous: false,
                                                set_hhomogeneous: false,
                                            },
                                        },
                                    },

                                    gtk::Box {
                                        set_hexpand: true,
                                    },
                                },
                            },

                            gtk::Box {
                                set_hexpand: true,
                            },

                            gtk::Box {
                                set_orientation: gtk::Orientation::Vertical,
                                set_hexpand: false,

                                // This box is required to prevent the top and bottom menus
                                // from matching each other's width if one is larger than the other
                                gtk::Box {

                                    gtk::Box {
                                        set_hexpand: true,
                                    },

                                    #[name = "top_right_revealer_container"]
                                    BoxWithResize::new() -> BoxWithResize {

                                        #[name = "top_right_revealer"]
                                        append = &DiagonalRevealer::new() {
                                            #[watch]
                                            set_revealed: model.top_right_revealed,

                                            #[wrap(Some)]
                                            #[name = "top_right_stack"]
                                            set_child = &gtk::Stack {
                                                set_transition_type: gtk::StackTransitionType::Crossfade,
                                                set_transition_duration: 200,
                                                set_vhomogeneous: false,
                                                set_hhomogeneous: false,
                                            },
                                        },
                                    },
                                },

                                gtk::Box {
                                    set_height_request: 200,
                                    set_vexpand: true,
                                },

                                // This box is required to prevent the top and bottom menus
                                // from matching each other's width if one is larger than the other
                                gtk::Box {

                                    gtk::Box {
                                        set_hexpand: true,
                                    },

                                    #[name = "bottom_right_revealer_container"]
                                    BoxWithResize::new() -> BoxWithResize {

                                        #[name = "bottom_right_revealer"]
                                        append = &DiagonalRevealer::new() {
                                            #[watch]
                                            set_revealed: model.bottom_right_revealed,

                                            #[wrap(Some)]
                                            #[name = "bottom_right_stack"]
                                            set_child = &gtk::Stack {
                                                set_transition_type: gtk::StackTransitionType::Crossfade,
                                                set_transition_duration: 200,
                                                set_vhomogeneous: false,
                                                set_hhomogeneous: false,
                                            },
                                        },
                                    },
                                },
                            },
                        },

                        #[name = "right_bar_and_menu_container"]
                        BoxWithResize::new() -> BoxWithResize {

                            #[name = "right_revealer_container"]
                            append = &BoxWithResize::new() -> BoxWithResize {

                                #[name = "right_revealer"]
                                append = &gtk::Revealer {
                                    set_transition_type: gtk::RevealerTransitionType::SlideLeft,
                                    set_transition_duration: MENU_REVEAL_MS,
                                    #[watch]
                                    set_reveal_child: model.right_revealed,

                                    gtk::Box {
                                        set_orientation: gtk::Orientation::Vertical,
                                        set_vexpand: true,
                                        set_hexpand: false,

                                        #[name = "top_right_expander"]
                                        BoxWithResize::new() -> BoxWithResize {
                                            set_hexpand: true,
                                            #[watch]
                                            set_vexpand: match model.right_menu_expansion_type {
                                                VerticalMenuExpansion::AlwaysExpanded => {false}
                                                VerticalMenuExpansion::ExpandBothWays => {true}
                                                VerticalMenuExpansion::ExpandDown => {false}
                                                VerticalMenuExpansion::ExpandUp => {true}
                                            },
                                        },

                                        #[name = "right_stack"]
                                        gtk::Stack {
                                            set_transition_type: gtk::StackTransitionType::Crossfade,
                                            set_transition_duration: 200,
                                            set_vhomogeneous: false,
                                            set_hhomogeneous: false,
                                        },

                                        #[name = "bottom_right_expander"]
                                        BoxWithResize::new() -> BoxWithResize {
                                            set_hexpand: true,
                                            #[watch]
                                            set_vexpand: match model.right_menu_expansion_type {
                                                VerticalMenuExpansion::AlwaysExpanded => {false}
                                                VerticalMenuExpansion::ExpandBothWays => {true}
                                                VerticalMenuExpansion::ExpandDown => {true}
                                                VerticalMenuExpansion::ExpandUp => {false}
                                            },
                                        },
                                    },
                                },
                            },

                            // NOTE: Vertical Right bar surface has been
                            // removed (see comment on left_bar_and_menu_container).
                            // Menus that anchor to the right edge live above
                            // in the `right_revealer`.
                        },
                    },

                    #[name = "bottom_bar_placeholder"]
                    gtk::Box {
                        set_hexpand: true,
                        set_can_target: false,
                    },
                },

                #[local_ref]
                frame_draw_widget -> FrameDrawWidget {
                    set_hexpand: true,
                    set_vexpand: true,
                }
            }
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        info!(
            monitor = params
                .monitor
                .connector()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "<unknown>".to_string()),
            "Initializing frame"
        );

        root.init_layer_shell();
        root.set_monitor(Some(&params.monitor));
        root.set_namespace(Some("mshell-frame"));
        root.set_layer(Layer::Top);
        root.set_exclusive_zone(-1);
        root.set_anchor(Edge::Top, true);
        root.set_anchor(Edge::Bottom, true);
        root.set_anchor(Edge::Left, true);
        root.set_anchor(Edge::Right, true);
        root.set_decorated(false);
        // Start with `None` keyboard interactivity (we don't want to
        // steal keys from the user's toplevels while no menu is open).
        // Each `ToggleXxxMenu` / `CloseMenus` handler calls
        // `sync_keyboard_mode(root)` to switch to `Exclusive` while
        // any menu is revealed and back to `None` when they all
        // close. `Exclusive` is required because margo's
        // `compute_desired_focus` only honours layer-surfaces with
        // exclusive interactivity — anything else gets focus stolen
        // by the active toplevel and ESC never reaches us.
        root.set_keyboard_mode(gtk4_layer_shell::KeyboardMode::None);
        // The root window is the menu overlay: it stays unmapped (and
        // unrealized, holding no buffers) until a menu opens. The bars and
        // the frame edges live on the strip surfaces (`strips`).
        root.set_cursor_from_name(Some("default"));

        keys::install_key_handlers(&root, &sender);

        let base_config = config_manager().config();
        let untracked_config = base_config.clone().get_untracked();

        let left_menu_expansion_type = untracked_config.menus.left_menu_expansion_type;
        let right_menu_expansion_type = untracked_config.menus.right_menu_expansion_type;

        let top_bar: Controller<BarModel> = Self::build_bar(&sender, BarType::Top);
        let bottom_bar: Controller<BarModel> = Self::build_bar(&sender, BarType::Bottom);

        // The 36 uniform [`MenuId`] menus are no longer built here: each is
        // deferred to first open via [`ensure_menu_built`], keyed into the
        // `menus` map. Only the two bespoke surfaces stay eager — screenshare
        // (answers a portal reply channel) and the plugin panel (owns re-anchor
        // position state consulted even with the WASM feature off).
        let screenshare_menu = Self::build_menu(&sender, MenuType::HyprlandScreenshare);
        let plugin_panel_menu = Self::build_menu(&sender, MenuType::PluginPanel);

        // Settings doesn't go through `build_menu` because its content
        // isn't a list of `MenuWidget`s — it's a custom sidebar + stack
        // laid out by `SettingsWindowModel`. It is also NOT built here:
        // the panel's ~48 page controllers are deferred to first open via
        // `ensure_settings_built` (see the field doc). The shell-level
        // dispatcher emits `ToggleSettingsMenu` to the right Frame, which
        // builds + attaches the panel on demand.

        let mut effects = EffectScope::new();

        hooks::push_background_hooks(&mut effects);

        effects::push_frame_effects(&mut effects, &base_config, &sender);

        let monitor_clone = params.monitor.clone();
        let top_spacer = FrameSpacerModel::builder()
            .launch(FrameSpacerInit {
                bar_type: BarType::Top,
                monitor: monitor_clone,
            })
            .detach();
        let monitor_clone = params.monitor.clone();
        let bottom_spacer = FrameSpacerModel::builder()
            .launch(FrameSpacerInit {
                bar_type: BarType::Bottom,
                monitor: monitor_clone,
            })
            .detach();
        let top_sender = top_spacer.sender().clone();
        let bottom_sender = bottom_spacer.sender().clone();
        effects.push(move |_| {
            let border_width = config_manager()
                .config()
                .theme()
                .attributes()
                .sizing()
                .border_width()
                .get();
            top_sender.emit(FrameSpacerInput::BorderHeightUpdated(border_width));
            bottom_sender.emit(FrameSpacerInput::BorderHeightUpdated(border_width));
        });

        let frame_draw_widget = FrameDrawWidget::new();
        let top_bar_widget: Widget = top_bar.widget().clone().upcast();
        let bottom_bar_widget: Widget = bottom_bar.widget().clone().upcast();
        let strips = strips::EdgeStrips::new(
            &params.monitor,
            &top_bar_widget,
            &bottom_bar_widget,
            &frame_draw_widget,
        );

        let model = Frame {
            strips,
            overlay_hide: std::rc::Rc::new(std::cell::RefCell::new(None)),
            top_bar,
            bottom_bar,
            left_menu_expansion_type,
            right_menu_expansion_type,
            top_revealed: false,
            left_revealed: false,
            right_revealed: false,
            top_left_revealed: false,
            top_right_revealed: false,
            bottom_revealed: false,
            bottom_left_revealed: false,
            bottom_right_revealed: false,
            last_menu_positions: None,
            monitor: params.monitor.clone(),
            top_spacer,
            bottom_spacer,
            menus: std::collections::HashMap::new(),
            screenshare_menu,
            plugin_panel_menu,
            plugin_panel_position: mshell_config::config_manager::config_manager()
                .config()
                .menus()
                .plugin_panel_menu()
                .position()
                .get_untracked(),
            #[cfg(feature = "wasm-plugins")]
            plugin_panel_runtime: mshell_plugin_host::PluginRuntime::with_providers(
                std::sync::Arc::new(crate::plugin_providers::WayleMediaProvider),
                std::sync::Arc::new(crate::plugin_providers::WayleSystemProvider),
            )
            .expect("plugin panel wasm runtime"),
            #[cfg(feature = "wasm-plugins")]
            plugin_panels: std::collections::HashMap::new(),
            settings_menu: None,
            pending_kbd_mode: std::rc::Rc::new(std::cell::RefCell::new(None)),
            pending_kbd_mode_timeout: std::rc::Rc::new(std::cell::RefCell::new(None)),
            _effects: effects,
        };

        let widgets = view_output!();

        strips::mirror_height(&model.strips.top_bar_box, &widgets.top_bar_placeholder);
        strips::mirror_height(
            &model.strips.bottom_bar_box,
            &widgets.bottom_bar_placeholder,
        );
        model.attach_resize_listeners(&widgets);

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
            FrameInput::SetDrawFrame(draw_frame) => {
                self.set_draw_frame(widgets, draw_frame);
            }
            FrameInput::QueueFrameRedraw => {
                widgets.frame_draw_widget.queue_draw();
                self.strips.queue_draw();
            }
            FrameInput::SetLeftMenuExpansionType(expansion_type) => {
                self.left_menu_expansion_type = expansion_type;
            }
            FrameInput::SetRightMenuExpansionType(expansion_type) => {
                self.right_menu_expansion_type = expansion_type;
            }
            FrameInput::RepositionMenus(positions) => {
                // Skip the destructive restack (CloseMenus +
                // remove_all-and-rebuild every stack) when no menu actually
                // moved. The position effect re-fires on every config write
                // because the store is coarse, so without this guard
                // editing any unrelated setting tore down the open menu and
                // re-stacked ~33 menus per monitor (the "bar flicker").
                if self.last_menu_positions.as_deref() != Some(positions.as_slice()) {
                    self.last_menu_positions = Some(positions);
                    sender.input(FrameInput::CloseMenus);
                    self.apply_left_and_right_side_children(widgets);
                }
            }
            FrameInput::ToggleMenu(id) => {
                self.ensure_menu_built(&sender, id, widgets);
                self.toggle_menu(id.menu_name(), widgets);
                self.sync_keyboard_mode(root);
            }
            FrameInput::SwitchToMenu(id) => {
                // Queued in order: the close lands before the toggle.
                sender.input(FrameInput::CloseMenus);
                sender.input(FrameInput::ToggleMenu(id));
            }
            FrameInput::SyncKeyboardMode => {
                self.sync_keyboard_mode(root);
            }
            FrameInput::HiddenBar(verb, target) => {
                self.top_bar
                    .sender()
                    .emit(BarInput::HiddenBar(verb, target.clone()));
                self.bottom_bar
                    .sender()
                    .emit(BarInput::HiddenBar(verb, target));
            }
            FrameInput::SpacerReserve { is_top, height } => {
                self.spacer_reserve(widgets, is_top, height);
            }
            FrameInput::ToggleAppLauncherMenuWithTab(tab) => {
                // If the launcher is already visible AND already
                // on the requested tab, treat the call as a
                // toggle (close it). Otherwise: open if hidden,
                // then forward SelectCategory so the AppLauncher
                // widget swaps tabs. The launcher state lives
                // inside its widget controller, so we just send
                // the message and let the runtime handle the
                // "unknown tab" fall back to "All".
                self.open_menu_then(&sender, MenuId::AppLauncher, widgets, root, |_| {
                    MenuInput::AppLauncherSelectCategory(tab)
                });
            }
            FrameInput::ToggleMdashMenuWithTab(tab) => {
                // mdash decides: switch tab, or close on the same tab.
                self.open_menu_then(&sender, MenuId::Mdash, widgets, root, |was_open| {
                    MenuInput::MdashSelectTab { tab, was_open }
                });
            }
            FrameInput::ToggleWasmPluginPanel {
                name,
                entry,
                settings,
                capabilities,
                min_width: _,
                max_height: _,
            } => {
                self.toggle_wasm_plugin_panel(name, entry, settings, capabilities, widgets, root);
            }
            FrameInput::TogglePluginMenu {
                name,
                rows,
                min_width: _,
                max_height: _,
            } => {
                self.toggle_plugin_menu(name, rows, widgets, &sender, root);
            }
            FrameInput::TogglePluginByKey(key) => {
                Self::toggle_plugin_by_key(key, &sender);
            }
            FrameInput::ReloadPlugin(key) => {
                self.reload_plugin(key);
            }
            FrameInput::FirePluginKeybind(key, bind_id) => {
                self.fire_plugin_keybind(key, bind_id, widgets, root);
            }
            FrameInput::ToggleSettingsMenu => {
                self.ensure_settings_built(widgets);
                self.toggle_menu(SETTINGS_MENU, widgets);
                self.sync_keyboard_mode(root);
            }
            FrameInput::OpenSettingsAtSection(section) => {
                // Build the panel on demand, then ensure Settings is
                // visible — toggle if currently hidden. Skip the toggle
                // when already visible so re-issuing the same section nav
                // doesn't close the panel.
                self.ensure_settings_built(widgets);
                if !self.is_menu_visible_now(SETTINGS_MENU, widgets) {
                    self.toggle_menu(SETTINGS_MENU, widgets);
                    self.sync_keyboard_mode(root);
                }
                if let Some(controller) = self.settings_menu.as_ref() {
                    let _ = controller.sender().send(
                        mshell_settings::SettingsWindowInput::ActivateSection(section),
                    );
                }
            }
            FrameInput::CloseSettingsMenu => {
                // Idempotent close: no-op if Settings isn't currently
                // visible on this frame, otherwise tear it down. Used
                // by the Shell router to prevent multi-monitor ghosts.
                if self.is_menu_visible_now(SETTINGS_MENU, widgets) {
                    self.toggle_menu(SETTINGS_MENU, widgets);
                    self.sync_keyboard_mode(root);
                }
            }
            FrameInput::ToggleScreenshareMenu(reply, payload) => {
                // The chooser queues the request and asks for ShowScreenshareMenu
                // once it is on screen; a toggle here let a second request
                // close (or hijack) the chooser the user was answering.
                self.screenshare_menu
                    .emit(ForwardHyprlandScreenshareReply(reply, payload));
            }
            FrameInput::ShowScreenshareMenu => {
                if !self.is_menu_visible_now(SCREENSHARE_MENU, widgets) {
                    self.toggle_menu(SCREENSHARE_MENU, widgets);
                }
                self.sync_keyboard_mode(root);
            }
            FrameInput::ClipboardExitSearch => {
                // Forward to this frame's own clipboard menu. The
                // keyboard-focused surface is the one that received
                // Esc, so its clipboard is the one in search mode. Search
                // mode can only be active once the clipboard was opened, so
                // the menu is always built here.
                if let Some(menu) = self.menus.get(CLIPBOARD_MENU) {
                    menu.sender()
                        .send(MenuInput::ClipboardExitSearch)
                        .unwrap_or_default();
                }
            }
            FrameInput::CloseMenus => {
                self.close_menus(root);
            }
            FrameInput::BarToggleTop => {
                self.top_bar.sender().emit(BarInput::ToggleRevealed);
            }
            FrameInput::BarToggleBottom => {
                self.bottom_bar.sender().emit(BarInput::ToggleRevealed);
            }
            FrameInput::BarToggleAll(exclude_hidden_by_default) => {
                self.bars_toggle_all(exclude_hidden_by_default);
            }
            FrameInput::BarRevealAll(exclude_hidden_by_default) => {
                self.bars_reveal_all(exclude_hidden_by_default);
            }
            FrameInput::BarHideAll(exclude_hidden_by_default) => {
                self.bars_hide_all(exclude_hidden_by_default);
            }
        }
        self.sync_overlay(root);
        self.update_view(widgets, sender);
    }
}

impl Drop for Frame {
    fn drop(&mut self) {
        self.top_spacer.widget().destroy();
        self.bottom_spacer.widget().destroy();
        self.strips.destroy();
    }
}
