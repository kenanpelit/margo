//! Small helpers shared by providers that wrap optional external tools.
//!
//! Optional tools (`fd`, `plocate`, `qalc`, `pass-otp`, a Chromium
//! browser …) are detected at runtime so a missing package degrades to a
//! hint row instead of a broken action.

use std::path::Path;

/// Single-quote `s` for `sh -c`, escaping embedded single quotes.
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Replace a leading `$HOME` with `~` for display.
pub fn tildify(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_and_tilde() {
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
        let home = Path::new("/home/u");
        assert_eq!(tildify(Path::new("/home/u/a/b"), home), "~/a/b");
        assert_eq!(tildify(Path::new("/home/u"), home), "~");
        assert_eq!(tildify(Path::new("/etc/x"), home), "/etc/x");
    }
}
