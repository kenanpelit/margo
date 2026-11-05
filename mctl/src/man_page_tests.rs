//! Drift gate for the hand-written man pages this crate ships with
//! (`man/mctl.1`, and `man/margo.1`'s title line).
//!
//! `man/mctl.1` keeps its prose, grouping and examples by hand, but its
//! COMMANDS section must name exactly the subcommands clap exposes: a new
//! subcommand fails here until it is documented, a removed one until its
//! entry goes. The `.TH` source field carries no version number — a
//! release only bumps Cargo versions, so a version there goes stale on the
//! next release (it read "margo 0.9.1" for months).

use std::collections::BTreeSet;
use std::path::PathBuf;

use clap::CommandFactory;

use super::Args;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn man_page(name: &str) -> std::io::Result<String> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../man")
        .join(name);
    std::fs::read_to_string(path)
}

/// Command names documented in a page's `.SH COMMANDS` section: the first
/// word of each `.B`/`.BR` tag line right after a `.TP`.
pub(crate) fn documented_commands(roff: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut in_commands = false;
    let mut after_tp = false;
    for line in roff.lines() {
        if let Some(title) = line.strip_prefix(".SH") {
            in_commands = title.trim().trim_matches('"') == "COMMANDS";
            after_tp = false;
            continue;
        }
        if in_commands && after_tp {
            let tag = line
                .strip_prefix(".B ")
                .or_else(|| line.strip_prefix(".BR "));
            if let Some(word) = tag.and_then(|t| t.split_whitespace().next()) {
                out.insert(word.trim_matches('"').replace("\\-", "-"));
            }
        }
        after_tp = line == ".TP";
    }
    out
}

/// The `.TH` line's quoted fields (date, source, manual).
pub(crate) fn title_fields(roff: &str) -> Vec<String> {
    let th = roff
        .lines()
        .find(|l| l.starts_with(".TH "))
        .unwrap_or_default();
    th.split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_string)
        .collect()
}

#[test]
fn documented_commands_reads_tagged_entries_only() {
    let roff = ".SH DESCRIPTION\n.TP\n.B ignored\n.SH COMMANDS\n.SS \"Group\"\n.TP\n\
                .B status\nPrint.\n.B not-a-tag\n.TP\n.BR client\\-tags \" \" \\fIMASK\\fR\n\
                .TP\n.B lock \" \" [\\fBcheck\\fR]\n.SH EXAMPLES\n.TP\n.B later\n";
    let got: Vec<_> = documented_commands(roff).into_iter().collect();
    assert_eq!(got, ["client-tags", "lock", "status"]);
}

#[test]
fn man_page_documents_every_subcommand() -> TestResult {
    let cli: BTreeSet<String> = Args::command()
        .get_subcommands()
        .filter(|c| !c.is_hide_set() && c.get_name() != "help")
        .map(|c| c.get_name().to_string())
        .collect();
    let man = documented_commands(&man_page("mctl.1")?);
    let missing: Vec<_> = cli.difference(&man).collect();
    let stale: Vec<_> = man.difference(&cli).collect();
    assert!(
        missing.is_empty() && stale.is_empty(),
        "man/mctl.1 COMMANDS drifted from `mctl --help`: undocumented {missing:?}, \
         no longer a subcommand {stale:?}"
    );
    Ok(())
}

#[test]
fn man_page_titles_carry_no_version() -> TestResult {
    for page in ["mctl.1", "margo.1"] {
        let fields = title_fields(&man_page(page)?);
        assert_eq!(
            fields.get(1).map(String::as_str),
            Some("margo"),
            "man/{page}: the .TH source field must be plain \"margo\" (no version), got {fields:?}"
        );
    }
    Ok(())
}
