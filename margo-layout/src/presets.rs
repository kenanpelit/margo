//! Preset-layout generator.
//!
//! Given the live monitor list (typically two: a laptop panel + an
//! external display), generates a small catalogue of common
//! arrangements and writes them as `layout_<slug>.conf` files. The
//! user can then flip between them with `margo-layout set <slug>`.
//!
//! ## Why this exists
//!
//! `margo-layout init` captures whatever the *current* output
//! arrangement happens to be — useful as a one-shot "remember
//! this", but unhelpful when the live state isn't actually the
//! layout you want. (Common: udev hot-plug puts the external
//! monitor side-by-side at scale 1, but you actually want it
//! stacked above the laptop screen.) `suggest` solves that by
//! laying out the *plausible* arrangements up front, naming each
//! one descriptively, and letting you pick.
//!
//! ## What gets generated
//!
//! Single output: nothing — `solo` is the only sensible layout.
//!
//! Two outputs (the common laptop+monitor case):
//!
//! | Slug                   | Description                                  |
//! |------------------------|----------------------------------------------|
//! | `horizontal-ext-left`  | external on the left, laptop on the right    |
//! | `horizontal-ext-right` | laptop on the left, external on the right    |
//! | `vertical-ext-top`     | external on top, laptop centred underneath   |
//! | `vertical-ext-bottom`  | laptop on top, external centred underneath   |
//! | `mirrored`             | both at the same origin (clone)              |
//! | `laptop-only`          | external explicitly disabled (off)           |
//! | `external-only`        | laptop explicitly disabled (off)             |
//!
//! For three or more outputs we emit `horizontal-row` and
//! `vertical-stack` only — the rest gets unwieldy quickly and the
//! user can hand-edit if their setup is exotic.

use crate::capture::{auto_color_for_connector, auto_label_for_connector, CapturedOutput};

/// One preset arrangement — title for the picker, file slug, and
/// the rectified output list (positions resolved, labels assigned).
pub struct Preset {
    pub slug: &'static str,
    pub name: String,
    pub outputs: Vec<PresetOutput>,
}

/// One output as it should land in the preset's layout file.
/// `Off` outputs get a special `off` flag in the monitorrule so
/// margo skips them entirely (matches `monitorrule = ...,off:1`
/// semantics — but margo doesn't have `off` today, so for now we
/// just omit the rule for off outputs and document it as a limit).
pub struct PresetOutput {
    pub connector: String,
    pub label: Option<&'static str>,
    pub color: Option<u8>,
    pub width: i32,
    pub height: i32,
    pub refresh: f32,
    pub scale: f32,
    pub x: i32,
    pub y: i32,
}

impl PresetOutput {
    fn from_captured(cap: &CapturedOutput, x: i32, y: i32) -> Self {
        Self {
            connector: cap.connector.clone(),
            label: auto_label_for_connector(&cap.connector),
            color: auto_color_for_connector(&cap.connector),
            width: cap.width,
            height: cap.height,
            refresh: cap.refresh,
            scale: cap.scale.max(0.1),
            x,
            y,
        }
    }
}

/// Generate the preset catalogue for `outputs`. Returns presets
/// in the order most users will want them — vertical-stack first
/// when the user has a laptop, then horizontal, then the niche
/// modes (mirrored, single-monitor).
pub fn generate(outputs: &[CapturedOutput]) -> Vec<Preset> {
    match classify(outputs) {
        Setup::None => Vec::new(),
        Setup::Solo(o) => vec![preset_solo(o)],
        Setup::TwoOutputs { laptop, external } => two_output_presets(laptop, external),
        Setup::Many(all) => many_output_presets(all),
    }
}

enum Setup<'a> {
    None,
    Solo(&'a CapturedOutput),
    TwoOutputs {
        laptop: &'a CapturedOutput,
        external: &'a CapturedOutput,
    },
    Many(&'a [CapturedOutput]),
}

fn classify(outputs: &[CapturedOutput]) -> Setup<'_> {
    let enabled: Vec<&CapturedOutput> = outputs.iter().filter(|o| o.enabled).collect();
    match enabled.len() {
        0 => Setup::None,
        1 => Setup::Solo(enabled[0]),
        2 => {
            // Determine which is the laptop panel vs external.
            // If both are non-laptop or both are laptop, fall
            // back to width-as-tiebreaker (smaller = laptop-ish
            // since most external monitors are bigger than the
            // built-in panel).
            let (a, b) = (enabled[0], enabled[1]);
            let a_laptop = is_laptop(&a.connector);
            let b_laptop = is_laptop(&b.connector);
            let (laptop, external) = match (a_laptop, b_laptop) {
                (true, false) => (a, b),
                (false, true) => (b, a),
                _ => {
                    if (a.width as f64 / a.scale as f64) <= (b.width as f64 / b.scale as f64) {
                        (a, b)
                    } else {
                        (b, a)
                    }
                }
            };
            Setup::TwoOutputs { laptop, external }
        }
        _ => Setup::Many(outputs),
    }
}

