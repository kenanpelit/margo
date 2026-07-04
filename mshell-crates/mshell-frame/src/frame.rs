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
const NDNS_MENU: &str = "dns";
const NVPN_MENU: &str = "vpn";
const NAI_MENU: &str = "ai";
const NPODMAN_MENU: &str = "podman";
const NNOTES_MENU: &str = "notes";
const NPLUGIN_PANEL_MENU: &str = "plugin-panel";
const NIP_MENU: &str = "ip";
const NVPN_INDICATOR_MENU: &str = "vpn_indicator";
const NNETWORK_MENU: &str = "network";
const NPOWER_MENU: &str = "power";
const MEDIA_PLAYER_MENU: &str = "media_player";
const LYRICS_MENU: &str = "lyrics";
const SESSION_MENU: &str = "session";
const SETTINGS_MENU: &str = "settings";
const MDASH_MENU: &str = "mdash";
const MARGO_LAYOUT_MENU: &str = "margo_layout";

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
    clock_menu: Controller<MenuModel>,
    clipboard_menu: Controller<MenuModel>,
    notification_menu: Controller<MenuModel>,
    screenshot_menu: Controller<MenuModel>,
    app_launcher_menu: Controller<MenuModel>,
    wallpaper_menu: Controller<MenuModel>,
    screenshare_menu: Controller<MenuModel>,
    wizard_menu: Controller<MenuModel>,
    ufw_menu: Controller<MenuModel>,
    privacy_menu: Controller<MenuModel>,
    bluetooth_menu: Controller<MenuModel>,
    cpu_dashboard_menu: Controller<MenuModel>,
    audio_dashboard_menu: Controller<MenuModel>,
    system_update_menu: Controller<MenuModel>,
    valent_menu: Controller<MenuModel>,
    weather_menu: Controller<MenuModel>,
    keep_awake_menu: Controller<MenuModel>,
    twilight_menu: Controller<MenuModel>,
    keybinds_menu: Controller<MenuModel>,
    alarmclock_menu: Controller<MenuModel>,
    dock_menu: Controller<MenuModel>,
    control_center_menu: Controller<MenuModel>,
    ssh_menu: Controller<MenuModel>,
    dns_menu: Controller<MenuModel>,
    vpn_menu: Controller<MenuModel>,
    ai_menu: Controller<MenuModel>,
    podman_menu: Controller<MenuModel>,
    notes_menu: Controller<MenuModel>,
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
    ip_menu: Controller<MenuModel>,
    vpn_indicator_menu: Controller<MenuModel>,
    network_menu: Controller<MenuModel>,
    power_menu: Controller<MenuModel>,
    media_player_menu: Controller<MenuModel>,
    lyrics_menu: Controller<MenuModel>,
    session_menu: Controller<MenuModel>,
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
    mdash_menu: Controller<MenuModel>,
    margo_layout_menu: Controller<MenuModel>,
    /// Pending keyboard-mode switch held inside the 90 ms debounce
    /// window. Replaced on every `sync_keyboard_mode` call; the
    /// timer reads whatever value was last written.
    pending_kbd_mode: std::rc::Rc<std::cell::RefCell<Option<gtk4_layer_shell::KeyboardMode>>>,
    pending_kbd_mode_timeout: std::rc::Rc<std::cell::RefCell<Option<gtk::glib::SourceId>>>,
    _effects: EffectScope,
}

/// The menus that share the uniform open path: toggle the menu's reveal,
/// then re-sync the layer surface's keyboard mode. Each maps to its
/// menu-name key via [`MenuId::menu_name`]. Folding these into the single
/// parametrised [`FrameInput::ToggleMenu`] replaces the ~34 data-less
/// `Toggle*Menu` variants (and their identical match arms) that used to
/// enumerate them one by one. Menus whose open path is *not* uniform —
/// Settings (built on demand), Screenshare (forwards a reply channel), the
/// WASM / declarative plugin panels, and AppLauncher-with-tab — keep their
/// own bespoke `FrameInput` variants below.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuId {
    Clock,
    Clipboard,
    Notification,
    Screenshot,
    AppLauncher,
    Wallpaper,
    Wizard,
    Ufw,
    Privacy,
    Bluetooth,
    CpuDashboard,
    AudioDashboard,
    SystemUpdate,
    Valent,
    Weather,
    KeepAwake,
    Twilight,
    Keybinds,
    AlarmClock,
    Dock,
    ControlCenter,
    SshSessions,
    Dns,
    Vpn,
    Ai,
    Podman,
    Notes,
    Ip,
    VpnIndicator,
    Network,
    Power,
    MediaPlayer,
    Lyrics,
    Session,
    Mdash,
    MargoLayout,
}

impl MenuId {
    /// The stable menu-name key this id toggles — the same `&str` the
    /// per-menu arms passed to [`Frame::toggle_menu`] before the collapse.
    const fn menu_name(self) -> &'static str {
        match self {
            MenuId::Clock => CLOCK_MENU,
            MenuId::Clipboard => CLIPBOARD_MENU,
            MenuId::Notification => NOTIFICATION_MENU,
            MenuId::Screenshot => SCREENSHOT_MENU,
            MenuId::AppLauncher => APP_LAUNCHER_MENU,
            MenuId::Wallpaper => WALLPAPER_MENU,
            MenuId::Wizard => WIZARD_MENU,
            MenuId::Ufw => NUFW_MENU,
            MenuId::Privacy => PRIVACY_MENU,
            MenuId::Bluetooth => BLUETOOTH_MENU,
            MenuId::CpuDashboard => CPU_DASHBOARD_MENU,
            MenuId::AudioDashboard => AUDIO_DASHBOARD_MENU,
            MenuId::SystemUpdate => SYSTEM_UPDATE_MENU,
            MenuId::Valent => VALENT_MENU,
            MenuId::Weather => WEATHER_MENU,
            MenuId::KeepAwake => KEEP_AWAKE_MENU,
            MenuId::Twilight => TWILIGHT_MENU,
            MenuId::Keybinds => KEYBINDS_MENU,
            MenuId::AlarmClock => ALARMCLOCK_MENU,
            MenuId::Dock => DOCK_MENU,
            MenuId::ControlCenter => CONTROL_CENTER_MENU,
            MenuId::SshSessions => SSH_MENU,
            MenuId::Dns => NDNS_MENU,
            MenuId::Vpn => NVPN_MENU,
            MenuId::Ai => NAI_MENU,
            MenuId::Podman => NPODMAN_MENU,
            MenuId::Notes => NNOTES_MENU,
            MenuId::Ip => NIP_MENU,
            MenuId::VpnIndicator => NVPN_INDICATOR_MENU,
            MenuId::Network => NNETWORK_MENU,
            MenuId::Power => NPOWER_MENU,
            MenuId::MediaPlayer => MEDIA_PLAYER_MENU,
            MenuId::Lyrics => LYRICS_MENU,
            MenuId::Session => SESSION_MENU,
            MenuId::Mdash => MDASH_MENU,
            MenuId::MargoLayout => MARGO_LAYOUT_MENU,
        }
    }
}

