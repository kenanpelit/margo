//! `mdots man` — render the man page.
//!
//! Generated from the live clap command definition (via clap_mangen), so the
//! manual never drifts out of sync with the actual CLI.

use anyhow::Result;
use clap::CommandFactory;
use std::io::Write;

/// Write the roff-formatted man page to `out`.
///
/// The page carries no version (the title line names the project and the
/// VERSION section is left out): `man/mdots.1` is this output checked in,
/// and a release bumps only the Cargo version, so a versioned page would be
/// stale after every release. `mdots --version` reports it.
pub fn write_man<W: Write>(out: &mut W) -> Result<()> {
    let cmd = crate::Cli::command();
    let mut roff = Vec::new();
    clap_mangen::Man::new(cmd)
        .source("margo")
        .manual("Margo Manual")
        .render(&mut roff)?;
    out.write_all(without_section(&String::from_utf8(roff)?, "VERSION").as_bytes())?;
    Ok(())
}

/// `roff` with its `.SH <name>` section (up to the next `.SH`) removed.
fn without_section(roff: &str, name: &str) -> String {
    let heading = format!(".SH {name}");
    let mut out = String::with_capacity(roff.len());
    let mut skipping = false;
    for line in roff.split_inclusive('\n') {
        if line.starts_with(".SH ") {
            skipping = line.trim_end() == heading;
        }
        if !skipping {
            out.push_str(line);
        }
    }
    out
}

/// CLI entry point: print the man page (roff) to stdout.
pub fn run() -> Result<()> {
    write_man(&mut std::io::stdout())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_renders_man_page_naming_binary() {
        let mut buf = Vec::new();
        write_man(&mut buf).expect("man page should render");
        let roff = String::from_utf8(buf).expect("man output is valid UTF-8");

        assert!(!roff.is_empty(), "man page must not be empty");
        // roff man pages start with a .TH (title heading) macro naming the page.
        assert!(roff.contains(".TH"), "should contain a roff title heading");
        assert!(roff.contains("mdots"), "should reference the binary name");
    }

    #[test]
    fn test_without_section_drops_only_that_section() {
        let roff = ".TH x\n.SH OPTIONS\na\n.SH VERSION\nv1\n.SH AUTHORS\nme\n";
        assert_eq!(
            without_section(roff, "VERSION"),
            ".TH x\n.SH OPTIONS\na\n.SH AUTHORS\nme\n"
        );
        assert_eq!(without_section(".SH VERSION\nv1\n", "VERSION"), "");
    }

    /// `man/mdots.1` is this command's output, checked in for packagers
    /// (the PKGBUILD installs `man/*.1`). Regenerate it with
    /// `./contrib/completions/generate.sh` (or `just completions`).
    #[test]
    fn test_checked_in_man_page_is_current() -> Result<()> {
        let mut buf = Vec::new();
        write_man(&mut buf)?;
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../man/mdots.1");
        let checked_in = std::fs::read(&path)?;
        assert!(
            checked_in == buf,
            "man/mdots.1 is stale: run ./contrib/completions/generate.sh and commit it"
        );
        Ok(())
    }

    /// The page names no version: a release bumps only the Cargo version,
    /// so a versioned page would drift on every release.
    #[test]
    fn test_page_carries_no_version() -> Result<()> {
        let mut buf = Vec::new();
        write_man(&mut buf)?;
        let roff = String::from_utf8(buf)?;
        assert!(
            !roff.contains(env!("CARGO_PKG_VERSION")) && !roff.contains(".SH VERSION"),
            "the man page must not carry the version"
        );
        assert!(roff.contains(".SH SUBCOMMANDS") && roff.contains(".SH OPTIONS"));
        Ok(())
    }
}
