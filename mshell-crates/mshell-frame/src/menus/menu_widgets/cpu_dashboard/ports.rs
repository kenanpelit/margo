//! Listening sockets for the CPU dashboard's Ports tab: `ss -tulpnH`
//! output → typed rows, grouped by exposure (loopback-only vs every
//! interface vs a specific address), with search. Pure; tested against
//! real `ss` output captured on the owner's machine (`testdata/`).

/// Who can reach a listening socket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Exposure {
    /// Bound to every interface (`0.0.0.0`, `[::]`, `*`): reachable from
    /// the network unless a firewall says otherwise. Listed first.
    AllInterfaces,
    /// Bound to one non-loopback address (incl. multicast groups).
    Specific,
    /// `127.0.0.0/8` / `::1`: this machine only.
    Loopback,
}

impl Exposure {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Exposure::AllInterfaces => "All interfaces",
            Exposure::Specific => "Specific address",
            Exposure::Loopback => "This machine only",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PortOwner {
    pub name: String,
    pub pid: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PortRow {
    /// `tcp` / `udp`.
    pub proto: String,
    /// Bound address without brackets / `%iface` (`127.0.0.53`, `::1`, `*`).
    pub addr: String,
    /// The `%iface` scope when present (`lo`).
    pub iface: Option<String>,
    pub port: u16,
    /// Empty when `ss` can't see the owner (another user's / root's socket).
    pub owners: Vec<PortOwner>,
}

impl PortRow {
    pub(crate) fn exposure(&self) -> Exposure {
        let a = self.addr.as_str();
        if a == "*" || a == "0.0.0.0" || a == "::" {
            Exposure::AllInterfaces
        } else if a.starts_with("127.") || a == "::1" || self.iface.as_deref() == Some("lo") {
            Exposure::Loopback
        } else {
            Exposure::Specific
        }
    }

    /// `name` of the first owner, or `None` when hidden from us.
    #[cfg(test)]
    pub(crate) fn owner_name(&self) -> Option<&str> {
        self.owners.first().map(|o| o.name.as_str())
    }
}

/// Split `ss`'s local address column: `127.0.0.53%lo:53`, `[::1]:5174`,
/// `*:1716`, `[::]:5355`.
fn split_addr(s: &str) -> Option<(String, Option<String>, u16)> {
    let (host, port) = s.rsplit_once(':')?;
    let port = port.parse().ok()?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let (host, iface) = match host.split_once('%') {
        Some((h, i)) => (h, Some(i.to_string())),
        None => (host, None),
    };
    Some((host.to_string(), iface, port))
}

/// `users:(("chrome",pid=10689,fd=82),("chrome",pid=10690,fd=3))`
fn parse_users(s: &str) -> Vec<PortOwner> {
    let mut out: Vec<PortOwner> = Vec::new();
    let mut rest = s;
    while let Some(start) = rest.find("(\"") {
        rest = &rest[start + 2..];
        let Some(end_name) = rest.find('"') else {
            break;
        };
        let name = rest[..end_name].to_string();
        rest = &rest[end_name..];
        let pid = rest
            .find("pid=")
            .and_then(|i| {
                rest[i + 4..]
                    .split(|c: char| !c.is_ascii_digit())
                    .next()
                    .and_then(|d| d.parse().ok())
            })
            .unwrap_or(0);
        if pid > 0 && !out.iter().any(|o| o.pid == pid) {
            out.push(PortOwner { name, pid });
        }
    }
    out
}

/// Parse `ss -tulpnH` (no header). Duplicate sockets (one process holding
/// several fds on the same address, e.g. Chrome's mDNS) collapse to one
/// row. Unparseable lines are skipped.
pub(crate) fn parse_ss(out: &str) -> Vec<PortRow> {
    let mut rows: Vec<PortRow> = Vec::new();
    for line in out.lines() {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 6 || cols[0] == "Netid" {
            continue;
        }
        let Some((addr, iface, port)) = split_addr(cols[4]) else {
            continue;
        };
        let owners = cols
            .get(6..)
            .map(|r| parse_users(&r.join(" ")))
            .unwrap_or_default();
        let row = PortRow {
            proto: cols[0].to_string(),
            addr,
            iface,
            port,
            owners,
        };
        if let Some(existing) = rows.iter_mut().find(|r| {
            r.proto == row.proto && r.addr == row.addr && r.port == row.port && r.iface == row.iface
        }) {
            for o in row.owners {
                if !existing.owners.iter().any(|e| e.pid == o.pid) {
                    existing.owners.push(o);
                }
            }
            continue;
        }
        rows.push(row);
    }
    rows
}

/// Which protocols the tab shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ProtoFilter {
    /// TCP listeners: the "what is serving" view, and the default.
    #[default]
    Tcp,
    Udp,
    All,
}

/// Search + protocol filter: port number prefix, owner name substring, or
/// PID prefix; empty query passes everything.
pub(crate) fn port_matches(row: &PortRow, query: &str, proto: ProtoFilter) -> bool {
    let proto_ok = match proto {
        ProtoFilter::Tcp => row.proto == "tcp",
        ProtoFilter::Udp => row.proto == "udp",
        ProtoFilter::All => true,
    };
    if !proto_ok {
        return false;
    }
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return true;
    }
    row.port.to_string().starts_with(&q)
        || row.addr.contains(&q)
        || row
            .owners
            .iter()
            .any(|o| o.name.to_lowercase().contains(&q) || o.pid.to_string().starts_with(&q))
}

