//! Per-widget menu settings — one component, parameterised by
//! `MenuKind`, that surfaces a given menu's `position` and
//! `minimum_width`. Used inside the `Widgets` sub-sidebar so each
//! menu gets its own focused settings page.
//!
//! The widgets-list editor (which BarWidget pills live inside a
//! menu) stays in the existing `menu_settings::Layout` page —
//! that's a cross-cutting view of every menu at once. These
//! per-menu pages are the "I just want to tweak THIS one"
//! shortcut.

use crate::cc_tiles_settings::{CcTilesSettingsInit, CcTilesSettingsModel};
use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    BarWidgetsStoreFields, BarsStoreFields, ConfigStoreFields, MenuStoreFields, MenusStoreFields,
    SystemUpdateBarWidgetStoreFields,
};
use mshell_config::schema::position::Position;
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentController, ComponentParts, ComponentSender, Controller, gtk};

/// Which menu this settings page targets. The enum carries
/// everything we need to read / write through `config_manager`
/// (descriptive label + reactive-field accessor dispatch).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MenuKind {
    AppLauncher,
    AudioDashboard,
    Bluetooth,
    Clipboard,
    Clock,
    CpuDashboard,
    Dashboard,
    MargoLayout,
    MediaPlayer,
    /// The combined VPN menu (the `mvpn` pill's menu — `vpn_menu` config).
    /// Carries the Mullvad controls + the collapsible DNS section.
    Vpn,
    /// The standalone DNS menu (`dns_menu`), opened by `mshellctl menu dns`.
    /// No bar pill — its config is only useful for that terminal verb.
    Dns,
    Ip,
    Network,
    Notes,
    Notifications,
    Podman,
    Power,
    Screenshot,
    SystemUpdate,
    Valent,
    Weather,
    KeepAwake,
    Twilight,
    Keybinds,
    AlarmClock,
    ControlCenter,
    SshSessions,
    Ufw,
    Wallpaper,
}

// ── Per-menu field dispatch ──────────────────────────────────────────────
// The MenuKind → reactive-store accessor map lives here ONCE (read-form +
// write-form), instead of being copy-pasted as a full 28-arm match in every
// read/tracked/write helper. Adding a new per-menu field is then one line per
// helper, not a fresh 28-arm match.
macro_rules! menu_read {
    ($self:expr, $field:ident, $g:ident) => {{
        let m = config_manager().config().menus();
        match $self {
            MenuKind::AppLauncher => m.app_launcher_menu().$field().$g(),
            MenuKind::Clipboard => m.clipboard_menu().$field().$g(),
            MenuKind::Clock => m.clock_menu().$field().$g(),
            MenuKind::Dashboard => m.dashboard_menu().$field().$g(),
            MenuKind::MediaPlayer => m.media_player_menu().$field().$g(),
            MenuKind::Vpn => m.vpn_menu().$field().$g(),
            MenuKind::Dns => m.dns_menu().$field().$g(),
            MenuKind::Ip => m.ip_menu().$field().$g(),
            MenuKind::Network => m.network_menu().$field().$g(),
            MenuKind::Notes => m.notes_menu().$field().$g(),
            MenuKind::Notifications => m.notification_menu().$field().$g(),
            MenuKind::Podman => m.podman_menu().$field().$g(),
            MenuKind::Wallpaper => m.wallpaper_menu().$field().$g(),
            MenuKind::Power => m.power_menu().$field().$g(),
            MenuKind::Screenshot => m.screenshot_menu().$field().$g(),
            MenuKind::Ufw => m.ufw_menu().$field().$g(),
            MenuKind::Bluetooth => m.bluetooth_menu().$field().$g(),
            MenuKind::CpuDashboard => m.cpu_dashboard_menu().$field().$g(),
            MenuKind::AudioDashboard => m.audio_dashboard_menu().$field().$g(),
            MenuKind::SystemUpdate => m.system_update_menu().$field().$g(),
            MenuKind::Valent => m.valent_menu().$field().$g(),
            MenuKind::Weather => m.weather_menu().$field().$g(),
            MenuKind::KeepAwake => m.keep_awake_menu().$field().$g(),
            MenuKind::Twilight => m.twilight_menu().$field().$g(),
            MenuKind::Keybinds => m.keybinds_menu().$field().$g(),
            MenuKind::AlarmClock => m.alarmclock_menu().$field().$g(),
            MenuKind::ControlCenter => m.control_center_menu().$field().$g(),
            MenuKind::SshSessions => m.ssh_menu().$field().$g(),
            MenuKind::MargoLayout => m.margo_layout_menu().$field().$g(),
        }
    }};
}

