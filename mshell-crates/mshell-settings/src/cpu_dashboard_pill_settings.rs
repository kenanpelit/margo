//! Settings → Widgets → CPU Dashboard: the bar pill's capsule
//! (`bars.widgets.cpu_dashboard`) — which metrics show, their tinting, the
//! "no sensor" behaviour, the disk path and the hover table. Composed with
//! the generic menu geometry page in `settings/widget_pages.rs`. The pill
//! rebuilds itself live from these values.
//!
//! The switches are table-driven ([`SWITCHES`]): one row per bool key, each
//! writing through the same `update_config` closure, so adding a metric is
//! one table line.

use crate::row::Row;
use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    BarWidgetsStoreFields, BarsStoreFields, ConfigStoreFields, CpuDashboardBarWidget,
};
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::glib;
use relm4::gtk::glib::prelude::ObjectExt;
use relm4::gtk::prelude::{BoxExt, EditableExt, EntryExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, WidgetTemplate, gtk};

type Getter = fn(&CpuDashboardBarWidget) -> bool;
type Setter = fn(&mut CpuDashboardBarWidget, bool);

/// One switch row: `(row tag, title, description, read, write)`.
struct Switch {
    tag: &'static str,
    title: &'static str,
    desc: &'static str,
    get: Getter,
    set: Setter,
}

/// The capsule's metrics, in pill order.
const METRICS: &[Switch] = &[
    Switch {
        tag: "setting:/bars/widgets/cpu_dashboard/show_cpu",
        title: "CPU usage",
        desc: "Total CPU load in percent.",
        get: |c| c.show_cpu,
        set: |c, v| c.show_cpu = v,
    },
    Switch {
        tag: "setting:/bars/widgets/cpu_dashboard/show_temperature",
        title: "CPU temperature",
        desc: "Package temperature in °C.",
        get: |c| c.show_temperature,
        set: |c, v| c.show_temperature = v,
    },
    Switch {
        tag: "setting:/bars/widgets/cpu_dashboard/show_gpu",
        title: "iGPU busy",
        desc: "Intel integrated GPU load (xe / i915 sysfs, no root). Sampled only while this is on.",
        get: |c| c.show_gpu,
        set: |c, v| c.show_gpu = v,
    },
    Switch {
        tag: "setting:/bars/widgets/cpu_dashboard/show_ram",
        title: "Memory",
        desc: "RAM in use, in percent. Right-click on the pill also shows or hides it for this session.",
        get: |c| c.show_ram,
        set: |c, v| c.show_ram = v,
    },
    Switch {
        tag: "setting:/bars/widgets/cpu_dashboard/show_swap",
        title: "Swap",
        desc: "Swap in use, in percent. Shows a dash when the machine has no swap.",
        get: |c| c.show_swap,
        set: |c, v| c.show_swap = v,
    },
    Switch {
        tag: "setting:/bars/widgets/cpu_dashboard/show_disk",
        title: "Disk usage",
        desc: "How full the filesystem holding the path below is.",
        get: |c| c.show_disk,
        set: |c, v| c.show_disk = v,
    },
    Switch {
        tag: "setting:/bars/widgets/cpu_dashboard/show_network",
        title: "Network rate",
        desc: "Download and upload speed over the physical interfaces (VPN tunnels and bridges are not counted twice).",
        get: |c| c.show_network,
        set: |c, v| c.show_network = v,
    },
];

/// How the capsule looks and what its tooltip holds.
const DISPLAY: &[Switch] = &[
    Switch {
        tag: "setting:/bars/widgets/cpu_dashboard/highlight",
        title: "Colour by load",
        desc: "Each metric turns the accent colour when high and red when critical, on its own thresholds.",
        get: |c| c.highlight,
        set: |c, v| c.highlight = v,
    },
    Switch {
        tag: "setting:/bars/widgets/cpu_dashboard/hide_unavailable",
        title: "Hide metrics without a reading",
        desc: "Off: a metric whose sensor is missing shows a dash. On: it is left out of the pill.",
        get: |c| c.hide_unavailable,
        set: |c, v| c.hide_unavailable = v,
    },
    Switch {
        tag: "setting:/bars/widgets/cpu_dashboard/detailed_tooltip",
        title: "Detailed tooltip",
        desc: "On hover: used / total for each metric, CPU frequency, load average and the top CPU and memory processes. Read only while the pointer is on the pill.",
        get: |c| c.detailed_tooltip,
        set: |c, v| c.detailed_tooltip = v,
    },
];

