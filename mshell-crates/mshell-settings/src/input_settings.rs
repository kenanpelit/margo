//! Settings → Input.
//!
//! Keyboard, touchpad and mouse tunables. Unlike most settings pages these
//! live in the **compositor** config (margo's `config.conf`), not the shell
//! YAML — so reads parse the `.conf` via `margo-config` and writes patch the
//! `key = value` line in place, then fire `mctl reload` so the change
//! applies live without a logout (margo's `reload_config` re-applies xkb +
//! libinput settings on the fly).
//!
//! Text fields (xkb layout / variant / options) apply on Enter to avoid a
//! reload per keystroke; switches, dropdowns and spinners apply on change.

use crate::compositor_conf::{read_block, write_block};
use crate::row::Row;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::path::PathBuf;

/// `~/.config/margo/config.conf` (XDG-aware), the same file the wizard
/// patches — so both edit one source of truth.
fn conf_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join("margo").join("config.conf")
}

/// Parse the compositor config with first-party defaults applied so the
/// controls reflect the effective values. Falls back to defaults if the
/// file is missing or unparseable.
fn read_config() -> margo_config::Config {
    margo_config::parse_config_with_defaults(Some(&conf_path())).unwrap_or_default()
}

/// Patch `key = value` lines in `config.conf` in place, keeping everything
/// else (comments, layout, unrelated keys). A missing key is appended.
/// Mirrors the wizard / gestures patcher so they never fight over format.
fn patch_conf(updates: &[(&str, String)]) -> std::io::Result<()> {
    let path = conf_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let mut out = String::with_capacity(existing.len() + 64);
    let mut seen = vec![false; updates.len()];
    for line in existing.lines() {
        let t = line.trim_start();
        let mut handled = false;
        for (i, (key, val)) in updates.iter().enumerate() {
            if let Some(rest) = t.strip_prefix(*key)
                && rest.trim_start().starts_with('=')
            {
                seen[i] = true;
                out.push_str(&format!("{key} = {val}\n"));
                handled = true;
                break;
            }
        }
        if !handled {
            out.push_str(line);
            out.push('\n');
        }
    }
    for (i, (key, val)) in updates.iter().enumerate() {
        if !seen[i] {
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(&format!("{key} = {val}\n"));
        }
    }
    std::fs::write(&path, out)
}

/// Patch one key, then reload the compositor live. Logged, never panics.
fn apply(key: &str, value: String) {
    if let Err(e) = patch_conf(&[(key, value)]) {
        tracing::warn!(error = %e, key, "input: failed to write compositor config");
        return;
    }
    reload();
}

/// Spawn `mctl reload`, reaping the child asynchronously.
fn reload() {
    match std::process::Command::new("mctl").args(["reload"]).spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(e) => tracing::warn!(error = %e, "input: `mctl reload` failed to spawn"),
    }
}

fn bit(on: bool) -> String {
    if on { "1" } else { "0" }.to_string()
}

/// Motion names for the gesture builder dropdown; the chosen index maps to
/// this list and the name is written verbatim into the `gesturebind` line.
const MOTIONS: [&str; 8] = [
    "up",
    "down",
    "left",
    "right",
    "up-right",
    "up-left",
    "down-left",
    "down-right",
];
const FINGER_OPTS: [&str; 2] = ["3", "4"];
/// Mouse-button tokens the parser accepts (`parse_button`), in dropdown order.
const MB_BUTTONS: [&str; 3] = ["lmb", "mmb", "rmb"];
/// Scroll/axis direction tokens (`parse_axis_direction`), in dropdown order.
const AXIS_DIRS: [&str; 4] = ["up", "down", "left", "right"];

/// Every `gesturebind = <rest>` value in config.conf (the part after `=`),
/// in file order — these are richer than a single key=value so we round-trip
/// the raw text rather than the typed `GestureBinding`.
fn read_gesturebinds() -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(conf_path()) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|l| {
            let rest = l.trim_start().strip_prefix("gesturebind")?;
            Some(rest.trim_start().strip_prefix('=')?.trim().to_string())
        })
        .filter(|s| !s.is_empty())
        .collect()
}

/// Replace every `gesturebind = …` line in config.conf with the given set
/// (other lines untouched), then reload the compositor live.
fn write_gesturebinds(binds: &[String]) {
    let path = conf_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let mut out = String::with_capacity(existing.len() + 64);
    for line in existing.lines() {
        let is_bind = line
            .trim_start()
            .strip_prefix("gesturebind")
            .map(|r| r.trim_start().starts_with('='))
            .unwrap_or(false);
        if !is_bind {
            out.push_str(line);
            out.push('\n');
        }
    }
    for b in binds {
        out.push_str(&format!("gesturebind = {b}\n"));
    }
    if let Err(e) = std::fs::write(&path, out) {
        tracing::warn!(error = %e, "input: failed to write gesturebinds");
        return;
    }
    reload();
}

/// "none, up, 3, focusstack, +1" → "3-finger up · focusstack +1".
fn prettify_bind(raw: &str) -> String {
    let f: Vec<&str> = raw.split(',').map(|s| s.trim()).collect();
    if f.len() < 4 {
        return raw.to_string();
    }
    let (motion, fingers, action) = (f[1], f[2], f[3]);
    let arg = f.get(4..).map(|a| a.join(", ")).unwrap_or_default();
    let arg = if arg.is_empty() {
        String::new()
    } else {
        format!(" {arg}")
    };
    let mods = if f[0].is_empty() || f[0].eq_ignore_ascii_case("none") {
        String::new()
    } else {
        format!("{} + ", f[0])
    };
    format!("{mods}{fingers}-finger {motion} · {action}{arg}")
}

fn click_idx(m: margo_config::ClickMethod) -> u32 {
    match m {
        margo_config::ClickMethod::None => 0,
        margo_config::ClickMethod::ButtonAreas => 1,
        margo_config::ClickMethod::Clickfinger => 2,
    }
}

fn scroll_idx(m: margo_config::ScrollMethod) -> u32 {
    match m {
        margo_config::ScrollMethod::NoScroll => 0,
        margo_config::ScrollMethod::TwoFinger => 1,
        margo_config::ScrollMethod::Edge => 2,
        margo_config::ScrollMethod::OnButtonDown => 3,
    }
}

