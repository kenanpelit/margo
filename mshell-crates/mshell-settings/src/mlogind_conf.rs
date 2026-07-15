//! Read + surgically edit `/etc/mlogind/config.toml` for Settings → Login.
//!
//! The file is root-owned **real config** — an admin may have edited it, and
//! mlogind ships it commented. So the writer is a *surgical text editor*, not
//! a serializer: it replaces exactly the `key = value` lines it manages
//! (creating the key or its `[section]` when missing) and leaves every other
//! byte — comments, unknown keys, ordering — untouched. Same philosophy as
//! `compositor_conf.rs` for margo's `.conf`.
//!
//! Reading is a matching section-aware scan. Only the handful of value shapes
//! the Login page needs are parsed (basic strings, booleans, integers); a
//! line this scanner cannot read is simply not reported, and the page shows
//! the default — it never blocks the login path.
//!
//! The *write to disk* is not here: the file is root's, so the page ships the
//! edited text through `sudo -n install` (see `login_settings.rs`).

/// The mlogind keys Settings → Login manages, with their defaults.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LoginConf {
    pub host: String,
    pub background_dir: String,
    pub greeter_css: String,
    pub osk: bool,
    pub blank_timeout: u32,
    pub autologin_user: String,
    pub autologin_session: String,
}

impl Default for LoginConf {
    fn default() -> Self {
        // Mirrors mlogind's baked extra/config.toml defaults.
        Self {
            host: "gui".to_string(),
            background_dir: String::new(),
            greeter_css: String::new(),
            osk: false,
            blank_timeout: 300,
            autologin_user: String::new(),
            autologin_session: String::new(),
        }
    }
}

pub(crate) const CONFIG_PATH: &str = "/etc/mlogind/config.toml";

/// A value the editor can write. Strings are emitted as TOML basic strings
/// (quoted, `\` and `"` escaped); booleans and integers bare.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Value {
    Str(String),
    Bool(bool),
    Int(i64),
}

impl Value {
    fn render(&self) -> String {
        match self {
            Value::Str(s) => format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")),
            Value::Bool(b) => b.to_string(),
            Value::Int(i) => i.to_string(),
        }
    }
}

/// One managed `key = value` under a `[section]`.
#[derive(Debug, Clone)]
pub(crate) struct Edit {
    pub section: &'static str,
    pub key: &'static str,
    pub value: Value,
}

/// Load the current on-disk config. Missing or unreadable → defaults; the
/// file is 0644 so the ordinary case reads fine without privilege.
pub(crate) fn load() -> LoginConf {
    match std::fs::read_to_string(CONFIG_PATH) {
        Ok(text) => parse(&text),
        Err(_) => LoginConf::default(),
    }
}

/// Parse the managed keys out of a config text; anything absent or unreadable
/// keeps its default.
pub(crate) fn parse(text: &str) -> LoginConf {
    let mut conf = LoginConf::default();
    let mut section = String::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(name) = section_header(trimmed) {
            section = name.to_string();
            continue;
        }
        let Some((key, raw)) = split_key_value(trimmed) else {
            continue;
        };
        match (section.as_str(), key) {
            ("display", "host") => {
                if let Some(v) = parse_string(raw) {
                    conf.host = v;
                }
            }
            ("display", "background_dir") => {
                if let Some(v) = parse_string(raw) {
                    conf.background_dir = v;
                }
            }
            ("display", "greeter_css") => {
                if let Some(v) = parse_string(raw) {
                    conf.greeter_css = v;
                }
            }
            ("display", "osk") => {
                if let Some(v) = parse_bool(raw) {
                    conf.osk = v;
                }
            }
            ("display", "blank_timeout") => {
                if let Some(v) = parse_int(raw) {
                    conf.blank_timeout = v.clamp(0, u32::MAX as i64) as u32;
                }
            }
            ("autologin", "user") => {
                if let Some(v) = parse_string(raw) {
                    conf.autologin_user = v;
                }
            }
            ("autologin", "session") => {
                if let Some(v) = parse_string(raw) {
                    conf.autologin_session = v;
                }
            }
            _ => {}
        }
    }
    conf
}

