//! Settings → About.
//!
//! Read-only system information — OS, kernel, host, CPU / GPU / memory,
//! desktop session, and the margo version. Everything is read from the
//! standard `/proc` + `/etc/os-release` files (and a best-effort `lspci`
//! for the GPU), so the page never needs elevated privileges.

use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};

#[derive(Debug)]
pub(crate) struct AboutSettingsModel {
    os: String,
    kernel: String,
    host: String,
    desktop: String,
    version: String,
    cpu: String,
    gpu: String,
    memory: String,
    uptime: String,
}

#[derive(Debug)]
pub(crate) enum AboutSettingsInput {
    Refresh,
    /// The `lspci` GPU probe finished off-thread (see `spawn_gpu`).
    GpuLoaded(String),
}

#[derive(Debug)]
pub(crate) enum AboutSettingsOutput {}

pub(crate) struct AboutSettingsInit {}

#[derive(Debug)]
pub(crate) enum AboutSettingsCommandOutput {}

#[relm4::component(pub)]
impl Component for AboutSettingsModel {
    type CommandOutput = AboutSettingsCommandOutput;
    type Input = AboutSettingsInput;
    type Output = AboutSettingsOutput;
    type Init = AboutSettingsInit;

    view! {
        #[root]
        gtk::ScrolledWindow {
            // The page is built eagerly at startup, so a once-at-init read
            // would freeze uptime (and the rest) at login time. Re-read every
            // time the page is mapped — i.e. each time About is opened.
            connect_map[sender] => move |_| {
                sender.input(AboutSettingsInput::Refresh);
            },
            set_vscrollbar_policy: gtk::PolicyType::Automatic,
            set_hscrollbar_policy: gtk::PolicyType::Never,
            set_propagate_natural_height: false,
            set_propagate_natural_width: false,
            set_hexpand: true,
            set_vexpand: true,

            gtk::Box {
                add_css_class: "settings-page",
                set_orientation: gtk::Orientation::Vertical,
                set_hexpand: true,
                set_spacing: 16,

                gtk::Box {
                    add_css_class: "settings-hero",
                    set_orientation: gtk::Orientation::Horizontal,
                    set_halign: gtk::Align::Start,
                    set_spacing: 16,
                    gtk::Image {
                        add_css_class: "settings-hero-icon",
                        set_icon_name: Some("help-about-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "About",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "This system at a glance — read-only.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                gtk::Box {
                    add_css_class: "settings-about-quote",
                    set_orientation: gtk::Orientation::Vertical,
                    set_hexpand: true,
                    gtk::Label {
                        add_css_class: "settings-about-quote-text",
                        set_label: "Margo is a deeply personal Linux desktop \
                                    environment built by a single human \
                                    amplified by AI — an experiment in whether \
                                    one person can design, implement, and \
                                    maintain a complete modern desktop stack \
                                    alone.",
                        set_halign: gtk::Align::Start,
                        set_xalign: 0.0,
                        set_wrap: true,
                        set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                    },
                    gtk::Label {
                        add_css_class: "settings-about-quote-attr",
                        set_label: "~ kenp",
                        set_halign: gtk::Align::End,
                        set_xalign: 1.0,
                    },
                },

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "System",
                    set_halign: gtk::Align::Start,
                },

                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    #[template] InfoRow { #[template_child] name { set_label: "Operating system" }, #[template_child] value { #[watch] set_label: &model.os } },
                    #[template] InfoRow { #[template_child] name { set_label: "Kernel" }, #[template_child] value { #[watch] set_label: &model.kernel } },
                    #[template] InfoRow { #[template_child] name { set_label: "Hostname" }, #[template_child] value { #[watch] set_label: &model.host } },
                    #[template] InfoRow { #[template_child] name { set_label: "Desktop" }, #[template_child] value { #[watch] set_label: &model.desktop } },
                    #[template] InfoRow { #[template_child] name { set_label: "margo version" }, #[template_child] value { #[watch] set_label: &model.version } },
                    #[template] InfoRow { #[template_child] name { set_label: "Uptime" }, #[template_child] value { #[watch] set_label: &model.uptime } },
                },

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Hardware",
                    set_halign: gtk::Align::Start,
                    set_margin_top: 12,
                },

                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    #[template] InfoRow { #[template_child] name { set_label: "Processor" }, #[template_child] value { #[watch] set_label: &model.cpu } },
                    #[template] InfoRow { #[template_child] name { set_label: "Graphics" }, #[template_child] value { #[watch] set_label: &model.gpu } },
                    #[template] InfoRow { #[template_child] name { set_label: "Memory" }, #[template_child] value { #[watch] set_label: &model.memory } },
                },

                gtk::Button {
                    add_css_class: "ok-button-surface",
                    set_halign: gtk::Align::Start,
                    set_margin_top: 12,
                    set_label: "Refresh",
                    connect_clicked[sender] => move |_| {
                        sender.input(AboutSettingsInput::Refresh);
                    },
                },
            }
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        // Everything here is a cheap /proc + /etc read EXCEPT the GPU, which
        // shells out to `lspci` (tens of ms). Settings pages are built eagerly
        // at login, so seed the GPU with a placeholder and probe it off-thread.
        let model = read_info("…".to_string());
        spawn_gpu(&sender);
        let widgets = view_output!();
        let _ = root;
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            AboutSettingsInput::Refresh => {
                // Re-read the cheap fields immediately; keep the last known GPU
                // string (avoids a flicker back to the placeholder) and re-probe
                // it off-thread.
                *self = read_info(self.gpu.clone());
                spawn_gpu(&sender);
            }
            AboutSettingsInput::GpuLoaded(gpu) => self.gpu = gpu,
        }
    }
}

