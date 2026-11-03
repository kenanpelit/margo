//! The window shell every OSD capsule shares (volume / mic / brightness /
//! network): an undecorated, initially hidden `osd-window` that lives on the
//! overlay layer of one monitor, plus the auto-hide timer that dismisses it.
//!
//! What the capsule *contains* is up to the caller; the chrome (width, radius,
//! border) is CSS-driven and the screen-edge placement comes from
//! [`crate::osd_geometry`].

use gtk4::gdk;
use gtk4::prelude::{GtkWindowExt, WidgetExt};
use gtk4_layer_shell::{Layer, LayerShell};
use mshell_common::WatcherToken;
use relm4::{Component, ComponentSender, gtk};
use std::time::Duration;

/// The CSS classes on every OSD window.
pub(crate) const WINDOW_CLASSES: [&str; 2] = ["osd-window", "window-opacity"];

/// The layer-shell namespace every OSD window registers under.
pub(crate) const NAMESPACE: &str = "mshell-osd";

/// A new, hidden OSD window (not yet a layer surface — see [`init_layer_shell`]).
pub(crate) fn new_window() -> gtk::Window {
    let window = gtk::Window::new();
    window.set_css_classes(&WINDOW_CLASSES);
    window.set_decorated(false);
    window.set_visible(false);
    window.set_default_height(1);
    window
}

/// Turn `root` into an overlay layer surface on `monitor`, placed by the live
/// `osd` position / distance config.
pub(crate) fn init_layer_shell(root: &gtk::Window, monitor: &gdk::Monitor) {
    root.init_layer_shell();
    root.set_monitor(Some(monitor));
    root.set_namespace(Some(NAMESPACE));
    root.set_layer(Layer::Overlay);
    root.set_exclusive_zone(0);
    let (position, distance) = crate::osd_geometry::read();
    crate::osd_geometry::apply(root, &position, distance);
}

/// (Re)start the auto-hide timer: `hide` is sent as a command output `after`
/// from now unless the timer is restarted first (resetting `token` cancels the
/// pending one), so the OSD stays up while the value keeps changing.
pub(crate) fn arm_hide<C>(
    sender: &ComponentSender<C>,
    token: &mut WatcherToken,
    after: Duration,
    hide: C::CommandOutput,
) where
    C: Component,
    C::CommandOutput: Send + 'static,
{
    let token = token.reset();
    sender.command(move |out, shutdown| {
        shutdown
            .register(async move {
                tokio::time::sleep(after).await;
                if !token.is_cancelled() {
                    out.send(hide).ok();
                }
            })
            .drop_on_shutdown()
    });
}
