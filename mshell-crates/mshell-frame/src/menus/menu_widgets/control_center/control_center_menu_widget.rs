//! Control Center menu widget — the panel content for
//! `MenuType::ControlCenter`.
//!
//! The body is a `gtk::Stack` with a slide-left-right transition:
//!   * `"main"` page — the existing sliders + tile grid.
//!   * `"wifi"`, `"bluetooth"`, `"audio_out"`, `"mic"`, `"battery"`,
//!     `"vpn"`, `"valent"` — detail sub-pages, each with a back-arrow
//!     row + an embedded detail component.
//!
//! Clicking an expandable tile → stack slides to the detail page and
//! the component's reveal input is emitted so it starts scanning/loading
//! lazily. The back arrow slides back to `"main"` and emits the matching
//! hidden input.

use crate::menus::menu_widgets::audio_in::audio_in_revealed_content::{
    AudioInRevealedContentInit, AudioInRevealedContentInput, AudioInRevealedContentModel,
};
use crate::menus::menu_widgets::audio_out::audio_out_revealed_content::{
    AudioOutRevealedContentInit, AudioOutRevealedContentInput, AudioOutRevealedContentModel,
};
use crate::menus::menu_widgets::bluetooth::bluetooth_menu_widget::{
    BluetoothMenuWidgetInit, BluetoothMenuWidgetInput, BluetoothMenuWidgetModel,
};
use crate::menus::menu_widgets::control_center::header::{
    ControlCenterHeaderInit, ControlCenterHeaderInput, ControlCenterHeaderModel,
    ControlCenterHeaderOutput,
};
use crate::menus::menu_widgets::control_center::sliders::{
    ControlCenterSlidersInit, ControlCenterSlidersModel, ControlCenterSlidersOutput,
};
use crate::menus::menu_widgets::control_center::tiles::{
    ControlCenterTilesInit, ControlCenterTilesInput, ControlCenterTilesModel,
    ControlCenterTilesOutput, DetailPage,
};
use crate::menus::menu_widgets::dns::dns_menu_widget::{
    DnsMenuWidgetInit, DnsMenuWidgetInput, DnsMenuWidgetModel,
};
use crate::menus::menu_widgets::keep_awake::keep_awake_menu_widget::{
    KeepAwakeMenuWidgetInit, KeepAwakeMenuWidgetInput, KeepAwakeMenuWidgetModel,
};
use crate::menus::menu_widgets::network::network_menu_widget::{
    NetworkMenuWidgetInit, NetworkMenuWidgetInput, NetworkMenuWidgetModel,
};
use crate::menus::menu_widgets::podman::podman_menu_widget::{
    PodmanMenuWidgetInit, PodmanMenuWidgetInput, PodmanMenuWidgetModel,
};
use crate::menus::menu_widgets::power::power_menu_widget::{
    PowerMenuWidgetInit, PowerMenuWidgetModel,
};
use crate::menus::menu_widgets::twilight::twilight_menu_widget::{
    TwilightMenuWidgetInit, TwilightMenuWidgetInput, TwilightMenuWidgetModel,
};
use crate::menus::menu_widgets::ufw::ufw_menu_widget::{
    UfwMenuWidgetInit, UfwMenuWidgetInput, UfwMenuWidgetModel,
};
use crate::menus::menu_widgets::valent::valent_menu_widget::{
    ValentMenuWidgetInit, ValentMenuWidgetInput, ValentMenuWidgetModel,
};
use relm4::gtk::prelude::{BoxExt, ButtonExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentController, ComponentParts, ComponentSender, Controller, gtk};

// ── Page name constants ────────────────────────────────────────────────────────

const PAGE_MAIN: &str = "main";
const PAGE_WIFI: &str = "wifi";
const PAGE_BLUETOOTH: &str = "bluetooth";
const PAGE_AUDIO_OUT: &str = "audio_out";
const PAGE_MIC: &str = "mic";
const PAGE_BATTERY: &str = "battery";
const PAGE_VPN: &str = "vpn";
const PAGE_VALENT: &str = "valent";
const PAGE_TWILIGHT: &str = "twilight";
const PAGE_KEEP_AWAKE: &str = "keep_awake";
const PAGE_UFW: &str = "ufw";
const PAGE_PODMAN: &str = "podman";

// ── Model ─────────────────────────────────────────────────────────────────────