macro_rules! menu_write {
    ($self:expr, $field:ident, $val:expr) => {
        config_manager().update_config(|c| match $self {
            MenuKind::AppLauncher => c.menus.app_launcher_menu.$field = $val,
            MenuKind::Clipboard => c.menus.clipboard_menu.$field = $val,
            MenuKind::Clock => c.menus.clock_menu.$field = $val,
            MenuKind::Dashboard => c.menus.dashboard_menu.$field = $val,
            MenuKind::MediaPlayer => c.menus.media_player_menu.$field = $val,
            MenuKind::Vpn => c.menus.vpn_menu.$field = $val,
            MenuKind::Dns => c.menus.dns_menu.$field = $val,
            MenuKind::Ip => c.menus.ip_menu.$field = $val,
            MenuKind::Network => c.menus.network_menu.$field = $val,
            MenuKind::Notes => c.menus.notes_menu.$field = $val,
            MenuKind::Notifications => c.menus.notification_menu.$field = $val,
            MenuKind::Podman => c.menus.podman_menu.$field = $val,
            MenuKind::Wallpaper => c.menus.wallpaper_menu.$field = $val,
            MenuKind::Power => c.menus.power_menu.$field = $val,
            MenuKind::Screenshot => c.menus.screenshot_menu.$field = $val,
            MenuKind::Ufw => c.menus.ufw_menu.$field = $val,
            MenuKind::Bluetooth => c.menus.bluetooth_menu.$field = $val,
            MenuKind::CpuDashboard => c.menus.cpu_dashboard_menu.$field = $val,
            MenuKind::AudioDashboard => c.menus.audio_dashboard_menu.$field = $val,
            MenuKind::SystemUpdate => c.menus.system_update_menu.$field = $val,
            MenuKind::Valent => c.menus.valent_menu.$field = $val,
            MenuKind::Weather => c.menus.weather_menu.$field = $val,
            MenuKind::KeepAwake => c.menus.keep_awake_menu.$field = $val,
            MenuKind::Twilight => c.menus.twilight_menu.$field = $val,
            MenuKind::Keybinds => c.menus.keybinds_menu.$field = $val,
            MenuKind::AlarmClock => c.menus.alarmclock_menu.$field = $val,
            MenuKind::ControlCenter => c.menus.control_center_menu.$field = $val,
            MenuKind::SshSessions => c.menus.ssh_menu.$field = $val,
            MenuKind::MargoLayout => c.menus.margo_layout_menu.$field = $val,
        });
    };
}

