//! `mdots man` — render the man page.
//!
//! Generated from the live clap command definition (via clap_mangen), so the
//! manual never drifts out of sync with the actual CLI.

use anyhow::Result;
use clap::CommandFactory;
use std::io::Write;

/// Write the roff-formatted man page to `out`.
pub fn write_man<W: Write>(out: &mut W) -> Result<()> {
    let cmd = crate::Cli::command();
    clap_mangen::Man::new(cmd).render(out)?;
    Ok(())
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
}