pub(crate) struct ControlCenterMenuWidgetModel {
    header: Controller<ControlCenterHeaderModel>,
    /// Held for widget lifetime; widget is embedded in the stack main page.
    #[allow(dead_code)]
    sliders: Controller<ControlCenterSlidersModel>,
    tiles: Controller<ControlCenterTilesModel>,
    // Detail page components — held to keep widget alive and for emit calls.
    wifi_detail: Controller<NetworkMenuWidgetModel>,
    bt_detail: Controller<BluetoothMenuWidgetModel>,
    audio_out_detail: Controller<AudioOutRevealedContentModel>,
    mic_detail: Controller<AudioInRevealedContentModel>,
    /// Held for widget lifetime; power detail has no separate lazy-load signal.
    #[allow(dead_code)]
    battery_detail: Controller<PowerMenuWidgetModel>,
    vpn_detail: Controller<DnsMenuWidgetModel>,
    valent_detail: Controller<ValentMenuWidgetModel>,
    twilight_detail: Controller<TwilightMenuWidgetModel>,
    keep_awake_detail: Controller<KeepAwakeMenuWidgetModel>,
    ufw_detail: Controller<UfwMenuWidgetModel>,
    podman_detail: Controller<PodmanMenuWidgetModel>,
    /// Whether edit-mode is active.
    edit_mode: bool,
    /// The GTK Stack widget — kept so `update` can switch pages.
    stack: gtk::Stack,
}

// ── Messages ──────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub(crate) enum ControlCenterMenuWidgetInput {
    ParentRevealChanged(bool),
    /// Forwarded from header output.
    ToggleEdit,
    /// Header SessionPower icon → ask the frame to open the session menu.
    RequestSessionMenu,
    /// Forwarded from header Lock/Settings outputs (handled in the header).
    _HeaderActionHandled,
    /// Tile grid emitted an expand request — open the matching detail page.
    OpenDetailPage(DetailPage),
    /// Back arrow on any detail page — return to the grid.
    BackToMain,
}

#[derive(Debug)]
pub(crate) enum ControlCenterMenuWidgetOutput {
    /// Open the session / power menu (the header power icon).
    ToggleSessionMenu,
}

pub(crate) struct ControlCenterMenuWidgetInit {}

// ── Component ─────────────────────────────────────────────────────────────────

#[relm4::component(pub(crate))]
impl Component for ControlCenterMenuWidgetModel {
    type CommandOutput = ();
    type Input = ControlCenterMenuWidgetInput;
    type Output = ControlCenterMenuWidgetOutput;
    type Init = ControlCenterMenuWidgetInit;

