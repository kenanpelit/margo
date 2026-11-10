//! The mdash dashboard component (`MenuWidget::Dashboard`).
//!
//! One header, one status strip, a segmented tab switch and a homogeneous
//! `gtk::Stack`: every tab gets the same allocation, so the panel keeps
//! one size whichever tab is on screen. Each page sits in its own
//! scroller (marked with `list_fit`) so a too-small screen or a configured
//! `maximum_height` scrolls inside the tab, never the header.
//!
//! Live data only flows while it is visible: the clock / strip tick runs
//! while the menu is open, the media and calendar feeds while Today is
//! shown, the system streams + limits poll while System is shown, and the
//! tile watchers while Controls is shown.

use super::controls::ControlsTab;
use super::header::{Header, HeaderEvent, datetime_line};
use super::strip::{Strip, StripState};
use super::system::{AiRow, SystemTab, UpdatesView};
use super::tab_state::{self, TabRequest};
use super::today::TodayTab;
use super::today::media::{Idle, MediaEvent, MediaView, NowPlaying, pick_player};
use super::today::timers::{alarm_line, next_alarm};
use super::tools::{ToolTarget, ToolsTab};
use crate::common_widgets::segmented::{Segmented, SegmentedStyle};
use crate::frame::MenuId;
use crate::menus::list_fit::mark_menu_list;
use crate::menus::menu_widgets::calendar_data;
use crate::menus::menu_widgets::panel_header::greeting_text;
use crate::menus::reveal_poll::{RevealGate, watch_stream};
use chrono::{Local, NaiveDateTime};
use futures::StreamExt;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    AlarmConfigStoreFields, ConfigStoreFields, GeneralStoreFields,
};
use mshell_config::schema::dashboard::{DashboardConfig, DashboardTab};
use mshell_utils::session::run_session_action;
use reactive_graph::traits::GetUntracked;
use relm4::gtk::glib;
use relm4::gtk::prelude::{BoxExt, WidgetExt};
use relm4::{Component, ComponentController, ComponentParts, ComponentSender, gtk};
use std::sync::Arc;
use std::time::Duration;
use wayle_media::core::player::Player;
use wayle_media::types::PlaybackState;

pub(crate) struct DashboardInit {
    pub(crate) config: DashboardConfig,
    /// `false` only in render fixtures: no service is touched.
    pub(crate) live: bool,
}

#[derive(Debug)]
pub(crate) enum DashboardInput {
    ParentRevealChanged(bool),
    /// A segment / key picked tab `i` (index into the shown tabs).
    Select(usize),
    Key(tab_state::TabKey),
    /// `mshellctl menu mdash <tab>`; `was_open` = mdash was on screen.
    Request {
        tab: DashboardTab,
        was_open: bool,
    },
    /// The 1 s clock tick while open.
    Tick,
    Header(HeaderEvent),
    Media(MediaEvent),
    OpenMenu(MenuId),
    #[cfg(test)]
    Fixture(Box<super::render_tests::DashboardFixture>),
}

#[derive(Debug)]
pub(crate) enum DashboardOutput {
    /// Close mdash and open this menu.
    OpenMenu(MenuId),
    /// Close mdash.
    CloseMenu,
}

#[derive(Debug)]
pub(crate) enum DashboardCommand {
    Events(Vec<mcal::Event>),
    WeatherChanged,
    Media(MediaView, Option<Arc<Player>>),
    Updates(UpdatesView),
    Ai(Vec<AiRow>),
    Cpu(f64, Option<u64>, Option<f64>),
    Memory(u64, u64, f64),
    Net(u64, u64),
    Disk(String, u64, u64),
}