fn accel_idx(m: margo_config::AccelProfile) -> u32 {
    match m {
        margo_config::AccelProfile::None => 0,
        margo_config::AccelProfile::Flat => 1,
        margo_config::AccelProfile::Adaptive => 2,
    }
}

#[derive(Debug)]
pub(crate) struct InputSettingsModel {
    // Keyboard
    xkb_layout: String,
    xkb_variant: String,
    xkb_options: String,
    repeat_rate: i32,
    repeat_delay: i32,
    numlock_on: bool,
    // Touchpad
    tap_to_click: bool,
    tap_and_drag: bool,
    drag_lock: bool,
    natural_scroll: bool,
    disable_while_typing: bool,
    left_handed: bool,
    middle_emulation: bool,
    click_method: u32,
    scroll_method: u32,
    scroll_button: i32,
    send_events: u32,
    // Mouse
    mouse_natural: bool,
    accel_profile: u32,
    accel_speed: f64,
    // Swipe
    swipe_threshold: i32,
    // Gesture bindings (richer, multi-field — round-tripped as raw lines).
    binds: Vec<String>,
    binds_box: gtk::Box,
    b_modifiers: String,
    b_motion: u32,
    b_fingers: u32,
    b_action: String,
    b_arg: String,
    /// `Some(i)` while editing the existing gesture bind at index `i`;
    /// `None` while composing a brand-new one.
    editing: Option<usize>,
    /// Handles to the gesture-form widgets, so `EditBind` can load a row's
    /// values back into them (and `AddBind` can clear them after a save).
    b_motion_dd: gtk::DropDown,
    b_fingers_dd: gtk::DropDown,
    b_action_entry: gtk::Entry,
    b_arg_entry: gtk::Entry,
    b_modifiers_entry: gtk::Entry,
    // Dropdown models
    click_model: gtk::StringList,
    scroll_model: gtk::StringList,
    accel_model: gtk::StringList,
    sendevents_model: gtk::StringList,
    motion_model: gtk::StringList,
    fingers_model: gtk::StringList,
    // Mouse bindings (`mousebind = MODS, button, action, arg`).
    mbinds: Vec<String>,
    mbinds_box: gtk::Box,
    mb_modifiers: String,
    mb_button: u32,
    mb_action: String,
    mb_arg: String,
    // Scroll/axis bindings (`axisbind = MODS, direction, action, arg`).
    abinds: Vec<String>,
    abinds_box: gtk::Box,
    ab_modifiers: String,
    ab_direction: u32,
    ab_action: String,
    ab_arg: String,
    button_model: gtk::StringList,
    axis_dir_model: gtk::StringList,
}

#[derive(Debug)]
pub(crate) enum InputSettingsInput {
    SetLayout(String),
    SetVariant(String),
    SetOptions(String),
    SetRepeatRate(i32),
    SetRepeatDelay(i32),
    SetNumlock(bool),
    SetTapToClick(bool),
    SetTapAndDrag(bool),
    SetDragLock(bool),
    SetNaturalScroll(bool),
    SetDisableWhileTyping(bool),
    SetLeftHanded(bool),
    SetMiddleEmulation(bool),
    SetClickMethod(u32),
    SetScrollMethod(u32),
    SetScrollButton(i32),
    SetSendEvents(u32),
    SetMouseNatural(bool),
    SetAccelProfile(u32),
    SetAccelSpeed(f64),
    SetSwipeThreshold(i32),
    SetBModifiers(String),
    SetBMotion(u32),
    SetBFingers(u32),
    SetBAction(String),
    SetBArg(String),
    AddBind,
    RemoveBind(usize),
    EditBind(usize),
    CancelEdit,
    SetMbModifiers(String),
    SetMbButton(u32),
    SetMbAction(String),
    SetMbArg(String),
    AddMbind,
    RemoveMbind(usize),
    SetAbModifiers(String),
    SetAbDirection(u32),
    SetAbAction(String),
    SetAbArg(String),
    AddAbind,
    RemoveAbind(usize),
}

#[derive(Debug)]
pub(crate) enum InputSettingsOutput {}

pub(crate) struct InputSettingsInit {}

#[derive(Debug)]
pub(crate) enum InputSettingsCommandOutput {}

impl InputSettingsModel {
    /// Clear the gesture-add form to its empty defaults and leave edit mode.
    /// Called after a save / cancel so the form is ready for the next binding.
    fn reset_gesture_form(&mut self) {
        self.editing = None;
        self.b_modifiers = "none".to_string();
        self.b_motion = 0;
        self.b_fingers = 0;
        self.b_action.clear();
        self.b_arg.clear();
        self.b_modifiers_entry.set_text("none");
        self.b_action_entry.set_text("");
        self.b_arg_entry.set_text("");
        self.b_motion_dd.set_selected(0);
        self.b_fingers_dd.set_selected(0);
    }
}

#[relm4::component(pub)]
impl Component for InputSettingsModel {
    type CommandOutput = InputSettingsCommandOutput;
    type Input = InputSettingsInput;
    type Output = InputSettingsOutput;
    type Init = InputSettingsInit;

