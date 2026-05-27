//! Control Center menu widget — the panel content for
//! `MenuType::ControlCenter`.
//!
//! Task 5: GNOME-style inline expand. The body is now a `gtk::Stack`
//! with a slide-left-right transition:
//!   * `"main"` page — the existing sliders + tile grid.
//!   * `"wifi"`, `"bluetooth"`, `"audio_out"`, `"mic"`, `"battery"` — detail
//!     sub-pages, each with a back-arrow row + an embedded detail component.
//!
//! Clicking an expandable tile → stack slides to the detail page and
//! the component's `Revealed` (or `ParentRevealChanged(true)`) input is
//! emitted so it starts scanning/loading lazily. The back arrow slides
//! back to `"main"` and emits the matching `Hidden`.

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
    ControlCenterSlidersInit, ControlCenterSlidersModel,
};
use crate::menus::menu_widgets::control_center::tiles::{
    ControlCenterTilesInit, ControlCenterTilesInput, ControlCenterTilesModel,
    ControlCenterTilesOutput, DetailPage,
};
use crate::menus::menu_widgets::network::network_menu_widget::{
    NetworkMenuWidgetInit, NetworkMenuWidgetInput, NetworkMenuWidgetModel,
};
use crate::menus::menu_widgets::power::power_menu_widget::{
    PowerMenuWidgetInit, PowerMenuWidgetModel,
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
    /// Whether edit-mode is active (inert until Task 6).
    edit_mode: bool,
    /// The GTK Stack widget — kept so `update` can switch pages.
    stack: gtk::Stack,
}

// ── Messages ──────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub(crate) enum ControlCenterMenuWidgetInput {
    ParentRevealChanged(bool),
    /// Forwarded from header output; inert until Task 6.
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
                ControlCenterHeaderOutput::ToggleEdit => {
                    ControlCenterMenuWidgetInput::ToggleEdit
                }
            });

        // ── Main-page components ─────────────────────────────────────────────
        let sliders = ControlCenterSlidersModel::builder()
            .launch(ControlCenterSlidersInit {})
            .detach();

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
            &build_detail_page("Audio Out", sender.input_sender(), audio_out_detail.widget()),
            Some(PAGE_AUDIO_OUT),
        );
        stack.add_named(
            &build_detail_page("Microphone", sender.input_sender(), mic_detail.widget()),
            Some(PAGE_MIC),
        );
        stack.add_named(
            &build_detail_page("Battery & Power", sender.input_sender(), battery_detail.widget()),
            Some(PAGE_BATTERY),
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

    fn update(
        &mut self,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
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
                    self.stack.set_transition_type(gtk::StackTransitionType::None);
                    self.stack.set_visible_child_name(PAGE_MAIN);
                    self.stack.set_transition_type(gtk::StackTransitionType::SlideLeftRight);
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