pub(crate) struct DashboardModel {
    tabs: Vec<DashboardTab>,
    current: usize,
    revealed: bool,
    live: bool,
    stack: gtk::Stack,
    segmented: Segmented,
    header: Header,
    strip: Strip,
    strip_state: StripState,
    today: Option<TodayTab>,
    controls: Option<ControlsTab>,
    system: Option<SystemTab>,
    /// Kept alive with the page it built.
    _tools: Option<ToolsTab>,
    clock: Option<glib::SourceId>,
    ticks: u32,
    /// Open and on Today / on System.
    today_gate: RevealGate,
    system_gate: RevealGate,
    ai_poll: Option<mshell_utils::shared_poll::PollSubscription<crate::agents::UsageSnapshot>>,
    /// The player the media card shows (transport target).
    player: Option<Arc<Player>>,
    /// A player picked with the switch buttons (by bus name).
    picked_player: Arc<std::sync::Mutex<Option<String>>>,
    /// Render fixtures freeze the clock.
    frozen_now: Option<NaiveDateTime>,
}

impl DashboardModel {
    fn now(&self) -> NaiveDateTime {
        self.frozen_now
            .unwrap_or_else(|| Local::now().naive_local())
    }

    fn current_tab(&self) -> DashboardTab {
        self.tabs[self.current]
    }

    /// Re-gate the per-tab feeds after a reveal or tab change.
    fn sync_gates(&mut self) {
        let tab = self.current_tab();
        let open = self.revealed;
        self.today_gate.set(open && tab == DashboardTab::Today);
        self.system_gate.set(open && tab == DashboardTab::System);
        if let Some(controls) = &self.controls {
            controls.set_visible(open && tab == DashboardTab::Controls);
        }
        if let Some(poll) = &self.ai_poll {
            poll.set_active(open && tab == DashboardTab::System);
        }
        if open
            && tab == DashboardTab::System
            && let Some(system) = &self.system
        {
            system.power_profile.emit(
                crate::menus::menu_widgets::control_center::power_profile::ControlCenterPowerProfileInput::Refresh,
            );
        }
    }

    fn select(&mut self, i: usize, sender: &ComponentSender<Self>) {
        if i >= self.tabs.len() {
            return;
        }
        let changed = i != self.current;
        self.current = i;
        self.segmented.set_active(i);
        self.stack.set_visible_child_name(self.tabs[i].token());
        if changed && self.live {
            tab_state::remember(self.tabs[i]);
        }
        self.sync_gates();
        if self.revealed && self.tabs[i] == DashboardTab::Today {
            self.refresh_today(sender);
        }
    }

    fn refresh_header(&self) {
        let now = self.now();
        let h24 = if self.live {
            config_manager()
                .config()
                .general()
                .clock_format_24_h()
                .get_untracked()
        } else {
            true
        };
        let greeting = if self.frozen_now.is_some() {
            crate::menus::menu_widgets::panel_header::greeting_for(
                chrono::Timelike::hour(&now) as u8,
                Some("kenan"),
            )
        } else {
            greeting_text()
        };
        self.header.set_time(&greeting, &datetime_line(now, h24));
    }

    fn refresh_timers(&self) {
        let Some(today) = &self.today else {
            return;
        };
        let now = self.now();
        let alarms = if self.live {
            config_manager().config().alarm().alarms().get_untracked()
        } else {
            Vec::new()
        };
        let line = alarm_line(next_alarm(&alarms, now).as_ref(), now);
        let unix = if self.frozen_now.is_some() {
            now.and_utc().timestamp()
        } else {
            crate::timers::now_unix()
        };
        today.timers.apply(&crate::timers::list(), unix, &line);
    }

    /// Today on screen: reload events + weather and follow the date.
    fn refresh_today(&self, sender: &ComponentSender<Self>) {
        let Some(today) = &self.today else {
            return;
        };
        today.calendar.follow_today(self.now().date());
        self.refresh_timers();
        if !self.live {
            return;
        }
        apply_weather(today, self.now());
        let config = calendar_data::shell_calendar_config();
        let window = calendar_data::load_window();
        sender.oneshot_command(async move {
            DashboardCommand::Events(calendar_data::fetch(config, window).await)
        });
    }

