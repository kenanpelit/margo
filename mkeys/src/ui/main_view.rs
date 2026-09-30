use std::cell::Cell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::thread;

use gdk4::prelude::ObjectExt;
use gtk::prelude::{
    ApplicationExt, BoxExt, ButtonExt, GestureDragExt, GestureSingleExt, GtkWindowExt,
    ToggleButtonExt, WidgetExt,
};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use relm4::gtk::{gdk, glib};
use relm4::{ComponentParts, ComponentSender, RelmWidgetExt, SimpleComponent, gtk};
use tracing::info;

use crate::{
    config::{Config, Position},
    layout::parse::{KeyType, LayoutDefinition},
    service::{IPCHandle, host::KeyboardHandle},
};

use super::components::ButtonEX;

pub struct UIModel {
    keyboard_handle: Box<dyn KeyboardHandle>,
}

#[derive(Debug)]
pub enum UIMessage {
    ButtonPress(u16),
    ButtonRelease(u16),
    ModPress(u16),
    ModRelease(u16),
    LockPress(u16),
    LockRelease(u16),
    AppQuit,
}

impl SimpleComponent for UIModel {
    type Init = (
        Box<dyn KeyboardHandle>,
        Box<dyn IPCHandle + Send>,
        LayoutDefinition,
        Config,
    );

    type Input = UIMessage;
    type Output = ();
    type Root = gtk::Window;
    type Widgets = ();

    fn init_root() -> Self::Root {
        gtk::Window::builder().build()
    }

