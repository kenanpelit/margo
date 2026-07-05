//! Settings → Animations.
//!
//! Curated animation presets for the compositor's scene-graph motion. Like
//! the Input page these live in margo's `config.conf` (not the shell YAML):
//! the master toggles apply live on change, and a preset — a coherent set of
//! per-domain durations / bezier curves / clocks / types — is applied as a
//! batch when you pick it and hit **Apply**, then `mctl reload` makes
//! it take effect without a logout.

use crate::row::Row;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::collections::HashMap;
use std::path::PathBuf;

/// `~/.config/margo/config.conf` — the compositor config (same file the Input
/// page and the wizard patch).
fn conf_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join("margo").join("config.conf")
}

fn read_config() -> margo_config::Config {
    margo_config::parse_config_with_defaults(Some(&conf_path())).unwrap_or_default()
}

/// Raw `key = value` pairs from `config.conf` (comments skipped, last write
/// wins). Used to detect which preset is currently applied.
fn read_conf_pairs() -> HashMap<String, String> {
    let text = std::fs::read_to_string(conf_path()).unwrap_or_default();
    let mut map = HashMap::new();
    for line in text.lines() {
        let t = line.trim_start();
        if t.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = t.split_once('=') {
            map.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    map
}

/// Normalise a config value so formatting differences don't defeat equality:
/// scalars and each bezier component parse to a canonical `f64` string
/// (`520.0` == `520`, `1.0` == `1`), commas lose their surrounding spaces
/// (`0.16, 1.00, …` == `0.16,1.00,…`), everything else is lower-cased.
fn norm_val(v: &str) -> String {
    let v = v.trim();
    if let Ok(f) = v.parse::<f64>() {
        return format!("{f}");
    }
    if v.contains(',') {
        return v
            .split(',')
            .map(|p| {
                let p = p.trim();
                p.parse::<f64>()
                    .map(|f| format!("{f}"))
                    .unwrap_or_else(|_| p.to_lowercase())
            })
            .collect::<Vec<_>>()
            .join(",");
    }
    v.to_lowercase()
}

/// Index of the preset whose full key set matches the live `config.conf`, if
/// any — so the accent-highlighted card reflects the *actually applied*
/// profile, not just a default cursor. `None` for a hand-tuned config that
/// matches no preset.
fn active_preset() -> Option<u32> {
    let pairs = read_conf_pairs();
    PRESETS
        .iter()
        .position(|p| {
            p.keys
                .iter()
                .all(|(k, v)| pairs.get(*k).is_some_and(|cv| norm_val(cv) == norm_val(v)))
        })
        .map(|i| i as u32)
}

/// Patch `key = value` lines in place (append if missing), preserving comments
/// and unrelated keys.
fn patch_conf(updates: &[(&str, String)]) -> std::io::Result<()> {
    let path = conf_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let mut out = String::with_capacity(existing.len() + 256);
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

/// Reload the compositor live, reaping the child asynchronously.
fn reload() {
    match std::process::Command::new("mctl").args(["reload"]).spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(e) => tracing::warn!(error = %e, "animations: `mctl config reload` failed to spawn"),
    }
}

fn bit(on: bool) -> String {
    if on { "1" } else { "0" }.to_string()
}

/// A named animation preset: the full set of `key = value` lines it writes.
struct Preset {
    name: &'static str,
    desc: &'static str,
    keys: &'static [(&'static str, &'static str)],
}

const PRESETS: &[Preset] = &[
    Preset {
        name: "Smooth",
        desc: "Balanced, niri-flavoured motion — a spring glide for moves, gentle zoom-in, snappy fade-out. The refined default.",
        keys: &[
            ("animations", "1"),
            ("layer_animations", "1"),
            ("animation_type_open", "zoom"),
            ("animation_type_close", "fade"),
            ("layer_animation_type_open", "slide_in"),
            ("layer_animation_type_close", "slide_out"),
            ("animation_fade_in", "1"),
            ("animation_fade_out", "1"),
            ("zoom_initial_ratio", "0.94"),
            ("zoom_end_ratio", "0.94"),
            ("fadein_begin_opacity", "0.72"),
            ("animation_duration_move", "220"),
            ("animation_duration_open", "180"),
            // Close a touch faster than open — dismissing should feel instant.
            ("animation_duration_close", "130"),
            ("animation_duration_tag", "280"),
            ("animation_duration_focus", "120"),
            ("animation_clock_move", "spring"),
            ("animation_clock_open", "bezier"),
            ("animation_clock_close", "bezier"),
            ("animation_clock_tag", "bezier"),
            ("animation_clock_focus", "bezier"),
            ("animation_clock_layer", "bezier"),
            ("animation_curve_move", "0.16, 1.00, 0.30, 1.00"),
            ("animation_curve_open", "0.16, 1.00, 0.30, 1.00"),
            ("animation_curve_close", "0.25, 0.46, 0.45, 0.94"),
            ("animation_curve_tag", "0.16, 1.00, 0.30, 1.00"),
            ("animation_curve_focus", "0.33, 1.00, 0.68, 1.00"),
        ],
    },
    Preset {
        name: "Snappy",
        desc: "Fast and sharp — short durations with a crisp ease-out. Keeps the desktop feeling instant without going fully static.",
        keys: &[
            ("animations", "1"),
            ("layer_animations", "1"),
            ("animation_type_open", "zoom"),
            ("animation_type_close", "fade"),
            ("layer_animation_type_open", "slide_in"),
            ("layer_animation_type_close", "slide_out"),
            ("animation_fade_in", "1"),
            ("animation_fade_out", "1"),
            ("zoom_initial_ratio", "0.96"),
            ("zoom_end_ratio", "0.96"),
            ("fadein_begin_opacity", "0.80"),
            ("animation_duration_move", "140"),
            ("animation_duration_open", "110"),
            ("animation_duration_close", "100"),
            ("animation_duration_tag", "160"),
            ("animation_duration_focus", "80"),
            ("animation_clock_move", "spring"),
            ("animation_clock_open", "bezier"),
            ("animation_clock_close", "bezier"),
            ("animation_clock_tag", "bezier"),
            ("animation_clock_focus", "bezier"),
            ("animation_clock_layer", "bezier"),
            ("animation_curve_move", "0.22, 1.00, 0.36, 1.00"),
            ("animation_curve_open", "0.22, 1.00, 0.36, 1.00"),
            ("animation_curve_close", "0.30, 0.00, 0.50, 1.00"),
            ("animation_curve_tag", "0.22, 1.00, 0.36, 1.00"),
            ("animation_curve_focus", "0.30, 1.00, 0.50, 1.00"),
        ],
    },
    Preset {
        name: "Bouncy",
        desc: "Playful spring physics with a little overshoot — windows pop in and kick out. Springs everywhere, pronounced zoom.",
        keys: &[
            ("animations", "1"),
            ("layer_animations", "1"),
            ("animation_type_open", "zoom"),
            ("animation_type_close", "zoom"),
            ("layer_animation_type_open", "slide_in"),
            ("layer_animation_type_close", "slide_out"),
            ("animation_fade_in", "1"),
            ("animation_fade_out", "1"),
            ("zoom_initial_ratio", "0.85"),
            ("zoom_end_ratio", "0.85"),
            ("fadein_begin_opacity", "0.60"),
            ("animation_duration_move", "300"),
            ("animation_duration_open", "260"),
            ("animation_duration_close", "240"),
            ("animation_duration_tag", "340"),
            ("animation_duration_focus", "150"),
            ("animation_clock_move", "spring"),
            ("animation_clock_open", "spring"),
            ("animation_clock_close", "spring"),
            ("animation_clock_tag", "spring"),
            ("animation_clock_focus", "spring"),
            ("animation_clock_layer", "spring"),
            ("animation_curve_move", "0.34, 1.56, 0.64, 1.00"),
            ("animation_curve_open", "0.34, 1.56, 0.64, 1.00"),
            ("animation_curve_close", "0.36, 0.00, 0.66, -0.56"),
            ("animation_curve_tag", "0.34, 1.56, 0.64, 1.00"),
            ("animation_curve_focus", "0.34, 1.56, 0.64, 1.00"),
        ],
    },
    Preset {
        name: "Cinematic",
        desc: "Slow and elegant — long ease-in-out cross-fades. Best on a desktop where you want motion to feel deliberate.",
        keys: &[
            ("animations", "1"),
            ("layer_animations", "1"),
            ("animation_type_open", "fade"),
            ("animation_type_close", "fade"),
            ("layer_animation_type_open", "slide_in"),
            ("layer_animation_type_close", "slide_out"),
            ("animation_fade_in", "1"),
            ("animation_fade_out", "1"),
            ("zoom_initial_ratio", "0.92"),
            ("zoom_end_ratio", "0.92"),
            ("fadein_begin_opacity", "0.00"),
            ("animation_duration_move", "420"),
            ("animation_duration_open", "380"),
            ("animation_duration_close", "340"),
            ("animation_duration_tag", "520"),
            ("animation_duration_focus", "240"),
            ("animation_clock_move", "bezier"),
            ("animation_clock_open", "bezier"),
            ("animation_clock_close", "bezier"),
            ("animation_clock_tag", "bezier"),
            ("animation_clock_focus", "bezier"),
            ("animation_clock_layer", "bezier"),
            ("animation_curve_move", "0.65, 0.00, 0.35, 1.00"),
            ("animation_curve_open", "0.65, 0.00, 0.35, 1.00"),
            ("animation_curve_close", "0.65, 0.00, 0.35, 1.00"),
            ("animation_curve_tag", "0.65, 0.00, 0.35, 1.00"),
            ("animation_curve_focus", "0.65, 0.00, 0.35, 1.00"),
        ],
    },
    Preset {
        name: "Glide",
        desc: "Windows slide in and out from the right; workspaces and bars glide. Smooth, directional, low-zoom.",
        keys: &[
            ("animations", "1"),
            ("layer_animations", "1"),
            ("animation_type_open", "slide_in"),
            ("animation_type_close", "slide_out"),
            ("layer_animation_type_open", "slide_in"),
            ("layer_animation_type_close", "slide_out"),
            ("animation_fade_in", "1"),
            ("animation_fade_out", "1"),
            ("zoom_initial_ratio", "0.98"),
            ("zoom_end_ratio", "0.98"),
            ("fadein_begin_opacity", "0.70"),
            ("animation_duration_move", "240"),
            ("animation_duration_open", "220"),
            ("animation_duration_close", "200"),
            ("animation_duration_tag", "300"),
            ("animation_duration_focus", "120"),
            ("animation_clock_move", "spring"),
            ("animation_clock_open", "bezier"),
            ("animation_clock_close", "bezier"),
            ("animation_clock_tag", "bezier"),
            ("animation_clock_focus", "bezier"),
            ("animation_clock_layer", "bezier"),
            ("animation_curve_move", "0.16, 1.00, 0.30, 1.00"),
            ("animation_curve_open", "0.16, 1.00, 0.30, 1.00"),
            ("animation_curve_close", "0.30, 0.00, 0.40, 1.00"),
            ("animation_curve_tag", "0.16, 1.00, 0.30, 1.00"),
            ("animation_curve_focus", "0.33, 1.00, 0.68, 1.00"),
        ],
    },
    Preset {
        name: "Silk",
        desc: "Fluid, silky motion — a soft spring glide with a gentle ease-out landing. The calm, smooth default.",
        keys: &[
            ("animations", "1"),
            ("layer_animations", "1"),
            ("animation_type_open", "zoom"),
            ("animation_type_close", "fade"),
            ("layer_animation_type_open", "slide_in"),
            ("layer_animation_type_close", "slide_out"),
            ("animation_fade_in", "1"),
            ("animation_fade_out", "1"),
            ("zoom_initial_ratio", "0.92"),
            ("zoom_end_ratio", "0.92"),
            ("fadein_begin_opacity", "0.60"),
            ("animation_duration_move", "190"),
            ("animation_duration_open", "200"),
            ("animation_duration_close", "160"),
            ("animation_duration_tag", "220"),
            ("animation_duration_focus", "110"),
            ("animation_clock_move", "spring"),
            ("animation_clock_open", "bezier"),
            ("animation_clock_close", "bezier"),
            ("animation_clock_tag", "bezier"),
            ("animation_clock_focus", "bezier"),
            ("animation_clock_layer", "bezier"),
            ("animation_curve_move", "0.16, 1.00, 0.30, 1.00"),
            ("animation_curve_open", "0.16, 1.00, 0.30, 1.00"),
            ("animation_curve_close", "0.25, 0.46, 0.45, 0.94"),
            ("animation_curve_tag", "0.16, 1.00, 0.30, 1.00"),
            ("animation_curve_focus", "0.25, 1.00, 0.50, 1.00"),
            ("animation_spring_stiffness", "520.0"),
            ("animation_spring_damping_ratio", "1.0"),
            ("animation_spring_mass", "1.0"),
        ],
    },
    Preset {
        name: "Swift",
        desc: "The silky feel, tightened — a stiffer spring and shorter durations. Pick this if the smooth glide feels a touch laggy.",
        keys: &[
            ("animations", "1"),
            ("layer_animations", "1"),
            ("animation_type_open", "zoom"),
            ("animation_type_close", "fade"),
            ("layer_animation_type_open", "slide_in"),
            ("layer_animation_type_close", "slide_out"),
            ("animation_fade_in", "1"),
            ("animation_fade_out", "1"),
            ("zoom_initial_ratio", "0.92"),
            ("zoom_end_ratio", "0.92"),
            ("fadein_begin_opacity", "0.60"),
            ("animation_duration_move", "165"),
            ("animation_duration_open", "175"),
            ("animation_duration_close", "135"),
            ("animation_duration_tag", "195"),
            ("animation_duration_focus", "90"),
            ("animation_clock_move", "spring"),
            ("animation_clock_open", "bezier"),
            ("animation_clock_close", "bezier"),
            ("animation_clock_tag", "bezier"),
            ("animation_clock_focus", "bezier"),
            ("animation_clock_layer", "bezier"),
            ("animation_curve_move", "0.16, 1.00, 0.30, 1.00"),
            ("animation_curve_open", "0.16, 1.00, 0.30, 1.00"),
            ("animation_curve_close", "0.25, 0.46, 0.45, 0.94"),
            ("animation_curve_tag", "0.16, 1.00, 0.30, 1.00"),
            ("animation_curve_focus", "0.25, 1.00, 0.50, 1.00"),
            ("animation_spring_stiffness", "650.0"),
            ("animation_spring_damping_ratio", "1.0"),
            ("animation_spring_mass", "1.0"),
        ],
    },
    Preset {
        name: "Satin",
        desc: "An even softer spring — the most flowing, liquid glide. Pick this if the smooth glide doesn't feel fluid enough.",
        keys: &[
            ("animations", "1"),
            ("layer_animations", "1"),
            ("animation_type_open", "zoom"),
            ("animation_type_close", "fade"),
            ("layer_animation_type_open", "slide_in"),
            ("layer_animation_type_close", "slide_out"),
            ("animation_fade_in", "1"),
            ("animation_fade_out", "1"),
            ("zoom_initial_ratio", "0.92"),
            ("zoom_end_ratio", "0.92"),
            ("fadein_begin_opacity", "0.60"),
            ("animation_duration_move", "190"),
            ("animation_duration_open", "200"),
            ("animation_duration_close", "160"),
            ("animation_duration_tag", "220"),
            ("animation_duration_focus", "110"),
            ("animation_clock_move", "spring"),
            ("animation_clock_open", "bezier"),
            ("animation_clock_close", "bezier"),
            ("animation_clock_tag", "bezier"),
            ("animation_clock_focus", "bezier"),
            ("animation_clock_layer", "bezier"),
            ("animation_curve_move", "0.16, 1.00, 0.30, 1.00"),
            ("animation_curve_open", "0.16, 1.00, 0.30, 1.00"),
            ("animation_curve_close", "0.25, 0.46, 0.45, 0.94"),
            ("animation_curve_tag", "0.16, 1.00, 0.30, 1.00"),
            ("animation_curve_focus", "0.25, 1.00, 0.50, 1.00"),
            ("animation_spring_stiffness", "440.0"),
            ("animation_spring_damping_ratio", "1.0"),
            ("animation_spring_mass", "1.0"),
        ],
    },
    Preset {
        name: "Breeze",
        desc: "A whisper of overshoot — the spring settles with a subtle organic kick. Aliveness without the wobble.",
        keys: &[
            ("animations", "1"),
            ("layer_animations", "1"),
            ("animation_type_open", "zoom"),
            ("animation_type_close", "fade"),
            ("layer_animation_type_open", "slide_in"),
            ("layer_animation_type_close", "slide_out"),
            ("animation_fade_in", "1"),
            ("animation_fade_out", "1"),
            ("zoom_initial_ratio", "0.92"),
            ("zoom_end_ratio", "0.92"),
            ("fadein_begin_opacity", "0.60"),
            ("animation_duration_move", "190"),
            ("animation_duration_open", "200"),
            ("animation_duration_close", "160"),
            ("animation_duration_tag", "220"),
            ("animation_duration_focus", "110"),
            ("animation_clock_move", "spring"),
            ("animation_clock_open", "bezier"),
            ("animation_clock_close", "bezier"),
            ("animation_clock_tag", "bezier"),
            ("animation_clock_focus", "bezier"),
            ("animation_clock_layer", "bezier"),
            ("animation_curve_move", "0.16, 1.00, 0.30, 1.00"),
            ("animation_curve_open", "0.16, 1.00, 0.30, 1.00"),
            ("animation_curve_close", "0.25, 0.46, 0.45, 0.94"),
            ("animation_curve_tag", "0.16, 1.00, 0.30, 1.00"),
            ("animation_curve_focus", "0.25, 1.00, 0.50, 1.00"),
            ("animation_spring_stiffness", "520.0"),
            ("animation_spring_damping_ratio", "0.9"),
            ("animation_spring_mass", "1.0"),
        ],
    },
];

pub(crate) struct AnimationsSettingsModel {
    animations: bool,
    layer_animations: bool,
    selected: Option<u32>,
    /// Preset cards, kept so `.selected` can be flipped without a rebuild.
    cards: Vec<gtk::Button>,
}

impl std::fmt::Debug for AnimationsSettingsModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnimationsSettingsModel")
            .field("animations", &self.animations)
            .field("selected", &self.selected)
            .finish()
    }
}

