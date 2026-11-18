//! Downloads menu — the panel for `MenuType::Downloads`: a link entry with
//! a format choice (Video / Audio), then the queue as boxed-list rows
//! (title or link, progress bar, speed · ETA or the error) with Cancel /
//! Open folder / Remove. The header pastes a link from the clipboard
//! (queued as video) and clears finished rows. When yt-dlp (or ffmpeg for
//! audio) is missing, a banner says what to install.

use crate::downloads::engine::{Job, Mode, Status};
use crate::downloads::{self, Command, DownloadsState};
use crate::menus::menu_widgets::panel_header::PanelHeader;
use crate::menus::reveal_poll::{RevealGate, watch_stream};
use mshell_config::schema::menu_widgets::MenuWidget;
use relm4::gtk::prelude::{
    BoxExt, ButtonExt, DisplayExt, EditableExt, EntryExt, OrientableExt, WidgetExt,
};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

pub(crate) struct DownloadsMenuWidgetModel {
    state: DownloadsState,
    list: gtk::Box,
    subtitle: gtk::Label,
}

#[derive(Debug)]
pub(crate) enum DownloadsMenuWidgetInput {
    Submit,
    Paste,
    Cmd(Command),
    OpenFolder(std::path::PathBuf),
}

#[derive(Debug)]
pub(crate) enum DownloadsMenuWidgetOutput {}

pub(crate) struct DownloadsMenuWidgetInit {}

#[derive(Debug)]
pub(crate) enum DownloadsMenuWidgetCommandOutput {
    State(DownloadsState),
}

#[relm4::component(pub(crate))]
impl Component for DownloadsMenuWidgetModel {
    type CommandOutput = DownloadsMenuWidgetCommandOutput;
    type Input = DownloadsMenuWidgetInput;
    type Output = DownloadsMenuWidgetOutput;
    type Init = DownloadsMenuWidgetInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "downloads-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 10,

            #[local_ref]
            header_root -> gtk::Box {},

            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 6,
                #[name = "entry"]
                gtk::Entry {
                    set_hexpand: true,
                    set_placeholder_text: Some("Paste a video or audio link…"),
                    connect_activate[sender] => move |_| {
                        sender.input(DownloadsMenuWidgetInput::Submit);
                    },
                },
                #[name = "format"]
                gtk::DropDown::from_strings(&["Video", "Audio"]) {
                    set_valign: gtk::Align::Center,
                    set_tooltip_text: Some("Video (best, or downloads.video_quality) or audio only (downloads.audio_format)"),
                },
                gtk::Button {
                    add_css_class: "ok-button-primary",
                    set_icon_name: "folder-download-symbolic",
                    set_tooltip_text: Some("Download"),
                    connect_clicked[sender] => move |_| {
                        sender.input(DownloadsMenuWidgetInput::Submit);
                    },
                },
            },

            #[name = "missing_label"]
            gtk::Label {
                set_css_classes: &["label-small-error"],
                set_xalign: 0.0,
                set_wrap: true,
                set_visible: false,
            },

            #[name = "fill_list"]
            gtk::ScrolledWindow {
                set_hscrollbar_policy: gtk::PolicyType::Never,
                set_min_content_height: 0,
                set_propagate_natural_height: true,

                #[local_ref]
                list -> gtk::Box {
                    set_css_classes: &["ok-boxed-list", "compact", "downloads-list"],
                    set_orientation: gtk::Orientation::Vertical,
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let (s_paste, s_clear) = (sender.clone(), sender.clone());
        let header = PanelHeader::new(
            "folder-download-symbolic",
            MenuWidget::Downloads.display_name(),
        )
        .with_subtitle()
        .action(
            "edit-paste-symbolic",
            "Download the link on the clipboard",
            move || {
                s_paste.input(DownloadsMenuWidgetInput::Paste);
            },
        )
        .action(
            "edit-clear-all-symbolic",
            "Clear finished downloads",
            move || {
                s_clear.input(DownloadsMenuWidgetInput::Cmd(Command::ClearFinished));
            },
        );
        let header_root = header.root().clone();
        let subtitle = header.subtitle().cloned().unwrap_or_default();
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);

        let open = RevealGate::follow_mapped(&root);
        sender.command(|out, shutdown| async move {
            let watch = open.stream_while_revealed(
                || watch_stream(downloads::subscribe()),
                |state| {
                    let _ = out.send(DownloadsMenuWidgetCommandOutput::State(state));
                },
            );
            tokio::select! {
                () = shutdown.wait() => {}
                () = watch => {}
            }
        });

        let model = DownloadsMenuWidgetModel {
            state: DownloadsState::default(),
            list: list.clone(),
            subtitle,
        };
        let widgets = view_output!();
        mshell_common::autofocus::mark(&widgets.entry);
        crate::menus::list_fit::mark_menu_list(&widgets.fill_list, 420);
        model.render(&widgets, &sender);
        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        msg: Self::Input,
        sender: ComponentSender<Self>,
        root: &Self::Root,
    ) {
        match msg {
            DownloadsMenuWidgetInput::Submit => {
                let mode = if widgets.format.selected() == 1 {
                    Mode::Audio
                } else {
                    Mode::Video
                };
                let text = widgets.entry.text().to_string();
                if downloads::add(&text, mode) {
                    widgets.entry.set_text("");
                    widgets.entry.remove_css_class("error");
                } else {
                    widgets.entry.add_css_class("error");
                }
            }
            DownloadsMenuWidgetInput::Paste => {
                let entry = widgets.entry.clone();
                root.display().clipboard().read_text_async(
                    None::<&relm4::gtk::gio::Cancellable>,
                    move |r| {
                        if let Ok(Some(t)) = r
                            && !downloads::add(&t, Mode::Video)
                        {
                            entry.set_text(&t);
                            entry.add_css_class("error");
                        }
                    },
                );
            }
            DownloadsMenuWidgetInput::Cmd(c) => downloads::send(c),
            DownloadsMenuWidgetInput::OpenFolder(p) => {
                if let Err(e) = mshell_process::desktop::open(&p) {
                    tracing::warn!(error = %e, "downloads: could not open the folder");
                }
            }
        }
        self.render(widgets, &sender);
    }

    fn update_cmd_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        msg: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match msg {
            DownloadsMenuWidgetCommandOutput::State(s) => {
                self.state = s;
                self.render(widgets, &sender);
            }
        }
    }
}

