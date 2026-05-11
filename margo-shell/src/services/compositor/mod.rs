//! Compositor service — margo only.
//!
//! Single backend: margo. The original ashell multi-backend
//! detection has been collapsed; this module's job is now just
//! "spin up the margo `run_listener`, expose the broadcaster as an
//! iced `Subscription`".

pub mod margo;
pub mod types;

pub use self::types::{
    CompositorCommand, CompositorEvent, CompositorService, CompositorState,
};

use crate::services::{ReadOnlyService, Service, ServiceEvent};
use iced::futures::SinkExt;
use iced::{Subscription, Task, stream::channel};
use std::{any::TypeId, ops::Deref};
use tokio::sync::{OnceCell, broadcast};

const BROADCAST_CAPACITY: usize = 64;

static BROADCASTER: OnceCell<broadcast::Sender<ServiceEvent<CompositorService>>> =
    OnceCell::const_new();

/// Subscribe to compositor events. Initialises the broadcaster on
/// first call and spawns the margo state.json watcher.
async fn broadcaster_subscribe() -> broadcast::Receiver<ServiceEvent<CompositorService>> {
    BROADCASTER
        .get_or_init(|| async {
            let (tx, _) = broadcast::channel(BROADCAST_CAPACITY);
            tokio::spawn(broadcaster_event_loop(tx.clone()));
            tx
        })
        .await
        .subscribe()
}

async fn broadcaster_event_loop(tx: broadcast::Sender<ServiceEvent<CompositorService>>) {
    if !margo::is_available() {
        log::error!("margo state.json not found — is the compositor running?");
        let _ = tx.send(ServiceEvent::Error(
            "margo compositor not detected".into(),
        ));
        return;
    }

    log::info!("Starting compositor event loop (margo backend)");

    if let Err(e) = margo::run_listener(&tx).await {
        log::error!("Compositor event loop failed: {}", e);
        let _ = tx.send(ServiceEvent::Error(e.to_string()));
    }
}

impl Deref for CompositorService {
    type Target = CompositorState;
    fn deref(&self) -> &Self::Target {
        &self.state
    }
}

impl ReadOnlyService for CompositorService {
    type UpdateEvent = CompositorEvent;
    type Error = String;

    fn update(&mut self, event: Self::UpdateEvent) {
        match event {
            CompositorEvent::StateChanged(new_state) => {
                self.state = *new_state;
            }
            CompositorEvent::ActionPerformed => {}
        }
    }

    fn subscribe() -> Subscription<ServiceEvent<Self>> {
        Subscription::run_with(TypeId::of::<Self>(), |_| {
            channel(10, async move |mut output| {
                let mut rx = broadcaster_subscribe().await;

                // Push an empty initial state so subscribers don't
                // sit with `None` until the first inotify event
                // fires.
                let empty_init = CompositorService {
                    state: CompositorState::default(),
                };
                if output.send(ServiceEvent::Init(empty_init)).await.is_err() {
                    log::debug!("Compositor subscriber disconnected before receiving Init");
                    return;
                }

                loop {
                    match rx.recv().await {
                        Ok(event) => {
                            if output.send(event).await.is_err() {
                                log::debug!("Compositor subscriber disconnected");
                                break;
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            log::warn!("Compositor subscriber lagged by {} messages", n);
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            log::error!("Compositor broadcaster closed unexpectedly");
                            break;
                        }
                    }
                }
            })
        })
    }
}

impl Service for CompositorService {
    type Command = CompositorCommand;

    fn command(&mut self, command: Self::Command) -> Task<ServiceEvent<Self>> {
        Task::perform(
            async move { margo::execute_command(command).await },
            |res| match res {
                Ok(()) => ServiceEvent::Update(CompositorEvent::ActionPerformed),
                Err(e) => ServiceEvent::Error(e.to_string()),
            },
        )
    }
}