    fn refresh_strip(&mut self) {
        if !self.live {
            self.strip.apply(&self.strip_state);
            return;
        }
        let s = &mut self.strip_state;
        let battery = mshell_services::battery_reading();
        s.battery = battery.present.then(|| {
            let charging = matches!(
                battery.state,
                wayle_battery::types::DeviceState::Charging
                    | wayle_battery::types::DeviceState::FullyCharged
            );
            (battery.percentage.round().clamp(0.0, 100.0) as u8, charging)
        });
        s.network = mshell_services::network_service().and_then(|n| {
            if let Some(ssid) = n.wifi.get().and_then(|w| w.ssid.get()) {
                return Some((ssid, "network-wireless-symbolic"));
            }
            matches!(
                n.wired.get().map(|w| w.connectivity.get()),
                Some(wayle_network::types::states::NetworkStatus::Connected)
            )
            .then(|| ("Wired".to_string(), "network-wired-symbolic"))
        });
        s.vpn = crate::menus::menu_widgets::control_center::tiles::tile_probes::read_vpn_state().1;
        s.volume = mshell_services::audio_service()
            .and_then(|a| a.default_output.get())
            .map(|d| {
                (
                    (d.volume.get().average() * 100.0).round().clamp(0.0, 150.0) as u8,
                    d.muted.get(),
                )
            });
        s.weather = mshell_services::weather_service()
            .weather
            .get()
            .map(|w| weather_chip(&w));
        s.unread = crate::notification_unread::unread();
        self.strip.apply(&self.strip_state);
        if let Some(system) = &self.system {
            let label = battery_state_label(battery.state);
            system.apply_battery(
                battery
                    .present
                    .then(|| (battery.percentage.round().clamp(0.0, 100.0) as u8, label)),
            );
        }
    }

    fn start_clock(&mut self, sender: &ComponentSender<Self>) {
        if self.clock.is_none() {
            let s = sender.clone();
            self.clock = Some(glib::timeout_add_local(Duration::from_secs(1), move || {
                s.input(DashboardInput::Tick);
                glib::ControlFlow::Continue
            }));
        }
    }

    fn stop_clock(&mut self) {
        if let Some(id) = self.clock.take() {
            id.remove();
        }
    }
}

fn weather_chip(w: &wayle_weather::Weather) -> (String, &'static str) {
    let unit = wayle_weather::TemperatureUnit::from(
        config_manager()
            .config()
            .general()
            .temperature_unit()
            .get_untracked(),
    );
    (
        super::today::weather::degrees(&w.current.temperature, &unit),
        mshell_utils::weather::get_weather_icon_name(&w.current.condition, w.current.is_day),
    )
}

fn battery_state_label(state: wayle_battery::types::DeviceState) -> &'static str {
    use wayle_battery::types::DeviceState as S;
    match state {
        S::Charging => "Charging",
        S::Discharging => "On battery",
        S::FullyCharged => "Fully charged",
        S::Empty => "Empty",
        S::PendingCharge | S::PendingDischarge => "Not charging",
        S::Unknown => "",
    }
}

fn apply_weather(today: &TodayTab, now: NaiveDateTime) {
    let weather = mshell_services::weather_service()
        .weather
        .get()
        .map(|arc| (*arc).clone())
        .or_else(mshell_utils::weather::load_weather_cache);
    let config = config_manager().config();
    let unit = wayle_weather::TemperatureUnit::from(
        config.clone().general().temperature_unit().get_untracked(),
    );
    let h24 = config.general().clock_format_24_h().get_untracked();
    today.weather.apply(weather.as_ref(), &unit, h24, now);
}

