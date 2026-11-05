//! Shared input preparation for the margo-config fuzz targets.

/// `text` with every `source = …` / `include = …` line blanked out (the
/// line break stays, so line numbers don't move).
///
/// Those directives read other files from disk (an absolute path like
/// `/dev/zero` would just read forever), which makes a run depend on the
/// machine instead of the input. Only lines whose key is exactly `source` or
/// `include` go — the key as the parser and the validator see it: the text
/// before the line's first `=`, trimmed. Every other line stays, including
/// ones that merely contain the words (`resource`, `sources`, a value
/// mentioning `include`), so they are still fuzzed. The include path itself
/// is covered by margo-config's unit tests.
pub fn strip_file_directives(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let body = line.strip_suffix('\n').unwrap_or(line);
        let is_directive = body
            .split_once('=')
            .is_some_and(|(key, _)| matches!(key.trim(), "source" | "include"));
        if is_directive {
            if line.ends_with('\n') {
                out.push('\n');
            }
        } else {
            out.push_str(line);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::strip_file_directives;

    #[test]
    fn drops_source_and_include_lines_only() {
        let text = "source = /dev/zero\n  include=~/x.conf\nborderpx = 2\n";
        assert_eq!(strip_file_directives(text), "\n\nborderpx = 2\n");
    }

    #[test]
    fn keeps_lines_that_only_contain_the_words() {
        let text =
            "resource = 1\nsources = 2\nexec = include source\nbind = SUPER,s,spawn,source\n";
        assert_eq!(strip_file_directives(text), text);
    }

    #[test]
    fn handles_crlf_tabs_and_a_missing_final_newline() {
        assert_eq!(strip_file_directives("\tsource\t= a\r\nx = 1"), "\nx = 1");
        assert_eq!(strip_file_directives("x = 1\ninclude = b"), "x = 1\n");
    }

    #[test]
    fn keeps_a_directive_word_without_an_equals_sign() {
        let text = "source\ninclude ~/x.conf\n";
        assert_eq!(strip_file_directives(text), text);
    }
}
