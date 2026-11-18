//! Screen Time menu — the panel for `MenuType::ScreenTime`.
//!
//! Panel header (§12) with today's total as subtitle, a Today / 7 days
//! switch, a seven-day bar strip (one level bar per day, today last) and
//! the most used apps of the chosen range as boxed-list rows (icon, name,
//! time, share bar). Apps under `screen_time.hide_under_minutes` are left
//! out of the list. While recording is off the panel says how to turn it
//! on instead.

use crate::menus::menu_widgets::panel_header::PanelHeader;
use crate::menus::reveal_poll::{RevealGate, watch_stream};
use crate::screen_time::engine::format_duration;
use crate::screen_time::{self, ScreenTimeState};
use chrono::{Datelike, Local};
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, ScreenTimeConfigStoreFields};
use mshell_config::schema::menu_widgets::MenuWidget;
use reactive_graph::prelude::GetUntracked;
use relm4::gtk::prelude::{
    AppInfoExt, BoxExt, ButtonExt, OrientableExt, ToggleButtonExt, WidgetExt,
};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

pub(crate) struct ScreenTimeMenuWidgetModel {
    state: ScreenTimeState,
    week: bool,
    list: gtk::Box,
    days: gtk::Box,
    subtitle: gtk::Label,
}

#[derive(Debug)]
pub(crate) enum ScreenTimeMenuWidgetInput {
    Range(bool),
    Enable,
}

#[derive(Debug)]
pub(crate) enum ScreenTimeMenuWidgetOutput {}

pub(crate) struct ScreenTimeMenuWidgetInit {}

#[derive(Debug)]
pub(crate) enum ScreenTimeMenuWidgetCommandOutput {
    State(ScreenTimeState),
}

#[relm4::component(pub(crate))]
impl Component for ScreenTimeMenuWidgetModel {
    type CommandOutput = ScreenTimeMenuWidgetCommandOutput;
    type Input = ScreenTimeMenuWidgetInput;
    type Output = ScreenTimeMenuWidgetOutput;
    type Init = ScreenTimeMenuWidgetInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "screen-time-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 10,

            #[local_ref]
            header_root -> gtk::Box {},

            #[name = "off_box"]
            gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 8,
                gtk::Label {
                    set_css_classes: &["label-small", "dim-label"],
                    set_label: "Screen time is off. When on, mshell counts how long each app has the keyboard focus (paused while you are away or the session is locked) and keeps the days in $XDG_STATE_HOME/margo/screen-time.json.",
                    set_wrap: true,
                    set_xalign: 0.0,
                },
                gtk::Button {
                    add_css_class: "ok-button-primary",
                    set_label: "Turn on",
                    set_halign: gtk::Align::Start,
                    connect_clicked[sender] => move |_| {
                        sender.input(ScreenTimeMenuWidgetInput::Enable);
                    },
                },
            },

            #[name = "on_box"]
            gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 10,

                gtk::Box {
                    add_css_class: "linked",
                    set_halign: gtk::Align::Start,
                    #[name = "today_btn"]
                    gtk::ToggleButton {
                        set_label: "Today",
                        set_active: true,
                        connect_toggled[sender] => move |b| {
                            if b.is_active() {
                                sender.input(ScreenTimeMenuWidgetInput::Range(false));
                            }
                        },
                    },
                    gtk::ToggleButton {
                        set_label: "7 days",
                        set_group: Some(&today_btn),
                        connect_toggled[sender] => move |b| {
                            if b.is_active() {
                                sender.input(ScreenTimeMenuWidgetInput::Range(true));
                            }
                        },
                    },
                },

                #[local_ref]
                days -> gtk::Box {
                    add_css_class: "screen-time-days",
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 6,
                    set_homogeneous: true,
                },

                #[name = "broken_label"]
                gtk::Label {
                    set_css_classes: &["label-small-error"],
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_visible: false,
                },

                #[name = "fill_list"]
                gtk::ScrolledWindow {
                    set_hscrollbar_policy: gtk::PolicyType::Never,
                    set_min_content_height: 0,
                    set_propagate_natural_height: true,

                    #[local_ref]
                    list -> gtk::Box {
                        set_css_classes: &["ok-boxed-list", "compact", "screen-time-list"],
                        set_orientation: gtk::Orientation::Vertical,
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
        let header = PanelHeader::new(
            "preferences-system-time-symbolic",
            MenuWidget::ScreenTime.display_name(),
        )
        .with_subtitle();
        let header_root = header.root().clone();
        let subtitle = header.subtitle().cloned().unwrap_or_default();
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let days = gtk::Box::new(gtk::Orientation::Horizontal, 6);

        let open = RevealGate::follow_mapped(&root);
        sender.command(|out, shutdown| async move {
            let watch = open.stream_while_revealed(
                || watch_stream(screen_time::subscribe()),
                |state| {
                    let _ = out.send(ScreenTimeMenuWidgetCommandOutput::State(state));
                },
            );
            tokio::select! {
                () = shutdown.wait() => {}
                () = watch => {}
            }
        });

        let model = ScreenTimeMenuWidgetModel {
            state: ScreenTimeState::default(),
            week: false,
            list: list.clone(),
            days: days.clone(),
            subtitle,
        };
        let widgets = view_output!();
        crate::menus::list_fit::mark_menu_list(&widgets.fill_list, 420);
        model.render(&widgets);
        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        msg: Self::Input,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match msg {
            ScreenTimeMenuWidgetInput::Range(week) => self.week = week,
            ScreenTimeMenuWidgetInput::Enable => {
                config_manager().update_config(|c| c.screen_time.enabled = true);
            }
        }
        self.render(widgets);
    }

    fn update_cmd_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        msg: Self::CommandOutput,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match msg {
            ScreenTimeMenuWidgetCommandOutput::State(s) => {
                self.state = s;
                self.render(widgets);
            }
        }
    }
}

