use tracing::info;
use zbus::proxy;

#[derive(Debug, Clone)]
pub enum FingerprintEvent {
    Ready,
    Scanning,
    Match,
    NoMatch,
    UnknownError,
    Error(String),
}

#[proxy(
    interface = "net.reactivated.Fprint.Manager",
    default_service = "net.reactivated.Fprint",
    default_path = "/net/reactivated/Fprint/Manager"
)]
trait FprintManager {
    fn get_default_device(&self) -> zbus::Result<zbus::zvariant::OwnedObjectPath>;
}

#[proxy(
    interface = "net.reactivated.Fprint.Device",
    default_service = "net.reactivated.Fprint"
)]
pub trait FprintDevice {
    fn claim(&self, username: &str) -> zbus::Result<()>;
    fn verify_start(&self, finger_name: &str) -> zbus::Result<()>;
    fn verify_stop(&self) -> zbus::Result<()>;
    fn release(&self) -> zbus::Result<()>;

    #[zbus(signal)]
    fn verify_status(&self, result: &str, done: bool) -> zbus::Result<()>;

    #[zbus(signal)]
    fn verify_finger_required(&self, finger_name: &str) -> zbus::Result<()>;
}

pub struct FingerprintAuth {
    pub device: FprintDeviceProxy<'static>,
}

impl FingerprintAuth {
    pub async fn new() -> zbus::Result<Self> {
        let conn = zbus::Connection::system().await?;

        let manager = FprintManagerProxy::new(&conn).await?;
        let device_path = manager.get_default_device().await?;

        let device = FprintDeviceProxy::builder(&conn)
            .path(device_path)?
            .build()
            .await?;

        Ok(Self { device })
    }

    pub async fn start(&self, username: &str) -> zbus::Result<()> {
        let _ = self.device.verify_stop().await;
        let _ = self.device.release().await;

        let mut delay = std::time::Duration::from_millis(200);
        for attempt in 1..=5 {
            match self.device.claim(username).await {
                Ok(()) => {
                    self.device.verify_start("any").await?;
                    return Ok(());
                }
                Err(e) if attempt < 5 => {
                    let msg = e.to_string();
                    if msg.contains("busy") || msg.contains("Internal") {
                        info!("fprintd claim attempt {attempt}/5: {msg}, retrying in {delay:?}");
                        tokio::time::sleep(delay).await;
                        delay *= 2;
                    } else {
                        return Err(e);
                    }
                }
                Err(e) => return Err(e),
            }
        }
        unreachable!()
    }

    /// Blocks until a verify result comes in. Returns the event.
    pub async fn wait_for_result(&self) -> zbus::Result<FingerprintEvent> {
        use futures::StreamExt;

        let mut stream = self.device.receive_verify_status().await?;

        if let Some(signal) = stream.next().await {
            let args = signal.args()?;
            info!("fprintd verify status: {} done: {}", args.result, args.done);

            Ok(classify_verify_status(args.result))
        } else {
            Ok(FingerprintEvent::Error("Signal stream ended".into()))
        }
    }

    pub async fn stop(&self) -> zbus::Result<()> {
        let _ = self.device.verify_stop().await;
        let _ = self.device.release().await;
        Ok(())
    }
}

/// Map an fprintd `VerifyStatus` result string to a [`FingerprintEvent`].
///
/// Split out from [`FingerprintAuth::wait_for_result`] so the fprintd
/// protocol mapping is unit-testable without a live D-Bus device. The
/// "retry" family (`verify-retry-scan`, `verify-swipe-too-short`, …) all
/// collapse to [`FingerprintEvent::NoMatch`] because the UI treats them
/// identically — "try again".
fn classify_verify_status(result: &str) -> FingerprintEvent {
    match result {
        "verify-match" => FingerprintEvent::Match,
        "verify-no-match"
        | "verify-retry-scan"
        | "verify-swipe-too-short"
        | "verify-finger-not-centered"
        | "verify-remove-and-retry" => FingerprintEvent::NoMatch,
        "verify-unknown-error" => FingerprintEvent::UnknownError,
        "verify-disconnected" => FingerprintEvent::Error("Device disconnected".into()),
        other => FingerprintEvent::Error(format!("Unexpected: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn match_result_authenticates() {
        assert!(matches!(
            classify_verify_status("verify-match"),
            FingerprintEvent::Match
        ));
    }

    #[test]
    fn every_retry_variant_collapses_to_no_match() {
        // Getting any of these wrong (e.g. treating a retry as an error) would
        // wrongly abort a scan the user could still complete.
        for r in [
            "verify-no-match",
            "verify-retry-scan",
            "verify-swipe-too-short",
            "verify-finger-not-centered",
            "verify-remove-and-retry",
        ] {
            assert!(
                matches!(classify_verify_status(r), FingerprintEvent::NoMatch),
                "`{r}` must map to NoMatch"
            );
        }
    }

    #[test]
    fn unknown_error_is_its_own_variant() {
        assert!(matches!(
            classify_verify_status("verify-unknown-error"),
            FingerprintEvent::UnknownError
        ));
    }

    #[test]
    fn disconnect_carries_a_fixed_message() {
        match classify_verify_status("verify-disconnected") {
            FingerprintEvent::Error(msg) => assert_eq!(msg, "Device disconnected"),
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[test]
    fn unrecognised_result_is_reported_verbatim() {
        // A future fprintd status we don't know about must surface as an
        // error naming the string, not be silently swallowed as a match.
        match classify_verify_status("verify-future-thing") {
            FingerprintEvent::Error(msg) => assert_eq!(msg, "Unexpected: verify-future-thing"),
            other => panic!("expected Error, got {other:?}"),
        }
    }
}
