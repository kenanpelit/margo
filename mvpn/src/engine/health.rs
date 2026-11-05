//! Gather the facts behind `mvpn health` (see `mvpn_proto::health` for
//! the diagnosis): the service, the tunnel's error cause, lockdown, the
//! account, and whether the running kernel's modules are still there.

use std::path::Path;

pub use mvpn_proto::health::{Daemon, Health, diagnose};

use super::{account, realloc, service, sys, tunnel};

/// `mullvad` output that means the CLI could not reach the daemon.
pub fn is_daemon_down(text: &str) -> bool {
    text.contains("Management RPC server or client error")
        || text.contains("Failed to connect to mullvad-daemon")
}

/// Whether `/usr/lib/modules/<release>` exists for the running kernel
/// (`/proc/sys/kernel/osrelease`, the same string `uname -r` prints).
/// `None` when the release cannot be read.
pub fn kernel_modules_present_in(modules_root: &Path, osrelease: &str) -> Option<bool> {
    let release = osrelease.trim();
    if release.is_empty() || release.contains('/') || release.starts_with('.') {
        return None;
    }
    Some(modules_root.join(release).is_dir())
}

pub fn kernel_modules_present() -> Option<bool> {
    let release = std::fs::read_to_string("/proc/sys/kernel/osrelease").ok()?;
    kernel_modules_present_in(Path::new("/usr/lib/modules"), &release)
}

/// Gather everything. Also records the real location when the tunnel is
/// down (see `realloc`), so "My country" stays current.
pub fn gather() -> Health {
    let status = sys::run("mullvad", &["status", "--json"], None);
    if !status.spawned {
        return Health {
            daemon: Daemon::Missing,
            ..Health::default()
        };
    }
    if !status.success && is_daemon_down(&status.text()) {
        return Health {
            daemon: Daemon::Down,
            boot_enabled: service::boot_enabled(),
            ..Health::default()
        };
    }
    let t = tunnel::parse(&status.stdout).unwrap_or_default();
    if let Some(real) = realloc::RealLocation::from_tunnel(&t) {
        realloc::save(&real);
    }
    let firewall = t.error_reason == "set_firewall_policy_error";
    Health {
        daemon: Daemon::Running,
        boot_enabled: service::boot_enabled(),
        locked_down: t.locked_down,
        kernel_modules_present: if firewall {
            kernel_modules_present()
        } else {
            None
        },
        account: account::query(),
        error_reason: t.error_reason,
        error_detail: t.error_detail,
        state: t.state,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// mullvad-cli 2026.5 with the daemon stopped (its own error text).
    const DOWN: &str = include_str!("../../tests/fixtures/synthetic-daemon-down.txt");

    #[test]
    fn recognises_a_stopped_daemon() {
        assert!(is_daemon_down(DOWN));
        assert!(!is_daemon_down("Disconnected"));
    }

    #[test]
    fn finds_the_running_kernels_modules() {
        let root = std::env::temp_dir().join(format!("mvpn-modules-{}", std::process::id()));
        std::fs::create_dir_all(root.join("7.3.1-1-meteor-lto")).expect("dir");
        assert_eq!(
            kernel_modules_present_in(&root, "7.3.1-1-meteor-lto\n"),
            Some(true)
        );
        // Upgraded without a reboot: the running release's dir is gone.
        assert_eq!(
            kernel_modules_present_in(&root, "7.3.0-2-cachyos-bore-lto"),
            Some(false)
        );
        assert_eq!(kernel_modules_present_in(&root, ""), None);
        assert_eq!(kernel_modules_present_in(&root, "../etc"), None);
        let _ = std::fs::remove_dir_all(root);
    }
}