    view! {
        #[root]
        #[name = "root_box"]
        gtk::Box {
            add_css_class: "control-center-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 16,
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        // ── Header ──────────────────────────────────────────────────────────
        let header = ControlCenterHeaderModel::builder()
            .launch(ControlCenterHeaderInit {})
            .forward(sender.input_sender(), |msg| match msg {
                ControlCenterHeaderOutput::Lock => {
                    ControlCenterMenuWidgetInput::_HeaderActionHandled
                }
                ControlCenterHeaderOutput::SessionPower => {
                    ControlCenterMenuWidgetInput::RequestSessionMenu
                }
                ControlCenterHeaderOutput::Settings => {
                    ControlCenterMenuWidgetInput::_HeaderActionHandled
                }
                ControlCenterHeaderOutput::ToggleEdit => ControlCenterMenuWidgetInput::ToggleEdit,
            });

        // ── Main-page components ─────────────────────────────────────────────
        let sliders = ControlCenterSlidersModel::builder()
            .launch(ControlCenterSlidersInit {})
            .forward(sender.input_sender(), |msg| match msg {
                ControlCenterSlidersOutput::OpenAudioOut => {
                    ControlCenterMenuWidgetInput::OpenDetailPage(DetailPage::AudioOut)
                }
                ControlCenterSlidersOutput::OpenMic => {
                    ControlCenterMenuWidgetInput::OpenDetailPage(DetailPage::Mic)
                }
            });

        let tiles = ControlCenterTilesModel::builder()
            .launch(ControlCenterTilesInit {})
            .forward(sender.input_sender(), |msg| match msg {
                ControlCenterTilesOutput::ExpandPage(page) => {
                    ControlCenterMenuWidgetInput::OpenDetailPage(page)
                }
            });

        // ── Detail-page components ───────────────────────────────────────────
        let wifi_detail = NetworkMenuWidgetModel::builder()
            .launch(NetworkMenuWidgetInit {})
            .detach();

        let bt_detail = BluetoothMenuWidgetModel::builder()
            .launch(BluetoothMenuWidgetInit {})
            .detach();

        let audio_out_detail = AudioOutRevealedContentModel::builder()
            .launch(AudioOutRevealedContentInit {})
            .detach();

        let mic_detail = AudioInRevealedContentModel::builder()
            .launch(AudioInRevealedContentInit {})
            .detach();

        let battery_detail = PowerMenuWidgetModel::builder()
            .launch(PowerMenuWidgetInit {})
            .detach();

        // New detail pages: VPN (DNS menu) and Valent
        let vpn_detail = DnsMenuWidgetModel::builder()
            .launch(DnsMenuWidgetInit {})
            .detach();

        let valent_detail = ValentMenuWidgetModel::builder()
            .launch(ValentMenuWidgetInit {})
            .detach();

        let twilight_detail = TwilightMenuWidgetModel::builder()
            .launch(TwilightMenuWidgetInit {})
            .detach();

        let keep_awake_detail = KeepAwakeMenuWidgetModel::builder()
            .launch(KeepAwakeMenuWidgetInit {})
            .detach();

        let ufw_detail = UfwMenuWidgetModel::builder()
            .launch(UfwMenuWidgetInit {})
            .detach();

        let podman_detail = PodmanMenuWidgetModel::builder()
            .launch(PodmanMenuWidgetInit {})
            .detach();

        // ── Build gtk::Stack ─────────────────────────────────────────────────
        let stack = gtk::Stack::new();
        stack.set_transition_type(gtk::StackTransitionType::SlideLeftRight);
        stack.set_transition_duration(250);
        stack.set_hexpand(true);

        // Main page: sliders + grid in a vertical box
        let main_page = gtk::Box::new(gtk::Orientation::Vertical, 16);
        main_page.append(sliders.widget());
        main_page.append(tiles.widget());
        stack.add_named(&main_page, Some(PAGE_MAIN));

        // Detail pages: back-row + embedded component widget
        stack.add_named(
            &build_detail_page("Wi-Fi", sender.input_sender(), wifi_detail.widget()),
            Some(PAGE_WIFI),
        );
        stack.add_named(
            &build_detail_page("Bluetooth", sender.input_sender(), bt_detail.widget()),
            Some(PAGE_BLUETOOTH),
        );
        stack.add_named(
            &build_detail_page(
                "Audio Out",
                sender.input_sender(),
                audio_out_detail.widget(),
            ),
            Some(PAGE_AUDIO_OUT),
        );
        stack.add_named(
            &build_detail_page("Microphone", sender.input_sender(), mic_detail.widget()),
            Some(PAGE_MIC),
        );
        stack.add_named(
            &build_detail_page(
                "Battery & Power",
                sender.input_sender(),
                battery_detail.widget(),
            ),
            Some(PAGE_BATTERY),
        );
        stack.add_named(
            &build_detail_page("VPN / DNS", sender.input_sender(), vpn_detail.widget()),
            Some(PAGE_VPN),
        );
        stack.add_named(
            &build_detail_page(
                "Valent Connect",
                sender.input_sender(),
                valent_detail.widget(),
            ),
            Some(PAGE_VALENT),
        );
        stack.add_named(
            &build_detail_page("Twilight", sender.input_sender(), twilight_detail.widget()),
            Some(PAGE_TWILIGHT),
        );
        stack.add_named(
            &build_detail_page(
                "Keep Awake",
                sender.input_sender(),
                keep_awake_detail.widget(),
            ),
            Some(PAGE_KEEP_AWAKE),
        );
        stack.add_named(
            &build_detail_page("Firewall (UFW)", sender.input_sender(), ufw_detail.widget()),
            Some(PAGE_UFW),
        );
        stack.add_named(
            &build_detail_page("Podman", sender.input_sender(), podman_detail.widget()),
            Some(PAGE_PODMAN),
        );

        // Show main by default
        stack.set_visible_child_name(PAGE_MAIN);

        let model = ControlCenterMenuWidgetModel {
            header,
            sliders,
            tiles,
            wifi_detail,
            bt_detail,
            audio_out_detail,
            mic_detail,
            battery_detail,
            vpn_detail,
            valent_detail,
            twilight_detail,
            keep_awake_detail,
            ufw_detail,
            podman_detail,
            edit_mode: false,
            stack: stack.clone(),
        };

        let widgets = view_output!();

        // Prepend the header widget at the top of the root box.
        widgets.root_box.prepend(model.header.widget());
        // Append the stack (contains main page + detail pages).
        widgets.root_box.append(&stack);

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            ControlCenterMenuWidgetInput::RequestSessionMenu => {
                sender
                    .output(ControlCenterMenuWidgetOutput::ToggleSessionMenu)
                    .ok();
            }

            ControlCenterMenuWidgetInput::ParentRevealChanged(revealed) => {
                if revealed {
                    // Refresh uptime whenever the menu is opened.
                    self.header
                        .sender()
                        .send(ControlCenterHeaderInput::RecomputeUptime)
                        .ok();
                }
                // Forward reveal state to the tile grid for lazy watcher start.
                self.tiles
                    .sender()
                    .send(ControlCenterTilesInput::Reveal(revealed))
                    .ok();

                // When the whole panel hides, snap back to main so the next
                // open starts clean.
                if !revealed {
                    self.stack
                        .set_transition_type(gtk::StackTransitionType::None);
                    self.stack.set_visible_child_name(PAGE_MAIN);
                    self.stack
                        .set_transition_type(gtk::StackTransitionType::SlideLeftRight);
                    // Tell any active detail component that it's hidden.
                    self.bt_detail
                        .sender()
                        .send(BluetoothMenuWidgetInput::ParentRevealChanged(false))
                        .ok();
                    self.wifi_detail
                        .sender()
                        .send(NetworkMenuWidgetInput::ParentRevealChanged(false))
                        .ok();
                    self.audio_out_detail
                        .sender()
                        .send(AudioOutRevealedContentInput::Hidden)
                        .ok();
                    self.mic_detail
                        .sender()
                        .send(AudioInRevealedContentInput::Hidden)
                        .ok();
                    self.vpn_detail
                        .sender()
                        .send(DnsMenuWidgetInput::ParentRevealChanged(false))
                        .ok();
                    self.twilight_detail
                        .sender()
                        .send(TwilightMenuWidgetInput::ParentRevealChanged(false))
                        .ok();
                    self.keep_awake_detail
                        .sender()
                        .send(KeepAwakeMenuWidgetInput::ParentRevealChanged(false))
                        .ok();
                    self.ufw_detail
                        .sender()
                        .send(UfwMenuWidgetInput::ParentRevealChanged(false))
                        .ok();
                    self.podman_detail
                        .sender()
                        .send(PodmanMenuWidgetInput::ParentRevealChanged(false))
                        .ok();
                }
            }

            ControlCenterMenuWidgetInput::OpenDetailPage(page) => {
                let page_name = match page {
                    DetailPage::Wifi => {
                        self.wifi_detail
                            .sender()
                            .send(NetworkMenuWidgetInput::ParentRevealChanged(true))
                            .ok();
                        PAGE_WIFI
                    }
                    DetailPage::Bluetooth => {
                        self.bt_detail
                            .sender()
                            .send(BluetoothMenuWidgetInput::ParentRevealChanged(true))
                            .ok();
                        PAGE_BLUETOOTH
                    }
                    DetailPage::AudioOut => {
                        self.audio_out_detail
                            .sender()
                            .send(AudioOutRevealedContentInput::Revealed)
                            .ok();
                        PAGE_AUDIO_OUT
                    }
                    DetailPage::Mic => {
                        self.mic_detail
                            .sender()
                            .send(AudioInRevealedContentInput::Revealed)
                            .ok();
                        PAGE_MIC
                    }
                    DetailPage::Battery => PAGE_BATTERY,
                    DetailPage::Vpn => {
                        self.vpn_detail
                            .sender()
                            .send(DnsMenuWidgetInput::ParentRevealChanged(true))
                            .ok();
                        PAGE_VPN
                    }
                    DetailPage::Valent => {
                        // Valent probes on init already; re-probe on every show.
                        self.valent_detail
                            .sender()
                            .send(ValentMenuWidgetInput::Reprobe)
                            .ok();
                        PAGE_VALENT
                    }
                    DetailPage::Twilight => {
                        self.twilight_detail
                            .sender()
                            .send(TwilightMenuWidgetInput::ParentRevealChanged(true))
                            .ok();
                        PAGE_TWILIGHT
                    }
                    DetailPage::KeepAwake => {
                        self.keep_awake_detail
                            .sender()
                            .send(KeepAwakeMenuWidgetInput::ParentRevealChanged(true))
                            .ok();
                        PAGE_KEEP_AWAKE
                    }
                    DetailPage::Ufw => {
                        self.ufw_detail
                            .sender()
                            .send(UfwMenuWidgetInput::ParentRevealChanged(true))
                            .ok();
                        PAGE_UFW
                    }
                    DetailPage::Podman => {
                        self.podman_detail
                            .sender()
                            .send(PodmanMenuWidgetInput::ParentRevealChanged(true))
                            .ok();
                        PAGE_PODMAN
                    }
                };
                self.stack.set_visible_child_name(page_name);
            }

            ControlCenterMenuWidgetInput::BackToMain => {
                // Determine which page we're leaving and send Hidden/stop.
                if let Some(current) = self.stack.visible_child_name() {
                    match current.as_str() {
                        PAGE_BLUETOOTH => {
                            self.bt_detail
                                .sender()
                                .send(BluetoothMenuWidgetInput::ParentRevealChanged(false))
                                .ok();
                        }
                        PAGE_WIFI => {
                            self.wifi_detail
                                .sender()
                                .send(NetworkMenuWidgetInput::ParentRevealChanged(false))
                                .ok();
                        }
                        PAGE_AUDIO_OUT => {
                            self.audio_out_detail
                                .sender()
                                .send(AudioOutRevealedContentInput::Hidden)
                                .ok();
                        }
                        PAGE_MIC => {
                            self.mic_detail
                                .sender()
                                .send(AudioInRevealedContentInput::Hidden)
                                .ok();
                        }
                        PAGE_VPN => {
                            self.vpn_detail
                                .sender()
                                .send(DnsMenuWidgetInput::ParentRevealChanged(false))
                                .ok();
                        }
                        PAGE_TWILIGHT => {
                            self.twilight_detail
                                .sender()
                                .send(TwilightMenuWidgetInput::ParentRevealChanged(false))
                                .ok();
                        }
                        PAGE_KEEP_AWAKE => {
                            self.keep_awake_detail
                                .sender()
                                .send(KeepAwakeMenuWidgetInput::ParentRevealChanged(false))
                                .ok();
                        }
                        PAGE_UFW => {
                            self.ufw_detail
                                .sender()
                                .send(UfwMenuWidgetInput::ParentRevealChanged(false))
                                .ok();
                        }
                        PAGE_PODMAN => {
                            self.podman_detail
                                .sender()
                                .send(PodmanMenuWidgetInput::ParentRevealChanged(false))
                                .ok();
                        }
                        _ => {}
                    }
                }
                self.stack.set_visible_child_name(PAGE_MAIN);
            }

            ControlCenterMenuWidgetInput::ToggleEdit => {
                self.edit_mode = !self.edit_mode;
                self.tiles
                    .sender()
                    .send(ControlCenterTilesInput::SetEditMode(self.edit_mode))
                    .ok();
            }

            ControlCenterMenuWidgetInput::_HeaderActionHandled => {}
        }
    }
}

