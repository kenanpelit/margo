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
    "bind", "bindr", "bindl", "bindc", "bindrl", "bindrc", "bindlc", "bindrlc",
    "mousebind", "gesturebind", "axisbind", "windowrule", "tagrule", "layerrule",
    "monitorrule", "source", "include",
];

pub fn parse(source: &str) -> Vec<ConfigSection> {
    let mut sections: Vec<ConfigSection> = Vec::new();
    let mut pending_comment: Vec<String> = Vec::new();

    for line in source.lines() {
        let trimmed = line.trim_end();

        if let Some(title) = section_title(trimmed) {
            sections.push(ConfigSection {
                title: title.to_string(),
                keys: Vec::new(),
            });
            pending_comment.clear();
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
            if let Some(section) = sections.last_mut() {
                section.keys.push(ConfigKey {
                    key,
                    default,
                    description: pending_comment.join(" ").trim().to_string(),
                });
            }
            pending_comment.clear();
            continue;
        }

        // Anything else (shouldn't happen in practice) resets the pending
        // comment run rather than attaching stale prose to the next key.
        pending_comment.clear();
    }

    sections
}

/// `# ── Title ──...──` -> `Some("Title")`. Requires the line to actually
/// start with `# ──` (the file's own divider convention) so an ordinary
/// `#` comment starting with a dash never misparses as a header.
fn section_title(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("# ── ")?;
    let title_end = rest.find(" ──")?;
    Some(&rest[..title_end])
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
}
