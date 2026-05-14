//! Public IP menu widget — content surface for `MenuType::Nip`.
//!
//! Mirrors the noctalia `nip/Panel.qml` layout: a hero card with
//! the IP address front-and-centre + a status pill, then a list
//! of detail rows (City / Region / Country / Coordinates /
//! Timezone / Organisation). Footer has Copy-IP, Refresh, and
//! "Open in browser" buttons.
//!
//! Polls ipinfo.io every 300 s on its own loop (so the panel
//! stays fresh while open even if the bar pill's poll cycle is
//! elsewhere), plus a manual Refresh button.

use crate::bars::bar_widgets::nip::{FetchState, IpInfo, NipSnapshot, fetch_snapshot};
use relm4::gtk::prelude::{BoxExt, ButtonExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::Duration;
use tracing::warn;

const REFRESH_INTERVAL: Duration = Duration::from_secs(300);
const STARTUP_DELAY: Duration = Duration::from_millis(250);

pub(crate) struct NipMenuWidgetModel {
    snapshot: NipSnapshot,
    ip_label: gtk::Label,
    status_badge: gtk::Label,
    /// Detail rows keyed by field name — the value Label is what
    /// `sync_view` updates. Order matters (it's the display
    /// order), so a Vec rather than a map.
    detail_values: Vec<(&'static str, gtk::Label)>,
    copy_button: gtk::Button,
    open_button: gtk::Button,
}

impl std::fmt::Debug for NipMenuWidgetModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NipMenuWidgetModel")
            .field("snapshot", &self.snapshot)
            .finish()
    }
}

#[derive(Debug)]
pub(crate) enum NipMenuWidgetInput {
    RefreshNow,
    CopyIp,
    OpenInBrowser,
}

#[derive(Debug)]
pub(crate) enum NipMenuWidgetOutput {}

pub(crate) struct NipMenuWidgetInit {}

#[derive(Debug)]
pub(crate) enum NipMenuWidgetCommandOutput {
    Refreshed(NipSnapshot),
}

#[relm4::component(pub(crate))]
impl Component for NipMenuWidgetModel {
    type CommandOutput = NipMenuWidgetCommandOutput;
    type Input = NipMenuWidgetInput;
    type Output = NipMenuWidgetOutput;
    type Init = NipMenuWidgetInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "nip-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 10,

            // ── Hero card ───────────────────────────────────────
            gtk::Box {
                add_css_class: "nip-hero",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 10,

                gtk::Image {
                    set_icon_name: Some("globe-symbolic"),
                    set_pixel_size: 32,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_hexpand: true,

                    gtk::Label {
                        add_css_class: "label-small",
                        set_label: "Public IP",
                        set_xalign: 0.0,
                    },
                    #[local_ref]
                    ip_label_widget -> gtk::Label {
                        add_css_class: "nip-ip-address",
                        set_xalign: 0.0,
                        set_selectable: true,
                    },
                },

                #[local_ref]
                status_badge_widget -> gtk::Label {
                    add_css_class: "nip-badge",
                    set_valign: gtk::Align::Center,
                },
            },

            gtk::Separator { set_orientation: gtk::Orientation::Horizontal },

