//! NetworkSpeed module — bar'da indirme/yükleme hızı + IP + VPN durumu.
//!
//! `[system_info]` modülünden ayırdık; ağ tipik olarak daha kısa
//! interval ister (1-2 sn) ve eşikleri farklı birim alanda (KB/s).
//! VPN bağlıysa IP, VPN arayüzünden alınır; menüde VPN durumu ayrı
//! satırda gösterilir.
//!
//! Kullanım (mshell.toml):
//!
//!     [modules]
//!     right = [["NetworkSpeed", "SystemInfo"], "Clock"]
//!
//!     [network_speed]
//!     indicators = ["IpAddress", "Download", "Upload"]
//!     interval   = 2
//!     unit       = "Auto"                # "Auto" | "Kbps" | "Mbps"
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
    Alignment, Element, Font, Length, Subscription, Theme,
    alignment::Horizontal,
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

/// VPN arayüz adlarını tanı. `wg0-mullvad`, `tun0`, `nordlynx`, `proton0`,
/// `mullvad-*`, `tap0` gibi yaygın isimleri kapsar. Fiziksel arayüzleri
/// (en/eth/wl/wlan/br) ELEM EZ — onlar başka filtrede.
fn is_vpn_iface(name: &str) -> bool {
    name.starts_with("wg")
        || name.starts_with("tun")
        || name.starts_with("tap")
        || name.starts_with("nordlynx")
        || name.starts_with("proton")
        || name.starts_with("mullvad")
        || name.starts_with("ipsec")
}

/// Fiziksel (Ethernet / Wi-Fi / bridge) arayüz mü?
fn is_physical_iface(name: &str) -> bool {
    name.starts_with("en")
        || name.starts_with("eth")
        || name.starts_with("wl")
        || name.starts_with("wlan")
        || name.starts_with("br")
}

struct Snapshot {
    /// Fiziksel arayüzün (en/eth/wl…) IPv4'ü — "LAN IP".
    lan_ip: Option<String>,
    /// VPN tüneline atanan IPv4 — yalnızca VPN bağlıyken.
    vpn_ip: Option<String>,
    /// VPN bağlıysa arayüz adı (`wg0-mullvad` vb.). None ise VPN yok.
    vpn_iface: Option<String>,
    /// KB/s — fiziksel arayüz toplamı.
    download_kbps: u32,
    upload_kbps: u32,
    last_check: Instant,
}

pub struct NetworkSpeed {
    config: NetworkSpeedModuleConfig,
    networks: Networks,
    snapshot: Snapshot,
}

impl NetworkSpeed {
    pub fn new(config: NetworkSpeedModuleConfig) -> Self {
        let mut networks = Networks::new_with_refreshed_list();
        networks.refresh(true);
        let mut me = Self {
            config,
            networks,
            snapshot: Snapshot {
                lan_ip: None,
                vpn_ip: None,
                vpn_iface: None,
                download_kbps: 0,
                upload_kbps: 0,
                last_check: Instant::now(),
            },
        };
        // İlk tick'i hemen yakala; aksi takdirde menü açılışında "Yok"
        // gözükür ve subscription'ın gelmesini bekleriz.
        me.refresh();
        me
    }

    fn refresh(&mut self) {
        let elapsed_secs = self.snapshot.last_check.elapsed().as_secs().max(1);
        self.networks.refresh(true);

        // Fiziksel ↔ aktif VPN ayrımı.
        let mut received_phys = 0u64;
        let mut transmitted_phys = 0u64;
        let mut vpn_iface: Option<String> = None;
        let mut vpn_ip: Option<String> = None;
        let mut phys_ip: Option<String> = None;

        // İlk fiziksel IP'yi tutarlı sıralamayla seç: en* > eth* > wl* > br*
        let phys_rank = |n: &str| -> u8 {
            if n.starts_with("en") {
                0
            } else if n.starts_with("eth") {
                1
            } else if n.starts_with("wl") {
                2
            } else if n.starts_with("wlan") {
                3
            } else {
                9
            }
        };

        let mut phys_candidates: Vec<(u8, String, Option<String>, u64, u64)> = Vec::new();

        for (name, data) in self.networks.iter() {
            if is_vpn_iface(name) {
                if vpn_iface.is_none() {
                    vpn_iface = Some(name.clone());
                    vpn_ip = data
                        .ip_networks()
                        .iter()
                        .find(|n| n.addr.is_ipv4())
                        .map(|n| n.addr.to_string());
                }
            } else if is_physical_iface(name) {
                let ip = data
                    .ip_networks()
                    .iter()
                    .find(|n| n.addr.is_ipv4())
                    .map(|n| n.addr.to_string());
                phys_candidates.push((
                    phys_rank(name),
                    name.clone(),
                    ip,
                    data.received(),
                    data.transmitted(),
                ));
            }
        }

        phys_candidates.sort_by_key(|c| c.0);
        for (_, _, ip, rx, tx) in &phys_candidates {
            if phys_ip.is_none() {
                phys_ip = ip.clone();
            }
            received_phys += rx;
            transmitted_phys += tx;
        }

        let to_kbps = |bytes: u64| (bytes / 1000 / elapsed_secs) as u32;
        self.snapshot = Snapshot {
            lan_ip: phys_ip,
            vpn_ip,
            vpn_iface,
            download_kbps: to_kbps(received_phys),
            upload_kbps: to_kbps(transmitted_phys),
            last_check: Instant::now(),
        };
    }

