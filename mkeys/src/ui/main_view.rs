use std::cell::Cell;
use std::rc::Rc;
use std::thread;

use gdk4::prelude::ObjectExt;
use gtk::prelude::{ApplicationExt, BoxExt, GtkWindowExt, ToggleButtonExt, WidgetExt};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use relm4::gtk::gdk;
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

        // Key cell height, scaled from the base 64px unit.
        let geometry_unit = (64.0 * config.scale).round() as i32;

        let floating_start = config
            .floating
            .then(|| setup_floating(&window, &config, &keyboard_definition, geometry_unit));
        if !config.floating {
            let bottom = matches!(config.position, Position::Bottom);
            window.set_anchor(Edge::Left, false);
            window.set_anchor(Edge::Right, false);
            window.set_anchor(Edge::Top, !bottom);
            window.set_anchor(Edge::Bottom, bottom);
            window.set_margin(if bottom { Edge::Bottom } else { Edge::Top }, config.margin);
        }
        window.set_opacity(config.opacity as f64);

        let container = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .build();
        window.set_child(Some(&container));
        container.set_align(gtk::Align::Center);
        container.set_expand(true);

        if let Some((start_pos, bounds)) = floating_start {
            let grip = gtk::Box::builder().css_classes(["mkeys-grip"]).build();
            attach_grip_drag(&grip, &window, start_pos, bounds);
            container.append(&grip);
        }

        keyboard_definition.layout.iter().for_each(|row| {
            let row_container = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .build();

            row.iter().for_each(|key| {
                let scan_code = key.scan_code;
                let width = (key.width.unwrap_or(1.0) * geometry_unit as f32).round() as i32;

                match key.key_type() {
                    KeyType::Mod => {
                        let toggle = gtk::ToggleButton::builder()
                            .label(format!(
                                "{} {}",
                                key.bottom_legend.clone().unwrap_or_default(),
                                key.top_legend.clone().unwrap_or_default()
                            ))
                            .width_request(width)
                            .height_request(geometry_unit)
                            .build();

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
                            .label(format!(
                                "{} {}",
                                key.bottom_legend.clone().unwrap_or_default(),
                                key.top_legend.clone().unwrap_or_default()
                            ))
                            .width_request(width)
                            .height_request(geometry_unit)
                            .build();

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
                            button.set_primary_content(key.top_legend.clone().unwrap_or_default());
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

/// Anchors the layer surface to (Left, Top) only — free absolute
/// positioning via margins, instead of docking to a screen edge — sets
/// the initial margins to a sensible starting spot (horizontally
/// centered, near the bottom like the docked default), and returns that
/// start position plus the (left, top) margin range that keeps the
/// keyboard fully on-screen, for [`attach_grip_drag`].
fn setup_floating(
    window: &gtk::Window,
    config: &Config,
    keyboard_definition: &LayoutDefinition,
    geometry_unit: i32,
) -> ((i32, i32), (i32, i32)) {
    window.set_anchor(Edge::Left, true);
    window.set_anchor(Edge::Top, true);
    window.set_anchor(Edge::Right, false);
    window.set_anchor(Edge::Bottom, false);

    let (kb_width, kb_height) = keyboard_pixel_size(keyboard_definition, geometry_unit);
    let (mon_w, mon_h) = primary_monitor_size();
    let start_left = ((mon_w - kb_width) / 2).max(0);
    let start_top = (mon_h - kb_height - config.margin).max(0);
    window.set_margin(Edge::Left, start_left);
    window.set_margin(Edge::Top, start_top);

    let bounds = ((mon_w - kb_width).max(0), (mon_h - kb_height).max(0));
    ((start_left, start_top), bounds)
}

/// Approximate on-screen size of the keyboard, for computing a starting
/// floating position and clamping the drag range — a few tens of pixels
/// off (button CSS margins, the grip bar) doesn't matter for either use.
fn keyboard_pixel_size(keyboard_definition: &LayoutDefinition, geometry_unit: i32) -> (i32, i32) {
    let width = (geometry_unit as f32 * keyboard_definition.width).round() as i32;
    let height = geometry_unit * keyboard_definition.height;
    (width.max(1), height.max(1))
}

/// The primary monitor's size in logical pixels, via `mctl get monitors`
/// (the first entry). Falls back to a common 1080p size on any failure —
/// only used to pick a reasonable floating start position and drag
/// bounds, never something that needs to be exact.
fn primary_monitor_size() -> (i32, i32) {
    const FALLBACK: (i32, i32) = (1920, 1080);
    (|| {
        let output = std::process::Command::new("mctl")
            .args(["get", "monitors"])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let v: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
        let mon = v.get("monitors")?.as_array()?.first()?;
        let w = mon.get("width")?.as_i64()?;
        let h = mon.get("height")?.as_i64()?;
        Some((w.max(1) as i32, h.max(1) as i32))
    })()
    .unwrap_or(FALLBACK)
}

/// Wires a `GestureDrag` on `grip` that moves `window`'s floating
/// position live, clamped to `bounds` (from [`setup_floating`]) so the
/// keyboard can't be dragged off-screen. Position is tracked in a local
/// `Cell` rather than read back from gtk4-layer-shell, seeded from
/// `start_pos` (the margins `setup_floating` already applied).
fn attach_grip_drag(
    grip: &gtk::Box,
    window: &gtk::Window,
    start_pos: (i32, i32),
    bounds: (i32, i32),
) {
    let pos = Rc::new(Cell::new(start_pos));
    let drag_start = Rc::new(Cell::new((0, 0)));

    grip.set_cursor_from_name(Some("grab"));

    let gesture = gtk::GestureDrag::new();
    gesture.set_button(gdk::BUTTON_PRIMARY);

    let grip_begin = grip.clone();
    let pos_begin = pos.clone();
    let start_begin = drag_start.clone();
    gesture.connect_drag_begin(move |_, _, _| {
        start_begin.set(pos_begin.get());
        grip_begin.set_cursor_from_name(Some("grabbing"));
    });

    let window_update = window.clone();
    let pos_update = pos.clone();
    let start_update = drag_start.clone();
    gesture.connect_drag_update(move |_, offset_x, offset_y| {
        let (base_left, base_top) = start_update.get();
        let new_left = (base_left + offset_x.round() as i32).clamp(0, bounds.0);
        let new_top = (base_top + offset_y.round() as i32).clamp(0, bounds.1);
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
