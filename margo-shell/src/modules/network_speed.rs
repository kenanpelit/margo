//! NetworkSpeed module — bar'da indirme/yükleme hızı göstergesi.
//!
//! `[system_info]` modülünden ayırdık; çünkü ağ tipik olarak çok daha
//! kısa interval (her saniye) güncelleme isterken cpu/ram/temp için bu
//! aşırıya kaçar. Ayrıca eşikler tamamen farklı bir birim alanda
//! (KB/s vs %).
//!
//! Kullanım (mshell.toml):
//!
//!     [modules]
//!     right = [["Cpu", "Memory"], "NetworkSpeed", "Clock"]
//!
//!     [network_speed]
//!     indicators = ["Download", "Upload"]
//!     interval   = 2
//!     unit       = "Auto"                 # "Auto" | "Kbps" | "Mbps"
//!     download_warn_kbps  = 5000
//!     download_alert_kbps = 20000
//!     upload_warn_kbps    = 2000
//!     upload_alert_kbps   = 10000

use crate::{
    components::MenuSize,
    components::divider,
    components::icons::{StaticIcon, icon},
    config::{NetworkSpeedIndicator, NetworkSpeedModuleConfig, NetworkSpeedUnit},
    t,
    theme::use_theme,
};
use iced::{
    Alignment, Element, Length, Subscription, Theme,
    time::every,
    widget::{Column, Row, column, container, row, text},
};
use itertools::Itertools;
use std::time::{Duration, Instant};
use sysinfo::Networks;

#[derive(Debug, Clone)]
pub enum Message {
    Update,
}

struct Speeds {
    /// KB/s
    download_kbps: u32,
    /// KB/s
    upload_kbps: u32,
    last_check: Instant,
}

pub struct NetworkSpeed {
    config: NetworkSpeedModuleConfig,
    networks: Networks,
    speeds: Speeds,
}

impl NetworkSpeed {
    pub fn new(config: NetworkSpeedModuleConfig) -> Self {
        let mut networks = Networks::new_with_refreshed_list();
        networks.refresh(true);
        Self {
            config,
            networks,
            speeds: Speeds {
                download_kbps: 0,
                upload_kbps: 0,
                last_check: Instant::now(),
            },
        }
    }

    pub fn update(&mut self, _msg: Message) {
        let elapsed_secs = self.speeds.last_check.elapsed().as_secs().max(1);
        self.networks.refresh(true);

        let (received, transmitted) = self
            .networks
            .iter()
            .filter(|(name, _)| {
                // Yalnızca gerçek arayüzler — lo, docker, vboxnet, vs. eleniyor.
                name.starts_with("en")
                    || name.starts_with("eth")
                    || name.starts_with("wl")
                    || name.starts_with("wlan")
                    || name.starts_with("br")
            })
            .fold((0u64, 0u64), |(r, t), (_, data)| {
                (r + data.received(), t + data.transmitted())
            });

        // bytes → KB/s
        let to_kbps = |bytes: u64| (bytes / 1000 / elapsed_secs) as u32;
        self.speeds = Speeds {
            download_kbps: to_kbps(received),
            upload_kbps: to_kbps(transmitted),
            last_check: Instant::now(),
        };
    }

    /// (display_value, unit_str)  — config.unit ve eşiklere göre formatla.
    fn format(&self, kbps: u32) -> (u32, &'static str) {
        match self.config.unit {
            NetworkSpeedUnit::Kbps => (kbps, "KB/s"),
            NetworkSpeedUnit::Mbps => (kbps / 1000, "MB/s"),
            NetworkSpeedUnit::Auto => {
                if kbps >= 1000 {
                    (kbps / 1000, "MB/s")
                } else {
                    (kbps, "KB/s")
                }
            }
        }
    }

