//! mshelldash — a standalone, tabbed dashboard inspired by
//! DankMaterialShell's DankDash but rebuilt on margo's DESIGN.md
//! language (surfaces over borders, --font-* scale, matugen tokens,
//! canonical hover). Coexists with the classic dashboard menu.
//!
//! Five tabs behind a top tab bar + a crossfading `gtk::Stack`:
//!   - Overview  — at-a-glance mosaic (built out in a later wave)
//!   - Media     — the full media player widget (reused)
//!   - Weather   — the all-in-one weather widget (reused)
//!   - Wallpaper — the wallpaper picker grid (reused)
//!   - System    — the CPU/system dashboard (reused)
//!
//! The non-Overview tabs reuse margo's existing menu-widget
//! components as child controllers, so they stay in sync with their
//! standalone menus instead of being reimplemented.

use crate::menus::menu_widgets::audio_dashboard::audio_dashboard_menu_widget::{
    AudioDashboardMenuWidgetInit, AudioDashboardMenuWidgetModel,
};
use crate::menus::menu_widgets::calendar::{CalendarInit, CalendarModel};
use crate::menus::menu_widgets::cpu_dashboard::cpu_dashboard_menu_widget::{
    CpuDashboardMenuWidgetInit, CpuDashboardMenuWidgetModel,
};
use crate::menus::menu_widgets::media_player::media_players::{
    MediaPlayersInit, MediaPlayersModel,
};
use crate::menus::menu_widgets::mshelldash::overview::{OverviewInit, OverviewModel};
use crate::menus::menu_widgets::mshelldash::screen_time::{
    ScreenTimeInit, ScreenTimeInput, ScreenTimeModel,
};
use crate::menus::menu_widgets::wallpaper::wallpaper_menu_widget::{
    WallpaperMenuWidgetInit, WallpaperMenuWidgetModel,
};
use crate::menus::menu_widgets::weather::weather::{WeatherInit, WeatherModel};
use relm4::gtk::prelude::{BoxExt, ButtonExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentController, ComponentParts, ComponentSender, Controller, gtk};

/// (stack name, label, symbolic icon) per tab, in display order.
const TABS: [(&str, &str, &str); 8] = [
    ("overview", "Overview", "view-grid-symbolic"),
    ("media", "Media", "multimedia-player-symbolic"),
    ("weather", "Weather", "weather-clear-symbolic"),
    ("wallpaper", "Wallpaper", "image-x-generic-symbolic"),
    ("system", "System", "utilities-system-monitor-symbolic"),
    ("audio", "Audio", "audio-volume-high-symbolic"),
    ("calendar", "Calendar", "x-office-calendar-symbolic"),
    ("screentime", "Screen Time", "appointment-soon-symbolic"),
];

pub(crate) struct MShellDashModel {
    active: usize,
    tab_buttons: Vec<gtk::Button>,
    // Child pages — kept alive for the lifetime of the dash.
    _overview: Controller<OverviewModel>,
    _media: Controller<MediaPlayersModel>,
    _weather: Controller<WeatherModel>,
    _wallpaper: Controller<WallpaperMenuWidgetModel>,
    _system: Controller<CpuDashboardMenuWidgetModel>,
    _audio: Controller<AudioDashboardMenuWidgetModel>,
    _calendar: Controller<CalendarModel>,
    screentime: Controller<ScreenTimeModel>,
}

impl std::fmt::Debug for MShellDashModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MShellDashModel")
            .field("active", &self.active)
            .finish()
    }
}

#[derive(Debug)]
pub(crate) enum MShellDashInput {
    SelectTab(usize),
    /// Jump to a tab by its stack name (e.g. "system"); no-op if the
    /// name is empty or unknown. Used when the dash is opened with an
    /// explicit target tab via IPC.
    SelectTabName(String),
}

#[derive(Debug)]
pub(crate) enum MShellDashOutput {}

pub(crate) struct MShellDashInit {}

#[relm4::component(pub(crate))]
impl Component for MShellDashModel {
    type CommandOutput = ();
    type Input = MShellDashInput;
    type Output = MShellDashOutput;
    type Init = MShellDashInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "mshelldash",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 12,
            // GtkBox ignores CSS `overflow:hidden`, so clip to the
            // rounded panel rect here (matches the .settings-panel fix).
            set_overflow: gtk::Overflow::Hidden,