#[derive(Debug)]
pub enum FrameInput {
    SetDrawFrame(bool),
    QueueFrameRedraw,
    SetLeftMenuExpansionType(VerticalMenuExpansion),
    SetRightMenuExpansionType(VerticalMenuExpansion),
    /// Re-place every left/right-side menu in the stack. Carries the
    /// snapshot of all menu positions the firing effect just read, so the
    /// handler can skip the (destructive) restack when nothing actually
    /// moved — the config store is coarse, so this effect re-fires on
    /// every unrelated setting write. Fired by the per-output effect that
    /// subscribes to all menu positions.
    RepositionMenus(Vec<Position>),
    /// Toggle one of the uniform menus — open path is reveal + keyboard-mode
    /// sync. The [`MenuId`] payload selects which. Menus with bespoke open
    /// logic keep their own variants below.
    ToggleMenu(MenuId),
    /// Re-assert the layer surface's keyboard interactivity from the
    /// current menu-reveal state. Fired once when the frame surface
    /// first maps: gtk4-layer-shell's initial commit can land with a
    /// non-`None` keyboard mode in some races (observed: the frame
    /// holding `Exclusive` at login while no menu is open, trapping
    /// keyboard focus into the invisible full-screen layer until the
    /// first menu toggle finally ran `sync_keyboard_mode`). This
    /// enforces the "Exclusive iff a menu is revealed" invariant
    /// proactively at map time instead of only reactively on toggle.
    SyncKeyboardMode,
    /// Forward a Hidden Bar IPC verb to both bars' drawers. The optional
    /// target name selects a single named drawer; `None` reaches all.
    HiddenBar(mshell_common::hidden_bar::HiddenBarVerb, Option<String>),
    /// A bar reported its target reserved height (`BarOutput::ReserveHeight`).
    /// Routed to that bar's FrameSpacer so the layer-shell exclusive zone
    /// jumps to the final value at toggle time (one smooth compositor
    /// resize), instead of being streamed from the Revealer slide.
    /// `is_top` picks which spacer; the height is the bar's natural content.
    SpacerReserve {
        is_top: bool,
        height: i32,
    },
    /// Same as `ToggleMenu(MenuId::AppLauncher)` but also forwards a
    /// `SelectCategory(tab)` into the underlying
    /// `AppLauncherModel` once the menu is open. Bridges
    /// `mshellctl menu app-launcher --tab Run` to the runtime's
    /// existing category-cycle path.
    ToggleAppLauncherMenuWithTab(String),
    /// Open the first-class plugin-panel menu hosting a plugin's WASM panel,
    /// carrying its compiled component path + resolved settings (JSON) + the
    /// granted host capabilities (comma-separated tokens; deny-by-default).
    ToggleWasmPluginPanel {
        name: String,
        entry: String,
        settings: String,
        capabilities: String,
        min_width: i32,
        max_height: i32,
    },
    /// Open a declarative plugin menu (its `[[widget.menu]]` command rows) in
    /// the first-class plugin menu — layer-shell, not a pill popover.
    TogglePluginMenu {
        name: String,
        rows: Vec<CustomMenuRow>,
        min_width: i32,
        max_height: i32,
    },
    /// Toggle a plugin's panel/menu addressed by key (from `mshellctl menu
    /// plugin <key>`). Generic — resolves the key to the plugin's derived
    /// widget, then dispatches to the panel or menu path. No per-plugin code.
    TogglePluginByKey(String),
    /// Force-reload an installed plugin's WASM panel — evict the cached
    /// instance so the next open re-instantiates from disk. Lets a plugin
    /// author wire `cargo watch` to `mshellctl plugin reload <key>` for a
    /// fast iteration loop without restarting mshell.
    ReloadPlugin(String),
    /// A global keybind for `(plugin-key, bind-id)` fired: open (or
    /// reveal) the plugin's panel and deliver a `Keybind` event to the
    /// guest with the bind id. Margo binds in the generated
    /// `binds.d/mshell-plugins.conf` spawn `mshellctl plugin keybind …`
    /// which lands here.
    FirePluginKeybind(String, String),
    ToggleSettingsMenu,
    /// Open Settings and jump to a specific sidebar section.
    /// Used by the launcher's Settings provider via the shell
    /// router. If Settings is already visible, just switches the
    /// section.
    OpenSettingsAtSection(String),
    /// Force-close Settings on this frame if it's currently open.
    /// Used by the Shell-level Settings router to keep Settings
    /// single-monitor: when a fresh toggle picks frame A, frame B's
    /// open Settings is closed out from under it so the panel
    /// doesn't linger on a monitor the user is no longer viewing.
    CloseSettingsMenu,
    CloseMenus,
    /// Esc while the clipboard `/` filter is open — tell the clipboard
    /// menu to leave search mode instead of closing the whole menu.
    ClipboardExitSearch,
    ToggleScreenshareMenu(tokio::sync::oneshot::Sender<String>, String),
    BarToggleTop,
    BarToggleBottom,
    BarToggleLeft,
    BarToggleRight,
    BarToggleAll(bool),
    BarRevealAll(bool),
    BarHideAll(bool),
}

#[derive(Debug)]
pub enum FrameOutput {}

pub struct FrameInit {
    pub monitor: gdk::Monitor,
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

                    #[name = "top_bar_container"]
                    BoxWithResize::new() -> BoxWithResize {
                        set_hexpand: true,
                        append = &model.top_bar.widget().clone() {},
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

                    #[name = "bottom_bar_container"]
                    BoxWithResize::new() -> BoxWithResize {
                        set_hexpand: true,

                        append = &model.bottom_bar.widget().clone() {},
                    },
                },