// ── Detail page builder ────────────────────────────────────────────────────────

/// Build the wrapper for a detail sub-page:
///   [← back btn]  [title label]
///   ─────────────────────────────
///   <body widget>
fn build_detail_page(
    title: &str,
    input_sender: &relm4::Sender<ControlCenterMenuWidgetInput>,
    body: &impl gtk::prelude::IsA<gtk::Widget>,
) -> gtk::Box {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 12);
    page.add_css_class("control-center-detail");

    // Back row
    let back_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    back_row.add_css_class("control-center-back-row");
    back_row.set_valign(gtk::Align::Center);

    let back_btn = gtk::Button::new();
    back_btn.add_css_class("control-center-back-btn");
    back_btn.set_icon_name("go-previous-symbolic");
    back_btn.set_valign(gtk::Align::Center);

    let title_label = gtk::Label::new(Some(title));
    title_label.add_css_class("control-center-detail-title");
    title_label.set_halign(gtk::Align::Start);
    title_label.set_valign(gtk::Align::Center);
    title_label.set_hexpand(true);

    back_row.append(&back_btn);
    back_row.append(&title_label);

    // Wire the back button
    let s = input_sender.clone();
    back_btn.connect_clicked(move |_| {
        s.emit(ControlCenterMenuWidgetInput::BackToMain);
    });

    // Separator
    let sep = gtk::Separator::new(gtk::Orientation::Horizontal);

    page.append(&back_row);
    page.append(&sep);
    page.append(body);

    page
}
