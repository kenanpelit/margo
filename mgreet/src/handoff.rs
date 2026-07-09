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
    let buf = build_payload(user, session, password);
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(buf.as_bytes())?;
    file.flush()
}

/// Assemble `LOGIN\n<user>\n<session>\n<password>` in a zeroizing buffer so the
/// plaintext is scrubbed from our memory once written, matching how the
/// orchestrator shreds the file. The password is the FINAL field so any byte in
/// it (including a newline) can't shift `read_and_shred_greet_result`'s parse.
/// Split from [`write`] so the security-critical layout is testable without
/// touching the /run/mlogind tmpfs.
fn build_payload(user: &str, session: &str, password: &str) -> zeroize::Zeroizing<String> {
    let mut buf = zeroize::Zeroizing::new(String::with_capacity(
        "LOGIN\n".len() + user.len() + session.len() + password.len() + 3,
    ));
    buf.push_str("LOGIN\n");
    buf.push_str(user);
    buf.push('\n');
    buf.push_str(session);
    buf.push('\n');
    buf.push_str(password);
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_layout_puts_the_password_last() {
        let payload = build_payload("alice", "GNOME", "test-password");
        assert_eq!(payload.as_str(), "LOGIN\nalice\nGNOME\ntest-password");
    }

    #[test]
    fn newline_in_password_cannot_shift_the_parse() {
        // With the password as the final field, an embedded newline stays part of
        // it — the verb/user/session fields ahead of it are untouched.
        let payload = build_payload("alice", "GNOME", "line1\nline2");
        let mut lines = payload.lines();
        assert_eq!(lines.next(), Some("LOGIN"));
        assert_eq!(lines.next(), Some("alice"));
        assert_eq!(lines.next(), Some("GNOME"));
        let header = "LOGIN\nalice\nGNOME\n";
        assert_eq!(&payload[header.len()..], "line1\nline2");
    }

    #[test]
    fn empty_fields_still_produce_a_valid_frame() {
        let payload = build_payload("", "", "");
        assert_eq!(payload.as_str(), "LOGIN\n\n\n");
    }
}
