//! Twilight menu widget — the panel content for `MenuType::Twilight`.
//!
//! Quick controls for margo's blue-light filter: a master on/off, the
//! live temperature / phase / mode readout, a source-mode selector
//! (Auto / Manual / Static / Schedule), a temperature slider that
//! previews live, and "Resume schedule" to drop any preview override.
//! Everything routes through `mctl twilight …` (see [`crate::twilight`])
//! — the rich schedule-preset editor lives in Settings → Display.

use crate::twilight::{self, TwilightStatus};
use relm4::gtk::glib;
use relm4::gtk::prelude::{BoxExt, ButtonExt, OrientableExt, RangeExt, ScaleExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::Duration;

/// Poll cadence while the panel is open.
const POLL: Duration = Duration::from_secs(2);
/// Trailing debounce on the temperature slider so a drag lands one
/// `preview`, not one per pixel.
const TEMP_DEBOUNCE_MS: u64 = 120;

pub(crate) struct TwilightMenuWidgetModel {
    status: TwilightStatus,
    temp_debounce: Option<glib::JoinHandle<()>>,
}

#[derive(Debug)]
pub(crate) enum TwilightMenuWidgetInput {
    /// Poll result.
    Refresh(TwilightStatus),
    /// Header toggle — flip the filter on/off.
    Toggle,
    /// Switch the source mode (`geo` / `manual` / `static` / `schedule`).
    SetMode(&'static str),
    /// Slider moved — debounce into a live `preview`.
    TempChanged(u32),
    /// Debounce fired — pin the previewed temperature.
    TempCommit(u32),
    /// Drop any preview/test override and resume the schedule.
    Reset,
}

#[derive(Debug)]
pub(crate) enum TwilightMenuWidgetOutput {}

pub(crate) struct TwilightMenuWidgetInit {}

#[derive(Debug)]
pub(crate) enum TwilightMenuWidgetCommandOutput {
    Polled(TwilightStatus),
}

#[relm4::component(pub(crate))]
impl Component for TwilightMenuWidgetModel {
    type CommandOutput = TwilightMenuWidgetCommandOutput;
    type Input = TwilightMenuWidgetInput;
    type Output = TwilightMenuWidgetOutput;
    type Init = TwilightMenuWidgetInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "twilight-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 12,

            // Header: icon + title/status + on-off toggle.
            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 8,
                gtk::Image {
                    add_css_class: "twilight-header-icon",
                    #[watch]
                    set_icon_name: Some(model.status.icon()),
                },
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_hexpand: true,
                    gtk::Label {
                        add_css_class: "label-medium-bold",
                        set_halign: gtk::Align::Start,
                        set_label: "Twilight",
                    },
                    gtk::Label {
                        add_css_class: "label-small",
                        set_halign: gtk::Align::Start,
                        set_xalign: 0.0,
                        #[watch]
                        set_label: &status_line(&model.status),
                    },
                },
                gtk::Button {
                    set_valign: gtk::Align::Center,
                    #[watch]
                    set_css_classes: if model.status.enabled {
                        &["ok-button-surface", "selected"]
                    } else {
                        &["ok-button-surface"]
                    },
                    #[watch]
                    set_label: if model.status.enabled { "On" } else { "Off" },
                    connect_clicked[sender] => move |_| {
                        sender.input(TwilightMenuWidgetInput::Toggle);
                    },
                },
            },

            // Source-mode selector.
            gtk::Box {
                add_css_class: "twilight-mode-row",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 6,
                set_homogeneous: true,

                gtk::Button {
                    #[watch]
                    set_css_classes: &mode_classes(&model.status.mode, "geo"),
                    set_label: "Auto",
                    connect_clicked[sender] => move |_| {
                        sender.input(TwilightMenuWidgetInput::SetMode("geo"));
                    },
                },
                gtk::Button {
                    #[watch]
                    set_css_classes: &mode_classes(&model.status.mode, "manual"),
                    set_label: "Manual",
                    connect_clicked[sender] => move |_| {
                        sender.input(TwilightMenuWidgetInput::SetMode("manual"));
                    },
                },
                gtk::Button {
                    #[watch]
                    set_css_classes: &mode_classes(&model.status.mode, "static"),
                    set_label: "Static",
                    connect_clicked[sender] => move |_| {
                        sender.input(TwilightMenuWidgetInput::SetMode("static"));
                    },
                },
                gtk::Button {
                    #[watch]
                    set_css_classes: &mode_classes(&model.status.mode, "schedule"),
                    set_label: "Schedule",
                    connect_clicked[sender] => move |_| {
                        sender.input(TwilightMenuWidgetInput::SetMode("schedule"));
                    },
                },
            },

            // Temperature slider — previews live (pins until Reset).
            gtk::Label {
                add_css_class: "twilight-section-label",
                set_halign: gtk::Align::Start,
                set_label: "Temperature",
            },
            gtk::Scale {
                add_css_class: "twilight-temp-scale",
                set_orientation: gtk::Orientation::Horizontal,
                set_hexpand: true,
                // Set range/value BEFORE wiring the handler so seeding
                // the slider on open doesn't fire a spurious preview.
                set_range: (1000.0, 6500.0),
                set_increments: (100.0, 500.0),
                set_value: 4000.0,
                set_draw_value: true,
                set_value_pos: gtk::PositionType::Right,
                connect_value_changed[sender] => move |s| {
                    sender.input(TwilightMenuWidgetInput::TempChanged(s.value() as u32));
                },
            },

            gtk::Button {
                add_css_class: "ok-button-surface",
                set_label: "Resume schedule",
                set_tooltip_text: Some("Drop any preview and follow the schedule again"),
                connect_clicked[sender] => move |_| {
                    sender.input(TwilightMenuWidgetInput::Reset);
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        // Poll `mctl twilight status` while the panel is open so the
        // readout + selected mode track the schedule and our actions.
        sender.command(|out, shutdown| async move {
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            let mut first = true;
            loop {
                let delay = if first {
                    Duration::from_millis(50)
                } else {
                    POLL
                };
                first = false;
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    _ = tokio::time::sleep(delay) => {}
                }
                if let Some(s) = twilight::probe().await {
                    let _ = out.send(TwilightMenuWidgetCommandOutput::Polled(s));
                }
            }
        });

        let model = TwilightMenuWidgetModel {
            status: TwilightStatus::default(),
            temp_debounce: None,
        };
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            TwilightMenuWidgetInput::Refresh(s) => {
                self.status = s;
            }
            TwilightMenuWidgetInput::Toggle => {
                let on = !self.status.enabled;
                self.status.enabled = on;
                twilight::run(vec![
                    "twilight".into(),
                    "set".into(),
                    format!("enabled={}", if on { 1 } else { 0 }),
                ]);
            }
            TwilightMenuWidgetInput::SetMode(mode) => {
                self.status.mode = mode.to_string();
                twilight::run(vec!["twilight".into(), "set".into(), format!("mode={mode}")]);
            }
            TwilightMenuWidgetInput::TempChanged(k) => {
                if let Some(h) = self.temp_debounce.take() {
                    h.abort();
                }
                let sender_clone = sender.clone();
                self.temp_debounce = Some(glib::spawn_future_local(async move {
                    glib::timeout_future(Duration::from_millis(TEMP_DEBOUNCE_MS)).await;
                    sender_clone.input(TwilightMenuWidgetInput::TempCommit(k));
                }));
            }
            TwilightMenuWidgetInput::TempCommit(k) => {
                self.temp_debounce = None;
                twilight::run(vec!["twilight".into(), "preview".into(), k.to_string()]);
            }
            TwilightMenuWidgetInput::Reset => {
                twilight::run(vec!["twilight".into(), "reset".into()]);
            }
        }
    }

    fn update_cmd(
        &mut self,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            TwilightMenuWidgetCommandOutput::Polled(s) => {
                sender.input(TwilightMenuWidgetInput::Refresh(s));
            }
        }
    }
}

/// CSS classes for a mode button — `selected` when it's the active mode.
fn mode_classes(active: &str, this: &str) -> Vec<&'static str> {
    if active == this {
        vec!["ok-button-surface", "twilight-mode-button", "selected"]
    } else {
        vec!["ok-button-surface", "twilight-mode-button"]
    }
}

/// "4200 K · Night · Schedule" — non-empty parts joined with " · ".
fn status_line(s: &TwilightStatus) -> String {
    if !s.enabled {
        return "Off — colours unfiltered".to_string();
    }
    let mut parts: Vec<String> = Vec::new();
    if let Some(k) = s.current_temp_k {
        parts.push(format!("{k} K"));
    }
    let phase = s.phase_label();
    if !phase.is_empty() {
        parts.push(phase.to_string());
    }
    parts.push(mode_label(&s.mode).to_string());
    parts.join(" · ")
}

fn mode_label(mode: &str) -> &'static str {
    match mode {
        "geo" => "Auto",
        "manual" => "Manual",
        "static" => "Static",
        "schedule" => "Schedule",
        _ => "—",
    }
}
