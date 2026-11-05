//! QR menu — content surface for `MenuType::Qr` (`mshellctl qr`,
//! `mshellctl menu qr`).
//!
//! A phone-share panel: a §12 header, a segmented control **Text · Wi-Fi ·
//! Hotspot**, the code itself ([`QrView`]) and its caption.
//!   * **Text** — type or paste a URL / text and press Enter (or *Show
//!     code*); *From clipboard* (and `mshellctl qr --clipboard`) encodes the
//!     clipboard's text. *Copy image* puts the code on the clipboard as a PNG.
//!   * **Wi-Fi** / **Hotspot** — the standard `WIFI:` code of the network
//!     this machine is on / of its hotspot, read from NetworkManager when
//!     the tab opens. These codes are the network password, so they are
//!     never copied to the clipboard (clipboard history would keep the key)
//!     and are dropped as soon as the menu closes.
//!
//! Encoding runs `qrencode` off the GTK thread with the payload on its
//! stdin (`mshell_utils::qr`). A request from `mshellctl qr …` is queued in
//! [`super::request`] and taken when the menu is revealed.

use super::{QrRequest, clipboard_text, take_request, wifi_code};
use crate::common_widgets::qr_view::QrView;
use crate::common_widgets::segmented::{Segmented, SegmentedStyle};
use crate::menus::menu_widgets::panel_header::PanelHeader;
use mshell_config::schema::menu_widgets::MenuWidget;
use mshell_utils::qr::QrMatrix;
use relm4::gtk::prelude::{BoxExt, ButtonExt, EditableExt, EntryExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tab {
    Text,
    Wifi,
    Hotspot,
}

impl Tab {
    const ALL: [Tab; 3] = [Tab::Text, Tab::Wifi, Tab::Hotspot];

    fn index(self) -> usize {
        Self::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }
}

pub(crate) struct QrMenuWidgetModel {
    tab: Tab,
    /// Bumped on every new encode, so a slow answer for an older request
    /// (or a tab the user already left) is dropped.
    generation: u64,
    busy: bool,
    tabs: Segmented,
    entry: gtk::Entry,
    view: QrView,
    copy_btn: gtk::Button,
    status: gtk::Label,
}

impl std::fmt::Debug for QrMenuWidgetModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QrMenuWidgetModel")
            .field("tab", &self.tab)
            .finish()
    }
}

#[derive(Debug)]
pub(crate) enum QrMenuWidgetInput {
    SelectTab(Tab),
    /// Encode the entry's text.
    Generate,
    /// Encode the clipboard's text.
    FromClipboard,
    CopyImage,
    ParentRevealChanged(bool),
}

#[derive(Debug)]
pub(crate) enum QrMenuWidgetOutput {}

pub(crate) struct QrMenuWidgetInit {}

#[derive(Debug)]
pub(crate) enum QrMenuWidgetCommandOutput {
    /// `(generation, result)`: a code with its caption, or why not.
    Encoded(u64, Result<(QrMatrix, String), String>),
    /// The clipboard's text (or why it could not be read).
    Clipboard(Result<String, String>),
}

#[relm4::component(pub(crate))]
impl Component for QrMenuWidgetModel {
    type CommandOutput = QrMenuWidgetCommandOutput;
    type Input = QrMenuWidgetInput;
    type Output = QrMenuWidgetOutput;
    type Init = QrMenuWidgetInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "qr-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 12,

            #[local_ref]
            header_root -> gtk::Box {},

            #[local_ref]
            tabs_root -> gtk::Box {},

            gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 8,
                #[watch]
                set_visible: model.tab == Tab::Text,

