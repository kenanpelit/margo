use crate::{
    components::MenuSize,
    components::divider,
    components::icons::{StaticIcon, icon},
    config::{CpuFormat, DiskFormat, MemoryFormat, SystemInfoIndicator, SystemInfoModuleConfig},
    i18n::{UnitSystem, unit_system},
    t,
    theme::use_theme,
    utils,
};
use iced::{
    Alignment, Element, Font, Length, Subscription, Theme,
    alignment::Horizontal,
    time::every,
    widget::{Column, Row, column, container, row, text},
};
use itertools::Itertools;
use std::time::Duration;
use sysinfo::{Components, Disks, System};

struct MemoryUsage {
    percentage: u32,
    fraction: String,
}

struct CpuUsage {
    percentage: u32,
    frequency: f32,
}

struct Temperature {
    celsius: Option<i32>,
}

struct DiskView {
    percentage: u32,
    fraction: String,
}

struct SystemInfoData {
    cpu_usage: CpuUsage,
    memory_usage: MemoryUsage,
    memory_swap_usage: MemoryUsage,
    temperature: Temperature,
    disks: Vec<(String, DiskView)>,
}

fn get_system_info(
    system: &mut System,
    components: &mut Components,
    disks: &mut Disks,
    temperature_sensor: &str,
    sensor_index: Option<usize>,
    mounts: Option<&[String]>,
) -> SystemInfoData {
    system.refresh_memory();
    system.refresh_cpu_all();

    components.refresh(true);
    disks.refresh(true);

    let cpus = system.cpus();
    let avg_freq = cpus.iter().map(|cpu| cpu.frequency() as f32).sum::<f32>() / cpus.len() as f32;

    let cpu_usage = CpuUsage {
        percentage: system.global_cpu_usage() as u32,
        frequency: utils::floor_dp(avg_freq / 1000.0, 2),
    };

    let total_mem = system.total_memory();
    let used_mem = total_mem - system.available_memory();

    let memory_usage = MemoryUsage {
        percentage: if total_mem > 0 {
            (used_mem as f32 / total_mem as f32 * 100.) as u32
        } else {
            0
        },
        fraction: format!(
            "{:.2}/{:.2}",
            utils::bytes_to_gib(used_mem),
            utils::bytes_to_gib(total_mem)
        ),
    };

    let total_swap = system.total_swap();
    let used_swap = total_swap - system.free_swap();

    let memory_swap_usage = MemoryUsage {
        percentage: if total_swap > 0 {
            (used_swap as f32 / total_swap as f32 * 100.) as u32
        } else {
            0
        },
        fraction: format!(
            "{:.2}/{:.2}",
            utils::bytes_to_gib(used_swap),
            utils::bytes_to_gib(total_swap)
        ),
    };

    let temperature_cel = sensor_index
        .and_then(|i| components.get(i))
        .and_then(|c| c.temperature().map(|t| t as i32))
        .or_else(|| {
            components
                .iter()
                .find(|c| c.label() == temperature_sensor)
                .and_then(|c| c.temperature().map(|t| t as i32))
        });

    let temperature = Temperature {
        celsius: temperature_cel,
    };

    let disks: Vec<(String, DiskView)> = disks
        .iter()
        .filter(|d| !d.is_removable() && d.total_space() != 0)
        .filter(|d| {
            if let Some(mounts) = mounts {
                let mount_str = d.mount_point().display().to_string();
                mounts.contains(&mount_str)
            } else {
                true
            }
        })
        .map(|d| {
            let total_space = d.total_space();
            let avail_space = d.available_space();

            let space_per = (total_space - avail_space) as f32 / total_space as f32 * 100.;

            (
                d.mount_point().display().to_string(),
                DiskView {
                    percentage: space_per as u32,
                    fraction: format!(
                        "{:.2}/{:.2}",
                        utils::bytes_to_gb(total_space - avail_space),
                        utils::bytes_to_gb(total_space)
                    ),
                },
            )
        })
        .sorted_by(|a, b| {
            if let Some(mounts_list) = mounts {
                let pos_a = mounts_list
                    .iter()
                    .position(|m| m == &a.0)
                    .unwrap_or(usize::MAX);
                let pos_b = mounts_list
                    .iter()
                    .position(|m| m == &b.0)
                    .unwrap_or(usize::MAX);
                pos_a.cmp(&pos_b)
            } else {
                a.0.cmp(&b.0)
            }
        })
        .collect();

    // Hız + IP NetworkSpeed modülüne taşındı.
    SystemInfoData {
        cpu_usage,
        memory_usage,
        memory_swap_usage,
        temperature,
        disks,
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    Update,
}

pub struct SystemInfo {
    config: SystemInfoModuleConfig,
    system: System,
    components: Components,
    disks: Disks,
    data: SystemInfoData,
    cached_sensor_index: Option<usize>,
}

impl SystemInfo {
    pub fn new(config: SystemInfoModuleConfig) -> Self {
        let mut system = System::new();
        let mut components = Components::new_with_refreshed_list();
        let mut disks = Disks::new_with_refreshed_list();

        let cached_sensor_index = components
            .iter()
            .position(|c| c.label() == config.temperature.sensor);

        let data = get_system_info(
            &mut system,
            &mut components,
            &mut disks,
            config.temperature.sensor.as_str(),
            cached_sensor_index,
            config.disk.mounts.as_deref(),
        );

        Self {
            config,
            system,
            components,
            disks,
            data,
            cached_sensor_index,
        }
    }

    pub fn update(&mut self, message: Message) {
        match message {
            Message::Update => {
                self.data = get_system_info(
                    &mut self.system,
                    &mut self.components,
                    &mut self.disks,
                    &self.config.temperature.sensor,
                    self.cached_sensor_index,
                    self.config.disk.mounts.as_deref(),
                );
            }
        }
    }

    fn info_element<'a>(
        info_icon: StaticIcon,
        label: String,
        value: String,
    ) -> Element<'a, Message> {
        let (font_size, space) = use_theme(|t| (t.font_size, t.space));
        row!(
            container(icon(info_icon).size(font_size.xl)).center_x(Length::Fixed(space.xl)),
            text(label).width(Length::Fill),
            text(value)
        )
        .align_y(Alignment::Center)
        .spacing(space.xs)
        .into()
    }

    fn indicator_info_element<'a, V: PartialOrd + 'a>(
        info_icon: StaticIcon,
        (display, unit): (impl std::fmt::Display + 'a, &str),
        threshold: Option<(V, V, V)>,
        prefix: Option<String>,
        value_chars: f32,
    ) -> Element<'a, Message> {
        let (space, bar_font) = use_theme(|t| (t.space, t.bar_font_size));
        // Fixed-width + monospace so digit-count churn (5% → 23% →
        // 100%) doesn't grow/shrink the text widget and chain into
        // the bar's animated_size. `value_chars` is the caller's
        // worst-case character count *including* prefix, unit and
        // any embedded spaces. Char width ≈ bar_font * 0.62 + 3px
        // buffer to absorb glyph-anti-alias rounding.
        let value_width = Length::Fixed(bar_font * 0.62 * value_chars + 3.0);
        let value_text = match &prefix {
            Some(p) => format!("{p} {display}{unit}"),
            None => format!("{display}{unit}"),
        };
        let element = container(
            row!(
                icon(info_icon).size(bar_font),
                text(value_text)
                    .size(bar_font)
                    .font(Font::MONOSPACE)
                    .width(value_width)
                    .align_x(Horizontal::Left)
            )
            .spacing(space.xxs),
        );

        if let Some((value, warn_threshold, alert_threshold)) = threshold {
            element
                .style(move |theme: &Theme| container::Style {
                    text_color: if value > warn_threshold && value < alert_threshold {
                        Some(theme.palette().warning)
                    } else if value >= alert_threshold {
                        Some(theme.palette().danger)
                    } else {
                        None
                    },
                    ..Default::default()
                })
                .into()
        } else {
            element.into()
        }
    }

    pub fn menu_view(&'_ self) -> Element<'_, Message> {
        let (font_size, space) = use_theme(|t| (t.font_size, t.space));
        container(
            column!(
                text(t!("system-info-heading")).size(font_size.lg),
                divider(),
                Column::with_capacity(6)
                    .push(Self::info_element(
                        StaticIcon::Cpu,
                        t!("system-info-cpu-usage"),
                        match self.config.cpu.format {
                            CpuFormat::Percentage => format!("{}%", self.data.cpu_usage.percentage),
                            CpuFormat::Frequency =>
                                format!("{} GHz", self.data.cpu_usage.frequency),
                        }
                    ))
                    .push(Self::info_element(
                        StaticIcon::Mem,
                        t!("system-info-memory-usage"),
                        match self.config.memory.format {
                            MemoryFormat::Percentage =>
                                format!("{}%", self.data.memory_usage.percentage),
                            MemoryFormat::Fraction =>
                                format!("{} GiB", self.data.memory_usage.fraction),
                        }
                    ))
                    .push(Self::info_element(
                        StaticIcon::Mem,
                        t!("system-info-swap-memory-usage"),
                        match self.config.memory.format {
                            MemoryFormat::Percentage =>
                                format!("{}%", self.data.memory_swap_usage.percentage),
                            MemoryFormat::Fraction =>
                                format!("{} GiB", self.data.memory_swap_usage.fraction),
                        }
                    ))
                    .push(self.data.temperature.celsius.map(|cel| {
                        Self::info_element(StaticIcon::Temp, t!("system-info-temperature"), {
                            let units = unit_system();
                            let value = match units {
                                UnitSystem::Metric => cel,
                                UnitSystem::Imperial => utils::celsius_to_fahrenheit(cel),
                            };
                            format!("{value}{}", units.temperature_symbol())
                        })
                    }))
                    .push(
                        Column::with_children(
                            self.data
                                .disks
                                .iter()
                                .map(|(mount_point, usage)| {
                                    Self::info_element(
                                        StaticIcon::Drive,
                                        t!("system-info-disk-usage", mount = mount_point.as_str()),
                                        match self.config.disk.format {
                                            DiskFormat::Percentage => {
                                                format!("{}%", usage.percentage)
                                            }
                                            DiskFormat::Fraction => {
                                                format!("{} GB", usage.fraction)
                                            }
                                        },
                                    )
                                })
                                .collect::<Vec<Element<_>>>(),
                        )
                        .spacing(space.xxs),
                    )
                    // Network detayları (IP + speed) artık NetworkSpeed
                    // modülünün menüsünde.
                    .spacing(space.xxs)
                    .padding([0.0, space.xs])
            )
            .spacing(space.xs),
        )
        .width(MenuSize::Medium)
        .into()
    }

    pub fn view(&'_ self) -> Element<'_, Message> {
        let space = use_theme(|t| t.space);
        let indicators = self.config.indicators.iter().filter_map(|i| match i {
            SystemInfoIndicator::Cpu => Some(Self::indicator_info_element(
                StaticIcon::Cpu,
                match self.config.cpu.format {
                    CpuFormat::Percentage => (self.data.cpu_usage.percentage.to_string(), "%"),
                    CpuFormat::Frequency => (self.data.cpu_usage.frequency.to_string(), " GHz"),
                },
                Some((
                    self.data.cpu_usage.percentage,
                    self.config.cpu.warn_threshold,
                    self.config.cpu.alert_threshold,
                )),
                None,
                // Percentage: "100%" (4); Frequency: "9.99 GHz" (8)
                match self.config.cpu.format {
                    CpuFormat::Percentage => 4.0,
                    CpuFormat::Frequency => 8.0,
                },
            )),

            SystemInfoIndicator::Memory => Some(Self::indicator_info_element(
                StaticIcon::Mem,
                match self.config.memory.format {
                    MemoryFormat::Percentage => {
                        (self.data.memory_usage.percentage.to_string(), "%")
                    }
                    MemoryFormat::Fraction => (self.data.memory_usage.fraction.clone(), " GiB"),
                },
                Some((
                    self.data.memory_usage.percentage,
                    self.config.memory.warn_threshold,
                    self.config.memory.alert_threshold,
                )),
                None,
                // Percentage: "100%" (4); Fraction: "99.99/99.99 GiB" (15)
                match self.config.memory.format {
                    MemoryFormat::Percentage => 4.0,
                    MemoryFormat::Fraction => 15.0,
                },
            )),

            SystemInfoIndicator::MemorySwap => Some(Self::indicator_info_element(
                StaticIcon::Mem,
                match self.config.memory.format {
                    MemoryFormat::Percentage => {
                        (self.data.memory_swap_usage.percentage.to_string(), "%")
                    }
                    MemoryFormat::Fraction => {
                        (self.data.memory_swap_usage.fraction.clone(), " GiB")
                    }
                },
                Some((
                    self.data.memory_swap_usage.percentage,
                    self.config.memory.warn_threshold,
                    self.config.memory.alert_threshold,
                )),
                Some(t!("system-info-swap-indicator-prefix")),
                {
                    // Prefix + space + percentage/fraction. Prefix string
                    // is localised, so measure at runtime.
                    let prefix_len = t!("system-info-swap-indicator-prefix").chars().count() as f32;
                    let body_chars = match self.config.memory.format {
                        MemoryFormat::Percentage => 4.0,
                        MemoryFormat::Fraction => 15.0,
                    };
                    prefix_len + 1.0 + body_chars
                },
            )),

            SystemInfoIndicator::Temperature => self.data.temperature.celsius.map(|cel| {
                let units = unit_system();
                let temp_value = match units {
                    UnitSystem::Metric => cel,
                    UnitSystem::Imperial => utils::celsius_to_fahrenheit(cel),
                };
                Self::indicator_info_element(
                    StaticIcon::Temp,
                    (temp_value, units.temperature_symbol()),
                    Some((
                        temp_value,
                        self.config.temperature.warn_threshold(),
                        self.config.temperature.alert_threshold(),
                    )),
                    None,
                    // "100°C" (5)
                    5.0,
                )
            }),
            SystemInfoIndicator::Disk(config) => {
                self.data.disks.iter().find_map(|(disk_mount, disk)| {
                    if disk_mount == &config.path {
                        let prefix_str = config.name.as_deref().unwrap_or(disk_mount).to_string();
                        let prefix_len = prefix_str.chars().count() as f32;
                        let body_chars = match self.config.disk.format {
                            DiskFormat::Percentage => 4.0,
                            DiskFormat::Fraction => 8.0,
                        };
                        Some(Self::indicator_info_element(
                            StaticIcon::Drive,
                            match self.config.disk.format {
                                DiskFormat::Percentage => (disk.percentage.to_string(), "%"),
                                DiskFormat::Fraction => (disk.fraction.clone(), " GB"),
                            },
                            Some((
                                disk.percentage,
                                self.config.disk.warn_threshold,
                                self.config.disk.alert_threshold,
                            )),
                            Some(prefix_str),
                            prefix_len + 1.0 + body_chars,
                        ))
                    } else {
                        None
                    }
                })
            }
        });

        Row::with_children(indicators)
            .align_y(Alignment::Center)
            .spacing(space.xxs)
            .into()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        every(Duration::from_secs(self.config.interval)).map(|_| Message::Update)
    }
}
