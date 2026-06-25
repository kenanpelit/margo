//! Audio-inhibit daemon — port of the DMS `audio-inhibit` plugin.
//!
//! Holds the idle inhibitor while any MPRIS media player is playing, and
//! restores the prior inhibitor state when playback stops — so a video /
//! music keeps the screen awake without the user toggling Keep Awake by
//! hand. Gated on the `idle.inhibit_while_media` setting (opt-in).
//!
//! This is a **headless** relm4 component (its root `gtk::Box` is never
//! shown): launch one instance at startup (see `mshell-core`'s `relm_app`)
//! and keep the `Controller` alive. It reuses the same media-watch
//! machinery as the media-player pill — `spawn_media_players_watcher`
//! (player list / active changes) plus a per-player `playback_state` watch
//! reset on every list change — so it reacts the instant any player
//! starts or stops.
//!
//! State model (deliberately simple — the DMS original notes that
//! detecting *external* inhibitor changes is unreliable, so we don't try):
//! when playback starts we remember `IdleInhibitor::get()` and enable the
//! inhibitor; when it stops we restore that remembered state. Turning the
//! setting off mid-playback releases our hold immediately.

use mshell_common::scoped_effects::EffectScope;
use mshell_common::{WatcherToken, watch_cancellable};
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, IdleStoreFields};
use mshell_idle::inhibitor::IdleInhibitor;
use mshell_services::media_service;
use mshell_utils::media::spawn_media_players_watcher;
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use wayle_media::types::PlaybackState;

pub struct AudioInhibitModel {
    /// The `idle.inhibit_while_media` setting.
    enabled: bool,
    /// `true` while *we* are holding the inhibitor on behalf of playback.
    audio_holds: bool,
    /// Inhibitor state captured when playback started, restored when it
    /// stops (so we only ever turn *off* what we turned on).
    restore_to: bool,
    /// Per-player `playback_state` subscriptions, reset on list changes.
    watcher_token: WatcherToken,
    _effects: EffectScope,
}

#[derive(Debug)]
pub enum AudioInhibitInput {
    /// The `idle.inhibit_while_media` setting changed.
    EnabledChanged(bool),
}

#[derive(Debug)]
pub enum AudioInhibitCmd {
    /// The player list (or active player) changed — re-subscribe to every
    /// player's `playback_state`, then reconcile.
    PlayersChanged,
    /// Some player's `playback_state` changed — reconcile only. Crucially we
    /// do **not** re-subscribe here: a `.watch()` emits its current value the
    /// instant it is subscribed, so re-subscribing on a playback change would
    /// re-emit → re-subscribe → spin forever and freeze the shell.
    PlaybackChanged,
}

pub struct AudioInhibitInit {}

#[relm4::component(pub)]
impl Component for AudioInhibitModel {
    type CommandOutput = AudioInhibitCmd;
    type Input = AudioInhibitInput;
    type Output = ();
    type Init = AudioInhibitInit;

    view! {
        // Headless — never attached to a surface.
        #[root]
        gtk::Box {}
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        spawn_media_players_watcher(
            &sender,
            || AudioInhibitCmd::PlayersChanged,
            || AudioInhibitCmd::PlayersChanged,
        );

        let mut effects = EffectScope::new();
        let sender_clone = sender.clone();
        effects.push(move |_| {
            let v = config_manager().config().idle().inhibit_while_media().get();
            sender_clone.input(AudioInhibitInput::EnabledChanged(v));
        });

        let mut model = AudioInhibitModel {
            enabled: config_manager()
                .config()
                .idle()
                .inhibit_while_media()
                .get_untracked(),
            audio_holds: false,
            restore_to: false,
            watcher_token: WatcherToken::new(),
            _effects: effects,
        };
        subscribe_playback(&sender, &mut model.watcher_token);

        let widgets = view_output!();
        let _ = root;
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            AudioInhibitInput::EnabledChanged(v) => {
                self.enabled = v;
                if v {
                    // Turned on — engage now if something is already playing.
                    self.reconcile();
                } else if self.audio_holds {
                    // Turned off while we hold the inhibitor — release it.
                    self.audio_holds = false;
                    if !self.restore_to {
                        spawn_disable();
                    }
                }
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
            AudioInhibitCmd::PlayersChanged => {
                subscribe_playback(&sender, &mut self.watcher_token);
                self.reconcile();
            }
            AudioInhibitCmd::PlaybackChanged => {
                self.reconcile();
            }
        }
    }
}

impl AudioInhibitModel {
    /// Engage / release the inhibitor to match current playback. Idempotent:
    /// the `audio_holds` gate means repeated calls while playing (or stopped)
    /// are no-ops.
    fn reconcile(&mut self) {
        let playing = any_playing();
        if playing && self.enabled && !self.audio_holds {
            self.restore_to = IdleInhibitor::global().get();
            self.audio_holds = true;
            if !self.restore_to {
                spawn_enable();
            }
        } else if !playing && self.audio_holds {
            self.audio_holds = false;
            if !self.restore_to {
                spawn_disable();
            }
        }
    }
}

/// Is any MPRIS player currently playing?
fn any_playing() -> bool {
    media_service()
        .player_list
        .get()
        .iter()
        .any(|p| p.playback_state.get() == PlaybackState::Playing)
}

/// Re-subscribe to every player's `playback_state` under a fresh token, so a
/// list change doesn't leave stale subscriptions behind.
fn subscribe_playback(
    sender: &ComponentSender<AudioInhibitModel>,
    watcher_token: &mut WatcherToken,
) {
    let token = watcher_token.reset();
    for player in media_service().player_list.get() {
        let playback_state = player.playback_state.clone();
        let t = token.clone();
        watch_cancellable!(sender, t, [playback_state.watch()], |out| {
            let _ = out.send(AudioInhibitCmd::PlaybackChanged);
        });
    }
}

fn spawn_enable() {
    relm4::spawn(async {
        let _ = IdleInhibitor::global().enable().await;
    });
}

fn spawn_disable() {
    relm4::spawn(async {
        IdleInhibitor::global().disable().await;
    });
}
