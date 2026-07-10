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

/// e.g. `"tr(f)"`, `"us"`. `None` when nothing configured a layout, in which
/// case the card says nothing rather than asserting a default it did not verify.
pub fn layout() -> Option<String> {
    if let Some(layout) = env("XKB_DEFAULT_LAYOUT") {
        return format(&layout, env("XKB_DEFAULT_VARIANT").as_deref());
    }
    let text = std::fs::read_to_string(VCONSOLE).ok()?;
    let (layout, variant) = parse_vconsole(&text);
    format(&layout?, variant.as_deref())
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

/// The first group of an xkb layout/variant pair, rendered `layout(variant)`.
///
/// Both fields are comma-separated lists indexed in lock step: `us,tr` with
/// `,f` means plain `us` and Turkish-F. Only group 0 is ever active in the
/// greeter, so we take index 0 of each — never "the first non-empty variant",
/// which would hang `f` off `us`.
fn format(layout: &str, variant: Option<&str>) -> Option<String> {
    let layout = group(layout, 0)?;
    match variant.and_then(|v| group(v, 0)) {
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
    use super::{format, group, parse_vconsole};

    #[test]
    fn a_layout_with_a_variant_reads_as_the_xkb_spelling() {
        assert_eq!(format("tr", Some("f")).as_deref(), Some("tr(f)"));
    }

    #[test]
    fn a_layout_without_a_variant_is_bare() {
        assert_eq!(format("us", None).as_deref(), Some("us"));
        assert_eq!(format("us", Some("")).as_deref(), Some("us"));
    }

    #[test]
    fn only_the_first_group_of_each_list_is_read() {
        // `us,tr` + `,f`: the `f` belongs to `tr`, and group 0 is `us`.
        assert_eq!(format("us,tr", Some(",f")).as_deref(), Some("us"));
        assert_eq!(format("tr,us", Some("f,")).as_deref(), Some("tr(f)"));
    }

    #[test]
    fn a_list_that_starts_empty_names_no_layout() {
        assert_eq!(format("", None), None);
        assert_eq!(format(",tr", None), None);
        assert_eq!(group("  ,f", 0), None);
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
