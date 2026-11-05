//! Discord voice menu — the `discord_voice` pill's panel.
//!
//! Panel header (§12), then by state: a calm "not running" line, the
//! one-time *Authorize Discord* prompt, or — in a call — the channel name,
//! Mute / Deafen / Leave, and a boxed list of participants (speaking dot,
//! name, muted/deafened glyphs, per-user volume 0–200 %). Participant rows
//! are kept per user id and updated in place, so a speaking flicker never
//! rebuilds the list under a slider being dragged.

use crate::discord::{self, DiscordCommand};
use crate::menus::menu_widgets::panel_header::PanelHeader;
use crate::menus::reveal_poll::{RevealGate, watch_stream};
use mshell_config::schema::menu_widgets::MenuWidget;
use mshell_utils::discord::{DiscordState, DiscordStatus, Participant};
use relm4::gtk::prelude::{
    AccessibleExtManual, BoxExt, ButtonExt, ObjectExt, OrientableExt, RangeExt, ScaleExt, WidgetExt,
};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::collections::HashMap;

struct ParticipantRow {
    row: gtk::Box,
    dot: gtk::Image,
    name: gtk::Label,
    state_icon: gtk::Image,
    scale: Option<(gtk::Scale, gtk::glib::SignalHandlerId)>,
}

pub(crate) struct DiscordVoiceMenuWidgetModel {
    state: DiscordState,
    rows: HashMap<String, ParticipantRow>,
}

#[derive(Debug)]
pub(crate) enum DiscordVoiceMenuWidgetInput {
    Authorize,
    ToggleMute,
    ToggleDeaf,
    Leave,
    SetVolume(String, u32),
}

pub(crate) struct DiscordVoiceMenuWidgetInit {}

#[derive(Debug)]
pub(crate) enum DiscordVoiceMenuWidgetCommandOutput {
    State(DiscordState),
}

#[relm4::component(pub(crate))]
impl Component for DiscordVoiceMenuWidgetModel {
    type CommandOutput = DiscordVoiceMenuWidgetCommandOutput;
    type Input = DiscordVoiceMenuWidgetInput;
    type Output = ();
    type Init = DiscordVoiceMenuWidgetInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "discord-voice-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 10,

            #[local_ref]
            header_root -> gtk::Box {},

            // Status / empty / auth line (DESIGN §17 tones: dim, neutral).
            #[name = "status_label"]
            gtk::Label {
                set_css_classes: &["label-small", "dim-label"],
                set_xalign: 0.0,
                set_wrap: true,
            },
            #[name = "authorize_button"]
            gtk::Button {
                add_css_class: "ok-button-primary",
                set_label: "Authorize Discord",
                set_tooltip_text: Some("Discord shows a consent prompt (voice read/write only)"),
                connect_clicked => DiscordVoiceMenuWidgetInput::Authorize,
            },

            #[name = "call_box"]
            gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 10,

                #[name = "channel_label"]
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_xalign: 0.0,
                    set_ellipsize: gtk::pango::EllipsizeMode::End,
                },

                gtk::Box {
                    add_css_class: "discord-voice-actions",
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 8,
                    set_homogeneous: true,

                    #[name = "mute_button"]
                    gtk::Button {
                        add_css_class: "ok-button-surface",
                        connect_clicked => DiscordVoiceMenuWidgetInput::ToggleMute,
                    },
                    #[name = "deaf_button"]
                    gtk::Button {
                        add_css_class: "ok-button-surface",
                        connect_clicked => DiscordVoiceMenuWidgetInput::ToggleDeaf,
                    },
                    gtk::Button {
                        add_css_class: "ok-button-surface",
                        add_css_class: "discord-voice-leave",
                        set_label: "Leave",
                        set_tooltip_text: Some("Disconnect from the voice channel"),
                        connect_clicked => DiscordVoiceMenuWidgetInput::Leave,
                    },
                },

                gtk::Label {
                    add_css_class: "section-label-menu-widget",
                    set_label: "Participants",
                    set_xalign: 0.0,
                },
                #[name = "fill_list"]
                gtk::ScrolledWindow {
                    set_min_content_height: 0,
                    set_max_content_height: 360,
                    set_hscrollbar_policy: gtk::PolicyType::Never,
                    set_propagate_natural_height: true,

                    #[name = "participants_box"]
                    gtk::Box {
                        set_css_classes: &["ok-boxed-list", "compact", "discord-voice-participants"],
                        set_orientation: gtk::Orientation::Vertical,
                    },
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        // Subscribed only while the menu is open, so a closed menu doesn't
        // keep the Discord client probing for its socket.
        let open = RevealGate::follow_mapped(&root);
        sender.command(|out, shutdown| async move {
            let watch = open.stream_while_revealed(
                || watch_stream(discord::subscribe()),
                |state| {
                    let _ = out.send(DiscordVoiceMenuWidgetCommandOutput::State(state));
                },
            );
            tokio::select! {
                () = shutdown.wait() => {}
                () = watch => {}
            }
        });
        let header = PanelHeader::new(
            "call-start-symbolic",
            MenuWidget::DiscordVoice.display_name(),
        );
        let header_root = header.root().clone();
        let model = DiscordVoiceMenuWidgetModel {
            state: DiscordState::default(),
            rows: HashMap::new(),
        };
        let widgets = view_output!();
        crate::menus::list_fit::mark_menu_list(&widgets.fill_list, 360);
        sync_static(&model.state, &widgets);
        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {
        discord::send(match msg {
            DiscordVoiceMenuWidgetInput::Authorize => DiscordCommand::Authorize,
            DiscordVoiceMenuWidgetInput::ToggleMute => {
                DiscordCommand::SetMute(!self.state.self_mute)
            }
            DiscordVoiceMenuWidgetInput::ToggleDeaf => {
                DiscordCommand::SetDeaf(!self.state.self_deaf)
            }
            DiscordVoiceMenuWidgetInput::Leave => DiscordCommand::Leave,
            DiscordVoiceMenuWidgetInput::SetVolume(id, v) => DiscordCommand::SetUserVolume(id, v),
        });
    }

    fn update_cmd_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        msg: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match msg {
            DiscordVoiceMenuWidgetCommandOutput::State(s) => {
                self.state = s;
                sync_static(&self.state, widgets);
                self.sync_rows(widgets, &sender);
            }
        }
    }
}

