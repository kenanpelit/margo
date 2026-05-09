//! Layout-file parser.
//!
//! A *layout* is a self-contained margo config snippet (one or more
//! `monitorrule = ...` lines) describing one specific multi-monitor
//! arrangement. Each layout lives in its own file under the margo
//! config directory:
//!
//!     ~/.config/margo/layout_<name>.conf
//!
//! Every line that margo's parser reads as a comment (`#` prefix)
//! is normal — but lines beginning with `#@` are *meta-directives*
//! consumed by `margo-layout` itself. They configure the picker
//! UX: layout title, keyboard shortcut, per-output display name +
//! color hint for the preview rectangles.
//!
//! When the active layout is switched (`margo-layout set <name>`),
//! the chosen file is symlinked to `~/.config/margo/margo-layout.conf`
//! and a `mctl reload` triggers margo to re-read its config without
//! a logout. The user's main `config.conf` is expected to contain a
//! `source = margo-layout.conf` line picking up the active layout.
//!
//! ## Meta-directive grammar
//!
//! Top-level (apply to the whole layout):
//!
//!   * `#@ name = "Vertical"` — display title in the picker. If
//!     omitted, the file name (with `layout_` prefix and `.conf`
//!     suffix stripped) is used.
//!   * `#@ shortcut = v` — single-key shortcut for the picker.
//!     May appear multiple times for alternates.
//!
//! Per-output (apply to the *next* `monitorrule` line below):
//!
//!   * `#@ output_name = "external"` — short label drawn inside the
//!     preview rectangle. Defaults to the connector name.
//!   * `#@ color = 9` — palette index 0..17 (0 = gray, 1..17 = the
//!     standard 17-color preview palette). If unset, a stable hash
//!     of the connector name picks a colour automatically.
//!
//! Anything else after `#@` is silently ignored — the prefix is
//! reserved for `margo-layout` and won't collide with margo's
//! parser, which treats every `#`-prefixed line as a comment.

use anyhow::{anyhow, bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// One concrete layout — a parsed `layout_<name>.conf` file.
#[derive(Debug, Clone)]
pub struct Layout {
    /// Absolute path of the layout file on disk. Used as the symlink
    /// target when this layout is activated.
    pub path: PathBuf,
    /// File-name slug — `layout_vertical.conf` → `vertical`. Stable
    /// across renames of the human-readable name; what `set <name>`
    /// matches against if the layout has no `#@ name` directive.
    pub slug: String,
    /// Picker title. Falls back to `slug` when no `#@ name` was
    /// specified.
    pub name: String,
    /// Keyboard shortcuts. Each entry is a single token (no
    /// whitespace). Multiple `#@ shortcut = ...` lines accumulate.
    pub shortcuts: Vec<String>,
    /// Output rectangles, one per `monitorrule` line. Off-outputs
    /// (height/width zero or `vrr`-only rules with no geometry) are
    /// dropped — the picker can't draw them anyway.
    pub outputs: Vec<LayoutOutput>,
}

/// One output entry in a layout — geometry + presentation hints.
#[derive(Debug, Clone)]
pub struct LayoutOutput {
    /// Connector name (`DP-3`, `eDP-1`, …) from the `monitorrule`
    /// `name:` field. Empty if the rule was matched by make/model
    /// instead of name — the preview falls back to "?" in that case.
    pub connector: String,
    /// Optional label override from `#@ output_name = ...`. Drawn
    /// inside the preview rectangle when set; otherwise we draw the
    /// connector name.
    pub label: Option<String>,
    /// Optional palette index from `#@ color = N`. None means the
    /// preview hashes the connector name to pick a colour.
    pub color: Option<u8>,
    /// Logical position of the output's top-left corner in the
    /// global compositor coordinate space. When unset (the
    /// `monitorrule` had no `x:`/`y:`), defaults to (0, 0) — the
    /// preview's auto-placement re-distributes overlapping outputs.
    pub x: i32,
    pub y: i32,
    /// Logical width / height in pixels — the *post-scale* size, so
    /// a 2560x1440 mode at scale 2 renders as a 1280x720 rectangle.
    /// The preview draws against these directly.
    pub width: i32,
    pub height: i32,
    /// Transform code (margo: 0 = normal, 1 = 90°, 2 = 180°, 3 =
    /// 270°, 4..7 = flipped variants). Used by the preview to swap
    /// width / height for 90°/270° rotations.
    pub transform: i32,
}

/// Walk `dir` for `layout_*.conf` files and parse each. Returns
/// the layouts sorted by display name so the picker order is
/// deterministic regardless of file-system iteration order.
pub fn gather_layouts(dir: &Path) -> Result<Vec<Layout>> {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            bail!("config directory does not exist: {}", dir.display());
        }
        Err(err) => return Err(err.into()),
    };

    let mut layouts = Vec::new();
    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if !name_str.starts_with("layout_") || !name_str.ends_with(".conf") {
            continue;
        }
        let path = entry.path();
        let layout = parse_file(&path)
            .with_context(|| format!("parse layout file: {}", path.display()))?;
        layouts.push(layout);
    }

    layouts.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(layouts)
}

