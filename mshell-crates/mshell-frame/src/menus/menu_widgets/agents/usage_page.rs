//! The Agents menu's Usage page: an overview card over every plan window
//! (the one closest to its limit in a ring, then load / peak / attention /
//! at-risk / next-reset tiles), one card per provider (its mark in a ring
//! of its fullest window, the headline percent, and each window as a bar
//! with an even-pace marker), then today's Claude Code activity, token and
//! cost tiles, a seven-day chart with today picked out, and the by-project
//! and by-hour views. Visual ideas from AiOverviewControl (a Dux Shell
//! plugin); the data is mshell's own (`crate::agents`).

use crate::agents::limits::{self, LimitWindow, PaceStatus, Provider, WindowKind};
use crate::agents::local_stats::compact_count;
use crate::agents::{
    HISTORY_DAYS, HistorySnapshot, ProviderLimits, UsageSnapshot, history, pricing, sessions,
};
use relm4::gtk;
use relm4::gtk::prelude::*;

// ── Overview ───────────────────────────────────────────────────────────

/// One plan window of one provider (or Claude account).
struct Entry<'a> {
    provider: Provider,
    plan: &'a str,
    window: &'a LimitWindow,
    pace: limits::Pace,
}

/// Every plan's numbers in one place, for the overview card.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Overview {
    /// Mean used share over every session / weekly / model window.
    pub(crate) avg_percent: f64,
    /// The fullest window: (provider, plan, window label, percent, severity class).
    pub(crate) peak: Option<(Provider, String, String, f64, &'static str)>,
    /// Providers (accounts) with a reading.
    pub(crate) active: usize,
    /// Windows heading for 85 %+ or the limit before the reset.
    pub(crate) attention: usize,
    /// Windows at the limit or about to hit it.
    pub(crate) at_risk: usize,
    /// The soonest reset of a window that has something used: ms from now.
    pub(crate) next_reset_ms: Option<i64>,
    /// Whether any provider's latest refresh failed (its numbers are old).
    pub(crate) stale: bool,
}

fn providers(s: &UsageSnapshot) -> impl Iterator<Item = &ProviderLimits> {
    std::iter::once(&s.claude)
        .chain(s.others.iter().map(|a| &a.limits))
        .chain(s.codex.as_ref())
}

fn entries(s: &UsageSnapshot) -> Vec<Entry<'_>> {
    providers(s)
        .filter_map(|p| p.usage.as_ref().map(|u| (p, u)))
        .flat_map(|(p, u)| {
            u.windows
                .iter()
                .filter(|w| w.kind != WindowKind::Other)
                .map(move |w| Entry {
                    provider: p.provider,
                    plan: &u.plan,
                    window: w,
                    pace: limits::pace(
                        w,
                        s.now_ms,
                        (std::ptr::eq(p, &s.claude) && w.kind == WindowKind::Session)
                            .then_some(s.session_rate)
                            .flatten(),
                    ),
                })
        })
        .collect()
}

pub(crate) fn overview(s: &UsageSnapshot) -> Overview {
    let all = entries(s);
    let n = all.len().max(1) as f64;
    let peak = all
        .iter()
        .max_by(|a, b| a.window.percent.total_cmp(&b.window.percent))
        .map(|e| {
            (
                e.provider,
                e.plan.to_string(),
                e.window.label.clone(),
                e.window.percent,
                e.pace.status.severity_class(),
            )
        });
    Overview {
        avg_percent: all.iter().map(|e| e.window.percent).sum::<f64>() / n,
        peak,
        active: providers(s).filter(|p| p.usage.is_some()).count(),
        attention: all
            .iter()
            .filter(|e| e.pace.status.severity_class() == "warn")
            .count(),
        at_risk: all
            .iter()
            .filter(|e| e.pace.status.severity_class() == "danger")
            .count(),
        next_reset_ms: all
            .iter()
            .filter(|e| e.window.percent > 0.0)
            .filter_map(|e| e.pace.remaining_ms)
            .min(),
        stale: providers(s).any(|p| p.error.is_some() && p.usage.is_some()),
    }
}

