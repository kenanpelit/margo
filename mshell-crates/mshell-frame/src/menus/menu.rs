use crate::menus::builder::build_widget;
use crate::menus::menu_widgets::app_launcher::app_launcher::{AppLauncherInput, AppLauncherModel};
use crate::menus::menu_widgets::audio_in::audio_in_menu_widget::{
    AudioInMenuWidgetInput, AudioInMenuWidgetModel,
};
use crate::menus::menu_widgets::audio_out::audio_out_menu_widget::{
    AudioOutMenuWidgetInput, AudioOutMenuWidgetModel,
};
use crate::menus::menu_widgets::bluetooth::bluetooth_menu_widget::{
    BluetoothMenuWidgetInput, BluetoothMenuWidgetModel,
};
use crate::menus::menu_widgets::clipboard::clipboard::{ClipboardInput, ClipboardModel};
use crate::menus::menu_widgets::notifications::notifications::{
    NotificationsInput, NotificationsModel,
};
use crate::menus::menu_widgets::dns::dns_menu_widget::{DnsMenuWidgetInput, DnsMenuWidgetModel};
use crate::menus::menu_widgets::ip::ip_menu_widget::{IpMenuWidgetInput, IpMenuWidgetModel};
use crate::menus::menu_widgets::network::network_menu_widget::{
    NetworkMenuWidgetInput, NetworkMenuWidgetModel,
};
use crate::menus::menu_widgets::podman::podman_menu_widget::{
    PodmanMenuWidgetInput, PodmanMenuWidgetModel,
};
use crate::menus::menu_widgets::ufw::ufw_menu_widget::{UfwMenuWidgetInput, UfwMenuWidgetModel};
use crate::menus::menu_widgets::system_update::system_update_menu_widget::{
    SystemUpdateMenuWidgetInput, SystemUpdateMenuWidgetModel,
};
use crate::menus::menu_widgets::network_toggle::network_menu_widget::{
    NetworkToggleMenuWidgetInput, NetworkToggleMenuWidgetModel,
};
use crate::menus::menu_widgets::screenshare::screenshare_menu_widget::{
    ScreenshareMenuWidgetInit, ScreenshareMenuWidgetInput, ScreenshareMenuWidgetModel,
    ScreenshareMenuWidgetOutput,
};
use crate::menus::menu_widgets::session::session_menu_widget::{
    SessionMenuWidgetInput, SessionMenuWidgetModel,
};
use crate::menus::menu_widgets::wizard::wizard_menu_widget::{
    WizardMenuWidgetInit, WizardMenuWidgetModel, WizardMenuWidgetOutput,
};
use crate::menus::menu_widgets::wallpaper::wallpaper_menu_widget::{
    WallpaperMenuWidgetInput, WallpaperMenuWidgetModel,
};
use mshell_common::dynamic_box::generic_widget_controller::{
    GenericWidgetController, GenericWidgetControllerExtSafe,
};
use mshell_common::scoped_effects::EffectScope;
use mshell_config::schema::config::{ConfigStoreFields, MenuStoreFields, MenusStoreFields};
use mshell_config::schema::menu_widgets::MenuWidget;
use mshell_utils::clear_box::clear_box;
use reactive_graph::traits::Get;
use relm4::{
    Component, ComponentController, ComponentParts, ComponentSender, Controller, RelmWidgetExt,
    gtk, gtk::prelude::*,
};
use std::fmt::Debug;

pub(crate) enum MenuType {
    Clipboard,
    Clock,
    Notifications,
    Screenshot,
    AppLauncher,
    Wallpaper,
    HyprlandScreenshare,
    Wizard,
    Ufw,
    Dns,
    Podman,
    Notes,
    Ip,
    Network,
    Power,
    Bluetooth,
    CpuDashboard,
    AudioDashboard,
    /// `system_update` bar pill's panel — pending updates grouped
    /// by source (repo / AUR / Flatpak) with Refresh + Update.
    SystemUpdate,
    /// `valent` bar pill's panel — paired phone status + actions.
    Valent,
    /// `weather` bar pill's panel — the Current / Hourly / Daily surface.
    Weather,
    /// `keep_awake` bar pill's panel — duration grid + countdown.
    KeepAwake,
    /// `twilight` bar pill's panel — toggle + temperature + mode +
    /// schedule presets.
    Twilight,
    /// `keybinds` bar pill's panel — searchable cheatsheet of every
    /// shortcut parsed live from margo's `config.conf`.
    Keybinds,
    /// `ssh_sessions` bar pill's panel — searchable `~/.ssh/config`
    /// host list with active-session indicators.
    SshSessions,
    MediaPlayer,
    Session,
    /// Combined clock + quick-settings dashboard. Renders the
    /// hero clock card on top, then calendar + weather + the
    /// full QS stack underneath. Coexists with `Clock` and
    /// `QuickSettings`; users wire a keybind / bar pill if they
    /// prefer the combined view.
    Dashboard,
    /// Margo layout switcher. Replaces the legacy in-bar
    /// `gtk::PopoverMenu` (xdg_popup, detached window feel)
    /// with a regular menu surface that slides out from the
    /// bar like every other menu.
    MargoLayout,
}

