//! Drift gate for `man/mshellctl.1`.
//!
//! The page keeps its prose, grouping and examples by hand, but its
//! COMMANDS section must name exactly the subcommands clap exposes: a new
//! subcommand fails here until it is documented, a removed one until its
//! entry goes. The `.TH` source field carries no version number — a
//! release only bumps Cargo versions, so a version there goes stale on the
//! next release (it read "margo 0.9.1" for months).

use std::collections::BTreeSet;
use std::path::PathBuf;

use clap::CommandFactory;
use mshellctl::app::Cli;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn man_page() -> std::io::Result<String> {
    std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../man/mshellctl.1"))
}

/// Command names documented in the `.SH COMMANDS` section: the first word
/// of each `.B`/`.BR` tag line right after a `.TP`.
fn documented_commands(roff: &str) -> BTreeSet<String> {
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

#[test]
fn man_page_documents_every_subcommand() -> TestResult {
    let cli: BTreeSet<String> = Cli::command()
        .get_subcommands()
        .filter(|c| !c.is_hide_set() && c.get_name() != "help")
        .map(|c| c.get_name().to_string())
        .collect();
    let man = documented_commands(&man_page()?);
    let missing: Vec<_> = cli.difference(&man).collect();
    let stale: Vec<_> = man.difference(&cli).collect();
    assert!(
        missing.is_empty() && stale.is_empty(),
        "man/mshellctl.1 COMMANDS drifted from `mshellctl --help`: undocumented {missing:?}, \
         no longer a subcommand {stale:?}"
    );
    Ok(())
}

/// The grouped overview in `mshellctl --help` is a hand-kept template
/// (clap has no subcommand groups), so hold it to the enum too.
#[test]
fn help_overview_lists_every_subcommand() {
    let mut cmd = Cli::command();
    let help = cmd.render_help().to_string();
    let listed: BTreeSet<&str> = help
        .lines()
        .filter_map(|l| l.strip_prefix("  "))
        .filter_map(|l| l.split_whitespace().next())
        .collect();
    let missing: Vec<String> = cmd
        .get_subcommands()
        .filter(|c| !c.is_hide_set() && c.get_name() != "help")
        .map(|c| c.get_name().to_string())
        .filter(|name| !listed.contains(name.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "mshellctl --help overview (app.rs help_template) misses {missing:?}"
    );
}

#[test]
fn man_page_title_carries_no_version() -> TestResult {
    let page = man_page()?;
    let th = page
        .lines()
        .find(|l| l.starts_with(".TH "))
        .unwrap_or_default();
    let fields: Vec<&str> = th.split('"').skip(1).step_by(2).collect();
    assert_eq!(
        fields.get(1).copied(),
        Some("margo"),
        "man/mshellctl.1: the .TH source field must be plain \"margo\" (no version), got {fields:?}"
    );
    Ok(())
}