impl DiscordVoiceMenuWidgetModel {
    fn sync_rows(
        &mut self,
        widgets: &DiscordVoiceMenuWidgetModelWidgets,
        sender: &ComponentSender<Self>,
    ) {
        let list = &widgets.participants_box;
        let wanted: Vec<Participant> = self
            .state
            .sorted_participants()
            .into_iter()
            .cloned()
            .collect();
        self.rows.retain(|id, r| {
            let keep = wanted.iter().any(|p| &p.id == id);
            if !keep {
                list.remove(&r.row);
            }
            keep
        });
        let me = self.state.user_id.clone();
        let mut prev: Option<gtk::Box> = None;
        for p in &wanted {
            let row = self
                .rows
                .entry(p.id.clone())
                .or_insert_with(|| build_row(p, p.id != me, sender));
            if row.row.parent().is_none() {
                list.append(&row.row);
            }
            // Keep the sorted order (you first, then by name).
            list.reorder_child_after(&row.row, prev.as_ref());
            prev = Some(row.row.clone());
            update_row(row, p);
        }
        list.set_visible(!wanted.is_empty());
    }
}

fn build_row(
    p: &Participant,
    with_volume: bool,
    sender: &ComponentSender<DiscordVoiceMenuWidgetModel>,
) -> ParticipantRow {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.add_css_class("discord-voice-participant");
    let dot = gtk::Image::from_icon_name("media-record-symbolic");
    dot.add_css_class("discord-voice-speaking-dot");
    dot.set_valign(gtk::Align::Center);
    let name = gtk::Label::new(None);
    name.set_xalign(0.0);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    name.set_hexpand(!with_volume);
    name.set_width_chars(10);
    let state_icon = gtk::Image::new();
    state_icon.add_css_class("dim-label");
    row.append(&dot);
    row.append(&name);
    row.append(&state_icon);
    let scale = with_volume.then(|| {
        let s = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 200.0, 5.0);
        s.add_css_class("ok-progress-bar");
        s.set_hexpand(true);
        s.set_draw_value(false);
        s.add_mark(100.0, gtk::PositionType::Bottom, None);
        s.set_tooltip_text(Some("Volume for you only (0–200 %)"));
        s.update_property(&[gtk::accessible::Property::Label(&format!(
            "Volume for {}",
            p.name
        ))]);
        s.set_value(f64::from(p.volume));
        let id = p.id.clone();
        let snd = sender.clone();
        let sig = s.connect_value_changed(move |s| {
            snd.input(DiscordVoiceMenuWidgetInput::SetVolume(
                id.clone(),
                s.value().round() as u32,
            ));
        });
        row.append(&s);
        (s, sig)
    });
    ParticipantRow {
        row,
        dot,
        name,
        state_icon,
        scale,
    }
}

fn update_row(r: &ParticipantRow, p: &Participant) {
    r.name.set_label(&p.name);
    if p.speaking {
        r.dot.add_css_class("speaking");
    } else {
        r.dot.remove_css_class("speaking");
    }
    let icon = if p.deafened {
        Some("audio-volume-muted-symbolic")
    } else if p.muted {
        Some("microphone-sensitivity-muted-symbolic")
    } else {
        None
    };
    r.state_icon.set_icon_name(icon);
    r.state_icon.set_visible(icon.is_some());
    if let Some((s, sig)) = &r.scale
        && (s.value() - f64::from(p.volume)).abs() >= 1.0
    {
        s.block_signal(sig);
        s.set_value(f64::from(p.volume));
        s.unblock_signal(sig);
        s.set_tooltip_text(Some(&format!("{} % (for you only)", p.volume)));
    }
}

fn sync_static(s: &DiscordState, w: &DiscordVoiceMenuWidgetModelWidgets) {
    let status = match s.status {
        DiscordStatus::NotRunning => "Discord isn't running (desktop app needed)".to_string(),
        DiscordStatus::NeedsAuth if !s.message.is_empty() => s.message.clone(),
        DiscordStatus::NeedsAuth => {
            "Allow mshell to read and control your voice state — once".to_string()
        }
        DiscordStatus::Authorizing => "Approve the prompt in Discord…".to_string(),
        DiscordStatus::Ready if !s.in_voice() => "Not in a voice channel".to_string(),
        DiscordStatus::Ready => s.message.clone(),
    };
    w.status_label.set_label(&status);
    w.status_label.set_visible(!status.is_empty());
    w.authorize_button
        .set_visible(s.status == DiscordStatus::NeedsAuth);
    w.call_box
        .set_visible(s.status == DiscordStatus::Ready && s.in_voice());
    w.channel_label.set_label(&s.channel_name);
    w.mute_button
        .set_label(if s.self_mute { "Unmute" } else { "Mute" });
    w.deaf_button
        .set_label(if s.self_deaf { "Undeafen" } else { "Deafen" });
    for (b, on) in [(&w.mute_button, s.self_mute), (&w.deaf_button, s.self_deaf)] {
        if on {
            b.add_css_class("selected");
        } else {
            b.remove_css_class("selected");
        }
    }
}