/// Probe the GPU via `lspci` off the GTK main thread, delivering the result
/// back as `GpuLoaded` (processed on the main thread by `update`).
fn spawn_gpu(sender: &ComponentSender<AboutSettingsModel>) {
    let sender = sender.clone();
    std::thread::spawn(move || {
        sender.input(AboutSettingsInput::GpuLoaded(gpu_name()));
    });
}

fn read_info(gpu: String) -> AboutSettingsModel {
    AboutSettingsModel {
        os: os_pretty_name(),
        kernel: trim_or_dash(std::fs::read_to_string("/proc/sys/kernel/osrelease").ok()),
        host: trim_or_dash(std::fs::read_to_string("/proc/sys/kernel/hostname").ok()),
        desktop: desktop_line(),
        version: format!("v{}", env!("CARGO_PKG_VERSION")),
        cpu: cpu_model(),
        gpu,
        memory: mem_total(),
        uptime: uptime(),
    }
}

fn trim_or_dash(s: Option<String>) -> String {
    match s {
        Some(s) if !s.trim().is_empty() => s.trim().to_string(),
        _ => "—".to_string(),
    }
}

/// `PRETTY_NAME="…"` from /etc/os-release.
fn os_pretty_name() -> String {
    let Ok(text) = std::fs::read_to_string("/etc/os-release") else {
        return "—".to_string();
    };
    text.lines()
        .find_map(|l| l.strip_prefix("PRETTY_NAME="))
        .map(|v| v.trim().trim_matches('"').to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "—".to_string())
}

fn desktop_line() -> String {
    let de = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| "margo".to_string());
    match std::env::var("XDG_SESSION_TYPE") {
        Ok(t) if !t.is_empty() => format!("{de} ({t})"),
        _ => de,
    }
}

/// First `model name` line in /proc/cpuinfo.
fn cpu_model() -> String {
    let Ok(text) = std::fs::read_to_string("/proc/cpuinfo") else {
        return "—".to_string();
    };
    text.lines()
        .find_map(|l| l.split_once(':').filter(|(k, _)| k.trim() == "model name"))
        .map(|(_, v)| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "—".to_string())
}

/// `MemTotal` (kB) from /proc/meminfo → GiB.
fn mem_total() -> String {
    let Ok(text) = std::fs::read_to_string("/proc/meminfo") else {
        return "—".to_string();
    };
    text.lines()
        .find_map(|l| l.strip_prefix("MemTotal:"))
        .and_then(|v| v.trim().trim_end_matches(" kB").trim().parse::<f64>().ok())
        .map(|kb| format!("{:.1} GiB", kb / 1024.0 / 1024.0))
        .unwrap_or_else(|| "—".to_string())
}

/// Best-effort GPU name from `lspci` (VGA / 3D / Display controllers).
fn gpu_name() -> String {
    let out = std::process::Command::new("lspci").output();
    let Ok(out) = out else {
        return "—".to_string();
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let names: Vec<String> = text
        .lines()
        .filter(|l| {
            let l = l.to_ascii_lowercase();
            l.contains("vga compatible controller")
                || l.contains("3d controller")
                || l.contains("display controller")
        })
        .filter_map(|l| l.split_once(": ").map(|(_, name)| name.trim().to_string()))
        .collect();
    if names.is_empty() {
        "—".to_string()
    } else {
        names.join(", ")
    }
}

/// Seconds from /proc/uptime → "Xd Yh Zm".
fn uptime() -> String {
    let Ok(text) = std::fs::read_to_string("/proc/uptime") else {
        return "—".to_string();
    };
    let Some(secs) = text
        .split_whitespace()
        .next()
        .and_then(|s| s.parse::<f64>().ok())
    else {
        return "—".to_string();
    };
    let total = secs as u64;
    let (d, h, m) = (total / 86400, (total % 86400) / 3600, (total % 3600) / 60);
    let mut parts = Vec::new();
    if d > 0 {
        parts.push(format!("{d}d"));
    }
    if h > 0 || d > 0 {
        parts.push(format!("{h}h"));
    }
    parts.push(format!("{m}m"));
    parts.join(" ")
}

/// An About info row: a left-hand name + a right-aligned, selectable value.
#[relm4::widget_template(pub)]
impl relm4::WidgetTemplate for InfoRow {
    view! {
        gtk::Box {
            add_css_class: "action-row",
            set_orientation: gtk::Orientation::Horizontal,
            set_spacing: 16,
            #[name = "name"]
            gtk::Label {
                add_css_class: "label-medium-bold",
                set_halign: gtk::Align::Start,
                set_hexpand: true,
                set_xalign: 0.0,
            },
            #[name = "value"]
            gtk::Label {
                add_css_class: "label-medium",
                set_halign: gtk::Align::End,
                set_xalign: 1.0,
                set_wrap: true,
                set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                set_selectable: true,
            },
        }
    }
}
