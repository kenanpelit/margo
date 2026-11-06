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

    // Removable-drive toasts: the udisks watcher otherwise only starts with
    // the USB pill or menu, which left Settings → Toasts → Removable drives
    // dead for everyone without them. The watcher is event-driven (udisks
    // signals), so running it costs nothing while no drive changes. The
    // watcher parks when nobody holds a receiver, so the toggle keeps one.
    thread_local! {
        static DRIVE_TOASTS: std::cell::RefCell<
            Option<tokio::sync::watch::Receiver<crate::removable_drives::DrivesState>>,
        > = const { std::cell::RefCell::new(None) };
    }
    effects.push(move |_| {
        let on = config_manager().config().toasts().removable_drives().get();
        DRIVE_TOASTS.with(|slot| {
            *slot.borrow_mut() = on.then(crate::removable_drives::subscribe);
        });
    });

    // Noise suppression the user left on comes back after a shell restart
    // or reboot (adopting a filter that survived; once per process).
    relm4::spawn(mshell_utils::denoise::restore_at_startup());

    // Crash notifications (systemd-coredump watcher): started and stopped
    // with the toggle, so it costs nothing while off.
    effects.push(move |_| {
        crate::crashes::set_watcher(config_manager().config().crashes().notify().get());
    });

    // ThinkPad logo LED automation (off while locked / during DND, blink
    // on notifications): runs only while a rule is on and the LED is
    // writable without a prompt.
    effects.push(move |_| {
        let rules = config_manager().config().control_center().logo_led().get();
        crate::menus::menu_widgets::control_center::tiles::led_auto::configure(rules);
    });

    // Drive health (alerts / trend history): a minute sysfs scan while
    // either is on; nothing runs while both are off.
    effects.push(move |_| {
        crate::drive_health::configure(&config_manager().config().drive_health().get());
    });
}
