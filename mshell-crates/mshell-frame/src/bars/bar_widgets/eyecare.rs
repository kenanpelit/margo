//! Eye Care — bar pill for the 20-20-20 break reminder
//! ([`crate::eyecare`]).
//!
//! * off: a dim crossed-out eye; click turns the reminder on;
//! * counting: an eye + the minutes until the next break (`18m`);
//! * away: a dim eye (the count is held and resets — you're resting);
//! * on a break: a primary-tinted eye + the seconds left (`15s`).
//!
//! Click takes a break now (or ends a running one); right-click restarts
//! the count. The pill sleeps until the reminder changes and otherwise
//! wakes only when its readout rolls over: once a minute while counting,
//! once a second during the (20 s) break.

use crate::bars::pill_classes::PillClasses;
use crate::eyecare::engine::Mode;
use crate::eyecare::{self, Command, Snapshot, ceil_secs};
use relm4::gtk::Orientation;
use relm4::gtk::prelude::{BoxExt, ButtonExt, GestureSingleExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::{Duration, Instant};

pub(crate) struct EyeCareModel {
    root: gtk::Box,
    icon: gtk::Image,
    label: gtk::Label,
}

#[derive(Debug)]
pub(crate) enum EyeCareInput {
    Clicked,
    Reset,
}

pub(crate) struct EyeCareInit {
    pub(crate) orientation: Orientation,
}

const STATE_CLASSES: &[&str] = &["off", "away", "on-break"];

#[relm4::component(pub)]
impl Component for EyeCareModel {
    type CommandOutput = ();
    type Input = EyeCareInput;
    type Output = ();
    type Init = EyeCareInit;

    view! {
        #[root]
        gtk::Box {
            set_css_classes: &["eyecare-bar-widget", "ok-button-surface", "ok-bar-widget"],
            set_hexpand: false,
            set_vexpand: false,

            #[name = "button"]
            gtk::Button {
                set_css_classes: &["ok-button-flat"],
                set_hexpand: true,
                set_vexpand: true,
                connect_clicked[sender] => move |_| sender.input(EyeCareInput::Clicked),

                gtk::Box {
                    set_spacing: 6,
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,
                    #[local_ref]
                    icon_widget -> gtk::Image {},
                    #[local_ref]
                    label_widget -> gtk::Label {
                        add_css_class: "eyecare-bar-time",
                    },
                },
            },
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let _ = params.orientation; // the readout is already compact
        let icon_widget = gtk::Image::new();
        let label_widget = gtk::Label::new(None);
        let model = EyeCareModel {
            root: root.clone(),
            icon: icon_widget.clone(),
            label: label_widget.clone(),
        };
        let widgets = view_output!();

        let gesture = gtk::GestureClick::new();
        gesture.set_button(gtk::gdk::BUTTON_SECONDARY);
        let reset_sender = sender.clone();
        gesture.connect_pressed(move |_, _, _, _| reset_sender.input(EyeCareInput::Reset));
        widgets.button.add_controller(gesture);

        refresh(&model);

        sender.command(|out, shutdown| async move {
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            let mut changes = eyecare::changes();
            loop {
                let p = *changes.borrow_and_update();
                let unit = if p.on_break {
                    Duration::from_secs(1)
                } else {
                    Duration::from_secs(60)
                };
                let tick = eyecare::next_readout_change(p.deadline, Instant::now(), unit);
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    changed = changes.changed() => {
                        if changed.is_err() {
                            break;
                        }
                    }
                    () = sleep_or_forever(tick) => {}
                }
                let _ = out.send(());
            }
        });

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {
        let snap = eyecare::snapshot();
        let cmd = match message {
            EyeCareInput::Clicked => click_command(&snap),
            EyeCareInput::Reset if snap.mode.is_some() => Command::Reset,
            EyeCareInput::Reset => return,
        };
        eyecare::run(cmd);
        refresh(self);
    }

    fn update_cmd(&mut self, _: Self::CommandOutput, _: ComponentSender<Self>, _: &Self::Root) {
        refresh(self);
    }
}

/// Left-click: turn on, take a break now, or end the running break.
fn click_command(snap: &Snapshot) -> Command {
    match snap.mode {
        None => Command::On,
        Some(Mode::Break { .. }) => Command::Skip,
        Some(_) => Command::Break,
    }
}

fn refresh(model: &EyeCareModel) {
    let snap = eyecare::snapshot();
    let (icon, text, classes) = look(&snap);
    model.icon.set_icon_name(Some(icon));
    model.label.set_label(&text);
    model.label.set_visible(!text.is_empty());
    model.root.set_state_classes(STATE_CLASSES, classes);
    model.root.set_tooltip_text(Some(&tooltip(&snap)));
}

/// `(icon, readout, state classes)` for a snapshot.
fn look(snap: &Snapshot) -> (&'static str, String, &'static [&'static str]) {
    match snap.mode {
        None => ("eye-off-symbolic", String::new(), &["off"]),
        Some(Mode::Away) => ("eye-symbolic", String::new(), &["away"]),
        Some(Mode::Active { left }) => (
            "eye-symbolic",
            format!("{}m", left.as_secs().div_ceil(60)),
            &[],
        ),
        Some(Mode::Break { left }) => (
            "eye-symbolic",
            format!("{}s", ceil_secs(left)),
            &["on-break"],
        ),
    }
}

fn tooltip(snap: &Snapshot) -> String {
    let s = snap.settings;
    let rule = format!(
        "Every {} min of screen time, look ~6 m away for {} s.",
        s.active.as_secs() / 60,
        s.break_len.as_secs()
    );
    match snap.mode {
        None => format!("Eye care is off\n{rule}\nClick: turn on"),
        Some(Mode::Away) => format!(
            "Eye care · you're away — that counts as a break\n{rule}\nClick: take a break now  ·  Right-click: restart the count"
        ),
        Some(Mode::Active { left }) => format!(
            "Eye care · next break in {} min\n{rule}\nClick: take a break now  ·  Right-click: restart the count",
            left.as_secs().div_ceil(60)
        ),
        Some(Mode::Break { left }) => format!(
            "Eye break · look about 6 m (20 ft) away · {} s left\nClick: end the break  ·  Right-click: restart the count",
            ceil_secs(left)
        ),
    }
}

async fn sleep_or_forever(delay: Option<Duration>) {
    match delay {
        Some(d) => tokio::time::sleep(d).await,
        None => std::future::pending().await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eyecare::engine::Settings;

    fn snap(mode: Option<Mode>) -> Snapshot {
        Snapshot {
            mode,
            settings: Settings::default(),
        }
    }

    #[test]
    fn each_mode_has_its_glyph_readout_and_tint() {
        assert_eq!(
            look(&snap(None)),
            ("eye-off-symbolic", String::new(), &["off"][..])
        );
        assert_eq!(look(&snap(Some(Mode::Away))).2, &["away"]);
        let active = look(&snap(Some(Mode::Active {
            left: Duration::from_secs(17 * 60 + 1),
        })));
        assert_eq!(active.1, "18m", "minutes round up");
        assert!(active.2.is_empty());
        let on_break = look(&snap(Some(Mode::Break {
            left: Duration::from_millis(14_200),
        })));
        assert_eq!(
            (on_break.1.as_str(), on_break.2),
            ("15s", &["on-break"][..])
        );
    }

    #[test]
    fn click_turns_on_starts_or_ends_a_break() {
        assert_eq!(click_command(&snap(None)), Command::On);
        assert_eq!(
            click_command(&snap(Some(Mode::Active {
                left: Duration::from_secs(60)
            }))),
            Command::Break
        );
        assert_eq!(click_command(&snap(Some(Mode::Away))), Command::Break);
        assert_eq!(
            click_command(&snap(Some(Mode::Break {
                left: Duration::from_secs(3)
            }))),
            Command::Skip
        );
    }
}