                #[name = "frame_draw_widget"]
                FrameDrawWidget::new() -> FrameDrawWidget {
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
        root.set_visible(true);
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

        let calendar_menu = Self::build_menu(&sender, MenuType::Clock);
        let clipboard_menu = Self::build_menu(&sender, MenuType::Clipboard);
        let notification_menu = Self::build_menu(&sender, MenuType::Notifications);
        let screenshot_menu = Self::build_menu(&sender, MenuType::Screenshot);
        let app_launcher_menu = Self::build_menu(&sender, MenuType::AppLauncher);
        let wallpaper_menu = Self::build_menu(&sender, MenuType::Wallpaper);
        let screenshare_menu = Self::build_menu(&sender, MenuType::HyprlandScreenshare);
        let wizard_menu = Self::build_menu(&sender, MenuType::Wizard);
        let ufw_menu = Self::build_menu(&sender, MenuType::Ufw);
        let privacy_menu = Self::build_menu(&sender, MenuType::Privacy);
        let bluetooth_menu = Self::build_menu(&sender, MenuType::Bluetooth);
        let cpu_dashboard_menu = Self::build_menu(&sender, MenuType::CpuDashboard);
        let audio_dashboard_menu = Self::build_menu(&sender, MenuType::AudioDashboard);
        let system_update_menu = Self::build_menu(&sender, MenuType::SystemUpdate);
        let valent_menu = Self::build_menu(&sender, MenuType::Valent);
        let weather_menu = Self::build_menu(&sender, MenuType::Weather);
        let keep_awake_menu = Self::build_menu(&sender, MenuType::KeepAwake);
        let twilight_menu = Self::build_menu(&sender, MenuType::Twilight);
        let keybinds_menu = Self::build_menu(&sender, MenuType::Keybinds);
        let alarmclock_menu = Self::build_menu(&sender, MenuType::AlarmClock);
        let dock_menu = Self::build_menu(&sender, MenuType::Dock);
        let control_center_menu = Self::build_menu(&sender, MenuType::ControlCenter);
        let ssh_menu = Self::build_menu(&sender, MenuType::SshSessions);
        let dns_menu = Self::build_menu(&sender, MenuType::Dns);
        let vpn_menu = Self::build_menu(&sender, MenuType::Vpn);
        let ai_menu = Self::build_menu(&sender, MenuType::Ai);
        let podman_menu = Self::build_menu(&sender, MenuType::Podman);
        let notes_menu = Self::build_menu(&sender, MenuType::Notes);
        let plugin_panel_menu = Self::build_menu(&sender, MenuType::PluginPanel);
        let ip_menu = Self::build_menu(&sender, MenuType::Ip);
        let vpn_indicator_menu = Self::build_menu(&sender, MenuType::VpnIndicator);
        let network_menu = Self::build_menu(&sender, MenuType::Network);
        let power_menu = Self::build_menu(&sender, MenuType::Power);
        let media_player_menu = Self::build_menu(&sender, MenuType::MediaPlayer);
        let lyrics_menu = Self::build_menu(&sender, MenuType::Lyrics);
        let session_menu = Self::build_menu(&sender, MenuType::Session);
        let mdash_menu = Self::build_menu(&sender, MenuType::Mdash);
        let margo_layout_menu = Self::build_menu(&sender, MenuType::MargoLayout);

        // Settings doesn't go through `build_menu` because its content
        // isn't a list of `MenuWidget`s — it's a custom sidebar + stack
        // laid out by `SettingsWindowModel`. It is also NOT built here:
        // the panel's ~48 page controllers are deferred to first open via
        // `ensure_settings_built` (see the field doc). The shell-level
        // dispatcher emits `ToggleSettingsMenu` to the right Frame, which
        // builds + attaches the panel on demand.

        let mut effects = EffectScope::new();

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
            pos!(notes_menu);
            pos!(ip_menu);
            pos!(vpn_indicator_menu);
            pos!(network_menu);
            pos!(power_menu);
            pos!(media_player_menu);
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

        let model = Frame {
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
            clock_menu: calendar_menu,
            clipboard_menu,
            notification_menu,
            screenshot_menu,
            app_launcher_menu,
            wallpaper_menu,
            screenshare_menu,
            wizard_menu,
            ufw_menu,
            privacy_menu,
            bluetooth_menu,
            cpu_dashboard_menu,
            audio_dashboard_menu,
            system_update_menu,
            valent_menu,
            weather_menu,
            keep_awake_menu,
            twilight_menu,
            keybinds_menu,
            alarmclock_menu,
            dock_menu,
            control_center_menu,
            ssh_menu,
            dns_menu,
            vpn_menu,
            ai_menu,
            podman_menu,
            notes_menu,
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
            ip_menu,
            vpn_indicator_menu,
            network_menu,
            power_menu,
            media_player_menu,
            lyrics_menu,
            session_menu,
            settings_menu: None,
            mdash_menu,
            margo_layout_menu,
            pending_kbd_mode: std::rc::Rc::new(std::cell::RefCell::new(None)),
            pending_kbd_mode_timeout: std::rc::Rc::new(std::cell::RefCell::new(None)),
            _effects: effects,
        };

        let widgets = view_output!();

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
                self.toggle_menu(id.menu_name(), widgets);
                self.sync_keyboard_mode(root);
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
                if !self.is_menu_visible_now(APP_LAUNCHER_MENU, widgets) {
                    self.toggle_menu(APP_LAUNCHER_MENU, widgets);
                }
                self.app_launcher_menu
                    .sender()
                    .send(MenuInput::AppLauncherSelectCategory(tab))
                    .ok();
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
                self.apply_plugin_layout(&name, widgets);
                #[cfg(feature = "wasm-plugins")]
                {
                    use std::collections::hash_map::Entry;
                    // Build the panel once per plugin key; reuse it (and its
                    // chat state) on later opens.
                    if let Entry::Vacant(slot) = self.plugin_panels.entry(name.clone()) {
                        let parsed: std::collections::HashMap<String, String> =
                            serde_json::from_str(settings.trim()).unwrap_or_default();
                        match mshell_plugin_ui::PluginPanel::new(
                            &self.plugin_panel_runtime,
                            &name,
                            std::path::Path::new(entry.trim()),
                            parsed,
                            &capabilities,
                        ) {
                            Ok(panel) => {
                                slot.insert(panel);
                            }
                            Err(e) => tracing::warn!("plugin panel `{name}`: load failed: {e}"),
                        }
                    }
                    if let Some(panel) = self.plugin_panels.get(&name) {
                        let content: Widget = panel.widget().clone().upcast();
                        self.plugin_panel_menu
                            .sender()
                            .send(MenuInput::SetExternalContent(content))
                            .ok();
                    }
                    self.toggle_menu(NPLUGIN_PANEL_MENU, widgets);
                    self.sync_keyboard_mode(root);
                }
                #[cfg(not(feature = "wasm-plugins"))]
                {
                    let _ = (name, entry, settings, capabilities);
                }
            }
            FrameInput::TogglePluginMenu {
                name,
                rows,
                min_width: _,
                max_height: _,
            } => {
                self.apply_plugin_layout(&name, widgets);
                let content = Self::build_plugin_menu_content(&rows, &sender);
                self.plugin_panel_menu
                    .sender()
                    .send(MenuInput::SetExternalContent(content))
                    .ok();
                self.toggle_menu(NPLUGIN_PANEL_MENU, widgets);
                self.sync_keyboard_mode(root);
            }
            FrameInput::TogglePluginByKey(key) => {
                // Generic `mshellctl menu plugin <key>`: resolve the key to an
                // enabled plugin's derived widget (matching its composite key,
                // widget key, or full name) and dispatch to the panel or menu
                // path. No per-plugin code — any installed plugin works.
                let found = mshell_config::config_manager::config_manager()
                    .config()
                    .bars()
                    .widgets()
                    .custom_widgets()
                    .get_untracked()
                    .into_iter()
                    .find(|c| {
                        let Some(rest) = c.name.strip_prefix("plugin:") else {
                            return false;
                        };
                        if c.panel_entry.trim().is_empty() && c.menu.is_empty() {
                            return false;
                        }
                        let (comp, w) = rest.rsplit_once(':').unwrap_or((rest, ""));
                        rest == key || comp == key || w == key
                    });
                match found {
                    Some(c) if !c.panel_entry.trim().is_empty() => {
                        sender.input(FrameInput::ToggleWasmPluginPanel {
                            name: c.name,
                            entry: c.panel_entry,
                            settings: c.panel_settings,
                            capabilities: c.panel_capabilities,
                            min_width: c.panel_min_width,
                            max_height: c.panel_max_height,
                        });
                    }
                    Some(c) => {
                        sender.input(FrameInput::TogglePluginMenu {
                            name: c.name,
                            rows: c.menu,
                            min_width: c.panel_min_width,
                            max_height: c.panel_max_height,
                        });
                    }
                    None => {
                        tracing::warn!("menu plugin: no enabled plugin panel/menu for key `{key}`")
                    }
                }
            }
            FrameInput::ReloadPlugin(key) => {
                #[cfg(feature = "wasm-plugins")]
                {
                    // Match any cached panel whose name encodes this key —
                    // composite key, widget key, or the full `plugin:<comp>:<w>`.
                    let names: Vec<String> = self
                        .plugin_panels
                        .keys()
                        .filter(|name| {
                            let Some(rest) = name.strip_prefix("plugin:") else {
                                return false;
                            };
                            let (comp, w) = rest.rsplit_once(':').unwrap_or((rest, ""));
                            rest == key || comp == key || w == key
                        })
                        .cloned()
                        .collect();
                    for name in names {
                        self.plugin_panels.remove(&name);
                    }
                }
                #[cfg(not(feature = "wasm-plugins"))]
                let _ = key;
            }
            FrameInput::FirePluginKeybind(key, bind_id) => {
                #[cfg(feature = "wasm-plugins")]
                {
                    use std::collections::hash_map::Entry;
                    // Resolve the key against the synthesised plugin widgets
                    // the same way TogglePluginByKey does — accept the
                    // composite key, the widget key, or the full name.
                    let found = mshell_config::config_manager::config_manager()
                        .config()
                        .bars()
                        .widgets()
                        .custom_widgets()
                        .get_untracked()
                        .into_iter()
                        .find(|c| {
                            let Some(rest) = c.name.strip_prefix("plugin:") else {
                                return false;
                            };
                            if c.panel_entry.trim().is_empty() {
                                return false;
                            }
                            let (comp, w) = rest.rsplit_once(':').unwrap_or((rest, ""));
                            rest == key || comp == key || w == key
                        });
                    if let Some(c) = found {
                        self.apply_plugin_layout(&c.name, widgets);
                        // Build/get the cached panel.
                        if let Entry::Vacant(slot) = self.plugin_panels.entry(c.name.clone()) {
                            let parsed: std::collections::HashMap<String, String> =
                                serde_json::from_str(c.panel_settings.trim()).unwrap_or_default();
                            match mshell_plugin_ui::PluginPanel::new(
                                &self.plugin_panel_runtime,
                                &c.name,
                                std::path::Path::new(c.panel_entry.trim()),
                                parsed,
                                &c.panel_capabilities,
                            ) {
                                Ok(panel) => {
                                    slot.insert(panel);
                                }
                                Err(e) => tracing::warn!(
                                    "plugin keybind `{key}/{bind_id}`: panel load failed: {e}"
                                ),
                            }
                        }
                        if let Some(panel) = self.plugin_panels.get(&c.name) {
                            // Deliver the keybind event so the guest can react.
                            panel.fire_event(mshell_plugin_ui::UiEvent {
                                id: bind_id,
                                kind: mshell_plugin_ui::UiEventKind::Keybind,
                                value: String::new(),
                            });
                            // Mount + open the panel surface.
                            let content: Widget = panel.widget().clone().upcast();
                            self.plugin_panel_menu
                                .sender()
                                .send(MenuInput::SetExternalContent(content))
                                .ok();
                            self.toggle_menu(NPLUGIN_PANEL_MENU, widgets);
                            self.sync_keyboard_mode(root);
                        }
                    } else {
                        tracing::warn!("plugin keybind: no enabled plugin matches `{key}`");
                    }
                }
                #[cfg(not(feature = "wasm-plugins"))]
                let _ = (key, bind_id);
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
                // Esc, so its clipboard is the one in search mode.
                self.clipboard_menu
                    .sender()
                    .send(MenuInput::ClipboardExitSearch)
                    .unwrap_or_default();
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

                self.clock_menu
                    .sender()
                    .send(MenuInput::RevealChanged(false))
                    .unwrap_or_default();

                self.clipboard_menu
                    .sender()
                    .send(MenuInput::RevealChanged(false))
                    .unwrap_or_default();

                self.notification_menu
                    .sender()
                    .send(MenuInput::RevealChanged(false))
                    .unwrap_or_default();

                self.screenshot_menu
                    .sender()
                    .send(MenuInput::RevealChanged(false))
                    .unwrap_or_default();

                self.app_launcher_menu
                    .sender()
                    .send(MenuInput::RevealChanged(false))
                    .unwrap_or_default();

                self.wallpaper_menu
                    .sender()
                    .send(MenuInput::RevealChanged(false))
                    .unwrap_or_default();

                self.screenshare_menu
                    .sender()
                    .send(MenuInput::RevealChanged(false))
                    .unwrap_or_default();

                // Idle the lazy-poll menus too, so closing the menu
                // stops their refresh loop's I/O until next reveal.
                self.ip_menu
                    .sender()
                    .send(MenuInput::RevealChanged(false))
                    .unwrap_or_default();

                self.vpn_indicator_menu
                    .sender()
                    .send(MenuInput::RevealChanged(false))
                    .unwrap_or_default();

                self.dns_menu
                    .sender()
                    .send(MenuInput::RevealChanged(false))
                    .unwrap_or_default();

                self.vpn_menu
                    .sender()
                    .send(MenuInput::RevealChanged(false))
                    .unwrap_or_default();

                self.ai_menu
                    .sender()
                    .send(MenuInput::RevealChanged(false))
                    .unwrap_or_default();

                self.ufw_menu
                    .sender()
                    .send(MenuInput::RevealChanged(false))
                    .unwrap_or_default();

                self.podman_menu
                    .sender()
                    .send(MenuInput::RevealChanged(false))
                    .unwrap_or_default();

                self.network_menu
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
            FrameInput::BarToggleLeft | FrameInput::BarToggleRight => {
                // Vertical Left / Right bars removed; treat the input
                // as a no-op so existing mshellctl bindings don't error.
                tracing::debug!("BarToggleLeft/Right ignored — vertical bars removed");
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
        self.update_view(widgets, sender);
    }
}

impl Frame {
    fn any_menu_revealed(&self) -> bool {
        self.left_revealed
            || self.right_revealed
            || self.top_revealed
            || self.top_left_revealed
            || self.top_right_revealed
            || self.bottom_revealed
            || self.bottom_left_revealed
            || self.bottom_right_revealed
    }