    view! {
        #[root]
        gtk::ScrolledWindow {
            set_vscrollbar_policy: gtk::PolicyType::Automatic,
            set_hscrollbar_policy: gtk::PolicyType::Never,
            set_propagate_natural_height: false,
            set_propagate_natural_width: false,
            set_hexpand: true,
            set_vexpand: true,

            gtk::Box {
                add_css_class: "settings-page",
                set_orientation: gtk::Orientation::Vertical,
                set_hexpand: true,
                set_spacing: 16,

                gtk::Box {
                    add_css_class: "settings-hero",
                    set_orientation: gtk::Orientation::Horizontal,
                    set_halign: gtk::Align::Start,
                    set_spacing: 16,
                    gtk::Image {
                        add_css_class: "settings-hero-icon",
                        set_icon_name: Some("input-keyboard-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "Input",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "Keyboard, touchpad and mouse. Applied to the compositor live.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                // ════════ Keyboard ════════
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Keyboard",
                    set_halign: gtk::Align::Start,
                },

                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Layout" },
                        #[template_child] desc {
                            set_label: "xkb layout the compositor loads (e.g. tr, us). Press Enter to apply.",
                        },
                        #[name = "layout_entry"]
                        gtk::Entry {
                            set_valign: gtk::Align::Center,
                            set_width_request: 200,
                            set_placeholder_text: Some("us"),
                            set_text: &model.xkb_layout,
                            connect_activate[sender] => move |e| {
                                sender.input(InputSettingsInput::SetLayout(e.text().to_string()));
                            },
                        },
                    },

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Variant" },
                        #[template_child] desc {
                            set_label: "xkb variant (e.g. f for Turkish-F). Blank for none. Enter to apply.",
                        },
                        #[name = "variant_entry"]
                        gtk::Entry {
                            set_valign: gtk::Align::Center,
                            set_width_request: 200,
                            set_placeholder_text: Some("(none)"),
                            set_text: &model.xkb_variant,
                            connect_activate[sender] => move |e| {
                                sender.input(InputSettingsInput::SetVariant(e.text().to_string()));
                            },
                        },
                    },

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Options" },
                        #[template_child] desc {
                            set_label: "xkb_rules_options, e.g. ctrl:nocaps (Caps→Ctrl). Enter to apply.",
                        },
                        #[name = "options_entry"]
                        gtk::Entry {
                            set_valign: gtk::Align::Center,
                            set_width_request: 200,
                            set_placeholder_text: Some("ctrl:nocaps"),
                            set_text: &model.xkb_options,
                            connect_activate[sender] => move |e| {
                                sender.input(InputSettingsInput::SetOptions(e.text().to_string()));
                            },
                        },
                    },

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Repeat rate" },
                        #[template_child] desc { set_label: "Key repeats per second once held." },
                        #[name = "repeat_rate_spin"]
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_range: (1.0, 100.0),
                            set_increments: (1.0, 5.0),
                            set_digits: 0,
                            #[block_signal(repeat_rate_handler)]
                            set_value: model.repeat_rate as f64,
                            connect_value_changed[sender] => move |s| {
                                sender.input(InputSettingsInput::SetRepeatRate(s.value() as i32));
                            } @repeat_rate_handler,
                        },
                    },

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Repeat delay" },
                        #[template_child] desc { set_label: "Milliseconds held before key repeat starts." },
                        #[name = "repeat_delay_spin"]
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_range: (100.0, 2000.0),
                            set_increments: (10.0, 50.0),
                            set_digits: 0,
                            #[block_signal(repeat_delay_handler)]
                            set_value: model.repeat_delay as f64,
                            connect_value_changed[sender] => move |s| {
                                sender.input(InputSettingsInput::SetRepeatDelay(s.value() as i32));
                            } @repeat_delay_handler,
                        },
                    },

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Num Lock on start" },
                        #[template_child] desc { set_label: "Enable Num Lock when the session starts." },
                        #[name = "numlock_switch"]
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[block_signal(numlock_handler)]
                            set_active: model.numlock_on,
                            connect_active_notify[sender] => move |s| {
                                sender.input(InputSettingsInput::SetNumlock(s.is_active()));
                            } @numlock_handler,
                        },
                    },
                },

                // ════════ Touchpad ════════
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Touchpad",
                    set_halign: gtk::Align::Start,
                    set_margin_top: 12,
                },

                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Tap to click" },
                        #[template_child] desc { set_label: "Register a tap as a click." },
                        #[name = "tap_switch"]
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[block_signal(tap_handler)]
                            set_active: model.tap_to_click,
                            connect_active_notify[sender] => move |s| {
                                sender.input(InputSettingsInput::SetTapToClick(s.is_active()));
                            } @tap_handler,
                        },
                    },

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Tap and drag" },
                        #[template_child] desc { set_label: "Tap then slide to drag without holding the button." },
                        #[name = "tap_drag_switch"]
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[block_signal(tap_drag_handler)]
                            set_active: model.tap_and_drag,
                            connect_active_notify[sender] => move |s| {
                                sender.input(InputSettingsInput::SetTapAndDrag(s.is_active()));
                            } @tap_drag_handler,
                        },
                    },

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Drag lock" },
                        #[template_child] desc { set_label: "Keep dragging after lifting the finger until the next tap." },
                        #[name = "drag_lock_switch"]
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[block_signal(drag_lock_handler)]
                            set_active: model.drag_lock,
                            connect_active_notify[sender] => move |s| {
                                sender.input(InputSettingsInput::SetDragLock(s.is_active()));
                            } @drag_lock_handler,
                        },
                    },

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Natural scrolling" },
                        #[template_child] desc { set_label: "Content follows the fingers (reverse of the classic direction)." },
                        #[name = "natural_switch"]
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[block_signal(natural_handler)]
                            set_active: model.natural_scroll,
                            connect_active_notify[sender] => move |s| {
                                sender.input(InputSettingsInput::SetNaturalScroll(s.is_active()));
                            } @natural_handler,
                        },
                    },

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Disable while typing" },
                        #[template_child] desc { set_label: "Ignore the touchpad briefly after a keypress." },
                        #[name = "dwt_switch"]
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[block_signal(dwt_handler)]
                            set_active: model.disable_while_typing,
                            connect_active_notify[sender] => move |s| {
                                sender.input(InputSettingsInput::SetDisableWhileTyping(s.is_active()));
                            } @dwt_handler,
                        },
                    },

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Left-handed" },
                        #[template_child] desc { set_label: "Swap the primary and secondary buttons." },
                        #[name = "lefthand_switch"]
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[block_signal(lefthand_handler)]
                            set_active: model.left_handed,
                            connect_active_notify[sender] => move |s| {
                                sender.input(InputSettingsInput::SetLeftHanded(s.is_active()));
                            } @lefthand_handler,
                        },
                    },

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Middle-button emulation" },
                        #[template_child] desc { set_label: "Press left + right together to emulate the middle button." },
                        #[name = "middle_switch"]
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[block_signal(middle_handler)]
                            set_active: model.middle_emulation,
                            connect_active_notify[sender] => move |s| {
                                sender.input(InputSettingsInput::SetMiddleEmulation(s.is_active()));
                            } @middle_handler,
                        },
                    },

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Click method" },
                        #[template_child] desc { set_label: "How button clicks are detected (clickfinger = tap zones by finger count)." },
                        #[name = "click_dd"]
                        gtk::DropDown {
                            set_valign: gtk::Align::Center,
                            set_width_request: 200,
                            set_model: Some(&model.click_model),
                            #[block_signal(click_handler)]
                            set_selected: model.click_method,
                            connect_selected_notify[sender] => move |d| {
                                sender.input(InputSettingsInput::SetClickMethod(d.selected()));
                            } @click_handler,
                        },
                    },

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Scroll method" },
                        #[template_child] desc { set_label: "How scrolling is detected." },
                        #[name = "scroll_dd"]
                        gtk::DropDown {
                            set_valign: gtk::Align::Center,
                            set_width_request: 200,
                            set_model: Some(&model.scroll_model),
                            #[block_signal(scroll_handler)]
                            set_selected: model.scroll_method,
                            connect_selected_notify[sender] => move |d| {
                                sender.input(InputSettingsInput::SetScrollMethod(d.selected()));
                            } @scroll_handler,
                        },
                    },

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Scroll button" },
                        #[template_child] desc { set_label: "Button code used for on-button scrolling (e.g. 274 = middle)." },
                        #[name = "scroll_button_spin"]
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_range: (0.0, 400.0),
                            set_increments: (1.0, 10.0),
                            set_digits: 0,
                            #[block_signal(scroll_button_handler)]
                            set_value: model.scroll_button as f64,
                            connect_value_changed[sender] => move |s| {
                                sender.input(InputSettingsInput::SetScrollButton(s.value() as i32));
                            } @scroll_button_handler,
                        },
                    },

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Send events" },
                        #[template_child] desc { set_label: "Whether the touchpad sends events (e.g. disable when an external mouse is plugged in)." },
                        #[name = "sendevents_dd"]
                        gtk::DropDown {
                            set_valign: gtk::Align::Center,
                            set_width_request: 200,
                            set_model: Some(&model.sendevents_model),
                            #[block_signal(sendevents_handler)]
                            set_selected: model.send_events,
                            connect_selected_notify[sender] => move |d| {
                                sender.input(InputSettingsInput::SetSendEvents(d.selected()));
                            } @sendevents_handler,
                        },
                    },
                },

                // ════════ Mouse ════════
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Mouse",
                    set_halign: gtk::Align::Start,
                    set_margin_top: 12,
                },

                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Natural scrolling" },
                        #[template_child] desc { set_label: "Reverse the mouse-wheel scroll direction." },
                        #[name = "mouse_natural_switch"]
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[block_signal(mouse_natural_handler)]
                            set_active: model.mouse_natural,
                            connect_active_notify[sender] => move |s| {
                                sender.input(InputSettingsInput::SetMouseNatural(s.is_active()));
                            } @mouse_natural_handler,
                        },
                    },

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Acceleration profile" },
                        #[template_child] desc { set_label: "Pointer acceleration curve (applies to mouse + touchpad)." },
                        #[name = "accel_dd"]
                        gtk::DropDown {
                            set_valign: gtk::Align::Center,
                            set_width_request: 200,
                            set_model: Some(&model.accel_model),
                            #[block_signal(accel_handler)]
                            set_selected: model.accel_profile,
                            connect_selected_notify[sender] => move |d| {
                                sender.input(InputSettingsInput::SetAccelProfile(d.selected()));
                            } @accel_handler,
                        },
                    },

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Acceleration speed" },
                        #[template_child] desc { set_label: "-1.0 (slowest) … 1.0 (fastest)." },
                        #[name = "accel_speed_spin"]
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_range: (-1.0, 1.0),
                            set_increments: (0.05, 0.25),
                            set_digits: 2,
                            #[block_signal(accel_speed_handler)]
                            set_value: model.accel_speed,
                            connect_value_changed[sender] => move |s| {
                                sender.input(InputSettingsInput::SetAccelSpeed(s.value()));
                            } @accel_speed_handler,
                        },
                    },
                },

                // ════════ Swipe ════════
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Swipe",
                    set_halign: gtk::Align::Start,
                    set_margin_top: 12,
                },

                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Swipe sensitivity" },
                        #[template_child] desc { set_label: "Minimum travel before a multi-finger swipe fires. Lower = more sensitive." },
                        #[name = "threshold_spin"]
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_range: (1.0, 100.0),
                            set_increments: (1.0, 5.0),
                            set_digits: 0,
                            #[block_signal(threshold_handler)]
                            set_value: model.swipe_threshold as f64,
                            connect_value_changed[sender] => move |s| {
                                sender.input(InputSettingsInput::SetSwipeThreshold(s.value() as i32));
                            } @threshold_handler,
                        },
                    },
                },

                // ════════ Gesture bindings ════════
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Gesture bindings",
                    set_halign: gtk::Align::Start,
                    set_margin_top: 12,
                },
                gtk::Label {
                    add_css_class: "label-small",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_label: "Map a multi-finger swipe to a compositor action (gesturebind). Applied live.",
                },

                #[local_ref]
                binds_box -> gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 6,
                },

                gtk::Label {
                    add_css_class: "label-medium-bold",
                    set_label: "Add a binding",
                    set_halign: gtk::Align::Start,
                    set_margin_top: 8,
                },

                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Direction" },
                        #[template_child] desc { set_label: "Swipe direction." },
                        #[name = "motion_dd"]
                        gtk::DropDown {
                            set_valign: gtk::Align::Center,
                            set_width_request: 200,
                            set_model: Some(&model.motion_model),
                            #[block_signal(motion_handler)]
                            set_selected: model.b_motion,
                            connect_selected_notify[sender] => move |d| {
                                sender.input(InputSettingsInput::SetBMotion(d.selected()));
                            } @motion_handler,
                        },
                    },

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Fingers" },
                        #[template_child] desc { set_label: "Number of fingers on the swipe." },
                        #[name = "fingers_dd"]
                        gtk::DropDown {
                            set_valign: gtk::Align::Center,
                            set_width_request: 200,
                            set_model: Some(&model.fingers_model),
                            #[block_signal(fingers_handler)]
                            set_selected: model.b_fingers,
                            connect_selected_notify[sender] => move |d| {
                                sender.input(InputSettingsInput::SetBFingers(d.selected()));
                            } @fingers_handler,
                        },
                    },

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Action" },
                        #[template_child] desc { set_label: "Dispatch name, e.g. focusstack, spawn, view, togglefloating." },
                        #[name = "action_entry"]
                        gtk::Entry {
                            set_valign: gtk::Align::Center,
                            set_width_request: 200,
                            set_placeholder_text: Some("focusstack"),
                            connect_changed[sender] => move |e| {
                                sender.input(InputSettingsInput::SetBAction(e.text().to_string()));
                            },
                        },
                    },

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Argument" },
                        #[template_child] desc { set_label: "Optional argument for the action (e.g. +1, or a command for spawn)." },
                        #[name = "arg_entry"]
                        gtk::Entry {
                            set_valign: gtk::Align::Center,
                            set_width_request: 200,
                            set_placeholder_text: Some("(optional)"),
                            connect_changed[sender] => move |e| {
                                sender.input(InputSettingsInput::SetBArg(e.text().to_string()));
                            },
                        },
                    },

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Modifiers" },
                        #[template_child] desc { set_label: "Held key(s); usually none (e.g. super)." },
                        #[name = "modifiers_entry"]
                        gtk::Entry {
                            set_valign: gtk::Align::Center,
                            set_width_request: 200,
                            set_text: "none",
                            connect_changed[sender] => move |e| {
                                sender.input(InputSettingsInput::SetBModifiers(e.text().to_string()));
                            },
                        },
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 8,
                    set_margin_top: 4,
                    gtk::Button {
                        add_css_class: "ok-button-surface",
                        #[watch]
                        set_label: if model.editing.is_some() { "Save changes" } else { "Add gesture binding" },
                        connect_clicked[sender] => move |_| {
                            sender.input(InputSettingsInput::AddBind);
                        },
                    },
                    gtk::Button {
                        add_css_class: "ok-button-surface",
                        set_label: "Cancel",
                        #[watch]
                        set_visible: model.editing.is_some(),
                        connect_clicked[sender] => move |_| {
                            sender.input(InputSettingsInput::CancelEdit);
                        },
                    },
                },

                // ════════ Mouse bindings ════════
                gtk::Separator {
                    set_orientation: gtk::Orientation::Horizontal,
                    add_css_class: "settings-section-sep",
                    set_margin_top: 16,
                    set_margin_bottom: 8,
                },
                gtk::Label { add_css_class: "label-large-bold", set_label: "Mouse bindings", set_halign: gtk::Align::Start },
                gtk::Label { add_css_class: "label-small", set_halign: gtk::Align::Start, set_xalign: 0.0, set_wrap: true,
                    set_label: "Bind a (modifier +) mouse button to a compositor action (mousebind). Applied live." },
                #[local_ref]
                mbinds_box -> gtk::Box { set_orientation: gtk::Orientation::Vertical, set_spacing: 6 },
                gtk::Label { add_css_class: "label-medium-bold", set_label: "Add a binding", set_halign: gtk::Align::Start, set_margin_top: 8 },
                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    #[template] Row {
                        #[template_child] title { set_label: "Button" },
                        gtk::DropDown { set_valign: gtk::Align::Center, set_width_request: 200, set_model: Some(&model.button_model),
                            connect_selected_notify[sender] => move |d| sender.input(InputSettingsInput::SetMbButton(d.selected())) } },
                    #[template] Row {
                        #[template_child] title { set_label: "Action" },
                        #[template_child] desc { set_label: "Dispatch name, e.g. killclient, togglefloating, spawn." },
                        gtk::Entry { set_valign: gtk::Align::Center, set_width_request: 200, set_placeholder_text: Some("killclient"),
                            connect_changed[sender] => move |e| sender.input(InputSettingsInput::SetMbAction(e.text().to_string())) } },
                    #[template] Row {
                        #[template_child] title { set_label: "Argument" },
                        #[template_child] desc { set_label: "Optional argument for the action." },
                        gtk::Entry { set_valign: gtk::Align::Center, set_width_request: 200, set_placeholder_text: Some("(optional)"),
                            connect_changed[sender] => move |e| sender.input(InputSettingsInput::SetMbArg(e.text().to_string())) } },
                    #[template] Row {
                        #[template_child] title { set_label: "Modifiers" },
                        #[template_child] desc { set_label: "Held key(s), e.g. super (use none with care — it grabs every click)." },
                        gtk::Entry { set_valign: gtk::Align::Center, set_width_request: 200, set_text: "super",
                            connect_changed[sender] => move |e| sender.input(InputSettingsInput::SetMbModifiers(e.text().to_string())) } },
                },
                gtk::Button { add_css_class: "ok-button-surface", set_label: "Add mouse binding", set_margin_top: 4,
                    set_halign: gtk::Align::Start,
                    connect_clicked[sender] => move |_| sender.input(InputSettingsInput::AddMbind) },

                // ════════ Scroll (axis) bindings ════════
                gtk::Separator {
                    set_orientation: gtk::Orientation::Horizontal,
                    add_css_class: "settings-section-sep",
                    set_margin_top: 16,
                    set_margin_bottom: 8,
                },
                gtk::Label { add_css_class: "label-large-bold", set_label: "Scroll bindings", set_halign: gtk::Align::Start },
                gtk::Label { add_css_class: "label-small", set_halign: gtk::Align::Start, set_xalign: 0.0, set_wrap: true,
                    set_label: "Bind a (modifier +) scroll direction to a compositor action (axisbind). Applied live." },
                #[local_ref]
                abinds_box -> gtk::Box { set_orientation: gtk::Orientation::Vertical, set_spacing: 6 },
                gtk::Label { add_css_class: "label-medium-bold", set_label: "Add a binding", set_halign: gtk::Align::Start, set_margin_top: 8 },
                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    #[template] Row {
                        #[template_child] title { set_label: "Direction" },
                        gtk::DropDown { set_valign: gtk::Align::Center, set_width_request: 200, set_model: Some(&model.axis_dir_model),
                            connect_selected_notify[sender] => move |d| sender.input(InputSettingsInput::SetAbDirection(d.selected())) } },
                    #[template] Row {
                        #[template_child] title { set_label: "Action" },
                        #[template_child] desc { set_label: "Dispatch name, e.g. focusstack, view." },
                        gtk::Entry { set_valign: gtk::Align::Center, set_width_request: 200, set_placeholder_text: Some("focusstack"),
                            connect_changed[sender] => move |e| sender.input(InputSettingsInput::SetAbAction(e.text().to_string())) } },
                    #[template] Row {
                        #[template_child] title { set_label: "Argument" },
                        #[template_child] desc { set_label: "Optional argument." },
                        gtk::Entry { set_valign: gtk::Align::Center, set_width_request: 200, set_placeholder_text: Some("(optional)"),
                            connect_changed[sender] => move |e| sender.input(InputSettingsInput::SetAbArg(e.text().to_string())) } },
                    #[template] Row {
                        #[template_child] title { set_label: "Modifiers" },
                        #[template_child] desc { set_label: "Held key(s), e.g. super (or none)." },
                        gtk::Entry { set_valign: gtk::Align::Center, set_width_request: 200, set_text: "super",
                            connect_changed[sender] => move |e| sender.input(InputSettingsInput::SetAbModifiers(e.text().to_string())) } },
                },
                gtk::Button { add_css_class: "ok-button-surface", set_label: "Add scroll binding", set_margin_top: 4,
                    set_halign: gtk::Align::Start,
                    connect_clicked[sender] => move |_| sender.input(InputSettingsInput::AddAbind) },
            }
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let cfg = read_config();
        // The gesture-binding list is rebuilt imperatively, so its container
        // is created here and shared into the model (the view binds it via
        // `#[local_ref]`).
        let binds_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let binds = read_gesturebinds();
        let mbinds_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let abinds_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let mut model = InputSettingsModel {
            xkb_layout: cfg.xkb_rules.layout.clone(),
            xkb_variant: cfg.xkb_rules.variant.clone(),
            xkb_options: cfg.xkb_rules.options.clone(),
            repeat_rate: cfg.repeat_rate,
            repeat_delay: cfg.repeat_delay,
            numlock_on: cfg.numlock_on,
            tap_to_click: cfg.tap_to_click,
            tap_and_drag: cfg.tap_and_drag,
            drag_lock: cfg.drag_lock,
            natural_scroll: cfg.trackpad_natural_scrolling,
            disable_while_typing: cfg.disable_while_typing,
            left_handed: cfg.left_handed,
            middle_emulation: cfg.middle_button_emulation,
            click_method: click_idx(cfg.click_method),
            scroll_method: scroll_idx(cfg.scroll_method),
            scroll_button: cfg.scroll_button as i32,
            send_events: cfg.send_events_mode.min(2),
            mouse_natural: cfg.mouse_natural_scrolling,
            accel_profile: accel_idx(cfg.mouse_accel_profile),
            accel_speed: cfg.mouse_accel_speed,
            swipe_threshold: cfg.swipe_min_threshold as i32,
            binds,
            binds_box: binds_box.clone(),
            b_modifiers: "none".to_string(),
            b_motion: 0,
            b_fingers: 0,
            b_action: String::new(),
            b_arg: String::new(),
            editing: None,
            // Placeholders; replaced with the real form widgets right after
            // `view_output!()` builds them (see below).
            b_motion_dd: gtk::DropDown::from_strings(&[]),
            b_fingers_dd: gtk::DropDown::from_strings(&[]),
            b_action_entry: gtk::Entry::new(),
            b_arg_entry: gtk::Entry::new(),
            b_modifiers_entry: gtk::Entry::new(),
            click_model: gtk::StringList::new(&["None", "Button areas", "Clickfinger"]),
            scroll_model: gtk::StringList::new(&["Disabled", "Two-finger", "Edge", "On-button"]),
            accel_model: gtk::StringList::new(&["None", "Flat", "Adaptive"]),
            sendevents_model: gtk::StringList::new(&[
                "Enabled",
                "Disabled",
                "Disabled on external mouse",
            ]),
            motion_model: gtk::StringList::new(&MOTIONS),
            fingers_model: gtk::StringList::new(&FINGER_OPTS),
            mbinds: read_block("mousebind"),
            mbinds_box: mbinds_box.clone(),
            mb_modifiers: "super".to_string(),
            mb_button: 0,
            mb_action: String::new(),
            mb_arg: String::new(),
            abinds: read_block("axisbind"),
            abinds_box: abinds_box.clone(),
            ab_modifiers: "super".to_string(),
            ab_direction: 0,
            ab_action: String::new(),
            ab_arg: String::new(),
            button_model: gtk::StringList::new(&["Left button", "Middle button", "Right button"]),
            axis_dir_model: gtk::StringList::new(&["Up", "Down", "Left", "Right"]),
        };
        let widgets = view_output!();
        // Grab the real form widgets so EditBind can repopulate them.
        model.b_motion_dd = widgets.motion_dd.clone();
        model.b_fingers_dd = widgets.fingers_dd.clone();
        model.b_action_entry = widgets.action_entry.clone();
        model.b_arg_entry = widgets.arg_entry.clone();
        model.b_modifiers_entry = widgets.modifiers_entry.clone();
        let _ = root;
        rebuild_binds(&model.binds_box, &model.binds, &sender);
        rebuild_raw_binds(
            &model.mbinds_box,
            &model.mbinds,
            "No mouse bindings yet.",
            InputSettingsInput::RemoveMbind,
            &sender,
        );
        rebuild_raw_binds(
            &model.abinds_box,
            &model.abinds,
            "No scroll bindings yet.",
            InputSettingsInput::RemoveAbind,
            &sender,
        );
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            InputSettingsInput::SetLayout(v) => {
                self.xkb_layout = v.trim().to_string();
                apply("xkb_rules_layout", self.xkb_layout.clone());
            }
            InputSettingsInput::SetVariant(v) => {
                self.xkb_variant = v.trim().to_string();
                apply("xkb_rules_variant", self.xkb_variant.clone());
            }
            InputSettingsInput::SetOptions(v) => {
                self.xkb_options = v.trim().to_string();
                apply("xkb_rules_options", self.xkb_options.clone());
            }
            InputSettingsInput::SetRepeatRate(v) => {
                self.repeat_rate = v;
                apply("repeat_rate", v.to_string());
            }
            InputSettingsInput::SetRepeatDelay(v) => {
                self.repeat_delay = v;
                apply("repeat_delay", v.to_string());
            }
            InputSettingsInput::SetNumlock(v) => {
                self.numlock_on = v;
                apply("numlockon", bit(v));
            }
            InputSettingsInput::SetTapToClick(v) => {
                self.tap_to_click = v;
                apply("tap_to_click", bit(v));
            }
            InputSettingsInput::SetTapAndDrag(v) => {
                self.tap_and_drag = v;
                apply("tap_and_drag", bit(v));
            }
            InputSettingsInput::SetDragLock(v) => {
                self.drag_lock = v;
                apply("drag_lock", bit(v));
            }
            InputSettingsInput::SetNaturalScroll(v) => {
                self.natural_scroll = v;
                apply("trackpad_natural_scrolling", bit(v));
            }
            InputSettingsInput::SetDisableWhileTyping(v) => {
                self.disable_while_typing = v;
                apply("disable_while_typing", bit(v));
            }
            InputSettingsInput::SetLeftHanded(v) => {
                self.left_handed = v;
                apply("left_handed", bit(v));
            }
            InputSettingsInput::SetMiddleEmulation(v) => {
                self.middle_emulation = v;
                apply("middle_button_emulation", bit(v));
            }
            InputSettingsInput::SetClickMethod(v) => {
                self.click_method = v;
                apply("click_method", v.to_string());
            }
            InputSettingsInput::SetScrollMethod(v) => {
                self.scroll_method = v;
                apply("scroll_method", v.to_string());
            }
            InputSettingsInput::SetScrollButton(v) => {
                let v = v.max(0);
                self.scroll_button = v;
                apply("scroll_button", v.to_string());
            }
            InputSettingsInput::SetSendEvents(v) => {
                self.send_events = v;
                apply("send_events_mode", v.to_string());
            }
            InputSettingsInput::SetMouseNatural(v) => {
                self.mouse_natural = v;
                apply("mouse_natural_scrolling", bit(v));
            }
            InputSettingsInput::SetAccelProfile(v) => {
                self.accel_profile = v;
                // Legacy unified key — margo applies it to mouse + touchpad.
                apply("accel_profile", v.to_string());
            }
            InputSettingsInput::SetAccelSpeed(v) => {
                let v = v.clamp(-1.0, 1.0);
                self.accel_speed = v;
                apply("accel_speed", format!("{v:.2}"));
            }
            InputSettingsInput::SetSwipeThreshold(v) => {
                let v = v.max(1);
                self.swipe_threshold = v;
                apply("swipe_min_threshold", v.to_string());
            }
            InputSettingsInput::SetBModifiers(s) => self.b_modifiers = s,
            InputSettingsInput::SetBMotion(v) => self.b_motion = v,
            InputSettingsInput::SetBFingers(v) => self.b_fingers = v,
            InputSettingsInput::SetBAction(s) => self.b_action = s.trim().to_string(),
            InputSettingsInput::SetBArg(s) => self.b_arg = s.trim().to_string(),
            InputSettingsInput::AddBind => {
                let action = self.b_action.trim().to_string();
                if action.is_empty() {
                    return; // an action is required
                }
                let motion = MOTIONS.get(self.b_motion as usize).copied().unwrap_or("up");
                let fingers = FINGER_OPTS
                    .get(self.b_fingers as usize)
                    .copied()
                    .unwrap_or("3");
                let mods = match self.b_modifiers.trim() {
                    "" => "none",
                    m => m,
                };
                let mut line = format!("{mods}, {motion}, {fingers}, {action}");
                let arg = self.b_arg.trim();
                if !arg.is_empty() {
                    line.push_str(&format!(", {arg}"));
                }
                // Editing an existing row replaces it in place; otherwise append.
                match self.editing.take() {
                    Some(idx) if idx < self.binds.len() => self.binds[idx] = line,
                    _ => self.binds.push(line),
                }
                write_gesturebinds(&self.binds);
                rebuild_binds(&self.binds_box, &self.binds, &sender);
                self.reset_gesture_form();
            }
            InputSettingsInput::RemoveBind(i) => {
                if i < self.binds.len() {
                    self.binds.remove(i);
                    write_gesturebinds(&self.binds);
                    rebuild_binds(&self.binds_box, &self.binds, &sender);
                    // If the row being edited vanished/shifted, leave edit mode.
                    if let Some(e) = self.editing
                        && e >= self.binds.len()
                    {
                        self.reset_gesture_form();
                    }
                }
            }
            InputSettingsInput::EditBind(i) => {
                let Some(raw) = self.binds.get(i).cloned() else {
                    return;
                };
                let f: Vec<String> = raw.split(',').map(|s| s.trim().to_string()).collect();
                if f.len() < 4 {
                    return;
                }
                let mods = if f[0].is_empty() || f[0].eq_ignore_ascii_case("none") {
                    "none".to_string()
                } else {
                    f[0].clone()
                };
                let motion = MOTIONS.iter().position(|m| *m == f[1]).unwrap_or(0) as u32;
                let fingers = FINGER_OPTS.iter().position(|x| *x == f[2]).unwrap_or(0) as u32;
                let action = f[3].clone();
                let arg = f.get(4..).map(|a| a.join(", ")).unwrap_or_default();
                self.b_modifiers = mods.clone();
                self.b_motion = motion;
                self.b_fingers = fingers;
                self.b_action = action.clone();
                self.b_arg = arg.clone();
                self.editing = Some(i);
                // Load the values into the form widgets.
                self.b_modifiers_entry.set_text(&mods);
                self.b_action_entry.set_text(&action);
                self.b_arg_entry.set_text(&arg);
                self.b_motion_dd.set_selected(motion);
                self.b_fingers_dd.set_selected(fingers);
            }
            InputSettingsInput::CancelEdit => self.reset_gesture_form(),
            InputSettingsInput::SetMbModifiers(s) => self.mb_modifiers = s,
            InputSettingsInput::SetMbButton(v) => self.mb_button = v,
            InputSettingsInput::SetMbAction(s) => self.mb_action = s.trim().to_string(),
            InputSettingsInput::SetMbArg(s) => self.mb_arg = s.trim().to_string(),
            InputSettingsInput::AddMbind => {
                let action = self.mb_action.trim().to_string();
                if action.is_empty() {
                    return;
                }
                let button = MB_BUTTONS
                    .get(self.mb_button as usize)
                    .copied()
                    .unwrap_or("rmb");
                let mods = match self.mb_modifiers.trim() {
                    "" => "none",
                    m => m,
                };
                let mut line = format!("{mods}, {button}, {action}");
                let arg = self.mb_arg.trim();
                if !arg.is_empty() {
                    line.push_str(&format!(", {arg}"));
                }
                self.mbinds.push(line);
                write_block("mousebind", &self.mbinds);
                rebuild_raw_binds(
                    &self.mbinds_box,
                    &self.mbinds,
                    "No mouse bindings yet.",
                    InputSettingsInput::RemoveMbind,
                    &sender,
                );
            }
            InputSettingsInput::RemoveMbind(i) => {
                if i < self.mbinds.len() {
                    self.mbinds.remove(i);
                    write_block("mousebind", &self.mbinds);
                    rebuild_raw_binds(
                        &self.mbinds_box,
                        &self.mbinds,
                        "No mouse bindings yet.",
                        InputSettingsInput::RemoveMbind,
                        &sender,
                    );
                }
            }
            InputSettingsInput::SetAbModifiers(s) => self.ab_modifiers = s,
            InputSettingsInput::SetAbDirection(v) => self.ab_direction = v,
            InputSettingsInput::SetAbAction(s) => self.ab_action = s.trim().to_string(),
            InputSettingsInput::SetAbArg(s) => self.ab_arg = s.trim().to_string(),
            InputSettingsInput::AddAbind => {
                let action = self.ab_action.trim().to_string();
                if action.is_empty() {
                    return;
                }
                let dir = AXIS_DIRS
                    .get(self.ab_direction as usize)
                    .copied()
                    .unwrap_or("up");
                let mods = match self.ab_modifiers.trim() {
                    "" => "none",
                    m => m,
                };
                let mut line = format!("{mods}, {dir}, {action}");
                let arg = self.ab_arg.trim();
                if !arg.is_empty() {
                    line.push_str(&format!(", {arg}"));
                }
                self.abinds.push(line);
                write_block("axisbind", &self.abinds);
                rebuild_raw_binds(
                    &self.abinds_box,
                    &self.abinds,
                    "No scroll bindings yet.",
                    InputSettingsInput::RemoveAbind,
                    &sender,
                );
            }
            InputSettingsInput::RemoveAbind(i) => {
                if i < self.abinds.len() {
                    self.abinds.remove(i);
                    write_block("axisbind", &self.abinds);
                    rebuild_raw_binds(
                        &self.abinds_box,
                        &self.abinds,
                        "No scroll bindings yet.",
                        InputSettingsInput::RemoveAbind,
                        &sender,
                    );
                }
            }
        }
    }
}

