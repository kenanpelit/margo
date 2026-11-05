//! Host services for the Podman menu's **Host** tab: the user's own
//! listening TCP services next to the containers, one row per process,
//! each port a chip that opens in the browser. Pure; fed by the CPU
//! dashboard's `ss -tulpnH` parser and `/proc/<pid>/cgroup`.
//!
//! Only TCP is listed (only TCP serves a web UI). Skipped: sockets whose
//! owner we cannot see (another user's), binds a browser cannot reach
//! (link-local / multicast, see `mshell_utils::local_url`), and the
//! container runtime's port forwarders — their ports already show on the
//! container rows.

use crate::menus::menu_widgets::cpu_dashboard::ports::PortRow;

/// Processes that only forward a port a container publishes.
const RUNTIME_HELPERS: &[&str] = &[
    "conmon",
    "rootlessport",
    "rootlessport-child",
    "pasta",
    "pasta.avx2",
    "slirp4netns",
    "docker-proxy",
    "containerd-shim",
    "containerd-shim-runc-v2",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HostPort {
    pub port: u16,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HostService {
    /// The systemd unit (`.service` dropped) when the process runs in one,
    /// else the process name.
    pub name: String,
    /// `transmission.service · pid 1790` / `pid 1723`.
    pub detail: String,
    pub pid: i32,
    /// Distinct ports, ascending (a service bound on `0.0.0.0` and `[::]`
    /// is one chip).
    pub ports: Vec<HostPort>,
}

/// The unit a process runs in, from `/proc/<pid>/cgroup` (cgroup v2 line
/// `0::/…/app.slice/mpd.service`). `None` for a scope (a terminal-spawned
/// process) or anything that is not a `.service`.
pub(crate) fn cgroup_unit(cgroup: &str) -> Option<String> {
    let path = cgroup.lines().find_map(|l| l.strip_prefix("0::"))?;
    let leaf = path.trim().rsplit('/').next()?;
    // `user@1000.service` is the user manager itself, not the app.
    (leaf.ends_with(".service") && !leaf.starts_with("user@")).then(|| leaf.to_string())
}

/// Group the TCP listeners by owning process. `unit_of(pid)` resolves the
/// systemd unit (reads `/proc` in production, a table in tests). Sorted by
/// name, then pid.
pub(crate) fn host_services(
    rows: &[PortRow],
    unit_of: impl Fn(i32) -> Option<String>,
) -> Vec<HostService> {
    let mut out: Vec<HostService> = Vec::new();
    for r in rows.iter().filter(|r| r.proto == "tcp") {
        let bind = match &r.iface {
            Some(iface) => format!("{}%{iface}", r.addr),
            None => r.addr.clone(),
        };
        let Some(url) = mshell_utils::local_url::url(&bind, r.port) else {
            continue;
        };
        for owner in &r.owners {
            if RUNTIME_HELPERS.contains(&owner.name.as_str()) {
                continue;
            }
            let idx = match out.iter().position(|s| s.pid == owner.pid) {
                Some(i) => i,
                None => {
                    let (name, detail) = match unit_of(owner.pid) {
                        Some(u) => (
                            u.trim_end_matches(".service").to_string(),
                            format!("{u} · pid {}", owner.pid),
                        ),
                        None => (owner.name.clone(), format!("pid {}", owner.pid)),
                    };
                    out.push(HostService {
                        name,
                        detail,
                        pid: owner.pid,
                        ports: Vec::new(),
                    });
                    out.len() - 1
                }
            };
            let svc = &mut out[idx];
            if !svc.ports.iter().any(|p| p.port == r.port) {
                svc.ports.push(HostPort {
                    port: r.port,
                    url: url.clone(),
                });
            }
        }
    }
    for s in &mut out {
        s.ports.sort_by_key(|p| p.port);
    }
    out.sort_by(|a, b| a.name.cmp(&b.name).then(a.pid.cmp(&b.pid)));
    out
}

/// Production `unit_of`: read `/proc/<pid>/cgroup`.
pub(crate) fn proc_unit(pid: i32) -> Option<String> {
    std::fs::read_to_string(format!("/proc/{pid}/cgroup"))
        .ok()
        .and_then(|c| cgroup_unit(&c))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menus::menu_widgets::cpu_dashboard::ports::parse_ss;

    // `/proc/<pid>/cgroup` captured on the owner's machine.
    const MPD_CGROUP: &str = "1:net_cls:/\n\
0::/user.slice/user-1000.slice/user@1000.service/app.slice/mpd.service\n";
    const TMUX_CGROUP: &str = "1:net_cls:/\n\
0::/user.slice/user-1000.slice/user@1000.service/app.slice/tmux-spawn-39fdba83-95cf-405f-9ad3-263e73620022.scope\n";

    #[test]
    fn unit_comes_from_the_cgroup_leaf() {
        assert_eq!(cgroup_unit(MPD_CGROUP).as_deref(), Some("mpd.service"));
        assert_eq!(cgroup_unit(TMUX_CGROUP), None);
        assert_eq!(
            cgroup_unit("0::/user.slice/user-1000.slice/user@1000.service\n"),
            None
        );
        assert_eq!(cgroup_unit(""), None);
    }

    #[test]
    fn one_row_per_process_from_real_ss_output() {
        let rows = parse_ss(include_str!("../cpu_dashboard/testdata/ss_tulpn.txt"));
        let services = host_services(&rows, |pid| {
            (pid == 1790).then(|| "transmission.service".to_string())
        });
        // transmission: 51413 on 0.0.0.0 and [::] collapse, 9091 added.
        let t = services
            .iter()
            .find(|s| s.name == "transmission")
            .expect("transmission row");
        assert_eq!(t.detail, "transmission.service · pid 1790");
        let ports: Vec<u16> = t.ports.iter().map(|p| p.port).collect();
        assert_eq!(ports, vec![9091, 51413]);
        assert_eq!(t.ports[0].url, "http://localhost:9091");
        // UDP-only owners and hidden-owner sockets (resolved :53) are absent.
        assert!(
            services
                .iter()
                .all(|s| s.ports.iter().all(|p| p.port != 53))
        );
        assert!(services.iter().all(|s| s.pid > 0));
        // Without a unit the process name names the row.
        assert!(services.iter().any(|s| s.detail.starts_with("pid ")));
    }

    #[test]
    fn container_forwarders_are_skipped() {
        let ss = "tcp LISTEN 0 4096 0.0.0.0:8080 0.0.0.0:* users:((\"rootlessport\",pid=4242,fd=10))\n\
tcp LISTEN 0 4096 127.0.0.1:3000 0.0.0.0:* users:((\"node\",pid=77,fd=20))\n";
        let services = host_services(&parse_ss(ss), |_| None);
        assert_eq!(services.len(), 1);
        assert_eq!(services[0].name, "node");
        assert_eq!(services[0].ports[0].url, "http://localhost:3000");
    }
}