    /// Switch the frame's layer-shell keyboard interactivity to track
    /// the current menu state. We need `Exclusive` while a menu is
    /// open so the compositor actually delivers keys to mshell — with
    /// margo's `compute_desired_focus`, anything weaker (OnDemand,
    /// None) gets the focus stolen back by the active toplevel
    /// window between the pointer click and the next refresh, and
    /// the ESC shortcut on the frame never fires. When no menus are
    /// open we go back to `None` so the user's toplevels keep the
    /// keyboard.
    ///
    /// **Debounced.** Each call (re)arms a 90 ms glib timer; only the
    /// last menu-state determines the applied mode. Without this,
    /// rapid widget clicks (open → close → open …) submit a flurry
    /// of `set_keyboard_interactivity` changes; each commit forces
    /// margo to `arrange()` the layer map, ack a fresh configure,
    /// recompute focus, and the bar visibly drops out for ~1 frame
    /// every cycle — the user sees the bar entirely disappear when
    /// clicking widgets in quick succession.
    fn sync_keyboard_mode(&self, root: &<Self as Component>::Root) {
        let desired = if self.any_menu_revealed() {
            gtk4_layer_shell::KeyboardMode::Exclusive
        } else {
            gtk4_layer_shell::KeyboardMode::None
        };

        // Cancel any pending switch and replace it with the new one.
        if let Some(id) = self.pending_kbd_mode_timeout.borrow_mut().take() {
            id.remove();
        }
        *self.pending_kbd_mode.borrow_mut() = Some(desired);

        let root_weak = root.downgrade();
        let pending_mode = self.pending_kbd_mode.clone();
        let pending_timeout = self.pending_kbd_mode_timeout.clone();
        let id =
            gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(90), move || {
                *pending_timeout.borrow_mut() = None;
                let Some(mode) = pending_mode.borrow_mut().take() else {
                    return;
                };
                let Some(root) = root_weak.upgrade() else {
                    return;
                };
                root.set_keyboard_mode(mode);
                // Force a surface commit so the new keyboard_interactivity
                // actually reaches the compositor. When a menu is closed via
                // IPC (`mshellctl dock toggle`) rather than a keypress, there is
                // no further input/render to flush the deferred layer-shell
                // state, so margo never sees the Exclusive→None flip and keeps
                // the keyboard until the next event (the user had to press Esc).
                root.queue_draw();
                tracing::debug!(?mode, "frame: sync_keyboard_mode (applied)");
            });
        *self.pending_kbd_mode_timeout.borrow_mut() = Some(id);
    }

    /// True if a menu by `name` is the visible child of any stack
    /// AND that stack is currently revealed on this frame. Used by
    /// the idempotent `Close*Menu` paths to skip the toggle when
    /// the menu isn't actually showing here.
    fn is_menu_visible_now(&self, name: &str, widgets: &FrameWidgets) -> bool {
        let stacks = [
            (&widgets.left_stack, self.left_revealed),
            (&widgets.right_stack, self.right_revealed),
            (&widgets.top_stack, self.top_revealed),
            (&widgets.top_left_stack, self.top_left_revealed),
            (&widgets.top_right_stack, self.top_right_revealed),
            (&widgets.bottom_stack, self.bottom_revealed),
            (&widgets.bottom_left_stack, self.bottom_left_revealed),
            (&widgets.bottom_right_stack, self.bottom_right_revealed),
        ];
        stacks.iter().any(|(stack, revealed)| {
            *revealed && stack.visible_child_name().map(|n| n.to_string()) == Some(name.to_string())
        })
    }

    /// Build the Settings panel + attach it to its stack on first open.
    /// Deferred from `init` because the panel launches ~48 page
    /// controllers and one Frame exists per monitor (see the
    /// `settings_menu` field doc). Idempotent: a no-op once built.
    fn ensure_settings_built(&mut self, widgets: &FrameWidgets) {
        if self.settings_menu.is_some() {
            return;
        }
        let controller = mshell_settings::SettingsWindowModel::builder()
            .launch(mshell_settings::SettingsWindowInit {
                monitor: Some(self.monitor.clone()),
            })
            .detach();
        let widget: Widget = controller.widget().clone().upcast();
        let position = mshell_config::config_manager::config_manager()
            .config()
            .menus()
            .settings_menu()
            .position()
            .get();
        Self::add_to_stack(widgets, &widget, SETTINGS_MENU, &position);
        self.settings_menu = Some(controller);
    }

    fn toggle_menu(&mut self, name: &str, widgets: &mut FrameWidgets) {
        let mut now_visible = true;
        let in_left = widgets.left_stack.child_by_name(name).is_some();
        let in_right = widgets.right_stack.child_by_name(name).is_some();
        let in_top = widgets.top_stack.child_by_name(name).is_some();
        let in_top_left = widgets.top_left_stack.child_by_name(name).is_some();
        let in_top_right = widgets.top_right_stack.child_by_name(name).is_some();
        let in_bottom = widgets.bottom_stack.child_by_name(name).is_some();
        let in_bottom_left = widgets.bottom_left_stack.child_by_name(name).is_some();
        let in_bottom_right = widgets.bottom_right_stack.child_by_name(name).is_some();

        let left_revealed = self.left_revealed;
        let right_revealed = self.right_revealed;
        let top_revealed = self.top_revealed;
        let top_left_revealed = self.top_left_revealed;
        let top_right_revealed = self.top_right_revealed;
        let bottom_revealed = self.bottom_revealed;
        let bottom_left_revealed = self.bottom_left_revealed;
        let bottom_right_revealed = self.bottom_right_revealed;

        self.left_revealed = false;
        self.right_revealed = false;
        self.top_revealed = false;
        self.top_left_revealed = false;
        self.top_right_revealed = false;
        self.bottom_revealed = false;
        self.bottom_left_revealed = false;
        self.bottom_right_revealed = false;

        if in_left {
            if let Some(visible) = widgets.left_stack.visible_child_name() {
                if visible.as_str() == name {
                    self.left_revealed = !left_revealed;
                    now_visible = self.left_revealed;
                } else {
                    widgets
                        .left_stack
                        .set_visible_child_full(name, gtk::StackTransitionType::None);
                    self.left_revealed = true;
                }
            }
        } else if in_right {
            if let Some(visible) = widgets.right_stack.visible_child_name() {
                if visible.as_str() == name {
                    self.right_revealed = !right_revealed;
                    now_visible = self.right_revealed;
                } else {
                    widgets
                        .right_stack
                        .set_visible_child_full(name, gtk::StackTransitionType::None);
                    self.right_revealed = true;
                }
            }
        } else if in_top {
            if let Some(visible) = widgets.top_stack.visible_child_name() {
                if visible.as_str() == name {
                    self.top_revealed = !top_revealed;
                    now_visible = self.top_revealed;
                } else {
                    widgets
                        .top_stack
                        .set_visible_child_full(name, gtk::StackTransitionType::None);
                    self.top_revealed = true;
                }
            }
        } else if in_top_left {
            if let Some(visible) = widgets.top_left_stack.visible_child_name() {
                if visible.as_str() == name {
                    self.top_left_revealed = !top_left_revealed;
                    now_visible = self.top_left_revealed;
                } else {
                    widgets
                        .top_left_stack
                        .set_visible_child_full(name, gtk::StackTransitionType::None);
                    self.top_left_revealed = true;
                }
            }
        } else if in_top_right {
            if let Some(visible) = widgets.top_right_stack.visible_child_name() {
                if visible.as_str() == name {
                    self.top_right_revealed = !top_right_revealed;
                    now_visible = self.top_right_revealed;
                } else {
                    widgets
                        .top_right_stack
                        .set_visible_child_full(name, gtk::StackTransitionType::None);
                    self.top_right_revealed = true;
                }
            }
        } else if in_bottom {
            if let Some(visible) = widgets.bottom_stack.visible_child_name() {
                if visible.as_str() == name {
                    self.bottom_revealed = !bottom_revealed;
                    now_visible = self.bottom_revealed;
                } else {
                    widgets
                        .bottom_stack
                        .set_visible_child_full(name, gtk::StackTransitionType::None);
                    self.bottom_revealed = true;
                }
            }
        } else if in_bottom_left {
            if let Some(visible) = widgets.bottom_left_stack.visible_child_name() {
                if visible.as_str() == name {
                    self.bottom_left_revealed = !bottom_left_revealed;
                    now_visible = self.bottom_left_revealed;
                } else {
                    widgets
                        .bottom_left_stack
                        .set_visible_child_full(name, gtk::StackTransitionType::None);
                    self.bottom_left_revealed = true;
                }
            }
        } else if in_bottom_right
            && let Some(visible) = widgets.bottom_right_stack.visible_child_name()
        {
            if visible.as_str() == name {
                self.bottom_right_revealed = !bottom_right_revealed;
                now_visible = self.bottom_right_revealed;
            } else {
                widgets
                    .bottom_right_stack
                    .set_visible_child_full(name, gtk::StackTransitionType::None);
                self.bottom_right_revealed = true;
            }
        }

        self.clock_menu
            .sender()
            .send(MenuInput::RevealChanged(name == CLOCK_MENU && now_visible))
            .unwrap_or_default();

        self.clipboard_menu
            .sender()
            .send(MenuInput::RevealChanged(
                name == CLIPBOARD_MENU && now_visible,
            ))
            .unwrap_or_default();

        self.notification_menu
            .sender()
            .send(MenuInput::RevealChanged(
                name == NOTIFICATION_MENU && now_visible,
            ))
            .unwrap_or_default();

        self.screenshot_menu
            .sender()
            .send(MenuInput::RevealChanged(
                name == SCREENSHOT_MENU && now_visible,
            ))
            .unwrap_or_default();

        self.app_launcher_menu
            .sender()
            .send(MenuInput::RevealChanged(
                name == APP_LAUNCHER_MENU && now_visible,
            ))
            .unwrap_or_default();

        self.wallpaper_menu
            .sender()
            .send(MenuInput::RevealChanged(
                name == WALLPAPER_MENU && now_visible,
            ))
            .unwrap_or_default();

        self.screenshare_menu
            .sender()
            .send(MenuInput::RevealChanged(
                name == SCREENSHARE_MENU && now_visible,
            ))
            .unwrap_or_default();

        self.session_menu
            .sender()
            .send(MenuInput::RevealChanged(
                name == SESSION_MENU && now_visible,
            ))
            .unwrap_or_default();

        // These menus poll lazily on reveal (network/IP/DNS/UFW/podman
        // do no background work until first opened), so they must be
        // told when they become visible — otherwise they never fetch.
        self.ip_menu
            .sender()
            .send(MenuInput::RevealChanged(name == NIP_MENU && now_visible))
            .unwrap_or_default();

        self.vpn_indicator_menu
            .sender()
            .send(MenuInput::RevealChanged(
                name == NVPN_INDICATOR_MENU && now_visible,
            ))
            .unwrap_or_default();

        self.dns_menu
            .sender()
            .send(MenuInput::RevealChanged(name == NDNS_MENU && now_visible))
            .unwrap_or_default();

        self.vpn_menu
            .sender()
            .send(MenuInput::RevealChanged(name == NVPN_MENU && now_visible))
            .unwrap_or_default();

        self.ai_menu
            .sender()
            .send(MenuInput::RevealChanged(name == NAI_MENU && now_visible))
            .unwrap_or_default();

        self.ufw_menu
            .sender()
            .send(MenuInput::RevealChanged(name == NUFW_MENU && now_visible))
            .unwrap_or_default();

        self.podman_menu
            .sender()
            .send(MenuInput::RevealChanged(
                name == NPODMAN_MENU && now_visible,
            ))
            .unwrap_or_default();

        self.network_menu
            .sender()
            .send(MenuInput::RevealChanged(
                name == NNETWORK_MENU && now_visible,
            ))
            .unwrap_or_default();
    }