/// Rebuild the gesture-binding rows in `binds_box` from `binds`. Run on init
/// and after every add/remove so each Remove button captures the right
/// current index.
fn rebuild_binds(
    binds_box: &gtk::Box,
    binds: &[String],
    sender: &ComponentSender<InputSettingsModel>,
) {
    while let Some(child) = binds_box.first_child() {
        binds_box.remove(&child);
    }
    if binds.is_empty() {
        let empty = gtk::Label::new(Some("No gesture bindings yet."));
        empty.add_css_class("label-small");
        empty.set_halign(gtk::Align::Start);
        empty.set_xalign(0.0);
        binds_box.append(&empty);
        return;
    }
    for (i, bind) in binds.iter().enumerate() {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let label = gtk::Label::new(Some(&prettify_bind(bind)));
        label.add_css_class("label-medium");
        label.set_halign(gtk::Align::Start);
        label.set_xalign(0.0);
        label.set_hexpand(true);
        label.set_wrap(true);
        row.append(&label);
        let edit = gtk::Button::with_label("Edit");
        edit.add_css_class("ok-button-surface");
        edit.set_valign(gtk::Align::Center);
        let se = sender.clone();
        edit.connect_clicked(move |_| se.input(InputSettingsInput::EditBind(i)));
        row.append(&edit);
        let remove = gtk::Button::with_label("Remove");
        remove.add_css_class("ok-button-surface");
        remove.set_valign(gtk::Align::Center);
        let s = sender.clone();
        remove.connect_clicked(move |_| s.input(InputSettingsInput::RemoveBind(i)));
        row.append(&remove);
        binds_box.append(&row);
    }
}

