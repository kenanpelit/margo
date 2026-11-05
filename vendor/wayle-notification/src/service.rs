use std::sync::{Arc, Mutex};

use derive_more::Debug;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tracing::{instrument, warn};
use wayle_core::Property;
use zbus::Connection;

use crate::{
    builder::NotificationServiceBuilder, core::notification::Notification, error::Error,
    events::NotificationEvent, persistence::NotificationStore, popup_timer::PopupTimerManager,
    types::ClosedReason,
};

/// Desktop notification service. See [crate-level docs](crate) for usage.
#[derive(Debug)]
pub struct NotificationService {
    #[debug(skip)]
    pub(crate) cancellation_token: CancellationToken,
    #[debug(skip)]
    pub(crate) notif_tx: broadcast::Sender<NotificationEvent>,
    #[debug(skip)]
    pub(crate) store: Option<NotificationStore>,
    #[debug(skip)]
    pub(crate) connection: Connection,

    /// All received notifications.
    pub notifications: Property<Vec<Arc<Notification>>>,
    /// Currently visible popups.
    pub popups: Property<Vec<Arc<Notification>>>,
    /// Popup display duration in milliseconds.
    pub popup_duration: Property<u32>,
    /// Do Not Disturb mode; suppresses popups when true.
    pub dnd: Property<bool>,
    #[debug(skip)]
    pub(crate) dnd_events: DndEvents,
    /// Auto-remove expired notifications.
    pub remove_expired: Property<bool>,
    /// Glob patterns for blocking notifications by app name.
    pub blocklist: Property<Vec<String>>,
    #[debug(skip)]
    pub(crate) popup_timers: Arc<PopupTimerManager>,
}

impl NotificationService {
    /// Creates a new notification service instance.
    ///
    /// # Errors
    /// Returns error if D-Bus connection fails or service registration fails.
    #[instrument(name = "NotificationService::new", err)]
    pub async fn new() -> Result<Arc<Self>, Error> {
        Self::builder().build().await
    }

    /// Creates a builder for configuring a NotificationService.
    pub fn builder() -> NotificationServiceBuilder {
        NotificationServiceBuilder::new()
    }

    /// Dismisses all notifications and emits `NotificationClosed` for each.
    ///
    /// # Errors
    /// Returns error if the event channel is closed.
    #[instrument(skip(self), err)]
    pub async fn dismiss_all(&self) -> Result<(), Error> {
        let notifications = self.notifications.get();

        for notif in notifications.iter() {
            if let Err(error) = self.notif_tx.send(NotificationEvent::Remove(
                notif.id,
                ClosedReason::DismissedByUser,
            )) {
                warn!(error = %error, id = notif.id, "cannot dismiss notification");
            }
        }

        Ok(())
    }

    /// Sets the Do Not Disturb mode.
    ///
    /// When enabled, new notifications will not appear as popups but will
    /// still be added to the notification list.
    pub fn set_dnd(&self, dnd: bool) {
        self.dnd.set(dnd);
        self.dnd_events.send(dnd);
    }

    /// Every value passed to [`Self::set_dnd`] from now on, in order.
    ///
    /// Unlike `dnd.watch()`, which only yields the latest value, nothing is
    /// coalesced: a quick off→on still arrives as `false`, `true`. The
    /// receiver must be drained; dropping it unsubscribes.
    pub fn dnd_changes(&self) -> tokio::sync::mpsc::UnboundedReceiver<bool> {
        self.dnd_events.subscribe()
    }

    /// Sets the duration for how long popup notifications are displayed.
    pub fn set_popup_duration(&self, duration: u32) {
        self.popup_duration.set(duration)
    }

    /// Replaces the blocklist patterns.
    pub fn set_blocklist(&self, patterns: Vec<String>) {
        self.blocklist.set(patterns)
    }

    /// Removes a popup from the visible list without affecting notification history.
    ///
    /// Cancels any running popup timer for this ID.
    pub fn dismiss_popup(&self, id: u32) {
        self.popup_timers.cancel(id);

        let mut list = self.popups.get();
        list.retain(|popup| popup.id != id);
        self.popups.set(list);
    }

    /// Pauses the popup countdown timer.
    pub fn inhibit_popup(&self, id: u32) {
        self.popup_timers.pause(id);
    }

    /// Resumes the popup countdown timer after a pause.
    pub fn release_popup(&self, id: u32) {
        self.popup_timers.resume(id);
    }
}

/// Subscribers to [`NotificationService::dnd_changes`].
#[derive(Default)]
pub(crate) struct DndEvents(Mutex<Vec<tokio::sync::mpsc::UnboundedSender<bool>>>);

impl DndEvents {
    fn subscribe(&self) -> tokio::sync::mpsc::UnboundedReceiver<bool> {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        self.0.lock().unwrap_or_else(|e| e.into_inner()).push(tx);
        rx
    }

    /// Queue `value` for every live subscriber; drop the closed ones.
    fn send(&self, value: bool) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|tx| tx.send(value).is_ok());
    }
}

impl Drop for NotificationService {
    fn drop(&mut self) {
        self.cancellation_token.cancel();
    }
}

#[cfg(test)]
mod dnd_event_tests {
    use super::DndEvents;

    #[test]
    fn fast_off_on_is_queued_in_order_for_every_subscriber() {
        let events = DndEvents::default();
        let mut a = events.subscribe();
        let mut b = events.subscribe();
        for value in [true, false, true] {
            events.send(value);
        }
        for rx in [&mut a, &mut b] {
            assert_eq!(rx.try_recv(), Ok(true));
            assert_eq!(rx.try_recv(), Ok(false));
            assert_eq!(rx.try_recv(), Ok(true));
            assert!(rx.try_recv().is_err());
        }
    }

    #[test]
    fn dropped_subscriber_is_forgotten() {
        let events = DndEvents::default();
        drop(events.subscribe());
        events.send(true);
        assert!(events.0.lock().expect("not poisoned").is_empty());
    }
}