/// One pass over the MPRIS players (off the GTK thread): what the media
/// card shows and the player its buttons drive.
async fn media_snapshot(picked: Option<String>) -> (MediaView, Option<Arc<Player>>) {
    let players = mshell_services::media_service()
        .map(|m| (m.players(), m.active_player.get()))
        .unwrap_or_default();
    let (players, active) = players;
    let playing: Vec<bool> = players
        .iter()
        .map(|p| p.playback_state.get() == PlaybackState::Playing)
        .collect();
    let stopped: Vec<bool> = players
        .iter()
        .map(|p| p.playback_state.get() == PlaybackState::Stopped)
        .collect();
    let picked_index = picked
        .as_deref()
        .and_then(|id| players.iter().position(|p| p.id.bus_name() == id))
        .filter(|i| !stopped[*i]);
    let active_index = active
        .as_ref()
        .and_then(|a| players.iter().position(|p| p.id == a.id));
    let index = picked_index.or_else(|| pick_player(&playing, &stopped, active_index));
    let Some(i) = index else {
        return (MediaView::Idle(idle_offers()), None);
    };
    let p = players[i].clone();
    let meta = &p.metadata;
    let art = meta
        .cover_art
        .get()
        .or_else(|| meta.art_url.get())
        .map(|a| a.strip_prefix("file://").map(str::to_string).unwrap_or(a));
    let position = p.position().await.unwrap_or_else(|_| p.position.get());
    let view = NowPlaying {
        player: p.identity.get(),
        title: meta.title.get(),
        artist: meta.artist.get(),
        art,
        position,
        length: meta.length.get(),
        playing: playing[i],
        can_prev: p.can_go_previous.get(),
        can_next: p.can_go_next.get(),
        players: stopped.iter().filter(|s| !**s).count(),
    };
    (MediaView::Playing(view), Some(p))
}

/// Nothing plays: the last Tune track to resume and the saved stations.
fn idle_offers() -> Idle {
    let tune = &mshell_services::mtune::mtune_service().player;
    let resume =
        (tune.running.get() && tune.has_song.get() && !tune.title.get().is_empty()).then(|| {
            let (title, artist) = (tune.title.get(), tune.artist.get());
            if artist.is_empty() {
                title
            } else {
                format!("{title} — {artist}")
            }
        });
    Idle {
        resume,
        stations: tune
            .radio_stations
            .get()
            .into_iter()
            .map(|(key, name, _, _)| (key, name))
            .collect(),
    }
}

/// The Claude / Codex windows worth a glance: session + weekly of each
/// provider that has a reading.
fn ai_rows(snap: &crate::agents::UsageSnapshot) -> Vec<AiRow> {
    use crate::agents::limits::WindowKind;
    let providers = std::iter::once(&snap.claude).chain(snap.codex.as_ref());
    let mut rows = Vec::new();
    for p in providers {
        let Some(usage) = &p.usage else { continue };
        for w in &usage.windows {
            let span = match w.kind {
                WindowKind::Session => "5-hour",
                WindowKind::Weekly => "weekly",
                _ => continue,
            };
            rows.push(AiRow {
                label: format!("{} · {span}", p.provider.name()),
                percent: w.percent,
            });
        }
    }
    rows
}

fn updates_view(report: &crate::system_update::UpdateReport) -> UpdatesView {
    use crate::system_update::Source;
    let parts: Vec<String> = [Source::Repo, Source::Aur, Source::Flatpak]
        .into_iter()
        .filter_map(|s| {
            let n = report.count(s);
            (n > 0).then(|| format!("{n} {}", s.label()))
        })
        .collect();
    UpdatesView {
        total: report.total(),
        detail: if parts.is_empty() {
            "Nothing to install".to_string()
        } else {
            parts.join(" · ")
        },
    }
}

/// Wrap a tab page in its own scroller: the page scrolls inside the tab
/// when the screen (or `maximum_height`) is too small.
fn page(child: &impl gtk::prelude::IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    let sw = gtk::ScrolledWindow::new();
    sw.add_css_class("mdash-page");
    sw.set_hscrollbar_policy(gtk::PolicyType::Never);
    sw.set_vscrollbar_policy(gtk::PolicyType::Automatic);
    sw.set_propagate_natural_height(true);
    sw.set_child(Some(child));
    // No built-in cap: the page is as tall as its content until the
    // menu's height setting (or the screen) bounds it.
    mark_menu_list(&sw, 4000);
    sw
}

impl Component for DashboardModel {
    type CommandOutput = DashboardCommand;
    type Input = DashboardInput;
    type Output = DashboardOutput;
    type Init = DashboardInit;
    type Root = gtk::Box;
    type Widgets = ();