#[derive(Debug)]
pub(crate) enum AnimationsSettingsInput {
    SetAnimations(bool),
    SetLayerAnimations(bool),
    SelectPreset(u32),
    ApplyPreset,
}

#[derive(Debug)]
pub(crate) enum AnimationsSettingsOutput {}

pub(crate) struct AnimationsSettingsInit {}

#[derive(Debug)]
pub(crate) enum AnimationsSettingsCommandOutput {}

#[relm4::component(pub)]
impl Component for AnimationsSettingsModel {
    type CommandOutput = AnimationsSettingsCommandOutput;
    type Input = AnimationsSettingsInput;
    type Output = AnimationsSettingsOutput;
    type Init = AnimationsSettingsInit;

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
                        set_icon_name: Some("preferences-desktop-screensaver-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "Animations",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "Compositor motion — toggles apply live; pick a preset and hit Apply.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                // ── General ──
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "General",
                    set_halign: gtk::Align::Start,
                },
                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Animations" },
                        #[template_child] desc { set_label: "Master switch. Off = instant transitions." },
                        #[name = "anim_switch"]
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(anim_handler)]
                            set_active: model.animations,
                            connect_active_notify[sender] => move |s| {
                                sender.input(AnimationsSettingsInput::SetAnimations(s.is_active()));
                            } @anim_handler,
                        },
                    },
                    #[template]
                    Row {
                        #[template_child] title { set_label: "Layer animations" },
                        #[template_child] desc { set_label: "Bars, launchers and menus animate in/out." },
                        #[name = "layer_switch"]
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(layer_handler)]
                            set_active: model.layer_animations,
                            connect_active_notify[sender] => move |s| {
                                sender.input(AnimationsSettingsInput::SetLayerAnimations(s.is_active()));
                            } @layer_handler,
                        },
                    },
                },

                // ── Presets ──
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Presets",
                    set_halign: gtk::Align::Start,
                    set_margin_top: 12,
                },
                gtk::Label {
                    add_css_class: "label-small",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_label: "Each preset is a full set of durations, curves and clocks across the move / open / close / workspace / focus domains. Pick one, then Apply.",
                },

                #[local_ref]
                cards_box -> gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 8,
                },

                gtk::Button {
                    add_css_class: "ok-button-primary",
                    set_halign: gtk::Align::Start,
                    set_margin_top: 4,
                    set_label: "Apply preset",
                    connect_clicked[sender] => move |_| {
                        sender.input(AnimationsSettingsInput::ApplyPreset);
                    },
                },
            }
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let cfg = read_config();
        let cards_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let mut cards = Vec::with_capacity(PRESETS.len());
        for (i, p) in PRESETS.iter().enumerate() {
            let btn = preset_card(p);
            let s = sender.clone();
            let idx = i as u32;
            btn.connect_clicked(move |_| s.input(AnimationsSettingsInput::SelectPreset(idx)));
            cards_box.append(&btn);
            cards.push(btn);
        }

        let model = AnimationsSettingsModel {
            animations: cfg.animations,
            layer_animations: cfg.layer_animations,
            selected: active_preset(),
            cards,
        };
        let widgets = view_output!();
        let _ = root;
        model.sync_cards();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            AnimationsSettingsInput::SetAnimations(v) => {
                self.animations = v;
                let _ = patch_conf(&[("animations", bit(v))]);
                reload();
            }
            AnimationsSettingsInput::SetLayerAnimations(v) => {
                self.layer_animations = v;
                let _ = patch_conf(&[("layer_animations", bit(v))]);
                reload();
            }
            AnimationsSettingsInput::SelectPreset(idx) => {
                self.selected = Some(idx);
                self.sync_cards();
            }
            AnimationsSettingsInput::ApplyPreset => {
                if let Some(sel) = self.selected
                    && let Some(preset) = PRESETS.get(sel as usize)
                {
                    let mut updates: Vec<(&str, String)> = preset
                        .keys
                        .iter()
                        .map(|(k, v)| (*k, v.to_string()))
                        .collect();
                    // Presets that don't tune the spring still reset it to the
                    // compositor default, so switching away from a spring-tuned
                    // preset (e.g. the Silk family) never leaves a stale global
                    // stiffness / damping behind.
                    if !preset
                        .keys
                        .iter()
                        .any(|(k, _)| k.starts_with("animation_spring_"))
                    {
                        updates.push(("animation_spring_stiffness", "800.0".to_string()));
                        updates.push(("animation_spring_damping_ratio", "1.0".to_string()));
                        updates.push(("animation_spring_mass", "1.0".to_string()));
                    }
                    if patch_conf(&updates).is_ok() {
                        // Applying a preset implies animations on.
                        self.animations = true;
                        self.layer_animations = true;
                        reload();
                    }
                }
            }
        }
    }
}

