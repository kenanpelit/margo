//! Render fixture for mdash: the real dashboard component, built offline
//! (`live: false` — no service, no state file, no menu is touched) and fed
//! realistic fake data, rendered at the real panel width to PNGs, with the
//! proportions measured: the width fits the menu, each tab gets its
//! natural height, Today's columns end level, tiles and tools are equal,
//! and no card is stretched past its content.
//!
//! Run (opt-in, under a private display + D-Bus; see README in mtune):
//! `MDASH_RENDER_DIR=… MDASH_RENDER_CSS=…/mshell-style.css \
//!  dbus-run-session -- xvfb-run -a cargo test -p mshell-frame --lib \
//!  mdash::render_tests -- --ignored --test-threads=1 --nocapture`

use super::dashboard::{DashboardInit, DashboardInput, DashboardModel};
use super::strip::StripState;
use super::system::{AiRow, UpdatesView};
use super::today::media::{Idle, MediaView, NowPlaying};
use super::today::timers::alarm_line;
use crate::menus::menu_widgets::control_center::sliders::ControlCenterSlidersInput;
use crate::menus::menu_widgets::control_center::tiles::ControlCenterTilesInput;
use crate::menus::menu_widgets::control_center::tiles::fixture::TilesFixture;
use crate::timers::{Timer, TimerKind};
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
            s.apply_battery(self.battery.then(|| {
                (
                    76,
                    "On battery",
                    super::system::battery_time_line(false, 11_520, 0),
                )
            }));
            s.apply_updates(Some(&UpdatesView {
                total: 7,
                detail: "5 Repo · 2 AUR".into(),
                download: Some(182_000_000),
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
        // A busy laptop day: an alarm tomorrow and two running timers.
        if self.events {
            let (today, ..) = m.fixture_parts();
            if let Some(t) = today {
                let now = fixture_now();
                let unix = now.and_utc().timestamp();
                let tomorrow = (now.date() + chrono::Duration::days(1))
                    .and_hms_opt(7, 0, 0)
                    .expect("valid");
                let timer = |id, left: i64, length: i64, label: &str| Timer {
                    id,
                    due: unix + left,
                    created: unix + left - length,
                    kind: TimerKind::Timer,
                    label: label.into(),
                };
                t.timers.apply(
                    &[timer(1, 272, 600, "Tea"), timer(2, 1500, 1800, "Pomodoro")],
                    unix,
                    &alarm_line(Some(&(tomorrow, "Work".into())), now),
                );
            }
        }
    }
}

/// reactive_graph effects (config watches) spawn through any_spawner; run
/// them on the GTK main context, as the shell does (mshell-core).
struct GlibExecutor;

impl any_spawner::CustomExecutor for GlibExecutor {
    fn spawn(&self, fut: any_spawner::PinnedFuture<()>) {
        gtk::glib::MainContext::default().spawn(fut);
    }

    fn spawn_local(&self, fut: any_spawner::PinnedLocalFuture<()>) {
        gtk::glib::MainContext::default().spawn_local(fut);
    }

    fn poll_local(&self) {}
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

/// Bottom edges (relative to `page`) of the lowest card in the left and
/// in the right half of the tab.
fn column_bottoms(page: &gtk::Widget) -> (f32, f32) {
    fn walk(w: &gtk::Widget, page: &gtk::Widget, half: f32, out: &mut (f32, f32)) {
        if w.has_css_class("mdash-card") && w.is_drawable() {
            let b = bounds(w, page);
            let bottom = b.y() + b.height();
            if b.x() < half - 1.0 {
                out.0 = out.0.max(bottom);
            }
            if b.x() + b.width() > half + 1.0 {
                out.1 = out.1.max(bottom);
            }
            return;
        }
        let mut child = w.first_child();
        while let Some(c) = child {
            walk(&c, page, half, out);
            child = c.next_sibling();
        }
    }
    let mut out = (0f32, 0f32);
    walk(page, page, page.width() as f32 / 2.0, &mut out);
    out
}

/// Debug aid when the dashboard is too wide: the widget tree with each
/// node's minimum width (only the nodes that need more than a column).
fn wide_report(w: &gtk::Widget, depth: usize) {
    let (min, nat, _, _) = w.measure(gtk::Orientation::Horizontal, -1);
    if min > 300 || depth < 2 {
        println!(
            "{:indent$}{} {:?} min {min} nat {nat}",
            "",
            w.type_().name(),
            w.css_classes(),
            indent = depth * 2
        );
        let mut child = w.first_child();
        while let Some(c) = child {
            wide_report(&c, depth + 1);
            child = c.next_sibling();
        }
    }
}

/// Every drawable widget carrying `class` under `root`.
fn with_class(root: &gtk::Widget, class: &str, out: &mut Vec<gtk::Widget>) {
    if root.has_css_class(class) && root.is_drawable() {
        out.push(root.clone());
        return;
    }
    let mut child = root.first_child();
    while let Some(c) = child {
        with_class(&c, class, out);
        child = c.next_sibling();
    }
}

/// No card is taller than its natural height at its width (a stretched
/// card = an empty interior). The agenda is the one flexible card.
fn assert_no_stretched_cards(name: &str, tab: &str, page: &gtk::Widget) {
    let mut cards = Vec::new();
    for class in ["mdash-card", "mdash-tile-frame", "mdash-tool"] {
        with_class(page, class, &mut cards);
    }
    for card in cards {
        if card.has_css_class("mdash-agenda-card") {
            continue;
        }
        // Border-box sizes (`width()` / `height()` leave out the padding).
        let b = bounds(&card, &card);
        let (_, natural, _, _) = card.measure(gtk::Orientation::Vertical, b.width() as i32);
        let height = b.height() as i32;
        if std::env::var("MDASH_RENDER_VERBOSE").is_ok_and(|v| !v.is_empty()) {
            println!(
                "{name}/{tab}: {:?} {height} / natural {natural}",
                card.css_classes()
            );
        }
        assert!(
            height - natural <= 4,
            "{name}/{tab}: {:?} is {height} px tall, natural {natural}",
            card.css_classes()
        );
    }
}

/// All widgets with `class` share one width (±1 px).
fn assert_equal_widths(name: &str, tab: &str, page: &gtk::Widget, class: &str) -> usize {
    let mut items = Vec::new();
    with_class(page, class, &mut items);
    let widths: Vec<i32> = items.iter().map(|w| w.width()).collect();
    let (min, max) = (
        widths.iter().min().copied().unwrap_or(0),
        widths.iter().max().copied().unwrap_or(0),
    );
    assert!(
        max - min <= 1,
        "{name}/{tab}: {class} widths differ: {widths:?}"
    );
    println!("{name}/{tab}: {} × {class} at {max} px", widths.len());
    widths.len()
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
    let _ = any_spawner::Executor::init_custom_executor(GlibExecutor);
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
    // The default (frame-drawing) shell paints menus with `--frame-bg`
    // behind them; the fixture paints the same so cards read as they do
    // on screen.
    let frame_css = gtk::CssProvider::new();
    frame_css.load_from_string(
        ".mdash-fixture-frame { background-color: var(--frame-bg); border-radius: 16px; padding: 20px; }",
    );
    gtk::style_context_add_provider_for_display(
        &display,
        &frame_css,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
    );
    if let Ok(icons) = std::env::var("MDASH_RENDER_ICONS") {
        // The scratch HOME has no icon theme configured: point the
        // display's theme at the repo's MargoMaterial set.
        gtk::Settings::for_display(&display).set_gtk_icon_theme_name(Some("MargoMaterial"));
        gtk::IconTheme::for_display(&display).add_search_path(icons);
    }
    mshell_services::init_absent_services_for_fixtures();

    // The real panel: mdash's default width (800) minus the menu's 20 px
    // margins on each side.
    const CONTENT_WIDTH: i32 = 760;
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
        frame.add_css_class("mdash-fixture-frame");
        frame.add_css_class("quick-settings-menu");
        frame.add_css_class("mdash-menu");
        frame.append(controller.widget());
        let window = gtk::Window::builder()
            .title("mdash fixture")
            .child(&frame)
            // Non-resizable = always at its natural size, like the menu
            // surface that follows the active tab's height.
            .resizable(false)
            .build();
        controller.widget().set_size_request(CONTENT_WIDTH, -1);
        window.present();
        pump();

        let stack = controller.model().stack().clone();
        let dash: gtk::Widget = controller.widget().clone().upcast();
        for (i, tab) in ["today", "controls", "system", "tools"]
            .into_iter()
            .enumerate()
        {
            controller.emit(DashboardInput::Select(i));
            // Past the crossfade + height interpolation.
            pump();
            pump();
            let page = stack.visible_child().expect("page");
            let (min_w, _, _, _) = dash.measure(gtk::Orientation::Horizontal, -1);
            let (_, nat_h, _, _) = page.measure(gtk::Orientation::Vertical, page.width());
            println!(
                "{name}/{tab}: dashboard {}x{} (min width {min_w}), page {}x{} (natural height {nat_h})",
                dash.width(),
                dash.height(),
                page.width(),
                page.height()
            );
            if min_w > CONTENT_WIDTH {
                wide_report(&dash, 0);
            }
            // Fits the menu's pinned width; never wider.
            assert!(min_w <= CONTENT_WIDTH, "{name}/{tab}: needs {min_w} px");
            assert_eq!(dash.width(), CONTENT_WIDTH, "{name}/{tab}: width");
            // The panel's height follows the tab: the page gets exactly
            // its natural height.
            assert!(
                (page.height() - nat_h).abs() <= 1,
                "{name}/{tab}: page {} vs natural {nat_h}",
                page.height()
            );
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
            assert_no_stretched_cards(name, tab, &page);
            match tab {
                "today" => {
                    let (l, r) = column_bottoms(&page);
                    println!("{name}/{tab}: column bottoms {l} / {r}");
                    assert!((l - r).abs() <= 2.0, "{name}/{tab}: columns end {l} vs {r}");
                }
                "controls" => {
                    assert!(assert_equal_widths(name, tab, &page, "mdash-tile-frame") > 8);
                    // The slider tracks end in one column.
                    assert!(assert_equal_widths(name, tab, &page, "control-center-slider") >= 2);
                }
                "tools" => {
                    assert!(assert_equal_widths(name, tab, &page, "mdash-tool") >= 24);
                }
                _ => {}
            }
        }
        window.close();
        pump();
    }
}
