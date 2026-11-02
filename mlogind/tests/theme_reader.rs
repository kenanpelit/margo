//! The theme-sync reader is a separate exec of the mlogind binary
//! (`mlogind __theme-sync-reader <home> <fd>…`), so the fork child of the
//! multi-threaded runner only makes syscalls before exec. This drives the real
//! binary through that entry point, minus the privilege drop (which needs
//! root): it must be dispatched before config parsing and fill the inherited
//! fds from the home directory.

use std::fs::{self, File, OpenOptions};
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::process::Command;

#[test]
fn the_reader_entry_point_fills_the_inherited_fds() {
    let base = std::env::temp_dir().join(format!("mlogind-reader-exec-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    let home = base.join("home");
    fs::create_dir_all(home.join(".cache/mshell")).unwrap();
    fs::write(home.join(".cache/mshell/last_theme.css"), "b { }").unwrap();

    let open = |name: &str| -> File {
        OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(base.join(name))
            .unwrap()
    };
    let files = [open("bg"), open("css"), open("avatar"), open("vars")];
    let fds: Vec<i32> = files.iter().map(|f| f.as_raw_fd()).collect();

    let mut cmd = Command::new(env!("CARGO_BIN_EXE_mlogind"));
    cmd.arg("__theme-sync-reader").arg(&home);
    for fd in &fds {
        cmd.arg(fd.to_string());
    }
    let inherit = fds.clone();
    // SAFETY: fcntl only, on descriptors this test owns.
    unsafe {
        cmd.pre_exec(move || {
            for &fd in &inherit {
                if libc::fcntl(fd, libc::F_SETFD, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
    let status = cmd.status().unwrap();
    assert!(status.success(), "reader exited {status}");

    assert_eq!(fs::read(base.join("css")).unwrap(), b"b { }");
    // Sources the home does not have leave their output empty.
    assert!(fs::read(base.join("bg")).unwrap().is_empty());
    assert!(fs::read(base.join("vars")).unwrap().is_empty());
    drop(files);
    fs::remove_dir_all(base).unwrap();
}

#[test]
fn the_reader_refuses_malformed_arguments() {
    let status = Command::new(env!("CARGO_BIN_EXE_mlogind"))
        .args(["__theme-sync-reader", "/nonexistent", "1"])
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(64));
}
