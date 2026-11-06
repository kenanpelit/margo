//! Discord voice menu — the `discord_voice` pill's panel.
//!
//! Panel header (§12), then by state: a calm "not running" line with
//! *Launch Discord*, the one-time *Authorize Discord* prompt, or — in a
//! call — the channel and server name with a favourite star and the voice
//! latency, Mute / Deafen / Leave, your microphone input volume (0–100 %)
//! and a boxed list of participants (speaking dot, name, muted/deafened
//! glyphs, per-user volume 0–200 %). Out of a call, *Saved channels* lists
//! the favourite and recent voice channels: *Join* rejoins one through
//! RPC, or — while Discord isn't running — starts Discord on it. Participant
//! rows are kept per user id and updated in place, so a speaking flicker
//! never rebuilds the list under a slider being dragged.

use crate::discord::{self, DiscordCommand};
use crate::menus::menu_widgets::panel_header::PanelHeader;
use crate::menus::reveal_poll::{RevealGate, watch_stream};
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, DiscordConfigStoreFields};
use mshell_config::schema::menu_widgets::MenuWidget;
use mshell_utils::discord::{
    DiscordState, DiscordStatus, Latency, Participant, SavedChannel, channel_deep_link,
};
use reactive_graph::prelude::GetUntracked;
use relm4::gtk::prelude::{
    AccessibleExtManual, BoxExt, ButtonExt, Cast, ObjectExt, OrientableExt, RangeExt, ScaleExt,
    WidgetExt,
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
    /// What the saved-channel list was last built from (it is rebuilt
    /// only when this changes: at most ten rows).
    saved_key: Option<(Vec<(SavedChannel, bool)>, bool)>,
    mic_signal: Option<gtk::glib::SignalHandlerId>,
}

#[derive(Debug)]
pub(crate) enum DiscordVoiceMenuWidgetInput {
    Authorize,
    ToggleMute,
    ToggleDeaf,
    Leave,
    SetVolume(String, u32),
    SetInputVolume(u32),
    Launch,
    /// Join a saved channel (or open it in a starting Discord).
    Join(String),
    SetFavorite(String, bool),
    ToggleCurrentFavorite,
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
            #[name = "launch_button"]
            gtk::Button {
                add_css_class: "ok-button-primary",
                set_label: "Launch Discord",
                set_visible: false,
                connect_clicked => DiscordVoiceMenuWidgetInput::Launch,
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

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 8,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_spacing: 2,
                        set_hexpand: true,

                        #[name = "channel_label"]
                        gtk::Label {
                            add_css_class: "label-large-bold",
                            set_xalign: 0.0,
                            set_ellipsize: gtk::pango::EllipsizeMode::End,
                        },
                        gtk::Box {
                            set_orientation: gtk::Orientation::Horizontal,
                            set_spacing: 6,

                            #[name = "guild_label"]
                            gtk::Label {
                                set_css_classes: &["label-small", "dim-label"],
                                set_xalign: 0.0,
                                set_ellipsize: gtk::pango::EllipsizeMode::End,
                            },
                            #[name = "latency_icon"]
                            gtk::Image {
                                add_css_class: "discord-voice-latency",
                            },
                            #[name = "latency_label"]
                            gtk::Label {
                                set_css_classes: &["label-small", "discord-voice-latency"],
                                set_xalign: 0.0,
                            },
                        },
                    },
                    #[name = "favorite_button"]
                    gtk::Button {
                        add_css_class: "ok-button-flat",
                        set_valign: gtk::Align::Center,
                        connect_clicked => DiscordVoiceMenuWidgetInput::ToggleCurrentFavorite,
                    },
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

