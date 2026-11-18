//! Downloads — bar pill for the media downloader ([`crate::downloads`]):
//! a download glyph and, while something downloads, the running job's
//! percent (plus how many wait, `45% +2`). Hover lists the queue; a click
//! opens the Downloads menu.

use crate::downloads::engine::Status;
use crate::downloads::{self, DownloadsState};
use relm4::gtk::Orientation;
use relm4::gtk::prelude::{BoxExt, ButtonExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, RelmWidgetExt, gtk};

pub(crate) struct DownloadsModel {
    label: String,
    tooltip: String,
    active: bool,
    _orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum DownloadsInput {
    Clicked,
}

#[derive(Debug)]
pub(crate) enum DownloadsOutput {
    Clicked,
}

pub(crate) struct DownloadsInit {
    pub(crate) orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum DownloadsCommandOutput {
    State(DownloadsState),
}

/// Pill label (empty while idle) and tooltip.
pub(crate) fn pill_text(s: &DownloadsState) -> (String, String) {
    let running = s.jobs.iter().find(|j| j.status == Status::Running);
    let waiting = s.jobs.iter().filter(|j| j.status == Status::Queued).count();
    let label = match (running, waiting) {
        (Some(j), 0) => format!("{:.0}%", j.progress * 100.0),
        (Some(j), n) => format!("{:.0}% +{n}", j.progress * 100.0),
        (None, 0) => String::new(),
        (None, n) => format!("+{n}"),
    };
    let mut tip = match s.active() {
        0 => "Downloads: nothing running".to_string(),
        n => format!("Downloads: {n} in the queue"),
    };
    for j in s.jobs.iter().filter(|j| !j.status.finished()).take(5) {
        let state = match j.status {
            Status::Running => format!("{:.0}%", j.progress * 100.0),
            _ => "waiting".to_string(),
        };
        tip.push_str(&format!("\n{state} · {}", j.label()));
    }
    if !s.missing.is_empty() {
        tip.push_str(&format!(
            "\n\nInstall {} to download",
            s.missing.join(" and ")
        ));
    }
    tip.push_str("\n\nClick: open the queue · launcher: dl <link>");
    (label, tip)
}

#[relm4::component(pub)]
impl Component for DownloadsModel {
    type CommandOutput = DownloadsCommandOutput;
    type Input = DownloadsInput;
    type Output = DownloadsOutput;
    type Init = DownloadsInit;

    view! {
        #[root]
        gtk::Box {
            set_css_classes: &["downloads-bar-widget", "ok-button-surface", "ok-bar-widget"],
            #[watch]
            set_class_active: ("active", model.active),
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
                    sender.input(DownloadsInput::Clicked);
                },

                gtk::Box {
                    set_orientation: Orientation::Horizontal,
                    set_spacing: 4,
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,

                    gtk::Image {
                        set_icon_name: Some("folder-download-symbolic"),
                    },
                    gtk::Label {
                        add_css_class: "downloads-bar-label",
                        #[watch]
                        set_label: &model.label,
                        #[watch]
                        set_visible: !model.label.is_empty()
                            && model._orientation == Orientation::Horizontal,
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
        let mut rx = downloads::subscribe();
        sender.command(|out, shutdown| async move {
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            loop {
                let state = rx.borrow_and_update().clone();
                let _ = out.send(DownloadsCommandOutput::State(state));
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    r = rx.changed() => if r.is_err() { break },
                }
            }
        });
        let model = DownloadsModel {
            label: String::new(),
            tooltip: String::new(),
            active: false,
            _orientation: params.orientation,
        };
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            DownloadsInput::Clicked => {
                let _ = sender.output(DownloadsOutput::Clicked);
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
            DownloadsCommandOutput::State(s) => {
                let (label, tooltip) = pill_text(&s);
                self.active = s.active() > 0;
                self.label = label;
                self.tooltip = tooltip;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::downloads::engine::{Mode, Queue};

    #[test]
    fn the_pill_shows_the_running_percent_and_the_waiting_count() {
        let mut q = Queue::default();
        let a = q.add("https://a.example/1", Mode::Video, Default::default());
        q.add("https://a.example/2", Mode::Audio, Default::default());
        let s = |q: &Queue| DownloadsState {
            jobs: q.jobs.clone(),
            missing: vec![],
        };
        assert_eq!(pill_text(&s(&q)).0, "+2");
        if let Some(j) = q.get_mut(a) {
            j.status = Status::Running;
            j.progress = 0.456;
        }
        let (label, tip) = pill_text(&s(&q));
        assert_eq!(label, "46% +1");
        assert!(tip.starts_with("Downloads: 2 in the queue\n46% · https://a.example/1"));
        assert_eq!(pill_text(&DownloadsState::default()).0, "");
    }
}
