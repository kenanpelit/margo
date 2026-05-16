//! Desktop-notification helper for activation feedback.
//!
//! Providers wrap their `on_activate` side-effect with [`toast`]
//! when the user can't easily tell the action happened (clipboard
//! copy, wallpaper cycle, twilight tweak). The notification fires
//! through whatever notification daemon the user runs — mshell's
//! own notification-popups module if available, otherwise the
//! system D-Bus default.
//!
//! Errors are swallowed: a missing or buggy notification daemon
//! must not prevent the underlying action from running.

/// Fire a one-shot desktop notification. `title` shows as the
/// summary, `body` as the body text. Both are owned strings so
/// the caller doesn't have to worry about lifetime when invoking
/// from inside a closure.
pub fn toast(title: impl Into<String>, body: impl Into<String>) {
    let title = title.into();
    let body = body.into();
    if let Err(err) = notify_rust::Notification::new()
        .summary(&title)
        .body(&body)
        // 3-second timeout matches the rest of mshell's transient
        // toasts (battery low, podman events, etc.). Longer
        // would clutter; shorter would feel flicker-y on slow
        // notification daemons.
        .timeout(notify_rust::Timeout::Milliseconds(3000))
        // Use a generic info-style hint so the popup picks up
        // the user's normal-priority style (no urgency bar).
        .hint(notify_rust::Hint::Category("im.received".into()))
        .show()
    {
        tracing::warn!(?err, %title, "launcher toast failed");
    }
}
