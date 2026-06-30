//! Consistent cargo-style status output for `mdots sync` and friends.
//!
//! A status line is a right-aligned, bold, colored verb followed by a message:
//!
//! ```text
//!    Installing  neovim, ripgrep, fd
//!      Finished  sync · 3 installed, 1 removed · 4.2s
//! ```
//!
//! Verbs are right-aligned to [`VERB_WIDTH`] so messages line up in a column.

use colored::*;

/// Column width the status verb is right-aligned to (matches cargo's 12).
pub(crate) const VERB_WIDTH: usize = 12;

/// Inded of the message column (`VERB_WIDTH` + the two-space gap), used to
/// align continuation/detail lines under a status message.
pub(crate) const DETAIL_INDENT: usize = VERB_WIDTH + 2;

/// Right-align a verb to [`VERB_WIDTH`] using its *plain* length, so coloring
/// the result afterwards never throws the alignment off.
pub(crate) fn align_verb(verb: &str) -> String {
    let pad = VERB_WIDTH.saturating_sub(verb.chars().count());
    format!("{}{}", " ".repeat(pad), verb)
}

fn emit(verb: ColoredString, msg: &str) {
    println!("{}  {}", verb, msg);
}

/// A completed/normal step: bold green verb.
pub(crate) fn step(verb: &str, msg: &str) {
    emit(align_verb(verb).green().bold(), msg);
}

/// A step that needs attention: bold yellow verb.
pub(crate) fn warn(verb: &str, msg: &str) {
    emit(align_verb(verb).yellow().bold(), msg);
}

/// A failed step: bold red verb.
pub(crate) fn error(verb: &str, msg: &str) {
    emit(align_verb(verb).red().bold(), msg);
}

/// A low-emphasis / informational step: bold cyan verb.
pub(crate) fn note(verb: &str, msg: &str) {
    emit(align_verb(verb).cyan().bold(), msg);
}

/// A continuation/detail line aligned under the message column.
pub(crate) fn detail(msg: &str) {
    println!("{}{}", " ".repeat(DETAIL_INDENT), msg.dimmed());
}

/// Human-friendly elapsed time for the `Finished` line: `"850ms"`, `"4.2s"`,
/// or `"1m03s"`.
pub(crate) fn format_elapsed(d: std::time::Duration) -> String {
    let secs = d.as_secs_f64();
    if secs < 1.0 {
        format!("{}ms", d.as_millis())
    } else if secs < 60.0 {
        format!("{:.1}s", secs)
    } else {
        let mins = d.as_secs() / 60;
        let rem = d.as_secs() % 60;
        format!("{}m{:02}s", mins, rem)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn align_verb_right_pads_shorter_verbs() {
        // "Installing" is 10 chars → 2 leading spaces to reach width 12.
        assert_eq!(align_verb("Installing"), "  Installing");
        // "Finished" is 8 chars → 4 leading spaces.
        assert_eq!(align_verb("Finished"), "    Finished");
    }

    #[test]
    fn align_verb_result_has_exact_width() {
        for verb in ["Validating", "Resolving", "Removing", "Syncing", "Linking"] {
            assert_eq!(
                align_verb(verb).chars().count(),
                VERB_WIDTH,
                "verb {:?} should pad to exactly VERB_WIDTH",
                verb
            );
        }
    }

    #[test]
    fn align_verb_does_not_truncate_overlong_verbs() {
        // A verb at/over the width is left intact (saturating_sub → 0 pad).
        assert_eq!(align_verb("Decrypting…x"), "Decrypting…x".to_string());
        let long = "Reconfiguring";
        assert_eq!(align_verb(long), long, "overlong verbs must not be cut");
    }

    #[test]
    fn detail_indent_aligns_under_message_column() {
        assert_eq!(DETAIL_INDENT, VERB_WIDTH + 2);
    }

    #[test]
    fn format_elapsed_uses_appropriate_units() {
        use std::time::Duration;
        assert_eq!(format_elapsed(Duration::from_millis(850)), "850ms");
        assert_eq!(format_elapsed(Duration::from_millis(4200)), "4.2s");
        assert_eq!(format_elapsed(Duration::from_secs(63)), "1m03s");
    }
}