    fn init(
        handle: Self::Init,
        window: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let (keyboard_handle, ipc_handle, keyboard_definition, config) = handle;

        // Listen for the `quit` command from message clients.
        let message_sender = sender.clone();
        thread::spawn(move || {
            loop {
                if let Ok(command) = String::from_utf8(ipc_handle.read())
                    && command.as_str() == "quit"
                {
                    info!("mkeys: received quit");
                    message_sender.input(UIMessage::AppQuit);
                    break;
                }
            }
        });

        // Layer-shell placement, driven by mkeys.toml.
        window.init_layer_shell();
        window.set_namespace(Some("mkeys"));
        window.set_layer(Layer::Overlay);
        window.set_keyboard_mode(KeyboardMode::None);
        window.set_exclusive_zone(0);
        window.set_opacity(config.opacity as f64);

        // Key cell height, scaled from the base 64px unit.
        let geometry_unit = (64.0 * config.scale).round() as i32;
        let kb_width_units = keyboard_definition.width;
        let kb_height_rows = keyboard_definition.height;

        let is_floating = Rc::new(Cell::new(config.floating));
        let float_state = FloatState {
            pos: Rc::new(Cell::new((0, 0))),
            bounds: Rc::new(Cell::new((0, 0))),
        };
        if is_floating.get() {
            setup_floating(
                &window,
                config.margin,
                kb_width_units,
                kb_height_rows,
                geometry_unit,
                &float_state,
            );
        } else {
            setup_docked(&window, config.position, config.margin);
        }

        // Toolbar: dock/undock on the left, hide on the right. Always
        // present, independent of `floating` — cosmic-osk parity 4/4.
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .build();
        window.set_child(Some(&root));
        root.set_align(gtk::Align::Center);
        root.set_expand(true);

        let toolbar = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .css_classes(["mkeys-toolbar"])
            .build();
        let dock_button = gtk::Button::from_icon_name(dock_icon_name(is_floating.get()));
        dock_button.add_css_class("flat");
        dock_button.set_tooltip_text(Some(dock_tooltip(is_floating.get())));
        let toolbar_spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        toolbar_spacer.set_hexpand(true);
        let close_button = gtk::Button::from_icon_name("window-close-symbolic");
        close_button.add_css_class("flat");
        close_button.set_tooltip_text(Some("Hide keyboard"));
        toolbar.append(&dock_button);
        toolbar.append(&toolbar_spacer);
        toolbar.append(&close_button);
        root.append(&toolbar);

        let close_sender = sender.clone();
        close_button.connect_clicked(move |_| {
            close_sender.input(UIMessage::AppQuit);
        });

        let container = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .build();
        root.append(&container);

        let grip = gtk::Box::builder().css_classes(["mkeys-grip"]).build();
        grip.set_visible(is_floating.get());
        attach_grip_drag(&grip, &window, &float_state);
        container.append(&grip);

        let dock_window = window.clone();
        let dock_grip = grip.clone();
        let dock_button_self = dock_button.clone();
        let dock_is_floating = is_floating.clone();
        let dock_float_state = float_state.clone();
        let dock_position = config.position;
        let dock_margin = config.margin;
        dock_button.connect_clicked(move |_| {
            let now_floating = !dock_is_floating.get();
            dock_is_floating.set(now_floating);

            if !now_floating {
                // Docking is purely local — no IPC round-trip needed.
                setup_docked(&dock_window, dock_position, dock_margin);
                dock_grip.set_visible(false);
                dock_button_self.set_icon_name(dock_icon_name(false));
                dock_button_self.set_tooltip_text(Some(dock_tooltip(false)));
                return;
            }

            // Undocking needs active_monitor_size(), an `mctl` IPC
            // round-trip that can block for up to its 5s socket timeout
            // if margo is wedged. This fires from a live, interactive
            // click (unlike the same call at init(), before the GTK
            // main loop pumps) — run it on a background thread and
            // apply the result on the main thread via a short poll
            // rather than blocking here, so a slow/hung compositor
            // can't freeze the whole on-screen keyboard.
            let monitor_size = Arc::new(Mutex::new(None));
            let monitor_size_bg = monitor_size.clone();
            thread::spawn(move || {
                *monitor_size_bg.lock().unwrap() = Some(active_monitor_size());
            });

            let window = dock_window.clone();
            let grip = dock_grip.clone();
            let button = dock_button_self.clone();
            let float_state = dock_float_state.clone();
            glib::timeout_add_local(std::time::Duration::from_millis(16), move || {
                let Some(size) = *monitor_size.lock().unwrap() else {
                    return glib::ControlFlow::Continue;
                };
                apply_floating(
                    &window,
                    dock_margin,
                    kb_width_units,
                    kb_height_rows,
                    geometry_unit,
                    &float_state,
                    size,
                );
                grip.set_visible(true);
                button.set_icon_name(dock_icon_name(true));
                button.set_tooltip_text(Some(dock_tooltip(true)));
                glib::ControlFlow::Break
            });
        });

        keyboard_definition.layout.iter().for_each(|row| {
            let row_container = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .build();

            row.iter().for_each(|key| {
                let scan_code = key.scan_code;
                let width = (key.width.unwrap_or(1.0) * geometry_unit as f32).round() as i32;

                // Special-key glyphs are plain Unicode symbols, not
                // icon-theme lookups: every install's font stack covers
                // them, no dependency on which icon theme (or whether it
                // inherits Adwaita/breeze) happens to be active.
                let glyph = special_key_glyph(scan_code);
                let mod_lock_label = || {
                    glyph.map(str::to_string).unwrap_or_else(|| {
                        // Join whichever legends are actually set — a
                        // key with only one (e.g. the numpad's Num
                        // Lock, top_legend only) must not print a
                        // stray leading/trailing space.
                        [key.bottom_legend.clone(), key.top_legend.clone()]
                            .into_iter()
                            .flatten()
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                };

                match key.key_type() {
                    KeyType::Mod => {
                        let toggle = gtk::ToggleButton::builder()
                            .label(mod_lock_label())
                            .width_request(width)
                            .height_request(geometry_unit)
                            .build();
                        if glyph.is_some() {
                            toggle.add_css_class("mkeys-glyph");
                        }

                        let button_sender = sender.clone();
                        toggle.connect_toggled(move |btn| {
                            if btn.is_active() {
                                button_sender.input(UIMessage::ModPress(scan_code));
                            } else {
                                button_sender.input(UIMessage::ModRelease(scan_code));
                            }
                        });
                        row_container.append(&toggle);
                    }
                    KeyType::Lock => {
                        let toggle = gtk::ToggleButton::builder()
                            .label(mod_lock_label())
                            .width_request(width)
                            .height_request(geometry_unit)
                            .build();
                        if glyph.is_some() {
                            toggle.add_css_class("mkeys-glyph");
                        }

                        let button_sender = sender.clone();
                        toggle.connect_toggled(move |btn| {
                            if btn.is_active() {
                                button_sender.input(UIMessage::LockPress(scan_code));
                            } else {
                                button_sender.input(UIMessage::LockRelease(scan_code));
                            }
                        });
                        row_container.append(&toggle);
                    }
                    KeyType::Normal => {
                        if scan_code == 0 {
                            let label = gtk::Label::default();
                            label.set_width_request(width);
                            row_container.append(&label);
                        } else {
                            let button = ButtonEX::default();
                            if let Some(glyph) = glyph {
                                button.set_primary_content(glyph.to_string());
                                button.add_css_class("mkeys-glyph");
                            } else {
                                button.set_primary_content(
                                    key.top_legend.clone().unwrap_or_default(),
                                );
                            }
                            button.set_secondary_content(
                                key.bottom_legend.clone().unwrap_or_default(),
                            );
                            button.set_width_request(width);
                            button.set_height_request(geometry_unit);

                            let press_sender = sender.clone();
                            button.connect("pressed", true, move |_| {
                                press_sender.input(UIMessage::ButtonPress(scan_code));
                                None
                            });

                            let release_sender = sender.clone();
                            button.connect("released", true, move |_| {
                                release_sender.input(UIMessage::ButtonRelease(scan_code));
                                None
                            });

                            row_container.append(&button);
                        }
                    }
                }
            });

            container.append(&row_container);
        });

        let model = UIModel { keyboard_handle };
        ComponentParts { model, widgets: () }
    }

    fn update(&mut self, msg: Self::Input, _sender: ComponentSender<Self>) {
        match msg {
            UIMessage::ButtonPress(scan_code) => {
                self.keyboard_handle
                    .key_press(evdev::KeyCode::new(scan_code));
            }
            UIMessage::ButtonRelease(scan_code) => {
                self.keyboard_handle
                    .key_release(evdev::KeyCode::new(scan_code));
            }
            UIMessage::ModPress(scan_code) => {
                self.keyboard_handle
                    .append_mod(evdev::KeyCode::new(scan_code));
            }
            UIMessage::ModRelease(scan_code) => {
                self.keyboard_handle
                    .remove_mod(evdev::KeyCode::new(scan_code));
            }
            UIMessage::LockPress(scan_code) => {
                self.keyboard_handle
                    .append_lock(evdev::KeyCode::new(scan_code));
            }
            UIMessage::LockRelease(scan_code) => {
                self.keyboard_handle
                    .remove_lock(evdev::KeyCode::new(scan_code));
            }
            UIMessage::AppQuit => {
                self.keyboard_handle.destroy();
                relm4::main_application().quit();
            }
        }
    }

    fn update_view(&self, _widgets: &mut Self::Widgets, _sender: ComponentSender<Self>) {}
}

/// Live floating position + drag bounds, shared between [`setup_floating`]
/// and [`attach_grip_drag`] (and refreshed by the dock/undock toolbar
/// button each time floating mode is re-entered).
#[derive(Clone)]
struct FloatState {
    pos: Rc<Cell<(i32, i32)>>,
    bounds: Rc<Cell<(i32, i32)>>,
}

/// Synchronous convenience for `init()`, where blocking briefly on the
/// `active_monitor_size()` IPC round-trip is harmless (the GTK main
/// loop hasn't started pumping events yet). The dock/undock toolbar
/// button — which fires *while* the keyboard is live and interactive —
/// instead calls `active_monitor_size()` off-thread and applies the
/// result via [`apply_floating`] once it's ready; see its call site.
fn setup_floating(
    window: &gtk::Window,
    margin: i32,
    kb_width_units: f32,
    kb_height_rows: i32,
    geometry_unit: i32,
    state: &FloatState,
) {
    apply_floating(
        window,
        margin,
        kb_width_units,
        kb_height_rows,
        geometry_unit,
        state,
        active_monitor_size(),
    );
}

/// Anchors the layer surface to (Left, Top) only — free absolute
/// positioning via margins, instead of docking to a screen edge — sets
/// the initial margins to a sensible starting spot (horizontally
/// centered, near the bottom like the docked default), and updates
/// `state` with that position and the (left, top) margin range that
/// keeps the keyboard fully on-screen, for [`attach_grip_drag`].
/// `monitor_size` is passed in (rather than queried here) so the caller
/// can compute it off the GTK main thread.
fn apply_floating(
    window: &gtk::Window,
    margin: i32,
    kb_width_units: f32,
    kb_height_rows: i32,
    geometry_unit: i32,
    state: &FloatState,
    (mon_w, mon_h): (i32, i32),
) {
    window.set_anchor(Edge::Left, true);
    window.set_anchor(Edge::Top, true);
    window.set_anchor(Edge::Right, false);
    window.set_anchor(Edge::Bottom, false);

    let (kb_width, kb_height) = keyboard_pixel_size(kb_width_units, kb_height_rows, geometry_unit);
    let start_left = ((mon_w - kb_width) / 2).max(0);
    let start_top = (mon_h - kb_height - margin).max(0);
    window.set_margin(Edge::Left, start_left);
    window.set_margin(Edge::Top, start_top);

    state.pos.set((start_left, start_top));
    state
        .bounds
        .set(((mon_w - kb_width).max(0), (mon_h - kb_height).max(0)));
}

/// Docks the layer surface to `position` (top/bottom), the pre-floating
/// mkeys behaviour.
fn setup_docked(window: &gtk::Window, position: Position, margin: i32) {
    let bottom = matches!(position, Position::Bottom);
    window.set_anchor(Edge::Left, false);
    window.set_anchor(Edge::Right, false);
    window.set_anchor(Edge::Top, !bottom);
    window.set_anchor(Edge::Bottom, bottom);
    window.set_margin(if bottom { Edge::Bottom } else { Edge::Top }, margin);
}

/// Unicode glyph for the handful of keys a real keyboard prints as a
/// symbol rather than a word — evdev scan code keyed, layout-independent.
fn special_key_glyph(scan_code: u16) -> Option<&'static str> {
    match scan_code {
        14 => Some("⌫"),      // Backspace
        15 => Some("⇥"),      // Tab
        28 => Some("⏎"),      // Enter
        42 | 54 => Some("⇧"), // Shift (left/right)
        58 => Some("⇪"),      // Caps Lock
        _ => None,
    }
}

fn dock_icon_name(floating: bool) -> &'static str {
    if floating {
        "view-restore-symbolic"
    } else {
        "view-fullscreen-symbolic"
    }
}

