//! Value-level parser warnings (a malformed number falling back to 0, an
//! over-long keyword being clamped) must say which file and line they came
//! from — the parser runs at startup and on every `mctl reload`, and a bare
//! "`12px` is not a valid number" in the log is unfindable in a config tree
//! of `source`d fragments.

use std::io::Write as _;
use std::sync::{Arc, Mutex};

use tracing_subscriber::fmt::MakeWriter;

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Capture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Capture {
    type Writer = Capture;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// Parse `content` as if it lived at `path` and return everything logged.
fn parse_and_capture(content: &str, path: &std::path::Path) -> String {
    let capture = Capture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(capture.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::WARN)
        .finish();
    tracing::subscriber::with_default(subscriber, || {
        margo_config::parse_config_str(content, Some(path)).unwrap();
    });
    let bytes = capture.0.lock().unwrap().clone();
    String::from_utf8(bytes).unwrap()
}

#[test]
fn bad_number_warning_names_file_and_line() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.conf");
    std::fs::File::create(&path)
        .unwrap()
        .write_all(b"")
        .unwrap();
    let log = parse_and_capture("# header\n\nborderpx = 12px\n", &path);
    let warning = log
        .lines()
        .find(|l| l.contains("is not a valid number"))
        .unwrap_or_else(|| panic!("no warning logged:\n{log}"));
    assert!(warning.contains("config.conf"), "no file in: {warning}");
    assert!(warning.contains("line=3"), "no line in: {warning}");
}

#[test]
fn clamped_keyword_warning_names_file_and_line() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("anim.conf");
    std::fs::File::create(&path)
        .unwrap()
        .write_all(b"")
        .unwrap();
    let log = parse_and_capture("animation_type_open = zoomzoomzoomzoom\n", &path);
    let warning = log
        .lines()
        .find(|l| l.contains("char limit"))
        .unwrap_or_else(|| panic!("no warning logged:\n{log}"));
    assert!(warning.contains("anim.conf"), "no file in: {warning}");
    assert!(warning.contains("line=1"), "no line in: {warning}");
}
