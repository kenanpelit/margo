//! Overview-tab ThinkPad fan row: current level + RPM from
//! `/proc/acpi/ibm/fan`, and manual levels (Auto · 0–7 · Max) when
//! thinkpad_acpi allows it. With `fan_control=0` (the default) the row
//! shows a calm hint instead of controls; on a non-ThinkPad it is hidden.
//! Never fails: every read / write error becomes a status line.

use super::fan::{FAN_CONTROL_HINT, FAN_PATH, FanControl, FanLevel, FanState, pkexec_argv, probe};
use super::ui;
use crate::menus::reveal_poll::RevealPoll;
use relm4::gtk::prelude::{BoxExt, ButtonExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_secs(2);

pub(crate) struct FanSectionModel {
    poll: RevealPoll,
    state: Option<FanState>,
    mode: FanControl,
    status: String,
    level_buttons: Vec<(FanLevel, gtk::Button)>,
}

#[derive(Debug)]
pub(crate) enum FanSectionInput {
    SetActive(bool),
    Poll,
    Set(FanLevel),
}

#[relm4::component(pub(crate))]
impl Component for FanSectionModel {
    type CommandOutput = Result<(), String>;
    type Input = FanSectionInput;
    type Output = ();
    type Init = ();

    view! {
        #[root]
        gtk::Box {
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 8,
            #[watch]
            set_visible: model.mode != FanControl::Absent,

            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 10,
                gtk::Label {
                    add_css_class: "cpu-dashboard-section-label",
                    set_label: "THINKPAD FAN",
                    set_hexpand: true,
                    set_xalign: 0.0,
                },
                gtk::Label {
                    add_css_class: "cpu-dashboard-stat-value",
                    #[watch]
                    set_label: &model.summary(),
                },
            },

            #[local_ref]
            level_row -> gtk::Box {
                add_css_class: "cpu-dashboard-sort-row",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 4,
                set_homogeneous: true,
                #[watch]
                set_visible: matches!(model.mode, FanControl::Direct | FanControl::Pkexec),
            },

            gtk::Label {
                set_css_classes: &["label-small", "dim-label"],
                set_xalign: 0.0,
                set_wrap: true,
                set_selectable: true,
                #[watch]
                set_visible: model.mode == FanControl::Disabled,
                set_label: FAN_CONTROL_HINT,
            },

            gtk::Label {
                set_css_classes: &["label-small", "dim-label"],
                set_xalign: 0.0,
                set_wrap: true,
                #[watch]
                set_visible: !model.status.is_empty(),
                #[watch]
                set_label: &model.status,
            },
        }
    }

    fn init(_: (), root: Self::Root, sender: ComponentSender<Self>) -> ComponentParts<Self> {
        let s = sender.clone();
        let poll = RevealPoll::new(POLL_INTERVAL, move || {
            s.input_sender().send(FanSectionInput::Poll).is_ok()
        });
        poll.set_revealed(false);
        let (state, mode) = probe();
        let level_row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        let mut level_buttons = Vec::new();
        for level in FanLevel::all() {
            let b = ui::text_button(&level.label(), false);
            b.set_tooltip_text(Some(&match level {
                FanLevel::Auto => "Firmware-controlled (default)".to_string(),
                FanLevel::FullSpeed => "Full speed".to_string(),
                FanLevel::Level(n) => format!("Fixed level {n}"),
            }));
            let s = sender.clone();
            b.connect_clicked(move |_| s.input(FanSectionInput::Set(level)));
            level_row.append(&b);
            level_buttons.push((level, b));
        }
        let model = FanSectionModel {
            poll,
            state,
            mode,
            status: String::new(),
            level_buttons,
        };
        let widgets = view_output!();
        model.sync_buttons();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match msg {
            FanSectionInput::SetActive(on) => {
                self.poll
                    .set_revealed(on && self.mode != FanControl::Absent);
                if on {
                    sender.input(FanSectionInput::Poll);
                }
            }
            FanSectionInput::Poll => {
                let (state, mode) = probe();
                self.state = state;
                self.mode = mode;
                self.sync_buttons();
            }
            FanSectionInput::Set(level) => match self.mode {
                FanControl::Direct => {
                    self.status = match std::fs::write(FAN_PATH, level.command()) {
                        Ok(()) => String::new(),
                        Err(e) => format!("Could not set the fan level: {e}"),
                    };
                    sender.input(FanSectionInput::Poll);
                }
                FanControl::Pkexec => {
                    self.status = "Waiting for authorization…".to_string();
                    sender.oneshot_command(async move {
                        let out = tokio::process::Command::new("pkexec")
                            .args(pkexec_argv(level))
                            .output()
                            .await
                            .map_err(|e| format!("pkexec unavailable: {e}"))?;
                        super::pkexec_outcome(
                            out.status.code(),
                            &String::from_utf8_lossy(&out.stderr),
                        )
                    });
                }
                FanControl::Disabled | FanControl::Absent => {}
            },
        }
    }

    fn update_cmd(
        &mut self,
        res: Result<(), String>,
        sender: ComponentSender<Self>,
        _r: &Self::Root,
    ) {
        self.status = res.err().unwrap_or_default();
        sender.input(FanSectionInput::Poll);
    }
}

impl FanSectionModel {
    fn summary(&self) -> String {
        match &self.state {
            Some(s) => {
                let level = if s.level.is_empty() {
                    "?"
                } else {
                    s.level.as_str()
                };
                match s.rpm {
                    Some(rpm) => format!("{level} · {rpm} RPM"),
                    None => level.to_string(),
                }
            }
            None => String::new(),
        }
    }

    /// Highlight the active level (`.selected`, the segmented-row rule).
    fn sync_buttons(&self) {
        let level = self.state.as_ref().map(|s| s.level.as_str()).unwrap_or("");
        for (l, b) in &self.level_buttons {
            if l.matches(level) {
                b.add_css_class("selected");
            } else {
                b.remove_css_class("selected");
            }
        }
    }
}