fn dock_tooltip(floating: bool) -> &'static str {
    if floating {
        "Dock keyboard"
    } else {
        "Detach keyboard"
    }
}

/// Approximate on-screen size of the keyboard, for computing a starting
/// floating position and clamping the drag range — a few tens of pixels
/// off (button CSS margins, the toolbar, the grip bar) doesn't matter
/// for either use.
fn keyboard_pixel_size(kb_width_units: f32, kb_height_rows: i32, geometry_unit: i32) -> (i32, i32) {
    let width = (geometry_unit as f32 * kb_width_units).round() as i32;
    let height = geometry_unit * kb_height_rows;
    (width.max(1), height.max(1))
}

/// The ACTIVE monitor's size in logical pixels — via margo's own IPC
/// client (`mctl::ipc_client::request_once`, same timeout + error
/// handling `mctl`'s CLI gets, rather than a hand-rolled subprocess),
/// picking the output flagged `"active"` in the snapshot rather than
/// just the first one (mkeys can be shown on a non-first monitor).
/// Falls back to a common 1080p size on any failure — this is only ever
/// used to pick a reasonable floating start position and drag bounds,
/// never something that needs to be exact. Blocking (up to `mctl`'s 5s
/// socket timeout in the worst case) — call off the GTK main thread.
fn active_monitor_size() -> (i32, i32) {
    const FALLBACK: (i32, i32) = (1920, 1080);
    (|| {
        let v = mctl::ipc_client::request_once("get monitors").ok()?;
        let monitors = v.get("monitors")?.as_array()?;
        let mon = monitors
            .iter()
            .find(|m| m.get("active").and_then(|a| a.as_bool()) == Some(true))
            .or_else(|| monitors.first())?;
        let w = mon.get("width")?.as_i64()?;
        let h = mon.get("height")?.as_i64()?;
        Some((w.max(1) as i32, h.max(1) as i32))
    })()
    .unwrap_or(FALLBACK)
}

