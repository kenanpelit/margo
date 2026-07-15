//! Which keyboard layout the greeter is actually typing in.
//!
//! A login screen that will not accept your password is the worst place to
//! discover that the machine came up in `us` while your keyboard is Turkish-F.
//! The layout is not switchable here — the greeter's compositor is started with
//! exactly one keymap — so this is a statement, not a control.
//!
//! `mlogind`'s runner translates `/etc/vconsole.conf` into `XKB_DEFAULT_*` and
//! puts them in the greeter compositor's environment, which we inherit; that is
//! the same keymap margo hands us, so reading the environment is reading the
//! truth rather than guessing at it. Under `--preview` there is no such
//! environment, and we fall back to the console config the runner would have
//! read.

const VCONSOLE: &str = "/etc/vconsole.conf";

/// Every configured layout group, formatted — e.g. `["tr(f)", "us"]` — in xkb
/// group order, group 0 (the group the compositor starts in) first. Empty when
/// nothing configured a layout, in which case the card says nothing rather
/// than asserting a default it did not verify.
///
/// With more than one entry the badge becomes a switcher: margo's
/// `cyclekblayout` dispatch steps through exactly this list in this order.
pub fn layouts() -> Vec<String> {
    let (layout, variant) = match env("XKB_DEFAULT_LAYOUT") {
        Some(layout) => (layout, env("XKB_DEFAULT_VARIANT")),
        None => {
            let Ok(text) = std::fs::read_to_string(VCONSOLE) else {
                return Vec::new();
            };
            let (layout, variant) = parse_vconsole(&text);
            let Some(layout) = layout else {
                return Vec::new();
            };
            (layout, variant)
        }
    };
    all_groups(&layout, variant.as_deref())
}

/// Format every group of a layout/variant list pair, in order.
fn all_groups(layout: &str, variant: Option<&str>) -> Vec<String> {
    (0..layout.split(',').count())
        .filter_map(|index| format_group(layout, variant, index))
        .collect()
}

fn env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.trim().is_empty())
}

/// `XKBLAYOUT` / `XKBVARIANT` out of `/etc/vconsole.conf`. Values may be quoted
/// (`localectl` writes `XKBLAYOUT="tr"`), and unrelated keys are ignored.
fn parse_vconsole(text: &str) -> (Option<String>, Option<String>) {
    let mut layout = None;
    let mut variant = None;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"').trim_matches('\'');
        if value.is_empty() {
            continue;
        }
        match key.trim() {
            "XKBLAYOUT" => layout = Some(value.to_string()),
            "XKBVARIANT" => variant = Some(value.to_string()),
            _ => {}
        }
    }
    (layout, variant)
}

/// One group of an xkb layout/variant pair, rendered `layout(variant)`.
///
/// Both fields are comma-separated lists indexed in lock step: `us,tr` with
/// `,f` means plain `us` and Turkish-F. Index into each at the same position —
/// never "the first non-empty variant", which would hang `f` off `us`.
fn format_group(layout: &str, variant: Option<&str>, index: usize) -> Option<String> {
    let layout = group(layout, index)?;
    match variant.and_then(|v| group(v, index)) {
        Some(variant) => Some(format!("{layout}({variant})")),
        None => Some(layout.to_string()),
    }
}

fn group(list: &str, index: usize) -> Option<&str> {
    list.split(',')
        .nth(index)
        .map(str::trim)
        .filter(|group| !group.is_empty())
}

#[cfg(test)]
mod tests {
    use super::{all_groups, format_group, group, parse_vconsole};

    #[test]
    fn a_layout_with_a_variant_reads_as_the_xkb_spelling() {
        assert_eq!(format_group("tr", Some("f"), 0).as_deref(), Some("tr(f)"));
    }

    #[test]
    fn a_layout_without_a_variant_is_bare() {
        assert_eq!(format_group("us", None, 0).as_deref(), Some("us"));
        assert_eq!(format_group("us", Some(""), 0).as_deref(), Some("us"));
    }

    #[test]
    fn variants_stay_in_lock_step_with_their_layouts() {
        // `us,tr` + `,f`: the `f` belongs to `tr`, and group 0 is `us`.
        assert_eq!(format_group("us,tr", Some(",f"), 0).as_deref(), Some("us"));
        assert_eq!(
            format_group("us,tr", Some(",f"), 1).as_deref(),
            Some("tr(f)")
        );
        assert_eq!(
            format_group("tr,us", Some("f,"), 0).as_deref(),
            Some("tr(f)")
        );
        assert_eq!(format_group("tr,us", Some("f,"), 1).as_deref(), Some("us"));
    }

    #[test]
    fn every_group_is_listed_in_compositor_order() {
        // This order IS the switcher contract: margo's cyclekblayout steps
        // through the same list, so index i here is xkb group i there.
        assert_eq!(all_groups("tr,us", Some("f,")), vec!["tr(f)", "us"]);
        assert_eq!(all_groups("us", None), vec!["us"]);
    }

    #[test]
    fn a_list_that_starts_empty_names_no_layout() {
        assert_eq!(format_group("", None, 0), None);
        assert_eq!(format_group(",tr", None, 0), None);
        assert_eq!(group("  ,f", 0), None);
        assert!(all_groups("", None).is_empty());
    }

    #[test]
    fn vconsole_values_may_be_quoted() {
        let text = "KEYMAP=\"trf\"\nXKBLAYOUT=\"tr\"\nXKBVARIANT=\"f\"\n";
        let (layout, variant) = parse_vconsole(text);
        assert_eq!(layout.as_deref(), Some("tr"));
        assert_eq!(variant.as_deref(), Some("f"));
    }

    #[test]
    fn vconsole_without_an_xkb_layout_says_so() {
        let (layout, variant) = parse_vconsole("KEYMAP=us\nFONT=lat9w-16\n");
        assert_eq!(layout, None);
        assert_eq!(variant, None);
    }

    #[test]
    fn vconsole_comments_and_empty_values_are_skipped() {
        let text = "# XKBLAYOUT=de\nXKBLAYOUT=tr\nXKBVARIANT=\n";
        let (layout, variant) = parse_vconsole(text);
        assert_eq!(layout.as_deref(), Some("tr"));
        assert_eq!(variant, None);
    }
}