    // Can't use sender for this.  Must queue redraw in the callback.  Otherwise, there is a slight
    // delay and the frame isn't draw immediately.
    fn attach_resize_listeners(&self, widgets: &FrameWidgets) {
        // NB: neither the spacer's reserved height (layer-shell exclusive
        // zone) NOR the drawn frame band's top/bottom thickness is driven from
        // a `resized` stream any more. The Revealer fires "resized" 60×/s
        // during a bar slide (which re-tiled the compositor every frame and
        // tore window borders off the slide), and — worse — it reports the bar
        // container's *allocated* height, which exceeds the bar's *reserved*
        // height (`bar_center`'s natural measure). Driving the painted band
        // off that allocated height made the frame (TOP layer, above tiled
        // windows) bleed past `work_area` and paint over the lone window's
        // top/bottom border once `smartgaps` collapsed the gap. Both the
        // exclusive zone AND the band are now driven by
        // `BarOutput::ReserveHeight` (→ `FrameInput::SpacerReserve`), which
        // jumps straight to the target once and keeps hole edge == work_area
        // edge. The left/right thickness listeners below stay: side menus have
        // no spacer and their width legitimately tracks the live content.
        let frame_widget = widgets.frame_draw_widget.clone();
        widgets.left_bar_and_menu_container.connect_local(
            "resized",
            false,
            move |values: &[glib::Value]| {
                let width = values[1].get::<i32>().expect("width i32");
                frame_widget.update_style(|s| s.left_thickness = width as f64);
                None
            },
        );
        // NOTE: Vertical Left / Right bars were removed; the
        // `left_bar_container` / `right_bar_container` widgets they
        // owned (with `left_spacer` / `right_spacer` listening for
        // resize) are gone with them. Menus that anchor to the side
        // edges still live inside `left_bar_and_menu_container` /
        // `right_bar_and_menu_container`, but they have no spacer —
        // their width is driven by the menu content directly.

        let frame_widget = widgets.frame_draw_widget.clone();
        widgets.right_bar_and_menu_container.connect_local(
            "resized",
            false,
            move |values: &[glib::Value]| {
                let width = values[1].get::<i32>().expect("width i32");
                frame_widget.update_style(|s| s.right_thickness = width as f64);
                None
            },
        );

        let frame_widget = widgets.frame_draw_widget.clone();
        widgets.top_left_expander.connect_local(
            "resized",
            false,
            move |values: &[glib::Value]| {
                let height = values[2].get::<i32>().expect("height i32");
                frame_widget.update_style(|s| s.left_top_expander_height = height as f64);
                None
            },
        );
        let frame_widget = widgets.frame_draw_widget.clone();
        widgets.top_right_expander.connect_local(
            "resized",
            false,
            move |values: &[glib::Value]| {
                let height = values[2].get::<i32>().expect("height i32");
                frame_widget.update_style(|s| s.right_top_expander_height = height as f64);
                None
            },
        );
        let frame_widget = widgets.frame_draw_widget.clone();
        widgets.bottom_left_expander.connect_local(
            "resized",
            false,
            move |values: &[glib::Value]| {
                let height = values[2].get::<i32>().expect("height i32");
                frame_widget.update_style(|s| s.left_bottom_expander_height = height as f64);
                None
            },
        );
        let frame_widget = widgets.frame_draw_widget.clone();
        widgets.bottom_right_expander.connect_local(
            "resized",
            false,
            move |values: &[glib::Value]| {
                let height = values[2].get::<i32>().expect("height i32");
                frame_widget.update_style(|s| s.right_bottom_expander_height = height as f64);
                None
            },
        );
        let frame_widget = widgets.frame_draw_widget.clone();
        widgets.left_revealer_container.connect_local(
            "resized",
            false,
            move |values: &[glib::Value]| {
                let width = values[1].get::<i32>().expect("width i32");
                frame_widget.update_style(|s| s.left_expander_width = width as f64);
                None
            },
        );
        let frame_widget = widgets.frame_draw_widget.clone();
        widgets.right_revealer_container.connect_local(
            "resized",
            false,
            move |values: &[glib::Value]| {
                let width = values[1].get::<i32>().expect("width i32");
                frame_widget.update_style(|s| s.right_expander_width = width as f64);
                None
            },
        );

        let frame_widget = widgets.frame_draw_widget.clone();
        widgets.top_revealer_container.connect_local(
            "resized",
            false,
            move |values: &[glib::Value]| {
                let width = values[1].get::<i32>().expect("width i32");
                let height = values[2].get::<i32>().expect("height i32");
                frame_widget.update_style(|s| s.top_revealer_size = (width as f64, height as f64));
                None
            },
        );

        let frame_widget = widgets.frame_draw_widget.clone();
        widgets.top_left_revealer_container.connect_local(
            "resized",
            false,
            move |values: &[glib::Value]| {
                let width = values[1].get::<i32>().expect("width i32");
                let height = values[2].get::<i32>().expect("height i32");
                frame_widget
                    .update_style(|s| s.top_left_revealer_size = (width as f64, height as f64));
                None
            },
        );

        let frame_widget = widgets.frame_draw_widget.clone();
        widgets.top_right_revealer_container.connect_local(
            "resized",
            false,
            move |values: &[glib::Value]| {
                let width = values[1].get::<i32>().expect("width i32");
                let height = values[2].get::<i32>().expect("height i32");
                frame_widget
                    .update_style(|s| s.top_right_revealer_size = (width as f64, height as f64));
                None
            },
        );

        let frame_widget = widgets.frame_draw_widget.clone();
        widgets.bottom_revealer_container.connect_local(
            "resized",
            false,
            move |values: &[glib::Value]| {
                let width = values[1].get::<i32>().expect("width i32");
                let height = values[2].get::<i32>().expect("height i32");
                frame_widget
                    .update_style(|s| s.bottom_revealer_size = (width as f64, height as f64));
                None
            },
        );

        let frame_widget = widgets.frame_draw_widget.clone();
        widgets.bottom_left_revealer_container.connect_local(
            "resized",
            false,
            move |values: &[glib::Value]| {
                let width = values[1].get::<i32>().expect("width i32");
                let height = values[2].get::<i32>().expect("height i32");
                frame_widget
                    .update_style(|s| s.bottom_left_revealer_size = (width as f64, height as f64));
                None
            },
        );

        let frame_widget = widgets.frame_draw_widget.clone();
        widgets.bottom_right_revealer_container.connect_local(
            "resized",
            false,
            move |values: &[glib::Value]| {
                let width = values[1].get::<i32>().expect("width i32");
                let height = values[2].get::<i32>().expect("height i32");
                frame_widget
                    .update_style(|s| s.bottom_right_revealer_size = (width as f64, height as f64));
                None
            },
        );
    }

