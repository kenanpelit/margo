//! Discord voice — bar pill for the current Discord voice call.
//!
//! A call glyph that tints `--primary` (DESIGN.md §3) while you're in a
//! voice channel, with the channel name (+ how many people are speaking)
//! beside it; idle it's just the glyph. Left click opens the Discord
//! voice menu — or, with Settings → Widgets → Discord Voice → *Launch
//! Discord on click*, starts Discord while it isn't running. *Hide when
//! not in a call* hides the pill outside voice. State comes from the
//! shared RPC client in [`crate::discord`] (event-driven, no polling).

use crate::discord;
use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, DiscordConfigStoreFields};
use mshell_utils::discord::{DiscordState, DiscordStatus, pill_label};
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::Orientation;
use relm4::gtk::prelude::{BoxExt, ButtonExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, RelmWidgetExt, gtk};

pub(crate) struct DiscordVoiceModel {
    state: DiscordState,
    hide_when_disconnected: bool,
    _orientation: Orientation,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum DiscordVoiceInput {
    Clicked,
    HideWhenDisconnected(bool),
}

#[derive(Debug)]
pub(crate) enum DiscordVoiceOutput {
    Clicked,
}

pub(crate) struct DiscordVoiceInit {
    pub(crate) orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum DiscordVoiceCommandOutput {
    State(DiscordState),
}

#[relm4::component(pub)]
impl Component for DiscordVoiceModel {
    type CommandOutput = DiscordVoiceCommandOutput;
    type Input = DiscordVoiceInput;
    type Output = DiscordVoiceOutput;
    type Init = DiscordVoiceInit;

    view! {
        #[root]
        gtk::Box {
            set_css_classes: &["discord-voice-bar-widget", "ok-button-surface", "ok-bar-widget"],
            #[watch]
            set_visible: !model.hide_when_disconnected || model.state.in_voice(),
            #[watch]
            set_class_active: ("active", model.state.in_voice()),
            set_hexpand: model._orientation == Orientation::Vertical,
            set_vexpand: model._orientation == Orientation::Horizontal,
            set_halign: gtk::Align::Center,
            set_valign: gtk::Align::Center,
            set_has_tooltip: true,
            #[watch]
            set_tooltip_text: Some(&tooltip(&model.state)),

            gtk::Button {
                set_css_classes: &["ok-button-flat"],
                set_hexpand: false,
                set_vexpand: false,
                connect_clicked[sender] => move |_| {
                    sender.input(DiscordVoiceInput::Clicked);
                },

                gtk::Box {
                    set_orientation: Orientation::Horizontal,
                    set_spacing: 4,
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,

                    gtk::Image {
                        #[watch]
                        set_icon_name: Some(if model.state.in_voice() && model.state.self_mute {
                            "microphone-sensitivity-muted-symbolic"
                        } else {
                            "call-start-symbolic"
                        }),
                    },
                    gtk::Label {
                        add_css_class: "discord-voice-bar-label",
                        set_max_width_chars: 18,
                        set_ellipsize: gtk::pango::EllipsizeMode::End,
                        #[watch]
                        set_label: &pill_label(&model.state),
                        #[watch]
                        set_visible: model.state.in_voice()
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
        sender.command(|out, shutdown| async move {
            let mut rx = discord::subscribe();
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            loop {
                let _ = out.send(DiscordVoiceCommandOutput::State(
                    rx.borrow_and_update().clone(),
                ));
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    r = rx.changed() => if r.is_err() { break },
                }
            }
        });
        let mut effects = EffectScope::new();
        let s = sender.clone();
        effects.push(move |_| {
            s.input(DiscordVoiceInput::HideWhenDisconnected(
                config_manager()
                    .config()
                    .discord()
                    .hide_when_disconnected()
                    .get(),
            ));
        });
        let model = DiscordVoiceModel {
            state: DiscordState::default(),
            hide_when_disconnected: config_manager()
                .config()
                .discord()
                .hide_when_disconnected()
                .get_untracked(),
            _orientation: params.orientation,
            _effects: effects,
        };
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            DiscordVoiceInput::Clicked => {
                let cfg = config_manager().config().discord().get_untracked();
                if launches_on_click(&self.state, cfg.launch_on_click) {
                    discord::launch_or_toast(&cfg.binary, None);
                } else {
                    let _ = sender.output(DiscordVoiceOutput::Clicked);
                }
            }
            DiscordVoiceInput::HideWhenDisconnected(on) => self.hide_when_disconnected = on,
        }
    }

    fn update_cmd(
        &mut self,
        message: Self::CommandOutput,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            DiscordVoiceCommandOutput::State(s) => self.state = s,
        }
    }
}

/// A click starts Discord (instead of opening the menu) only when the
/// user asked for it and Discord isn't running.
fn launches_on_click(s: &DiscordState, launch_on_click: bool) -> bool {
    launch_on_click && s.status == DiscordStatus::NotRunning
}

fn tooltip(s: &DiscordState) -> String {
    let head = match s.status {
        DiscordStatus::NotRunning => "Discord isn't running".to_string(),
        DiscordStatus::NeedsAuth | DiscordStatus::Authorizing => {
            "Discord: authorize in the menu".to_string()
        }
        DiscordStatus::Ready if s.in_voice() => format!(
            "Discord: {} · {} in call · {}{}",
            s.channel_name,
            s.participants.len(),
            s.latency().label(),
            if s.self_mute { " · muted" } else { "" }
        ),
        DiscordStatus::Ready => "Discord: not in a voice channel".to_string(),
    };
    format!("{head}\n\nClick: voice controls")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_click_launches_only_when_asked_and_not_running() {
        let idle = DiscordState::default();
        assert!(!launches_on_click(&idle, false), "default: the menu opens");
        assert!(launches_on_click(&idle, true));
        let ready = DiscordState {
            status: DiscordStatus::Ready,
            ..Default::default()
        };
        assert!(!launches_on_click(&ready, true), "running: the menu opens");
    }
}