                #[local_ref]
                entry_widget -> gtk::Entry {
                    set_hexpand: true,
                    set_placeholder_text: Some("Text or URL to share…"),
                    connect_activate => QrMenuWidgetInput::Generate,
                },
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 8,
                    set_homogeneous: true,
                    gtk::Button {
                        set_css_classes: &["ok-button-surface", "ok-button-cell"],
                        set_label: "From clipboard",
                        set_tooltip_text: Some("Encode the text on the clipboard"),
                        #[watch]
                        set_sensitive: !model.busy,
                        connect_clicked => QrMenuWidgetInput::FromClipboard,
                    },
                    gtk::Button {
                        set_css_classes: &["ok-button-primary", "ok-button-cell"],
                        set_label: "Show code",
                        #[watch]
                        set_sensitive: !model.busy,
                        connect_clicked => QrMenuWidgetInput::Generate,
                    },
                },
            },

            #[local_ref]
            view_root -> gtk::Box {},

            #[local_ref]
            status_widget -> gtk::Label {
                set_css_classes: &["label-small", "dim-label"],
                set_xalign: 0.0,
                set_wrap: true,
            },

            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_halign: gtk::Align::End,
                #[local_ref]
                copy_btn_widget -> gtk::Button {
                    set_css_classes: &["ok-button-surface", "ok-button-cell"],
                    set_label: "Copy image",
                    set_tooltip_text: Some("Copy the code to the clipboard as a PNG"),
                    connect_clicked => QrMenuWidgetInput::CopyImage,
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let s = sender.clone();
        let tabs = Segmented::new(
            &["Text", "Wi-Fi", "Hotspot"],
            0,
            SegmentedStyle::Capsule,
            move |i| s.input(QrMenuWidgetInput::SelectTab(Tab::ALL[i.min(2)])),
        );
        let tabs_root = tabs.root().clone();
        let s = sender.clone();
        let header = PanelHeader::new("screenshot-qr-symbolic", MenuWidget::Qr.display_name())
            .action(
                "edit-paste-symbolic",
                "Encode the clipboard text",
                move || {
                    s.input(QrMenuWidgetInput::FromClipboard);
                },
            );
        let header_root = header.root().clone();
        let entry_widget = gtk::Entry::new();
        let view = QrView::new();
        let view_root = view.root().clone();
        let copy_btn_widget = gtk::Button::new();
        let status_widget = gtk::Label::new(None);
        let model = QrMenuWidgetModel {
            tab: Tab::Text,
            generation: 0,
            busy: false,
            tabs,
            entry: entry_widget.clone(),
            view,
            copy_btn: copy_btn_widget.clone(),
            status: status_widget.clone(),
        };
        let widgets = view_output!();
        model
            .view
            .message("Type or paste something to share with your phone", false);
        model.sync_copy();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        self.status.set_label("");
        match message {
            QrMenuWidgetInput::SelectTab(tab) => self.select(tab, &sender),
            QrMenuWidgetInput::Generate => {
                let text = self.entry.text().to_string();
                self.encode_text(text, &sender);
            }
            QrMenuWidgetInput::FromClipboard => {
                self.select_quiet(Tab::Text);
                self.busy = true;
                sender.oneshot_command(async {
                    QrMenuWidgetCommandOutput::Clipboard(clipboard_text().await)
                });
            }
            QrMenuWidgetInput::CopyImage => {
                if self.tab == Tab::Text && self.view.copy_image() {
                    self.status.set_label("Copied the code as an image");
                }
            }
            QrMenuWidgetInput::ParentRevealChanged(visible) => {
                if visible {
                    match take_request() {
                        Some(QrRequest::Text(t)) => {
                            self.select_quiet(Tab::Text);
                            self.entry.set_text(&t);
                            self.encode_text(t, &sender);
                        }
                        Some(QrRequest::Clipboard) => {
                            sender.input(QrMenuWidgetInput::FromClipboard);
                        }
                        Some(QrRequest::Wifi) => self.select(Tab::Wifi, &sender),
                        Some(QrRequest::Hotspot) => self.select(Tab::Hotspot, &sender),
                        // Reopened on a Wi-Fi tab: read it afresh.
                        None if self.tab != Tab::Text => self.select(self.tab, &sender),
                        None => {}
                    }
                } else if self.tab != Tab::Text {
                    // Off screen: drop the password code at once.
                    self.generation += 1;
                    self.busy = false;
                    self.view.clear();
                }
            }
        }
        self.sync_copy();
    }

    fn update_cmd(
        &mut self,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            QrMenuWidgetCommandOutput::Encoded(generation, result) => {
                if generation != self.generation {
                    return;
                }
                self.busy = false;
                match result {
                    Ok((matrix, caption)) => self.view.show(matrix, &caption),
                    Err(e) => self.view.message(&e, true),
                }
            }
            QrMenuWidgetCommandOutput::Clipboard(result) => {
                self.busy = false;
                match result {
                    Ok(text) => {
                        self.entry.set_text(&text);
                        self.encode_text(text, &sender);
                    }
                    Err(e) => self.view.message(&e, true),
                }
            }
        }
        self.sync_copy();
    }
}

impl QrMenuWidgetModel {
    /// Switch the visible tab without starting any read.
    fn select_quiet(&mut self, tab: Tab) {
        self.tab = tab;
        self.tabs.set_active(tab.index());
    }

    /// Switch tab; a Wi-Fi tab reads its network and encodes it.
    fn select(&mut self, tab: Tab, sender: &ComponentSender<Self>) {
        self.select_quiet(tab);
        self.generation += 1;
        match tab {
            Tab::Text => {
                self.busy = false;
                self.view
                    .message("Type or paste something to share with your phone", false);
            }
            Tab::Wifi | Tab::Hotspot => {
                self.busy = true;
                self.view
                    .message("Reading the network from NetworkManager…", false);
                let generation = self.generation;
                let hotspot = tab == Tab::Hotspot;
                sender.oneshot_command(async move {
                    QrMenuWidgetCommandOutput::Encoded(generation, wifi_code(hotspot).await)
                });
            }
        }
    }

    fn encode_text(&mut self, text: String, sender: &ComponentSender<Self>) {
        self.select_quiet(Tab::Text);
        self.generation += 1;
        if let Err(e) = mshell_utils::qr::check_payload(&text) {
            self.view.message(&e, true);
            return;
        }
        self.busy = true;
        let generation = self.generation;
        let chars = text.chars().count();
        sender.oneshot_command(async move {
            let result = mshell_utils::qr::render(&text).await.map(|m| {
                let caption = if chars == 1 {
                    "1 character".to_string()
                } else {
                    format!("{chars} characters")
                };
                (m, caption)
            });
            QrMenuWidgetCommandOutput::Encoded(generation, result)
        });
    }

    /// Copy is offered for text codes only (a Wi-Fi code is the password).
    fn sync_copy(&self) {
        self.copy_btn.set_visible(self.tab == Tab::Text);
        self.copy_btn
            .set_sensitive(self.tab == Tab::Text && self.view.has_code() && !self.busy);
    }
}