impl MenuKind {
    pub(crate) fn display_name(self) -> &'static str {
        match self {
            Self::AppLauncher => "App Launcher",
            Self::AudioDashboard => "Audio Dashboard",
            Self::Bluetooth => "Bluetooth",
            Self::Clipboard => "Clipboard",
            Self::Clock => "Clock",
            Self::CpuDashboard => "CPU Dashboard",
            Self::Dashboard => "Dashboard",
            Self::MargoLayout => "Margo Layout",
            Self::MediaPlayer => "Media Player",
            Self::Vpn => "VPN",
            Self::Dns => "DNS",
            Self::Ip => "Public IP",
            Self::Network => "Network Console",
            Self::Notes => "Notes Hub",
            Self::Notifications => "Notifications",
            Self::Podman => "Podman",
            Self::Power => "Power Profile",
            Self::Screenshot => "Screenshot",
            Self::SystemUpdate => "System Updates",
            Self::Valent => "Valent Connect",
            Self::Weather => "Weather",
            Self::KeepAwake => "Keep Awake",
            Self::Twilight => "Twilight",
            Self::Keybinds => "Keyboard Shortcuts",
            Self::AlarmClock => "Alarm Clock",
            Self::ControlCenter => "Control Center",
            Self::SshSessions => "SSH Sessions",
            Self::Ufw => "UFW Firewall",
            Self::Wallpaper => "Wallpaper",
        }
    }

    /// All known menu kinds, in the order they should appear in
    /// the cross-cutting Menus settings page. Kept stable so the
    /// scroll position survives a config reload.
    pub(crate) fn all() -> &'static [MenuKind] {
        &[
            MenuKind::Clock,
            MenuKind::Dashboard,
            MenuKind::Clipboard,
            MenuKind::Screenshot,
            MenuKind::Notifications,
            MenuKind::AppLauncher,
            MenuKind::Wallpaper,
            MenuKind::MediaPlayer,
            MenuKind::Power,
            MenuKind::Bluetooth,
            MenuKind::CpuDashboard,
            MenuKind::AudioDashboard,
            MenuKind::SystemUpdate,
            MenuKind::Valent,
            MenuKind::Weather,
            MenuKind::KeepAwake,
            MenuKind::Twilight,
            MenuKind::Keybinds,
            MenuKind::AlarmClock,
            MenuKind::ControlCenter,
            MenuKind::SshSessions,
            MenuKind::Ufw,
            MenuKind::Vpn,
            MenuKind::Dns,
            MenuKind::Podman,
            MenuKind::Notes,
            MenuKind::Ip,
            MenuKind::Network,
            MenuKind::MargoLayout,
        ]
    }

    /// Snapshot the menu's current position. `_untracked` so the
    /// initial model load doesn't subscribe; the `EffectScope` subscribes
    /// explicitly via the `tracked_*` variants.
    fn read_position(self) -> Position {
        menu_read!(self, position, get_untracked)
    }

    fn read_min_width(self) -> i32 {
        menu_read!(self, minimum_width, get_untracked)
    }

    fn tracked_position(self) -> Position {
        menu_read!(self, position, get)
    }

    fn tracked_min_width(self) -> i32 {
        menu_read!(self, minimum_width, get)
    }

    fn write_position(self, p: Position) {
        menu_write!(self, position, p);
    }

    fn write_min_width(self, w: i32) {
        menu_write!(self, minimum_width, w);
    }

    fn read_max_height(self) -> i32 {
        menu_read!(self, maximum_height, get_untracked)
    }

    fn tracked_max_height(self) -> i32 {
        menu_read!(self, maximum_height, get)
    }

    fn write_max_height(self, h: i32) {
        menu_write!(self, maximum_height, h);
    }

    /// Snapshot the menu's current widget list. Used to seed the
    /// `MenuWidgetListModel` factory at panel-creation time.
    pub(crate) fn read_widgets(self) -> Vec<mshell_config::schema::menu_widgets::MenuWidget> {
        menu_read!(self, widgets, get_untracked)
    }

    /// Tracked read — subscribes the calling effect to widget-list
    /// changes so an external `mshellctl config reload` repaints
    /// the panel without a UI restart.
    pub(crate) fn tracked_widgets(self) -> Vec<mshell_config::schema::menu_widgets::MenuWidget> {
        menu_read!(self, widgets, get)
    }

    /// Persist a new widget list to disk. Called from the panel
    /// when the in-UI reorder/add/remove fires.
    pub(crate) fn write_widgets(
        self,
        widgets: Vec<mshell_config::schema::menu_widgets::MenuWidget>,
    ) {
        menu_write!(self, widgets, widgets);
    }
}

#[derive(Debug)]
pub(crate) struct WidgetMenuSettingsModel {
    kind: MenuKind,
    position: Position,
    minimum_width: i32,
    /// Maximum visible content height in pixels. 0 = no cap.
    maximum_height: i32,
    /// SystemUpdate-only: the pill's poll cadence in minutes.
    /// Unused (kept at 0) for every other kind — the view hides
    /// the cadence section unless `kind == SystemUpdate`.
    check_interval_minutes: u32,
    position_model: gtk::StringList,
    /// ControlCenter-only: the tiles order/visibility sub-section.
    /// `None` for every other menu kind.
    _cc_tiles_controller: Option<Controller<CcTilesSettingsModel>>,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum WidgetMenuSettingsInput {
    PositionPicked(u32),
    MinWidthChanged(i32),
    MaxHeightChanged(i32),
    PositionEffect(Position),
    MinWidthEffect(i32),
    MaxHeightEffect(i32),
    CheckIntervalChanged(u32),
    CheckIntervalEffect(u32),
}

#[derive(Debug)]
pub(crate) enum WidgetMenuSettingsOutput {}

pub(crate) struct WidgetMenuSettingsInit {
    pub(crate) kind: MenuKind,
}

#[relm4::component(pub(crate))]
impl Component for WidgetMenuSettingsModel {
    type CommandOutput = ();
    type Input = WidgetMenuSettingsInput;
    type Output = WidgetMenuSettingsOutput;
    type Init = WidgetMenuSettingsInit;

