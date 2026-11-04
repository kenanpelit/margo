//! Settings → Monitors: the pure half of the display editor. Parses the
//! live outputs (`wlr-randr --json`), places one output next to another,
//! generates the `wlr-randr` apply/validate arguments and the persisted
//! `monitorrule` lines, and runs the apply → confirm-or-revert countdown.
//! The GTK page (`monitors_settings.rs`) only wires these together.

use serde_json::Value;

/// One advertised mode.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Mode {
    pub width: i32,
    pub height: i32,
    /// Hz, as wlr-output-management advertises it (e.g. 59.951).
    pub refresh: f64,
    pub preferred: bool,
}

impl Mode {
    /// `2560x1440@59.951Hz`, wlr-randr's `--mode` syntax.
    pub fn wlr_arg(&self) -> String {
        format!("{}x{}@{:.3}Hz", self.width, self.height, self.refresh)
    }

    /// `59.95 Hz` for a dropdown entry.
    pub fn refresh_label(&self) -> String {
        format!("{:.2} Hz", self.refresh)
    }

    fn same_as(&self, other: &Mode) -> bool {
        self.width == other.width
            && self.height == other.height
            && (self.refresh - other.refresh).abs() < 0.0005
    }
}

/// wl_output transforms, in protocol order (also margo's `rr:` value).
pub(crate) const TRANSFORMS: [&str; 8] = [
    "normal",
    "90",
    "180",
    "270",
    "flipped",
    "flipped-90",
    "flipped-180",
    "flipped-270",
];

/// Human labels for [`TRANSFORMS`], same order.
pub(crate) const TRANSFORM_LABELS: [&str; 8] = [
    "Normal",
    "90°",
    "180°",
    "270°",
    "Flipped",
    "Flipped 90°",
    "Flipped 180°",
    "Flipped 270°",
];

/// One output as the editor sees and edits it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Head {
    pub name: String,
    pub enabled: bool,
    pub modes: Vec<Mode>,
    /// The mode in use (or chosen in the editor).
    pub mode: Option<Mode>,
    pub x: i32,
    pub y: i32,
    /// Index into [`TRANSFORMS`].
    pub transform: usize,
    pub scale: f64,
    pub adaptive_sync: bool,
}

impl Head {
    /// Layout-space size: mode size, swapped by a 90°/270° transform,
    /// divided by the scale — the space positions live in.
    pub fn logical_size(&self) -> (i32, i32) {
        let Some(m) = self.mode else {
            return (0, 0);
        };
        let (w, h) = if self.transform % 2 == 1 {
            (m.height, m.width)
        } else {
            (m.width, m.height)
        };
        let s = if self.scale > 0.0 { self.scale } else { 1.0 };
        (
            (f64::from(w) / s).round() as i32,
            (f64::from(h) / s).round() as i32,
        )
    }

    /// Distinct resolutions, largest first.
    pub fn resolutions(&self) -> Vec<(i32, i32)> {
        let mut r: Vec<(i32, i32)> = Vec::new();
        for m in &self.modes {
            if !r.contains(&(m.width, m.height)) {
                r.push((m.width, m.height));
            }
        }
        r.sort_by(|a, b| (b.0 * b.1).cmp(&(a.0 * a.1)).then(b.0.cmp(&a.0)));
        r
    }

    /// The modes of one resolution, fastest first.
    pub fn rates(&self, width: i32, height: i32) -> Vec<Mode> {
        let mut r: Vec<Mode> = self
            .modes
            .iter()
            .copied()
            .filter(|m| m.width == width && m.height == height)
            .collect();
        r.sort_by(|a, b| b.refresh.total_cmp(&a.refresh));
        r
    }

    /// Pick a resolution: keep the current refresh if that resolution has
    /// it, else its fastest mode.
    pub fn set_resolution(&mut self, width: i32, height: i32) {
        let rates = self.rates(width, height);
        let keep = self.mode.and_then(|cur| {
            rates
                .iter()
                .find(|m| (m.refresh - cur.refresh).abs() < 0.5)
                .copied()
        });
        if let Some(m) = keep.or_else(|| rates.first().copied()) {
            self.mode = Some(m);
        }
    }
}

