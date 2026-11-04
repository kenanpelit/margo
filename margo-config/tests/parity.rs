//! Parser ⇄ validator parity.
//!
//! margo loads a config with the permissive parser; `mctl check-config`,
//! the reload pre-check and `mctl config-errors` judge it with the
//! validator. When the two disagree, either a working config is refused
//! (a validator error on a line the parser loads fine) or a broken one is
//! waved through (a line the parser drops or misreads with no diagnostic).
//!
//! "Agree" here means, line by line over the whole `source` tree:
//!
//! 1. every validator **error** sits on a line the parser also complained
//!    about (logged a warning or an error) — the validator never refuses
//!    what margo accepts;
//! 2. every line the parser complained about carries at least one
//!    validator diagnostic — nothing margo drops is silent.
//!
//! Checked for every ```` ```ini ```` example in docs/configuration.md, for
//! every config margo ships, and (rule 1) for generated lines.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use margo_config::diagnostics::DiagnosticReport;
use proptest::prelude::*;
use tracing_subscriber::fmt::MakeWriter;

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Capture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Capture {
    type Writer = Capture;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

type Loc = (PathBuf, usize);

fn canon(p: &Path) -> PathBuf {
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
}

/// (file, line) of every warning/error the parser logs for `main`, taken
/// from the innermost `config{file=… line=…}` span. The log text goes
/// alongside so a failure can show it.
fn parser_complaints(main: &Path) -> (BTreeSet<Loc>, String) {
    let capture = Capture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(capture.clone())
        .with_ansi(false)
        .without_time()
        .with_max_level(tracing::Level::WARN)
        .finish();
    tracing::subscriber::with_default(subscriber, || {
        margo_config::parse_config(Some(main)).expect("parse_config");
    });
    let log = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
    let mut out = BTreeSet::new();
    for l in log.lines() {
        let Some(at) = l.rfind("config{file=") else {
            out.insert((canon(main), 0));
            continue;
        };
        let rest = &l[at + "config{file=".len()..];
        let Some((file, rest)) = rest.split_once(" line=") else {
            continue;
        };
        let n: String = rest.chars().take_while(char::is_ascii_digit).collect();
        out.insert((canon(Path::new(file)), n.parse().unwrap_or(0)));
    }
    (out, log)
}

fn validator_report(main: &Path) -> DiagnosticReport {
    margo_config::validator::validate_config(Some(main)).expect("validate_config")
}

/// Rule 1 and rule 2 above; returns the violations as readable lines.
fn disagreements(main: &Path) -> Vec<String> {
    let (complaints, log) = parser_complaints(main);
    let report = validator_report(main);
    let mut out = Vec::new();
    for d in report.errors() {
        if !complaints.contains(&(canon(&d.path), d.line)) {
            out.push(format!(
                "validator error the parser accepts silently: {}:{} {} {} | {}",
                d.path.display(),
                d.line,
                d.code,
                d.message,
                d.line_text
            ));
        }
    }
    let diag_lines: BTreeSet<Loc> = report
        .diagnostics
        .iter()
        .map(|d| (canon(&d.path), d.line))
        .collect();
    for c in &complaints {
        if !diag_lines.contains(c) {
            out.push(format!(
                "parser complaint with no validator diagnostic: {}:{}\n{log}",
                c.0.display(),
                c.1
            ));
        }
    }
    out
}

fn write(dir: &Path, rel: &str, body: &str) -> PathBuf {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, body).unwrap();
    p
}

fn ini_blocks(md: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut cur: Option<(usize, String)> = None;
    for (i, line) in md.lines().enumerate() {
        match &mut cur {
            None if line.trim_start().starts_with("```ini") => cur = Some((i + 1, String::new())),
            Some(_) if line.trim_start().starts_with("```") => out.push(cur.take().unwrap()),
            Some((_, body)) => {
                body.push_str(line);
                body.push('\n');
            }
            None => {}
        }
    }
    out
}

