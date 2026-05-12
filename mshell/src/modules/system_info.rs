//! System CPU% + temperature row.
//!
//! eww splits these into separate ring widgets next to battery/
//! memory; we collapse them into a compact icon + value indicator
//! pair so the bar stays tight. CPU% via the `cpu::Sampler` (delta
//! between two /proc/stat reads), temperature via the
//! `cpu_temp` sysfs zone.

use std::rc::Rc;

use gtk::prelude::*;
use gtk::{Box as GtkBox, Orientation};

use crate::services::{cpu, cpu_temp};
use crate::widgets::indicator::Indicator;

const CPU_ICON: &str = "\u{f4bc}"; // nf-md-cpu_64_bit
const TEMP_ICON: &str = "\u{f2c9}"; // nf-fa-thermometer_half
const REFRESH_SECS: u32 = 3;

pub fn build() -> GtkBox {
    let row = GtkBox::builder()
        .name("system-info")
        .orientation(Orientation::Horizontal)
        .spacing(4)
        .build();
    row.add_css_class("module");
    row.add_css_class("system-info");

    let sampler = Rc::new(cpu::Sampler::new());

    let cpu_ind = Indicator::icon_text("sys-cpu", CPU_ICON, "0%");
    cpu_ind.widget.add_css_class("sys-cpu");
    row.append(&cpu_ind.widget);

    let temp_ind = Indicator::icon_text("sys-temp", TEMP_ICON, "—");
    temp_ind.widget.add_css_class("sys-temp");
    row.append(&temp_ind.widget);

    // Seed the sampler so the first visible value isn't 0.
    sampler.sample();

    let sampler_tick = sampler.clone();
    let cpu_label = cpu_ind.label.clone();
    let cpu_widget = cpu_ind.widget.clone();
    let temp_label = temp_ind.label.clone();
    let temp_widget = temp_ind.widget.clone();
    glib::timeout_add_seconds_local(REFRESH_SECS, move || {
        let cpu_pct = sampler_tick.sample();
        if let Some(lbl) = &cpu_label {
            lbl.set_text(&format!("{cpu_pct}%"));
        }
        cpu_widget.remove_css_class("high");
        if cpu_pct >= 80 {
            cpu_widget.add_css_class("high");
        }

        match cpu_temp::current_celsius() {
            Some(t) => {
                if let Some(lbl) = &temp_label {
                    lbl.set_text(&format!("{t}°C"));
                }
                temp_widget.remove_css_class("hot");
                if t >= 80 {
                    temp_widget.add_css_class("hot");
                }
                temp_widget.set_visible(true);
            }
            None => {
                temp_widget.set_visible(false);
            }
        }

        glib::ControlFlow::Continue
    });

    row
}
