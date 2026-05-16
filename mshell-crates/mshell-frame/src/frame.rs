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

const CLOCK_MENU: &str = "clock";
const CLIPBOARD_MENU: &str = "clipboard";
const QUICK_SETTINGS_MENU: &str = "quick_settings";
const APP_LAUNCHER_MENU: &str = "app_launcher";
const SCREENSHOT_MENU: &str = "screenshot";
const NOTIFICATION_MENU: &str = "notification";
const WALLPAPER_MENU: &str = "wallpaper";
const SCREENSHARE_MENU: &str = "screenshare";
const NUFW_MENU: &str = "nufw";
const NDNS_MENU: &str = "ndns";
const NPODMAN_MENU: &str = "npodman";
const NNOTES_MENU: &str = "nnotes";
const NIP_MENU: &str = "nip";
const NNETWORK_MENU: &str = "nnetwork";
const NPOWER_MENU: &str = "npower";
const MEDIA_PLAYER_MENU: &str = "media_player";
const SESSION_MENU: &str = "session";
const SETTINGS_MENU: &str = "settings";
const DASHBOARD_MENU: &str = "dashboard";
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
    top_spacer: Controller<FrameSpacerModel>,
    bottom_spacer: Controller<FrameSpacerModel>,
    clock_menu: Controller<MenuModel>,
    clipboard_menu: Controller<MenuModel>,
    quick_settings_menu: Controller<MenuModel>,
    notification_menu: Controller<MenuModel>,
    screenshot_menu: Controller<MenuModel>,
    app_launcher_menu: Controller<MenuModel>,
    wallpaper_menu: Controller<MenuModel>,
    screenshare_menu: Controller<MenuModel>,
    nufw_menu: Controller<MenuModel>,
    ndns_menu: Controller<MenuModel>,
    npodman_menu: Controller<MenuModel>,
    nnotes_menu: Controller<MenuModel>,
    nip_menu: Controller<MenuModel>,
    nnetwork_menu: Controller<MenuModel>,
    npower_menu: Controller<MenuModel>,
    media_player_menu: Controller<MenuModel>,
    session_menu: Controller<MenuModel>,
    /// Settings panel — uses its own dedicated model (not
    /// `MenuModel`) because its content is a custom sidebar +
    /// stack rather than the generic menu-widget pipeline.
    settings_menu: Controller<mshell_settings::SettingsWindowModel>,
    dashboard_menu: Controller<MenuModel>,
    margo_layout_menu: Controller<MenuModel>,
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
        Position, Position, Position, Position, Position, Position, Position, Position, Position,
        // dashboard_menu_position
        Position,
    ),
    ToggleClockMenu,
    ToggleClipboardMenu,
    ToggleQuickSettingsMenu,
    ToggleNotificationMenu,
    ToggleScreenshotMenu,
    ToggleAppLauncherMenu,
    ToggleWallpaperMenu,
    ToggleNufwMenu,
    ToggleNdnsMenu,
    ToggleNpodmanMenu,
    ToggleNnotesMenu,
    ToggleNipMenu,
    ToggleNnetworkMenu,
    ToggleNpowerMenu,
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
    /// Open / close the Margo layout switcher menu (in-frame
    /// replacement for the legacy bar popover).
    ToggleMargoLayoutMenu,
    CloseMenus,
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
                sender_esc.input(FrameInput::CloseMenus);
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
                sender_esc2.input(FrameInput::CloseMenus);
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
        let main_menu = Self::build_menu(&sender, MenuType::QuickSettings);
        let notification_menu = Self::build_menu(&sender, MenuType::Notifications);
        let screenshot_menu = Self::build_menu(&sender, MenuType::Screenshot);
        let app_launcher_menu = Self::build_menu(&sender, MenuType::AppLauncher);
        let wallpaper_menu = Self::build_menu(&sender, MenuType::Wallpaper);
        let screenshare_menu = Self::build_menu(&sender, MenuType::HyprlandScreenshare);
        let nufw_menu = Self::build_menu(&sender, MenuType::Nufw);
        let ndns_menu = Self::build_menu(&sender, MenuType::Ndns);
        let npodman_menu = Self::build_menu(&sender, MenuType::Npodman);
        let nnotes_menu = Self::build_menu(&sender, MenuType::Nnotes);
        let nip_menu = Self::build_menu(&sender, MenuType::Nip);
        let nnetwork_menu = Self::build_menu(&sender, MenuType::Nnetwork);
        let npower_menu = Self::build_menu(&sender, MenuType::Npower);
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
            let quick_settings_menu_position =
                config.menus().quick_settings_menu().position().get();
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
            let nufw_menu_position = config.menus().nufw_menu().position().get();
            let config = menu_config.clone();
            let ndns_menu_position = config.menus().ndns_menu().position().get();
            let config = menu_config.clone();
            let npodman_menu_position = config.menus().npodman_menu().position().get();
            let config = menu_config.clone();
            let nnotes_menu_position = config.menus().nnotes_menu().position().get();
            let config = menu_config.clone();
            let nip_menu_position = config.menus().nip_menu().position().get();
            let config = menu_config.clone();
            let nnetwork_menu_position = config.menus().nnetwork_menu().position().get();
            let config = menu_config.clone();
            let npower_menu_position = config.menus().npower_menu().position().get();
            let config = menu_config.clone();
            let media_player_menu_position =
                config.menus().media_player_menu().position().get();
            let config = menu_config.clone();
            let session_menu_position = config.menus().session_menu().position().get();
            let config = menu_config.clone();
            let settings_menu_position = config.menus().settings_menu().position().get();
            let config = menu_config.clone();
            let dashboard_menu_position = config.menus().dashboard_menu().position().get();
            sender_clone.input(FrameInput::RepositionMenus(
                clock_menu_position,
                clipboard_menu_position,
                quick_settings_menu_position,
                notification_menu_position,
                screenshot_menu_position,
                app_launcher_menu_position,
                wallpaper_menu_position,
                screenshare_menu_position,
                nufw_menu_position,
                ndns_menu_position,
                npodman_menu_position,
                nnotes_menu_position,
                nip_menu_position,
                nnetwork_menu_position,
                npower_menu_position,
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
            quick_settings_menu: main_menu,
            notification_menu,
            screenshot_menu,
            app_launcher_menu,
            wallpaper_menu,
            screenshare_menu,
            nufw_menu,
            ndns_menu,
            npodman_menu,
            nnotes_menu,
            nip_menu,
            nnetwork_menu,
            npower_menu,
            media_player_menu,
            session_menu,
            settings_menu,
            dashboard_menu,
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
                quick_settings_menu_position,
                notification_menu_position,
                screenshot_menu_position,
                app_launcher_menu_position,
                wallpaper_menu_position,
                screenshare_menu_position,
                nufw_menu_position,
                ndns_menu_position,
                npodman_menu_position,
                nnotes_menu_position,
                nip_menu_position,
                nnetwork_menu_position,
                npower_menu_position,
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
                    quick_settings_menu_position,
                    notification_menu_position,
                    screenshot_menu_position,
                    app_launcher_menu_position,
                    wallpaper_menu_position,
                    screenshare_menu_position,
                    nufw_menu_position,
                    ndns_menu_position,
                    npodman_menu_position,
                    nnotes_menu_position,
                    nip_menu_position,
                    nnetwork_menu_position,
                    npower_menu_position,
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
            FrameInput::ToggleQuickSettingsMenu => {
                self.toggle_menu(QUICK_SETTINGS_MENU, widgets);
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
            FrameInput::ToggleWallpaperMenu => {
                self.toggle_menu(WALLPAPER_MENU, widgets);
                self.sync_keyboard_mode(root);
            }
            FrameInput::ToggleNufwMenu => {
                self.toggle_menu(NUFW_MENU, widgets);
                self.sync_keyboard_mode(root);
            }
            FrameInput::ToggleNdnsMenu => {
                self.toggle_menu(NDNS_MENU, widgets);
                self.sync_keyboard_mode(root);
            }
            FrameInput::ToggleNpodmanMenu => {
                self.toggle_menu(NPODMAN_MENU, widgets);
                self.sync_keyboard_mode(root);
            }
            FrameInput::ToggleNnotesMenu => {
                self.toggle_menu(NNOTES_MENU, widgets);
                self.sync_keyboard_mode(root);
            }
            FrameInput::ToggleNipMenu => {
                self.toggle_menu(NIP_MENU, widgets);
                self.sync_keyboard_mode(root);
            }
            FrameInput::ToggleNnetworkMenu => {
                self.toggle_menu(NNETWORK_MENU, widgets);
                self.sync_keyboard_mode(root);
            }
            FrameInput::ToggleNpowerMenu => {
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

                self.quick_settings_menu
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

        self.quick_settings_menu
            .sender()
            .send(MenuInput::RevealChanged(
                name == QUICK_SETTINGS_MENU && now_visible,
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
        quick_settings_position: Position,
        notification_menu_position: Position,
        screenshot_menu_position: Position,
        app_launcher_menu_position: Position,
        wallpaper_menu_position: Position,
        screenshare_menu_position: Position,
        nufw_menu_position: Position,
        ndns_menu_position: Position,
        npodman_menu_position: Position,
        nnotes_menu_position: Position,
        nip_menu_position: Position,
        nnetwork_menu_position: Position,
        npower_menu_position: Position,
        media_player_menu_position: Position,
        session_menu_position: Position,
        settings_menu_position: Position,
        dashboard_menu_position: Position,
    ) {
        let clock_widget: Widget = self.clock_menu.widget().clone().upcast();
        let clipboard_widget: Widget = self.clipboard_menu.widget().clone().upcast();
        let quick_settings_widget: Widget = self.quick_settings_menu.widget().clone().upcast();
        let notification_menu_widget: Widget = self.notification_menu.widget().clone().upcast();
        let screenshot_menu_widget: Widget = self.screenshot_menu.widget().clone().upcast();
        let app_launcher_menu_widget: Widget = self.app_launcher_menu.widget().clone().upcast();
        let wallpaper_menu_widget: Widget = self.wallpaper_menu.widget().clone().upcast();
        let screenshare_menu_widget: Widget = self.screenshare_menu.widget().clone().upcast();
        let nufw_menu_widget: Widget = self.nufw_menu.widget().clone().upcast();
        let ndns_menu_widget: Widget = self.ndns_menu.widget().clone().upcast();
        let npodman_menu_widget: Widget = self.npodman_menu.widget().clone().upcast();
        let nnotes_menu_widget: Widget = self.nnotes_menu.widget().clone().upcast();
        let nip_menu_widget: Widget = self.nip_menu.widget().clone().upcast();
        let nnetwork_menu_widget: Widget = self.nnetwork_menu.widget().clone().upcast();
        let npower_menu_widget: Widget = self.npower_menu.widget().clone().upcast();
        let media_player_menu_widget: Widget =
            self.media_player_menu.widget().clone().upcast();
        let session_menu_widget: Widget = self.session_menu.widget().clone().upcast();
        let settings_menu_widget: Widget = self.settings_menu.widget().clone().upcast();
        let dashboard_menu_widget: Widget = self.dashboard_menu.widget().clone().upcast();

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
            &quick_settings_widget,
            QUICK_SETTINGS_MENU,
            &quick_settings_position,
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
            &nufw_menu_widget,
            NUFW_MENU,
            &nufw_menu_position,
        );
        Self::add_to_stack(
            widgets,
            &ndns_menu_widget,
            NDNS_MENU,
            &ndns_menu_position,
        );
        Self::add_to_stack(
            widgets,
            &npodman_menu_widget,
            NPODMAN_MENU,
            &npodman_menu_position,
        );
        Self::add_to_stack(
            widgets,
            &nnotes_menu_widget,
            NNOTES_MENU,
            &nnotes_menu_position,
        );
        Self::add_to_stack(widgets, &nip_menu_widget, NIP_MENU, &nip_menu_position);
        Self::add_to_stack(
            widgets,
            &nnetwork_menu_widget,
            NNETWORK_MENU,
            &nnetwork_menu_position,
        );
        Self::add_to_stack(
            widgets,
            &npower_menu_widget,
            NPOWER_MENU,
            &npower_menu_position,
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
        // Margo Layout menu uses the same Top anchor as Clock /
        // Dashboard since its content (a vertical layout list) is
        // most natural under the bar. Bar pill output cascades
        // through `BarOutput::MargoLayoutClicked` to
        // `FrameInput::ToggleMargoLayoutMenu` which calls
        // `toggle_menu(MARGO_LAYOUT_MENU, …)` against the same
        // stack. Position is hardcoded `Top` for the MVP — a
        // follow-up can wire it through `RepositionMenus` once
        // the Settings UI exposes the per-menu position knob.
        let margo_layout_menu_widget: Widget =
            self.margo_layout_menu.widget().clone().upcast();
        Self::add_to_stack(
            widgets,
            &margo_layout_menu_widget,
            MARGO_LAYOUT_MENU,
            &Position::Top,
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
                BarOutput::ClipboardClicked => FrameInput::ToggleClipboardMenu,
                BarOutput::MainMenuClicked => FrameInput::ToggleQuickSettingsMenu,
                BarOutput::NotificationsClicked => FrameInput::ToggleNotificationMenu,
                BarOutput::ScreenshotClicked => FrameInput::ToggleScreenshotMenu,
                BarOutput::AppLauncherClicked => FrameInput::ToggleAppLauncherMenu,
                BarOutput::WallpaperClicked => FrameInput::ToggleWallpaperMenu,
                BarOutput::NufwClicked => FrameInput::ToggleNufwMenu,
                BarOutput::NdnsClicked => FrameInput::ToggleNdnsMenu,
                BarOutput::NpodmanClicked => FrameInput::ToggleNpodmanMenu,
                BarOutput::NnotesClicked => FrameInput::ToggleNnotesMenu,
                BarOutput::NipClicked => FrameInput::ToggleNipMenu,
                BarOutput::NnetworkClicked => FrameInput::ToggleNnetworkMenu,
                BarOutput::NpowerClicked => FrameInput::ToggleNpowerMenu,
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