fn is_laptop(connector: &str) -> bool {
    let upper = connector.to_uppercase();
    upper.starts_with("EDP") || upper.starts_with("LVDS")
}

fn preset_solo(o: &CapturedOutput) -> Preset {
    Preset {
        slug: "solo",
        name: "Solo".to_string(),
        outputs: vec![PresetOutput::from_captured(o, 0, 0)],
    }
}

/// Two-output catalogue. Logical-sized values used for the
/// horizontal/vertical math so the visual layout matches what
/// margo will actually render at the configured scale.
fn two_output_presets(laptop: &CapturedOutput, external: &CapturedOutput) -> Vec<Preset> {
    let lw = (laptop.width as f64 / laptop.scale.max(0.1) as f64).round() as i32;
    let lh = (laptop.height as f64 / laptop.scale.max(0.1) as f64).round() as i32;
    let ew = (external.width as f64 / external.scale.max(0.1) as f64).round() as i32;
    let eh = (external.height as f64 / external.scale.max(0.1) as f64).round() as i32;

    let mut presets = Vec::new();

    // 1. Vertical stack with external on top — this is the
    //    "looking-up-at-the-monitor, laptop in front" arrangement
    //    that's standard for desk setups.
    {
        let lap_x = (ew - lw) / 2; // centre laptop under external
        let lap_y = eh;
        presets.push(Preset {
            slug: "vertical-ext-top",
            name: "Vertical — external on top".to_string(),
            outputs: vec![
                PresetOutput::from_captured(external, 0, 0),
                PresetOutput::from_captured(laptop, lap_x.max(0), lap_y),
            ],
        });
    }

    // 2. Vertical stack with external below the laptop. Less
    //    common but valid for elevated laptop stands.
    {
        let ext_x = (lw - ew) / 2;
        let ext_y = lh;
        presets.push(Preset {
            slug: "vertical-ext-bottom",
            name: "Vertical — external below laptop".to_string(),
            outputs: vec![
                PresetOutput::from_captured(laptop, 0, 0),
                PresetOutput::from_captured(external, ext_x.max(0), ext_y),
            ],
        });
    }

    // 3. Horizontal — external left, laptop right.
    {
        // Bottom-align both: the smaller height goes higher in
        // local-y so their bottom edges line up. Niri/sway/etc.
        // do the same.
        let lh_off = (eh - lh).max(0);
        presets.push(Preset {
            slug: "horizontal-ext-left",
            name: "Horizontal — external left".to_string(),
            outputs: vec![
                PresetOutput::from_captured(external, 0, 0),
                PresetOutput::from_captured(laptop, ew, lh_off),
            ],
        });
    }

    // 4. Horizontal — laptop left, external right.
    {
        let eh_off = 0;
        let lh_off = (eh - lh).max(0);
        presets.push(Preset {
            slug: "horizontal-ext-right",
            name: "Horizontal — external right".to_string(),
            outputs: vec![
                PresetOutput::from_captured(laptop, 0, lh_off),
                PresetOutput::from_captured(external, lw, eh_off),
            ],
        });
    }

    // 5. Mirrored — both at the same origin. Useful for
    //    presentations / projector-mode where you want the
    //    laptop to mirror what's on screen. Note: margo can't
    //    actually clone outputs today (no `wl_mirror` analogue),
    //    so this preset positions both at (0,0) which yields
    //    overlapping content with the bigger one in front;
    //    treat it as "informational" until the mirror feature
    //    lands.
    presets.push(Preset {
        slug: "mirrored",
        name: "Mirrored (overlap at origin)".to_string(),
        outputs: vec![
            PresetOutput::from_captured(external, 0, 0),
            PresetOutput::from_captured(laptop, 0, 0),
        ],
    });

    // 6. Laptop-only — external dropped from the rule list, so
    //    margo leaves the EDID-default position; with no
    //    monitorrule the external still gets auto-placed. To
    //    truly disable we'd need an `off:1` rule margo doesn't
    //    yet honour. For now this is "drive only the laptop"
    //    which means the external ends up with default geometry.
    presets.push(Preset {
        slug: "laptop-only",
        name: "Laptop only".to_string(),
        outputs: vec![PresetOutput::from_captured(laptop, 0, 0)],
    });

    // 7. External-only — same caveat as above.
    presets.push(Preset {
        slug: "external-only",
        name: "External only".to_string(),
        outputs: vec![PresetOutput::from_captured(external, 0, 0)],
    });

    presets
}