/// The severity class of a used share alone (no pace): the ring and the
/// headline percent of a provider.
fn level_class(percent: f64) -> &'static str {
    if percent >= 90.0 {
        "danger"
    } else if percent >= 70.0 {
        "warn"
    } else {
        "calm"
    }
}

/// The provider's mark from the shell icon theme, the generic agents icon
/// when the installed theme doesn't have it yet.
fn provider_icon(p: Provider) -> &'static str {
    let name = match p {
        Provider::Claude => "claude-ai-symbolic",
        Provider::Codex => "openai-codex-symbolic",
    };
    let has = gtk::gdk::Display::default()
        .map(|d| gtk::IconTheme::for_display(&d).has_icon(name))
        .unwrap_or(false);
    if has { name } else { "ai-agents-symbolic" }
}

// ── Drawn pieces ───────────────────────────────────────────────────────

/// A ring gauge `size` px wide, filled to `fraction` in the widget's CSS
/// colour (`.agents-ring.warn` / `.danger` tint it), with `child` in the
/// middle.
fn ring(fraction: f64, class: &str, size: i32, child: Option<&gtk::Widget>) -> gtk::Overlay {
    let fraction = fraction.clamp(0.0, 1.0);
    let area = gtk::DrawingArea::new();
    area.set_content_width(size);
    area.set_content_height(size);
    area.set_css_classes(&["agents-ring", class]);
    area.set_draw_func(move |a, cr, w, h| {
        let c = a.color();
        let (r, g, b) = (c.red() as f64, c.green() as f64, c.blue() as f64);
        let line = (w.min(h) as f64 * 0.09).max(3.0);
        let (cx, cy) = (w as f64 / 2.0, h as f64 / 2.0);
        let radius = w.min(h) as f64 / 2.0 - line / 2.0 - 0.5;
        let top = -std::f64::consts::FRAC_PI_2;
        cr.set_line_width(line);
        cr.set_line_cap(gtk::cairo::LineCap::Round);
        cr.set_source_rgba(r, g, b, 0.18);
        cr.arc(cx, cy, radius, 0.0, std::f64::consts::TAU);
        let _ = cr.stroke();
        if fraction > 0.0 {
            cr.set_source_rgba(r, g, b, 1.0);
            cr.arc(cx, cy, radius, top, top + fraction * std::f64::consts::TAU);
            let _ = cr.stroke();
        }
    });
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&area));
    overlay.set_halign(gtk::Align::Center);
    overlay.set_valign(gtk::Align::Center);
    if let Some(child) = child {
        child.set_halign(gtk::Align::Center);
        child.set_valign(gtk::Align::Center);
        overlay.add_overlay(child);
    }
    overlay
}

/// A window's bar: the used share in the severity colour over a faint
/// track, and a marker where an even pace would be now (the share of the
/// window gone by): a fill past it burns faster than the window lasts.
fn pace_bar(fraction: f64, marker: Option<f64>, class: &str) -> gtk::DrawingArea {
    let fraction = fraction.clamp(0.0, 1.0);
    let area = gtk::DrawingArea::new();
    area.set_content_height(10);
    area.set_hexpand(true);
    area.set_css_classes(&["agents-pace-bar", class]);
    area.set_draw_func(move |a, cr, w, h| {
        let c = a.color();
        let (r, g, b) = (c.red() as f64, c.green() as f64, c.blue() as f64);
        let (w, h) = (w as f64, h as f64);
        let bar = 6.0_f64.min(h);
        let y = (h - bar) / 2.0;
        let rounded = |x: f64, width: f64| {
            let rad = bar / 2.0;
            let width = width.max(bar);
            cr.new_sub_path();
            cr.arc(
                x + width - rad,
                y + rad,
                rad,
                -std::f64::consts::FRAC_PI_2,
                std::f64::consts::FRAC_PI_2,
            );
            cr.arc(
                x + rad,
                y + rad,
                rad,
                std::f64::consts::FRAC_PI_2,
                3.0 * std::f64::consts::FRAC_PI_2,
            );
            cr.close_path();
        };
        cr.set_source_rgba(r, g, b, 0.16);
        rounded(0.0, w);
        let _ = cr.fill();
        if fraction > 0.0 {
            cr.set_source_rgba(r, g, b, 1.0);
            rounded(0.0, w * fraction);
            let _ = cr.fill();
        }
        if let Some(m) = marker {
            let x = (w * m.clamp(0.0, 1.0)).clamp(1.0, w - 1.0);
            cr.set_source_rgba(r, g, b, 0.95);
            cr.set_line_width(2.0);
            cr.move_to(x, 0.0);
            cr.line_to(x, h);
            let _ = cr.stroke();
        }
    });
    area
}