/// Rows sorted for display: exposure (widest first), then port, proto.
pub(crate) fn sort_ports(rows: &mut [PortRow]) {
    rows.sort_by(|a, b| {
        a.exposure()
            .cmp(&b.exposure())
            .then(a.port.cmp(&b.port))
            .then(a.proto.cmp(&b.proto))
            .then(a.addr.cmp(&b.addr))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const SS: &str = include_str!("testdata/ss_tulpn.txt");

    #[test]
    fn parses_real_ss_output() {
        let rows = parse_ss(SS);
        let hermes = rows
            .iter()
            .find(|r| r.port == 8642)
            .expect("hermes listener");
        assert_eq!(hermes.proto, "tcp");
        assert_eq!(hermes.addr, "127.0.0.1");
        assert_eq!(
            hermes.owners,
            [PortOwner {
                name: "hermes".into(),
                pid: 14374
            }]
        );
        assert_eq!(hermes.exposure(), Exposure::Loopback);

        // systemd-resolved's stub: `127.0.0.53%lo:53`, owner hidden (root).
        let stub = rows
            .iter()
            .find(|r| r.proto == "tcp" && r.addr == "127.0.0.53")
            .expect("resolved stub");
        assert_eq!(stub.iface.as_deref(), Some("lo"));
        assert_eq!(stub.port, 53);
        assert!(stub.owners.is_empty());
        assert_eq!(stub.owner_name(), None);

        // IPv6 brackets.
        let node = rows.iter().find(|r| r.port == 5174).expect("vite");
        assert_eq!(node.addr, "::1");
        assert_eq!(node.exposure(), Exposure::Loopback);
        assert_eq!(node.owner_name(), Some("node-MainThread"));

        // `*:1716` (valent) and `0.0.0.0:6600` (mpd) are network-exposed.
        let valent = rows
            .iter()
            .find(|r| r.proto == "tcp" && r.port == 1716)
            .expect("valent");
        assert_eq!(valent.addr, "*");
        assert_eq!(valent.exposure(), Exposure::AllInterfaces);
        let mpd = rows.iter().find(|r| r.port == 6600).expect("mpd");
        assert_eq!(mpd.exposure(), Exposure::AllInterfaces);
        assert_eq!(mpd.owners[0].pid, 1766);

        // Multicast group bind → specific address.
        let mdns = rows.iter().find(|r| r.addr == "224.0.0.251").expect("mdns");
        assert_eq!(mdns.exposure(), Exposure::Specific);
    }

    #[test]
    fn duplicate_fds_collapse_into_one_row() {
        let rows = parse_ss(SS);
        let mdns: Vec<_> = rows.iter().filter(|r| r.addr == "224.0.0.251").collect();
        assert_eq!(mdns.len(), 1, "6 sockets on 224.0.0.251:5353 → one row");
        let pids: Vec<i32> = mdns[0].owners.iter().map(|o| o.pid).collect();
        assert_eq!(pids, [5653, 14028, 10689]);
        assert!(rows.len() < SS.lines().count());
    }

    #[test]
    fn multiple_owners_and_garbage() {
        let rows = parse_ss(
            "tcp LISTEN 0 4096 0.0.0.0:80 0.0.0.0:* users:((\"nginx\",pid=10,fd=6),(\"nginx\",pid=11,fd=6))\n\
             Netid State Recv-Q Send-Q Local Address:Port Peer Address:Port Process\n\
             tcp LISTEN 0 1 nonsense 0.0.0.0:*\nshort line\n",
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].owners.len(), 2);
    }

    #[test]
    fn search_and_filter() {
        let rows = parse_ss(SS);
        let tcp: Vec<_> = rows
            .iter()
            .filter(|r| port_matches(r, "", ProtoFilter::Tcp))
            .collect();
        assert!(tcp.iter().all(|r| r.proto == "tcp"));
        assert!(
            rows.iter()
                .any(|r| port_matches(r, "TRANSMISSION", ProtoFilter::All))
        );
        let by_port: Vec<_> = rows
            .iter()
            .filter(|r| port_matches(r, "8787", ProtoFilter::All))
            .collect();
        assert_eq!(by_port.len(), 1);
        assert_eq!(by_port[0].owner_name(), Some("headroom"));
        assert!(
            rows.iter()
                .any(|r| port_matches(r, "14374", ProtoFilter::Tcp))
        );
        assert!(
            !rows
                .iter()
                .any(|r| port_matches(r, "8642", ProtoFilter::Udp))
        );
    }

    #[test]
    fn sort_puts_exposed_first() {
        let mut rows = parse_ss(SS);
        sort_ports(&mut rows);
        assert_eq!(rows[0].exposure(), Exposure::AllInterfaces);
        assert_eq!(rows.last().map(|r| r.exposure()), Some(Exposure::Loopback));
    }
}