    fn apply_left_and_right_side_children(&self, widgets: &FrameWidgets) {
        // Every menu's position is read straight from config (the uniform
        // pattern — no positional args). `menu_pos!($menu)` is the
        // read-from-config shorthand used throughout this function.
        macro_rules! menu_pos {
            ($menu:ident) => {
                mshell_config::config_manager::config_manager()
                    .config()
                    .menus()
                    .$menu()
                    .position()
                    .get()
            };
        }
        let clock_menu_position = menu_pos!(clock_menu);
        let clipboard_menu_position = menu_pos!(clipboard_menu);
        let notification_menu_position = menu_pos!(notification_menu);
        let screenshot_menu_position = menu_pos!(screenshot_menu);
        let app_launcher_menu_position = menu_pos!(app_launcher_menu);
        let wallpaper_menu_position = menu_pos!(wallpaper_menu);
        let screenshare_menu_position = menu_pos!(screenshare_menu);
        let ufw_menu_position = menu_pos!(ufw_menu);
        let dns_menu_position = menu_pos!(dns_menu);
        let podman_menu_position = menu_pos!(podman_menu);
        let notes_menu_position = menu_pos!(notes_menu);
        let ip_menu_position = menu_pos!(ip_menu);
        let vpn_indicator_menu_position = menu_pos!(vpn_indicator_menu);
        let network_menu_position = menu_pos!(network_menu);
        let power_menu_position = menu_pos!(power_menu);
        let media_player_menu_position = menu_pos!(media_player_menu);
        let lyrics_menu_position = menu_pos!(lyrics_menu);
        let session_menu_position = menu_pos!(session_menu);
        let settings_menu_position = menu_pos!(settings_menu);
        let clock_widget: Widget = self.clock_menu.widget().clone().upcast();
        let clipboard_widget: Widget = self.clipboard_menu.widget().clone().upcast();
        let notification_menu_widget: Widget = self.notification_menu.widget().clone().upcast();
        let screenshot_menu_widget: Widget = self.screenshot_menu.widget().clone().upcast();
        let app_launcher_menu_widget: Widget = self.app_launcher_menu.widget().clone().upcast();
        let wallpaper_menu_widget: Widget = self.wallpaper_menu.widget().clone().upcast();
        let screenshare_menu_widget: Widget = self.screenshare_menu.widget().clone().upcast();
        let ufw_menu_widget: Widget = self.ufw_menu.widget().clone().upcast();
        let privacy_menu_widget: Widget = self.privacy_menu.widget().clone().upcast();
        let privacy_menu_position = mshell_config::config_manager::config_manager()
            .config()
            .menus()
            .privacy_menu()
            .position()
            .get();
        // Bluetooth menu position read directly from config (skip
        // the 19-arg RepositionMenus signature — defaults work).
        let bluetooth_menu_widget: Widget = self.bluetooth_menu.widget().clone().upcast();
        let bluetooth_menu_position = mshell_config::config_manager::config_manager()
            .config()
            .menus()
            .bluetooth_menu()
            .position()
            .get();
        let cpu_dashboard_menu_widget: Widget = self.cpu_dashboard_menu.widget().clone().upcast();
        let cpu_dashboard_menu_position = mshell_config::config_manager::config_manager()
            .config()
            .menus()
            .cpu_dashboard_menu()
            .position()
            .get();
        let audio_dashboard_menu_widget: Widget =
            self.audio_dashboard_menu.widget().clone().upcast();
        let audio_dashboard_menu_position = mshell_config::config_manager::config_manager()
            .config()
            .menus()
            .audio_dashboard_menu()
            .position()
            .get();
        let system_update_menu_widget: Widget = self.system_update_menu.widget().clone().upcast();
        let system_update_menu_position = mshell_config::config_manager::config_manager()
            .config()
            .menus()
            .system_update_menu()
            .position()
            .get();
        let valent_menu_widget: Widget = self.valent_menu.widget().clone().upcast();
        let valent_menu_position = mshell_config::config_manager::config_manager()
            .config()
            .menus()
            .valent_menu()
            .position()
            .get();
        let weather_menu_widget: Widget = self.weather_menu.widget().clone().upcast();
        let weather_menu_position = mshell_config::config_manager::config_manager()
            .config()
            .menus()
            .weather_menu()
            .position()
            .get();
        let keep_awake_menu_widget: Widget = self.keep_awake_menu.widget().clone().upcast();
        let keep_awake_menu_position = mshell_config::config_manager::config_manager()
            .config()
            .menus()
            .keep_awake_menu()
            .position()
            .get();
        let twilight_menu_widget: Widget = self.twilight_menu.widget().clone().upcast();
        let twilight_menu_position = mshell_config::config_manager::config_manager()
            .config()
            .menus()
            .twilight_menu()
            .position()
            .get();
        let keybinds_menu_widget: Widget = self.keybinds_menu.widget().clone().upcast();
        let keybinds_menu_position = mshell_config::config_manager::config_manager()
            .config()
            .menus()
            .keybinds_menu()
            .position()
            .get();
        let alarmclock_menu_widget: Widget = self.alarmclock_menu.widget().clone().upcast();
        let alarmclock_menu_position = mshell_config::config_manager::config_manager()
            .config()
            .menus()
            .alarmclock_menu()
            .position()
            .get();
        let dock_menu_widget: Widget = self.dock_menu.widget().clone().upcast();
        let dock_menu_position = mshell_config::config_manager::config_manager()
            .config()
            .menus()
            .dock_menu()
            .position()
            .get();
        let control_center_menu_widget: Widget = self.control_center_menu.widget().clone().upcast();
        let control_center_menu_position = mshell_config::config_manager::config_manager()
            .config()
            .menus()
            .control_center_menu()
            .position()
            .get();
        let ssh_menu_widget: Widget = self.ssh_menu.widget().clone().upcast();
        let ssh_menu_position = mshell_config::config_manager::config_manager()
            .config()
            .menus()
            .ssh_menu()
            .position()
            .get();
        let dns_menu_widget: Widget = self.dns_menu.widget().clone().upcast();
        let vpn_menu_widget: Widget = self.vpn_menu.widget().clone().upcast();
        let vpn_menu_position = mshell_config::config_manager::config_manager()
            .config()
            .menus()
            .vpn_menu()
            .position()
            .get();
        let ai_menu_widget: Widget = self.ai_menu.widget().clone().upcast();
        let ai_menu_position = mshell_config::config_manager::config_manager()
            .config()
            .menus()
            .ai_menu()
            .position()
            .get();
        let podman_menu_widget: Widget = self.podman_menu.widget().clone().upcast();
        let notes_menu_widget: Widget = self.notes_menu.widget().clone().upcast();
        let plugin_panel_menu_widget: Widget = self.plugin_panel_menu.widget().clone().upcast();
        let plugin_panel_menu_position = mshell_config::config_manager::config_manager()
            .config()
            .menus()
            .plugin_panel_menu()
            .position()
            .get();
        let ip_menu_widget: Widget = self.ip_menu.widget().clone().upcast();
        let vpn_indicator_menu_widget: Widget = self.vpn_indicator_menu.widget().clone().upcast();
        let network_menu_widget: Widget = self.network_menu.widget().clone().upcast();
        let power_menu_widget: Widget = self.power_menu.widget().clone().upcast();
        let media_player_menu_widget: Widget = self.media_player_menu.widget().clone().upcast();
        let lyrics_menu_widget: Widget = self.lyrics_menu.widget().clone().upcast();
        let session_menu_widget: Widget = self.session_menu.widget().clone().upcast();
        // Settings is built lazily (`ensure_settings_built`); only attach
        // it once it exists. `None` until the user first opens it.
        let settings_menu_widget: Option<Widget> = self
            .settings_menu
            .as_ref()
            .map(|c| c.widget().clone().upcast());

        // Snapshot which child each region's stack currently shows. The
        // `remove_all` + re-add below would otherwise leave every stack
        // defaulting to its first-added child (clock) — which flashes the
        // clock menu over whatever menu is actually open whenever a widget's
        // position/config change triggers a restack. We restore these at the
        // end so an open menu keeps showing the right child.
        let prev_visible = [
            widgets.left_stack.visible_child_name(),
            widgets.right_stack.visible_child_name(),
            widgets.top_stack.visible_child_name(),
            widgets.top_left_stack.visible_child_name(),
            widgets.top_right_stack.visible_child_name(),
            widgets.bottom_stack.visible_child_name(),
            widgets.bottom_left_stack.visible_child_name(),
            widgets.bottom_right_stack.visible_child_name(),
        ];

        widgets.left_stack.remove_all();
        widgets.right_stack.remove_all();
        widgets.top_stack.remove_all();
        widgets.top_left_stack.remove_all();
        widgets.top_right_stack.remove_all();
        widgets.bottom_stack.remove_all();
        widgets.bottom_left_stack.remove_all();
        widgets.bottom_right_stack.remove_all();

        Self::add_to_stack(widgets, &clock_widget, CLOCK_MENU, &clock_menu_position);
        Self::add_to_stack(
            widgets,
            &clipboard_widget,
            CLIPBOARD_MENU,
            &clipboard_menu_position,
        );
        Self::add_to_stack(
            widgets,
            &notification_menu_widget,
            NOTIFICATION_MENU,
            &notification_menu_position,
        );
        Self::add_to_stack(
            widgets,
            &screenshot_menu_widget,
            SCREENSHOT_MENU,
            &screenshot_menu_position,
        );
        Self::add_to_stack(
            widgets,
            &app_launcher_menu_widget,
            APP_LAUNCHER_MENU,
            &app_launcher_menu_position,
        );
        Self::add_to_stack(
            widgets,
            &wallpaper_menu_widget,
            WALLPAPER_MENU,
            &wallpaper_menu_position,
        );
        Self::add_to_stack(
            widgets,
            &screenshare_menu_widget,
            SCREENSHARE_MENU,
            &screenshare_menu_position,
        );
        Self::add_to_stack(widgets, &ufw_menu_widget, NUFW_MENU, &ufw_menu_position);
        Self::add_to_stack(
            widgets,
            &privacy_menu_widget,
            PRIVACY_MENU,
            &privacy_menu_position,
        );
        Self::add_to_stack(
            widgets,
            &bluetooth_menu_widget,
            BLUETOOTH_MENU,
            &bluetooth_menu_position,
        );
        Self::add_to_stack(
            widgets,
            &cpu_dashboard_menu_widget,
            CPU_DASHBOARD_MENU,
            &cpu_dashboard_menu_position,
        );
        Self::add_to_stack(
            widgets,
            &audio_dashboard_menu_widget,
            AUDIO_DASHBOARD_MENU,
            &audio_dashboard_menu_position,
        );
        Self::add_to_stack(
            widgets,
            &system_update_menu_widget,
            SYSTEM_UPDATE_MENU,
            &system_update_menu_position,
        );
        Self::add_to_stack(
            widgets,
            &valent_menu_widget,
            VALENT_MENU,
            &valent_menu_position,
        );
        Self::add_to_stack(
            widgets,
            &weather_menu_widget,
            WEATHER_MENU,
            &weather_menu_position,
        );
        Self::add_to_stack(
            widgets,
            &keep_awake_menu_widget,
            KEEP_AWAKE_MENU,
            &keep_awake_menu_position,
        );
        Self::add_to_stack(
            widgets,
            &twilight_menu_widget,
            TWILIGHT_MENU,
            &twilight_menu_position,
        );
        Self::add_to_stack(
            widgets,
            &keybinds_menu_widget,
            KEYBINDS_MENU,
            &keybinds_menu_position,
        );
        Self::add_to_stack(
            widgets,
            &alarmclock_menu_widget,
            ALARMCLOCK_MENU,
            &alarmclock_menu_position,
        );
        Self::add_to_stack(widgets, &dock_menu_widget, DOCK_MENU, &dock_menu_position);
        Self::add_to_stack(
            widgets,
            &control_center_menu_widget,
            CONTROL_CENTER_MENU,
            &control_center_menu_position,
        );
        Self::add_to_stack(widgets, &ssh_menu_widget, SSH_MENU, &ssh_menu_position);
        Self::add_to_stack(widgets, &dns_menu_widget, NDNS_MENU, &dns_menu_position);
        Self::add_to_stack(widgets, &vpn_menu_widget, NVPN_MENU, &vpn_menu_position);
        Self::add_to_stack(widgets, &ai_menu_widget, NAI_MENU, &ai_menu_position);
        Self::add_to_stack(
            widgets,
            &podman_menu_widget,
            NPODMAN_MENU,
            &podman_menu_position,
        );
        Self::add_to_stack(
            widgets,
            &notes_menu_widget,
            NNOTES_MENU,
            &notes_menu_position,
        );
        Self::add_to_stack(
            widgets,
            &plugin_panel_menu_widget,
            NPLUGIN_PANEL_MENU,
            &plugin_panel_menu_position,
        );
        Self::add_to_stack(widgets, &ip_menu_widget, NIP_MENU, &ip_menu_position);
        Self::add_to_stack(
            widgets,
            &vpn_indicator_menu_widget,
            NVPN_INDICATOR_MENU,
            &vpn_indicator_menu_position,
        );
        Self::add_to_stack(
            widgets,
            &network_menu_widget,
            NNETWORK_MENU,
            &network_menu_position,
        );
        Self::add_to_stack(
            widgets,
            &power_menu_widget,
            NPOWER_MENU,
            &power_menu_position,
        );
        Self::add_to_stack(
            widgets,
            &media_player_menu_widget,
            MEDIA_PLAYER_MENU,
            &media_player_menu_position,
        );
        Self::add_to_stack(
            widgets,
            &lyrics_menu_widget,
            LYRICS_MENU,
            &lyrics_menu_position,
        );
        Self::add_to_stack(
            widgets,
            &session_menu_widget,
            SESSION_MENU,
            &session_menu_position,
        );
        if let Some(settings_menu_widget) = &settings_menu_widget {
            Self::add_to_stack(
                widgets,
                settings_menu_widget,
                SETTINGS_MENU,
                &settings_menu_position,
            );
        }
        // The wizard shares the settings slot/position (both center
        // panels, mutually exclusive via toggle_menu by name).
        let wizard_menu_widget: Widget = self.wizard_menu.widget().clone().upcast();
        Self::add_to_stack(
            widgets,
            &wizard_menu_widget,
            WIZARD_MENU,
            &settings_menu_position,
        );
        // mdash — position read straight from config (newer pattern, like
        // cpu_dashboard / margo_layout); toggled via FrameInput::ToggleMenu(MenuId::Mdash).
        let mdash_menu_widget: Widget = self.mdash_menu.widget().clone().upcast();
        let mdash_menu_position = mshell_config::config_manager::config_manager()
            .config()
            .menus()
            .mdash_menu()
            .position()
            .get();
        Self::add_to_stack(
            widgets,
            &mdash_menu_widget,
            MDASH_MENU,
            &mdash_menu_position,
        );
        // Margo Layout menu — position read from config (the
        // Settings → Menus page exposes the knob). Bar pill output
        // cascades through `BarOutput::MargoLayoutClicked` to
        // `FrameInput::ToggleMenu(MenuId::MargoLayout)` which calls
        // `toggle_menu(MARGO_LAYOUT_MENU, …)` against the same
        // stack.
        let margo_layout_menu_widget: Widget = self.margo_layout_menu.widget().clone().upcast();
        let margo_layout_menu_position = mshell_config::config_manager::config_manager()
            .config()
            .menus()
            .margo_layout_menu()
            .position()
            .get();
        Self::add_to_stack(
            widgets,
            &margo_layout_menu_widget,
            MARGO_LAYOUT_MENU,
            &margo_layout_menu_position,
        );

        // Restore the pre-restack visible child for each region (see the
        // `prev_visible` snapshot above) so a re-add doesn't flash the
        // first-added menu (clock) over the menu that's actually open.
        let stacks = [
            &widgets.left_stack,
            &widgets.right_stack,
            &widgets.top_stack,
            &widgets.top_left_stack,
            &widgets.top_right_stack,
            &widgets.bottom_stack,
            &widgets.bottom_left_stack,
            &widgets.bottom_right_stack,
        ];
        for (stack, prev) in stacks.iter().zip(prev_visible.iter()) {
            if let Some(name) = prev
                && stack.child_by_name(name.as_str()).is_some()
            {
                stack.set_visible_child_full(name.as_str(), gtk::StackTransitionType::None);
            }
        }
    }