// ── Small builders ─────────────────────────────────────────────────────

pub(crate) fn clear(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

pub(crate) fn label(text: &str, classes: &[&str]) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_css_classes(classes);
    l.set_halign(gtk::Align::Start);
    l.set_xalign(0.0);
    l.set_wrap(true);
    l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    l
}

fn section(title: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(title));
    l.add_css_class("section-label-menu-widget");
    l.set_halign(gtk::Align::Start);
    l.set_xalign(0.0);
    l
}

fn boxed_list() -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
    b.set_css_classes(&["ok-boxed-list", "compact"]);
    b
}

fn card(classes: &[&str]) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let mut all = vec!["agents-card"];
    all.extend_from_slice(classes);
    b.set_css_classes(&all);
    b
}

fn chip(text: &str, class: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_css_classes(&["agents-chip", class]);
    l
}

fn stat_row(title: &str, value: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let t = label(title, &["label-medium"]);
    t.set_hexpand(true);
    t.set_valign(gtk::Align::Center);
    row.append(&t);
    let v = gtk::Label::new(Some(value));
    v.set_css_classes(&["label-medium", "dim-label", "agents-stat-value"]);
    v.set_valign(gtk::Align::Center);
    row.append(&v);
    row
}

/// An overview tile: a tinted icon square, the value, a caption.
fn tile(icon: &str, value: &str, caption: &str, class: &str) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    b.add_css_class("agents-tile");
    b.set_hexpand(true);
    let i = gtk::Image::from_icon_name(icon);
    i.set_css_classes(&["agents-tile-icon", class]);
    i.set_valign(gtk::Align::Center);
    b.append(&i);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let v = label(value, &["agents-tile-value", class]);
    v.set_wrap(false);
    v.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text.append(&v);
    let c = label(caption, &["label-small", "dim-label"]);
    c.set_wrap(false);
    c.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text.append(&c);
    b.append(&text);
    b
}

// ── The page ───────────────────────────────────────────────────────────

/// Rebuild the Usage page from `s` (`None` = still loading) and the
/// usage history `h` (once read).
pub(crate) fn rebuild_usage(
    container: &gtk::Box,
    s: Option<&UsageSnapshot>,
    h: Option<&HistorySnapshot>,
) {
    clear(container);
    let Some(s) = s else {
        container.append(&label("Loading…", &["label-small", "dim-label"]));
        return;
    };
    overview_card(container, s);
    provider_card(container, &s.claude, s.claude_label.as_deref(), s);
    for a in &s.others {
        provider_card(container, &a.limits, Some(&a.label), s);
    }
    if let Some(codex) = &s.codex {
        provider_card(container, codex, None, s);
    }
    today_card(container, s);
    if let Some(h) = h {
        history_cards(container, h);
    }
}

