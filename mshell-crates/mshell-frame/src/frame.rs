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

/// Self-less controller/content builders + GTK-stack attach/detach helpers.
mod build;
mod hooks;
mod input;
mod layout;
mod menu_id;
mod menu_state;
mod pieces;
mod plugins;
mod strips;
pub use input::*;
use layout::*;
pub use menu_id::*;

const CLOCK_MENU: &str = "clock";
const CLIPBOARD_MENU: &str = "clipboard";
const APP_LAUNCHER_MENU: &str = "app_launcher";
const SCREENSHOT_MENU: &str = "screenshot";
const NOTIFICATION_MENU: &str = "notification";
const WALLPAPER_MENU: &str = "wallpaper";
const SCREENSHARE_MENU: &str = "screenshare";
const WIZARD_MENU: &str = "wizard";
const NUFW_MENU: &str = "ufw";
const PRIVACY_MENU: &str = "privacy";
const BLUETOOTH_MENU: &str = "bluetooth";
const CPU_DASHBOARD_MENU: &str = "cpu_dashboard";
const AUDIO_DASHBOARD_MENU: &str = "audio_dashboard";
const SYSTEM_UPDATE_MENU: &str = "system_update";
const VALENT_MENU: &str = "valent";
const WEATHER_MENU: &str = "weather";
const KEEP_AWAKE_MENU: &str = "keep_awake";
const TWILIGHT_MENU: &str = "twilight";
const KEYBINDS_MENU: &str = "keybinds";
const ALARMCLOCK_MENU: &str = "alarmclock";
const DOCK_MENU: &str = "dock";
const CONTROL_CENTER_MENU: &str = "control_center";
const SSH_MENU: &str = "ssh_sessions";
const USB_MENU: &str = "usb";
const DISCORD_MENU: &str = "discord_voice";
const CRASHES_MENU: &str = "crashes";
const NDNS_MENU: &str = "dns";
const NVPN_MENU: &str = "vpn";
const NAI_MENU: &str = "ai";
const NPODMAN_MENU: &str = "podman";
const NAGENTS_MENU: &str = "agents";
const NNOTES_MENU: &str = "notes";
const NPLUGIN_PANEL_MENU: &str = "plugin-panel";
const NIP_MENU: &str = "ip";
const NVPN_INDICATOR_MENU: &str = "vpn_indicator";
const NNETWORK_MENU: &str = "network";
const NPOWER_MENU: &str = "power";
const MEDIA_PLAYER_MENU: &str = "media_player";
const MTUNE_MENU: &str = "mtune";
const LYRICS_MENU: &str = "lyrics";
const SESSION_MENU: &str = "session";
const SETTINGS_MENU: &str = "settings";

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
const MDASH_MENU: &str = "mdash";
const MARGO_LAYOUT_MENU: &str = "margo_layout";
const TRANSLATE_MENU: &str = "translate";

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

        // Re-assert keyboard interactivity once the surface actually
        // maps. The `set_keyboard_mode(None)` above is queued before
        // the layer surface exists; in practice the frame has been
        // observed committing `Exclusive` at login anyway, which traps
        // keyboard focus into this invisible full-screen layer (margo's
        // `compute_desired_focus` honours any Exclusive Top/Overlay
        // layer) until the user's first menu toggle finally runs
        // `sync_keyboard_mode`. Firing it here closes that gap: at first
        // map no menu is revealed, so it resolves to `None`.
        {
            let sender_map = sender.clone();
            root.connect_map(move |w| {
                tracing::debug!(
                    mode = ?w.keyboard_mode(),
                    "frame: surface mapped — re-asserting keyboard mode"
                );
                sender_map.input(FrameInput::SyncKeyboardMode);
            });
        }

        // ESC closes any open menu. Belt-and-suspenders setup:
        //
        // 1) `ShortcutController(scope=Global)` with a `KeyvalTrigger`
        //    on Escape — this is the global-shortcut path that fires
        //    as soon as the layer surface receives the keypress,
        //    regardless of which child widget has internal focus.
        //
        // 2) An `EventControllerKey` in `Capture` phase as a fallback,
        //    plus a debug log so we can see whether keys are reaching
        //    the surface at all. (Layer-shell + OnDemand keyboard
        //    interactivity sometimes never delivers keys until the
        //    compositor decides to grant focus.)
        let sender_esc = sender.clone();
        let shortcut = gtk::Shortcut::builder()
            .trigger(&gtk::KeyvalTrigger::new(
                gdk::Key::Escape,
                gdk::ModifierType::empty(),
            ))
            .action(&gtk::CallbackAction::new(move |_, _| {
                tracing::debug!("frame: ESC shortcut fired");
                // If the clipboard `/` filter is open, Esc leaves search
                // mode (vim semantics) instead of closing the menu.
                if crate::menus::menu_widgets::clipboard::clipboard::search_is_active() {
                    sender_esc.input(FrameInput::ClipboardExitSearch);
                } else {
                    sender_esc.input(FrameInput::CloseMenus);
                }
                gtk::glib::Propagation::Stop
            }))
            .build();
        let shortcut_ctrl = gtk::ShortcutController::new();
        shortcut_ctrl.set_scope(gtk::ShortcutScope::Global);
        shortcut_ctrl.add_shortcut(shortcut);
        root.add_controller(shortcut_ctrl);

        let key_ctrl = gtk::EventControllerKey::new();
        key_ctrl.set_propagation_phase(gtk::PropagationPhase::Capture);
        let sender_esc2 = sender.clone();
        key_ctrl.connect_key_pressed(move |_, keyval, _, _| {
            tracing::debug!(?keyval, "frame: key_pressed");
            if keyval == gdk::Key::Escape {
                if crate::menus::menu_widgets::clipboard::clipboard::search_is_active() {
                    sender_esc2.input(FrameInput::ClipboardExitSearch);
                } else {
                    sender_esc2.input(FrameInput::CloseMenus);
                }
                gtk::glib::Propagation::Stop
            } else {
                gtk::glib::Propagation::Proceed
            }
        });
        root.add_controller(key_ctrl);

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

        let config = base_config.clone();
        let sender_clone = sender.clone();
        effects.push(move |_| {
            let config = config.clone();
            let enable_frame = config.clone().bars().frame().enable_frame().get();
            // Subscribe to the inset too, so editing `frameless_gap` re-fires
            // this effect → `SetDrawFrame` re-reads the fresh gap and re-pushes
            // the inset to the bars (live slider, no shell restart).
            let _ = config.bars().frame().frameless_gap().get();
            sender_clone.input(FrameInput::SetDrawFrame(enable_frame));
        });

        let config = base_config.clone();
        let sender_clone = sender.clone();
        effects.push(move |_| {
            let config = config.clone();
            let expansion_type = config.menus().left_menu_expansion_type().get();
            sender_clone.input(FrameInput::SetLeftMenuExpansionType(expansion_type));
        });

        let config = base_config.clone();
        let sender_clone = sender.clone();
        effects.push(move |_| {
            let config = config.clone();
            let expansion_type = config.menus().right_menu_expansion_type().get();
            sender_clone.input(FrameInput::SetRightMenuExpansionType(expansion_type));
        });

        let menu_config = base_config.clone();
        let sender_clone = sender.clone();
        effects.push(move |_| {
            // Each StoreField accessor (`.menus()`) consumes its receiver, so
            // every position read needs a fresh clone of the config handle.
            // `pos!` packages that clone-then-read so the per-menu lines stay
            // one-liners. Reading a menu's position also subscribes this effect
            // to that menu — which is why the side-placed menus below are read
            // into `_` (purely to re-fire when their position changes).
            // Collect every menu's position into one ordered Vec. The
            // `.get()` both reads the value AND subscribes this effect to
            // that menu (so a position change re-fires it), and the
            // collected snapshot lets `RepositionMenus` skip the
            // destructive restack when nothing moved (the config store is
            // coarse: any write re-fires this effect).
            let mut positions: Vec<Position> = Vec::new();
            macro_rules! pos {
                ($menu:ident) => {
                    positions.push(menu_config.clone().menus().$menu().position().get())
                };
            }
            pos!(clock_menu);
            pos!(clipboard_menu);
            pos!(notification_menu);
            pos!(screenshot_menu);
            pos!(app_launcher_menu);
            pos!(wallpaper_menu);
            pos!(screenshare_menu);
            pos!(ufw_menu);
            pos!(dns_menu);
            pos!(podman_menu);
            pos!(agents_menu);
            pos!(notes_menu);
            pos!(ip_menu);
            pos!(vpn_indicator_menu);
            pos!(network_menu);
            pos!(power_menu);
            pos!(media_player_menu);
            pos!(mtune_menu);
            pos!(lyrics_menu);
            pos!(session_menu);
            pos!(settings_menu);
            pos!(cpu_dashboard_menu);
            pos!(audio_dashboard_menu);
            pos!(mdash_menu);
            pos!(bluetooth_menu);
            pos!(system_update_menu);
            pos!(valent_menu);
            pos!(weather_menu);
            pos!(keep_awake_menu);
            pos!(twilight_menu);
            pos!(keybinds_menu);
            pos!(alarmclock_menu);
            pos!(dock_menu);
            pos!(control_center_menu);
            pos!(ssh_menu);
            pos!(usb_menu);
            pos!(discord_voice_menu);
            pos!(crashes_menu);
            pos!(vpn_menu);
            pos!(ai_menu);
            pos!(margo_layout_menu);
            sender_clone.input(FrameInput::RepositionMenus(positions));
        });

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
                widgets
                    .frame_draw_widget
                    .update_style(|s| s.draw_frame = draw_frame);
                // The frame_draw_widget is the bars' + menus' only
                // background (they sit transparent on top of it). With
                // it off they'd vanish, so flip `.frame-disabled` on the
                // shared overlay — SCSS then gives each bar/menu its own
                // opaque standalone surface (see `_frame_fallback.scss`).
                if draw_frame {
                    widgets.overlay.remove_css_class("frame-disabled");
                } else {
                    widgets.overlay.add_css_class("frame-disabled");
                }
                self.strips.set_draw_frame(draw_frame);
                // Toggling `.frame-disabled` changes each bar's natural height
                // (the frameless bar adds an inset floating panel). Push the
                // inset (margin = `frameless_gap`, or 0 when the frame is back
                // on) to both bars now: that re-measures them and refreshes the
                // layer-shell exclusive zone immediately, so windows follow the
                // toggle instead of staying tiled under the old reserve. (relm4
                // queues these, so they run after the CSS class is applied.)
                let gap = config_manager()
                    .config()
                    .bars()
                    .frame()
                    .frameless_gap()
                    .get_untracked();
                let disabled = !draw_frame;
                self.top_bar
                    .sender()
                    .emit(BarInput::SetFrameInset { disabled, gap });
                self.bottom_bar
                    .sender()
                    .emit(BarInput::SetFrameInset { disabled, gap });
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
                let spacer = if is_top {
                    &self.top_spacer
                } else {
                    &self.bottom_spacer
                };
                let _ = spacer
                    .sender()
                    .send(FrameSpacerInput::HeightUpdated(height));
                // Lock the drawn frame band to the SAME height as the layer-
                // shell exclusive zone. The band (`top/bottom_thickness`) is
                // otherwise driven off the bar container's *allocated* height
                // (the `resized` listeners below), while the exclusive zone is
                // the bar's *reserved* height (`BarOutput::ReserveHeight`,
                // measured off `bar_center`). When the two differ the frame —
                // which paints on the TOP layer, above tiled windows — bleeds
                // past `work_area` by that difference, painting over the
                // window's top/bottom border. With gaps that overhang hides in
                // the gap; with `smartgaps` (gap → 0) the lone window sits
                // flush against `work_area` and the overhang eats its top and
                // bottom border. Driving the band from the reserve guarantees
                // hole edge == work_area edge, so the border always shows.
                if is_top {
                    widgets
                        .frame_draw_widget
                        .update_style(|s| s.top_thickness = height as f64);
                } else {
                    widgets
                        .frame_draw_widget
                        .update_style(|s| s.bottom_thickness = height as f64);
                }
                self.strips.set_band(is_top, f64::from(height));
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
                self.ensure_menu_built(&sender, MenuId::AppLauncher, widgets);
                if !self.is_menu_visible_now(APP_LAUNCHER_MENU, widgets) {
                    self.toggle_menu(APP_LAUNCHER_MENU, widgets);
                }
                if let Some(menu) = self.menus.get(APP_LAUNCHER_MENU) {
                    menu.sender()
                        .send(MenuInput::AppLauncherSelectCategory(tab))
                        .ok();
                }
                self.sync_keyboard_mode(root);
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
                self.screenshare_menu
                    .emit(ForwardHyprlandScreenshareReply(reply, payload));
                self.toggle_menu(SCREENSHARE_MENU, widgets);
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
                self.left_revealed = false;
                self.right_revealed = false;
                self.top_revealed = false;
                self.top_left_revealed = false;
                self.top_right_revealed = false;
                self.bottom_revealed = false;
                self.bottom_left_revealed = false;
                self.bottom_right_revealed = false;

                // Idle every menu that was ever opened (built). The list is
                // the same one the eager fields drove before — clock/clipboard/
                // notification/screenshot/app-launcher/wallpaper plus the
                // lazy-poll menus (ip/vpn-indicator/dns/vpn/ai/ufw/podman/
                // network) that must stop their refresh I/O on close. An
                // unbuilt (never-opened) menu was never revealed, so skipping
                // it is a no-op.
                for name in [
                    CLOCK_MENU,
                    CLIPBOARD_MENU,
                    NOTIFICATION_MENU,
                    SCREENSHOT_MENU,
                    APP_LAUNCHER_MENU,
                    WALLPAPER_MENU,
                    NIP_MENU,
                    NVPN_INDICATOR_MENU,
                    NDNS_MENU,
                    NVPN_MENU,
                    NAI_MENU,
                    NUFW_MENU,
                    NPODMAN_MENU,
                    NAGENTS_MENU,
                    NNETWORK_MENU,
                ] {
                    if let Some(menu) = self.menus.get(name) {
                        menu.sender()
                            .send(MenuInput::RevealChanged(false))
                            .unwrap_or_default();
                    }
                }

                // Screenshare stays eager (bespoke portal path).
                self.screenshare_menu
                    .sender()
                    .send(MenuInput::RevealChanged(false))
                    .unwrap_or_default();

                self.sync_keyboard_mode(root);
            }
            FrameInput::BarToggleTop => {
                self.top_bar.sender().emit(BarInput::ToggleRevealed);
            }
            FrameInput::BarToggleBottom => {
                self.bottom_bar.sender().emit(BarInput::ToggleRevealed);
            }
            FrameInput::BarToggleAll(exclude_hidden_by_default) => {
                if exclude_hidden_by_default {
                    if config_manager()
                        .config()
                        .bars()
                        .top_bar()
                        .reveal_by_default()
                        .get_untracked()
                    {
                        self.top_bar.sender().emit(BarInput::ToggleRevealed);
                    }
                    if config_manager()
                        .config()
                        .bars()
                        .bottom_bar()
                        .reveal_by_default()
                        .get_untracked()
                    {
                        self.bottom_bar.sender().emit(BarInput::ToggleRevealed);
                    }
                } else {
                    self.top_bar.sender().emit(BarInput::ToggleRevealed);
                    self.bottom_bar.sender().emit(BarInput::ToggleRevealed);
                }
            }
            FrameInput::BarRevealAll(exclude_hidden_by_default) => {
                if exclude_hidden_by_default {
                    if config_manager()
                        .config()
                        .bars()
                        .top_bar()
                        .reveal_by_default()
                        .get_untracked()
                    {
                        self.top_bar.sender().emit(BarInput::SetRevealed(true));
                    }
                    if config_manager()
                        .config()
                        .bars()
                        .bottom_bar()
                        .reveal_by_default()
                        .get_untracked()
                    {
                        self.bottom_bar.sender().emit(BarInput::SetRevealed(true));
                    }
                } else {
                    self.top_bar.sender().emit(BarInput::SetRevealed(true));
                    self.bottom_bar.sender().emit(BarInput::SetRevealed(true));
                }
            }
            FrameInput::BarHideAll(exclude_hidden_by_default) => {
                if exclude_hidden_by_default {
                    if config_manager()
                        .config()
                        .bars()
                        .top_bar()
                        .reveal_by_default()
                        .get_untracked()
                    {
                        self.top_bar.sender().emit(BarInput::SetRevealed(false));
                    }
                    if config_manager()
                        .config()
                        .bars()
                        .bottom_bar()
                        .reveal_by_default()
                        .get_untracked()
                    {
                        self.bottom_bar.sender().emit(BarInput::SetRevealed(false));
                    }
                } else {
                    self.top_bar.sender().emit(BarInput::SetRevealed(false));
                    self.bottom_bar.sender().emit(BarInput::SetRevealed(false));
                }
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
