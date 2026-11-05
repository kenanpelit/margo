//! World Clocks — a Clock-menu widget (`MenuWidget::WorldClocks`) that
//! compares the device's time with a list of IANA zones
//! (`tempo.world_clocks`).
//!
//! The device row comes first, then one boxed-list row per city: name,
//! local time, "3h ahead · Tomorrow morning · Outside working hours" and
//! an hour strip around now with the working day (`tempo.work_day_start`
//! .. `work_day_end`) shaded. Per row: pin (make it the device timezone —
//! `timedatectl set-timezone`, which asks polkit), move up, remove. The
//! add button opens an inline search over the bundled zone database
//! (chrono-tz; no subprocess, no network). The rows refresh once a minute
//! and only while the menu is open.

pub(crate) mod logic;

use chrono::{Datelike, Local, Utc};
use logic::HourCell;
use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, GeneralStoreFields};
use mshell_osd::toast::{ToastEvent, ToastSeverity, push_toast};
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::glib::{self, SourceId};
use relm4::gtk::prelude::{BoxExt, ButtonExt, EditableExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::Duration;

/// Hours shown before / after now in each strip.
const STRIP_BEFORE: u32 = 2;
const STRIP_AFTER: u32 = 9;
/// Search results shown at once.
const SEARCH_LIMIT: usize = 8;

pub(crate) struct WorldClocksModel {
    rows: gtk::Box,
    empty: gtk::Label,
    search_revealer: gtk::Revealer,
    search: gtk::SearchEntry,
    results: gtk::Box,
    tick: Option<SourceId>,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum WorldClocksInput {
    Refresh,
    ToggleSearch,
    SearchChanged,
    Add(String),
    Remove(usize),
    MoveUp(usize),
    Pin(String),
    ParentRevealChanged(bool),
}

pub(crate) struct WorldClocksInit {}

#[relm4::component(pub(crate))]
impl Component for WorldClocksModel {
    type CommandOutput = ();
    type Input = WorldClocksInput;
    type Output = ();
    type Init = WorldClocksInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "world-clocks",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 8,

            gtk::Box {
                set_spacing: 8,
                gtk::Label {
                    add_css_class: "world-clocks-section-label",
                    set_label: "World clocks",
                    set_hexpand: true,
                    set_halign: gtk::Align::Start,
                },
                #[name = "add_button"]
                gtk::Button {
                    add_css_class: "ok-button-flat",
                    add_css_class: "world-clocks-add",
                    set_icon_name: "list-add-symbolic",
                    set_tooltip_text: Some("Add a city"),
                    connect_clicked => WorldClocksInput::ToggleSearch,
                },
            },

            #[local_ref]
            search_revealer_widget -> gtk::Revealer {
                set_transition_type: gtk::RevealerTransitionType::SlideDown,
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 4,
                    #[local_ref]
                    search_widget -> gtk::SearchEntry {
                        add_css_class: "menu-search",
                        set_placeholder_text: Some("Search a city or region…"),
                        connect_search_changed => WorldClocksInput::SearchChanged,
                    },
                    #[local_ref]
                    results_widget -> gtk::Box {
                        add_css_class: "ok-boxed-list",
                        add_css_class: "compact",
                        set_orientation: gtk::Orientation::Vertical,
                        set_visible: false,
                    },
                },
            },

            #[local_ref]
            rows_widget -> gtk::Box {
                add_css_class: "ok-boxed-list",
                add_css_class: "compact",
                set_orientation: gtk::Orientation::Vertical,
            },
            #[local_ref]
            empty_widget -> gtk::Label {
                add_css_class: "world-clocks-empty",
                set_label: "Add a city to compare times.",
                set_halign: gtk::Align::Center,
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let rows_widget = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let empty_widget = gtk::Label::new(None);
        let search_revealer_widget = gtk::Revealer::new();
        let search_widget = gtk::SearchEntry::new();
        let results_widget = gtk::Box::new(gtk::Orientation::Vertical, 0);

        let mut effects = EffectScope::new();
        let s = sender.clone();
        effects.push(move |_| {
            let _ = config_manager().config().tempo().get();
            let _ = config_manager()
                .config()
                .general()
                .clock_format_24_h()
                .get();
            s.input(WorldClocksInput::Refresh);
        });

        let model = WorldClocksModel {
            rows: rows_widget.clone(),
            empty: empty_widget.clone(),
            search_revealer: search_revealer_widget.clone(),
            search: search_widget.clone(),
            results: results_widget.clone(),
            tick: None,
            _effects: effects,
        };
        let widgets = view_output!();
        mshell_common::a11y::name(&widgets.add_button, "Add a city");
        let s = sender.clone();
        search_widget.connect_activate(move |entry| {
            if let Some(zone) = logic::search_zones(&entry.text(), 1).first() {
                s.input(WorldClocksInput::Add((*zone).to_string()));
            }
        });
        rebuild(&model, &sender);
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            WorldClocksInput::Refresh => rebuild(self, &sender),
            WorldClocksInput::ToggleSearch => {
                let open = !self.search_revealer.reveals_child();
                self.search_revealer.set_reveal_child(open);
                if open {
                    self.search.grab_focus();
                } else {
                    self.search.set_text("");
                }
            }
            WorldClocksInput::SearchChanged => fill_results(self, &sender),
            WorldClocksInput::Add(zone) => {
                config_manager().update_config(move |c| {
                    if !c.tempo.world_clocks.contains(&zone) {
                        c.tempo.world_clocks.push(zone);
                    }
                });
                self.search.set_text("");
                self.search_revealer.set_reveal_child(false);
            }
            WorldClocksInput::Remove(idx) => {
                config_manager().update_config(move |c| {
                    if idx < c.tempo.world_clocks.len() {
                        c.tempo.world_clocks.remove(idx);
                    }
                });
            }
            WorldClocksInput::MoveUp(idx) => {
                config_manager().update_config(move |c| {
                    if idx > 0 && idx < c.tempo.world_clocks.len() {
                        c.tempo.world_clocks.swap(idx - 1, idx);
                    }
                });
            }
            WorldClocksInput::Pin(zone) => pin_device_zone(zone, sender.clone()),
            WorldClocksInput::ParentRevealChanged(visible) => {
                if visible {
                    rebuild(self, &sender);
                    if self.tick.is_none() {
                        self.tick = Some(start_tick(&sender));
                    }
                } else if let Some(id) = self.tick.take() {
                    id.remove();
                    self.search_revealer.set_reveal_child(false);
                }
            }
        }
    }
}

