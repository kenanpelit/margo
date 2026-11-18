//! Settings → Widgets → Downloads (`config.downloads`): where yt-dlp saves
//! video and audio, the audio format and video quality, playlists, extra
//! yt-dlp arguments and notifications. Composed with the generic per-menu
//! geometry page. Rows are built in code: every one is a label pair and
//! one control writing its key on change (entries on Enter / focus out).

use crate::github_settings::row_text;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    AUDIO_FORMATS, ConfigStoreFields, DownloadsConfig, VIDEO_QUALITIES,
};
use reactive_graph::prelude::GetUntracked;
use relm4::gtk::glib;
use relm4::gtk::prelude::{BoxExt, EditableExt, EntryExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

pub(crate) struct DownloadsSettingsModel {}

#[derive(Debug)]
pub(crate) enum DownloadsSettingsInput {}

#[derive(Debug)]
pub(crate) enum DownloadsSettingsOutput {}

pub(crate) struct DownloadsSettingsInit {}

fn cfg() -> DownloadsConfig {
    config_manager().config().downloads().get_untracked()
}

fn action_row(title: &str, desc: &str, control: &impl glib::object::IsA<gtk::Widget>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 20);
    row.add_css_class("action-row");
    row.append(&row_text(title, desc));
    control.set_valign(gtk::Align::Center);
    row.append(control);
    row
}

fn entry_row(
    title: &str,
    desc: &str,
    placeholder: &str,
    value: String,
    write: fn(&mut DownloadsConfig, String),
) -> gtk::Box {
    let e = gtk::Entry::new();
    e.set_width_request(220);
    e.set_placeholder_text(Some(placeholder));
    e.set_text(&value);
    mshell_common::a11y::name(&e, title);
    let commit = move |e: &gtk::Entry| {
        let text = e.text().trim().to_string();
        config_manager().update_config(move |c| write(&mut c.downloads, text.clone()));
    };
    e.connect_activate(commit);
    let focus = gtk::EventControllerFocus::new();
    let e2 = e.clone();
    focus.connect_leave(move |_| commit(&e2));
    e.add_controller(focus);
    action_row(title, desc, &e)
}

fn choice_row(
    title: &str,
    desc: &str,
    choices: &'static [&'static str],
    value: &str,
    write: fn(&mut DownloadsConfig, String),
) -> gtk::Box {
    let d = gtk::DropDown::from_strings(choices);
    d.set_selected(choices.iter().position(|c| *c == value).unwrap_or(0) as u32);
    d.connect_selected_notify(move |d| {
        if let Some(v) = choices.get(d.selected() as usize) {
            config_manager().update_config(move |c| write(&mut c.downloads, v.to_string()));
        }
    });
    action_row(title, desc, &d)
}

fn switch_row(
    title: &str,
    desc: &str,
    value: bool,
    write: fn(&mut DownloadsConfig, bool),
) -> gtk::Box {
    let s = gtk::Switch::new();
    s.set_active(value);
    s.connect_state_set(move |_, v| {
        config_manager().update_config(move |c| write(&mut c.downloads, v));
        glib::Propagation::Proceed
    });
    action_row(title, desc, &s)
}

#[relm4::component(pub)]
impl Component for DownloadsSettingsModel {
    type CommandOutput = ();
    type Input = DownloadsSettingsInput;
    type Output = DownloadsSettingsOutput;
    type Init = DownloadsSettingsInit;

    view! {
        #[root]
        gtk::ScrolledWindow {
            set_vscrollbar_policy: gtk::PolicyType::Automatic,
            set_hscrollbar_policy: gtk::PolicyType::Never,
            set_propagate_natural_height: false,
            set_propagate_natural_width: false,
            set_hexpand: true,
            set_vexpand: true,

            gtk::Box {
                add_css_class: "settings-page",
                set_orientation: gtk::Orientation::Vertical,
                set_hexpand: true,
                set_spacing: 16,

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Downloads (yt-dlp)",
                    set_halign: gtk::Align::Start,
                },
                gtk::Label {
                    set_css_classes: &["label-small", "dim-label"],
                    set_label: "Queue links from the Downloads menu, the launcher (dl <link>) or a YouTube video's actions (yt, then Ctrl+Enter). Needs yt-dlp, and ffmpeg for audio and for merging the best video and audio; mshell does not install them.",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                },
                #[name = "rows"]
                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,
                },
            }
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        _sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = DownloadsSettingsModel {};
        let widgets = view_output!();
        let c = cfg();
        let rows = &widgets.rows;
        rows.append(&entry_row(
            "Video folder",
            "Blank: your XDG videos folder (~/Videos). ~ is expanded.",
            "~/Videos",
            c.video_dir.clone(),
            |c, v| c.video_dir = v,
        ));
        rows.append(&entry_row(
            "Audio folder",
            "Blank: your XDG music folder (~/Music).",
            "~/Music",
            c.audio_dir.clone(),
            |c, v| c.audio_dir = v,
        ));
        rows.append(&choice_row(
            "Audio format",
            "What an audio download is converted to (best: the site's own, no re-encode).",
            AUDIO_FORMATS,
            &c.audio_format,
            |c, v| c.audio_format = v,
        ));
        rows.append(&choice_row(
            "Video quality",
            "Highest video height preferred.",
            VIDEO_QUALITIES,
            &c.video_quality,
            |c, v| c.video_quality = v,
        ));
        rows.append(&switch_row(
            "Whole playlists",
            "When a link names a playlist, download all of it (off: only the linked video).",
            c.playlist,
            |c, v| c.playlist = v,
        ));
        rows.append(&switch_row(
            "Notify when done",
            "A notification when a download finishes or fails.",
            c.notify,
            |c, v| c.notify = v,
        ));
        rows.append(&entry_row(
            "Extra yt-dlp arguments",
            "Added last, split on spaces, e.g. --cookies-from-browser firefox.",
            "",
            c.extra_args.clone(),
            |c, v| c.extra_args = v,
        ));
        ComponentParts { model, widgets }
    }
}