    fn add_to_stack(widgets: &FrameWidgets, widget: &Widget, name: &str, position: &Position) {
        match position {
            Position::Top => {
                widgets.top_stack.add_named(widget, Some(name));
            }
            Position::Bottom => {
                widgets.bottom_stack.add_named(widget, Some(name));
            }
            Position::Left => {
                widgets.left_stack.add_named(widget, Some(name));
            }
            Position::Right => {
                widgets.right_stack.add_named(widget, Some(name));
            }
            Position::TopLeft => {
                widgets.top_left_stack.add_named(widget, Some(name));
            }
            Position::TopRight => {
                widgets.top_right_stack.add_named(widget, Some(name));
            }
            Position::BottomLeft => {
                widgets.bottom_left_stack.add_named(widget, Some(name));
            }
            Position::BottomRight => {
                widgets.bottom_right_stack.add_named(widget, Some(name));
            }
        }
    }

    fn build_bar(sender: &ComponentSender<Self>, bar_type: BarType) -> Controller<BarModel> {
        BarModel::builder().launch(BarInit { bar_type }).forward(
            sender.input_sender(),
            move |msg| match msg {
                BarOutput::ReserveHeight(h) => FrameInput::SpacerReserve {
                    is_top: matches!(bar_type, BarType::Top),
                    height: h,
                },
                BarOutput::ClockClicked => FrameInput::ToggleMenu(MenuId::Clock),
                BarOutput::CatwalkClicked => FrameInput::ToggleMenu(MenuId::CpuDashboard),
                BarOutput::MdashClicked => FrameInput::ToggleMenu(MenuId::Mdash),
                BarOutput::ClipboardClicked => FrameInput::ToggleMenu(MenuId::Clipboard),
                BarOutput::NotificationsClicked => FrameInput::ToggleMenu(MenuId::Notification),
                BarOutput::ScreenshotClicked => FrameInput::ToggleMenu(MenuId::Screenshot),
                BarOutput::AppLauncherClicked => FrameInput::ToggleMenu(MenuId::AppLauncher),
                BarOutput::WallpaperClicked => FrameInput::ToggleMenu(MenuId::Wallpaper),
                BarOutput::UfwClicked => FrameInput::ToggleMenu(MenuId::Ufw),
                BarOutput::PrivacyClicked => FrameInput::ToggleMenu(MenuId::Privacy),
                BarOutput::BluetoothClicked => FrameInput::ToggleMenu(MenuId::Bluetooth),
                BarOutput::CpuDashboardClicked => FrameInput::ToggleMenu(MenuId::CpuDashboard),
                BarOutput::AudioDashboardClicked => FrameInput::ToggleMenu(MenuId::AudioDashboard),
                BarOutput::SystemUpdateClicked => FrameInput::ToggleMenu(MenuId::SystemUpdate),
                BarOutput::ValentClicked => FrameInput::ToggleMenu(MenuId::Valent),
                BarOutput::WeatherClicked => FrameInput::ToggleMenu(MenuId::Weather),
                BarOutput::KeepAwakeClicked => FrameInput::ToggleMenu(MenuId::KeepAwake),
                BarOutput::TwilightClicked => FrameInput::ToggleMenu(MenuId::Twilight),
                BarOutput::KeybindsClicked => FrameInput::ToggleMenu(MenuId::Keybinds),
                BarOutput::AlarmClockClicked => FrameInput::ToggleMenu(MenuId::AlarmClock),
                // The pill already set the pending-tab hint (crate::countdown);
                // opening the Alarm Clock menu lands on its Countdown tab.
                BarOutput::CountdownClicked => FrameInput::ToggleMenu(MenuId::AlarmClock),
                BarOutput::ControlCenterClicked => FrameInput::ToggleMenu(MenuId::ControlCenter),
                BarOutput::SshSessionsClicked => FrameInput::ToggleMenu(MenuId::SshSessions),
                BarOutput::VpnClicked => FrameInput::ToggleMenu(MenuId::Vpn),
                BarOutput::AiClicked => FrameInput::ToggleMenu(MenuId::Ai),
                BarOutput::DnsClicked => FrameInput::ToggleMenu(MenuId::Dns),
                BarOutput::PodmanClicked => FrameInput::ToggleMenu(MenuId::Podman),
                BarOutput::NotesClicked => FrameInput::ToggleMenu(MenuId::Notes),
                BarOutput::PluginPanelClicked {
                    name,
                    entry,
                    settings,
                    capabilities,
                    min_width,
                    max_height,
                } => FrameInput::ToggleWasmPluginPanel {
                    name,
                    entry,
                    settings,
                    capabilities,
                    min_width,
                    max_height,
                },
                BarOutput::PluginMenuClicked {
                    name,
                    rows,
                    min_width,
                    max_height,
                } => FrameInput::TogglePluginMenu {
                    name,
                    rows,
                    min_width,
                    max_height,
                },
                BarOutput::IpClicked => FrameInput::ToggleMenu(MenuId::Ip),
                BarOutput::VpnIndicatorClicked => FrameInput::ToggleMenu(MenuId::VpnIndicator),
                BarOutput::NetworkClicked => FrameInput::ToggleMenu(MenuId::Network),
                BarOutput::PowerClicked => FrameInput::ToggleMenu(MenuId::Power),
                BarOutput::MediaPlayerClicked => FrameInput::ToggleMenu(MenuId::MediaPlayer),
                BarOutput::LyricsClicked => FrameInput::ToggleMenu(MenuId::Lyrics),
                BarOutput::MargoLayoutClicked => FrameInput::ToggleMenu(MenuId::MargoLayout),
                BarOutput::CloseMenu => FrameInput::CloseMenus,
            },
        )
    }