impl Drop for WorldClocksModel {
    fn drop(&mut self) {
        if let Some(id) = self.tick.take() {
            id.remove();
        }
    }
}

/// Refresh once a minute while open (the readouts are `HH:MM`).
fn start_tick(sender: &ComponentSender<WorldClocksModel>) -> SourceId {
    let s = sender.clone();
    glib::timeout_add_local(Duration::from_secs(60), move || {
        if s.input_sender().send(WorldClocksInput::Refresh).is_err() {
            return glib::ControlFlow::Break;
        }
        glib::ControlFlow::Continue
    })
}

struct Prefs {
    zones: Vec<String>,
    use_24h: bool,
    work: (u32, u32),
}

fn prefs() -> Prefs {
    let tempo = config_manager().config().tempo().get_untracked();
    Prefs {
        zones: tempo.world_clocks,
        use_24h: config_manager()
            .config()
            .general()
            .clock_format_24_h()
            .get_untracked(),
        work: (tempo.work_day_start, tempo.work_day_end),
    }
}

/// The device's zone id from `/etc/localtime` (what `timedatectl` sets).
fn device_zone() -> Option<String> {
    let target = std::fs::read_link("/etc/localtime").ok()?;
    logic::zone_from_localtime_target(&target.to_string_lossy())
}

fn clear(b: &gtk::Box) {
    while let Some(child) = b.first_child() {
        b.remove(&child);
    }
}

fn rebuild(model: &WorldClocksModel, sender: &ComponentSender<WorldClocksModel>) {
    clear(&model.rows);
    let p = prefs();
    let now = Utc::now();
    let local = Local::now();
    let device_offset = local.offset().local_minus_utc() / 60;
    let device_day = (local.year(), local.ordinal());
    let device = device_zone();

    // The device row.
    let device_tz = device.as_deref().and_then(logic::parse_zone);
    let title = match &device {
        Some(id) => format!("This device · {}", logic::city_label(id)),
        None => "This device".to_string(),
    };
    let subtitle = device.clone().unwrap_or_else(|| "Local time".to_string());
    let time = if p.use_24h {
        local.format("%H:%M").to_string()
    } else {
        local.format("%-I:%M %P").to_string()
    };
    let strip = device_tz
        .map(|tz| logic::hour_strip(tz, now, STRIP_BEFORE, STRIP_AFTER, p.use_24h, p.work));
    model.rows.append(&city_row(
        &title,
        &subtitle,
        &time,
        strip.as_deref(),
        None,
        sender,
    ));

    let mut shown = 0;
    for (idx, id) in p.zones.iter().enumerate() {
        let Some(tz) = logic::parse_zone(id) else {
            continue; // a hand-edited typo: skip it rather than show garbage
        };
        if device.as_deref() == Some(id.as_str()) {
            continue; // already the device row
        }
        shown += 1;
        let strip = logic::hour_strip(tz, now, STRIP_BEFORE, STRIP_AFTER, p.use_24h, p.work);
        let row = city_row(
            &logic::city_label(id),
            &logic::describe(tz, now, device_offset, device_day, p.work),
            &logic::clock_text(tz, now, p.use_24h),
            Some(&strip),
            Some((idx, id.as_str())),
            sender,
        );
        model.rows.append(&row);
    }
    model.empty.set_visible(shown == 0);
}