fn overview_card(container: &gtk::Box, s: &UsageSnapshot) {
    let o = overview(s);
    let Some((provider, plan, window, percent, _)) = o.peak.clone() else {
        return;
    };
    let level = level_class(percent);
    let hero = card(&["agents-hero", level]);

    let top = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let mark = gtk::Image::from_icon_name(provider_icon(provider));
    mark.add_css_class("agents-hero-mark");
    top.append(&ring(percent / 100.0, level, 64, Some(mark.upcast_ref())));
    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_valign(gtk::Align::Center);
    text.set_hexpand(true);
    let status = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let lamp = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    lamp.set_css_classes(&["agents-lamp", if o.stale { "waiting" } else { "busy" }]);
    lamp.set_valign(gtk::Align::Center);
    status.append(&lamp);
    let status_text = label(
        if o.stale {
            "SOME READINGS ARE OLD"
        } else {
            "LIMITS LIVE"
        },
        &["agents-hero-status"],
    );
    status_text.set_wrap(false);
    status.append(&status_text);
    text.append(&status);
    text.append(&label(&plan, &["agents-hero-title"]));
    let reset = entries(s)
        .into_iter()
        .find(|e| e.window.label == window && e.plan == plan)
        .and_then(|e| e.pace.remaining_ms)
        .map(|ms| format!("{window} is the fullest — resets in {}", limits::span(ms)))
        .unwrap_or_else(|| format!("{window} is the fullest"));
    text.append(&label(&reset, &["label-small", "dim-label"]));
    top.append(&text);
    let big = gtk::Label::new(Some(&format!("{percent:.0}%")));
    big.set_css_classes(&["agents-hero-pct", level]);
    big.set_valign(gtk::Align::Center);
    top.append(&big);
    hero.append(&top);

    let grid = gtk::Grid::new();
    grid.add_css_class("agents-tiles");
    grid.set_column_homogeneous(true);
    grid.set_row_spacing(6);
    grid.set_column_spacing(6);
    let peak_caption = format!("Peak · {}", provider.name());
    let tiles = [
        tile(
            "power-profile-balanced-symbolic",
            &format!("{:.0}%", o.avg_percent),
            "Avg load",
            level_class(o.avg_percent),
        ),
        tile(
            "temperature-symbolic",
            &format!("{percent:.0}%"),
            &peak_caption,
            level,
        ),
        tile(
            "check-circle-symbolic",
            &o.active.to_string(),
            "Active",
            "calm",
        ),
        tile(
            "notification-alert-symbolic",
            &o.attention.to_string(),
            "Attention",
            if o.attention > 0 { "warn" } else { "calm" },
        ),
        tile(
            "firewall-error-symbolic",
            &o.at_risk.to_string(),
            "At risk",
            if o.at_risk > 0 { "danger" } else { "calm" },
        ),
        tile(
            "timer-symbolic",
            &o.next_reset_ms.map_or_else(|| "—".into(), limits::span),
            "Next reset",
            "calm",
        ),
    ];
    for (i, t) in tiles.iter().enumerate() {
        grid.attach(t, (i % 3) as i32, (i / 3) as i32, 1, 1);
    }
    hero.append(&grid);
    container.append(&hero);
}