/// Three+ outputs: just emit a horizontal row and a vertical
/// stack. The user can hand-edit for niche arrangements.
fn many_output_presets(outputs: &[CapturedOutput]) -> Vec<Preset> {
    let mut sorted: Vec<&CapturedOutput> = outputs.iter().filter(|o| o.enabled).collect();
    sorted.sort_by(|a, b| a.connector.cmp(&b.connector));

    let mut presets = Vec::with_capacity(2);

    // Horizontal row, sorted by connector name.
    {
        let mut x = 0;
        let outs: Vec<PresetOutput> = sorted
            .iter()
            .map(|o| {
                let lw = (o.width as f64 / o.scale.max(0.1) as f64).round() as i32;
                let p = PresetOutput::from_captured(o, x, 0);
                x += lw;
                p
            })
            .collect();
        presets.push(Preset {
            slug: "horizontal-row",
            name: format!("Horizontal row ({} outputs)", sorted.len()),
            outputs: outs,
        });
    }

    // Vertical stack.
    {
        let mut y = 0;
        let outs: Vec<PresetOutput> = sorted
            .iter()
            .map(|o| {
                let lh = (o.height as f64 / o.scale.max(0.1) as f64).round() as i32;
                let p = PresetOutput::from_captured(o, 0, y);
                y += lh;
                p
            })
            .collect();
        presets.push(Preset {
            slug: "vertical-stack",
            name: format!("Vertical stack ({} outputs)", sorted.len()),
            outputs: outs,
        });
    }

    presets
}

/// Marker string written into every auto-generated preset file.
/// `cleanup_auto_generated_presets` looks for this exact line to
/// decide whether a layout file is safe to delete (vs being a
/// user-hand-edited layout that should be preserved).
pub const AUTOGEN_MARKER: &str = "# margo-layout: auto-generated preset";

/// Render one preset to the `layout_<slug>.conf` text format
/// `parser::parse_file` expects.
pub fn render(preset: &Preset, shortcut: Option<&str>) -> String {
    let mut buf = String::new();
    buf.push_str(AUTOGEN_MARKER);
    buf.push('\n');
    buf.push_str("# Re-run `margo-layout suggest` to refresh.\n");
    buf.push_str("# Hand-edit and remove the marker line above to opt out\n");
    buf.push_str("# of automatic cleanup.\n\n");
    buf.push_str(&format!("#@ name = \"{}\"\n", preset.name));
    if let Some(s) = shortcut {
        buf.push_str(&format!("#@ shortcut = {}\n", s));
    }
    buf.push('\n');

    for o in &preset.outputs {
        if let Some(label) = o.label {
            buf.push_str(&format!("#@ output_name = \"{}\"\n", label));
        }
        if let Some(c) = o.color {
            buf.push_str(&format!("#@ color = {}\n", c));
        }
        let mut parts = vec![format!("name:{}", o.connector)];
        parts.push(format!("width:{}", o.width));
        parts.push(format!("height:{}", o.height));
        if o.refresh > 0.0 {
            parts.push(format!("refresh:{}", o.refresh.round() as i32));
        }
        parts.push(format!("x:{}", o.x));
        parts.push(format!("y:{}", o.y));
        if (o.scale - 1.0).abs() > 0.001 {
            parts.push(format!("scale:{}", trim_zeros(o.scale)));
        } else {
            parts.push("scale:1".to_string());
        }
        buf.push_str(&format!("monitorrule = {}\n\n", parts.join(",")));
    }

    buf
}

fn trim_zeros(v: f32) -> String {
    let s = format!("{:.3}", v);
    let s = s.trim_end_matches('0');
    let s = s.trim_end_matches('.');
    s.to_string()
}

/// Pick a single-character shortcut for each preset slug. Stable
/// across runs (same slug → same letter), and unique within the
/// catalogue. Falls through to None if all candidates collide.
pub fn shortcut_for(slug: &str, taken: &[String]) -> Option<&'static str> {
    let candidates: &[&'static str] = match slug {
        "solo" => &["s", "1"],
        "vertical-ext-top" => &["v", "t"],
        "vertical-ext-bottom" => &["b"],
        "horizontal-ext-left" => &["h", "l"],
        "horizontal-ext-right" => &["r"],
        "mirrored" => &["m"],
        "laptop-only" => &["L", "p"],
        "external-only" => &["e", "x"],
        "horizontal-row" => &["h", "r"],
        "vertical-stack" => &["v", "k"],
        _ => &[],
    };
    candidates
        .iter()
        .find(|c| !taken.iter().any(|t| t == *c))
        .copied()
}
