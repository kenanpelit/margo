//! Shell-wide background drivers started from the frame: each is a shared,
//! idempotent singleton driven by a config effect from its toggle.

use super::*;

pub(super) fn push_background_hooks(effects: &mut EffectScope) {
    // Auto-DND while screen sharing.
    effects.push(move |_| {
        if config_manager()
            .config()
            .notifications()
            .auto_dnd_screen_share()
            .get()
        {
            crate::bars::bar_widgets::privacy::auto_dnd::ensure_started();
        }
    });

    // Crash notifications (systemd-coredump watcher): started and stopped
    // with the toggle, so it costs nothing while off.
    effects.push(move |_| {
        crate::crashes::set_watcher(config_manager().config().crashes().notify().get());
    });
}