    fn build_menu(sender: &ComponentSender<Self>, menu_type: MenuType) -> Controller<MenuModel> {
        MenuModel::builder()
            .launch(MenuInit { menu_type })
            .forward(sender.input_sender(), |msg| match msg {
                MenuOutput::CloseMenu => FrameInput::CloseMenus,
                MenuOutput::ToggleSessionMenu => FrameInput::ToggleMenu(MenuId::Session),
                MenuOutput::OpenAppLauncher => FrameInput::ToggleMenu(MenuId::AppLauncher),
            })
    }

    /// Apply a plugin's per-plugin panel layout (size + position) to the shared
    /// plugin-menu surface before showing it. Read **fresh** from the plugin
    /// store (keyed off the widget name) so a change just made in the gear takes
    /// effect — the bar pill may still hold the value it was built with. Size
    /// 0 = leave as-is; position re-anchors the menu between stacks.
    fn apply_plugin_layout(&mut self, widget_name: &str, widgets: &FrameWidgets) {
        let Some(key) = plugin_key_from_widget(widget_name) else {
            return;
        };
        let layout = mshell_plugins::PluginStore::new().load_state().panel(&key);
        if layout.min_width > 0 {
            self.plugin_panel_menu
                .sender()
                .send(MenuInput::SetMinimumWidth(layout.min_width))
                .ok();
        }
        if layout.max_height > 0 {
            self.plugin_panel_menu
                .sender()
                .send(MenuInput::SetMaximumHeight(layout.max_height))
                .ok();
        }
        let new_pos = position_from_kebab(&layout.position);
        if new_pos != self.plugin_panel_position {
            let widget: Widget = self.plugin_panel_menu.widget().clone().upcast();
            Self::remove_from_stack(widgets, &widget, &self.plugin_panel_position);
            Self::add_to_stack(widgets, &widget, NPLUGIN_PANEL_MENU, &new_pos);
            self.plugin_panel_position = new_pos;
        }
    }

    /// Inverse of [`add_to_stack`](Self::add_to_stack): detach the menu from the
    /// stack for `position` so it can be re-anchored elsewhere.
    fn remove_from_stack(widgets: &FrameWidgets, widget: &Widget, position: &Position) {
        match position {
            Position::Top => widgets.top_stack.remove(widget),
            Position::Bottom => widgets.bottom_stack.remove(widget),
            Position::Left => widgets.left_stack.remove(widget),
            Position::Right => widgets.right_stack.remove(widget),
            Position::TopLeft => widgets.top_left_stack.remove(widget),
            Position::TopRight => widgets.top_right_stack.remove(widget),
            Position::BottomLeft => widgets.bottom_left_stack.remove(widget),
            Position::BottomRight => widgets.bottom_right_stack.remove(widget),
        }
    }

    /// Build the content widget for a declarative plugin menu: a vertical list
    /// of command-row buttons (icon + label). Clicking a row runs its `exec`
    /// and closes the menu. Hosted in the first-class plugin menu surface.
    fn build_plugin_menu_content(rows: &[CustomMenuRow], sender: &ComponentSender<Self>) -> Widget {
        let list = gtk::Box::new(gtk::Orientation::Vertical, 4);
        list.add_css_class("plugin-menu-list");
        for row in rows {
            let label = row.label.trim();
            if label.is_empty() && row.exec.trim().is_empty() {
                continue;
            }
            let btn = gtk::Button::new();
            btn.add_css_class("plugin-menu-row");
            if row.severity.trim() == "danger" {
                btn.add_css_class("plugin-menu-row-danger");
            }
            btn.set_has_frame(false);
            let hb = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            if !row.icon.trim().is_empty() {
                let img = gtk::Image::from_icon_name(row.icon.trim());
                img.set_pixel_size(16);
                hb.append(&img);
            }
            let text = if label.is_empty() {
                row.exec.trim()
            } else {
                label
            };
            let lbl = gtk::Label::new(Some(text));
            lbl.set_halign(gtk::Align::Start);
            lbl.set_hexpand(true);
            hb.append(&lbl);
            btn.set_child(Some(&hb));
            let cmd = row.exec.clone();
            let sender = sender.clone();
            btn.connect_clicked(move |_| {
                run_plugin_cmd(&cmd);
                sender.input(FrameInput::CloseMenus);
            });
            list.append(&btn);
        }
        list.upcast()
    }
}

/// The plugin's composite key from a derived widget name. Names look like
/// `plugin:<composite-key>:<widget-key>` (the composite key may itself contain
/// a `:` for custom sources), so take everything between the prefix and the
/// last `:`.
fn plugin_key_from_widget(name: &str) -> Option<String> {
    let rest = name.strip_prefix("plugin:")?;
    let (key, _widget) = rest.rsplit_once(':')?;
    Some(key.to_string())
}

/// Map a stored kebab position string to a menu anchor (default top-right).
fn position_from_kebab(s: &str) -> Position {
    match s.trim() {
        "left" => Position::Left,
        "right" => Position::Right,
        "top" => Position::Top,
        "top-left" => Position::TopLeft,
        "bottom" => Position::Bottom,
        "bottom-left" => Position::BottomLeft,
        "bottom-right" => Position::BottomRight,
        _ => Position::TopRight,
    }
}

/// Fire-and-forget a plugin menu row's `sh -c` command (reaped to avoid
/// zombies).
fn run_plugin_cmd(cmd: &str) {
    let cmd = cmd.trim().to_string();
    if cmd.is_empty() {
        return;
    }
    relm4::spawn(async move {
        let _ = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(&cmd)
            .status()
            .await;
    });
}

impl Drop for Frame {
    fn drop(&mut self) {
        self.top_spacer.widget().destroy();
        self.bottom_spacer.widget().destroy();
    }
}
