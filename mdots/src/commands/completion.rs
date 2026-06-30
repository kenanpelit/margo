//! `dcli completion <shell>` — generate shell completion scripts.
//!
//! Generated from the live clap command definition, so completions never
//! drift out of sync with the actual CLI surface. Supports every shell
//! `clap_complete` knows about (bash, zsh, fish, elvish, powershell).

use clap::CommandFactory;
use clap_complete::{generate, Shell};
use std::io::Write;

/// Write the completion script for `shell` to `out`.
pub fn write_completion<W: Write>(shell: Shell, out: &mut W) {
    let mut cmd = crate::Cli::command();
    generate(shell, &mut cmd, "dcli", out);
}

/// CLI entry point: print the completion script for `shell` to stdout.
pub fn run(shell: Shell) {
    write_completion(shell, &mut std::io::stdout());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generates_nonempty_completion_naming_binary_and_subcommands() {
        let mut buf = Vec::new();
        write_completion(Shell::Zsh, &mut buf);
        let script = String::from_utf8(buf).expect("completion output is valid UTF-8");

        assert!(!script.is_empty(), "completion script must not be empty");
        assert!(
            script.contains("dcli"),
            "completion must reference the binary name"
        );
        // A real completion enumerates the actual subcommands, proving it was
        // generated from the live clap definition rather than a stale stub.
        assert!(
            script.contains("sync") && script.contains("module"),
            "completion must include known subcommands"
        );
    }
}