/// One provider's (or Claude account's) plan windows. `account` names the
/// account when there are several.
fn provider_card(
    container: &gtk::Box,
    p: &ProviderLimits,
    account: Option<&str>,
    s: &UsageSnapshot,
) {
    let primary = std::ptr::eq(p, &s.claude);
    let peak = p
        .usage
        .as_ref()
        .and_then(|u| {
            u.windows
                .iter()
                .filter(|w| w.kind != WindowKind::Other)
                .map(|w| w.percent)
                .max_by(f64::total_cmp)
        })
        .unwrap_or(0.0);
    let level = level_class(peak);
    let c = card(&["agents-provider-card", level]);
    if p.error.is_some() {
        c.add_css_class("stale");
    }

    let head = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let mark = gtk::Image::from_icon_name(provider_icon(p.provider));
    mark.add_css_class("agents-provider-mark");
    head.append(&ring(peak / 100.0, level, 40, Some(mark.upcast_ref())));
    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    text.append(&label(p.provider.name(), &["label-medium-bold"]));
    let mut sub = p
        .usage
        .as_ref()
        .map_or_else(|| "Loading…".to_string(), |u| u.plan.clone());
    if let Some(a) = account {
        sub.push_str(&format!(" · {a}"));
    }
    text.append(&label(&sub, &["label-small", "dim-label"]));
    let chips = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    match (&p.error, p.fetched_at) {
        (Some(_), Some(at)) => {
            chips.append(&chip(
                &format!("Stale · {}", limits::span(s.now_ms - at)),
                "warn",
            ));
        }
        (Some(_), None) => chips.append(&chip("Unavailable", "danger")),
        (None, Some(_)) => chips.append(&chip("Live", "accent")),
        (None, None) => {}
    }
    if primary && s.claude_label.is_some() {
        chips.append(&chip("Primary", ""));
    }
    if chips.first_child().is_some() {
        text.append(&chips);
    }
    head.append(&text);
    if p.usage.is_some() {
        let pct = gtk::Label::new(Some(&format!("{peak:.0}%")));
        pct.set_css_classes(&["agents-provider-pct", level]);
        pct.set_valign(gtk::Align::Center);
        head.append(&pct);
    }
    let open = gtk::Button::from_icon_name("open-in-new-symbolic");
    open.set_css_classes(&["ok-button-flat", "agents-provider-open"]);
    open.set_tooltip_text(Some("Open the usage page"));
    mshell_common::a11y::name(&open, &format!("Open the {} usage page", p.provider.name()));
    open.set_valign(gtk::Align::Center);
    let url = p.provider.dashboard_url();
    open.connect_clicked(move |_| super::agents_menu_widget::open_url(url));
    head.append(&open);
    c.append(&head);

    if let Some(u) = &p.usage {
        for w in &u.windows {
            // The recent burn rate is sampled for the primary account.
            let rate = (primary && w.kind == WindowKind::Session)
                .then_some(s.session_rate)
                .flatten();
            c.append(&window_row(w, s.now_ms, rate));
        }
    }
    if let Some(e) = &p.error {
        c.append(&label(&e.message(p.provider), &["label-small-error"]));
    }
    container.append(&c);
}

fn window_row(w: &LimitWindow, now_ms: i64, rate: Option<f64>) -> gtk::Box {
    let pace = limits::pace(w, now_ms, rate);
    let severity = pace.status.severity_class();
    let row = gtk::Box::new(gtk::Orientation::Vertical, 3);
    row.add_css_class("agents-window-row");

    let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let name = label(&w.label, &["label-medium-bold"]);
    name.set_hexpand(true);
    top.append(&name);
    let pct = gtk::Label::new(Some(&limits::headline_text(w, &pace)));
    pct.set_css_classes(&["agents-window-pct", severity]);
    top.append(&pct);
    row.append(&top);

    // The share of the window gone by: where an even pace would be now.
    let marker = pace
        .remaining_ms
        .filter(|_| w.period_ms > 0 && pace.status != PaceStatus::Idle)
        .map(|rem| 1.0 - rem as f64 / w.period_ms as f64);
    let bar = pace_bar(w.percent / 100.0, marker, severity);
    bar.set_tooltip_text(Some("The mark is where an even pace would be now"));
    row.append(&bar);

    row.append(&label(
        &limits::detail_text(w, &pace),
        &["label-small", "dim-label"],
    ));
    row
}

fn today_card(container: &gtk::Box, s: &UsageSnapshot) {
    container.append(&section("Today · Claude Code"));
    let list = boxed_list();
    let l = &s.local;
    list.append(&stat_row(
        "Prompts",
        &format!(
            "{} in {} session{}",
            l.prompts,
            l.sessions,
            if l.sessions == 1 { "" } else { "s" }
        ),
    ));
    list.append(&stat_row(
        "Tokens",
        &format!(
            "{} · {} output",
            compact_count(l.tokens.total()),
            compact_count(l.tokens.output)
        ),
    ));
    for (model, n) in l.by_model.iter().take(4) {
        list.append(&stat_row(model, &compact_count(*n)));
    }
    container.append(&list);
}

