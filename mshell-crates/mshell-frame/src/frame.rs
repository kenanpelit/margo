use crate::bars::bar::{BarInit, BarInput, BarModel, BarOutput, BarType};
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use mshell_common::box_with_resize::BoxWithResize;
use mshell_common::diagonal_revealer::DiagonalRevealer;
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
use crate::menus::menu_widgets::mshelldash::mshelldash::{
    MShellDashInit, MShellDashInput, MShellDashModel,
};

const CLOCK_MENU: &str = "clock";
const CLIPBOARD_MENU: &str = "clipboard";
const APP_LAUNCHER_MENU: &str = "app_launcher";
const SCREENSHOT_MENU: &str = "screenshot";
const NOTIFICATION_MENU: &str = "notification";
const WALLPAPER_MENU: &str = "wallpaper";
const SCREENSHARE_MENU: &str = "screenshare";
const NUFW_MENU: &str = "ufw";
const BLUETOOTH_MENU: &str = "bluetooth";
const CPU_DASHBOARD_MENU: &str = "cpu_dashboard";
const AUDIO_DASHBOARD_MENU: &str = "audio_dashboard";
const SYSTEM_UPDATE_MENU: &str = "system_update";
const VALENT_MENU: &str = "valent";
const WEATHER_MENU: &str = "weather";
const KEEP_AWAKE_MENU: &str = "keep_awake";
const TWILIGHT_MENU: &str = "twilight";
const KEYBINDS_MENU: &str = "keybinds";
const SSH_MENU: &str = "ssh_sessions";
const NDNS_MENU: &str = "dns";
const NPODMAN_MENU: &str = "podman";
const NNOTES_MENU: &str = "notes";
const NIP_MENU: &str = "ip";
const NNETWORK_MENU: &str = "network";
const NPOWER_MENU: &str = "power";
const MEDIA_PLAYER_MENU: &str = "media_player";
const SESSION_MENU: &str = "session";
const SETTINGS_MENU: &str = "settings";
const DASHBOARD_MENU: &str = "dashboard";
const MARGO_LAYOUT_MENU: &str = "margo_layout";
const MSHELLDASH_MENU: &str = "mshelldash";

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
    top_spacer: Controller<FrameSpacerModel>,
    bottom_spacer: Controller<FrameSpacerModel>,
    clock_menu: Controller<MenuModel>,
    clipboard_menu: Controller<MenuModel>,
    notification_menu: Controller<MenuModel>,
    screenshot_menu: Controller<MenuModel>,
    app_launcher_menu: Controller<MenuModel>,
    wallpaper_menu: Controller<MenuModel>,
    screenshare_menu: Controller<MenuModel>,
    ufw_menu: Controller<MenuModel>,
    bluetooth_menu: Controller<MenuModel>,
    cpu_dashboard_menu: Controller<MenuModel>,
    audio_dashboard_menu: Controller<MenuModel>,
    system_update_menu: Controller<MenuModel>,
    valent_menu: Controller<MenuModel>,
    weather_menu: Controller<MenuModel>,
    keep_awake_menu: Controller<MenuModel>,
    twilight_menu: Controller<MenuModel>,
    keybinds_menu: Controller<MenuModel>,
    ssh_menu: Controller<MenuModel>,
    dns_menu: Controller<MenuModel>,
    podman_menu: Controller<MenuModel>,
    notes_menu: Controller<MenuModel>,
    ip_menu: Controller<MenuModel>,
    network_menu: Controller<MenuModel>,
    power_menu: Controller<MenuModel>,
    media_player_menu: Controller<MenuModel>,
    session_menu: Controller<MenuModel>,
    /// Settings panel — uses its own dedicated model (not
    /// `MenuModel`) because its content is a custom sidebar +
    /// stack rather than the generic menu-widget pipeline.
    settings_menu: Controller<mshell_settings::SettingsWindowModel>,
    dashboard_menu: Controller<MenuModel>,
    margo_layout_menu: Controller<MenuModel>,
    /// mshelldash — standalone tabbed dashboard (own model, like
    /// `settings_menu`; not part of the generic `MenuModel` pipeline).
    mshelldash_menu: Controller<MShellDashModel>,
    /// Pending keyboard-mode switch held inside the 90 ms debounce
    /// window. Replaced on every `sync_keyboard_mode` call; the
    /// timer reads whatever value was last written.
    pending_kbd_mode: std::rc::Rc<std::cell::RefCell<Option<gtk4_layer_shell::KeyboardMode>>>,
    pending_kbd_mode_timeout: std::rc::Rc<std::cell::RefCell<Option<gtk::glib::SourceId>>>,
    _effects: EffectScope,
}

