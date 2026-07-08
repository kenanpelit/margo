//! The credential hand-off the mlogind orchestrator consumes.
//!
//! On a validated login the greeter writes `$MLOGIND_RESULT_PATH` in the exact
//! shape `read_and_shred_greet_result` parses — `LOGIN\n<user>\n<session>\n
//! <password>` — then quits so the greeter compositor exits and the orchestrator
//! launches the session. The password is the final field so any byte in it
//! (including a newline) can't shift the parse. The orchestrator overwrites +
//! unlinks the file after reading; it lives only on the 0700 root tmpfs at
//! /run/mlogind, so the plaintext never touches disk.

use std::fs::OpenOptions;
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

/// Write the `LOGIN` hand-off for `user`/`session`/`password` to `path`, 0600.
pub fn write(path: &Path, user: &str, session: &str, password: &str) -> io::Result<()> {
    // Assemble in a zeroizing buffer so the plaintext is scrubbed from our own
    // memory once written, matching how the orchestrator shreds the file.
    let mut buf = zeroize::Zeroizing::new(String::with_capacity(
        "LOGIN\n".len() + user.len() + session.len() + password.len() + 3,
    ));
    buf.push_str("LOGIN\n");
    buf.push_str(user);
    buf.push('\n');
    buf.push_str(session);
    buf.push('\n');
    buf.push_str(password);

    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(buf.as_bytes())?;
    file.flush()
}
