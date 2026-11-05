//! `mscreenshot measure` — the on-screen ruler.
//!
//! Drag a box with the region selector (the in-shell one live-labels the
//! size and aspect while you drag; the `slurp` fallback runs with `-d` so it
//! does too), release, and the size lands on the clipboard as `WxH` with a
//! notification spelling out the position, aspect ratio and output. Sizes
//! are reported in the output's **device pixels** — what a screenshot of the
//! area would measure; on a scaled output the logical size is shown too.

/// A `slurp` / selector geometry: `"X,Y WxH"` in layout (logical) pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// One output from `mctl status --json`: name, layout rect and scale.
#[derive(Clone, Debug, PartialEq)]
pub struct Output {
    pub name: String,
    pub rect: Rect,
    pub scale: f64,
}

/// Parse `"X,Y WxH"`; `None` for anything else (including a cancel's
/// empty string).
pub fn parse_geom(s: &str) -> Option<Rect> {
    let (pos, size) = s.trim().split_once(' ')?;
    let (x, y) = pos.split_once(',')?;
    let (w, h) = size.trim().split_once('x')?;
    let r = Rect {
        x: x.trim().parse().ok()?,
        y: y.trim().parse().ok()?,
        w: w.trim().parse().ok()?,
        h: h.trim().parse().ok()?,
    };
    (r.w > 0 && r.h > 0).then_some(r)
}