            #[name = "tabbar"]
            gtk::Box {
                add_css_class: "mshelldash-tabbar",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 6,
                set_homogeneous: true,
            },

            #[name = "stack"]
            gtk::Stack {
                add_css_class: "mshelldash-stack",
                set_transition_type: gtk::StackTransitionType::Crossfade,
                set_transition_duration: 180,
                set_vhomogeneous: false,
                set_hexpand: true,
                set_vexpand: true,
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let overview = OverviewModel::builder().launch(OverviewInit {}).detach();
        let media = MediaPlayersModel::builder()
            .launch(MediaPlayersInit {})
            .detach();
        let weather = WeatherModel::builder()
            .launch(WeatherInit { all_in_one: true })
            .detach();
        let wallpaper = WallpaperMenuWidgetModel::builder()
            .launch(WallpaperMenuWidgetInit {
                thumbnail_width: 180,
                thumbnail_height: 120,
                row_count: 3,
            })
            .detach();
        let system = CpuDashboardMenuWidgetModel::builder()
            .launch(CpuDashboardMenuWidgetInit {})
            .detach();
        let audio = AudioDashboardMenuWidgetModel::builder()
            .launch(AudioDashboardMenuWidgetInit {})
            .detach();
        let calendar = CalendarModel::builder().launch(CalendarInit {}).detach();
        let screentime = ScreenTimeModel::builder()
            .launch(ScreenTimeInit {})
            .detach();

        let mut model = MShellDashModel {
            active: 0,
            tab_buttons: Vec::new(),
            _overview: overview,
            _media: media,
            _weather: weather,
            _wallpaper: wallpaper,
            _system: system,
            _audio: audio,
            _calendar: calendar,
            screentime,
        };

        let widgets = view_output!();

        widgets
            .stack
            .add_named(model._overview.widget(), Some("overview"));
        widgets
            .stack
            .add_named(model._media.widget(), Some("media"));
        widgets
            .stack
            .add_named(model._weather.widget(), Some("weather"));
        widgets
            .stack
            .add_named(model._wallpaper.widget(), Some("wallpaper"));
        widgets
            .stack
            .add_named(model._system.widget(), Some("system"));
        widgets
            .stack
            .add_named(model._audio.widget(), Some("audio"));
        widgets
            .stack
            .add_named(model._calendar.widget(), Some("calendar"));
        widgets
            .stack
            .add_named(model.screentime.widget(), Some("screentime"));

        // Tab bar — one button per tab (icon + label), selected state
        // via the shared `.selected` surface treatment.
        for (i, (_, label, icon)) in TABS.iter().enumerate() {
            let btn = gtk::Button::new();
            btn.set_css_classes(&tab_classes(0, i));
            let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            content.set_halign(gtk::Align::Center);
            content.append(&gtk::Image::from_icon_name(icon));
            content.append(&gtk::Label::new(Some(label)));
            btn.set_child(Some(&content));
            let s = sender.clone();
            btn.connect_clicked(move |_| {
                s.input(MShellDashInput::SelectTab(i));
            });
            widgets.tabbar.append(&btn);
            model.tab_buttons.push(btn);
        }

        widgets.stack.set_visible_child_name("overview");

        let _ = root;
        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::Input,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        let target = match message {
            MShellDashInput::SelectTab(i) => Some(i),
            MShellDashInput::SelectTabName(name) => TABS.iter().position(|(n, _, _)| *n == name),
        };
        if let Some(i) = target {
            self.active = i;
            for (j, b) in self.tab_buttons.iter().enumerate() {
                b.set_css_classes(&tab_classes(self.active, j));
            }
            widgets.stack.set_visible_child_name(TABS[i].0);
            // Lazily refresh the screen-time snapshot when its tab is
            // shown — nothing polls while another tab is active.
            if TABS[i].0 == "screentime" {
                self.screentime.emit(ScreenTimeInput::Refresh);
            }
        }
    }
}

/// CSS classes for a tab button — `selected` when it's the active tab.
fn tab_classes(active: usize, i: usize) -> Vec<&'static str> {
    if active == i {
        vec!["ok-button-surface", "mshelldash-tab", "selected"]
    } else {
        vec!["ok-button-surface", "mshelldash-tab"]
    }
}
