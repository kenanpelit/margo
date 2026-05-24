//! Settings → Gestures.
//!
//! Touchpad + swipe gesture knobs. Unlike most settings pages these live
//! in the **compositor** config (margo's `config.conf`), not the shell
//! YAML — so reads parse the `.conf` via `margo-config` and writes patch
//! the `key = value` line in place, then fire `mctl config reload` so the
//! change applies live without a logout. Swipe→action mappings
//! (`gesturebind` lines) are richer than a single key=value and stay in
//! `config.conf` for now; this page tunes the touchpad behaviour + swipe
//! sensitivity those gestures ride on.

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

/// Parse the compositor config (with first-party defaults applied) so the
/// controls reflect the effective values. Falls back to defaults if the
/// file is missing or unparseable.
fn read_config() -> margo_config::Config {
    margo_config::parse_config_with_defaults(Some(&conf_path())).unwrap_or_default()
}

/// Patch `key = value` lines in `config.conf` in place, keeping everything
/// else (comments, layout, unrelated keys). A missing key is appended.
/// Mirrors the wizard's patcher so the two never fight over formatting.
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
            // `strip_prefix` + a `=` after optional whitespace guards
            // against prefix collisions (`swipe_min_threshold` won't eat a
            // hypothetical `swipe_min_threshold_x` line).
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
        tracing::warn!(error = %e, key, "gestures: failed to write compositor config");
        return;
    }
    match std::process::Command::new("mctl")
        .args(["config", "reload"])
        .spawn()
    {
        Ok(mut child) => {
            // Reap asynchronously so we don't leave a zombie.
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(e) => tracing::warn!(error = %e, "gestures: `mctl config reload` failed to spawn"),
    }
}

fn bit(on: bool) -> String {
    if on { "1" } else { "0" }.to_string()
}

#[derive(Debug)]
pub(crate) struct GesturesSettingsModel {
    tap_to_click: bool,
    tap_and_drag: bool,
    drag_lock: bool,
    natural_scroll: bool,
    disable_while_typing: bool,
    swipe_threshold: i32,
}

#[derive(Debug)]
pub(crate) enum GesturesSettingsInput {
    SetTapToClick(bool),
    SetTapAndDrag(bool),
    SetDragLock(bool),
    SetNaturalScroll(bool),
    SetDisableWhileTyping(bool),
    SetSwipeThreshold(i32),
}

#[derive(Debug)]
pub(crate) enum GesturesSettingsOutput {}

pub(crate) struct GesturesSettingsInit {}

#[derive(Debug)]
pub(crate) enum GesturesSettingsCommandOutput {}

#[relm4::component(pub)]
impl Component for GesturesSettingsModel {
    type CommandOutput = GesturesSettingsCommandOutput;
    type Input = GesturesSettingsInput;
    type Output = GesturesSettingsOutput;
    type Init = GesturesSettingsInit;

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
                        set_icon_name: Some("input-touchpad-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "Gestures",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "Touchpad tap, scroll, and swipe behaviour. Applied to the compositor live.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                // ── Touchpad ──
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Touchpad",
                    set_halign: gtk::Align::Start,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_hexpand: true,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Tap to click",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Register a tap on the touchpad as a click.",
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },
                    #[name = "tap_switch"]
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        #[block_signal(tap_handler)]
                        set_active: model.tap_to_click,
                        connect_active_notify[sender] => move |s| {
                            sender.input(GesturesSettingsInput::SetTapToClick(s.is_active()));
                        } @tap_handler,
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_hexpand: true,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Tap and drag",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Tap then slide to drag without holding the button down.",
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },
                    #[name = "tap_drag_switch"]
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        #[block_signal(tap_drag_handler)]
                        set_active: model.tap_and_drag,
                        connect_active_notify[sender] => move |s| {
                            sender.input(GesturesSettingsInput::SetTapAndDrag(s.is_active()));
                        } @tap_drag_handler,
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_hexpand: true,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Drag lock",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Keep dragging after lifting the finger until the next tap.",
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },
                    #[name = "drag_lock_switch"]
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        #[block_signal(drag_lock_handler)]
                        set_active: model.drag_lock,
                        connect_active_notify[sender] => move |s| {
                            sender.input(GesturesSettingsInput::SetDragLock(s.is_active()));
                        } @drag_lock_handler,
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_hexpand: true,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Natural scrolling",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Content follows the fingers (reverse of the classic direction).",
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },
                    #[name = "natural_switch"]
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        #[block_signal(natural_handler)]
                        set_active: model.natural_scroll,
                        connect_active_notify[sender] => move |s| {
                            sender.input(GesturesSettingsInput::SetNaturalScroll(s.is_active()));
                        } @natural_handler,
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_hexpand: true,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Disable while typing",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Ignore the touchpad briefly after a keypress to avoid stray cursor jumps.",
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },
                    #[name = "dwt_switch"]
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        #[block_signal(dwt_handler)]
                        set_active: model.disable_while_typing,
                        connect_active_notify[sender] => move |s| {
                            sender.input(GesturesSettingsInput::SetDisableWhileTyping(s.is_active()));
                        } @dwt_handler,
                    },
                },

                // ── Swipe ──
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Swipe",
                    set_halign: gtk::Align::Start,
                    set_margin_top: 12,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 20,
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_hexpand: true,
                        gtk::Label {
                            add_css_class: "label-medium-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Swipe sensitivity",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Minimum travel before a multi-finger swipe fires. Lower = more sensitive.",
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },
                    #[name = "threshold_spin"]
                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_range: (1.0, 100.0),
                        set_increments: (1.0, 5.0),
                        set_digits: 0,
                        #[block_signal(threshold_handler)]
                        set_value: model.swipe_threshold as f64,
                        connect_value_changed[sender] => move |s| {
                            sender.input(GesturesSettingsInput::SetSwipeThreshold(s.value() as i32));
                        } @threshold_handler,
                    },
                },

                gtk::Label {
                    add_css_class: "label-small",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_margin_top: 8,
                    set_label: "Swipe → action mappings (e.g. 3-finger swipe → overview) are set with `gesturebind` lines in ~/.config/margo/config.conf; run `mctl config reload` after editing.",
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let cfg = read_config();
        let model = GesturesSettingsModel {
            tap_to_click: cfg.tap_to_click,
            tap_and_drag: cfg.tap_and_drag,
            drag_lock: cfg.drag_lock,
            natural_scroll: cfg.trackpad_natural_scrolling,
            disable_while_typing: cfg.disable_while_typing,
            swipe_threshold: cfg.swipe_min_threshold as i32,
        };
        let widgets = view_output!();
        let _ = root;
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            GesturesSettingsInput::SetTapToClick(v) => {
                self.tap_to_click = v;
                apply("tap_to_click", bit(v));
            }
            GesturesSettingsInput::SetTapAndDrag(v) => {
                self.tap_and_drag = v;
                apply("tap_and_drag", bit(v));
            }
            GesturesSettingsInput::SetDragLock(v) => {
                self.drag_lock = v;
                apply("drag_lock", bit(v));
            }
            GesturesSettingsInput::SetNaturalScroll(v) => {
                self.natural_scroll = v;
                apply("trackpad_natural_scrolling", bit(v));
            }
            GesturesSettingsInput::SetDisableWhileTyping(v) => {
                self.disable_while_typing = v;
                apply("disable_while_typing", bit(v));
            }
            GesturesSettingsInput::SetSwipeThreshold(v) => {
                let v = v.max(1);
                self.swipe_threshold = v;
                apply("swipe_min_threshold", v.to_string());
            }
        }
    }
}
