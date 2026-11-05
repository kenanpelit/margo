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
//! must not prevent the underlying action from running. The D-Bus call
//! runs off the calling thread (`mshell_process::desktop::notify`).

/// Fire a one-shot desktop notification. `title` shows as the
/// summary, `body` as the body text. Both are owned strings so
/// the caller doesn't have to worry about lifetime when invoking
/// from inside a closure.
pub fn toast(title: impl Into<String>, body: impl Into<String>) {
    // 3-second timeout matches the rest of mshell's transient toasts
    // (battery low, podman events, etc.); the generic info-style category
    // picks up the user's normal-priority style (no urgency bar).
    mshell_process::desktop::notify(
        mshell_process::desktop::Notice::new("mshell", title, body)
            .timeout_ms(3000)
            .category("im.received"),
    );
}