impl AnimationsSettingsModel {
    /// Flip `.selected` onto the chosen preset card.
    fn sync_cards(&self) {
        for (i, btn) in self.cards.iter().enumerate() {
            if Some(i as u32) == self.selected {
                btn.set_css_classes(&["ok-button-surface", "anim-preset-card", "anim-preset-active"]);
            } else {
                btn.set_css_classes(&["ok-button-surface", "anim-preset-card"]);
            }
        }
    }
}

/// A preset card: bold name + a wrapped description, on the shared surface.
fn preset_card(p: &Preset) -> gtk::Button {
    let inner = gtk::Box::new(gtk::Orientation::Vertical, 2);
    inner.set_hexpand(true);
    let name = gtk::Label::new(Some(p.name));
    name.add_css_class("label-medium-bold");
    name.set_halign(gtk::Align::Start);
    name.set_xalign(0.0);
    inner.append(&name);
    let desc = gtk::Label::new(Some(p.desc));
    desc.add_css_class("label-small");
    desc.set_halign(gtk::Align::Start);
    desc.set_xalign(0.0);
    desc.set_wrap(true);
    desc.set_natural_wrap_mode(gtk::NaturalWrapMode::None);
    inner.append(&desc);
    gtk::Button::builder()
        .child(&inner)
        .css_classes(vec!["ok-button-surface", "anim-preset-card"])
        .hexpand(true)
        .build()
}