/// Rebuild a raw-line bind list (mouse / axis) into `box_`. The raw payload is
/// shown verbatim; `ctor` builds the Remove message carrying the row index.
fn rebuild_raw_binds(
    box_: &gtk::Box,
    binds: &[String],
    empty_msg: &str,
    ctor: fn(usize) -> InputSettingsInput,
    sender: &ComponentSender<InputSettingsModel>,
) {
    while let Some(child) = box_.first_child() {
        box_.remove(&child);
    }
    if binds.is_empty() {
        let empty = gtk::Label::new(Some(empty_msg));
        empty.add_css_class("label-small");
        empty.set_halign(gtk::Align::Start);
        empty.set_xalign(0.0);
        box_.append(&empty);
        return;
    }
    for (i, bind) in binds.iter().enumerate() {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let label = gtk::Label::new(Some(bind));
        label.add_css_class("label-medium");
        label.set_halign(gtk::Align::Start);
        label.set_xalign(0.0);
        label.set_hexpand(true);
        label.set_wrap(true);
        row.append(&label);
        let remove = gtk::Button::with_label("Remove");
        remove.add_css_class("ok-button-surface");
        remove.set_valign(gtk::Align::Center);
        let s = sender.clone();
        remove.connect_clicked(move |_| s.input(ctor(i)));
        row.append(&remove);
        box_.append(&row);
    }
}
