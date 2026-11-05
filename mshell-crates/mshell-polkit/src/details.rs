//! What the auth dialog shows besides polkit's message: the action id and
//! the command root will run.
//!
//! The message alone does not say what happens: for pkexec it reads "run
//! `/bin/sh' as the super user", whatever script follows. Any process of
//! this user can call `pkexec sh -c '<payload>'` and get a dialog that looks
//! like the shell's own requests, so the dialog shows `details`'
//! `command_line` (or `program`) and the action id.
//!
//! The text comes from the requesting process, so it is made inert before
//! it reaches the label: control characters and invisible/bidi format
//! characters are shown as visible escapes (a line break or a right-to-left
//! override cannot hide or reorder part of the command), and a very long
//! command is cut with an explicit count of what is not shown, so padding
//! cannot push the real payload out of view unnoticed.

use std::collections::HashMap;

/// Longest command shown, in characters. A real pkexec command line fits
/// easily; past this the dialog says how much is hidden.
const MAX_COMMAND_CHARS: usize = 600;

/// The details block for the dialog, one `Label: value` line each. Empty
/// when there is nothing worth showing.
pub(crate) fn dialog_details(action_id: &str, details: &HashMap<String, String>) -> String {
    let mut lines = Vec::new();
    let command = details
        .get("command_line")
        .filter(|c| !c.trim().is_empty())
        .map(|c| ("Command", c))
        .or_else(|| {
            details
                .get("program")
                .filter(|p| !p.trim().is_empty())
                .map(|p| ("Program", p))
        });
    if let Some((label, value)) = command {
        lines.push(format!("{label}: {}", inert(value, MAX_COMMAND_CHARS)));
    }
    if !action_id.is_empty() {
        lines.push(format!("Action: {}", inert(action_id, 200)));
    }
    lines.join("\n")
}

/// `s` as visible, single-line text of at most `max` characters (plus the
/// "not shown" note).
fn inert(s: &str, max: usize) -> String {
    let mut out = String::new();
    let total = s.chars().count();
    for (shown, c) in s.chars().enumerate() {
        if shown == max {
            out.push_str(&format!(" … ({} more characters not shown)", total - max));
            return out;
        }
        match c {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() || is_invisible_format(c) => {
                out.push_str(&format!("\\u{{{:x}}}", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}

/// Format characters that render as nothing or reorder text: zero-width
/// spaces/joiners, bidi marks/embeddings/overrides/isolates, word joiner
/// and invisible operators, the BOM.
fn is_invisible_format(c: char) -> bool {
    matches!(
        c,
        '\u{00ad}'
            | '\u{061c}'
            | '\u{180e}'
            | '\u{200b}'..='\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2060}'..='\u{2069}'
            | '\u{feff}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn details(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn shows_the_command_line_and_the_action() {
        let d = details(&[
            ("program", "/bin/sh"),
            ("command_line", "/bin/sh -c 'cp /etc/x.pacnew /etc/x'"),
        ]);
        assert_eq!(
            dialog_details("org.freedesktop.policykit.exec", &d),
            "Command: /bin/sh -c 'cp /etc/x.pacnew /etc/x'\nAction: org.freedesktop.policykit.exec"
        );
    }

    #[test]
    fn falls_back_to_the_program() {
        let d = details(&[("program", "/usr/bin/ufw"), ("command_line", "  ")]);
        assert_eq!(
            dialog_details("a.b", &d),
            "Program: /usr/bin/ufw\nAction: a.b"
        );
    }

    #[test]
    fn non_pkexec_actions_show_the_action_only() {
        assert_eq!(
            dialog_details("org.freedesktop.login1.reboot", &HashMap::new()),
            "Action: org.freedesktop.login1.reboot"
        );
        assert_eq!(dialog_details("", &HashMap::new()), "");
    }

    #[test]
    fn line_breaks_and_bidi_tricks_are_shown_not_obeyed() {
        let d = details(&[(
            "command_line",
            "/bin/sh -c 'true\n\n\nrm -rf ~' \u{202e}txt.exe\u{200b}\x1b[2J",
        )]);
        let text = dialog_details("x", &d);
        let command = text.lines().next().unwrap();
        assert_eq!(
            command,
            "Command: /bin/sh -c 'true\\n\\n\\nrm -rf ~' \\u{202e}txt.exe\\u{200b}\\u{1b}[2J"
        );
        assert_eq!(text.lines().count(), 2, "{text}");
    }

    #[test]
    fn a_padded_command_says_how_much_is_hidden() {
        let padded = format!("/bin/true{}; evil", " ".repeat(2000));
        let text = dialog_details("x", &details(&[("command_line", &padded)]));
        let total = padded.chars().count();
        assert!(
            text.contains(&format!(
                "({} more characters not shown)",
                total - MAX_COMMAND_CHARS
            )),
            "{text}"
        );
        // Exactly at the limit nothing is cut.
        let exact = "a".repeat(MAX_COMMAND_CHARS);
        let text = dialog_details("", &details(&[("command_line", &exact)]));
        assert_eq!(text, format!("Command: {exact}"));
    }
}
