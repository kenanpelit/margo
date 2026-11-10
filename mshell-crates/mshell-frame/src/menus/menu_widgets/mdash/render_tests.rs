//! Render fixture for mdash: the real dashboard component, built offline
//! (`live: false` — no service, no state file, no menu is touched) and fed
//! realistic fake data, rendered at the real panel width to PNGs, with the
//! proportions measured (equal size on every tab, level columns).
//!
//! Run (opt-in, under a private display + D-Bus; see README in mtune):
//! `MDASH_RENDER_DIR=… MDASH_RENDER_CSS=…/mshell-style.css \
//!  dbus-run-session -- xvfb-run -a cargo test -p mshell-frame --lib \
//!  mdash::render_tests -- --ignored --test-threads=1 --nocapture`

use super::dashboard::{DashboardInit, DashboardInput, DashboardModel};
use super::strip::StripState;
use super::system::{AiRow, UpdatesView};
use super::today::media::{Idle, MediaView, NowPlaying};
use crate::menus::menu_widgets::control_center::sliders::ControlCenterSlidersInput;
use crate::menus::menu_widgets::control_center::tiles::ControlCenterTilesInput;
use crate::menus::menu_widgets::control_center::tiles::fixture::TilesFixture;
use chrono::{NaiveDate, NaiveDateTime, TimeZone, Utc};
use mshell_config::schema::dashboard::DashboardConfig;
use relm4::gtk;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentController};
use std::time::Duration;

/// What one render shows.
#[derive(Debug, Clone)]
pub(crate) struct DashboardFixture {
    pub(crate) playing: bool,
    pub(crate) events: bool,
    pub(crate) battery: bool,
}

fn fixture_now() -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2026, 11, 10)
        .and_then(|d| d.and_hms_opt(20, 23, 0))
        .expect("valid")
}

fn event(summary: &str, h: u32, m: u32, mins: i64, location: Option<&str>) -> mcal::Event {
    let start = Utc
        .with_ymd_and_hms(2026, 11, 10, h, m, 0)
        .single()
        .expect("valid");
    mcal::Event {
        id: summary.into(),
        calendar_id: "fixture".into(),
        uid: summary.into(),
        summary: summary.into(),
        description: None,
        location: location.map(str::to_string),
        url: None,
        status: None,
        start,
        end: start + chrono::Duration::minutes(mins),
        all_day: false,
        recurrence: Vec::new(),
        attendees: Vec::new(),
        categories: Vec::new(),
    }
}

impl DashboardFixture {
    pub(crate) fn apply(&self, m: &mut DashboardModel) {
        let (today, controls, system, _strip, strip_state, frozen) = m.fixture_parts();
        *frozen = Some(fixture_now());
        *strip_state = StripState {
            battery: self.battery.then_some((76, false)),
            network: Some(("Home-5G".into(), "network-wireless-symbolic")),
            vpn: false,
            volume: Some((45, false)),
            weather: Some(("11°".into(), "weather-overcast-symbolic")),
            unread: 3,
            updates: 7,
        };
        if let Some(t) = today {
            t.calendar.select(fixture_now().date());
            t.calendar.set_events(if self.events {
                vec![
                    event("Stand-up", 7, 0, 15, Some("Room 2")),
                    event("Design review", 11, 30, 60, None),
                    event("Dentist", 15, 0, 45, Some("Kadıköy")),
                ]
            } else {
                Vec::new()
            });
            let weather: wayle_weather::Weather =
                serde_json::from_str(include_str!("testdata/weather.json")).expect("weather");
            let at = NaiveDate::from_ymd_opt(2026, 11, 10)
                .and_then(|d| d.and_hms_opt(14, 10, 0))
                .expect("valid");
            t.weather.apply(
                Some(&weather),
                &wayle_weather::TemperatureUnit::Metric,
                true,
                at,
            );
            t.media.apply(&if self.playing {
                MediaView::Playing(NowPlaying {
                    player: "Tune".into(),
                    title: "A quiet place to begin".into(),
                    artist: "Evening Sessions".into(),
                    art: None,
                    position: Duration::from_secs(87),
                    length: Some(Duration::from_secs(240)),
                    playing: true,
                    can_prev: true,
                    can_next: true,
                    players: 2,
                })
            } else {
                MediaView::Idle(Idle {
                    resume: Some("Late night jazz — Radio One".into()),
                    stations: vec![
                        ("one".into(), "Radio One".into()),
                        ("two".into(), "Night Signals".into()),
                    ],
                })
            });
        }
        if let Some(c) = controls {
            c.sliders.emit(ControlCenterSlidersInput::Fixture(
                0.45,
                0.6,
                self.battery.then_some(0.7),
            ));
            c.tiles
                .emit(ControlCenterTilesInput::Fixture(Box::new(TilesFixture {
                    battery: self.battery,
                    led: self.battery,
                })));
        }
        if let Some(s) = system {
            for i in 0..30 {
                let wave = (f64::from(i) / 3.0).sin();
                s.apply_cpu(22.0 + 14.0 * wave, Some(3200));
                s.apply_memory(9_800_000_000, 16_000_000_000, 61.0 + 3.0 * wave);
                s.apply_temp(Some(58.0 + 6.0 * wave));
                s.apply_net((1_200_000.0 * (1.2 + wave)) as u64, 85_000);
            }
            s.apply_disk("/", 182_000_000_000, 476_000_000_000);
            s.apply_battery(self.battery.then_some((76, "On battery")));
            s.apply_updates(Some(&UpdatesView {
                total: 7,
                detail: "5 Repo · 2 AUR".into(),
            }));
            s.apply_ai(&[
                AiRow {
                    label: "Claude · 5-hour".into(),
                    percent: 42.0,
                },
                AiRow {
                    label: "Claude · weekly".into(),
                    percent: 18.0,
                },
                AiRow {
                    label: "Codex · 5-hour".into(),
                    percent: 81.0,
                },
            ]);
        }
        m.fixture_refresh();
    }
}