pub(crate) struct CpuDashboardPillSettingsModel {
    cfg: CpuDashboardBarWidget,
    /// `(switch, its getter, the handler to block while syncing)`.
    switches: Vec<(gtk::Switch, Getter, glib::SignalHandlerId)>,
    disk_entry: gtk::Entry,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum CpuDashboardPillSettingsInput {
    /// The stored block changed (from anywhere).
    Effect(CpuDashboardBarWidget),
    DiskPath(String),
}

#[derive(Debug)]
pub(crate) enum CpuDashboardPillSettingsOutput {}

pub(crate) struct CpuDashboardPillSettingsInit {}

fn stored() -> CpuDashboardBarWidget {
    config_manager()
        .config()
        .bars()
        .widgets()
        .cpu_dashboard()
        .get_untracked()
}

/// Write one key of the block.
fn write(set: Setter, v: bool) {
    config_manager().update_config(move |c| set(&mut c.bars.widgets.cpu_dashboard, v));
}

/// A blank path means the root filesystem.
fn normalise_disk_path(raw: &str) -> String {
    match raw.trim() {
        "" => "/".to_string(),
        p => p.to_string(),
    }
}

#[relm4::component(pub)]
impl Component for CpuDashboardPillSettingsModel {
    type CommandOutput = ();
    type Input = CpuDashboardPillSettingsInput;
    type Output = CpuDashboardPillSettingsOutput;
    type Init = CpuDashboardPillSettingsInit;

    view! {
        #[root]
        gtk::ScrolledWindow {
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

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Bar pill",
                    set_halign: gtk::Align::Start,
                },
                gtk::Label {
                    add_css_class: "label-small",
                    set_label: "What the CPU Dashboard pill shows, left to right. Click on the pill opens the dashboard.",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                },
                #[local_ref]
                metrics_list -> gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,
                },
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Display",
                    set_halign: gtk::Align::Start,
                },
                #[local_ref]
                display_list -> gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,
                },
            }
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let cfg = stored();
        let metrics_list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let display_list = gtk::Box::new(gtk::Orientation::Vertical, 0);

        // The disk path sits right under the Disk usage switch.
        let disk_row = Row::init(());
        disk_row.set_widget_name("setting:/bars/widgets/cpu_dashboard/disk_path");
        disk_row.title.set_label("Disk path");
        disk_row.desc.set_label(
            "Any path; the pill shows the filesystem it lives on (for example / or /home). Press Enter to apply.",
        );
        let disk_entry = gtk::Entry::new();
        disk_entry.set_valign(gtk::Align::Center);
        disk_entry.set_width_chars(16);
        disk_entry.set_placeholder_text(Some("/"));
        disk_entry.set_text(&cfg.disk_path);
        let s = sender.clone();
        disk_entry.connect_activate(move |e| {
            s.input(CpuDashboardPillSettingsInput::DiskPath(
                e.text().to_string(),
            ));
        });
        disk_row.append(&disk_entry);

        let mut switches = Vec::new();
        for (list, table) in [(&metrics_list, METRICS), (&display_list, DISPLAY)] {
            for sw in table {
                let row = Row::init(());
                row.set_widget_name(sw.tag);
                row.title.set_label(sw.title);
                row.desc.set_label(sw.desc);
                let switch = gtk::Switch::new();
                switch.set_valign(gtk::Align::Center);
                switch.set_active((sw.get)(&cfg));
                let set = sw.set;
                let handler = switch.connect_state_set(move |_, v| {
                    write(set, v);
                    glib::Propagation::Proceed
                });
                row.append(&switch);
                list.append(&*row);
                if sw.tag.ends_with("/show_disk") {
                    list.append(&*disk_row);
                }
                switches.push((switch, sw.get, handler));
            }
        }

        let mut effects = EffectScope::new();
        let s = sender.clone();
        effects.push(move |_| {
            let cfg = config_manager()
                .config()
                .bars()
                .widgets()
                .cpu_dashboard()
                .get();
            s.input(CpuDashboardPillSettingsInput::Effect(cfg));
        });

        let model = CpuDashboardPillSettingsModel {
            cfg,
            switches,
            disk_entry,
            _effects: effects,
        };
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            CpuDashboardPillSettingsInput::Effect(cfg) => {
                for (switch, get, handler) in &self.switches {
                    let v = get(&cfg);
                    if switch.is_active() != v {
                        switch.block_signal(handler);
                        switch.set_active(v);
                        switch.unblock_signal(handler);
                    }
                }
                // Don't fight the user mid-edit: only sync the entry when
                // the stored value moved away from what it last held.
                if cfg.disk_path != self.cfg.disk_path {
                    self.disk_entry.set_text(&cfg.disk_path);
                }
                self.cfg = cfg;
            }
            CpuDashboardPillSettingsInput::DiskPath(raw) => {
                let path = normalise_disk_path(&raw);
                self.disk_entry.set_text(&path);
                config_manager()
                    .update_config(move |c| c.bars.widgets.cpu_dashboard.disk_path = path);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_switch_reads_and_writes_its_own_key() {
        for sw in METRICS.iter().chain(DISPLAY) {
            let key = sw.tag.rsplit('/').next().unwrap();
            for v in [true, false] {
                let mut c = CpuDashboardBarWidget::default();
                (sw.set)(&mut c, v);
                assert_eq!((sw.get)(&c), v, "{key}");
                let json = serde_json::to_value(&c).unwrap();
                assert_eq!(
                    json[key],
                    serde_json::Value::Bool(v),
                    "{key} tag matches the field"
                );
            }
        }
    }

    #[test]
    fn a_blank_disk_path_means_root() {
        assert_eq!(normalise_disk_path("  "), "/");
        assert_eq!(normalise_disk_path(" /home "), "/home");
    }
}
