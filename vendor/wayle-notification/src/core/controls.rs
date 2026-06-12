use tracing::instrument;
use zbus::Connection;

use crate::{
    error::Error,
    types::{
        Signal,
        dbus::{SERVICE_INTERFACE, SERVICE_PATH},
    },
};

pub(super) struct NotificationControls;

impl NotificationControls {
    #[instrument(skip(connection), fields(notification_id = %id, action = %action_key), err)]
    pub(super) async fn invoke(
        connection: &Connection,
        id: &u32,
        action_key: &str,
    ) -> Result<(), Error> {
        connection
            .emit_signal(
                None::<()>,
                SERVICE_PATH,
                SERVICE_INTERFACE,
                Signal::ActionInvoked.as_str(),
                &(id, action_key),
            )
            .await?;

        Ok(())
    }

    /// Emit the KDE-style `NotificationReplied(id, text)` signal — the
    /// delivery half of the `inline-reply` capability. Clients that sent
    /// an `"inline-reply"` action listen for this to receive the text the
    /// user typed into the notification (margo vendor extension).
    #[instrument(skip(connection, text), fields(notification_id = %id), err)]
    pub(super) async fn reply(connection: &Connection, id: &u32, text: &str) -> Result<(), Error> {
        connection
            .emit_signal(
                None::<()>,
                SERVICE_PATH,
                SERVICE_INTERFACE,
                Signal::NotificationReplied.as_str(),
                &(id, text),
            )
            .await?;

        Ok(())
    }
}