pub(crate) struct MenuModel {
    widget_controllers: Vec<Box<dyn GenericWidgetController>>,
    // The `MenuWidget` kinds backing `widget_controllers`, so
    // `SetWidget` can skip the destructive clear+rebuild when the
    // config layer re-notifies with an identical list. The config
    // store is coarse — a write to any field reaches every effect
    // bound to it — so without this guard every unrelated config
    // touch tears down and recreates each menu's content widgets,
    // which silently re-runs their probe loops (dns / ufw /
    // podman shell out on init). Mirrors the bar's guard.
    widget_kinds: Vec<MenuWidget>,
    minimum_width: i32,
    /// Maximum visible content height in pixels. 0 = no cap
    /// (legacy "grow to fit children" behaviour). When > 0, the
    /// outer ScrolledWindow caps the viewport at this value and
    /// the inner content scrolls vertically. Maps onto GTK's
    /// `set_max_content_height` — works as advertised here
    /// because `vscrollbar_policy` is Automatic.
    maximum_height: i32,
    css_class: String,
    /// `true` only for the standalone weather menu (`MenuType::Weather`);
    /// passed to `build_widget` so weather stacks all sections there and
    /// stays paged everywhere else (notably the dashboard).
    weather_all_in_one: bool,
    /// `false` until the content widget tree has been built. Building is
    /// deferred to the first reveal (the menu's `map`), so menus the user
    /// never opens never construct their GTK trees — otherwise ~30 menus
    /// per monitor build their full content at shell startup.
    built: bool,
    /// `true` only for the wizard menu, whose dedicated widget is built
    /// lazily on first reveal (via `AddWizardWidget`) instead of from the
    /// config-driven `widget_kinds`. Screenshare can't do this (it must
    /// exist before the portal reply lands), so it stays eager.
    lazy_wizard: bool,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum MenuInput {
    RevealChanged(bool),
    /// Esc was pressed while the clipboard `/` filter is open — leave
    /// search mode (instead of closing the menu). Routed by the frame.
    ClipboardExitSearch,
    SetWidget(Vec<MenuWidget>),
    SetMinimumWidth(i32),
    SetMaximumHeight(i32),
    AddHyprlandScreenshareWidget,
    ForwardHyprlandScreenshareReply(tokio::sync::oneshot::Sender<String>, String),
    AddWizardWidget,
    /// Forward a category-tab pick to the embedded
    /// AppLauncherModel when this menu hosts one. Used by
    /// `mshellctl menu app-launcher --tab <name>` to open the
    /// launcher on a specific tab — frame fires the toggle,
    /// then sends this so the runtime's `select_category` runs.
    AppLauncherSelectCategory(String),
}

#[derive(Debug)]
pub(crate) enum MenuOutput {
    CloseMenu,
}

pub(crate) struct MenuInit {
    pub(crate) menu_type: MenuType,
}

#[relm4::component(pub)]
impl Component for MenuModel {
    type CommandOutput = ();
    type Input = MenuInput;
    type Output = MenuOutput;
    type Init = MenuInit;