/// Most projects listed under "where it goes".
const TOP_PROJECTS: usize = 5;

/// Tokens and estimated cost: Today / Week / Month tiles, the last seven
/// days, all time, then the last 30 days by project and by hour.
fn history_cards(container: &gtk::Box, h: &HistorySnapshot) {
    let prices = pricing::current();
    let currency = crate::agents::settings::currency();
    let cost = |t: &history::Totals| {
        t.cost(&prices)
            .and_then(|c| pricing::money(c, currency, prices.eur))
    };
    let value = |t: &history::Totals| {
        let mut v = compact_count(t.tokens());
        if let Some(c) = cost(t) {
            v.push_str(&format!(" · ~{c}"));
        }
        v
    };

    let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let title = section("Tokens & estimated cost · Claude Code");
    title.set_hexpand(true);
    head.append(&title);
    if let Some(c) = cost(&h.month) {
        let m = gtk::Label::new(Some(&format!("~{c} this month")));
        m.set_css_classes(&["label-small", "agents-month-cost"]);
        head.append(&m);
    }
    container.append(&head);

    let tiles = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    tiles.set_homogeneous(true);
    for (title, t) in [("Today", &h.today), ("Week", &h.week), ("Month", &h.month)] {
        let b = gtk::Box::new(gtk::Orientation::Vertical, 1);
        b.add_css_class("agents-token-tile");
        b.append(&label(title, &["label-small", "dim-label"]));
        let tokens = label(&compact_count(t.tokens()), &["agents-tile-value"]);
        tokens.set_wrap(false);
        b.append(&tokens);
        let money = label(
            &cost(t).map_or_else(|| "—".into(), |c| format!("~{c}")),
            &["label-small", "dim-label"],
        );
        money.set_wrap(false);
        b.append(&money);
        tiles.append(&b);
    }
    container.append(&tiles);

    if !h.last_days.is_empty() {
        let chart_card = card(&["agents-chart-card"]);
        let values: Vec<u64> = h.last_days.iter().map(|(_, t)| t.tokens()).collect();
        let labels: Vec<String> = h
            .last_days
            .iter()
            .map(|(d, _)| d.format("%a").to_string())
            .collect();
        let tips: Vec<String> = h
            .last_days
            .iter()
            .map(|(d, t)| format!("{} · {}", d.format("%a %d %b"), value(t)))
            .collect();
        chart_card.append(&bar_chart(
            &values,
            &labels,
            Some(values.len() - 1),
            Some(&tips),
        ));
        container.append(&chart_card);
    }

    let list = boxed_list();
    list.append(&stat_row("All time", &value(&h.all)));
    list.append(&stat_row(
        "All-time activity",
        &format!(
            "{} sessions · {} messages",
            h.all_sessions,
            compact_count(u64::from(h.all.messages))
        ),
    ));
    container.append(&list);

    if !h.projects.is_empty() {
        container.append(&section(&format!("Last {HISTORY_DAYS} days · by project")));
        let list = boxed_list();
        let max = h.projects.first().map_or(1, |(_, t)| t.tokens().max(1)) as f64;
        for (project, t) in h.projects.iter().take(TOP_PROJECTS) {
            let row = gtk::Box::new(gtk::Orientation::Vertical, 3);
            row.append(&stat_row(&sessions::tilde(project), &value(t)));
            row.append(&pace_bar(t.tokens() as f64 / max, None, "calm"));
            list.append(&row);
        }
        container.append(&list);
    }
    container.append(&section(&format!("Last {HISTORY_DAYS} days · by hour")));
    let chart = card(&["agents-chart-card"]);
    chart.append(&bar_chart(&h.hours, &hour_labels(), None, None));
    container.append(&chart);
}

/// `0`, `6`, `12`, `18` under their hours, blank elsewhere.
fn hour_labels() -> Vec<String> {
    (0..24)
        .map(|h| {
            if h % 6 == 0 {
                h.to_string()
            } else {
                String::new()
            }
        })
        .collect()
}

