//! Shell-wide background drivers started from the frame: each is a shared,
//! idempotent singleton started by a config effect the first time its
//! toggle is on (it handles being turned off itself).

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
}