/// Outputs from a `mctl status --json` document (missing fields skip the
/// output; a missing scale is 1).
pub fn outputs_in(json: &serde_json::Value) -> Vec<Output> {
    json["outputs"]
        .as_array()
        .map(|outs| {
            outs.iter()
                .filter_map(|o| {
                    let int = |k: &str| o[k].as_i64().map(|v| v as i32);
                    Some(Output {
                        name: o["name"].as_str()?.to_string(),
                        rect: Rect {
                            x: int("x")?,
                            y: int("y")?,
                            w: int("width")?,
                            h: int("height")?,
                        },
                        scale: o["scale"].as_f64().filter(|s| *s > 0.0).unwrap_or(1.0),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The output holding the region's centre (a box can't usefully span two
/// outputs with different scales; the centre decides).
pub fn output_for<'a>(r: &Rect, outputs: &'a [Output]) -> Option<&'a Output> {
    let cx = r.x + r.w / 2;
    let cy = r.y + r.h / 2;
    outputs.iter().find(|o| {
        cx >= o.rect.x && cx < o.rect.x + o.rect.w && cy >= o.rect.y && cy < o.rect.y + o.rect.h
    })
}

/// A finished measurement, ready to copy and announce.
#[derive(Clone, Debug, PartialEq)]
pub struct Measurement {
    /// Device-pixel size.
    pub px_w: i32,
    pub px_h: i32,
    /// The selection in logical pixels, its origin relative to its output
    /// (what a designer compares against; the layout origin rarely is).
    pub logical: Rect,
    pub output: Option<String>,
    pub scale: f64,
}

impl Measurement {
    pub fn new(r: Rect, outputs: &[Output]) -> Self {
        let out = output_for(&r, outputs);
        let scale = out.map(|o| o.scale).unwrap_or(1.0);
        let logical = match out {
            Some(o) => Rect {
                x: r.x - o.rect.x,
                y: r.y - o.rect.y,
                ..r
            },
            None => r,
        };
        Self {
            px_w: (f64::from(r.w) * scale).round() as i32,
            px_h: (f64::from(r.h) * scale).round() as i32,
            logical,
            output: out.map(|o| o.name.clone()),
            scale,
        }
    }

    /// What goes on the clipboard.
    pub fn clipboard(&self) -> String {
        format!("{}x{}", self.px_w, self.px_h)
    }

    pub fn title(&self) -> String {
        format!(
            "{} × {} px · {}",
            self.px_w,
            self.px_h,
            aspect(self.px_w, self.px_h)
        )
    }

    pub fn body(&self) -> String {
        let r = self.logical;
        let mut body = match &self.output {
            Some(name) => format!("at {}, {} on {name}", r.x, r.y),
            None => format!("at {}, {}", r.x, r.y),
        };
        if (self.scale - 1.0).abs() > f64::EPSILON {
            body.push_str(&format!(
                "\n{} × {} logical at {}× scale",
                r.w,
                r.h,
                trim_float(self.scale)
            ));
        }
        body.push_str("\nCopied as ");
        body.push_str(&self.clipboard());
        body
    }
}

fn trim_float(v: f64) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn gcd(a: i32, b: i32) -> i32 {
    if b == 0 { a.abs() } else { gcd(b, a % b) }
}

/// `16:9`-style ratio; a close common ratio (within 1 %) is named instead
/// of an unreadable exact one (`1735×973` → `16:9`).
pub fn aspect(w: i32, h: i32) -> String {
    if w <= 0 || h <= 0 {
        return "—".to_string();
    }
    const COMMON: &[(i32, i32)] = &[
        (1, 1),
        (4, 3),
        (3, 2),
        (16, 10),
        (16, 9),
        (21, 9),
        (32, 9),
        (3, 4),
        (2, 3),
        (9, 16),
    ];
    let g = gcd(w, h);
    let (rw, rh) = (w / g, h / g);
    if rw <= 32 && rh <= 32 {
        return format!("{rw}:{rh}");
    }
    let r = f64::from(w) / f64::from(h);
    for (cw, ch) in COMMON {
        let c = f64::from(*cw) / f64::from(*ch);
        if ((r - c) / c).abs() < 0.01 {
            return format!("{cw}:{ch}");
        }
    }
    format!("{:.2}:1", r)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shape of this machine's `mctl status --json` outputs (trimmed).
    fn status() -> serde_json::Value {
        serde_json::json!({"outputs": [
            {"name": "eDP-1", "x": 320, "y": 1440, "width": 1920, "height": 1200, "scale": 1.0, "active": false},
            {"name": "DP-3", "x": 0, "y": 0, "width": 2560, "height": 1440, "scale": 1.0, "active": true},
            {"name": "HiDPI", "x": 2560, "y": 0, "width": 1280, "height": 800, "scale": 2.0}
        ]})
    }

    #[test]
    fn parses_slurp_geometry() {
        assert_eq!(
            parse_geom("100,200 640x480\n"),
            Some(Rect {
                x: 100,
                y: 200,
                w: 640,
                h: 480
            })
        );
        assert_eq!(parse_geom(""), None, "cancel");
        assert_eq!(parse_geom("1,2 0x5"), None, "empty box");
        assert_eq!(parse_geom("garbage"), None);
        assert_eq!(parse_geom("-5,10 3x4").map(|r| r.x), Some(-5));
    }

    #[test]
    fn finds_the_output_under_the_centre() {
        let outs = outputs_in(&status());
        assert_eq!(outs.len(), 3);
        let r = parse_geom("400,1500 100x100").unwrap();
        assert_eq!(output_for(&r, &outs).unwrap().name, "eDP-1");
        let r = parse_geom("2600,10 50x50").unwrap();
        assert_eq!(output_for(&r, &outs).unwrap().name, "HiDPI");
        let r = parse_geom("9000,9000 5x5").unwrap();
        assert!(output_for(&r, &outs).is_none());
    }

    #[test]
    fn device_pixels_follow_the_scale() {
        let outs = outputs_in(&status());
        let m2 = Measurement::new(parse_geom("2600,10 50x40").unwrap(), &outs);
        assert_eq!((m2.px_w, m2.px_h), (100, 80));
        assert_eq!(m2.clipboard(), "100x80");
        assert!(m2.body().contains("50 × 40 logical at 2× scale"));
        assert!(m2.body().starts_with("at 40, 10 on HiDPI"));
        let m3 = Measurement::new(parse_geom("10,10 1920x1080").unwrap(), &outs);
        assert_eq!(m3.title(), "1920 × 1080 px · 16:9");
        assert!(!m3.body().contains("logical"));
    }

    #[test]
    fn aspect_names() {
        assert_eq!(aspect(1920, 1080), "16:9");
        assert_eq!(aspect(1735, 973), "16:9");
        assert_eq!(aspect(300, 300), "1:1");
        assert_eq!(aspect(1700, 973), "1.75:1");
        assert_eq!(aspect(0, 10), "—");
    }

    #[test]
    fn position_is_relative_to_the_output() {
        let outs = outputs_in(&status());
        let m = Measurement::new(parse_geom("400,1500 10x10").unwrap(), &outs);
        assert_eq!((m.logical.x, m.logical.y), (80, 60));
        assert_eq!(m.output.as_deref(), Some("eDP-1"));
    }
}
