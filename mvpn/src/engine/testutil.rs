//! Test-only helpers: a fake `mullvad` binary, so engine code that shells
//! out can be exercised without touching the real daemon.

use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

/// Fakes are serialised: a script being written while another test forks
/// leaks its write fd into that child until it execs, and running the
/// script then fails with ETXTBSY ("text file busy").
static SERIAL: Mutex<()> = Mutex::new(());

/// A throwaway fake program in its own temp dir. It records its argv
/// (`argv`, one word a line) and stdin (`stdin`), prints the given
/// stdout / stderr, and exits with `code`.
pub(crate) struct Fake {
    pub dir: PathBuf,
    pub bin: PathBuf,
    _serial: MutexGuard<'static, ()>,
}

impl Fake {
    pub fn new(tag: &str, code: i32, stdout: &str, stderr: &str) -> Self {
        let serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("mvpn-fake-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("fake dir");
        std::fs::write(dir.join("out"), stdout).expect("out");
        std::fs::write(dir.join("err"), stderr).expect("err");
        let bin = dir.join("mullvad");
        let script = format!(
            "#!/bin/sh\nd='{d}'\nprintf '%s\\n' \"$@\" > \"$d/argv\"\ncat > \"$d/stdin\"\ncat \"$d/out\"\ncat \"$d/err\" >&2\nexit {code}\n",
            d = dir.display()
        );
        std::fs::write(&bin, script).expect("fake script");
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        Self {
            dir,
            bin,
            _serial: serial,
        }
    }

    pub fn program(&self) -> &str {
        self.bin.to_str().expect("utf8 path")
    }

    pub fn argv(&self) -> String {
        std::fs::read_to_string(self.dir.join("argv")).unwrap_or_default()
    }

    pub fn stdin(&self) -> String {
        std::fs::read_to_string(self.dir.join("stdin")).unwrap_or_default()
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