fn pump() {
    let context = gtk::glib::MainContext::default();
    for _ in 0..60 {
        while context.pending() {
            context.iteration(false);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn screenshot(window: &gtk::Window, path: &std::path::Path) {
    let paintable = gtk::WidgetPaintable::new(Some(window));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, window.width() as f64, window.height() as f64);
    let node = snapshot.to_node().expect("rendered native widget tree");
    let surface = window.surface().expect("fixture surface");
    let renderer = gtk::gsk::Renderer::for_surface(&surface).expect("fixture renderer");
    let texture = renderer.render_texture(&node, None);
    texture.save_to_png(path).expect("save GTK screenshot");
    renderer.unrealize();
}

/// The widget's box in `ancestor`'s coordinates.
fn bounds(w: &impl IsA<gtk::Widget>, ancestor: &impl IsA<gtk::Widget>) -> gtk::graphene::Rect {
    w.compute_bounds(ancestor).expect("allocated")
}

/// Bottom edges (relative to `grid`) of the cards in each grid column.
fn column_bottoms(grid: &gtk::Widget) -> (f32, f32) {
    let width = grid.width() as f32;
    let (mut left, mut right) = (0f32, 0f32);
    let mut child = grid.first_child();
    while let Some(c) = child {
        if c.is_visible() {
            let b = bounds(&c, grid);
            let bottom = b.y() + b.height();
            if b.x() < width / 2.0 - 1.0 {
                left = left.max(bottom);
            }
            if b.x() + b.width() > width / 2.0 + 1.0 {
                right = right.max(bottom);
            }
        }
        child = c.next_sibling();
    }
    (left, right)
}

/// Labels cut by ellipsis. Titles and labels may never be cut (a
/// truncated tile / card / tool name = a layout bug); values and subtitles
/// may (a long SSID), and are listed for the report.
fn truncated_labels(w: &gtk::Widget, out: &mut Vec<(String, String)>) {
    if let Some(l) = w.downcast_ref::<gtk::Label>()
        && l.is_drawable()
        && l.ellipsize() != gtk::pango::EllipsizeMode::None
    {
        let layout = l.layout();
        if layout.is_ellipsized() {
            let class = l
                .css_classes()
                .first()
                .map(|c| c.to_string())
                .unwrap_or_default();
            out.push((class, l.label().to_string()));
        }
    }
    let mut child = w.first_child();
    while let Some(c) = child {
        truncated_labels(&c, out);
        child = c.next_sibling();
    }
}

const NEVER_CUT: &[&str] = &[
    "mdash-card-title",
    "control-center-tile-title",
    "mdash-tool-label",
    "mdash-greeting",
    "mdash-datetime",
    "mdash-section-caption",
];

#[test]
#[ignore = "requires an isolated display; writes GTK screenshots to MDASH_RENDER_DIR"]
fn native_mdash_render_fixture() {
    gtk::init().expect("isolated GTK display");
    let out = std::path::PathBuf::from(std::env::var("MDASH_RENDER_DIR").expect("output dir"));
    std::fs::create_dir_all(&out).expect("output dir");
    let display = gtk::gdk::Display::default().expect("display");
    let css = gtk::CssProvider::new();
    css.load_from_path(std::env::var("MDASH_RENDER_CSS").expect("compiled stylesheet"));
    gtk::style_context_add_provider_for_display(
        &display,
        &css,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    if let Ok(icons) = std::env::var("MDASH_RENDER_ICONS") {
        let theme = gtk::IconTheme::for_display(&display);
        theme.add_search_path(icons);
        theme.set_theme_name(Some("MargoMaterial"));
    }
    mshell_services::init_absent_services_for_fixtures();

    // The real panel: mdash's width (760) minus the menu's 20 px margins.
    const CONTENT_WIDTH: i32 = 720;
    let mut sizes = Vec::new();
    for (name, fx) in [
        (
            "laptop-playing",
            DashboardFixture {
                playing: true,
                events: true,
                battery: true,
            },
        ),
        (
            "desktop-idle",
            DashboardFixture {
                playing: false,
                events: false,
                battery: false,
            },
        ),
    ] {
        let controller = DashboardModel::builder()
            .launch(DashboardInit {
                config: DashboardConfig::default(),
                live: false,
            })
            .detach();
        controller.emit(DashboardInput::Fixture(Box::new(fx)));
        let frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
        frame.add_css_class("menu-scroll-window");
        frame.add_css_class("quick-settings-menu");
        frame.add_css_class("mdash-menu");
        frame.set_margin_top(20);
        frame.set_margin_bottom(20);
        frame.set_margin_start(20);
        frame.set_margin_end(20);
        frame.append(controller.widget());
        let window = gtk::Window::builder()
            .title("mdash fixture")
            .child(&frame)
            .build();
        window.add_css_class("frame-disabled");
        controller.widget().set_size_request(CONTENT_WIDTH, -1);
        window.present();
        pump();

        let stack = controller.model().stack().clone();
        for (i, tab) in ["today", "controls", "system", "tools"]
            .into_iter()
            .enumerate()
        {
            controller.emit(DashboardInput::Select(i));
            pump();
            let page = stack.visible_child().expect("page");
            let size = (
                stack.width(),
                stack.height(),
                window.width(),
                window.height(),
            );
            println!(
                "{name}/{tab}: stack {}x{} window {}x{}",
                size.0, size.1, size.2, size.3
            );
            sizes.push((name, tab, size));
            let path = out.join(format!("mdash-{name}-{tab}.png"));
            screenshot(&window, &path);
            println!("wrote {}", path.display());
            let mut cut = Vec::new();
            truncated_labels(controller.widget().upcast_ref(), &mut cut);
            if !cut.is_empty() {
                println!("{name}/{tab}: ellipsized {cut:?}");
            }
            for (class, text) in &cut {
                assert!(
                    !NEVER_CUT.contains(&class.as_str()),
                    "{name}/{tab}: {class} {text:?} is cut"
                );
            }
            // Level columns on the two-column tabs.
            if tab == "today" || tab == "system" {
                let grid = page
                    .first_child()
                    .and_then(|v| v.first_child())
                    .expect("tab grid");
                let (l, r) = column_bottoms(&grid);
                println!("{name}/{tab}: column bottoms {l} / {r}");
                assert!((l - r).abs() <= 2.0, "{name}/{tab}: columns end {l} vs {r}");
            }
        }
        for w in sizes.iter().filter(|s| s.0 == name) {
            assert_eq!(
                w.2,
                sizes.iter().find(|s| s.0 == name).expect("first").2,
                "{name}: tab {} resized the panel",
                w.1
            );
        }
        window.close();
        pump();
    }
}
