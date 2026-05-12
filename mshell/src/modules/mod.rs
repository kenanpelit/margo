use crate::{
    app::{App, Message},
    components::animated_size,
    components::menu::MenuType,
    components::{module_group, module_item},
    config::{ModuleDef, ModuleName},
    theme::use_theme,
};
use iced::{Alignment, Element, Length, Subscription, SurfaceId, widget::Row};

pub mod custom_module;
pub mod dns;
pub mod keyboard_layout;
pub mod media_player;
pub mod network_speed;
pub mod notifications;
pub mod podman;
pub mod power;
pub mod privacy;
pub mod settings;
pub mod system_info;
pub mod tempo;
pub mod ufw;
pub mod tray;
pub mod updates;
pub mod window_title;
pub mod workspaces;

#[derive(Debug, Clone)]
pub enum OnModulePress {
    Action(Box<Message>),
    ToggleMenu(MenuType),
    ToggleMenuWithExtra {
        menu_type: MenuType,
        on_right_press: Option<Box<Message>>,
        on_scroll_up: Option<Box<Message>>,
        on_scroll_down: Option<Box<Message>>,
    },
}

impl App {
    pub fn modules_section<'a>(&'a self, id: SurfaceId) -> [Element<'a, Message>; 3] {
        let space = use_theme(|t| t.space);
        let current_menu = self.outputs.open_menu_type_for_bar(id);
        [
            &self.general_config.modules.left,
            &self.general_config.modules.center,
            &self.general_config.modules.right,
        ]
        .map(|modules_def| {
            let mut row = Row::with_capacity(modules_def.len())
                .height(Length::Shrink)
                .align_y(Alignment::Center)
                .spacing(space.xxs);

            for module_def in modules_def {
                row = row.push(match module_def {
                    // life parsing of string to module
                    ModuleDef::Single(module) => {
                        self.single_module_wrapper(id, module, current_menu.as_ref())
                    }
                    ModuleDef::Group(group) => {
                        self.group_module_wrapper(id, group, current_menu.as_ref())
                    }
                });
            }

            row.into()
        })
    }

    pub fn modules_subscriptions(&self, modules_def: &[ModuleDef]) -> Vec<Subscription<Message>> {
        modules_def
            .iter()
            .flat_map(|module_def| match module_def {
                ModuleDef::Single(module) => {
                    vec![self.get_module_subscription(module)]
                }
                ModuleDef::Group(group) => group
                    .iter()
                    .map(|module| self.get_module_subscription(module))
                    .collect(),
            })
            .flatten()
            .collect()
    }

    fn build_module_item<'a>(
        &'a self,
        id: SurfaceId,
        content: Element<'a, Message>,
        action: Option<OnModulePress>,
        current_menu: Option<&MenuType>,
        module_name: Option<&ModuleName>,
    ) -> Element<'a, Message> {
        // High-churn numeric modules (CPU%, Memory%, Temp, network
        // speed) refresh every few seconds and naturally change
        // text width by one digit-advance. Wrapping them in
        // `animated_size` would mean the bar tweens that width
        // change over ~150 ms every tick — visible as a continuous
        // shake. They render fast enough that an instant reflow is
        // imperceptible.
        let suppress_size_anim = matches!(
            module_name,
            Some(ModuleName::SystemInfo) | Some(ModuleName::NetworkSpeed)
        );
        let content = if use_theme(|t| t.animations_enabled) && !suppress_size_anim {
            animated_size(content).into()
        } else {
            content
        };
        match action {
            Some(action) => {
                let mut item = module_item(content);
                let mut active_for: Option<MenuType> = None;
                match action {
                    OnModulePress::Action(msg) => {
                        item = item.on_press(*msg);
                    }
                    OnModulePress::ToggleMenu(menu_type) => {
                        active_for = Some(menu_type.clone());
                        item = item.on_press_with_position(move |button_ui_ref| {
                            Message::ToggleMenu(menu_type.clone(), id, button_ui_ref)
                        });
                    }
                    OnModulePress::ToggleMenuWithExtra {
                        menu_type,
                        on_right_press,
                        on_scroll_up,
                        on_scroll_down,
                    } => {
                        active_for = Some(menu_type.clone());
                        item = item.on_press_with_position(move |button_ui_ref| {
                            Message::ToggleMenu(menu_type.clone(), id, button_ui_ref)
                        });
                        if let Some(msg) = on_right_press {
                            item = item.on_right_press(*msg);
                        }
                        if let Some(msg) = on_scroll_up {
                            item = item.on_scroll_up(*msg);
                        }
                        if let Some(msg) = on_scroll_down {
                            item = item.on_scroll_down(*msg);
                        }
                    }
                }
                if let (Some(my_type), Some(open_type)) = (active_for.as_ref(), current_menu)
                    && my_type == open_type
                {
                    item = item.is_active(true);
                }
                item.into()
            }
            None => module_item(content).into(),
        }
    }

    fn single_module_wrapper<'a>(
        &'a self,
        id: SurfaceId,
        module_name: &'a ModuleName,
        current_menu: Option<&MenuType>,
    ) -> Option<Element<'a, Message>> {
        self.get_module_view(id, module_name).map(|(content, action)| {
            module_group(self.build_module_item(
                id,
                content,
                action,
                current_menu,
                Some(module_name),
            ))
        })
    }

    fn group_module_wrapper<'a>(
        &'a self,
        id: SurfaceId,
        group: &'a [ModuleName],
        current_menu: Option<&MenuType>,
    ) -> Option<Element<'a, Message>> {
        let modules: Vec<_> = group
            .iter()
            .filter_map(|module| {
                self.get_module_view(id, module)
                    .map(|(c, a)| (module, c, a))
            })
            .collect();

        if modules.is_empty() {
            None
        } else {
            let items = Row::with_children(
                modules
                    .into_iter()
                    .map(|(module_name, content, action)| {
                        self.build_module_item(
                            id,
                            content,
                            action,
                            current_menu,
                            Some(module_name),
                        )
                    })
                    .collect::<Vec<_>>(),
            );
            Some(module_group(items.into()))
        }
    }

    fn get_module_view<'a>(
        &'a self,
        id: SurfaceId,
        module_name: &'a ModuleName,
    ) -> Option<(Element<'a, Message>, Option<OnModulePress>)> {
        match module_name {
            ModuleName::Custom(name) => self.custom.get(name).map(|custom| {
                let action = match custom.module_type() {
                    crate::config::CustomModuleType::Text => None,
                    crate::config::CustomModuleType::Button => {
                        Some(OnModulePress::Action(Box::new(Message::Custom(
                            name.clone(),
                            custom_module::Message::LaunchCommand,
                        ))))
                    }
                };
                (
                    custom.view().map(|msg| Message::Custom(name.clone(), msg)),
                    action,
                )
            }),
            ModuleName::Updates => self.updates.as_ref().map(|updates| {
                (
                    updates.view().map(Message::Updates),
                    Some(OnModulePress::ToggleMenu(MenuType::Updates)),
                )
            }),
            ModuleName::Workspaces => Some((
                self.workspaces
                    .view(id, &self.outputs)
                    .map(Message::Workspaces),
                None,
            )),
            ModuleName::WindowTitle => self.window_title.get_value().map(|title| {
                (
                    self.window_title.view(title).map(Message::WindowTitle),
                    None,
                )
            }),
            ModuleName::SystemInfo => Some((
                self.system_info.view().map(Message::SystemInfo),
                Some(OnModulePress::ToggleMenu(MenuType::SystemInfo)),
            )),
            ModuleName::NetworkSpeed => Some((
                self.network_speed.view().map(Message::NetworkSpeed),
                Some(OnModulePress::ToggleMenu(MenuType::NetworkSpeed)),
            )),
            ModuleName::Dns => Some((
                self.dns.view().map(Message::Dns),
                Some(OnModulePress::ToggleMenu(MenuType::Dns)),
            )),
            ModuleName::Ufw => Some((
                self.ufw.view().map(Message::Ufw),
                Some(OnModulePress::ToggleMenu(MenuType::Ufw)),
            )),
            ModuleName::Power => Some((
                self.power.view().map(Message::Power),
                Some(OnModulePress::ToggleMenu(MenuType::Power)),
            )),
            ModuleName::Podman => Some((
                self.podman.view().map(Message::Podman),
                Some(OnModulePress::ToggleMenu(MenuType::Podman)),
            )),
            ModuleName::KeyboardLayout => self.keyboard_layout.view().map(|view| {
                (
                    view.map(Message::KeyboardLayout),
                    Some(OnModulePress::Action(Box::new(Message::KeyboardLayout(
                        keyboard_layout::Message::ChangeLayout,
                    )))),
                )
            }),
            ModuleName::Tray => self
                .tray
                .view(id)
                .map(|view| (view.map(Message::Tray), None)),
            ModuleName::Tempo => Some((
                self.tempo.view().map(Message::Tempo),
                Some(OnModulePress::ToggleMenuWithExtra {
                    menu_type: MenuType::Tempo,
                    on_right_press: Some(Box::new(Message::Tempo(tempo::Message::CycleFormat))),
                    on_scroll_up: Some(Box::new(Message::Tempo(tempo::Message::CycleTimezone(
                        tempo::TimezoneDirection::Forward,
                    )))),
                    on_scroll_down: Some(Box::new(Message::Tempo(tempo::Message::CycleTimezone(
                        tempo::TimezoneDirection::Backward,
                    )))),
                }),
            )),
            ModuleName::Privacy => self
                .privacy
                .view()
                .map(|view| (view.map(Message::Privacy), None)),
            ModuleName::MediaPlayer => self.media_player.view().map(|view| {
                (
                    view.map(Message::MediaPlayer),
                    Some(OnModulePress::ToggleMenu(MenuType::MediaPlayer)),
                )
            }),
            ModuleName::Settings => Some((
                self.settings.view().map(Message::Settings),
                Some(OnModulePress::ToggleMenu(MenuType::Settings)),
            )),
            ModuleName::Notifications => Some((
                self.notifications.view().map(Message::Notifications),
                Some(OnModulePress::ToggleMenu(MenuType::Notifications)),
            )),
        }
    }

    fn get_module_subscription(&self, module_name: &ModuleName) -> Option<Subscription<Message>> {
        match module_name {
            ModuleName::Custom(name) => self.custom.get(name).map(|custom| {
                custom
                    .subscription()
                    .map(|(name, msg)| Message::Custom(name, msg))
            }),
            ModuleName::Updates => self
                .updates
                .as_ref()
                .map(|updates| updates.subscription().map(Message::Updates)),
            ModuleName::Workspaces => Some(self.workspaces.subscription().map(Message::Workspaces)),
            ModuleName::WindowTitle => {
                Some(self.window_title.subscription().map(Message::WindowTitle))
            }
            ModuleName::SystemInfo => {
                Some(self.system_info.subscription().map(Message::SystemInfo))
            }
            ModuleName::NetworkSpeed => {
                Some(self.network_speed.subscription().map(Message::NetworkSpeed))
            }
            ModuleName::Dns => Some(self.dns.subscription().map(Message::Dns)),
            ModuleName::Ufw => Some(self.ufw.subscription().map(Message::Ufw)),
            ModuleName::Power => Some(self.power.subscription().map(Message::Power)),
            ModuleName::Podman => Some(self.podman.subscription().map(Message::Podman)),
            ModuleName::KeyboardLayout => Some(
                self.keyboard_layout
                    .subscription()
                    .map(Message::KeyboardLayout),
            ),
            ModuleName::Tray => Some(self.tray.subscription().map(Message::Tray)),
            ModuleName::Tempo => Some(self.tempo.subscription().map(Message::Tempo)),
            ModuleName::Privacy => Some(self.privacy.subscription().map(Message::Privacy)),
            ModuleName::MediaPlayer => {
                Some(self.media_player.subscription().map(Message::MediaPlayer))
            }
            ModuleName::Settings => Some(self.settings.subscription().map(Message::Settings)),
            ModuleName::Notifications => Some(
                self.notifications
                    .subscription()
                    .map(Message::Notifications),
            ),
        }
    }
}
