//! Turns `margo/src/config.example.conf` into structured sections/keys for
//! the Guide page's Settings tab. Deliberately ignores directive lines
//! (`bind`, `windowrule`, `tagrule`, `layerrule`, `monitorrule`,
//! `mousebind`, `gesturebind`, `axisbind`, `source`, `include`) — those are
//! reference *examples*, not standalone settings; showing all ~120 of them
//! as "settings" would bury the ~240 real keys.

/// One `key = value` line from `config.example.conf`, with the `#`-comment
/// lines that preceded it (its section header's own preceding comments if
/// none were directly above this key) as its description.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigKey {
    pub key: String,
    pub default: String,
    pub description: String,
}

/// One `# ── Title ──...──` divider and the keys under it, up to the next
/// divider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigSection {
    pub title: String,
    pub keys: Vec<ConfigKey>,
}

const DIRECTIVE_NAMES: &[&str] = &[
    "bind",
    "bindr",
    "bindl",
    "bindc",
    "bindrl",
    "bindrc",
    "bindlc",
    "bindrlc",
    "mousebind",
    "gesturebind",
    "axisbind",
    "windowrule",
    "tagrule",
    "layerrule",
    "monitorrule",
    "source",
    "include",
    // Repeatable, like the directives above (7 occurrences in the real
    // file) rather than a scalar setting -- excluded for the same reason.
    "env",
];

pub fn parse(source: &str) -> Vec<ConfigSection> {
    let mut sections: Vec<ConfigSection> = Vec::new();
    let mut pending_comment: Vec<String> = Vec::new();
    // The comment run directly above a section's *first* key doubles as
    // that section's fallback description for any later key in the same
    // section that has no comment of its own -- config.example.conf's
    // usual shape is one intro comment followed by several bare keys.
    let mut section_intro = String::new();
    let mut section_has_keys = false;

    // Seed a synthetic leading section so keys appearing before the
    // file's first section header (e.g. `borderpx`, the four `gap*`
    // knobs, `focused_opacity`/`unfocused_opacity`) aren't silently
    // dropped -- `config.example.conf` opens with several real keys
    // under its "1. Look" chapter header before the first `## Palette`
    // sub-divider. Removed at the end if it stayed empty.
    sections.push(ConfigSection {
        title: "General".to_string(),
        keys: Vec::new(),
    });

    for line in source.lines() {
        let trimmed = line.trim_end();

        if let Some(title) = section_title(trimmed) {
            sections.push(ConfigSection {
                title: title.to_string(),
                keys: Vec::new(),
            });
            pending_comment.clear();
            section_intro.clear();
            section_has_keys = false;
            continue;
        }

        if let Some(comment) = trimmed.strip_prefix('#') {
            pending_comment.push(comment.trim().to_string());
            continue;
        }

        if trimmed.trim().is_empty() {
            pending_comment.clear();
            continue;
        }

        if let Some((key, default)) = key_value(trimmed) {
            if DIRECTIVE_NAMES.contains(&key.as_str()) {
                pending_comment.clear();
                continue;
            }
            let own_comment = pending_comment.join(" ").trim().to_string();
            if !section_has_keys && !own_comment.is_empty() {
                section_intro = own_comment.clone();
            }
            section_has_keys = true;
            let description = if own_comment.is_empty() {
                section_intro.clone()
            } else {
                own_comment
            };
            if let Some(section) = sections.last_mut() {
                section.keys.push(ConfigKey {
                    key,
                    default,
                    description,
                });
            }
            pending_comment.clear();
            continue;
        }

        // Anything else (shouldn't happen in practice) resets the pending
        // comment run rather than attaching stale prose to the next key.
        pending_comment.clear();
    }

    if sections
        .first()
        .is_some_and(|s| s.title == "General" && s.keys.is_empty())
    {
        sections.remove(0);
    }

    sections
}

/// `# ── Title ──...──` -> `Some("Title")`, or the file's chapter-header
/// box-drawing form `# │ N. Title ... │` -> `Some("N. Title ...")`.
/// Requires an exact prefix match on one of the two divider conventions
/// so an ordinary `#` comment never misparses as a header.
fn section_title(line: &str) -> Option<&str> {
    if let Some(rest) = line.strip_prefix("# ── ") {
        let title_end = rest.find(" ──")?;
        return Some(&rest[..title_end]);
    }
    if let Some(rest) = line.strip_prefix("# │ ") {
        let title = rest.strip_suffix('│')?.trim_end();
        if !title.is_empty() {
            return Some(title);
        }
    }
    None
}