    fn init_root() -> Self::Root {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
        root.add_css_class("mdash-dashboard");
        root
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let live = params.live;
        let shown = params.config.visible_tabs();
        let tabs: Vec<DashboardTab> = shown.iter().map(|t| t.tab).collect();
        let remembered = if live {
            tab_state::read_remembered(&tab_state::state_path())
        } else {
            None
        };
        let current = tab_state::initial_index(&tabs, remembered);

        let s = sender.clone();
        let header = Header::new(move |e| s.input(DashboardInput::Header(e)));
        root.append(&header.root);
        let s = sender.clone();
        let strip = Strip::new(move |id| s.input(DashboardInput::OpenMenu(id)));
        root.append(&strip.root);

        let labels: Vec<&str> = tabs.iter().map(|t| t.label()).collect();
        let s = sender.clone();
        let segmented = Segmented::new(&labels, current, SegmentedStyle::Capsule, move |i| {
            s.input(DashboardInput::Select(i))
        });
        segmented.root().add_css_class("mdash-tabs");
        for (b, tab) in segmented.buttons().iter().zip(&tabs) {
            b.set_tooltip_text(Some(&format!(
                "{} ({})",
                tab.label(),
                tabs.iter().position(|t| t == tab).map_or(0, |i| i + 1)
            )));
        }
        segmented.root().set_visible(tabs.len() > 1);
        root.append(segmented.root());

        let stack = gtk::Stack::new();
        stack.add_css_class("mdash-stack");
        stack.set_hhomogeneous(true);
        stack.set_vhomogeneous(true);
        stack.set_transition_type(gtk::StackTransitionType::Crossfade);
        stack.set_transition_duration(120);
        stack.set_vexpand(true);

        let open = {
            let s = sender.clone();
            move |id: MenuId| s.input(DashboardInput::OpenMenu(id))
        };
        let (mut today, mut controls, mut system, mut tools) = (None, None, None, None);
        for cfg in &shown {
            let widget: gtk::Widget = match cfg.tab {
                DashboardTab::Today => {
                    let s = sender.clone();
                    let t = TodayTab::new(cfg, move |e| s.input(DashboardInput::Media(e)));
                    let w = t.root.clone().into();
                    today = Some(t);
                    w
                }
                DashboardTab::Controls => {
                    let t = ControlsTab::new(cfg, live, open.clone());
                    let w = t.root.clone().into();
                    controls = Some(t);
                    w
                }
                DashboardTab::System => {
                    let t = SystemTab::new(cfg, open.clone());
                    let w = t.root.clone().into();
                    system = Some(t);
                    w
                }
                DashboardTab::Tools => {
                    let s = sender.clone();
                    let t = ToolsTab::new(move |target| match target {
                        ToolTarget::Menu(id) => s.input(DashboardInput::OpenMenu(id)),
                        ToolTarget::Settings => mshell_settings::open_settings(),
                    });
                    let w = t.root.clone().into();
                    tools = Some(t);
                    w
                }
            };
            stack.add_named(&page(&widget), Some(cfg.tab.token()));
        }
        stack.set_visible_child_name(tabs[current].token());
        root.append(&stack);

        // 1–9 / Ctrl+Tab / Ctrl+Shift+Tab. Bubble phase: a focused entry
        // (the timer field) keeps its digits.
        let keys = gtk::EventControllerKey::new();
        let s = sender.clone();
        keys.connect_key_pressed(
            move |_, key, _, state| match tab_state::tab_key(key, state) {
                Some(k) => {
                    s.input(DashboardInput::Key(k));
                    glib::Propagation::Stop
                }
                None => glib::Propagation::Proceed,
            },
        );
        root.add_controller(keys);

        let today_gate = RevealGate::new(false);
        let system_gate = RevealGate::new(false);
        let picked_player: Arc<std::sync::Mutex<Option<String>>> = Arc::default();

        let mut ai_poll = None;
        if live {
            spawn_live_feeds(&sender, &today_gate, &system_gate, &picked_player);
            if system.is_some() {
                let poll = crate::agents::USAGE_POLL.subscribe();
                let rx = poll.receiver();
                sender.command(move |out, shutdown| {
                    shutdown
                        .register(async move {
                            let mut stream = Box::pin(watch_stream(rx));
                            while let Some(snap) = stream.next().await {
                                if let Some(snap) = snap
                                    && out.send(DashboardCommand::Ai(ai_rows(&snap))).is_err()
                                {
                                    break;
                                }
                            }
                        })
                        .drop_on_shutdown()
                });
                ai_poll = Some(poll);
            }
        }

        let model = DashboardModel {
            tabs,
            current,
            revealed: false,
            live,
            stack,
            segmented,
            header,
            strip,
            strip_state: StripState::default(),
            today,
            controls,
            system,
            _tools: tools,
            clock: None,
            ticks: 0,
            today_gate,
            system_gate,
            ai_poll,
            player: None,
            picked_player,
            frozen_now: None,
        };
        model.refresh_header();
        model.refresh_timers();
        ComponentParts { model, widgets: () }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            DashboardInput::ParentRevealChanged(open) => {
                self.revealed = open;
                if open {
                    self.start_clock(&sender);
                    self.refresh_header();
                    self.refresh_strip();
                    if self.live {
                        let s = sender.clone();
                        relm4::spawn(async move {
                            if let Some(report) = crate::system_update::glance_report().await {
                                s.command_sender()
                                    .emit(DashboardCommand::Updates(updates_view(&report)));
                            }
                        });
                    }
                    if self.current_tab() == DashboardTab::Today {
                        self.refresh_today(&sender);
                    }
                } else {
                    self.stop_clock();
                }
                self.sync_gates();
            }
            DashboardInput::Select(i) => self.select(i, &sender),
            DashboardInput::Key(k) => {
                if let Some(i) = tab_state::apply_key(k, self.tabs.len(), self.current) {
                    self.select(i, &sender);
                }
            }
            DashboardInput::Request { tab, was_open } => {
                match tab_state::request(&self.tabs, self.current, tab, was_open) {
                    TabRequest::Show(i) => self.select(i, &sender),
                    TabRequest::Close => {
                        let _ = sender.output(DashboardOutput::CloseMenu);
                    }
                    TabRequest::Ignore => {}
                }
            }
            DashboardInput::Tick => {
                self.ticks = self.ticks.wrapping_add(1);
                self.refresh_header();
                if self.current_tab() == DashboardTab::Today {
                    self.refresh_timers();
                }
                if self.ticks.is_multiple_of(5) {
                    self.refresh_strip();
                }
            }
            DashboardInput::Header(HeaderEvent::Settings) => mshell_settings::open_settings(),
            DashboardInput::Header(HeaderEvent::Session(action)) => {
                // Close first so a lock screen / logout isn't drawn over
                // an open dashboard.
                let _ = sender.output(DashboardOutput::CloseMenu);
                if self.live {
                    glib::idle_add_local_once(move || run_session_action(action));
                }
            }
            DashboardInput::OpenMenu(id) => {
                let _ = sender.output(DashboardOutput::OpenMenu(id));
            }
            DashboardInput::Media(event) => self.media_event(event, &sender),
            #[cfg(test)]
            DashboardInput::Fixture(f) => f.apply(self),
        }
    }

    fn update_cmd(
        &mut self,
        message: Self::CommandOutput,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            DashboardCommand::Events(events) => {
                if let Some(today) = &self.today {
                    today.calendar.set_events(events);
                }
            }
            DashboardCommand::WeatherChanged => {
                if let Some(today) = &self.today
                    && self.revealed
                {
                    apply_weather(today, self.now());
                }
            }
            DashboardCommand::Media(view, player) => {
                self.player = player;
                if let Some(today) = &self.today {
                    today.media.apply(&view);
                }
            }
            DashboardCommand::Updates(view) => {
                self.strip_state.updates = view.total;
                self.strip.apply(&self.strip_state);
                if let Some(system) = &self.system {
                    system.apply_updates(Some(&view));
                }
            }
            DashboardCommand::Ai(rows) => {
                if let Some(system) = &self.system {
                    system.apply_ai(&rows);
                }
            }
            DashboardCommand::Cpu(pct, freq, temp) => {
                if let Some(system) = &self.system {
                    system.apply_cpu(pct, freq);
                    system.apply_temp(temp);
                }
            }
            DashboardCommand::Memory(used, total, pct) => {
                if let Some(system) = &self.system {
                    system.apply_memory(used, total, pct);
                }
            }
            DashboardCommand::Net(rx, tx) => {
                if let Some(system) = &self.system {
                    system.apply_net(rx, tx);
                }
            }
            DashboardCommand::Disk(mount, used, total) => {
                if let Some(system) = &self.system {
                    system.apply_disk(&mount, used, total);
                }
            }
        }
    }
}