#[derive(Debug)]
pub enum FrameInput {
    SetDrawFrame(bool),
    QueueFrameRedraw,
    SetLeftMenuExpansionType(VerticalMenuExpansion),
    SetRightMenuExpansionType(VerticalMenuExpansion),
    RepositionMenus(
        Position, Position, Position, Position, Position, Position, Position, Position, Position,
        Position, Position, Position, Position, Position, Position, Position, Position,
        // dashboard_menu_position
        Position,
    ),
    ToggleClockMenu,
    ToggleClipboardMenu,
    ToggleNotificationMenu,
    ToggleScreenshotMenu,
    ToggleAppLauncherMenu,
    /// Same as `ToggleAppLauncherMenu` but also forwards a
    /// `SelectCategory(tab)` into the underlying
    /// `AppLauncherModel` once the menu is open. Bridges
    /// `mshellctl menu app-launcher --tab Run` to the runtime's
    /// existing category-cycle path.
    ToggleAppLauncherMenuWithTab(String),
    ToggleWallpaperMenu,
    ToggleUfwMenu,
    ToggleBluetoothMenu,
    ToggleCpuDashboardMenu,
    ToggleAudioDashboardMenu,
    ToggleSystemUpdateMenu,
    ToggleValentMenu,
    ToggleWeatherMenu,
    ToggleKeepAwakeMenu,
    ToggleTwilightMenu,
    ToggleKeybindsMenu,
    ToggleSshSessionsMenu,
    ToggleDnsMenu,
    TogglePodmanMenu,
    ToggleNotesMenu,
    ToggleIpMenu,
    ToggleNetworkMenu,
    TogglePowerMenu,
    ToggleMediaPlayerMenu,
    ToggleSessionMenu,
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
    ToggleDashboardMenu,
    /// Open / close the standalone tabbed mshelldash surface. The
    /// `String` is an optional target tab name ("" = leave current).
    ToggleMShellDashMenu(String),
    /// Open / close the Margo layout switcher menu (in-frame
    /// replacement for the legacy bar popover).
    ToggleMargoLayoutMenu,
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
            monitor = params.monitor.connector().unwrap().to_string(),
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
            .trigger(&gtk::KeyvalTrigger::new(gdk::Key::Escape, gdk::ModifierType::empty()))
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
        let ufw_menu = Self::build_menu(&sender, MenuType::Ufw);
        let bluetooth_menu = Self::build_menu(&sender, MenuType::Bluetooth);
        let cpu_dashboard_menu = Self::build_menu(&sender, MenuType::CpuDashboard);
        let audio_dashboard_menu = Self::build_menu(&sender, MenuType::AudioDashboard);
        let system_update_menu = Self::build_menu(&sender, MenuType::SystemUpdate);
        let valent_menu = Self::build_menu(&sender, MenuType::Valent);
        let weather_menu = Self::build_menu(&sender, MenuType::Weather);
        let keep_awake_menu = Self::build_menu(&sender, MenuType::KeepAwake);
        let twilight_menu = Self::build_menu(&sender, MenuType::Twilight);
        let keybinds_menu = Self::build_menu(&sender, MenuType::Keybinds);
        let ssh_menu = Self::build_menu(&sender, MenuType::SshSessions);
        let dns_menu = Self::build_menu(&sender, MenuType::Dns);
        let podman_menu = Self::build_menu(&sender, MenuType::Podman);
        let notes_menu = Self::build_menu(&sender, MenuType::Notes);
        let ip_menu = Self::build_menu(&sender, MenuType::Ip);
        let network_menu = Self::build_menu(&sender, MenuType::Network);
        let power_menu = Self::build_menu(&sender, MenuType::Power);
        let media_player_menu = Self::build_menu(&sender, MenuType::MediaPlayer);
        let session_menu = Self::build_menu(&sender, MenuType::Session);
        let dashboard_menu = Self::build_menu(&sender, MenuType::Dashboard);
        let margo_layout_menu = Self::build_menu(&sender, MenuType::MargoLayout);

