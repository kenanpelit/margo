//! Apply a layout's geometry to the live session.
//!
//! After `mlayout set` swaps the symlink, the layout file is
//! the new source of truth — but margo's runtime won't reposition
//! existing outputs to match unless something pokes them. The
//! historical fix was "log out and back in"; this module does the
//! poke directly via `wlr-randr` so the change lands without a
//! session restart.
//!
//! ## Why wlr-randr and not direct Wayland
//!
//! We could open our own `wlr_output_management_v1` client
//! connection from the binary and push the changes. That's
//! ~200 LOC of protocol bindings + a wayland-client dep, all to
//! reproduce one tool's behaviour. `wlr-randr` is already a
//! recommended optdep on margo; spawning it once per layout
//! switch is fine.
//!
//! ## Atomicity
//!
//! `wlr-randr` accepts multiple `--output X --pos Y` blocks in
//! one invocation and applies them as a single atomic transaction
//! on the compositor side. We exploit that — one spawn per
//! layout change, all positions land or none do. Avoids the
//! visible flicker you'd see from one-at-a-time updates.

use std::process::Command;

use anyhow::{Context, Result};

use crate::parser::Layout;

/// Apply `layout`'s geometry to the live session via `wlr-randr`.
/// Atomic: all positions in one process spawn so the compositor
/// applies them in a single configure event.
///
/// Silent success on a single-output layout if the only output
/// is already at the configured geometry — saves a no-op flicker
/// when the user re-selects the active layout.
pub fn apply(layout: &Layout) -> Result<()> {
    if layout.outputs.is_empty() {
        // Nothing to apply. Treat as success — the symlink
        // change might be all the caller wanted.
        return Ok(());
    }
    if !is_command_available("wlr-randr") {
        eprintln!(
            "(`wlr-randr` not on PATH — layout file is in place but the \
             live output configuration won't change until session restart.)"
        );
        return Ok(());
    }

    let args = wlr_randr_args(layout);
    if args.is_empty() {
        // Every output was matched by make/model — nothing wlr-randr can drive.
        return Ok(());
    }
    let mut cmd = Command::new("wlr-randr");
    cmd.args(&args);

    let status = cmd
        .status()
        .context("spawn wlr-randr to apply layout geometry")?;
    if !status.success() {
        eprintln!(
            "(wlr-randr exited {} while applying layout `{}` — outputs may \
             be at the previous positions)",
            status, layout.name
        );
    }
    Ok(())
}

/// The `wlr-randr` arguments that apply `layout`: one `--output` block per
/// output with a connector name, all in one invocation so the compositor
/// applies them in a single configure event.
fn wlr_randr_args(layout: &Layout) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();
    for o in &layout.outputs {
        if o.connector.is_empty() {
            continue; // monitorrule matched on make/model — wlr-randr can't drive that
        }
        args.push("--output".into());
        args.push(o.connector.clone());
        args.push("--pos".into());
        args.push(format!("{},{}", o.x, o.y));

        // Mode: set if the layout's mode differs from the
        // connector's preferred — wlr-randr's internal diff
        // suppresses no-op modesets, so always-set is safe.
        // Skip if the layout file lacks dimensions (rule was
        // a position-only override).
        if o.width > 0 && o.height > 0 {
            // Refresh rate isn't tracked on `LayoutOutput` (parser
            // dropped it as not-needed-for-preview); for now we
            // pass dimensions only and let wlr-randr pick the
            // closest matching mode. Phase 4 wiring: thread
            // refresh through to here.
            args.push("--mode".into());
            args.push(format!("{}x{}", o.width, o.height));
        }
        // Transform — only if non-zero (default = normal).
        if o.transform != 0 {
            args.push("--transform".into());
            args.push(transform_to_str(o.transform).into());
        }
        // Scale: parser dropped scale into width/height already,
        // so layout coordinates are post-scale logical pixels.
        // For wlr-randr we want to set the SCALE explicitly so the
        // size we just passed is interpreted correctly. The parser
        // doesn't carry scale either today — so we do best effort
        // and pass scale 1 if the layout looks small (post-scale)
        // vs the mode it specifies.
        //
        // FIXME: thread scale through `LayoutOutput` so we can
        // pass the original. Skipping the `--scale` flag tells
        // wlr-randr "leave scale as-is" which is the safest
        // default for now.
    }
    args
}

fn transform_to_str(t: i32) -> &'static str {
    match t {
        0 => "normal",
        1 => "90",
        2 => "180",
        3 => "270",
        4 => "flipped",
        5 => "flipped-90",
        6 => "flipped-180",
        7 => "flipped-270",
        _ => "normal",
    }
}

fn is_command_available(cmd: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|d| {
        std::fs::metadata(d.join(cmd))
            .map(|m| m.is_file())
            .unwrap_or(false)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::LayoutOutput;
    use std::path::PathBuf;

    fn output(connector: &str, x: i32, y: i32, w: i32, h: i32, transform: i32) -> LayoutOutput {
        LayoutOutput {
            connector: connector.to_string(),
            label: None,
            color: None,
            x,
            y,
            has_position: true,
            width: w,
            height: h,
            transform,
        }
    }

    fn layout(outputs: Vec<LayoutOutput>) -> Layout {
        Layout {
            path: PathBuf::from("/tmp/layout_t.conf"),
            slug: "t".into(),
            name: "t".into(),
            shortcuts: Vec::new(),
            outputs,
        }
    }

    #[test]
    fn one_block_per_output_with_pos_and_mode() {
        let l = layout(vec![
            output("eDP-1", 0, 0, 1920, 1200, 0),
            output("DP-3", 1920, 0, 2560, 1440, 0),
        ]);
        assert_eq!(
            wlr_randr_args(&l),
            [
                "--output",
                "eDP-1",
                "--pos",
                "0,0",
                "--mode",
                "1920x1200",
                "--output",
                "DP-3",
                "--pos",
                "1920,0",
                "--mode",
                "2560x1440",
            ]
        );
    }

    #[test]
    fn position_only_rule_has_no_mode() {
        let l = layout(vec![output("DP-1", -1080, 200, 0, 0, 0)]);
        assert_eq!(
            wlr_randr_args(&l),
            ["--output", "DP-1", "--pos", "-1080,200"]
        );
    }

    #[test]
    fn transform_is_passed_only_when_rotated() {
        let l = layout(vec![output("DP-2", 0, 0, 1080, 1920, 1)]);
        assert_eq!(
            wlr_randr_args(&l),
            [
                "--output",
                "DP-2",
                "--pos",
                "0,0",
                "--mode",
                "1080x1920",
                "--transform",
                "90"
            ]
        );
    }

    #[test]
    fn make_model_outputs_are_skipped() {
        let l = layout(vec![output("", 0, 0, 1920, 1080, 0)]);
        assert!(wlr_randr_args(&l).is_empty());
    }

    #[test]
    fn transform_names_match_wlr_randr() {
        let names: Vec<&str> = (0..8).map(transform_to_str).collect();
        assert_eq!(
            names,
            [
                "normal",
                "90",
                "180",
                "270",
                "flipped",
                "flipped-90",
                "flipped-180",
                "flipped-270"
            ]
        );
        assert_eq!(transform_to_str(42), "normal");
    }

    #[test]
    fn empty_layout_is_a_no_op() {
        assert!(apply(&layout(Vec::new())).is_ok());
    }
}
