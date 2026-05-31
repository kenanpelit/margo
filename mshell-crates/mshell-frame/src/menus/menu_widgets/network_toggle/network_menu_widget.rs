use crate::common_widgets::revealer_row::revealer_row::{
    RevealerRowInit, RevealerRowInput, RevealerRowModel, RevealerRowOutput,
};
use crate::common_widgets::revealer_row::revealer_row_label::{
    RevealerRowLabelInit, RevealerRowLabelModel,
};
use crate::menus::menu_widgets::network_toggle::network_revealed_content::{
    NetworkRevealedContentInit, NetworkRevealedContentInput, NetworkRevealedContentModel,
};
use mshell_common::WatcherToken;
use mshell_services::network_service;
use mshell_utils::network::{
    set_network_icon, set_network_label, spawn_network_watcher, spawn_wifi_watcher,
    spawn_wired_watcher,
};
use relm4::gtk::glib;
use relm4::gtk::prelude::WidgetExt;
use relm4::{Component, ComponentController, ComponentParts, ComponentSender, Controller, gtk};
use std::time::Duration;
use tokio::select;

pub(crate) struct NetworkToggleMenuWidgetModel {
    revealer_row: Controller<RevealerRowModel<RevealerRowLabelModel, NetworkRevealedContentModel>>,
    wifi_watcher_token: WatcherToken,
    wired_watcher_token: WatcherToken,
    scan_token: WatcherToken,
}

#[derive(Debug)]
pub(crate) enum NetworkToggleMenuWidgetInput {
    UpdateState,
    RevealerRowRevealed,
    RevealerRowHidden,
    ActionButtonClicked,
    ResetChildren,
    ParentRevealChanged(bool),
}

#[derive(Debug)]
pub(crate) enum NetworkToggleMenuWidgetOutput {}

pub(crate) struct NetworkToggleMenuWidgetInit {}

#[derive(Debug)]
pub(crate) enum NetworkToggleMenuWidgetCommandOutput {
    StateChanged,
    WifiChanged,
    WiredChanged,
}

#[relm4::component(pub)]
impl Component for NetworkToggleMenuWidgetModel {
    type CommandOutput = NetworkToggleMenuWidgetCommandOutput;
    type Input = NetworkToggleMenuWidgetInput;
    type Output = NetworkToggleMenuWidgetOutput;
    type Init = NetworkToggleMenuWidgetInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "network-menu-widget",

            model.revealer_row.widget().clone() {},
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        spawn_network_watcher(
            &sender,
            || NetworkToggleMenuWidgetCommandOutput::StateChanged,
            || NetworkToggleMenuWidgetCommandOutput::WifiChanged,
            || NetworkToggleMenuWidgetCommandOutput::WiredChanged,
        );

        let row_content = RevealerRowLabelModel::builder()
            .launch(RevealerRowLabelInit {
                label: "No Connection".to_string(),
            })
            .detach();

        let network_revealed_content = NetworkRevealedContentModel::builder()
            .launch(NetworkRevealedContentInit {})
            .detach();

        let revealer_row =
            RevealerRowModel::<RevealerRowLabelModel, NetworkRevealedContentModel>::builder()
                .launch(RevealerRowInit {
                    icon_name: "network-wireless-disabled-symbolic".into(),
                    action_button_sensitive: false,
                    content: row_content,
                    revealed_content: network_revealed_content,
                })
                .forward(sender.input_sender(), |msg| match msg {
                    RevealerRowOutput::ActionButtonClicked => {
                        NetworkToggleMenuWidgetInput::ActionButtonClicked
                    }
                    RevealerRowOutput::Revealed => {
                        NetworkToggleMenuWidgetInput::RevealerRowRevealed
                    }
                    RevealerRowOutput::Hidden => NetworkToggleMenuWidgetInput::RevealerRowHidden,
                });

        let model = NetworkToggleMenuWidgetModel {
            revealer_row,
            wifi_watcher_token: WatcherToken::new(),
            wired_watcher_token: WatcherToken::new(),
            scan_token: WatcherToken::new(),
        };

        let widgets = view_output!();

        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        _widgets: &mut Self::Widgets,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            NetworkToggleMenuWidgetInput::UpdateState => {
                set_network_icon(&self.revealer_row.widgets().action_icon_image);
                set_network_label(&self.revealer_row.model().content.widgets().label);
            }
            NetworkToggleMenuWidgetInput::RevealerRowRevealed => {
                let network = network_service();
                if let Some(wifi) = network.wifi.get() {
                    self.revealer_row
                        .model()
                        .revealed_content
                        .emit(NetworkRevealedContentInput::SetScanning(true));
                    let sender = self.revealer_row.model().revealed_content.sender().clone();

                    let token = self.scan_token.reset(); // cancel previous, get new token
                    tokio::spawn(async move {
                        let _ = wifi.device.request_scan().await;
                        select! {
                            _ = tokio::time::sleep(Duration::from_secs(15)) => {
                                glib::idle_add_once(move || {
                                    sender.emit(NetworkRevealedContentInput::SetScanning(false));
                                });
                            }
                            _ = token.cancelled() => {}
                        }
                        loop {
                            select! {
                                _ = tokio::time::sleep(Duration::from_secs(30)) => {
                                    let _ = wifi.device.request_scan().await;
                                }
                                _ = token.cancelled() => return,
                            }
                        }
                    });
                }
            }
            NetworkToggleMenuWidgetInput::RevealerRowHidden => {
                sender.input(NetworkToggleMenuWidgetInput::ResetChildren);
            }
            NetworkToggleMenuWidgetInput::ActionButtonClicked => {}
            NetworkToggleMenuWidgetInput::ResetChildren => {
                self.scan_token.reset();
                self.revealer_row
                    .model()
                    .revealed_content
                    .emit(NetworkRevealedContentInput::Reset);
            }
            NetworkToggleMenuWidgetInput::ParentRevealChanged(revealed) => {
                if !revealed {
                    sender.input(NetworkToggleMenuWidgetInput::ResetChildren);
                    self.revealer_row.emit(RevealerRowInput::SetRevealed(false))
                }
            }
        }
    }

    fn update_cmd(
        &mut self,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            NetworkToggleMenuWidgetCommandOutput::StateChanged => {
                sender.input(NetworkToggleMenuWidgetInput::UpdateState)
            }
            NetworkToggleMenuWidgetCommandOutput::WifiChanged => {
                let token = self.wifi_watcher_token.reset();
                spawn_wifi_watcher(&sender, token, || {
                    NetworkToggleMenuWidgetCommandOutput::StateChanged
                });
            }
            NetworkToggleMenuWidgetCommandOutput::WiredChanged => {
                let token = self.wired_watcher_token.reset();
                spawn_wired_watcher(&sender, token, || {
                    NetworkToggleMenuWidgetCommandOutput::StateChanged
                });
            }
        }
    }
}