/// Wires a `GestureDrag` on `grip` that moves `window`'s floating
/// position live, clamped to `state.bounds` (updated by [`setup_floating`]
/// each time floating mode is entered, incl. via the dock/undock toolbar
/// button) so the keyboard can't be dragged off-screen. Position is
/// tracked in `state.pos` rather than read back from gtk4-layer-shell.
fn attach_grip_drag(grip: &gtk::Box, window: &gtk::Window, state: &FloatState) {
    let drag_start = Rc::new(Cell::new((0, 0)));

    grip.set_cursor_from_name(Some("grab"));

    let gesture = gtk::GestureDrag::new();
    gesture.set_button(gdk::BUTTON_PRIMARY);

    let grip_begin = grip.clone();
    let pos_begin = state.pos.clone();
    let start_begin = drag_start.clone();
    gesture.connect_drag_begin(move |_, _, _| {
        start_begin.set(pos_begin.get());
        grip_begin.set_cursor_from_name(Some("grabbing"));
    });

    let window_update = window.clone();
    let pos_update = state.pos.clone();
    let bounds_update = state.bounds.clone();
    let start_update = drag_start.clone();
    gesture.connect_drag_update(move |_, offset_x, offset_y| {
        let (base_left, base_top) = start_update.get();
        let (bound_x, bound_y) = bounds_update.get();
        let new_left = (base_left + offset_x.round() as i32).clamp(0, bound_x);
        let new_top = (base_top + offset_y.round() as i32).clamp(0, bound_y);
        window_update.set_margin(Edge::Left, new_left);
        window_update.set_margin(Edge::Top, new_top);
        pos_update.set((new_left, new_top));
    });

    let grip_end = grip.clone();
    gesture.connect_drag_end(move |_, _, _| {
        grip_end.set_cursor_from_name(Some("grab"));
    });

    grip.add_controller(gesture);
}
