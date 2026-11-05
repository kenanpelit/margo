//! Keyboard handling on the frame's root surface: re-asserting the keyboard
//! mode when the surface maps, and Escape closing the open menu.

use super::*;

/// Install the map-time keyboard-mode re-assert and the Escape handlers on
/// the frame's root window.
pub(super) fn install_key_handlers(root: &gtk::Window, sender: &ComponentSender<Frame>) {
    // Re-assert keyboard interactivity once the surface actually
    // maps. The `set_keyboard_mode(None)` in `Frame::init` is queued before
    // the layer surface exists; in practice the frame has been
    // observed committing `Exclusive` at login anyway, which traps
    // keyboard focus into this invisible full-screen layer (margo's
    // `compute_desired_focus` honours any Exclusive Top/Overlay
    // layer) until the user's first menu toggle finally runs
    // `sync_keyboard_mode`. Firing it here closes that gap: at first
    // map no menu is revealed, so it resolves to `None`.
    {
        let sender_map = sender.clone();
        root.connect_map(move |w| {
            tracing::debug!(
                mode = ?w.keyboard_mode(),
                "frame: surface mapped — re-asserting keyboard mode"
            );
            sender_map.input(FrameInput::SyncKeyboardMode);
        });
    }

    // ESC closes any open menu. Belt-and-suspenders setup:
    //
    // 1) `ShortcutController(scope=Global)` with a `KeyvalTrigger`
    //    on Escape — this is the global-shortcut path that fires
    //    as soon as the layer surface receives the keypress,
    //    regardless of which child widget has internal focus.
    //
    // 2) An `EventControllerKey` in `Capture` phase as a fallback,
    //    plus a debug log so we can see whether keys are reaching
    //    the surface at all. (Layer-shell + OnDemand keyboard
    //    interactivity sometimes never delivers keys until the
    //    compositor decides to grant focus.)
    let sender_esc = sender.clone();
    let shortcut = gtk::Shortcut::builder()
        .trigger(&gtk::KeyvalTrigger::new(
            gdk::Key::Escape,
            gdk::ModifierType::empty(),
        ))
        .action(&gtk::CallbackAction::new(move |_, _| {
            tracing::debug!("frame: ESC shortcut fired");
            // If the clipboard `/` filter is open, Esc leaves search
            // mode (vim semantics) instead of closing the menu.
            if crate::menus::menu_widgets::clipboard::clipboard::search_is_active() {
                sender_esc.input(FrameInput::ClipboardExitSearch);
            } else {
                sender_esc.input(FrameInput::CloseMenus);
            }
            gtk::glib::Propagation::Stop
        }))
        .build();
    let shortcut_ctrl = gtk::ShortcutController::new();
    shortcut_ctrl.set_scope(gtk::ShortcutScope::Global);
    shortcut_ctrl.add_shortcut(shortcut);
    root.add_controller(shortcut_ctrl);

    let key_ctrl = gtk::EventControllerKey::new();
    key_ctrl.set_propagation_phase(gtk::PropagationPhase::Capture);
    let sender_esc2 = sender.clone();
    key_ctrl.connect_key_pressed(move |_, keyval, _, _| {
        tracing::debug!(?keyval, "frame: key_pressed");
        if keyval == gdk::Key::Escape {
            if crate::menus::menu_widgets::clipboard::clipboard::search_is_active() {
                sender_esc2.input(FrameInput::ClipboardExitSearch);
            } else {
                sender_esc2.input(FrameInput::CloseMenus);
            }
            gtk::glib::Propagation::Stop
        } else {
            gtk::glib::Propagation::Proceed
        }
    });
    root.add_controller(key_ctrl);
}