    fn indicator<'a>(
        ico: StaticIcon,
        display: u32,
        unit: &str,
        threshold: Option<(u32, u32, u32)>,
    ) -> Element<'a, Message> {
        let space = use_theme(|t| t.space);
        let body = container(
            row!(icon(ico), text(format!("{display}{unit}"))).spacing(space.xxs),
        );
        if let Some((value, warn, alert)) = threshold {
            body.style(move |theme: &Theme| container::Style {
                text_color: if value >= alert {
                    Some(theme.palette().danger)
                } else if value > warn {
                    Some(theme.palette().warning)
                } else {
                    None
                },
                ..Default::default()
            })
            .into()
        } else {
            body.into()
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        let space = use_theme(|t| t.space);
        let elements = self.config.indicators.iter().map(|i| match i {
            NetworkSpeedIndicator::Download => {
                let (display, unit) = self.format(self.speeds.download_kbps);
                Self::indicator(
                    StaticIcon::DownloadSpeed,
                    display,
                    unit,
                    Some((
                        self.speeds.download_kbps,
                        self.config.download_warn_kbps,
                        self.config.download_alert_kbps,
                    )),
                )
            }
            NetworkSpeedIndicator::Upload => {
                let (display, unit) = self.format(self.speeds.upload_kbps);
                Self::indicator(
                    StaticIcon::UploadSpeed,
                    display,
                    unit,
                    Some((
                        self.speeds.upload_kbps,
                        self.config.upload_warn_kbps,
                        self.config.upload_alert_kbps,
                    )),
                )
            }
        });
        Row::with_children(elements)
            .align_y(Alignment::Center)
            .spacing(space.xxs)
            .into()
    }

    /// Açılır menü — bar göstergesinin tıklanmasıyla açılır.
    /// Anlık indirme/yükleme + arayüz başına toplam (boot'tan bu yana).
    pub fn menu_view(&'_ self) -> Element<'_, Message> {
        let (font_size, space) = use_theme(|t| (t.font_size, t.space));

        let row_element = |ico: StaticIcon, label: String, value: String| {
            row!(
                container(icon(ico).size(font_size.xl)).center_x(Length::Fixed(space.xl)),
                text(label).width(Length::Fill),
                text(value)
            )
            .align_y(Alignment::Center)
            .spacing(space.xs)
        };

        let (dl_v, dl_u) = self.format(self.speeds.download_kbps);
        let (ul_v, ul_u) = self.format(self.speeds.upload_kbps);

        // Arayüz başına kümülatif (boot'tan beri) RX/TX — quick glance.
        let iface_rows = self
            .networks
            .iter()
            .filter(|(name, _)| {
                name.starts_with("en")
                    || name.starts_with("eth")
                    || name.starts_with("wl")
                    || name.starts_with("wlan")
                    || name.starts_with("br")
            })
            .sorted_by_key(|(name, _)| (*name).clone())
            .map(|(name, data)| {
                let rx = data.total_received() / 1_048_576; // MiB
                let tx = data.total_transmitted() / 1_048_576;
                row_element(
                    StaticIcon::IpAddress,
                    name.clone(),
                    format!("↓ {rx} MiB · ↑ {tx} MiB"),
                )
                .into()
            })
            .collect::<Vec<Element<_>>>();

        container(
            column!(
                text(t!("network-speed-heading")).size(font_size.lg),
                divider(),
                Column::with_capacity(2 + iface_rows.len())
                    .push(row_element(
                        StaticIcon::DownloadSpeed,
                        t!("system-info-download-speed"),
                        format!("{dl_v} {dl_u}"),
                    ))
                    .push(row_element(
                        StaticIcon::UploadSpeed,
                        t!("system-info-upload-speed"),
                        format!("{ul_v} {ul_u}"),
                    ))
                    .push(divider())
                    .push(Column::with_children(iface_rows).spacing(space.xxs))
                    .spacing(space.xxs)
                    .padding([0.0, space.xs])
            )
            .spacing(space.xs),
        )
        .width(MenuSize::Medium)
        .into()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        every(Duration::from_secs(self.config.interval)).map(|_| Message::Update)
    }
}