/// `2 downloading · 1 done` style summary.
pub(crate) fn summary(jobs: &[Job]) -> String {
    let active = jobs.iter().filter(|j| !j.status.finished()).count();
    let done = jobs.iter().filter(|j| j.status == Status::Done).count();
    match (active, done) {
        (0, 0) => "Nothing queued".to_string(),
        (a, 0) => format!("{a} in the queue"),
        (0, d) => format!("{d} done"),
        (a, d) => format!("{a} in the queue · {d} done"),
    }
}

impl DownloadsMenuWidgetModel {
    fn render(&self, widgets: &DownloadsMenuWidgetModelWidgets, sender: &ComponentSender<Self>) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        for j in self.state.jobs.iter().rev() {
            self.list.append(&row(j, sender));
        }
        self.list.set_visible(!self.state.jobs.is_empty());
        self.subtitle.set_label(&summary(&self.state.jobs));
        let missing = &self.state.missing;
        widgets.missing_label.set_visible(!missing.is_empty());
        if !missing.is_empty() {
            widgets.missing_label.set_label(&format!(
                "{} not found. Install it (e.g. `pacman -S yt-dlp ffmpeg`) and try again.",
                missing.join(" and ")
            ));
        }
    }
}

fn row(j: &Job, sender: &ComponentSender<DownloadsMenuWidgetModel>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.add_css_class("downloads-row");
    let icon = gtk::Image::from_icon_name(match j.mode {
        Mode::Video => "video-x-generic-symbolic",
        Mode::Audio => "audio-x-generic-symbolic",
    });
    icon.set_valign(gtk::Align::Center);
    row.append(&icon);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 3);
    text.set_hexpand(true);
    let name = gtk::Label::new(Some(j.label()));
    name.add_css_class("label-medium");
    name.set_xalign(0.0);
    name.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    name.set_tooltip_text(Some(&j.url));
    text.append(&name);
    if j.status == Status::Running {
        let bar = gtk::ProgressBar::new();
        bar.set_fraction(j.progress);
        text.append(&bar);
    }
    let detail = match &j.status {
        Status::Queued => "Waiting".to_string(),
        Status::Running => {
            let mut s = format!("{:.0}%", j.progress * 100.0);
            for x in [&j.speed, &j.eta] {
                if !x.is_empty() {
                    s.push_str(" · ");
                    s.push_str(x);
                }
            }
            s
        }
        Status::Done => j.file.as_ref().and_then(|f| f.file_name()).map_or_else(
            || "Done".to_string(),
            |f| format!("Done · {}", f.to_string_lossy()),
        ),
        Status::Failed(e) => format!("Failed · {e}"),
        Status::Cancelled => "Cancelled".to_string(),
    };
    let d = gtk::Label::new(Some(&detail));
    d.set_css_classes(if matches!(j.status, Status::Failed(_)) {
        &["label-small-error"]
    } else {
        &["label-small", "dim-label"]
    });
    d.set_xalign(0.0);
    d.set_wrap(true);
    text.append(&d);
    row.append(&text);

    let button = |icon: &str, tip: &str, msg: DownloadsMenuWidgetInput| {
        let b = gtk::Button::from_icon_name(icon);
        b.add_css_class("ok-button-flat");
        b.set_valign(gtk::Align::Center);
        b.set_tooltip_text(Some(tip));
        let s = sender.clone();
        let msg = std::cell::RefCell::new(Some(msg));
        b.connect_clicked(move |_| {
            if let Some(m) = msg.borrow_mut().take() {
                s.input(m);
            }
        });
        b
    };
    let id = j.id;
    if j.status.finished() {
        let dir = j
            .file
            .as_ref()
            .and_then(|f| f.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| j.dir.clone());
        row.append(&button(
            "folder-open-symbolic",
            "Open the folder",
            DownloadsMenuWidgetInput::OpenFolder(dir),
        ));
        row.append(&button(
            "window-close-symbolic",
            "Remove from the list",
            DownloadsMenuWidgetInput::Cmd(Command::Remove(id)),
        ));
    } else {
        row.append(&button(
            "process-stop-symbolic",
            "Cancel",
            DownloadsMenuWidgetInput::Cmd(Command::Cancel(id)),
        ));
    }
    row
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::downloads::engine::Queue;

    #[test]
    fn summaries_count_active_and_done() {
        let mut q = Queue::default();
        assert_eq!(summary(&q.jobs), "Nothing queued");
        let a = q.add("https://a.example/1", Mode::Video, Default::default());
        q.add("https://a.example/2", Mode::Video, Default::default());
        assert_eq!(summary(&q.jobs), "2 in the queue");
        if let Some(j) = q.get_mut(a) {
            j.status = Status::Done;
        }
        assert_eq!(summary(&q.jobs), "1 in the queue · 1 done");
    }
}