/// `Mo`, `Tu`, … for the day strip.
fn weekday_short(d: chrono::NaiveDate) -> &'static str {
    ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"][d.weekday().num_days_from_monday() as usize]
}

fn clear(b: &gtk::Box) {
    while let Some(child) = b.first_child() {
        b.remove(&child);
    }
}

impl ScreenTimeMenuWidgetModel {
    fn render(&self, widgets: &ScreenTimeMenuWidgetModelWidgets) {
        let s = &self.state;
        widgets.off_box.set_visible(!s.enabled);
        widgets.on_box.set_visible(s.enabled);
        let today = Local::now().date_naive();
        let h = &s.history;
        let total_today = h.day_total(today);
        let week = h.daily_totals(today, 7);
        let week_total: u64 = week.iter().map(|(_, t)| t).sum();
        self.subtitle.set_label(&if !s.enabled {
            "Off".to_string()
        } else if self.week {
            format!(
                "7 days: {} · {} a day",
                format_duration(week_total),
                format_duration(week_total / 7)
            )
        } else {
            let state = if s.counting { "" } else { " · paused" };
            format!("Today: {}{state}", format_duration(total_today))
        });
        widgets.broken_label.set_visible(s.broken.is_some());
        if let Some(e) = &s.broken {
            widgets.broken_label.set_label(&format!(
                "screen-time.json can't be read or saved ({e}); today's time is kept until the shell restarts."
            ));
        }

        clear(&self.days);
        let max = week.iter().map(|(_, t)| *t).max().unwrap_or(0).max(1);
        for (d, t) in &week {
            let col = gtk::Box::new(gtk::Orientation::Vertical, 2);
            col.set_tooltip_text(Some(&format!(
                "{} · {}",
                d.format("%a %d %b"),
                format_duration(*t)
            )));
            let bar = gtk::LevelBar::for_interval(0.0, max as f64);
            bar.set_orientation(gtk::Orientation::Vertical);
            bar.set_inverted(true);
            bar.set_value(*t as f64);
            bar.set_size_request(-1, 48);
            bar.add_css_class("screen-time-day-bar");
            if *d == today {
                bar.add_css_class("today");
            }
            col.append(&bar);
            let l = gtk::Label::new(Some(weekday_short(*d)));
            l.set_css_classes(&["label-small", "dim-label"]);
            col.append(&l);
            self.days.append(&col);
        }

        clear(&self.list);
        let min = config_manager()
            .config()
            .screen_time()
            .hide_under_minutes()
            .get_untracked();
        let apps = h.top_apps(today, if self.week { 7 } else { 1 }, u64::from(min) * 60);
        let top = apps.first().map_or(1, |(_, s)| (*s).max(1));
        for (app, secs) in apps.iter().take(30) {
            self.list.append(&row(app, *secs, top));
        }
        if apps.is_empty() {
            let l = gtk::Label::new(Some("Nothing recorded yet."));
            l.set_css_classes(&["label-small", "dim-label"]);
            l.set_margin_top(8);
            l.set_margin_bottom(8);
            self.list.append(&l);
        }
    }
}

/// The `.desktop` entry of an app id (`firefox`, `org.gnome.Nautilus`),
/// also tried lower-cased.
fn desktop_info(app: &str) -> Option<gio_unix::DesktopAppInfo> {
    gio_unix::DesktopAppInfo::new(&format!("{app}.desktop"))
        .or_else(|| gio_unix::DesktopAppInfo::new(&format!("{}.desktop", app.to_lowercase())))
}

fn row(app: &str, secs: u64, top: u64) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.add_css_class("screen-time-row");
    let info = desktop_info(app);
    let icon = match info.as_ref().and_then(|i| i.icon()) {
        Some(g) => gtk::Image::from_gicon(&g),
        None => gtk::Image::from_icon_name("application-x-executable-symbolic"),
    };
    icon.set_pixel_size(24);
    row.append(&icon);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 4);
    text.set_hexpand(true);
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let shown = info.map_or_else(|| app.to_string(), |i| i.display_name().to_string());
    let name = gtk::Label::new(Some(&shown));
    name.set_tooltip_text(Some(app));
    name.add_css_class("label-medium");
    name.set_xalign(0.0);
    name.set_hexpand(true);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    head.append(&name);
    let t = gtk::Label::new(Some(&format_duration(secs)));
    t.set_css_classes(&["label-small", "screen-time-row-time"]);
    head.append(&t);
    text.append(&head);
    let bar = gtk::LevelBar::for_interval(0.0, top as f64);
    bar.set_value(secs as f64);
    bar.add_css_class("screen-time-share");
    text.append(&bar);
    row.append(&text);
    row
}