/// One boxed-list row: title / subtitle / time, the hour strip, and the
/// row actions for a comparison city (`actions = Some((index, zone))`).
fn city_row(
    title: &str,
    subtitle: &str,
    time: &str,
    strip: Option<&[HourCell]>,
    actions: Option<(usize, &str)>,
    sender: &ComponentSender<WorldClocksModel>,
) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Vertical, 6);
    row.add_css_class("world-clock-row");
    if actions.is_none() {
        row.add_css_class("device");
    }

    let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
    text.set_hexpand(true);
    let t = gtk::Label::new(Some(title));
    t.add_css_class("world-clock-title");
    t.set_halign(gtk::Align::Start);
    text.append(&t);
    let s = gtk::Label::new(Some(subtitle));
    s.add_css_class("world-clock-subtitle");
    s.set_halign(gtk::Align::Start);
    s.set_xalign(0.0);
    s.set_wrap(true);
    text.append(&s);
    top.append(&text);

    let clock = gtk::Label::new(Some(time));
    clock.add_css_class("world-clock-time");
    clock.set_valign(gtk::Align::Center);
    top.append(&clock);

    if let Some((idx, zone)) = actions {
        let action = |icon: &str, name: &str, msg: WorldClocksInput| {
            let b = gtk::Button::from_icon_name(icon);
            b.add_css_class("ok-button-flat");
            b.add_css_class("world-clock-action");
            b.set_valign(gtk::Align::Center);
            b.set_tooltip_text(Some(name));
            mshell_common::a11y::name(&b, name);
            let s = sender.clone();
            let msg = std::cell::Cell::new(Some(msg));
            b.connect_clicked(move |_| {
                if let Some(m) = msg.take() {
                    s.input(m);
                }
            });
            top.append(&b);
        };
        action(
            "mark-location-symbolic",
            "Make this the device timezone",
            WorldClocksInput::Pin(zone.to_string()),
        );
        if idx > 0 {
            action("go-up-symbolic", "Move up", WorldClocksInput::MoveUp(idx));
        }
        action(
            "user-trash-symbolic",
            "Remove",
            WorldClocksInput::Remove(idx),
        );
    }
    row.append(&top);

    if let Some(cells) = strip {
        let strip = gtk::Box::new(gtk::Orientation::Horizontal, 1);
        strip.add_css_class("world-clock-strip");
        strip.set_homogeneous(true);
        for cell in cells {
            let l = gtk::Label::new(Some(&cell.label));
            l.add_css_class("world-clock-cell");
            if cell.now {
                l.add_css_class("now");
            } else if cell.work {
                l.add_css_class("work");
            }
            if cell.midnight {
                l.add_css_class("midnight");
            }
            strip.append(&l);
        }
        row.append(&strip);
    }
    row
}

fn fill_results(model: &WorldClocksModel, sender: &ComponentSender<WorldClocksModel>) {
    clear(&model.results);
    let query = model.search.text();
    let hits = logic::search_zones(&query, SEARCH_LIMIT);
    let now = Utc::now();
    let use_24h = prefs().use_24h;
    for zone in &hits {
        let b = gtk::Button::new();
        b.add_css_class("ok-button-flat");
        b.add_css_class("world-clock-result");
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let city = gtk::Label::new(Some(&logic::city_label(zone)));
        city.add_css_class("world-clock-title");
        line.append(&city);
        let id = gtk::Label::new(Some(zone));
        id.add_css_class("world-clock-subtitle");
        id.set_hexpand(true);
        id.set_halign(gtk::Align::Start);
        line.append(&id);
        if let Some(tz) = logic::parse_zone(zone) {
            let t = gtk::Label::new(Some(&logic::clock_text(tz, now, use_24h)));
            t.add_css_class("world-clock-subtitle");
            line.append(&t);
        }
        b.set_child(Some(&line));
        let s = sender.clone();
        let zone = zone.to_string();
        b.connect_clicked(move |_| s.input(WorldClocksInput::Add(zone.clone())));
        model.results.append(&b);
    }
    model.results.set_visible(!hits.is_empty());
}

/// `timedatectl set-timezone <zone>` off the GTK thread (systemd-timedated
/// asks polkit). A failure — tool missing, auth refused — is a toast.
fn pin_device_zone(zone: String, sender: ComponentSender<WorldClocksModel>) {
    if !margo_fs::path::on_path("timedatectl") {
        push_toast(ToastEvent {
            icon: "dialog-warning-symbolic".to_string(),
            title: "Can't change the timezone".to_string(),
            body: Some("timedatectl (systemd) is not installed.".to_string()),
            severity: ToastSeverity::Warn,
        });
        return;
    }
    std::thread::spawn(move || {
        let status = std::process::Command::new("timedatectl")
            .args(["set-timezone", &zone])
            .status();
        let ok = matches!(status, Ok(s) if s.success());
        if !ok {
            push_toast(ToastEvent {
                icon: "dialog-error-symbolic".to_string(),
                title: "Timezone not changed".to_string(),
                body: Some(format!(
                    "timedatectl set-timezone {zone} failed or was cancelled."
                )),
                severity: ToastSeverity::Danger,
            });
        }
        let _ = sender.input_sender().send(WorldClocksInput::Refresh);
    });
}