/// Apply every edit to `text`, replacing managed lines in place and creating
/// missing keys/sections, leaving everything else byte-identical. Returns the
/// new file content (always newline-terminated).
pub(crate) fn apply_edits(text: &str, edits: &[Edit]) -> String {
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    for edit in edits {
        apply_one(&mut lines, edit);
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

fn apply_one(lines: &mut Vec<String>, edit: &Edit) {
    let rendered = format!("{} = {}", edit.key, edit.value.render());

    // Find the section's line range.
    let Some(start) = lines
        .iter()
        .position(|l| section_header(l.trim()) == Some(edit.section))
    else {
        // No such section: append one at the end. The blank line keeps it
        // readable next to whatever the admin left above.
        if lines.last().is_some_and(|l| !l.trim().is_empty()) {
            lines.push(String::new());
        }
        lines.push(format!("[{}]", edit.section));
        lines.push(rendered);
        return;
    };
    let end = lines[start + 1..]
        .iter()
        .position(|l| l.trim().starts_with('['))
        .map(|off| start + 1 + off)
        .unwrap_or(lines.len());

    // Replace the key's line if it exists in the section.
    for line in &mut lines[start + 1..end] {
        if split_key_value(line.trim()).is_some_and(|(k, _)| k == edit.key) {
            *line = rendered;
            return;
        }
    }

    // Absent: insert after the section's last non-empty line, so trailing
    // blank lines stay where they were (they belong to the next section
    // visually).
    let insert_at = lines[start + 1..end]
        .iter()
        .rposition(|l| !l.trim().is_empty())
        .map(|off| start + 1 + off + 1)
        .unwrap_or(start + 1);
    lines.insert(insert_at, rendered);
}

/// `[name]` → `name`. Not a TOML array-of-tables parser — mlogind's
/// `[[power_controls.base_entries]]` headers simply never match a managed
/// section name, which is all the editor needs.
fn section_header(trimmed: &str) -> Option<&str> {
    trimmed
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.starts_with('['))
}

/// `key = rest` for a plain assignment line; comments and section headers
/// yield `None`.
fn split_key_value(trimmed: &str) -> Option<(&str, &str)> {
    if trimmed.starts_with('#') || trimmed.starts_with('[') {
        return None;
    }
    let (key, rest) = trimmed.split_once('=')?;
    let key = key.trim();
    if key.is_empty() || key.contains(char::is_whitespace) {
        return None;
    }
    Some((key, rest.trim()))
}

/// A TOML basic string: `"…"` with `\"` / `\\` escapes. Anything after the
/// closing quote (a comment, junk) is ignored.
fn parse_string(raw: &str) -> Option<String> {
    let rest = raw.strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                'n' => out.push('\n'),
                't' => out.push('\t'),
                other => {
                    out.push('\\');
                    out.push(other);
                }
            },
            other => out.push(other),
        }
    }
    None // unterminated
}

fn parse_bool(raw: &str) -> Option<bool> {
    match strip_comment(raw) {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn parse_int(raw: &str) -> Option<i64> {
    strip_comment(raw).parse().ok()
}

/// Bare (unquoted) values may carry a trailing `# comment`.
fn strip_comment(raw: &str) -> &str {
    raw.split('#').next().unwrap_or("").trim()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
tty = 2

# How the greeter is hosted.
[display]
host = \"gui\"      # gui | cage | tty
blank_timeout = 300
dynamic_vt = false

[autologin]
user = \"\"
session = \"\"
";

    #[test]
    fn the_managed_keys_parse_and_the_rest_defaults() {
        let conf = parse(SAMPLE);
        assert_eq!(conf.host, "gui");
        assert_eq!(conf.blank_timeout, 300);
        // Not in the sample → defaults.
        assert_eq!(conf.background_dir, "");
        assert!(!conf.osk);
        assert_eq!(conf.autologin_user, "");
    }

    #[test]
    fn an_existing_key_is_replaced_in_place_and_comments_survive() {
        let out = apply_edits(
            SAMPLE,
            &[Edit {
                section: "display",
                key: "blank_timeout",
                value: Value::Int(0),
            }],
        );
        assert!(out.contains("blank_timeout = 0"));
        assert!(!out.contains("blank_timeout = 300"));
        // Everything the editor does not manage is byte-identical.
        assert!(out.contains("# How the greeter is hosted."));
        assert!(out.contains("dynamic_vt = false"));
        assert!(out.contains("tty = 2"));
    }

    #[test]
    fn a_missing_key_lands_inside_its_section_not_the_next_one() {
        let out = apply_edits(
            SAMPLE,
            &[Edit {
                section: "display",
                key: "background_dir",
                value: Value::Str("/srv/photos".into()),
            }],
        );
        let display = out.find("[display]").unwrap();
        let autologin = out.find("[autologin]").unwrap();
        let key = out.find("background_dir = \"/srv/photos\"").unwrap();
        assert!(display < key && key < autologin);
    }

    #[test]
    fn a_missing_section_is_appended_whole() {
        let out = apply_edits(
            "tty = 2\n",
            &[Edit {
                section: "autologin",
                key: "user",
                value: Value::Str("kenan".into()),
            }],
        );
        assert!(out.ends_with("[autologin]\nuser = \"kenan\"\n"));
    }

    #[test]
    fn strings_round_trip_through_render_and_parse() {
        // A path with quotes and backslashes must come back unharmed —
        // whatever we write, the next page load re-reads.
        let tricky = r#"/pics/it's "fine"\really"#;
        let out = apply_edits(
            SAMPLE,
            &[Edit {
                section: "display",
                key: "background_dir",
                value: Value::Str(tricky.into()),
            }],
        );
        assert_eq!(parse(&out).background_dir, tricky);
    }

    #[test]
    fn comments_after_bare_values_do_not_poison_them() {
        let conf = parse("[display]\nosk = true # touch login\nblank_timeout = 60 # s\n");
        assert!(conf.osk);
        assert_eq!(conf.blank_timeout, 60);
        // And a '#' inside a quoted string is not a comment.
        let conf = parse("[display]\nbackground_dir = \"/pics/#1\"\n");
        assert_eq!(conf.background_dir, "/pics/#1");
    }

    #[test]
    fn editing_the_same_key_twice_keeps_one_line() {
        let out = apply_edits(
            SAMPLE,
            &[
                Edit {
                    section: "autologin",
                    key: "user",
                    value: Value::Str("a".into()),
                },
                Edit {
                    section: "autologin",
                    key: "user",
                    value: Value::Str("b".into()),
                },
            ],
        );
        assert_eq!(out.matches("user = ").count(), 1);
        assert_eq!(parse(&out).autologin_user, "b");
    }
}