/// `key = value` (optionally with a trailing `  # inline comment`) ->
/// `Some(("key", "value"))`. Rejects anything whose "key" isn't a bare
/// identifier (covers indented / commented-out lines, which never start
/// at column 0 with a plain word).
fn key_value(line: &str) -> Option<(String, String)> {
    let (key, rest) = line.split_once('=')?;
    let key = key.trim();
    if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    let value = rest.split(" #").next().unwrap_or(rest).trim();
    Some((key.to_string(), value.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_key_value_line() {
        let src = "\
# ── Example ────────────────────────────────────────────────────────────────
# Comment for foo.
foo = bar
";
        let sections = parse(src);
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].title, "Example");
        assert_eq!(sections[0].keys.len(), 1);
        assert_eq!(sections[0].keys[0].key, "foo");
        assert_eq!(sections[0].keys[0].default, "bar");
        assert_eq!(sections[0].keys[0].description, "Comment for foo.");
    }

    #[test]
    fn multiple_comment_lines_concatenate() {
        let src = "\
# ── Example ────────────────────────────────────────────────────────────────
# First line.
# Second line.
foo = bar
";
        let sections = parse(src);
        assert_eq!(sections[0].keys[0].description, "First line. Second line.");
    }

    #[test]
    fn trailing_inline_comment_is_stripped_from_the_default() {
        let src = "\
# ── Example ────────────────────────────────────────────────────────────────
foo = bar    # not part of the value
";
        let sections = parse(src);
        assert_eq!(sections[0].keys[0].default, "bar");
    }

    #[test]
    fn directive_lines_are_skipped() {
        let src = "\
# ── Example ────────────────────────────────────────────────────────────────
bind = super,q,killclient
windowrule = isfloating:1, appid:^pavucontrol$
foo = bar
";
        let sections = parse(src);
        assert_eq!(sections[0].keys.len(), 1);
        assert_eq!(sections[0].keys[0].key, "foo");
    }

    #[test]
    fn key_with_no_preceding_comment_has_empty_description() {
        let src = "\
# ── Example ────────────────────────────────────────────────────────────────
foo = bar
baz = qux
";
        let sections = parse(src);
        // `foo` has no comment directly above it either (the divider isn't
        // a description) so its description is empty; `baz` likewise.
        assert_eq!(sections[0].keys[0].description, "");
        assert_eq!(sections[0].keys[1].description, "");
    }

    #[test]
    fn two_sections_split_correctly() {
        let src = "\
# ── First ───────────────────────────────────────────────────────────────────
a = 1
# ── Second ──────────────────────────────────────────────────────────────────
b = 2
";
        let sections = parse(src);
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].title, "First");
        assert_eq!(sections[1].title, "Second");
    }

    #[test]
    fn keys_before_the_first_section_header_are_not_dropped() {
        let src = "\
# Some preamble prose, not a divider.
borderpx = 3
# ── Palette ──────────────────────────────────────────────────────────────────
rootcolor = 0x1e1e2eff
";
        let sections = parse(src);
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].keys[0].key, "borderpx");
        assert_eq!(sections[1].title, "Palette");
    }

    #[test]
    fn box_drawing_chapter_headers_are_recognized_as_section_titles() {
        let src = "\
# ╭───────────────────────────────────────────────────────────────────────────╮
# │ 1. Look — borders, gaps, opacity, colors, shadows, blur                   │
# ╰───────────────────────────────────────────────────────────────────────────╯
borderpx = 3
";
        let sections = parse(src);
        assert_eq!(sections.len(), 1);
        assert_eq!(
            sections[0].title,
            "1. Look — borders, gaps, opacity, colors, shadows, blur"
        );
        assert_eq!(sections[0].keys[0].key, "borderpx");
    }

    #[test]
    fn env_is_treated_as_a_directive_not_a_setting() {
        let src = "\
# ── Example ────────────────────────────────────────────────────────────────
env = XDG_CURRENT_DESKTOP, margo
foo = bar
";
        let sections = parse(src);
        assert_eq!(sections[0].keys.len(), 1);
        assert_eq!(sections[0].keys[0].key, "foo");
    }

    #[test]
    fn later_keys_in_a_section_inherit_its_first_keys_comment_as_a_fallback() {
        let src = "\
# ── Blur ─────────────────────────────────────────────────────────────────────
# Prose describing blur.
blur = 0
blur_layer = 0
";
        let sections = parse(src);
        assert_eq!(sections[0].keys[0].description, "Prose describing blur.");
        assert_eq!(sections[0].keys[1].description, "Prose describing blur.");
    }

    #[test]
    fn a_keys_own_comment_still_wins_over_the_section_fallback() {
        let src = "\
# ── Blur ─────────────────────────────────────────────────────────────────────
# Section intro.
blur = 0
# Specific to blur_layer.
blur_layer = 0
";
        let sections = parse(src);
        assert_eq!(sections[0].keys[0].description, "Section intro.");
        assert_eq!(sections[0].keys[1].description, "Specific to blur_layer.");
    }

    /// Guards against `config.example.conf` regressing on any of the three
    /// bugs fixed above (dropped pre-divider keys, un-recognized chapter
    /// headers, `env` leaking through as a fake setting) -- fails loudly
    /// here instead of silently shrinking the Settings tab.
    #[test]
    fn parses_the_real_config_example_conf_without_dropping_known_keys() {
        let source = include_str!("../../../../margo/src/config.example.conf");
        let sections = parse(source);
        let all_keys: Vec<&str> = sections
            .iter()
            .flat_map(|s| s.keys.iter().map(|k| k.key.as_str()))
            .collect();

        for expected in [
            "borderpx",
            "border_radius",
            "gappih",
            "gappiv",
            "gappoh",
            "gappov",
            "focused_opacity",
            "unfocused_opacity",
            "cursor_size",
            "blur",
            "default_layout",
        ] {
            assert!(
                all_keys.contains(&expected),
                "expected config.example.conf to parse a \"{expected}\" key -- \
                 either the real file changed or config_parser.rs regressed"
            );
        }
        assert!(
            !all_keys.contains(&"env"),
            "\"env\" is a repeatable directive, not a setting -- it should \
             never appear as a parsed key"
        );
        assert!(
            all_keys.len() > 150,
            "expected 150+ real keys, got {} -- a section-attribution bug \
             could inflate/deflate this without changing which keys exist",
            all_keys.len()
        );
    }
}