/// Parse `wlr-randr --json`. Outputs without a name are skipped; a
/// missing field falls back to a neutral default.
pub(crate) fn parse_wlr_randr(body: &str) -> Result<Vec<Head>, String> {
    let v: Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    let arr = v.as_array().ok_or("expected a JSON array")?;
    Ok(arr.iter().filter_map(parse_head).collect())
}

fn parse_head(v: &Value) -> Option<Head> {
    let name = v.get("name")?.as_str()?.to_string();
    let mut current = None;
    let modes: Vec<Mode> = v
        .get("modes")
        .and_then(Value::as_array)
        .map(|ms| {
            ms.iter()
                .filter_map(|m| {
                    let mode = Mode {
                        width: m.get("width")?.as_i64()? as i32,
                        height: m.get("height")?.as_i64()? as i32,
                        refresh: m.get("refresh")?.as_f64()?,
                        preferred: m.get("preferred").and_then(Value::as_bool).unwrap_or(false),
                    };
                    if m.get("current").and_then(Value::as_bool) == Some(true) {
                        current = Some(mode);
                    }
                    Some(mode)
                })
                .collect()
        })
        .unwrap_or_default();
    let transform = v
        .get("transform")
        .and_then(Value::as_str)
        .and_then(|t| TRANSFORMS.iter().position(|n| *n == t))
        .unwrap_or(0);
    Some(Head {
        name,
        enabled: v.get("enabled").and_then(Value::as_bool).unwrap_or(true),
        mode: current.or_else(|| modes.iter().find(|m| m.preferred).copied()),
        modes,
        x: v["position"]["x"].as_i64().unwrap_or(0) as i32,
        y: v["position"]["y"].as_i64().unwrap_or(0) as i32,
        transform,
        scale: v.get("scale").and_then(Value::as_f64).unwrap_or(1.0),
        adaptive_sync: v
            .get("adaptive_sync")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

/// Which side of the anchor output a moved output goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Side {
    Left,
    Right,
    Above,
    Below,
}

/// Put `moved` edge to edge on `side` of `anchor`, centred on the other
/// axis, then shift the whole arrangement so its top-left is (0, 0).
/// Unknown names or `moved == anchor` change nothing.
pub(crate) fn place(heads: &mut [Head], moved: &str, anchor: &str, side: Side) {
    if moved == anchor {
        return;
    }
    let (Some(a), Some(mi)) = (
        heads.iter().find(|h| h.name == anchor).cloned(),
        heads.iter().position(|h| h.name == moved),
    ) else {
        return;
    };
    let (aw, ah) = a.logical_size();
    let (mw, mh) = heads[mi].logical_size();
    let (x, y) = match side {
        Side::Left => (a.x - mw, a.y + (ah - mh) / 2),
        Side::Right => (a.x + aw, a.y + (ah - mh) / 2),
        Side::Above => (a.x + (aw - mw) / 2, a.y - mh),
        Side::Below => (a.x + (aw - mw) / 2, a.y + ah),
    };
    heads[mi].x = x;
    heads[mi].y = y;
    normalise_origin(heads);
}

/// The side of `anchor` a drop centred at `(cx, cy)` (layout coords)
/// lands on: whichever axis the drop is further out along, relative to
/// the anchor's size.
pub(crate) fn side_for_drop(anchor: &Head, cx: f64, cy: f64) -> Side {
    let (w, h) = anchor.logical_size();
    let (w, h) = (f64::from(w.max(1)), f64::from(h.max(1)));
    let dx = (cx - (f64::from(anchor.x) + w / 2.0)) / w;
    let dy = (cy - (f64::from(anchor.y) + h / 2.0)) / h;
    if dx.abs() >= dy.abs() {
        if dx < 0.0 { Side::Left } else { Side::Right }
    } else if dy < 0.0 {
        Side::Above
    } else {
        Side::Below
    }
}

/// The enabled output whose centre is nearest `(cx, cy)`, other than
/// `except`.
pub(crate) fn nearest_other<'a>(
    heads: &'a [Head],
    except: &str,
    cx: f64,
    cy: f64,
) -> Option<&'a Head> {
    heads
        .iter()
        .filter(|h| h.enabled && h.name != except)
        .min_by(|a, b| {
            let d = |h: &Head| {
                let (w, hh) = h.logical_size();
                let x = f64::from(h.x) + f64::from(w) / 2.0 - cx;
                let y = f64::from(h.y) + f64::from(hh) / 2.0 - cy;
                x * x + y * y
            };
            d(a).total_cmp(&d(b))
        })
}

