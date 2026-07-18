//! Property-based fuzzing of the hand-written config parser.
//!
//! The parser is ~2000 lines of manual string surgery and the classic
//! compositor crash source is exactly this code path (a bad line in
//! `config.conf` taking the session down at `mctl reload` time). Three
//! generators attack it:
//!
//! 1. arbitrary text — any unicode soup must parse without panicking;
//! 2. structured key=value lines — real key names paired with
//!    adversarial values (overflow numbers, empty fields, stray
//!    commas, regex metacharacters) to reach the per-key arms;
//! 3. mutated real config — the shipped `config.example.conf` with
//!    random line drops, duplications, truncations and splices, so
//!    coverage reaches the deep multi-line forms (binds, rules,
//!    gestures) that random text never hits.
//!
//! The invariant everywhere is the same: `parse_config` and
//! `validate_config` RETURN (Ok or Err) — they never panic, whatever
//! the input. 256 cases per property by default; soak with
//! `PROPTEST_CASES=100000 cargo test -p margo-config`.

use std::io::Write;

use margo_config::{parse_config, validator::validate_config};
use proptest::prelude::*;

/// The shipped example config — the richest real-world corpus we have
/// (binds, monitor/window/tag rules, gestures, env, includes).
const EXAMPLE: &str = include_str!("../../margo/src/config.example.conf");

/// Representative key names. Not exhaustive — the mutation generator
/// covers the rest — but enough to reach every *shape* of value
/// parser (bool, int, float, color, list, regex, bind grammar).
const KEYS: &[&str] = &[
    "default_layout",
    "circle_layout",
    "default_mfact",
    "default_nmaster",
    "gappih",
    "gappiv",
    "gappoh",
    "gappov",
    "border_width",
    "focused_border_color",
    "animations",
    "animation_duration",
    "bind",
    "gesturebind",
    "monitorrule",
    "windowrule",
    "tagrule",
    "taglayout",
    "env",
    "exec",
    "exec_once",
    "source",
    "hot_corner_dwell_ms",
    "scroller_default_proportion",
    "repeat_rate",
    "cursor_size",
    "color_management",
];

fn adversarial_value() -> impl Strategy<Value = String> {
    prop_oneof![
        // Plain garbage.
        "[ -~]{0,40}",
        // Overflow-shaped numbers.
        Just("99999999999999999999".to_string()),
        Just("-2147483649".to_string()),
        Just("3.4e39".to_string()),
        Just("NaN".to_string()),
        // Empty / whitespace / separators only.
        Just(String::new()),
        Just("   ".to_string()),
        Just(",,,,,,,".to_string()),
        Just(", , , , , , , , ,".to_string()),
        // Bind-grammar shrapnel.
        Just("super+shift".to_string()),
        Just("super, k".to_string()),
        Just("SUPER,code:99999,view,4294967295".to_string()),
        // Regex metacharacters (window rules compile PCRE2-style).
        Just("^($[".to_string()),
        Just("(?P<x>*)".to_string()),
        Just("a{99999999}".to_string()),
        // Unicode.
        Just("çğıöşü–🚀\u{202e}".to_string()),
    ]
}

fn structured_line() -> impl Strategy<Value = String> {
    (
        proptest::sample::select(KEYS),
        adversarial_value(),
        0usize..4,
    )
        .prop_map(|(key, value, decor)| match decor {
            0 => format!("{key}={value}"),
            1 => format!("  {key} = {value}  # trailing comment"),
            2 => format!("{key}={value}={value}"),
            _ => format!("{key} {value}"),
        })
}

/// Parse + validate a config body; the property is simply "no panic".
fn parse_no_panic(body: &str) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.conf");
    {
        let mut f = std::fs::File::create(&path).expect("create temp config");
        f.write_all(body.as_bytes()).expect("write temp config");
    }
    let _ = parse_config(Some(&path));
    let _ = validate_config(Some(&path));
}

proptest! {
    #[test]
    fn arbitrary_text_never_panics(body in "\\PC{0,2000}") {
        parse_no_panic(&body);
    }

    #[test]
    fn structured_lines_never_panic(lines in proptest::collection::vec(structured_line(), 0..40)) {
        parse_no_panic(&lines.join("\n"));
    }

    #[test]
    fn mutated_example_config_never_panics(
        seed in any::<u64>(),
        drops in proptest::collection::vec(0usize..2000, 0..30),
        truncate_at in proptest::option::of(0usize..60_000),
        splice in proptest::collection::vec(structured_line(), 0..10),
    ) {
        let mut lines: Vec<&str> = EXAMPLE.lines().collect();
        // Drop / duplicate some lines (index reuse duplicates the
        // swap-in, which is exactly the point).
        for d in &drops {
            if !lines.is_empty() {
                let i = d % lines.len();
                if seed & 1 == 0 {
                    lines.remove(i);
                } else {
                    let dup = lines[i];
                    lines.insert(i, dup);
                }
            }
        }
        let mut body = lines.join("\n");
        // Mid-byte truncation — must land on a char boundary to stay
        // a valid &str slice (parse only ever sees UTF-8 anyway:
        // read_to_string rejects invalid sequences before the parser).
        if let Some(t) = truncate_at
            && t < body.len()
        {
            let mut cut = t;
            while !body.is_char_boundary(cut) {
                cut -= 1;
            }
            body.truncate(cut);
        }
        for line in splice {
            body.push('\n');
            body.push_str(&line);
        }
        parse_no_panic(&body);
    }
}