    view! {
        #[root]
        gtk::ScrolledWindow {
            set_vscrollbar_policy: gtk::PolicyType::Automatic,
            set_hscrollbar_policy: gtk::PolicyType::Never,
            set_propagate_natural_height: false,
            set_propagate_natural_width: false,
            set_hexpand: true,
            set_vexpand: true,

            #[name = "page_box"]
            gtk::Box {
                add_css_class: "settings-page",
                set_orientation: gtk::Orientation::Vertical,
                set_hexpand: true,
                set_spacing: 16,

                gtk::Box {
                    add_css_class: "settings-hero",
                    set_orientation: gtk::Orientation::Horizontal,
                    set_halign: gtk::Align::Start,
                    set_spacing: 16,
                    gtk::Image {
                        add_css_class: "settings-hero-icon",
                        set_icon_name: Some("view-list-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "Menu widget",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "Per-menu widget configuration — which sub-widgets show up inside a given menu and in what order.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                gtk::Label {
                    add_css_class: "label-large-bold",
                    #[watch]
                    set_label: model.kind.display_name(),
                    set_halign: gtk::Align::Start,
                },

                gtk::Label {
                    add_css_class: "label-small",
                    set_label: "Per-menu layout. The widgets that appear inside this menu are configured under Widgets → Layout.",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Position",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Which screen edge this menu anchors to.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    gtk::DropDown {
                        set_width_request: 180,
                        set_valign: gtk::Align::Center,
                        set_model: Some(&model.position_model),
                        #[watch]
                        #[block_signal(position_handler)]
                        set_selected: model.position.to_index(),
                        connect_selected_notify[sender] => move |dd| {
                            sender.input(WidgetMenuSettingsInput::PositionPicked(dd.selected()));
                        } @position_handler,
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Minimum Width",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Width floor in pixels. The menu may grow past this for long content.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_range: (200.0, 2000.0),
                        set_increments: (10.0, 50.0),
                        set_digits: 0,
                        #[watch]
                        #[block_signal(min_width_handler)]
                        set_value: model.minimum_width as f64,
                        connect_value_changed[sender] => move |s| {
                            sender.input(WidgetMenuSettingsInput::MinWidthChanged(s.value() as i32));
                        } @min_width_handler,
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Maximum Height",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Viewport cap in pixels. The menu scrolls vertically past this height. Set to 0 to disable the cap and let the menu grow to fit its contents.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        // 0 = uncapped; otherwise reasonable monitor-sized range.
                        set_range: (0.0, 2000.0),
                        set_increments: (10.0, 50.0),
                        set_digits: 0,
                        #[watch]
                        #[block_signal(max_height_handler)]
                        set_value: model.maximum_height as f64,
                        connect_value_changed[sender] => move |s| {
                            sender.input(WidgetMenuSettingsInput::MaxHeightChanged(s.value() as i32));
                        } @max_height_handler,
                    },
                },

                // ── System-update-only cadence ───────────────
                //
                // The repo / AUR / Flatpak source toggles live in
                // the panel itself (open the menu → top row); only
                // the poll cadence is a set-once preference, so it
                // stays here. Hidden for every other menu kind.
                gtk::Separator {
                    #[watch]
                    set_visible: model.kind == MenuKind::SystemUpdate,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    #[watch]
                    set_visible: model.kind == MenuKind::SystemUpdate,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Check interval (minutes)",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "How often the pill re-checks pending upgrades. Default 180 (3 h). Right-click the pill for an immediate manual re-check. Which sources to probe (Repo / AUR / Flatpak) is toggled inside the panel itself.",
                            set_hexpand: true,
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },

                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_range: (1.0, 1440.0),
                        set_increments: (5.0, 30.0),
                        set_digits: 0,
                        #[watch]
                        #[block_signal(interval_handler)]
                        set_value: model.check_interval_minutes as f64,
                        connect_value_changed[sender] => move |s| {
                            sender.input(WidgetMenuSettingsInput::CheckIntervalChanged(s.value() as u32));
                        } @interval_handler,
                    },
                },
            }
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let position_refs: Vec<&str> = Position::all().iter().map(|p| p.display_name()).collect();
        let position_model = gtk::StringList::new(&position_refs);

        let mut effects = EffectScope::new();

        let kind = params.kind;
        let sender_clone = sender.clone();
        effects.push(move |_| {
            let p = kind.tracked_position();
            sender_clone.input(WidgetMenuSettingsInput::PositionEffect(p));
        });
        let sender_clone = sender.clone();
        effects.push(move |_| {
            let w = kind.tracked_min_width();
            sender_clone.input(WidgetMenuSettingsInput::MinWidthEffect(w));
        });
        let sender_clone = sender.clone();
        effects.push(move |_| {
            let h = kind.tracked_max_height();
            sender_clone.input(WidgetMenuSettingsInput::MaxHeightEffect(h));
        });
        // SystemUpdate-only: track the pill's poll cadence so an
        // external `mshellctl config reload` repaints the spin.
        // Harmless for other kinds — the read just doesn't drive
        // a visible field.
        let sender_clone = sender.clone();
        effects.push(move |_| {
            let v = config_manager()
                .config()
                .bars()
                .widgets()
                .system_update()
                .check_interval_minutes()
                .get();
            sender_clone.input(WidgetMenuSettingsInput::CheckIntervalEffect(v));
        });

        // ControlCenter-only: build the Tiles sub-section and append it to
        // the page box after the generic position/size controls.
        let cc_tiles_controller = if kind == MenuKind::ControlCenter {
            Some(
                CcTilesSettingsModel::builder()
                    .launch(CcTilesSettingsInit {})
                    .detach(),
            )
        } else {
            None
        };

        let model = WidgetMenuSettingsModel {
            kind,
            position: kind.read_position(),
            minimum_width: kind.read_min_width(),
            maximum_height: kind.read_max_height(),
            check_interval_minutes: config_manager()
                .config()
                .bars()
                .widgets()
                .system_update()
                .check_interval_minutes()
                .get_untracked(),
            position_model,
            _cc_tiles_controller: cc_tiles_controller,
            _effects: effects,
        };

        let widgets = view_output!();

        // Append the CC tiles section widget to the page box when present.
        if let Some(ctrl) = &model._cc_tiles_controller {
            widgets.page_box.append(ctrl.widget());
        }

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            WidgetMenuSettingsInput::PositionPicked(idx) => {
                let p = Position::from_index(idx);
                if self.position != p {
                    self.position = p.clone();
                    self.kind.write_position(p);
                }
            }
            WidgetMenuSettingsInput::MinWidthChanged(w) => {
                if self.minimum_width != w {
                    self.minimum_width = w;
                    self.kind.write_min_width(w);
                }
            }
            WidgetMenuSettingsInput::MaxHeightChanged(h) => {
                if self.maximum_height != h {
                    self.maximum_height = h;
                    self.kind.write_max_height(h);
                }
            }
            WidgetMenuSettingsInput::CheckIntervalChanged(v) => {
                if self.check_interval_minutes != v {
                    self.check_interval_minutes = v;
                    config_manager().update_config(move |c| {
                        c.bars.widgets.system_update.check_interval_minutes = v;
                    });
                }
            }
            WidgetMenuSettingsInput::PositionEffect(p) => self.position = p,
            WidgetMenuSettingsInput::MinWidthEffect(w) => self.minimum_width = w,
            WidgetMenuSettingsInput::MaxHeightEffect(h) => self.maximum_height = h,
            WidgetMenuSettingsInput::CheckIntervalEffect(v) => self.check_interval_minutes = v,
        }
    }
}
