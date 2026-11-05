//! One portal chooser across all output frames, including the monitor/area
//! overlay. The active monitor can change between D-Bus calls, so per-frame
//! request queues alone cannot provide this invariant.
use std::future::Future;

static PICKER: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Refuse a second request while one owns the chooser, rather than opening
/// it on another output and racing the shared monitor/area overlay. The
/// original request's dialog and reply channel are untouched.
pub(super) async fn one_picker(pick: impl Future<Output = String>) -> String {
    let Ok(_guard) = PICKER.try_lock() else {
        return String::new();
    };
    pick.await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn simultaneous_choose_calls_cannot_cross_their_replies() {
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let (finish_tx, finish_rx) = tokio::sync::oneshot::channel();
        let a = tokio::spawn(one_picker(async {
            ready_tx.send(()).unwrap();
            finish_rx.await.unwrap();
            "screen:DP-1".into()
        }));
        ready_rx.await.unwrap();
        let b = one_picker(async { "window:7".into() }).await;
        assert_eq!(
            b, "",
            "second request reached a chooser while first was active"
        );
        finish_tx.send(()).unwrap();
        assert_eq!(a.await.unwrap(), "screen:DP-1");
        assert_eq!(one_picker(async { "window:8".into() }).await, "window:8");
    }
}