        // Settings doesn't go through `build_menu` because its
        // content isn't a list of `MenuWidget`s — it's a custom
        // sidebar + stack laid out by `SettingsWindowModel`.
        // Build one controller per Frame; the shell-level
        // dispatcher registers the toggle backend that resolves
        // active monitor and emits `ToggleSettingsMenu` to the
        // right Frame.
        let settings_menu = mshell_settings::SettingsWindowModel::builder()
            .launch(mshell_settings::SettingsWindowInit {
                monitor: Some(params.monitor.clone()),
            })
            .detach();

        // mshelldash — same custom-model pattern as Settings: own
        // controller per Frame, toggled by name via `toggle_menu`.
        let mshelldash_menu = MShellDashModel::builder()
            .launch(MShellDashInit {})
            .detach();

        let mut effects = EffectScope::new();

        let config = base_config.clone();
        let sender_clone = sender.clone();
        effects.push(move |_| {
            let config = config.clone();
            let enable_frame = config.bars().frame().enable_frame().get();
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
            let config = menu_config.clone();
            let clock_menu_position = config.menus().clock_menu().position().get();
            let config = menu_config.clone();
            let clipboard_menu_position = config.menus().clipboard_menu().position().get();
            let config = menu_config.clone();
            let notification_menu_position = config.menus().notification_menu().position().get();
            let config = menu_config.clone();
            let screenshot_menu_position = config.menus().screenshot_menu().position().get();
            let config = menu_config.clone();
            let app_launcher_menu_position = config.menus().app_launcher_menu().position().get();
            let config = menu_config.clone();
            let wallpaper_menu_position = config.menus().wallpaper_menu().position().get();
            let config = menu_config.clone();
            let screenshare_menu_position = config.menus().screenshare_menu().position().get();
            let config = menu_config.clone();
            let ufw_menu_position = config.menus().ufw_menu().position().get();
            let config = menu_config.clone();
            let dns_menu_position = config.menus().dns_menu().position().get();
            let config = menu_config.clone();
            let podman_menu_position = config.menus().podman_menu().position().get();
            let config = menu_config.clone();
            let notes_menu_position = config.menus().notes_menu().position().get();
            let config = menu_config.clone();
            let ip_menu_position = config.menus().ip_menu().position().get();
            let config = menu_config.clone();
            let network_menu_position = config.menus().network_menu().position().get();
            let config = menu_config.clone();
            let power_menu_position = config.menus().power_menu().position().get();
            let config = menu_config.clone();
            let media_player_menu_position =
                config.menus().media_player_menu().position().get();
            let config = menu_config.clone();
            let session_menu_position = config.menus().session_menu().position().get();
            let config = menu_config.clone();
            let settings_menu_position = config.menus().settings_menu().position().get();
            let config = menu_config.clone();
            let dashboard_menu_position = config.menus().dashboard_menu().position().get();
            // These menus are placed by `apply_left_and_right_side_children`
            // reading their position straight from config (not passed as a
            // RepositionMenus arg), so subscribe the effect to them here too
            // — otherwise moving them in Settings doesn't re-fire this effect
            // and the menu stays put until restart.
            let config = menu_config.clone();
            let _ = config.menus().cpu_dashboard_menu().position().get();
            let config = menu_config.clone();
            let _ = config.menus().audio_dashboard_menu().position().get();
            let config = menu_config.clone();
            let _ = config.menus().bluetooth_menu().position().get();
            let config = menu_config.clone();
            let _ = config.menus().system_update_menu().position().get();
            let config = menu_config.clone();
            let _ = config.menus().valent_menu().position().get();
            let config = menu_config.clone();
            let _ = config.menus().weather_menu().position().get();
            let config = menu_config.clone();
            let _ = config.menus().keep_awake_menu().position().get();
            let config = menu_config.clone();
            let _ = config.menus().twilight_menu().position().get();
            let config = menu_config.clone();
            let _ = config.menus().keybinds_menu().position().get();
            let config = menu_config.clone();
            let _ = config.menus().ssh_menu().position().get();
            let config = menu_config.clone();
            let _ = config.menus().margo_layout_menu().position().get();
            sender_clone.input(FrameInput::RepositionMenus(
                clock_menu_position,
                clipboard_menu_position,
                notification_menu_position,
                screenshot_menu_position,
                app_launcher_menu_position,
                wallpaper_menu_position,
                screenshare_menu_position,
                ufw_menu_position,
                dns_menu_position,
                podman_menu_position,
                notes_menu_position,
                ip_menu_position,
                network_menu_position,
                power_menu_position,
                media_player_menu_position,
                session_menu_position,
                settings_menu_position,
                dashboard_menu_position,
            ));
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
            top_spacer,
            bottom_spacer,
            clock_menu: calendar_menu,
            clipboard_menu,
            notification_menu,
            screenshot_menu,
            app_launcher_menu,
            wallpaper_menu,
            screenshare_menu,
            ufw_menu,
            bluetooth_menu,
            cpu_dashboard_menu,
            audio_dashboard_menu,
            system_update_menu,
            valent_menu,
            weather_menu,
            keep_awake_menu,
            twilight_menu,
            keybinds_menu,
            ssh_menu,
            dns_menu,
            podman_menu,
            notes_menu,
            ip_menu,
            network_menu,
            power_menu,
            media_player_menu,
            session_menu,
            settings_menu,
            dashboard_menu,
            margo_layout_menu,
            mshelldash_menu,
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
            FrameInput::RepositionMenus(
                clock_menu_position,
                clipboard_menu_position,
                notification_menu_position,
                screenshot_menu_position,
                app_launcher_menu_position,
                wallpaper_menu_position,
                screenshare_menu_position,
                ufw_menu_position,
                dns_menu_position,
                podman_menu_position,
                notes_menu_position,
                ip_menu_position,
                network_menu_position,
                power_menu_position,
                media_player_menu_position,
                session_menu_position,
                settings_menu_position,
                dashboard_menu_position,
            ) => {
                sender.input(FrameInput::CloseMenus);
                self.apply_left_and_right_side_children(
                    widgets,
                    clock_menu_position,
                    clipboard_menu_position,
                    notification_menu_position,
                    screenshot_menu_position,
                    app_launcher_menu_position,
                    wallpaper_menu_position,
                    screenshare_menu_position,
                    ufw_menu_position,
                    dns_menu_position,
                    podman_menu_position,
                    notes_menu_position,
                    ip_menu_position,
                    network_menu_position,
                    power_menu_position,
                    media_player_menu_position,
                    session_menu_position,
                    settings_menu_position,
                    dashboard_menu_position,
                );
            }
            FrameInput::ToggleClockMenu => {
                self.toggle_menu(CLOCK_MENU, widgets);
                self.sync_keyboard_mode(root);
            }
            FrameInput::ToggleClipboardMenu => {
                self.toggle_menu(CLIPBOARD_MENU, widgets);
                self.sync_keyboard_mode(root);
            }
            FrameInput::ToggleNotificationMenu => {
                self.toggle_menu(NOTIFICATION_MENU, widgets);
                self.sync_keyboard_mode(root);
            }
            FrameInput::ToggleScreenshotMenu => {
                self.toggle_menu(SCREENSHOT_MENU, widgets);
                self.sync_keyboard_mode(root);
            }
            FrameInput::ToggleAppLauncherMenu => {
                self.toggle_menu(APP_LAUNCHER_MENU, widgets);
                self.sync_keyboard_mode(root);
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
            FrameInput::ToggleWallpaperMenu => {
                self.toggle_menu(WALLPAPER_MENU, widgets);
                self.sync_keyboard_mode(root);
            }
            FrameInput::ToggleUfwMenu => {
                self.toggle_menu(NUFW_MENU, widgets);
                self.sync_keyboard_mode(root);
            }
            FrameInput::ToggleBluetoothMenu => {
                self.toggle_menu(BLUETOOTH_MENU, widgets);
                self.sync_keyboard_mode(root);
            }
            FrameInput::ToggleCpuDashboardMenu => {
                self.toggle_menu(CPU_DASHBOARD_MENU, widgets);
                self.sync_keyboard_mode(root);
            }
            FrameInput::ToggleAudioDashboardMenu => {
                self.toggle_menu(AUDIO_DASHBOARD_MENU, widgets);
                self.sync_keyboard_mode(root);
            }
            FrameInput::ToggleSystemUpdateMenu => {
                self.toggle_menu(SYSTEM_UPDATE_MENU, widgets);
                self.sync_keyboard_mode(root);
            }
            FrameInput::ToggleValentMenu => {
                self.toggle_menu(VALENT_MENU, widgets);
                self.sync_keyboard_mode(root);
            }
            FrameInput::ToggleWeatherMenu => {
                self.toggle_menu(WEATHER_MENU, widgets);
                self.sync_keyboard_mode(root);
            }
            FrameInput::ToggleKeepAwakeMenu => {
                self.toggle_menu(KEEP_AWAKE_MENU, widgets);
                self.sync_keyboard_mode(root);
            }
            FrameInput::ToggleTwilightMenu => {
                self.toggle_menu(TWILIGHT_MENU, widgets);
                self.sync_keyboard_mode(root);
            }
            FrameInput::ToggleKeybindsMenu => {
                self.toggle_menu(KEYBINDS_MENU, widgets);
                self.sync_keyboard_mode(root);
            }
            FrameInput::ToggleSshSessionsMenu => {
                self.toggle_menu(SSH_MENU, widgets);
                self.sync_keyboard_mode(root);
            }
            FrameInput::ToggleDnsMenu => {
                self.toggle_menu(NDNS_MENU, widgets);
                self.sync_keyboard_mode(root);
            }
            FrameInput::TogglePodmanMenu => {
                self.toggle_menu(NPODMAN_MENU, widgets);
                self.sync_keyboard_mode(root);
            }
            FrameInput::ToggleNotesMenu => {
                self.toggle_menu(NNOTES_MENU, widgets);
                self.sync_keyboard_mode(root);
            }
            FrameInput::ToggleIpMenu => {
                self.toggle_menu(NIP_MENU, widgets);
                self.sync_keyboard_mode(root);
            }
            FrameInput::ToggleNetworkMenu => {
                self.toggle_menu(NNETWORK_MENU, widgets);
                self.sync_keyboard_mode(root);
            }
            FrameInput::TogglePowerMenu => {
                self.toggle_menu(NPOWER_MENU, widgets);
                self.sync_keyboard_mode(root);
            }
            FrameInput::ToggleMediaPlayerMenu => {
                self.toggle_menu(MEDIA_PLAYER_MENU, widgets);
                self.sync_keyboard_mode(root);
            }
            FrameInput::ToggleSessionMenu => {
                self.toggle_menu(SESSION_MENU, widgets);
                self.sync_keyboard_mode(root);
            }
            FrameInput::ToggleSettingsMenu => {
                self.toggle_menu(SETTINGS_MENU, widgets);
                self.sync_keyboard_mode(root);
            }
            FrameInput::OpenSettingsAtSection(section) => {
                // Ensure Settings is visible — toggle if currently
                // hidden. Skip the toggle when already visible so
                // re-issuing the same section nav doesn't close
                // the panel.
                if !self.is_menu_visible_now(SETTINGS_MENU, widgets) {
                    self.toggle_menu(SETTINGS_MENU, widgets);
                    self.sync_keyboard_mode(root);
                }
                let _ = self
                    .settings_menu
                    .sender()
                    .send(mshell_settings::SettingsWindowInput::ActivateSection(section));
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
            FrameInput::ToggleDashboardMenu => {
                self.toggle_menu(DASHBOARD_MENU, widgets);
                self.sync_keyboard_mode(root);
            }
            FrameInput::ToggleMShellDashMenu(tab) => {
                if !tab.is_empty() {
                    self.mshelldash_menu.emit(MShellDashInput::SelectTabName(tab));
                }
                self.toggle_menu(MSHELLDASH_MENU, widgets);
                self.sync_keyboard_mode(root);
            }
            FrameInput::ToggleMargoLayoutMenu => {
                self.toggle_menu(MARGO_LAYOUT_MENU, widgets);
                self.sync_keyboard_mode(root);
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
        let id = gtk::glib::timeout_add_local_once(
            std::time::Duration::from_millis(90),
            move || {
                *pending_timeout.borrow_mut() = None;
                let Some(mode) = pending_mode.borrow_mut().take() else {
                    return;
                };
                let Some(root) = root_weak.upgrade() else {
                    return;
                };
                root.set_keyboard_mode(mode);
                tracing::debug!(?mode, "frame: sync_keyboard_mode (applied)");
            },
        );
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
            *revealed
                && stack.visible_child_name().map(|n| n.to_string()) == Some(name.to_string())
        })
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
                    widgets.left_stack.set_visible_child_name(name);
                    self.left_revealed = true;
                }
            }
        } else if in_right {
            if let Some(visible) = widgets.right_stack.visible_child_name() {
                if visible.as_str() == name {
                    self.right_revealed = !right_revealed;
                    now_visible = self.right_revealed;
                } else {
                    widgets.right_stack.set_visible_child_name(name);
                    self.right_revealed = true;
                }
            }
        } else if in_top {
            if let Some(visible) = widgets.top_stack.visible_child_name() {
                if visible.as_str() == name {
                    self.top_revealed = !top_revealed;
                    now_visible = self.top_revealed;
                } else {
                    widgets.top_stack.set_visible_child_name(name);
                    self.top_revealed = true;
                }
            }
        } else if in_top_left {
            if let Some(visible) = widgets.top_left_stack.visible_child_name() {
                if visible.as_str() == name {
                    self.top_left_revealed = !top_left_revealed;
                    now_visible = self.top_left_revealed;
                } else {
                    widgets.top_left_stack.set_visible_child_name(name);
                    self.top_left_revealed = true;
                }
            }
        } else if in_top_right {
            if let Some(visible) = widgets.top_right_stack.visible_child_name() {
                if visible.as_str() == name {
                    self.top_right_revealed = !top_right_revealed;
                    now_visible = self.top_right_revealed;
                } else {
                    widgets.top_right_stack.set_visible_child_name(name);
                    self.top_right_revealed = true;
                }
            }
        } else if in_bottom {
            if let Some(visible) = widgets.bottom_stack.visible_child_name() {
                if visible.as_str() == name {
                    self.bottom_revealed = !bottom_revealed;
                    now_visible = self.bottom_revealed;
                } else {
                    widgets.bottom_stack.set_visible_child_name(name);
                    self.bottom_revealed = true;
                }
            }
        } else if in_bottom_left {
            if let Some(visible) = widgets.bottom_left_stack.visible_child_name() {
                if visible.as_str() == name {
                    self.bottom_left_revealed = !bottom_left_revealed;
                    now_visible = self.bottom_left_revealed;
                } else {
                    widgets.bottom_left_stack.set_visible_child_name(name);
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
                widgets.bottom_right_stack.set_visible_child_name(name);
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
    }

    // Can't use sender for this.  Must queue redraw in the callback.  Otherwise, there is a slight
    // delay and the frame isn't draw immediately.
    fn attach_resize_listeners(&self, widgets: &FrameWidgets) {
        let frame_widget = widgets.frame_draw_widget.clone();
        let top_sender = self.top_spacer.sender().clone();
        widgets
            .top_bar_container
            .connect_local("resized", false, move |values| {
                let height = values[2].get::<i32>().expect("height i32");
                frame_widget.update_style(|s| s.top_thickness = height as f64);
                let _ = top_sender.send(FrameSpacerInput::HeightUpdated(height));
                None
            });

        let frame_widget = widgets.frame_draw_widget.clone();
        let bottom_sender = self.bottom_spacer.sender().clone();
        widgets
            .bottom_bar_container
            .connect_local("resized", false, move |values| {
                let height = values[2].get::<i32>().expect("height i32");
                frame_widget.update_style(|s| s.bottom_thickness = height as f64);
                let _ = bottom_sender.send(FrameSpacerInput::HeightUpdated(height));
                None
            });

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

    fn apply_left_and_right_side_children(
        &self,
        widgets: &FrameWidgets,
        clock_menu_position: Position,
        clipboard_menu_position: Position,
        notification_menu_position: Position,
        screenshot_menu_position: Position,
        app_launcher_menu_position: Position,
        wallpaper_menu_position: Position,
        screenshare_menu_position: Position,
        ufw_menu_position: Position,
        dns_menu_position: Position,
        podman_menu_position: Position,
        notes_menu_position: Position,
        ip_menu_position: Position,
        network_menu_position: Position,
        power_menu_position: Position,
        media_player_menu_position: Position,
        session_menu_position: Position,
        settings_menu_position: Position,
        dashboard_menu_position: Position,
    ) {
        let clock_widget: Widget = self.clock_menu.widget().clone().upcast();
        let clipboard_widget: Widget = self.clipboard_menu.widget().clone().upcast();
        let notification_menu_widget: Widget = self.notification_menu.widget().clone().upcast();
        let screenshot_menu_widget: Widget = self.screenshot_menu.widget().clone().upcast();
        let app_launcher_menu_widget: Widget = self.app_launcher_menu.widget().clone().upcast();
        let wallpaper_menu_widget: Widget = self.wallpaper_menu.widget().clone().upcast();
        let screenshare_menu_widget: Widget = self.screenshare_menu.widget().clone().upcast();
        let ufw_menu_widget: Widget = self.ufw_menu.widget().clone().upcast();
        // Bluetooth menu position read directly from config (skip
        // the 19-arg RepositionMenus signature — defaults work).
        let bluetooth_menu_widget: Widget = self.bluetooth_menu.widget().clone().upcast();
        let bluetooth_menu_position = mshell_config::config_manager::config_manager()
            .config()
            .menus()
            .bluetooth_menu()
            .position()
            .get();
        let cpu_dashboard_menu_widget: Widget =
            self.cpu_dashboard_menu.widget().clone().upcast();
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
        let system_update_menu_widget: Widget =
            self.system_update_menu.widget().clone().upcast();
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
        let ssh_menu_widget: Widget = self.ssh_menu.widget().clone().upcast();
        let ssh_menu_position = mshell_config::config_manager::config_manager()
            .config()
            .menus()
            .ssh_menu()
            .position()
            .get();
        let dns_menu_widget: Widget = self.dns_menu.widget().clone().upcast();
        let podman_menu_widget: Widget = self.podman_menu.widget().clone().upcast();
        let notes_menu_widget: Widget = self.notes_menu.widget().clone().upcast();
        let ip_menu_widget: Widget = self.ip_menu.widget().clone().upcast();
        let network_menu_widget: Widget = self.network_menu.widget().clone().upcast();
        let power_menu_widget: Widget = self.power_menu.widget().clone().upcast();
        let media_player_menu_widget: Widget =
            self.media_player_menu.widget().clone().upcast();
        let session_menu_widget: Widget = self.session_menu.widget().clone().upcast();
        let settings_menu_widget: Widget = self.settings_menu.widget().clone().upcast();
        let dashboard_menu_widget: Widget = self.dashboard_menu.widget().clone().upcast();
        let mshelldash_menu_widget: Widget =
            self.mshelldash_menu.widget().clone().upcast();

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
        Self::add_to_stack(
            widgets,
            &ufw_menu_widget,
            NUFW_MENU,
            &ufw_menu_position,
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
            &ssh_menu_widget,
            SSH_MENU,
            &ssh_menu_position,
        );
        Self::add_to_stack(
            widgets,
            &dns_menu_widget,
            NDNS_MENU,
            &dns_menu_position,
        );
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
        Self::add_to_stack(widgets, &ip_menu_widget, NIP_MENU, &ip_menu_position);
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
            &session_menu_widget,
            SESSION_MENU,
            &session_menu_position,
        );
        Self::add_to_stack(
            widgets,
            &settings_menu_widget,
            SETTINGS_MENU,
            &settings_menu_position,
        );
        Self::add_to_stack(
            widgets,
            &dashboard_menu_widget,
            DASHBOARD_MENU,
            &dashboard_menu_position,
        );
        // mshelldash — fixed Top anchor for now (wave 1). Config-driven
        // positioning can follow once it earns a Settings → Menus entry.
        Self::add_to_stack(
            widgets,
            &mshelldash_menu_widget,
            MSHELLDASH_MENU,
            &Position::Top,
        );
        // Margo Layout menu — position read from config (the
        // Settings → Menus page exposes the knob). Bar pill output
        // cascades through `BarOutput::MargoLayoutClicked` to
        // `FrameInput::ToggleMargoLayoutMenu` which calls
        // `toggle_menu(MARGO_LAYOUT_MENU, …)` against the same
        // stack.
        let margo_layout_menu_widget: Widget =
            self.margo_layout_menu.widget().clone().upcast();
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
        BarModel::builder()
            .launch(BarInit { bar_type })
            .forward(sender.input_sender(), |msg| match msg {
                BarOutput::ClockClicked => FrameInput::ToggleClockMenu,
                BarOutput::DashboardClicked => FrameInput::ToggleDashboardMenu,
                BarOutput::ClipboardClicked => FrameInput::ToggleClipboardMenu,
                BarOutput::NotificationsClicked => FrameInput::ToggleNotificationMenu,
                BarOutput::ScreenshotClicked => FrameInput::ToggleScreenshotMenu,
                BarOutput::AppLauncherClicked => FrameInput::ToggleAppLauncherMenu,
                BarOutput::WallpaperClicked => FrameInput::ToggleWallpaperMenu,
                BarOutput::UfwClicked => FrameInput::ToggleUfwMenu,
                BarOutput::BluetoothClicked => FrameInput::ToggleBluetoothMenu,
                BarOutput::CpuDashboardClicked => FrameInput::ToggleCpuDashboardMenu,
                BarOutput::AudioDashboardClicked => FrameInput::ToggleAudioDashboardMenu,
                BarOutput::SystemUpdateClicked => FrameInput::ToggleSystemUpdateMenu,
                BarOutput::ValentClicked => FrameInput::ToggleValentMenu,
                BarOutput::WeatherClicked => FrameInput::ToggleWeatherMenu,
                BarOutput::KeepAwakeClicked => FrameInput::ToggleKeepAwakeMenu,
                BarOutput::TwilightClicked => FrameInput::ToggleTwilightMenu,
                BarOutput::KeybindsClicked => FrameInput::ToggleKeybindsMenu,
                BarOutput::SshSessionsClicked => FrameInput::ToggleSshSessionsMenu,
                BarOutput::DnsClicked => FrameInput::ToggleDnsMenu,
                BarOutput::PodmanClicked => FrameInput::TogglePodmanMenu,
                BarOutput::NotesClicked => FrameInput::ToggleNotesMenu,
                BarOutput::IpClicked => FrameInput::ToggleIpMenu,
                BarOutput::NetworkClicked => FrameInput::ToggleNetworkMenu,
                BarOutput::PowerClicked => FrameInput::TogglePowerMenu,
                BarOutput::MediaPlayerClicked => FrameInput::ToggleMediaPlayerMenu,
                BarOutput::MargoLayoutClicked => FrameInput::ToggleMargoLayoutMenu,
                BarOutput::CloseMenu => FrameInput::CloseMenus,
            })
    }

    fn build_menu(sender: &ComponentSender<Self>, menu_type: MenuType) -> Controller<MenuModel> {
        MenuModel::builder()
            .launch(MenuInit { menu_type })
            .forward(sender.input_sender(), |msg| match msg {
                MenuOutput::CloseMenu => FrameInput::CloseMenus,
            })
    }
}

impl Drop for Frame {
    fn drop(&mut self) {
        self.top_spacer.widget().destroy();
        self.bottom_spacer.widget().destroy();
    }
}
