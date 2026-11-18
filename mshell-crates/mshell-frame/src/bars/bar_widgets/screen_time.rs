//! Screen Time — bar pill for [`crate::screen_time`]: a clock glyph and
//! today's focused time (`3h 12m`). Hover lists the five most used apps of
//! today; a click opens the Screen Time menu. While recording is off the
//! pill shows "off", dimmed, and its tooltip says where to turn it on.

use crate::screen_time::engine::format_duration;
use crate::screen_time::{self, ScreenTimeState};
use chrono::Local;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, ScreenTimeConfigStoreFields};
use reactive_graph::prelude::GetUntracked;
use relm4::gtk::Orientation;
use relm4::gtk::prelude::{BoxExt, ButtonExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, RelmWidgetExt, gtk};

pub(crate) struct ScreenTimeModel {
    label: String,
    tooltip: String,
    enabled: bool,
    _orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum ScreenTimeInput {
    Clicked,
}

#[derive(Debug)]
pub(crate) enum ScreenTimeOutput {
    Clicked,
}

pub(crate) struct ScreenTimeInit {
    pub(crate) orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum ScreenTimeCommandOutput {
    State(ScreenTimeState),
}

/// Pill label and tooltip for `s`.
pub(crate) fn pill_text(s: &ScreenTimeState, min_secs: u64) -> (String, String) {
    if !s.enabled {
        return (
            "off".to_string(),
            "Screen time is off\n\nTurn it on in Settings → Widgets → Screen Time".to_string(),
        );
    }
    let today = Local::now().date_naive();
    let total = s.history.day_total(today);
    let mut tip = format!("Screen time today: {}", format_duration(total));
    for (app, secs) in s.history.top_apps(today, 1, min_secs).iter().take(5) {
        tip.push_str(&format!("\n{app}  {}", format_duration(*secs)));
    }
    match (&s.app, s.counting) {
        (Some(app), true) => tip.push_str(&format!("\n\nCounting: {app}")),
        _ => tip.push_str("\n\nPaused (away, locked or nothing focused)"),
    }
    (format_duration(total), tip)
}

#[relm4::component(pub)]
impl Component for ScreenTimeModel {
    type CommandOutput = ScreenTimeCommandOutput;
    type Input = ScreenTimeInput;
    type Output = ScreenTimeOutput;
    type Init = ScreenTimeInit;

    view! {
        #[root]
        gtk::Box {
            set_css_classes: &["screen-time-bar-widget", "ok-button-surface", "ok-bar-widget"],
            #[watch]
            set_class_active: ("dim", !model.enabled),
            set_hexpand: model._orientation == Orientation::Vertical,
            set_vexpand: model._orientation == Orientation::Horizontal,
            set_halign: gtk::Align::Center,
            set_valign: gtk::Align::Center,
            set_has_tooltip: true,
            #[watch]
            set_tooltip_text: Some(&model.tooltip),

            gtk::Button {
                set_css_classes: &["ok-button-flat"],
                set_hexpand: false,
                set_vexpand: false,
                connect_clicked[sender] => move |_| {
                    sender.input(ScreenTimeInput::Clicked);
                },

                gtk::Box {
                    set_orientation: Orientation::Horizontal,
                    set_spacing: 4,
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,

                    gtk::Image {
                        set_icon_name: Some("preferences-system-time-symbolic"),
                    },
                    gtk::Label {
                        add_css_class: "screen-time-bar-label",
                        #[watch]
                        set_label: &model.label,
                        set_visible: model._orientation == Orientation::Horizontal,
                    },
                },
            }
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let mut rx = screen_time::subscribe();
        sender.command(|out, shutdown| async move {
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            loop {
                let state = rx.borrow_and_update().clone();
                let _ = out.send(ScreenTimeCommandOutput::State(state));
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    r = rx.changed() => if r.is_err() { break },
                }
            }
        });
        let model = ScreenTimeModel {
            label: "off".to_string(),
            tooltip: String::new(),
            enabled: false,
            _orientation: params.orientation,
        };
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            ScreenTimeInput::Clicked => {
                let _ = sender.output(ScreenTimeOutput::Clicked);
            }
        }
    }

    fn update_cmd(
        &mut self,
        message: Self::CommandOutput,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            ScreenTimeCommandOutput::State(s) => {
                let min = config_manager()
                    .config()
                    .screen_time()
                    .hide_under_minutes()
                    .get_untracked();
                let (label, tooltip) = pill_text(&s, u64::from(min) * 60);
                self.enabled = s.enabled;
                self.label = label;
                self.tooltip = tooltip;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_state_says_where_to_enable() {
        let (label, tip) = pill_text(&ScreenTimeState::default(), 60);
        assert_eq!(label, "off");
        assert!(tip.contains("Settings"));
    }

    #[test]
    fn on_state_shows_today_and_paused() {
        let s = ScreenTimeState {
            enabled: true,
            ..Default::default()
        };
        let (label, tip) = pill_text(&s, 60);
        assert_eq!(label, "<1m");
        assert!(tip.starts_with("Screen time today: <1m"));
        assert!(tip.contains("Paused"));
    }
}