                #[name = "mic_row"]
                gtk::Box {
                    add_css_class: "discord-voice-mic",
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 8,

                    gtk::Image {
                        set_icon_name: Some("audio-input-microphone-symbolic"),
                    },
                    gtk::Label {
                        add_css_class: "label-small",
                        set_label: "Microphone",
                    },
                    #[name = "mic_scale"]
                    gtk::Scale {
                        add_css_class: "ok-progress-bar",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_range: (0.0, 100.0),
                        set_increments: (5.0, 10.0),
                        set_hexpand: true,
                        set_draw_value: false,
                        set_tooltip_text: Some("Your input volume in Discord (0–100 %)"),
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

            #[name = "saved_box"]
            gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 6,
                set_visible: false,

                gtk::Label {
                    add_css_class: "section-label-menu-widget",
                    set_label: "Saved channels",
                    set_xalign: 0.0,
                },
                #[name = "saved_list"]
                gtk::Box {
                    set_css_classes: &["ok-boxed-list", "compact", "discord-voice-saved"],
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
        let mut model = DiscordVoiceMenuWidgetModel {
            state: DiscordState::default(),
            rows: HashMap::new(),
            saved_key: None,
            mic_signal: None,
        };
        let widgets = view_output!();
        crate::menus::list_fit::mark_menu_list(&widgets.fill_list, 360);
        mshell_common::a11y::name(&widgets.mic_scale, "Microphone input volume");
        let s = sender.clone();
        model.mic_signal = Some(widgets.mic_scale.connect_value_changed(move |sc| {
            s.input(DiscordVoiceMenuWidgetInput::SetInputVolume(
                sc.value().round() as u32,
            ));
        }));
        sync_static(&model.state, &widgets);
        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {
        let binary = || config_manager().config().discord().binary().get_untracked();
        discord::send(match msg {
            DiscordVoiceMenuWidgetInput::Launch => {
                discord::launch_or_toast(&binary(), None);
                return;
            }
            DiscordVoiceMenuWidgetInput::Join(id)
                if self.state.status == DiscordStatus::NotRunning =>
            {
                let link = self
                    .state
                    .history
                    .find(&id)
                    .and_then(|c| channel_deep_link(&c.guild_id, &c.id));
                discord::launch_or_toast(&binary(), link.as_deref());
                return;
            }
            DiscordVoiceMenuWidgetInput::Join(id) => DiscordCommand::JoinChannel(id),
            DiscordVoiceMenuWidgetInput::SetFavorite(id, on) => DiscordCommand::SetFavorite(id, on),
            DiscordVoiceMenuWidgetInput::ToggleCurrentFavorite => {
                let Some(id) = self.state.channel_id.clone() else {
                    return;
                };
                let on = !self.state.history.is_favorite(&id);
                DiscordCommand::SetFavorite(id, on)
            }
            DiscordVoiceMenuWidgetInput::SetInputVolume(v) => DiscordCommand::SetInputVolume(v),
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
                self.sync_mic(widgets);
                self.sync_rows(widgets, &sender);
                self.sync_saved(widgets, &sender);
            }
        }
    }
}

impl DiscordVoiceMenuWidgetModel {
    fn sync_mic(&self, widgets: &DiscordVoiceMenuWidgetModelWidgets) {
        let Some(v) = self.state.input_volume else {
            widgets.mic_row.set_visible(false);
            return;
        };
        widgets.mic_row.set_visible(true);
        let scale = &widgets.mic_scale;
        if (scale.value() - f64::from(v)).abs() >= 1.0
            && let Some(sig) = &self.mic_signal
        {
            scale.block_signal(sig);
            scale.set_value(f64::from(v));
            scale.unblock_signal(sig);
        }
    }

    /// Saved channels, out of a call only: rejoin via RPC when ready, or
    /// open the channel in a starting Discord when it isn't running.
    fn sync_saved(
        &mut self,
        widgets: &DiscordVoiceMenuWidgetModelWidgets,
        sender: &ComponentSender<Self>,
    ) {
        let s = &self.state;
        let usable = match s.status {
            DiscordStatus::Ready => !s.in_voice(),
            DiscordStatus::NotRunning => true,
            DiscordStatus::NeedsAuth | DiscordStatus::Authorizing => false,
        };
        let entries: Vec<(SavedChannel, bool)> = s
            .history
            .entries()
            .into_iter()
            .map(|(c, f)| (c.clone(), f))
            .collect();
        widgets.saved_box.set_visible(usable && !entries.is_empty());
        if !usable {
            return;
        }
        let running = s.status == DiscordStatus::Ready;
        let key = (entries, running);
        if self.saved_key.as_ref() == Some(&key) {
            return;
        }
        let list = &widgets.saved_list;
        while let Some(child) = list.first_child() {
            list.remove(&child);
        }
        for (c, fav) in &key.0 {
            list.append(&saved_row(c, *fav, running, sender));
        }
        self.saved_key = Some(key);
    }

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

fn saved_row(
    c: &SavedChannel,
    favorite: bool,
    running: bool,
    sender: &ComponentSender<DiscordVoiceMenuWidgetModel>,
) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.add_css_class("discord-voice-saved-row");
    // Stars are changed through the live session (insensitive while
    // Discord isn't running).
    let star = gtk::Button::from_icon_name(if favorite {
        "starred-symbolic"
    } else {
        "non-starred-symbolic"
    });
    mshell_common::a11y::name(
        &star,
        &if favorite {
            format!("Remove {} from favourites", c.name)
        } else {
            format!("Add {} to favourites", c.name)
        },
    );
    star.add_css_class("ok-button-flat");
    if favorite {
        star.add_css_class("selected");
    }
    star.set_valign(gtk::Align::Center);
    star.set_sensitive(running);
    {
        let (s, id) = (sender.clone(), c.id.clone());
        star.connect_clicked(move |_| {
            s.input(DiscordVoiceMenuWidgetInput::SetFavorite(
                id.clone(),
                !favorite,
            ));
        });
    }
    row.append(&star);

    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    let name = gtk::Label::new(Some(&c.name));
    name.add_css_class("label-medium");
    name.set_xalign(0.0);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text.append(&name);
    let server = if c.guild_name.is_empty() {
        if c.guild_id.is_empty() {
            "Direct call"
        } else {
            "Server"
        }
    } else {
        c.guild_name.as_str()
    };
    let sub = gtk::Label::new(Some(server));
    sub.set_css_classes(&["label-small", "dim-label"]);
    sub.set_xalign(0.0);
    sub.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text.append(&sub);
    row.append(&text);

    let join = gtk::Button::with_label(if running { "Join" } else { "Open" });
    join.add_css_class("ok-button-surface");
    join.set_valign(gtk::Align::Center);
    join.set_tooltip_text(Some(if running {
        "Join this voice channel"
    } else {
        "Start Discord on this channel"
    }));
    {
        let (s, id) = (sender.clone(), c.id.clone());
        join.connect_clicked(move |_| s.input(DiscordVoiceMenuWidgetInput::Join(id.clone())));
    }
    row.append(&join);
    row
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
    w.launch_button.set_visible(
        s.status == DiscordStatus::NotRunning
            && discord::discord_program(
                &config_manager().config().discord().binary().get_untracked(),
            )
            .is_some(),
    );
    let guild = if s.guild_name.is_empty() && !s.guild_id.is_empty() {
        "Server"
    } else if s.guild_id.is_empty() {
        "Direct call"
    } else {
        s.guild_name.as_str()
    };
    w.guild_label.set_label(guild);
    let latency = s.latency();
    let (icon, tone) = match latency {
        Latency::Good(_) => ("network-cellular-signal-excellent-symbolic", "good"),
        Latency::Fair(_) => ("network-cellular-signal-ok-symbolic", "fair"),
        Latency::Poor(_) => ("network-cellular-signal-weak-symbolic", "poor"),
        Latency::NoRoute => ("network-cellular-offline-symbolic", "poor"),
        Latency::Connecting => ("network-cellular-acquiring-symbolic", "fair"),
        Latency::Disconnected => ("network-cellular-offline-symbolic", "idle"),
    };
    w.latency_icon.set_icon_name(Some(icon));
    w.latency_label.set_label(&latency.label());
    for widget in [
        w.latency_icon.upcast_ref::<gtk::Widget>(),
        w.latency_label.upcast_ref(),
    ] {
        for t in ["good", "fair", "poor", "idle"] {
            widget.remove_css_class(t);
        }
        widget.add_css_class(tone);
    }
    w.latency_label
        .set_tooltip_text(Some("Average ping to Discord's voice server"));
    let fav = s
        .channel_id
        .as_deref()
        .is_some_and(|id| s.history.is_favorite(id));
    w.favorite_button.set_icon_name(if fav {
        "starred-symbolic"
    } else {
        "non-starred-symbolic"
    });
    if fav {
        w.favorite_button.add_css_class("selected");
    } else {
        w.favorite_button.remove_css_class("selected");
    }
    mshell_common::a11y::name(
        &w.favorite_button,
        if fav {
            "Remove from favourites"
        } else {
            "Add to favourites"
        },
    );
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