impl DashboardModel {
    fn media_event(&mut self, event: MediaEvent, sender: &ComponentSender<Self>) {
        if !self.live {
            return;
        }
        let player = self.player.clone();
        match event {
            MediaEvent::PlayPause | MediaEvent::Next | MediaEvent::Previous => {
                if let Some(p) = player {
                    mshell_services::tokio_rt_spawn(async move {
                        let _ = match event {
                            MediaEvent::Next => p.next().await,
                            MediaEvent::Previous => p.previous().await,
                            _ => p.play_pause().await,
                        };
                    });
                }
            }
            MediaEvent::SwitchPlayer(forward) => {
                let ids: Vec<String> = mshell_services::media_service()
                    .map(|m| {
                        m.players()
                            .into_iter()
                            .filter(|p| p.playback_state.get() != PlaybackState::Stopped)
                            .map(|p| p.id.bus_name().to_string())
                            .collect()
                    })
                    .unwrap_or_default();
                let current = self.player.as_ref().map(|p| p.id.bus_name().to_string());
                let at = current
                    .and_then(|c| ids.iter().position(|i| *i == c))
                    .unwrap_or(0);
                if !ids.is_empty() {
                    let next = tab_state::cycle(ids.len(), at, forward);
                    if let Ok(mut picked) = self.picked_player.lock() {
                        *picked = Some(ids[next].clone());
                    }
                }
            }
            MediaEvent::Resume => {
                mshell_services::tokio_rt_spawn(async move {
                    mshell_services::mtune::mtune_service()
                        .player
                        .play_pause()
                        .await;
                });
            }
            MediaEvent::PlayStation(key) => {
                mshell_services::tokio_rt_spawn(async move {
                    if let Err(e) = mshell_services::mtune::mtune_service()
                        .player
                        .play_radio_station(&key)
                        .await
                    {
                        tracing::warn!(error = %e, "mdash: couldn't start the station");
                    }
                });
            }
            MediaEvent::OpenMusic => {
                let _ = sender.output(DashboardOutput::OpenMenu(MenuId::Mtune));
            }
        }
    }
}

