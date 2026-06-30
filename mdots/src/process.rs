//! Small process-execution helpers shared across commands.

use std::process::{Command, ExitStatus, Stdio};

/// Run `cmd` with the terminal inherited (stdin/stdout/stderr passed straight
/// through) and return its exit status. Centralizes the interactive-launch
/// boilerplate that the editor, hook and sudo-script call sites otherwise
/// repeat. The caller decides how to interpret the status.
pub fn status_inherited(cmd: &mut Command) -> std::io::Result<ExitStatus> {
    cmd.stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
}