/// Shift enabled outputs so the arrangement's top-left is (0, 0).
pub(crate) fn normalise_origin(heads: &mut [Head]) {
    let enabled = heads.iter().filter(|h| h.enabled);
    let (Some(min_x), Some(min_y)) = (
        enabled.clone().map(|h| h.x).min(),
        enabled.map(|h| h.y).min(),
    ) else {
        return;
    };
    for h in heads.iter_mut().filter(|h| h.enabled) {
        h.x -= min_x;
        h.y -= min_y;
    }
}

/// Why an arrangement can't be applied, or `None`.
pub(crate) fn validate(heads: &[Head]) -> Option<String> {
    let on: Vec<&Head> = heads.iter().filter(|h| h.enabled).collect();
    if on.is_empty() {
        return Some("At least one display must stay on".into());
    }
    for h in &on {
        match h.mode {
            None => return Some(format!("{} has no mode selected", h.name)),
            Some(m) if !h.modes.is_empty() && !h.modes.iter().any(|a| a.same_as(&m)) => {
                return Some(format!(
                    "{} does not offer {}×{}",
                    h.name, m.width, m.height
                ));
            }
            _ => {}
        }
        if !(0.25..=4.0).contains(&h.scale) {
            return Some(format!("{} scale must be between 0.25 and 4", h.name));
        }
    }
    for (i, a) in on.iter().enumerate() {
        for b in &on[i + 1..] {
            let (aw, ah) = a.logical_size();
            let (bw, bh) = b.logical_size();
            let overlap = a.x < b.x + bw && b.x < a.x + aw && a.y < b.y + bh && b.y < a.y + ah;
            if overlap {
                return Some(format!("{} and {} overlap", a.name, b.name));
            }
        }
    }
    None
}

/// `wlr-randr` arguments that set every output to `heads` (one
/// `--output` block each). Prefix `--dryrun` to validate only.
pub(crate) fn wlr_randr_args(heads: &[Head]) -> Vec<String> {
    let mut args = Vec::new();
    for h in heads {
        args.push("--output".into());
        args.push(h.name.clone());
        if !h.enabled {
            args.push("--off".into());
            continue;
        }
        args.push("--on".into());
        if let Some(m) = h.mode {
            args.push("--mode".into());
            args.push(m.wlr_arg());
        }
        args.push("--pos".into());
        args.push(format!("{},{}", h.x, h.y));
        args.push("--transform".into());
        args.push(TRANSFORMS[h.transform.min(7)].into());
        args.push("--scale".into());
        args.push(format_scale(h.scale));
        args.push("--adaptive-sync".into());
        args.push(
            if h.adaptive_sync {
                "enabled"
            } else {
                "disabled"
            }
            .into(),
        );
    }
    args
}

fn format_scale(s: f64) -> String {
    let t = format!("{s:.3}");
    let t = t.trim_end_matches('0').trim_end_matches('.');
    t.to_string()
}

/// The `monitorrule = …` payload that makes margo start `h` the same way
/// next session. Refresh is written rounded to whole Hz: margo matches it
/// against the kernel's integer `vrefresh`.
pub(crate) fn monitorrule_line(h: &Head) -> String {
    let mut parts = vec![format!("name:{}", h.name)];
    if let Some(m) = h.mode {
        parts.push(format!("width:{}", m.width));
        parts.push(format!("height:{}", m.height));
        parts.push(format!("refresh:{}", m.refresh.round() as i64));
    }
    parts.push(format!("x:{}", h.x));
    parts.push(format!("y:{}", h.y));
    parts.push(format!("scale:{}", format_scale(h.scale)));
    if h.transform != 0 {
        parts.push(format!("rr:{}", h.transform));
    }
    if h.adaptive_sync {
        parts.push("vrr:1".into());
    }
    parts.join(",")
}

/// Merge freshly generated rules into the existing `monitorrule` block:
/// every rule naming one of `heads` is replaced, rules for other outputs
/// (or matched by make/model/serial) are kept, in their order.
pub(crate) fn merge_rules(existing: &[String], heads: &[Head]) -> Vec<String> {
    let names: Vec<&str> = heads
        .iter()
        .filter(|h| h.enabled)
        .map(|h| h.name.as_str())
        .collect();
    let mut out: Vec<String> = existing
        .iter()
        .filter(|r| !rule_name(r).is_some_and(|n| names.contains(&n)))
        .cloned()
        .collect();
    out.extend(heads.iter().filter(|h| h.enabled).map(monitorrule_line));
    out
}