/// Parse one layout file end-to-end.
pub fn parse_file(path: &Path) -> Result<Layout> {
    let body = fs::read_to_string(path)
        .with_context(|| format!("read {}", path.display()))?;

    let file_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow!("non-UTF8 filename: {}", path.display()))?;
    let slug = file_name
        .strip_prefix("layout_")
        .and_then(|s| s.strip_suffix(".conf"))
        .ok_or_else(|| anyhow!("not a layout_*.conf file: {}", file_name))?
        .to_string();

    let mut layout = Layout {
        path: path.to_path_buf(),
        slug: slug.clone(),
        name: String::new(),
        shortcuts: Vec::new(),
        outputs: Vec::new(),
    };

    // Per-output meta directives accumulate into this scratch slot
    // and attach to the *next* `monitorrule` line. Reset after
    // each rule.
    let mut pending_label: Option<String> = None;
    let mut pending_color: Option<u8> = None;

    for (line_no, raw) in body.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }

        if let Some(rest) = line.strip_prefix("#@") {
            let directive = rest.trim();
            apply_directive(
                directive,
                &mut layout,
                &mut pending_label,
                &mut pending_color,
            )
            .with_context(|| {
                format!(
                    "{}:{}: meta-directive `#@ {}`",
                    path.display(),
                    line_no + 1,
                    directive
                )
            })?;
            continue;
        }
        if line.starts_with('#') {
            // Plain comment — skip entirely.
            continue;
        }

        // Active line. We only act on `monitorrule = ...`; anything
        // else (`source =`, blank assignments, stray text) is the
        // user's responsibility — they can put whatever they want
        // in the layout file as long as it parses with margo.
        let Some(rule_val) = strip_kv(line, "monitorrule") else {
            continue;
        };

        let mut output = parse_monitorrule(rule_val)
            .with_context(|| format!("{}:{}", path.display(), line_no + 1))?;
        output.label = pending_label.take();
        output.color = pending_color.take();

        // Drop output entries with no usable geometry — the
        // picker can't preview them. Reasonable: an `vrr-only`
        // rule against the fallback connector geometry tells
        // margo to enable VRR but leaves the output's shape to
        // its EDID-preferred mode, which we don't know offline.
        if output.width > 0 && output.height > 0 {
            layout.outputs.push(output);
        }
    }

    if layout.name.is_empty() {
        layout.name = slug;
    }

    Ok(layout)
}

/// Apply one `#@ key = value` (or `#@ key value`) directive.
fn apply_directive(
    directive: &str,
    layout: &mut Layout,
    pending_label: &mut Option<String>,
    pending_color: &mut Option<u8>,
) -> Result<()> {
    // Accept both `key = value` and `key value` separators — KDL
    // habit dies hard.
    let (key, val) = split_kv(directive);
    match key {
        "name" => {
            layout.name = unquote(val).trim().to_string();
        }
        "shortcut" => {
            for tok in val.split_whitespace() {
                let s = unquote(tok).trim().to_string();
                if !s.is_empty() {
                    layout.shortcuts.push(s);
                }
            }
        }
        "output_name" | "label" => {
            *pending_label = Some(unquote(val).trim().to_string());
        }
        "color" => {
            let n: u8 = unquote(val).trim().parse().with_context(|| {
                format!("color must be 0..=17, got `{}`", val)
            })?;
            if n > 17 {
                bail!("color must be 0..=17, got {}", n);
            }
            *pending_color = Some(n);
        }
        _ => {
            // Forward-compatible: unknown directives are tolerated
            // so adding new options doesn't break older binaries.
        }
    }
    Ok(())
}