/// One bar per value, scaled to the largest, with a label under each and
/// the value (or `tips`) in its tooltip; `highlight` picks one out (today).
pub(crate) fn bar_chart(
    values: &[u64],
    labels: &[String],
    highlight: Option<usize>,
    tips: Option<&[String]>,
) -> gtk::Box {
    let max = values.iter().copied().max().unwrap_or(0).max(1) as f64;
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 3);
    row.add_css_class("agents-chart");
    row.set_homogeneous(true);
    for (i, v) in values.iter().enumerate() {
        let col = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let bar = gtk::ProgressBar::new();
        bar.set_orientation(gtk::Orientation::Vertical);
        bar.set_inverted(true);
        bar.set_css_classes(&["agents-chart-bar"]);
        let picked = highlight == Some(i);
        if picked {
            bar.add_css_class("today");
        }
        bar.set_fraction(*v as f64 / max);
        bar.set_halign(gtk::Align::Center);
        let tip = tips
            .and_then(|t| t.get(i).cloned())
            .unwrap_or_else(|| compact_count(*v));
        bar.set_tooltip_text(Some(&tip));
        col.append(&bar);
        let l = gtk::Label::new(Some(labels.get(i).map_or("", String::as_str)));
        l.set_css_classes(&["label-small", "dim-label", "agents-chart-label"]);
        if picked {
            l.add_css_class("today");
        }
        col.append(&l);
        row.append(&col);
    }
    row
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::limits::{PlanUsage, Severity};

    const H: i64 = 3_600_000;

    fn window(
        kind: WindowKind,
        label: &str,
        percent: f64,
        resets_in: i64,
        period: i64,
    ) -> LimitWindow {
        LimitWindow {
            kind,
            label: label.into(),
            percent,
            resets_at: Some(1_000 * H + resets_in),
            period_ms: period,
            severity: Severity::Normal,
        }
    }

    fn limits(provider: Provider, plan: &str, windows: Vec<LimitWindow>) -> ProviderLimits {
        ProviderLimits {
            provider,
            usage: Some(PlanUsage {
                provider,
                plan: plan.into(),
                windows,
            }),
            error: None,
            fetched_at: Some(1_000 * H),
        }
    }

    #[test]
    fn overview_sums_every_window_and_picks_the_fullest() {
        let mut s = UsageSnapshot {
            claude: limits(
                Provider::Claude,
                "Claude Max 5x",
                vec![
                    window(WindowKind::Session, "5-hour", 10.0, 4 * H, 5 * H),
                    window(WindowKind::Weekly, "Weekly", 50.0, 60 * H, 168 * H),
                ],
            ),
            claude_label: None,
            others: Vec::new(),
            codex: Some(limits(
                Provider::Codex,
                "Codex Plus",
                vec![
                    // 96 % with 3 h of the 5 left: at risk
                    window(WindowKind::Session, "5-hour", 96.0, 3 * H, 5 * H),
                    window(WindowKind::Other, "Credits", 100.0, 0, 0),
                ],
            )),
            local: Default::default(),
            session_rate: None,
            now_ms: 1_000 * H,
        };
        let o = overview(&s);
        assert_eq!(o.avg_percent, 52.0, "the Other window is not a plan window");
        let (provider, plan, label, percent, _) = o.peak.clone().unwrap();
        assert_eq!(
            (provider, plan.as_str(), label.as_str(), percent),
            (Provider::Codex, "Codex Plus", "5-hour", 96.0)
        );
        assert_eq!((o.active, o.at_risk), (2, 1));
        assert_eq!(o.next_reset_ms, Some(3 * H));
        assert!(!o.stale);
        s.codex.as_mut().unwrap().error = Some(limits::LimitsError::Http(500));
        assert!(overview(&s).stale, "an old reading kept across an error");
    }

    #[test]
    fn level_class_steps_at_70_and_90() {
        assert_eq!(level_class(69.0), "calm");
        assert_eq!(level_class(70.0), "warn");
        assert_eq!(level_class(90.0), "danger");
    }
}