/// The `name:` field of a monitorrule payload.
fn rule_name(rule: &str) -> Option<&str> {
    rule.split(',')
        .filter_map(|kv| kv.trim().split_once(':'))
        .find(|(k, _)| k.trim() == "name")
        .map(|(_, v)| v.trim())
}

/// How long a just-applied arrangement waits for "Keep" before it
/// reverts by itself.
pub(crate) const CONFIRM_SECS: u32 = 15;

/// Apply → confirm-or-revert. Pure: the page feeds it events and carries
/// out the returned [`ConfirmAction`].
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) enum ConfirmFlow {
    #[default]
    Idle,
    /// Applied; reverts to `previous` when `left` reaches 0.
    Pending {
        previous: Vec<Head>,
        applied: Vec<Head>,
        left: u32,
    },
}

/// What the page must do after a [`ConfirmFlow`] step.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ConfirmAction {
    None,
    /// Keep it: persist these as monitor rules.
    Persist(Vec<Head>),
    /// Put these back live (wlr-randr), nothing persisted.
    Revert(Vec<Head>),
}

impl ConfirmFlow {
    /// The new arrangement went live; start the countdown. A second apply
    /// while one is pending keeps the *original* `previous`, so a revert
    /// always returns to the last confirmed state.
    pub fn applied(&mut self, previous: Vec<Head>, applied: Vec<Head>) {
        let previous = match std::mem::take(self) {
            ConfirmFlow::Pending { previous, .. } => previous,
            ConfirmFlow::Idle => previous,
        };
        *self = ConfirmFlow::Pending {
            previous,
            applied,
            left: CONFIRM_SECS,
        };
    }

    /// One second passed.
    pub fn tick(&mut self) -> ConfirmAction {
        if let ConfirmFlow::Pending { left, .. } = self {
            *left = left.saturating_sub(1);
            if *left == 0 {
                return self.revert();
            }
        }
        ConfirmAction::None
    }

    pub fn keep(&mut self) -> ConfirmAction {
        match std::mem::take(self) {
            ConfirmFlow::Pending { applied, .. } => ConfirmAction::Persist(applied),
            ConfirmFlow::Idle => ConfirmAction::None,
        }
    }

    pub fn revert(&mut self) -> ConfirmAction {
        match std::mem::take(self) {
            ConfirmFlow::Pending { previous, .. } => ConfirmAction::Revert(previous),
            ConfirmFlow::Idle => ConfirmAction::None,
        }
    }

