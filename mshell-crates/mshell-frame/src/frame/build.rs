//! Child-controller construction and GTK-stack attach/detach for `Frame`.
//!
//! Extracted from frame.rs: the self-less builders that spin up the bar and
//! menu relm4 controllers (and translate their `*Output` messages back into
//! `FrameInput`), the declarative-plugin menu content builder + per-plugin
//! layout application, and the `add_to_stack` / `remove_from_stack` helpers
//! that name-attach a widget into one of the eight positional `gtk::Stack`s.
//!
//! `pub(super)` because `Frame`'s own methods (the parent module) call these;
//! kept as a child module so `Self`, `Frame`'s fields, the free helpers
//! (`plugin_key_from_widget`, `position_from_kebab`, `run_plugin_cmd`) and
//! every imported type resolve through `use super::*`.

use super::*;

impl Frame {
    pub(super) fn add_to_stack(
        widgets: &FrameWidgets,
        widget: &Widget,
        name: &str,
        position: &Position,
    ) {
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

    pub(super) fn build_bar(
        sender: &ComponentSender<Self>,
        bar_type: BarType,
    ) -> Controller<BarModel> {
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

    pub(super) fn build_menu(
        sender: &ComponentSender<Self>,
        menu_type: MenuType,
    ) -> Controller<MenuModel> {
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
    pub(super) fn apply_plugin_layout(&mut self, widget_name: &str, widgets: &FrameWidgets) {
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
    pub(super) fn remove_from_stack(widgets: &FrameWidgets, widget: &Widget, position: &Position) {
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
    pub(super) fn build_plugin_menu_content(
        rows: &[CustomMenuRow],
        sender: &ComponentSender<Self>,
    ) -> Widget {
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