    view! {
        #[root]
        #[name = "scrolled_window"]
        gtk::ScrolledWindow {
            // CSS classes are wired post-`view_output!` so the
            // dashboard's space-separated `"quick-settings-menu
            // dashboard-menu"` is split into two distinct classes
            // (a single slice entry would be treated as one
            // multi-word class and break `.quick-settings-menu`
            // descendant selectors).
            set_css_classes: &["menu-scroll-window"],
            set_vscrollbar_policy: gtk::PolicyType::Automatic,
            set_hscrollbar_policy: gtk::PolicyType::Never,
            set_propagate_natural_height: true,
            // Pin the viewport to exactly `minimum_width` on both
            // axes (min_content_width = max_content_width = w).
            // `set_width_request` alone is just a floor; the
            // ScrolledWindow would still grow if any nested
            // widget reported a larger natural width (the launcher
            // result list does — long row names + the binds-strip
            // footer push the natural well past 720). Clamping the
            // *content area* with min == max gives GTK a hard
            // outer dimension regardless of what the child wants,
            // and makes the Settings → Menus minimum-width spinner
            // actually shrink the panel.
            #[watch]
            set_width_request: model.minimum_width,
            #[watch]
            set_min_content_width: model.minimum_width,
            #[watch]
            set_max_content_width: model.minimum_width,
            set_propagate_natural_width: false,
            // Vertical height cap. 0 (config default) maps to -1
            // ("no cap"), so legacy menus keep their grow-to-fit
            // behaviour unchanged. When the user sets a positive
            // value, GTK clamps the viewport at that height and
            // the inner content scrolls — unlike the horizontal
            // axis, this one actually works because
            // `vscrollbar_policy` is Automatic (GTK's
            // `min/max_content_*` are no-ops only with the Never
            // policy, see gtkscrolledwindow.c:1896).
            #[watch]
            set_max_content_height: if model.maximum_height > 0 {
                model.maximum_height
            } else {
                -1
            },

            #[name = "widget_container"]
            gtk::Box {
                set_margin_all: 20,
                set_orientation: gtk::Orientation::Vertical,
                set_hexpand: false,
            }
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let base_config = mshell_config::config_manager::config_manager().config();

        let mut effects = EffectScope::new();

        let css_class: String;

        match params.menu_type {
            MenuType::Clock => {
                css_class = "clock-menu".to_string();
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let widgets = config.menus().clock_menu().widgets().get();
                    sender_clone.input(MenuInput::SetWidget(widgets));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let minimum_width = config.menus().clock_menu().minimum_width().get();
                    sender_clone.input(MenuInput::SetMinimumWidth(minimum_width));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let maximum_height = config.menus().clock_menu().maximum_height().get();
                    sender_clone.input(MenuInput::SetMaximumHeight(maximum_height));
                });
            }
            MenuType::Clipboard => {
                css_class = "clipboard-menu".to_string();
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let widgets = config.menus().clipboard_menu().widgets().get();
                    sender_clone.input(MenuInput::SetWidget(widgets));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let minimum_width = config.menus().clipboard_menu().minimum_width().get();
                    sender_clone.input(MenuInput::SetMinimumWidth(minimum_width));
                });
                // NOTE: unlike other menus, the clipboard does NOT cap its
                // *outer* scroller at `maximum_height`. The clipboard
                // widget applies that cap to its own inner history
                // scroller instead (see clipboard.rs), so the header +
                // tabs stay fixed while only the list scrolls — and the
                // bounded inner viewport lets the ListView virtualize.
                // Capping both would double-scroll (chrome scrolls away).
            }
            MenuType::Notifications => {
                css_class = "notifications-menu".to_string();
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let widgets = config.menus().notification_menu().widgets().get();
                    sender_clone.input(MenuInput::SetWidget(widgets));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let minimum_width = config.menus().notification_menu().minimum_width().get();
                    sender_clone.input(MenuInput::SetMinimumWidth(minimum_width));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let maximum_height = config.menus().notification_menu().maximum_height().get();
                    sender_clone.input(MenuInput::SetMaximumHeight(maximum_height));
                });
            }
            MenuType::Screenshot => {
                css_class = "screenshot-menu".to_string();
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let widgets = config.menus().screenshot_menu().widgets().get();
                    sender_clone.input(MenuInput::SetWidget(widgets));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let minimum_width = config.menus().screenshot_menu().minimum_width().get();
                    sender_clone.input(MenuInput::SetMinimumWidth(minimum_width));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let maximum_height = config.menus().screenshot_menu().maximum_height().get();
                    sender_clone.input(MenuInput::SetMaximumHeight(maximum_height));
                });
            }
            MenuType::AppLauncher => {
                css_class = "app-launcher-menu".to_string();
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let widgets = config.menus().app_launcher_menu().widgets().get();
                    sender_clone.input(MenuInput::SetWidget(widgets));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let minimum_width = config.menus().app_launcher_menu().minimum_width().get();
                    sender_clone.input(MenuInput::SetMinimumWidth(minimum_width));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let maximum_height = config.menus().app_launcher_menu().maximum_height().get();
                    sender_clone.input(MenuInput::SetMaximumHeight(maximum_height));
                });
            }
            MenuType::Wallpaper => {
                css_class = "wallpaper-menu".to_string();
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let widgets = config.menus().wallpaper_menu().widgets().get();
                    sender_clone.input(MenuInput::SetWidget(widgets));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let minimum_width = config.menus().wallpaper_menu().minimum_width().get();
                    sender_clone.input(MenuInput::SetMinimumWidth(minimum_width));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let maximum_height = config.menus().wallpaper_menu().maximum_height().get();
                    sender_clone.input(MenuInput::SetMaximumHeight(maximum_height));
                });
            }
            MenuType::HyprlandScreenshare => {
                css_class = "hyprland-screenshare-menu".to_string();
                sender.input(MenuInput::AddHyprlandScreenshareWidget);
            }
            MenuType::Wizard => {
                css_class = "wizard-menu".to_string();
                // Built lazily on first reveal (see `lazy_wizard` +
                // RevealChanged) — no eager AddWizardWidget here, so the
                // 8-page Stack + its startup nmcli scan only happen when
                // the user actually opens the wizard.
            }
            MenuType::Ufw => {
                css_class = "ufw-menu".to_string();
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let widgets = config.menus().ufw_menu().widgets().get();
                    sender_clone.input(MenuInput::SetWidget(widgets));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let minimum_width = config.menus().ufw_menu().minimum_width().get();
                    sender_clone.input(MenuInput::SetMinimumWidth(minimum_width));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let maximum_height = config.menus().ufw_menu().maximum_height().get();
                    sender_clone.input(MenuInput::SetMaximumHeight(maximum_height));
                });
            }
            MenuType::Dns => {
                css_class = "dns-menu".to_string();
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let widgets = config.menus().dns_menu().widgets().get();
                    sender_clone.input(MenuInput::SetWidget(widgets));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let minimum_width = config.menus().dns_menu().minimum_width().get();
                    sender_clone.input(MenuInput::SetMinimumWidth(minimum_width));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let maximum_height = config.menus().dns_menu().maximum_height().get();
                    sender_clone.input(MenuInput::SetMaximumHeight(maximum_height));
                });
            }
            MenuType::Podman => {
                css_class = "podman-menu".to_string();
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let widgets = config.menus().podman_menu().widgets().get();
                    sender_clone.input(MenuInput::SetWidget(widgets));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let minimum_width = config.menus().podman_menu().minimum_width().get();
                    sender_clone.input(MenuInput::SetMinimumWidth(minimum_width));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let maximum_height = config.menus().podman_menu().maximum_height().get();
                    sender_clone.input(MenuInput::SetMaximumHeight(maximum_height));
                });
            }
            MenuType::Notes => {
                css_class = "notes-menu".to_string();
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let widgets = config.menus().notes_menu().widgets().get();
                    sender_clone.input(MenuInput::SetWidget(widgets));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let minimum_width = config.menus().notes_menu().minimum_width().get();
                    sender_clone.input(MenuInput::SetMinimumWidth(minimum_width));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let maximum_height = config.menus().notes_menu().maximum_height().get();
                    sender_clone.input(MenuInput::SetMaximumHeight(maximum_height));
                });
            }
            MenuType::Ip => {
                css_class = "ip-menu".to_string();
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let widgets = config.menus().ip_menu().widgets().get();
                    sender_clone.input(MenuInput::SetWidget(widgets));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let minimum_width = config.menus().ip_menu().minimum_width().get();
                    sender_clone.input(MenuInput::SetMinimumWidth(minimum_width));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let maximum_height = config.menus().ip_menu().maximum_height().get();
                    sender_clone.input(MenuInput::SetMaximumHeight(maximum_height));
                });
            }
            MenuType::Network => {
                css_class = "network-menu".to_string();
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let widgets = config.menus().network_menu().widgets().get();
                    sender_clone.input(MenuInput::SetWidget(widgets));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let minimum_width = config.menus().network_menu().minimum_width().get();
                    sender_clone.input(MenuInput::SetMinimumWidth(minimum_width));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let maximum_height = config.menus().network_menu().maximum_height().get();
                    sender_clone.input(MenuInput::SetMaximumHeight(maximum_height));
                });
            }
            MenuType::Bluetooth => {
                // Reuses the .quick-settings-menu CSS so the
                // existing BluetoothMenuWidget revealer-row gets
                // the same card chrome it has inside QS panel.
                css_class = "quick-settings-menu bluetooth-menu".to_string();
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let widgets = config.menus().bluetooth_menu().widgets().get();
                    sender_clone.input(MenuInput::SetWidget(widgets));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let minimum_width = config.menus().bluetooth_menu().minimum_width().get();
                    sender_clone.input(MenuInput::SetMinimumWidth(minimum_width));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let maximum_height = config.menus().bluetooth_menu().maximum_height().get();
                    sender_clone.input(MenuInput::SetMaximumHeight(maximum_height));
                });
            }
            MenuType::CpuDashboard => {
                css_class = "cpu-dashboard-menu".to_string();
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let widgets = config.menus().cpu_dashboard_menu().widgets().get();
                    sender_clone.input(MenuInput::SetWidget(widgets));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let minimum_width =
                        config.menus().cpu_dashboard_menu().minimum_width().get();
                    sender_clone.input(MenuInput::SetMinimumWidth(minimum_width));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let maximum_height =
                        config.menus().cpu_dashboard_menu().maximum_height().get();
                    sender_clone.input(MenuInput::SetMaximumHeight(maximum_height));
                });
            }
            MenuType::AudioDashboard => {
                // Same card-stack chrome as the Bluetooth menu so
                // the AudioOut / AudioIn revealer rows get the
                // polished surface-variant card treatment.
                css_class = "quick-settings-menu audio-dashboard-menu".to_string();
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let widgets = config.menus().audio_dashboard_menu().widgets().get();
                    sender_clone.input(MenuInput::SetWidget(widgets));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let minimum_width =
                        config.menus().audio_dashboard_menu().minimum_width().get();
                    sender_clone.input(MenuInput::SetMinimumWidth(minimum_width));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let maximum_height =
                        config.menus().audio_dashboard_menu().maximum_height().get();
                    sender_clone.input(MenuInput::SetMaximumHeight(maximum_height));
                });
            }
            MenuType::SystemUpdate => {
                css_class = "system-update-menu".to_string();
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let widgets = config.menus().system_update_menu().widgets().get();
                    sender_clone.input(MenuInput::SetWidget(widgets));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let minimum_width =
                        config.menus().system_update_menu().minimum_width().get();
                    sender_clone.input(MenuInput::SetMinimumWidth(minimum_width));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let maximum_height =
                        config.menus().system_update_menu().maximum_height().get();
                    sender_clone.input(MenuInput::SetMaximumHeight(maximum_height));
                });
            }
            MenuType::Valent => {
                css_class = "valent-menu".to_string();
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let widgets = config.menus().valent_menu().widgets().get();
                    sender_clone.input(MenuInput::SetWidget(widgets));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let minimum_width = config.menus().valent_menu().minimum_width().get();
                    sender_clone.input(MenuInput::SetMinimumWidth(minimum_width));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let maximum_height = config.menus().valent_menu().maximum_height().get();
                    sender_clone.input(MenuInput::SetMaximumHeight(maximum_height));
                });
            }
            MenuType::Weather => {
                css_class = "weather-menu".to_string();
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let widgets = config.menus().weather_menu().widgets().get();
                    sender_clone.input(MenuInput::SetWidget(widgets));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let minimum_width = config.menus().weather_menu().minimum_width().get();
                    sender_clone.input(MenuInput::SetMinimumWidth(minimum_width));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let maximum_height = config.menus().weather_menu().maximum_height().get();
                    sender_clone.input(MenuInput::SetMaximumHeight(maximum_height));
                });
            }
            MenuType::KeepAwake => {
                css_class = "keep-awake-menu".to_string();
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let widgets = config.menus().keep_awake_menu().widgets().get();
                    sender_clone.input(MenuInput::SetWidget(widgets));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let minimum_width = config.menus().keep_awake_menu().minimum_width().get();
                    sender_clone.input(MenuInput::SetMinimumWidth(minimum_width));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let maximum_height =
                        config.menus().keep_awake_menu().maximum_height().get();
                    sender_clone.input(MenuInput::SetMaximumHeight(maximum_height));
                });
            }
            MenuType::Twilight => {
                css_class = "twilight-menu".to_string();
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let widgets = config.menus().twilight_menu().widgets().get();
                    sender_clone.input(MenuInput::SetWidget(widgets));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let minimum_width = config.menus().twilight_menu().minimum_width().get();
                    sender_clone.input(MenuInput::SetMinimumWidth(minimum_width));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let maximum_height = config.menus().twilight_menu().maximum_height().get();
                    sender_clone.input(MenuInput::SetMaximumHeight(maximum_height));
                });
            }
            MenuType::Keybinds => {
                css_class = "keybinds-menu".to_string();
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let widgets = config.menus().keybinds_menu().widgets().get();
                    sender_clone.input(MenuInput::SetWidget(widgets));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let minimum_width = config.menus().keybinds_menu().minimum_width().get();
                    sender_clone.input(MenuInput::SetMinimumWidth(minimum_width));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let maximum_height = config.menus().keybinds_menu().maximum_height().get();
                    sender_clone.input(MenuInput::SetMaximumHeight(maximum_height));
                });
            }
            MenuType::SshSessions => {
                css_class = "ssh-sessions-menu".to_string();
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let widgets = config.menus().ssh_menu().widgets().get();
                    sender_clone.input(MenuInput::SetWidget(widgets));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let minimum_width = config.menus().ssh_menu().minimum_width().get();
                    sender_clone.input(MenuInput::SetMinimumWidth(minimum_width));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let maximum_height = config.menus().ssh_menu().maximum_height().get();
                    sender_clone.input(MenuInput::SetMaximumHeight(maximum_height));
                });
            }
            MenuType::Power => {
                css_class = "power-menu".to_string();
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let widgets = config.menus().power_menu().widgets().get();
                    sender_clone.input(MenuInput::SetWidget(widgets));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let minimum_width = config.menus().power_menu().minimum_width().get();
                    sender_clone.input(MenuInput::SetMinimumWidth(minimum_width));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let maximum_height = config.menus().power_menu().maximum_height().get();
                    sender_clone.input(MenuInput::SetMaximumHeight(maximum_height));
                });
            }
            MenuType::MediaPlayer => {
                css_class = "media-player-menu".to_string();
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let widgets = config.menus().media_player_menu().widgets().get();
                    sender_clone.input(MenuInput::SetWidget(widgets));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let minimum_width = config.menus().media_player_menu().minimum_width().get();
                    sender_clone.input(MenuInput::SetMinimumWidth(minimum_width));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let maximum_height = config.menus().media_player_menu().maximum_height().get();
                    sender_clone.input(MenuInput::SetMaximumHeight(maximum_height));
                });
            }
            MenuType::Session => {
                css_class = "session-menu".to_string();
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let widgets = config.menus().session_menu().widgets().get();
                    sender_clone.input(MenuInput::SetWidget(widgets));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let minimum_width = config.menus().session_menu().minimum_width().get();
                    sender_clone.input(MenuInput::SetMinimumWidth(minimum_width));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let maximum_height = config.menus().session_menu().maximum_height().get();
                    sender_clone.input(MenuInput::SetMaximumHeight(maximum_height));
                });
            }
            MenuType::Dashboard => {
                // Same card-stack CSS as quick-settings — dashboard
                // reuses the .quick-settings-menu class so all the
                // surface-variant card + hero clock rules apply.
                css_class = "quick-settings-menu dashboard-menu".to_string();
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let widgets = config.menus().dashboard_menu().widgets().get();
                    sender_clone.input(MenuInput::SetWidget(widgets));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let minimum_width = config.menus().dashboard_menu().minimum_width().get();
                    sender_clone.input(MenuInput::SetMinimumWidth(minimum_width));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let maximum_height = config.menus().dashboard_menu().maximum_height().get();
                    sender_clone.input(MenuInput::SetMaximumHeight(maximum_height));
                });
            }
            MenuType::MargoLayout => {
                css_class = "quick-settings-menu margo-layout-menu".to_string();
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let widgets = config.menus().margo_layout_menu().widgets().get();
                    sender_clone.input(MenuInput::SetWidget(widgets));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let minimum_width = config.menus().margo_layout_menu().minimum_width().get();
                    sender_clone.input(MenuInput::SetMinimumWidth(minimum_width));
                });
                let config = base_config.clone();
                let sender_clone = sender.clone();
                effects.push(move |_| {
                    let config = config.clone();
                    let maximum_height = config.menus().margo_layout_menu().maximum_height().get();
                    sender_clone.input(MenuInput::SetMaximumHeight(maximum_height));
                });
            }
        }

        let model = MenuModel {
            widget_controllers: Vec::new(),
            widget_kinds: Vec::new(),
            minimum_width: 410,
            maximum_height: 0,
            css_class,
            weather_all_in_one: matches!(params.menu_type, MenuType::Weather),
            built: false,
            lazy_wizard: matches!(params.menu_type, MenuType::Wizard),
            _effects: effects,
        };

        let widgets = view_output!();

        // Apply per-menu CSS classes one-by-one so multi-class
        // strings like dashboard's `"quick-settings-menu
        // dashboard-menu"` register as two separate classes —
        // letting `.quick-settings-menu .network-menu-widget`
        // rules match descendants of the dashboard root.
        let mut classes: Vec<&str> = vec!["menu-scroll-window"];
        classes.extend(model.css_class.split_whitespace());
        widgets.scrolled_window.set_css_classes(&classes);

        if let MenuType::Wallpaper = params.menu_type {
            widgets.widget_container.set_margin_all(8);
        }

        // Lazy content build. The menu lives inside a Revealer→Stack, so
        // GTK maps its root only when the menu is actually shown — `map`
        // / `unmap` therefore mark open/close for *every* menu, regardless
        // of whether the frame's per-name RevealChanged dispatch covers
        // it. The first `map` builds the content (deferred from init);
        // both also drive the inner widgets' reveal state.
        let map_sender = sender.clone();
        widgets.scrolled_window.connect_map(move |_| {
            map_sender.input(MenuInput::RevealChanged(true));
        });
        let unmap_sender = sender.clone();
        widgets.scrolled_window.connect_unmap(move |_| {
            unmap_sender.input(MenuInput::RevealChanged(false));
        });

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
            MenuInput::RevealChanged(visible) => {
                // Build the content tree on first reveal (deferred from
                // init) — most menus are never opened, so this skips ~30
                // GTK tree builds per monitor at startup.
                if visible && !self.built {
                    if self.lazy_wizard {
                        // Dedicated widget, not config-driven: build it
                        // (sets `built`) instead of the widget_kinds path.
                        sender.input(MenuInput::AddWizardWidget);
                    } else {
                        self.build_content(&widgets.widget_container, &sender);
                    }
                }
                // Let widgets that care know they are being revealed
                for controller in &self.widget_controllers {
                    if let Some(controller) =
                        controller.downcast_ref::<Controller<AppLauncherModel>>()
                    {
                        controller
                            .sender()
                            .send(AppLauncherInput::ParentRevealChanged(visible))
                            .ok();
                    }
                    if let Some(controller) =
                        controller.downcast_ref::<Controller<NetworkToggleMenuWidgetModel>>()
                    {
                        controller
                            .sender()
                            .send(NetworkToggleMenuWidgetInput::ParentRevealChanged(visible))
                            .ok();
                    }
                    if let Some(controller) =
                        controller.downcast_ref::<Controller<BluetoothMenuWidgetModel>>()
                    {
                        controller
                            .sender()
                            .send(BluetoothMenuWidgetInput::ParentRevealChanged(visible))
                            .ok();
                    }
                    if let Some(controller) =
                        controller.downcast_ref::<Controller<AudioOutMenuWidgetModel>>()
                    {
                        controller
                            .sender()
                            .send(AudioOutMenuWidgetInput::ParentRevealChanged(visible))
                            .ok();
                    }
                    if let Some(controller) =
                        controller.downcast_ref::<Controller<AudioInMenuWidgetModel>>()
                    {
                        controller
                            .sender()
                            .send(AudioInMenuWidgetInput::ParentRevealChanged(visible))
                            .ok();
                    }
                    if let Some(controller) =
                        controller.downcast_ref::<Controller<ScreenshareMenuWidgetModel>>()
                    {
                        controller
                            .sender()
                            .send(ScreenshareMenuWidgetInput::ParentRevealChanged(visible))
                            .ok();
                    }
                    if let Some(controller) =
                        controller.downcast_ref::<Controller<WallpaperMenuWidgetModel>>()
                    {
                        controller
                            .sender()
                            .send(WallpaperMenuWidgetInput::ParentRevealChanged(visible))
                            .ok();
                    }
                    if let Some(controller) =
                        controller.downcast_ref::<Controller<SessionMenuWidgetModel>>()
                    {
                        controller
                            .sender()
                            .send(SessionMenuWidgetInput::ParentRevealChanged(visible))
                            .ok();
                    }
                    if let Some(controller) =
                        controller.downcast_ref::<Controller<ClipboardModel>>()
                    {
                        controller
                            .sender()
                            .send(ClipboardInput::ParentRevealChanged(visible))
                            .ok();
                    }
                    if let Some(controller) =
                        controller.downcast_ref::<Controller<NotificationsModel>>()
                    {
                        controller
                            .sender()
                            .send(NotificationsInput::ParentRevealChanged(visible))
                            .ok();
                    }
                    if let Some(controller) =
                        controller.downcast_ref::<Controller<SystemUpdateMenuWidgetModel>>()
                    {
                        controller
                            .sender()
                            .send(SystemUpdateMenuWidgetInput::ParentRevealChanged(visible))
                            .ok();
                    }
                    if let Some(controller) =
                        controller.downcast_ref::<Controller<IpMenuWidgetModel>>()
                    {
                        controller
                            .sender()
                            .send(IpMenuWidgetInput::ParentRevealChanged(visible))
                            .ok();
                    }
                    if let Some(controller) =
                        controller.downcast_ref::<Controller<DnsMenuWidgetModel>>()
                    {
                        controller
                            .sender()
                            .send(DnsMenuWidgetInput::ParentRevealChanged(visible))
                            .ok();
                    }
                    if let Some(controller) =
                        controller.downcast_ref::<Controller<UfwMenuWidgetModel>>()
                    {
                        controller
                            .sender()
                            .send(UfwMenuWidgetInput::ParentRevealChanged(visible))
                            .ok();
                    }
                    if let Some(controller) =
                        controller.downcast_ref::<Controller<PodmanMenuWidgetModel>>()
                    {
                        controller
                            .sender()
                            .send(PodmanMenuWidgetInput::ParentRevealChanged(visible))
                            .ok();
                    }
                    if let Some(controller) =
                        controller.downcast_ref::<Controller<NetworkMenuWidgetModel>>()
                    {
                        controller
                            .sender()
                            .send(NetworkMenuWidgetInput::ParentRevealChanged(visible))
                            .ok();
                    }
                }
            }
            MenuInput::ClipboardExitSearch => {
                for controller in &self.widget_controllers {
                    if let Some(controller) =
                        controller.downcast_ref::<Controller<ClipboardModel>>()
                    {
                        controller
                            .sender()
                            .send(ClipboardInput::ExitSearch)
                            .ok();
                    }
                }
            }
            MenuInput::SetWidget(menu_widgets) => {
                // Skip the destructive clear+rebuild when the config
                // layer re-notifies with an identical widget list —
                // see the `widget_kinds` field comment.
                if self.widget_kinds != menu_widgets {
                    self.widget_kinds = menu_widgets;
                    // Only build now if the content is already live (a
                    // config hot-reload while the menu is open). Otherwise
                    // the build is deferred to the first reveal — see
                    // `RevealChanged` and the `map` hook in `init`.
                    if self.built {
                        self.build_content(&widgets.widget_container, &sender);
                    }
                }
            }
            MenuInput::SetMinimumWidth(width) => {
                self.minimum_width = width;
            }
            MenuInput::SetMaximumHeight(height) => {
                self.maximum_height = height;
            }
            MenuInput::AddHyprlandScreenshareWidget => {
                let controller = Box::new(
                    ScreenshareMenuWidgetModel::builder()
                        .launch(ScreenshareMenuWidgetInit {})
                        .forward(sender.output_sender(), |msg| match msg {
                            ScreenshareMenuWidgetOutput::CloseMenu => MenuOutput::CloseMenu,
                        }),
                );
                widgets.widget_container.append(&controller.root_widget());
                self.widget_controllers.push(controller);
                // The screenshare menu builds its one widget eagerly here
                // (not from the config-driven `widget_kinds`), so mark it
                // built: otherwise the lazy first-reveal `build_content`
                // would clear+rebuild from the empty `widget_kinds` and
                // destroy this widget — taking the pending portal reply
                // Sender with it (→ the picker returns empty → screen-share
                // "user cancelled"). See `RevealChanged`.
                self.built = true;
            }
            MenuInput::AddWizardWidget => {
                let controller = Box::new(
                    WizardMenuWidgetModel::builder()
                        .launch(WizardMenuWidgetInit {})
                        .forward(sender.output_sender(), |msg| match msg {
                            WizardMenuWidgetOutput::CloseMenu => MenuOutput::CloseMenu,
                        }),
                );
                widgets.widget_container.append(&controller.root_widget());
                self.widget_controllers.push(controller);
                // Eagerly built (not from config widget_kinds) — mark built
                // so the lazy first-reveal rebuild doesn't wipe it.
                self.built = true;
            }
            MenuInput::ForwardHyprlandScreenshareReply(reply, payload) => {
                if let Some(first_controller) = self.widget_controllers.first()
                    && let Some(controller) =
                        first_controller.downcast_ref::<Controller<ScreenshareMenuWidgetModel>>()
                {
                    controller
                        .sender()
                        .send(ScreenshareMenuWidgetInput::SetReply(reply, payload))
                        .ok();
                }
            }
            MenuInput::AppLauncherSelectCategory(label) => {
                // Forward to the AppLauncherModel if this menu
                // hosts one. The launcher widget is the only
                // controller in the AppLauncher menu's widget
                // list (per `dashboard_menu.widgets =
                // [AppLauncher]` in the default config), but we
                // scan-and-downcast to stay robust against future
                // configs that interleave other widgets.
                for controller in &self.widget_controllers {
                    if let Some(launcher) =
                        controller.downcast_ref::<Controller<AppLauncherModel>>()
                    {
                        launcher
                            .sender()
                            .send(AppLauncherInput::SelectCategory(label.clone()))
                            .ok();
                        break;
                    }
                }
            }
        }
        self.update_view(widgets, sender);
    }
}

impl MenuModel {
    /// (Re)build the content widget tree from `widget_kinds` into the
    /// container. Called on the first reveal (lazy startup deferral) and
    /// on config hot-reload while the menu is already open.
    fn build_content(&mut self, container: &gtk::Box, sender: &ComponentSender<Self>) {
        clear_box(container);
        self.widget_controllers.clear();
        let weather_all_in_one = self.weather_all_in_one;
        // Move the kinds out so the build loop isn't holding an immutable
        // borrow of `self` while it pushes into `widget_controllers`.
        let kinds = std::mem::take(&mut self.widget_kinds);
        for item in &kinds {
            // The standalone weather menu stacks all sections; every other
            // host (the dashboard) keeps the compact paged weather view.
            let controller =
                build_widget(item, gtk::Orientation::Vertical, sender, weather_all_in_one);
            container.append(&controller.root_widget());
            self.widget_controllers.push(controller);
        }
        self.widget_kinds = kinds;
        self.built = true;
    }
}

impl Debug for MenuModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MenuModel").finish()
    }
}
