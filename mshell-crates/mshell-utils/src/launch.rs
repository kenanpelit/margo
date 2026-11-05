use gio_unix::DesktopAppInfo;
use relm4::gtk::prelude::AppInfoExt;
use std::os::unix::prelude::CommandExt;
use std::process::{Command, Stdio};

/// Launch `app` in its own process group, so it outlives an mshell restart.
/// `gtk4-launch` ships with GTK 4 (a hard dependency); GTK 3's `gtk-launch`
/// is not guaranteed to be installed, and a missing launcher fails silently.
pub fn launch_detached(app: &DesktopAppInfo) {
    if let Some(id) = app.id() {
        let _ = Command::new("gtk4-launch")
            .arg(id.as_str())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .map(|mut child| child.wait());
    }
}