/// Margo's `monitorrule` value uses comma-separated `key:value`
/// pairs. Direct-port the relevant subset; ignore unknown keys so
/// margo can grow new options without breaking us.
fn parse_monitorrule(val: &str) -> Result<LayoutOutput> {
    let mut out = LayoutOutput {
        connector: String::new(),
        label: None,
        color: None,
        x: 0,
        y: 0,
        width: 0,
        height: 0,
        transform: 0,
    };

    for token in val.split(',') {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        let Some((k, v)) = token.split_once(':') else {
            continue;
        };
        let v = v.trim();
        match k.trim() {
            "name" => out.connector = v.to_string(),
            "x" => out.x = v.parse().unwrap_or(0),
            "y" => out.y = v.parse().unwrap_or(0),
            "width" => out.width = v.parse().unwrap_or(0),
            "height" => out.height = v.parse().unwrap_or(0),
            "scale" => {
                let s: f32 = v.parse().unwrap_or(1.0);
                if s > 0.0 {
                    // Preview wants logical (post-scale) size:
                    // shrink the physical mode size by the scale.
                    out.width = (out.width as f32 / s).round() as i32;
                    out.height = (out.height as f32 / s).round() as i32;
                }
            }
            "transform" | "rr" => out.transform = v.parse().unwrap_or(0),
            _ => {}
        }
    }

    if matches!(out.transform, 1 | 3 | 5 | 7) {
        std::mem::swap(&mut out.width, &mut out.height);
    }

    Ok(out)
}

fn split_kv(input: &str) -> (&str, &str) {
    if let Some((k, v)) = input.split_once('=') {
        (k.trim(), v.trim())
    } else if let Some((k, v)) = input.split_once(char::is_whitespace) {
        (k.trim(), v.trim())
    } else {
        (input.trim(), "")
    }
}

fn strip_kv<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let line = line.trim();
    let after = line.strip_prefix(key)?;
    let after = after.trim_start();
    let after = after.strip_prefix('=')?;
    Some(after.trim())
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_layout() {
        let body = "monitorrule = name:DP-3,width:2560,height:1440,x:0,y:0,scale:1\n";
        let path = std::path::PathBuf::from("/tmp/layout_simple.conf");
        std::fs::write(&path, body).unwrap();
        let layout = parse_file(&path).unwrap();
        assert_eq!(layout.slug, "simple");
        assert_eq!(layout.name, "simple");
        assert_eq!(layout.outputs.len(), 1);
        assert_eq!(layout.outputs[0].connector, "DP-3");
        assert_eq!(layout.outputs[0].width, 2560);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn parses_meta_directives() {
        let body = r#"
#@ name = "Vertical"
#@ shortcut = v
#@ shortcut = V
#@ output_name = external
#@ color = 9
monitorrule = name:DP-3,width:2560,height:1440,x:0,y:0,scale:1
#@ output_name = "laptop"
monitorrule = name:eDP-1,width:1920,height:1200,x:320,y:1440,scale:1.5
"#;
        let path = std::path::PathBuf::from("/tmp/layout_test_meta.conf");
        std::fs::write(&path, body).unwrap();
        let layout = parse_file(&path).unwrap();
        assert_eq!(layout.name, "Vertical");
        assert_eq!(layout.shortcuts, vec!["v", "V"]);
        assert_eq!(layout.outputs.len(), 2);
        assert_eq!(layout.outputs[0].label.as_deref(), Some("external"));
        assert_eq!(layout.outputs[0].color, Some(9));
        assert_eq!(layout.outputs[1].label.as_deref(), Some("laptop"));
        // Logical size = 1920 / 1.5 = 1280
        assert_eq!(layout.outputs[1].width, 1280);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn handles_transform_rotation() {
        let body = "monitorrule = name:DP-3,width:1920,height:1080,transform:1\n";
        let path = std::path::PathBuf::from("/tmp/layout_rotated.conf");
        std::fs::write(&path, body).unwrap();
        let layout = parse_file(&path).unwrap();
        assert_eq!(layout.outputs[0].width, 1080);
        assert_eq!(layout.outputs[0].height, 1920);
        let _ = std::fs::remove_file(path);
    }
}