    pub fn update(&mut self, _msg: Message) {
        self.refresh();
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

    fn speed_indicator<'a>(
        ico: StaticIcon,
        display: u32,
        unit: &str,
        threshold: Option<(u32, u32, u32)>,
    ) -> Element<'a, Message> {
        let (space, bar_font) = use_theme(|t| (t.space, t.bar_font_size));
        // Fixed-width + monospace so a 7 KB/s → 999 KB/s digit jump
        // doesn't grow/shrink the text widget and chain into the
        // bar's animated_size. Width covers 8 chars ("9999KB/s",
        // "9999MB/s"); align Left so short values stay glued to
        // the icon and the slack pads on the right (reads as
        // natural inter-indicator spacing).
        let value_width = Length::Fixed(bar_font * 0.62 * 8.0 + 3.0);
        let body = container(
            row!(
                icon(ico).size(bar_font),
                text(format!("{display}{unit}"))
                    .size(bar_font)
                    .font(Font::MONOSPACE)
                    .width(value_width)
                    .align_x(Horizontal::Left)
            )
            .spacing(space.xxs),
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

    /// IP göstergesi — VPN bağlıysa shield ikonuyla ve success rengiyle
    /// vurgulanır, değilse normal IP ikonu.
    fn ip_indicator<'a>(
        ip: Option<&str>,
        vpn: bool,
    ) -> Element<'a, Message> {
        let (space, bar_font) = use_theme(|t| (t.space, t.bar_font_size));
        let text_value = ip.unwrap_or("—").to_string();
        let body = container(
            row!(
                icon(if vpn {
                    StaticIcon::Vpn
                } else {
                    StaticIcon::IpAddress
                })
                .size(bar_font),
                text(text_value).size(bar_font)
            )
            .spacing(space.xxs),
        );
        if vpn {
            body.style(|theme: &Theme| container::Style {
                text_color: Some(theme.palette().success),
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
                let (display, unit) = self.format(self.snapshot.download_kbps);
                Self::speed_indicator(
                    StaticIcon::DownloadSpeed,
                    display,
                    unit,
                    Some((
                        self.snapshot.download_kbps,
                        self.config.download_warn_kbps,
                        self.config.download_alert_kbps,
                    )),
                )
            }
            NetworkSpeedIndicator::Upload => {
                let (display, unit) = self.format(self.snapshot.upload_kbps);
                Self::speed_indicator(
                    StaticIcon::UploadSpeed,
                    display,
                    unit,
                    Some((
                        self.snapshot.upload_kbps,
                        self.config.upload_warn_kbps,
                        self.config.upload_alert_kbps,
                    )),
                )
            }
            NetworkSpeedIndicator::IpAddress => {
                // Bar'da IP isteyenler için: VPN aktifse VPN IP'si,
                // yoksa LAN IP. Varsayılan config'de bu indicator yok —
                // IP normalde menüde gösteriliyor.
                let displayed = self
                    .snapshot
                    .vpn_ip
                    .as_deref()
                    .or(self.snapshot.lan_ip.as_deref());
                Self::ip_indicator(displayed, self.snapshot.vpn_iface.is_some())
            }
        });
        Row::with_children(elements)
            .align_y(Alignment::Center)
            .spacing(space.xxs)
            .into()
    }

    /// Açılır menü — VPN durumu, IP, anlık ↓/↑, arayüz listesi.
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

        let (dl_v, dl_u) = self.format(self.snapshot.download_kbps);
        let (ul_v, ul_u) = self.format(self.snapshot.upload_kbps);

        // VPN durumu satırı — bağlıysa Vpn ikonu + arayüz adı; değilse "Yok".
        let vpn_row = if let Some(iface) = &self.snapshot.vpn_iface {
            row_element(
                StaticIcon::Vpn,
                t!("network-speed-vpn"),
                iface.clone(),
            )
        } else {
            row_element(
                StaticIcon::Vpn,
                t!("network-speed-vpn"),
                t!("network-speed-vpn-off"),
            )
        };

        // LAN IP — fiziksel arayüz IPv4'ü (her durumda göster).
        let lan_ip_row = row_element(
            StaticIcon::Ethernet,
            t!("network-speed-lan-ip"),
            self.snapshot
                .lan_ip
                .clone()
                .unwrap_or_else(|| "—".to_string()),
        );

        // VPN IP — VPN bağlıyken IP, değilse tire.
        let vpn_ip_row = row_element(
            StaticIcon::Vpn,
            t!("network-speed-vpn-ip"),
            self.snapshot
                .vpn_ip
                .clone()
                .unwrap_or_else(|| "—".to_string()),
        );

        // Arayüz başına kümülatif RX/TX (boot'tan beri, MiB). VPN'i
        // physical-only filtrenin DIŞINDA tutmadığımızdan ayrı listede
        // göstermek için filtre düzeltilir.
        let iface_rows = self
            .networks
            .iter()
            .filter(|(name, _)| is_physical_iface(name) || is_vpn_iface(name))
            .sorted_by_key(|(name, _)| {
                // VPN'leri sonda topla → görsel hiyerarşi
                if is_vpn_iface(name) {
                    (1, (*name).clone())
                } else {
                    (0, (*name).clone())
                }
            })
            .map(|(name, data)| {
                let rx = data.total_received() / 1_048_576;
                let tx = data.total_transmitted() / 1_048_576;
                let prefix = if is_vpn_iface(name) { "🔒 " } else { "" };
                row_element(
                    if is_vpn_iface(name) {
                        StaticIcon::Vpn
                    } else {
                        StaticIcon::IpAddress
                    },
                    format!("{prefix}{name}"),
                    format!("↓ {rx} MiB · ↑ {tx} MiB"),
                )
                .into()
            })
            .collect::<Vec<Element<_>>>();

        container(
            column!(
                text(t!("network-speed-heading")).size(font_size.lg),
                divider(),
                Column::with_capacity(6 + iface_rows.len())
                    .push(vpn_row)
                    .push(lan_ip_row)
                    .push(vpn_ip_row)
                    .push(divider())
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
