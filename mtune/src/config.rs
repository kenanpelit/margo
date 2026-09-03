// SPDX-License-Identifier: GPL-3.0-or-later

pub static VERSION: &str = env!("CARGO_PKG_VERSION");
pub static GETTEXT_PACKAGE: &str = "mtune";
pub static APPLICATION_ID: &str = "org.margo.Tune";
pub static PROFILE: &str = "";

/// Localedir: the system dir in a packaged build, the source `po/` tree in dev.
pub fn localedir() -> String {
    if cfg!(debug_assertions) {
        concat!(env!("CARGO_MANIFEST_DIR"), "/po").to_string()
    } else {
        "/usr/share/locale".to_string()
    }
}