/// The feeds that run only while their tab is on screen.
fn spawn_live_feeds(
    sender: &ComponentSender<DashboardModel>,
    today_gate: &RevealGate,
    system_gate: &RevealGate,
    picked: &Arc<std::sync::Mutex<Option<String>>>,
) {
    mshell_utils::weather::spawn_weather_watcher(sender, || DashboardCommand::WeatherChanged);

    // Media: a 1 s pass while Today is on screen (position, track, state).
    let (gate, picked) = (today_gate.clone(), picked.clone());
    sender.command(move |out, shutdown| {
        shutdown
            .register(async move {
                loop {
                    gate.until_revealed().await;
                    let pick = picked.lock().ok().and_then(|p| p.clone());
                    let (view, player) = media_snapshot(pick).await;
                    if out.send(DashboardCommand::Media(view, player)).is_err() {
                        break;
                    }
                    gate.tick(Duration::from_secs(1)).await;
                }
            })
            .drop_on_shutdown()
    });

    // System: the shared sysinfo streams, subscribed only while System is
    // on screen (wayle polls only with a subscriber).
    let sensor = crate::bars::bar_widgets::sysstat::find_cpu_temp_sensor_pub();
    let gate = system_gate.clone();
    sender.command(move |out, shutdown| {
        shutdown
            .register(async move {
                gate.stream_while_revealed(
                    || mshell_services::sys_info_service().cpu.watch().boxed(),
                    |cpu| {
                        let temp = cpu.temperature_celsius.map(f64::from).or_else(|| {
                            sensor
                                .as_ref()
                                .and_then(crate::bars::bar_widgets::sysstat::read_temp_millideg_pub)
                                .map(|m| f64::from(m) / 1000.0)
                        });
                        let _ = out.send(DashboardCommand::Cpu(
                            f64::from(cpu.usage_percent),
                            Some(cpu.avg_frequency_mhz),
                            temp,
                        ));
                    },
                )
                .await;
            })
            .drop_on_shutdown()
    });
    let gate = system_gate.clone();
    sender.command(move |out, shutdown| {
        shutdown
            .register(async move {
                gate.stream_while_revealed(
                    || mshell_services::sys_info_service().memory.watch().boxed(),
                    |m| {
                        let _ = out.send(DashboardCommand::Memory(
                            m.used_bytes,
                            m.total_bytes,
                            f64::from(m.usage_percent),
                        ));
                    },
                )
                .await;
            })
            .drop_on_shutdown()
    });
    let gate = system_gate.clone();
    sender.command(move |out, shutdown| {
        shutdown
            .register(async move {
                gate.stream_while_revealed(
                    || mshell_services::sys_info_service().network.watch().boxed(),
                    |ifaces| {
                        // Physical links only: a tunnel's bytes also cross
                        // the link that carries it.
                        let (rx, tx) = ifaces
                            .iter()
                            .filter(|i| !mshell_utils::netstat::is_virtual(&i.interface))
                            .fold((0, 0), |(rx, tx), i| {
                                (rx + i.rx_bytes_per_sec, tx + i.tx_bytes_per_sec)
                            });
                        let _ = out.send(DashboardCommand::Net(rx, tx));
                    },
                )
                .await;
            })
            .drop_on_shutdown()
    });
    let gate = system_gate.clone();
    sender.command(move |out, shutdown| {
        shutdown
            .register(async move {
                gate.stream_while_revealed(
                    || mshell_services::sys_info_service().disks.watch().boxed(),
                    |disks| {
                        let root = disks
                            .iter()
                            .find(|d| d.mount_point == std::path::Path::new("/"))
                            .or_else(|| disks.iter().max_by_key(|d| d.total_bytes));
                        if let Some(d) = root {
                            let _ = out.send(DashboardCommand::Disk(
                                d.mount_point.display().to_string(),
                                d.used_bytes,
                                d.total_bytes,
                            ));
                        }
                    },
                )
                .await;
            })
            .drop_on_shutdown()
    });
}

impl Drop for DashboardModel {
    fn drop(&mut self) {
        self.stop_clock();
    }
}

impl std::fmt::Debug for DashboardModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DashboardModel")
            .field("tabs", &self.tabs)
            .field("current", &self.current)
            .finish()
    }
}

/// Test access for the render fixture.
#[cfg(test)]
impl DashboardModel {
    pub(crate) fn fixture_parts(
        &mut self,
    ) -> (
        Option<&TodayTab>,
        Option<&ControlsTab>,
        Option<&SystemTab>,
        &Strip,
        &mut StripState,
        &mut Option<NaiveDateTime>,
    ) {
        (
            self.today.as_ref(),
            self.controls.as_ref(),
            self.system.as_ref(),
            &self.strip,
            &mut self.strip_state,
            &mut self.frozen_now,
        )
    }

    pub(crate) fn fixture_refresh(&mut self) {
        self.refresh_header();
        self.refresh_timers();
        self.strip.apply(&self.strip_state);
    }

    pub(crate) fn stack(&self) -> &gtk::Stack {
        &self.stack
    }
}