    pub fn seconds_left(&self) -> Option<u32> {
        match self {
            ConfirmFlow::Pending { left, .. } => Some(*left),
            ConfirmFlow::Idle => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `wlr-randr --json` captured on the owner's ThinkPad (margo 4.8.2,
    /// before margo advertised every connector mode), plus a second
    /// eDP-1 mode to exercise the dropdowns.
    const SAMPLE: &str = r#"[
      { "name": "DP-3", "description": "Unknown - Unknown - DP-3", "make": "Unknown",
        "model": "Unknown", "serial": "Unknown", "physical_size": { "width": 600, "height": 340 },
        "enabled": true,
        "modes": [ { "width": 2560, "height": 1440, "refresh": 59.951000, "preferred": true, "current": true } ],
        "position": { "x": 0, "y": 0 }, "transform": "normal", "scale": 1.000000, "adaptive_sync": false },
      { "name": "eDP-1", "description": "Unknown - Unknown - eDP-1", "make": "Unknown",
        "model": "Unknown", "serial": "Unknown", "physical_size": { "width": 300, "height": 190 },
        "enabled": true,
        "modes": [ { "width": 1920, "height": 1200, "refresh": 60.002998, "preferred": true, "current": true },
                   { "width": 1920, "height": 1200, "refresh": 48.002000, "preferred": false, "current": false },
                   { "width": 1280, "height": 800, "refresh": 60.000000, "preferred": false, "current": false } ],
        "position": { "x": 320, "y": 1440 }, "transform": "normal", "scale": 1.000000, "adaptive_sync": false }
    ]"#;

    fn heads() -> Vec<Head> {
        parse_wlr_randr(SAMPLE).unwrap()
    }

    fn get<'a>(hs: &'a [Head], n: &str) -> &'a Head {
        hs.iter().find(|h| h.name == n).unwrap()
    }

    #[test]
    fn parses_the_live_outputs() {
        let hs = heads();
        assert_eq!(hs.len(), 2);
        let edp = get(&hs, "eDP-1");
        assert_eq!((edp.x, edp.y), (320, 1440));
        assert_eq!(edp.mode.unwrap().refresh, 60.002998);
        assert_eq!(edp.logical_size(), (1920, 1200));
        assert_eq!(edp.transform, 0);
        assert!(parse_wlr_randr("{}").is_err());
    }

    #[test]
    fn resolutions_and_rates_come_from_the_advertised_modes() {
        let hs = heads();
        let edp = get(&hs, "eDP-1");
        assert_eq!(edp.resolutions(), vec![(1920, 1200), (1280, 800)]);
        let rates: Vec<f64> = edp.rates(1920, 1200).iter().map(|m| m.refresh).collect();
        assert_eq!(rates, vec![60.002998, 48.002]);
        assert_eq!(edp.rates(1920, 1200)[0].wlr_arg(), "1920x1200@60.003Hz");
    }

    #[test]
    fn changing_resolution_keeps_the_refresh_when_offered() {
        let mut edp = get(&heads(), "eDP-1").clone();
        edp.set_resolution(1280, 800);
        assert_eq!(edp.mode.unwrap().width, 1280);
        assert_eq!(edp.mode.unwrap().refresh, 60.0);
    }

    #[test]
    fn a_rotated_or_scaled_output_takes_its_logical_size() {
        let mut edp = get(&heads(), "eDP-1").clone();
        edp.transform = 1; // 90°
        assert_eq!(edp.logical_size(), (1200, 1920));
        edp.transform = 0;
        edp.scale = 1.25;
        assert_eq!(edp.logical_size(), (1536, 960));
    }

    #[test]
    fn placing_below_centres_the_laptop_under_the_monitor() {
        // The owner's real layout: eDP-1 centred under DP-3.
        let mut hs = heads();
        place(&mut hs, "eDP-1", "DP-3", Side::Right);
        assert_eq!((get(&hs, "eDP-1").x, get(&hs, "eDP-1").y), (2560, 120));
        place(&mut hs, "eDP-1", "DP-3", Side::Below);
        assert_eq!((get(&hs, "eDP-1").x, get(&hs, "eDP-1").y), (320, 1440));
        assert_eq!((get(&hs, "DP-3").x, get(&hs, "DP-3").y), (0, 0));
    }

    #[test]
    fn placing_left_or_above_renormalises_to_the_origin() {
        let mut hs = heads();
        place(&mut hs, "eDP-1", "DP-3", Side::Left);
        assert_eq!((get(&hs, "eDP-1").x, get(&hs, "eDP-1").y), (0, 120));
        assert_eq!((get(&hs, "DP-3").x, get(&hs, "DP-3").y), (1920, 0));
        place(&mut hs, "eDP-1", "DP-3", Side::Above);
        assert_eq!((get(&hs, "eDP-1").x, get(&hs, "eDP-1").y), (320, 0));
        assert_eq!((get(&hs, "DP-3").x, get(&hs, "DP-3").y), (0, 1200));
        assert_eq!(validate(&hs), None);
    }

    #[test]
    fn a_drop_picks_the_side_it_is_furthest_out_on() {
        let hs = heads();
        let dp = get(&hs, "DP-3");
        assert_eq!(side_for_drop(dp, 1280.0, 2000.0), Side::Below);
        assert_eq!(side_for_drop(dp, 3000.0, 700.0), Side::Right);
        assert_eq!(side_for_drop(dp, -500.0, 800.0), Side::Left);
        assert_eq!(side_for_drop(dp, 1300.0, -400.0), Side::Above);
        assert_eq!(nearest_other(&hs, "eDP-1", 0.0, 0.0).unwrap().name, "DP-3");
    }

    #[test]
    fn validation_catches_overlap_no_output_and_unoffered_modes() {
        let mut hs = heads();
        assert_eq!(validate(&hs), None);
        hs[1].y = 1000;
        assert_eq!(validate(&hs).unwrap(), "DP-3 and eDP-1 overlap");
        let mut hs = heads();
        for h in &mut hs {
            h.enabled = false;
        }
        assert!(validate(&hs).unwrap().contains("must stay on"));
        let mut hs = heads();
        hs[1].mode = Some(Mode {
            width: 800,
            height: 600,
            refresh: 60.0,
            preferred: false,
        });
        assert!(validate(&hs).unwrap().contains("does not offer"));
    }

    #[test]
    fn wlr_randr_args_set_every_field() {
        let mut hs = heads();
        hs[1].transform = 3;
        hs[1].scale = 1.25;
        let args = wlr_randr_args(&hs).join(" ");
        assert_eq!(
            args,
            "--output DP-3 --on --mode 2560x1440@59.951Hz --pos 0,0 --transform normal \
             --scale 1 --adaptive-sync disabled \
             --output eDP-1 --on --mode 1920x1200@60.003Hz --pos 320,1440 --transform 270 \
             --scale 1.25 --adaptive-sync disabled"
        );
        hs[0].enabled = false;
        assert!(
            wlr_randr_args(&hs)
                .join(" ")
                .starts_with("--output DP-3 --off --output eDP-1")
        );
    }

    #[test]
    fn monitorrule_lines_round_trip_through_margos_parser() {
        let mut hs = heads();
        hs[1].transform = 1;
        hs[1].adaptive_sync = true;
        let line = monitorrule_line(&hs[1]);
        assert_eq!(
            line,
            "name:eDP-1,width:1920,height:1200,refresh:60,x:320,y:1440,scale:1,rr:1,vrr:1"
        );
        let cfg = margo_config::parse_config_str(&format!("monitorrule = {line}\n"), None).unwrap();
        let r = &cfg.monitor_rules[0];
        assert_eq!(r.name.as_deref(), Some("eDP-1"));
        assert_eq!(
            (r.width, r.height, r.x, r.y, r.transform),
            (1920, 1200, 320, 1440, 1)
        );
        assert_eq!(r.refresh, 60.0);
        assert!(r.vrr);
    }

    #[test]
    fn merging_replaces_rules_for_these_outputs_only() {
        let existing = vec![
            "name:eDP-1,width:1280,height:800,x:0,y:0,scale:1".to_string(),
            "name:HDMI-A-1,x:0,y:0,scale:1".to_string(),
            "make:Dell,scale:1.5".to_string(),
        ];
        let merged = merge_rules(&existing, &heads());
        assert_eq!(merged.len(), 4);
        assert_eq!(merged[0], existing[1]);
        assert_eq!(merged[1], existing[2]);
        assert!(merged[2].starts_with("name:DP-3,"));
        assert!(merged[3].starts_with("name:eDP-1,width:1920"));
    }

    #[test]
    fn an_unconfirmed_apply_reverts_after_the_countdown() {
        let before = heads();
        let mut after = before.clone();
        place(&mut after, "eDP-1", "DP-3", Side::Right);
        let mut flow = ConfirmFlow::default();
        flow.applied(before.clone(), after.clone());
        assert_eq!(flow.seconds_left(), Some(CONFIRM_SECS));
        for _ in 1..CONFIRM_SECS {
            assert_eq!(flow.tick(), ConfirmAction::None);
        }
        assert_eq!(flow.tick(), ConfirmAction::Revert(before));
        assert_eq!(flow, ConfirmFlow::Idle);
        assert_eq!(flow.tick(), ConfirmAction::None);
    }

    #[test]
    fn keep_persists_and_revert_restores_the_last_confirmed_state() {
        let before = heads();
        let mut a = before.clone();
        place(&mut a, "eDP-1", "DP-3", Side::Right);
        let mut b = a.clone();
        place(&mut b, "eDP-1", "DP-3", Side::Left);

        let mut flow = ConfirmFlow::default();
        flow.applied(before.clone(), a.clone());
        flow.tick();
        // A second apply restarts the countdown but still reverts to `before`.
        flow.applied(a.clone(), b.clone());
        assert_eq!(flow.seconds_left(), Some(CONFIRM_SECS));
        assert_eq!(flow.clone().revert(), ConfirmAction::Revert(before));
        assert_eq!(flow.keep(), ConfirmAction::Persist(b));
        assert_eq!(flow.keep(), ConfirmAction::None);
    }
}