/// Every ```ini example in docs/configuration.md: parser and validator
/// agree, and the example is clean — no error, no parser complaint, no
/// warning. (Its `include`/`source` example points at files the snippet
/// doesn't ship; those get empty stand-ins.)
#[test]
fn docs_configuration_examples_agree_and_are_clean() {
    let md = include_str!("../../docs/configuration.md");
    let blocks = ini_blocks(md);
    assert!(blocks.len() > 10, "found only {} ini blocks", blocks.len());
    let mut failures = Vec::new();
    for (start, body) in blocks {
        let dir = tempfile::tempdir().unwrap();
        // Stand-ins for the include example's fragments.
        write(dir.path(), "keybinds.conf", "");
        write(dir.path(), "rules.conf", "");
        let body = body.replace("~/.config/margo/", "./");
        let main = write(dir.path(), "config.conf", &body);
        for d in disagreements(&main) {
            failures.push(format!("docs/configuration.md block at line {start}: {d}"));
        }
        let (complaints, log) = parser_complaints(&main);
        if !complaints.is_empty() {
            failures.push(format!(
                "docs/configuration.md block at line {start}: parser complains:\n{log}"
            ));
        }
        for d in &validator_report(&main).diagnostics {
            failures.push(format!(
                "docs/configuration.md:{} (block line {}): {} {} | {}",
                start + d.line,
                d.line,
                d.code,
                d.message,
                d.line_text
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// The configs margo ships — the annotated example and the first-run
/// default (with the `binds.conf` and `conf.d/colors.conf` margo writes
/// next to it) — agree and are clean.
#[test]
fn shipped_configs_agree_and_are_clean() {
    let dir = tempfile::tempdir().unwrap();
    let example = write(
        dir.path(),
        "example/config.conf",
        include_str!("../../margo/src/config.example.conf"),
    );
    let default = write(
        dir.path(),
        "default/config.conf",
        include_str!("../../margo/assets/default-config.conf"),
    );
    write(
        dir.path(),
        "default/binds.conf",
        include_str!("../../margo/assets/default-binds.conf"),
    );
    write(dir.path(), "default/conf.d/colors.conf", "# placeholder\n");
    let mut failures = Vec::new();
    for main in [&example, &default] {
        failures.extend(disagreements(main));
        let (complaints, log) = parser_complaints(main);
        if !complaints.is_empty() {
            failures.push(format!("{}: parser complains:\n{log}", main.display()));
        }
        for d in &validator_report(main).diagnostics {
            failures.push(format!(
                "{}:{} {} {} | {}",
                d.path.display(),
                d.line,
                d.code,
                d.message,
                d.line_text
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// The documented `var` syntax (docs/configuration.md "Configuration
/// variables") in a bind and in a `source` path: the parser loads both,
/// so the validator must not call them errors.
#[test]
fn vars_in_bind_and_source_are_expanded_like_the_parser() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "conf.d/a.conf",
        "gappih = 5\nbind = $mod,q,killclient\n",
    );
    let main = write(
        dir.path(),
        "config.conf",
        "var = mod, super\nvar = dir, conf.d\nbind = $mod,Return,spawn,kitty\nsource = $dir/a.conf\n",
    );
    let report = validator_report(&main);
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    assert!(
        disagreements(&main).is_empty(),
        "{:?}",
        disagreements(&main)
    );
    let cfg = margo_config::parse_config(Some(&main)).unwrap();
    assert_eq!(cfg.gappih, 5);
}

/// `source = dir/*.conf` (documented as a glob) loads every match in
/// lexical order in the parser, and the validator walks the same files.
#[test]
fn source_glob_loads_matches_in_lexical_order() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "conf.d/10-a.conf", "gappih = 5\n");
    write(
        dir.path(),
        "conf.d/20-b.conf",
        "gappih = 7\nbind = super,q,bogus_action\n",
    );
    write(dir.path(), "conf.d/.hidden.conf", "gappih = 99\n");
    write(dir.path(), "conf.d/notes.txt", "gappih = 98\n");
    let main = write(dir.path(), "config.conf", "source = conf.d/*.conf\n");
    let cfg = margo_config::parse_config(Some(&main)).unwrap();
    assert_eq!(cfg.gappih, 7, "both fragments load, 20-b last");
    let report = validator_report(&main);
    assert!(!report.has_errors(), "{:?}", report.diagnostics);
    // The validator walked into the fragment (W005 for the bad action).
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.code == "W005" && d.path.ends_with("20-b.conf")),
        "{:?}",
        report.diagnostics
    );
    // A glob that matches nothing is fine (an empty fragments directory).
    let main = write(dir.path(), "empty.conf", "source = nothing.d/*.conf\n");
    assert!(validator_report(&main).diagnostics.is_empty());
    assert!(disagreements(&main).is_empty());
}

const KEYS: &[&str] = &[
    "bind",
    "binds",
    "bindsl",
    "mousebind",
    "axisbind",
    "switchbind",
    "gesturebind",
    "windowrule",
    "layerrule",
    "tagrule",
    "monitorrule",
    "env",
    "var",
    "exec",
    "exec-once",
    "source",
    "include",
    "circle_layout",
    "taglayout",
    "keymode",
    "borderpx",
    "default_mfact",
    "animations",
    "focuscolor",
    "mru_filter",
    "monitor",
];

fn value() -> impl Strategy<Value = String> {
    prop_oneof![
        "[ -~]{0,30}",
        Just(String::new()),
        Just(",".to_string()),
        Just(",,".to_string()),
        Just("a,,b".to_string()),
        Just("super,q,killclient,".to_string()),
        Just(",super,q,killclient".to_string()),
        Just("super,,q,killclient".to_string()),
        Just("$mod,q,killclient".to_string()),
        Just("super,q,spawn,sh -c 'a,,b'".to_string()),
        Just("isfloating:1,appid:^x$,".to_string()),
        Just("mod, super".to_string()),
        Just("x".to_string()),
        Just("name:DP-1,scale:2".to_string()),
        Just("FOO,a,,b".to_string()),
        Just("tile,,scroller".to_string()),
        Just("conf.d/*.conf".to_string()),
    ]
}

fn line() -> impl Strategy<Value = String> {
    (proptest::sample::select(KEYS), value()).prop_map(|(k, v)| format!("{k} = {v}"))
}

proptest! {
    /// Rule 1 on generated lines: the validator never reports an error on
    /// a line the parser loads without complaint.
    #[test]
    fn validator_errors_imply_parser_complaints(
        lines in proptest::collection::vec(line(), 1..12)
    ) {
        let dir = tempfile::tempdir().unwrap();
        let main = write(dir.path(), "config.conf", &lines.join("\n"));
        let (complaints, log) = parser_complaints(&main);
        for d in validator_report(&main).errors() {
            prop_assert!(
                complaints.contains(&(canon(&d.path), d.line)),
                "validator {} on a line the parser accepts: {} | {}\nparser log:\n{}",
                d.code, d.message, d.line_text, log
            );
        }
    }
}