            // ── Detail rows ─────────────────────────────────────
            #[local_ref]
            details_box -> gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 4,
            },

            // ── Footer actions ──────────────────────────────────
            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 6,
                set_margin_top: 4,

                #[local_ref]
                copy_button_widget -> gtk::Button {
                    set_css_classes: &["ok-button-surface"],
                    set_label: "Copy IP",
                    connect_clicked[sender] => move |_| {
                        sender.input(NipMenuWidgetInput::CopyIp);
                    },
                },
                gtk::Button {
                    set_css_classes: &["ok-button-surface"],
                    set_label: "Refresh",
                    connect_clicked[sender] => move |_| {
                        sender.input(NipMenuWidgetInput::RefreshNow);
                    },
                },
                #[local_ref]
                open_button_widget -> gtk::Button {
                    set_css_classes: &["ok-button-surface"],
                    set_label: "Open in browser",
                    set_hexpand: true,
                    set_halign: gtk::Align::End,
                    connect_clicked[sender] => move |_| {
                        sender.input(NipMenuWidgetInput::OpenInBrowser);
                    },
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let ip_label_widget = gtk::Label::new(Some("…"));
        let status_badge_widget = gtk::Label::new(Some("Loading"));
        let copy_button_widget = gtk::Button::new();
        let open_button_widget = gtk::Button::new();
        let details_box = gtk::Box::new(gtk::Orientation::Vertical, 4);

        // Build the fixed set of detail rows once. Each row is a
        // `Label(caption) — Label(value)` pair; we keep the value
        // Labels on the model so `sync_view` only touches text.
        let mut detail_values: Vec<(&'static str, gtk::Label)> = Vec::new();
        for caption in ["City", "Region", "Country", "Coordinates", "Timezone", "Organisation"] {
            let (row, value) = make_detail_row(caption);
            details_box.append(&row);
            detail_values.push((caption, value));
        }

        sender.command(|out, shutdown| {
            async move {
                let shutdown_fut = shutdown.wait();
                tokio::pin!(shutdown_fut);
                let mut first = true;
                loop {
                    let delay = if first { STARTUP_DELAY } else { REFRESH_INTERVAL };
                    first = false;
                    tokio::select! {
                        () = &mut shutdown_fut => break,
                        _ = tokio::time::sleep(delay) => {}
                    }
                    let snap = fetch_snapshot().await;
                    let _ = out.send(NipMenuWidgetCommandOutput::Refreshed(snap));
                }
            }
        });

        let model = NipMenuWidgetModel {
            snapshot: NipSnapshot::default(),
            ip_label: ip_label_widget.clone(),
            status_badge: status_badge_widget.clone(),
            detail_values,
            copy_button: copy_button_widget.clone(),
            open_button: open_button_widget.clone(),
        };

        let widgets = view_output!();
        sync_view(&model);

        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            NipMenuWidgetInput::RefreshNow => {
                sender.command(|out, _shutdown| async move {
                    let snap = fetch_snapshot().await;
                    let _ = out.send(NipMenuWidgetCommandOutput::Refreshed(snap));
                });
            }
            NipMenuWidgetInput::CopyIp => {
                if let Some(ip) = self.snapshot.info.as_ref().map(|i| i.ip.clone()) {
                    if !ip.is_empty() {
                        // `wl-copy` is in the package depends; pipe
                        // the IP to it. Detached — we don't care
                        // about the exit status, the clipboard set
                        // is fire-and-forget.
                        tokio::spawn(async move {
                            use tokio::io::AsyncWriteExt;
                            match tokio::process::Command::new("wl-copy")
                                .stdin(std::process::Stdio::piped())
                                .spawn()
                            {
                                Ok(mut child) => {
                                    if let Some(mut stdin) = child.stdin.take() {
                                        let _ = stdin.write_all(ip.as_bytes()).await;
                                    }
                                    let _ = child.wait().await;
                                }
                                Err(e) => warn!(error = %e, "wl-copy spawn failed"),
                            }
                        });
                    }
                }
            }
            NipMenuWidgetInput::OpenInBrowser => {
                let url = match self.snapshot.info.as_ref() {
                    Some(i) if !i.ip.is_empty() => format!("https://ipinfo.io/{}", i.ip),
                    _ => "https://ipinfo.io".to_string(),
                };
                tokio::spawn(async move {
                    let _ = tokio::process::Command::new("xdg-open")
                        .arg(&url)
                        .status()
                        .await;
                });
            }
        }
    }

    fn update_cmd_with_view(
        &mut self,
        _widgets: &mut Self::Widgets,
        message: Self::CommandOutput,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            NipMenuWidgetCommandOutput::Refreshed(snap) => {
                if self.snapshot != snap {
                    self.snapshot = snap;
                    sync_view(self);
                }
            }
        }
    }
}

fn make_detail_row(caption: &str) -> (gtk::Box, gtk::Label) {
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .css_classes(vec!["nip-detail-row"])
        .build();
    let cap = gtk::Label::new(Some(caption));
    cap.add_css_class("label-small");
    cap.set_xalign(0.0);
    cap.set_width_request(110);
    row.append(&cap);
    let value = gtk::Label::new(Some("—"));
    value.add_css_class("label-small-bold");
    value.set_xalign(0.0);
    value.set_hexpand(true);
    value.set_selectable(true);
    value.set_wrap(true);
    value.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    row.append(&value);
    (row, value)
}

fn sync_view(model: &NipMenuWidgetModel) {
    let snap = &model.snapshot;

    // Hero — IP + status badge.
    let (ip_text, badge_text, badge_class) = match (snap.state, &snap.info) {
        (FetchState::Loading, _) => ("…".to_string(), "Loading", "nip-badge-loading"),
        (FetchState::Ok, Some(i)) if !i.ip.is_empty() => {
            (i.ip.clone(), "Online", "nip-badge-ok")
        }
        (FetchState::Ok, _) => ("unavailable".to_string(), "No data", "nip-badge-err"),
        (FetchState::Err, _) => ("unavailable".to_string(), "Error", "nip-badge-err"),
    };
    model.ip_label.set_label(&ip_text);
    model.status_badge.set_label(badge_text);
    model.status_badge.set_css_classes(&["nip-badge", badge_class]);

    // Detail rows.
    let info = snap.info.clone().unwrap_or_default();
    for (caption, label) in &model.detail_values {
        let value = detail_value(*caption, &info);
        label.set_label(if value.is_empty() { "—" } else { &value });
    }

    // Footer buttons enabled only when there's an IP to act on.
    let has_ip = snap
        .info
        .as_ref()
        .map(|i| !i.ip.is_empty())
        .unwrap_or(false);
    model.copy_button.set_sensitive(has_ip);
    model.open_button.set_sensitive(true); // open always works (falls back to ipinfo.io)
}

fn detail_value(caption: &str, info: &IpInfo) -> String {
    match caption {
        "City" => info.city.clone(),
        "Region" => info.region.clone(),
        "Country" => info.country.clone(),
        "Coordinates" => info.loc.clone(),
        "Timezone" => info.timezone.clone(),
        "Organisation" => info.org.clone(),
        _ => String::new(),
    }
}
