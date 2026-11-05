//! `ssh <query>` — pick a host from `~/.ssh/assh.yml` and open
//! a terminal connected to it.
//!
//! assh (https://github.com/moul/assh) is a thin YAML wrapper
//! around OpenSSH config. Users define hosts under a `hosts:`
//! map; assh expands those to a regenerated `~/.ssh/config` and
//! plain `ssh <name>` then works transparently. This provider
//! reads the YAML directly so we don't need assh installed at
//! launcher start — even users who only sometimes run `assh
//! config build` will see their declared hosts.
//!
//! ## Behaviour
//!
//! | Query | Result |
//! |---|---|
//! | `ssh` (bare) | Every host in the file, frecency-sorted |
//! | `ssh <partial>` | Substring filter on name + Hostname + User |
//!
//! Activation spawns `$TERMINAL -e ssh <name>` (falls back to
//! kitty when `$TERMINAL` is unset).
//!
//! ## Fallback: plain `~/.ssh/config`
//!
//! Without assh (no `assh.yml`, or one that declares no connectable
//! host) the provider lists the `Host` aliases of `~/.ssh/config`
//! instead, following `Include` lines. Wildcard / negated patterns
//! (`Host *`, `Host !foo`) and `Match` blocks are skipped; every literal
//! alias on a multi-alias `Host` line becomes its own row. Aliases
//! without a `HostName` are kept — ssh resolves the alias itself.

use crate::{item::LauncherItem, notify::toast, provider::Provider};
use mshell_process::SpawnReaped;
use serde::Deserialize;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;

/// One host snapshot, projected from the YAML map (or an OpenSSH
/// config's `Host` alias) for cheap search-time access. Public for the
/// shell's SSH-sessions pill, which lists the same hosts
/// ([`load_ssh_config`]).
#[derive(Debug, Clone)]
pub struct Host {
    /// The map key / `Host` alias — `kenan_e14u7`, `vhay`, etc.
    /// Passed to `ssh` verbatim.
    pub name: String,
    /// `Hostname:` field. In assh, hosts without this are pure
    /// templates (inherited via `Inherits:`) and the provider filters
    /// them out; an OpenSSH alias without one is kept.
    pub hostname: String,
    /// `User:` field, empty when unset.
    pub user: String,
    /// `Port:` field as a string, empty when unset.
    pub port: String,
}

/// Minimal YAML schema — just enough to identify hosts and pull
/// their display fields. `yaml_serde::Value` for everything else
/// because assh accepts many fields we don't surface.
#[derive(Debug, Deserialize)]
struct AsshFile {
    #[serde(default)]
    hosts: BTreeMap<String, HostEntry>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "PascalCase")]
struct HostEntry {
    #[serde(default)]
    hostname: Option<String>,
    #[serde(default)]
    user: Option<String>,
    #[serde(default)]
    port: Option<yaml_serde::Value>,
}

pub struct SshProvider {
    config_path: PathBuf,
    /// OpenSSH client config read when assh yields no hosts.
    ssh_config_path: PathBuf,
    hosts: RefCell<Vec<Host>>,
}

impl SshProvider {
    /// Use the default assh location (`~/.ssh/assh.yml`).
    pub fn new() -> Self {
        let config_path = dirs::home_dir()
            .map(|h| h.join(".ssh").join("assh.yml"))
            .unwrap_or_else(|| PathBuf::from("/home/nobody/.ssh/assh.yml"));
        Self::with_path(config_path)
    }

    pub fn with_path(path: PathBuf) -> Self {
        let ssh_config = path
            .parent()
            .map(|d| d.join("config"))
            .unwrap_or_else(|| PathBuf::from("/nonexistent/config"));
        Self::with_paths(path, ssh_config)
    }

    /// assh YAML path plus the OpenSSH config used as fallback.
    pub fn with_paths(path: PathBuf, ssh_config_path: PathBuf) -> Self {
        let me = Self {
            config_path: path,
            ssh_config_path,
            hosts: RefCell::new(Vec::new()),
        };
        me.refresh();
        me
    }

    pub fn refresh(&self) {
        let hosts = std::fs::read_to_string(&self.config_path)
            .ok()
            .and_then(|src| parse_assh(&src).ok())
            .unwrap_or_default();
        if hosts.is_empty() {
            *self.hosts.borrow_mut() = load_ssh_config(&self.ssh_config_path);
        } else {
            *self.hosts.borrow_mut() = hosts;
        }
    }
}

impl Default for SshProvider {
    fn default() -> Self {
        Self::new()
    }
}

/// Read an OpenSSH client config (and its `Include`s) into hosts: every
/// literal `Host` alias once (first definition wins), sorted by name.
/// Wildcard / negated patterns and `Match` blocks are skipped.
pub fn load_ssh_config(path: &std::path::Path) -> Vec<Host> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let base = path
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_default();
    collect_ssh_config(path, &base, &mut out, &mut seen, 0);
    let mut names = std::collections::HashSet::new();
    out.retain(|h| names.insert(h.name.clone()));
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

fn collect_ssh_config(
    path: &std::path::Path,
    base: &std::path::Path,
    out: &mut Vec<Host>,
    seen: &mut std::collections::HashSet<PathBuf>,
    depth: usize,
) {
    // ssh itself caps Include recursion at 16; cycles are cut by `seen`.
    if depth > 16 || !seen.insert(path.to_path_buf()) {
        return;
    }
    let Ok(src) = std::fs::read_to_string(path) else {
        return;
    };
    let parsed = parse_ssh_config(&src);
    out.extend(parsed.hosts);
    for pattern in parsed.includes {
        for file in expand_include(&pattern, base) {
            collect_ssh_config(&file, base, out, seen, depth + 1);
        }
    }
}

#[derive(Debug, Default)]
struct SshConfig {
    hosts: Vec<Host>,
    includes: Vec<String>,
}

/// Split a config line into lowercase keyword + value (`Key value`,
/// `Key=value`, `Key = value`).
fn split_kv(line: &str) -> Option<(String, &str)> {
    let idx = line.find(|c: char| c.is_whitespace() || c == '=')?;
    let (key, rest) = line.split_at(idx);
    let value = rest
        .trim_start_matches(|c: char| c.is_whitespace() || c == '=')
        .trim();
    (!key.is_empty()).then(|| (key.to_ascii_lowercase(), value))
}

/// Parse one OpenSSH config file. Keys inside a `Host` block apply to
/// every literal alias of that block; `Match` ends the block.
fn parse_ssh_config(src: &str) -> SshConfig {
    let mut cfg = SshConfig::default();
    // Indices into cfg.hosts of the current block's aliases.
    let mut current: Vec<usize> = Vec::new();
    for line in src.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let Some((key, value)) = split_kv(t) else {
            continue;
        };
        match key.as_str() {
            "host" => {
                current.clear();
                for alias in value.split_whitespace().map(|a| a.trim_matches('"')) {
                    if alias.is_empty() || alias.contains(['*', '?', '!']) {
                        continue;
                    }
                    current.push(cfg.hosts.len());
                    cfg.hosts.push(Host {
                        name: alias.to_string(),
                        hostname: String::new(),
                        user: String::new(),
                        port: String::new(),
                    });
                }
            }
            "match" => current.clear(),
            "include" => cfg
                .includes
                .extend(value.split_whitespace().map(str::to_string)),
            "hostname" | "user" | "port" => {
                // First value wins in ssh_config.
                for &i in &current {
                    let h = &mut cfg.hosts[i];
                    let slot = match key.as_str() {
                        "hostname" => &mut h.hostname,
                        "user" => &mut h.user,
                        _ => &mut h.port,
                    };
                    if slot.is_empty() {
                        *slot = value.to_string();
                    }
                }
            }
            _ => {}
        }
    }
    cfg
}

/// Resolve an `Include` argument: `~/` expands to home, relative paths
/// are relative to `~/.ssh` (`base`), and a `*` / `?` glob in the final
/// component is matched against that directory.
fn expand_include(pattern: &str, base: &std::path::Path) -> Vec<PathBuf> {
    let path = if let Some(rest) = pattern.strip_prefix("~/") {
        match dirs::home_dir() {
            Some(h) => h.join(rest),
            None => return Vec::new(),
        }
    } else if pattern.starts_with('/') {
        PathBuf::from(pattern)
    } else {
        base.join(pattern)
    };
    let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else {
        return Vec::new();
    };
    if !name.contains(['*', '?']) {
        return vec![path];
    }
    let Some(dir) = path.parent() else {
        return Vec::new();
    };
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = rd
        .flatten()
        .filter(|e| glob_match(&name, &e.file_name().to_string_lossy()))
        .map(|e| e.path())
        .collect();
    files.sort();
    files
}

/// Minimal `*` / `?` glob over one path component.
fn glob_match(pattern: &str, text: &str) -> bool {
    fn go(p: &[char], t: &[char]) -> bool {
        match p.split_first() {
            None => t.is_empty(),
            Some(('*', rest)) => (0..=t.len()).any(|i| go(rest, &t[i..])),
            Some(('?', rest)) => !t.is_empty() && go(rest, &t[1..]),
            Some((c, rest)) => t.first() == Some(c) && go(rest, &t[1..]),
        }
    }
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    go(&p, &t)
}

/// Parse the YAML and project each `hosts.<name>` entry into a
/// `Host` snapshot. Hosts without a `Hostname:` field are
/// dropped — they're inheritance templates (`kenan_base:`) that
/// can't be SSH'd into directly. Returns the host list sorted
/// alphabetically by name so the empty-query browse view is
/// predictable.
fn parse_assh(src: &str) -> Result<Vec<Host>, yaml_serde::Error> {
    let file: AsshFile = yaml_serde::from_str(src)?;
    let mut out: Vec<Host> = file
        .hosts
        .into_iter()
        .filter_map(|(name, entry)| {
            let hostname = entry.hostname?;
            let user = entry.user.unwrap_or_default();
            let port = entry
                .port
                .map(|v| match v {
                    yaml_serde::Value::Number(n) => n.to_string(),
                    yaml_serde::Value::String(s) => s,
                    _ => String::new(),
                })
                .unwrap_or_default();
            Some(Host {
                name,
                hostname,
                user,
                port,
            })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

impl Provider for SshProvider {
    fn name(&self) -> &str {
        "SSH"
    }

    fn category(&self) -> &str {
        "Connect"
    }

    fn handles_search(&self) -> bool {
        // `ssh` is a real word the user might type to find an
        // app (Filezilla SSH? OpenSSH-something?). Stay out of
        // the regular search path; require the bare-prefix
        // invocation.
        false
    }

    fn handles_command(&self, query: &str) -> bool {
        let q = query.trim_start();
        q == "ssh" || q.starts_with("ssh ")
    }

    fn commands(&self) -> Vec<LauncherItem> {
        vec![LauncherItem {
            id: "ssh:palette".into(),
            name: "ssh".into(),
            description: "Connect to a host from ~/.ssh/assh.yml or ~/.ssh/config".into(),
            icon: "network-server-symbolic".into(),
            icon_is_path: false,
            score: 0.0,
            provider_name: "SSH".into(),
            usage_key: None,
            on_activate: Rc::new(|| {}),
        }]
    }

    fn search(&self, query: &str) -> Vec<LauncherItem> {
        let q = query.trim_start();
        if !(q == "ssh" || q.starts_with("ssh ")) {
            return Vec::new();
        }
        let filter = q.trim_start_matches("ssh").trim().to_ascii_lowercase();

        let hosts = self.hosts.borrow();
        if hosts.is_empty() {
            return vec![LauncherItem {
                id: "ssh:none".into(),
                name: "No SSH hosts found".into(),
                description: format!(
                    "Expected hosts in {} or Host entries in {}",
                    self.config_path.display(),
                    self.ssh_config_path.display()
                ),
                icon: "dialog-warning-symbolic".into(),
                icon_is_path: false,
                score: 100.0,
                provider_name: "SSH".into(),
                usage_key: None,
                on_activate: Rc::new(|| {}),
            }];
        }

        hosts
            .iter()
            .filter(|h| {
                if filter.is_empty() {
                    return true;
                }
                h.name.to_ascii_lowercase().contains(&filter)
                    || h.hostname.to_ascii_lowercase().contains(&filter)
                    || h.user.to_ascii_lowercase().contains(&filter)
            })
            .enumerate()
            .map(|(idx, h)| {
                let name = h.name.clone();
                // ~/.ssh/config aliases may have no HostName: ssh then
                // connects to the alias itself.
                let target = if h.hostname.is_empty() {
                    h.name.clone()
                } else {
                    h.hostname.clone()
                };
                let label_host = if h.user.is_empty() {
                    target
                } else {
                    format!("{}@{target}", h.user)
                };
                let description = if h.port.is_empty() {
                    label_host.clone()
                } else {
                    format!("{label_host}:{}", h.port)
                };
                let name_for_toast = name.clone();
                LauncherItem {
                    id: format!("ssh:{}", h.name),
                    name: format!("ssh {}", h.name),
                    description,
                    icon: "network-server-symbolic".into(),
                    icon_is_path: false,
                    score: 180.0 - idx as f64,
                    provider_name: "SSH".into(),
                    usage_key: Some(format!("ssh:{}", h.name)),
                    on_activate: Rc::new(move || {
                        spawn_terminal_ssh(&name);
                        toast("SSH", format!("Connecting to {name_for_toast}"));
                    }),
                }
            })
            .collect()
    }

    fn on_opened(&mut self) {
        // Re-read assh.yml on every open so edits made between
        // launcher invocations show up. Cheap (~ms).
        self.refresh();
    }

    /// Connect tab — list every host from assh.yml without
    /// requiring the `ssh ` prefix; `filter` narrows by host /
    /// hostname / user substring.
    fn browse(&self, filter: &str) -> Vec<LauncherItem> {
        if filter.is_empty() {
            self.search("ssh")
        } else {
            self.search(&format!("ssh {filter}"))
        }
    }
}

/// Open the user's terminal running `ssh <host>`
/// (`mshell_process::terminal`).
fn spawn_terminal_ssh(host: &str) {
    match mshell_process::terminal::command(&["ssh", host], "ssh") {
        Ok((terminal, mut cmd)) => {
            if let Err(err) = cmd.spawn_reaped() {
                tracing::warn!(?err, %terminal, host, "ssh provider spawn failed");
            }
        }
        Err(err) => tracing::warn!(%err, host, "ssh provider: no terminal"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_assh_yaml() {
        let yaml = r#"
hosts:
  kenan_base:
    User: git
    IdentityFile: ~/.ssh/id_ed25519
  github:
    Inherits: kenan_base
    Hostname: github.com
  vhay:
    Hostname: localhost
    User: kenan
    Port: 2288
"#;
        let hosts = parse_assh(yaml).unwrap();
        // `kenan_base` has no Hostname → filtered out.
        let names: Vec<&str> = hosts.iter().map(|h| h.name.as_str()).collect();
        assert!(names.contains(&"github"));
        assert!(names.contains(&"vhay"));
        assert!(!names.contains(&"kenan_base"));
    }

    #[test]
    fn port_accepts_number_or_string() {
        let yaml = r#"
hosts:
  a:
    Hostname: a.example
    Port: 22
  b:
    Hostname: b.example
    Port: "2222"
"#;
        let hosts = parse_assh(yaml).unwrap();
        let a = hosts.iter().find(|h| h.name == "a").unwrap();
        let b = hosts.iter().find(|h| h.name == "b").unwrap();
        assert_eq!(a.port, "22");
        assert_eq!(b.port, "2222");
    }

    #[test]
    fn missing_file_yields_empty_hosts() {
        let p = SshProvider::with_path(PathBuf::from("/nonexistent/assh.yml"));
        assert!(
            p.search("ssh")
                .iter()
                .any(|i| i.name == "No SSH hosts found")
        );
    }

    // Shape of the owner's assh-generated ~/.ssh/config (values changed).
    const SSH_CONFIG: &str = "# generated by assh\n\nHost agu_admin\n  User root\n  # HostName 10.0.0.1\n\nHost agu_draco draco\n  Port 45652\n  User root\n  HostName draco.example\n  HostName ignored.example\n\nHost *.lan !bad\n  User nobody\n\nMatch host foo\n  User matchuser\n\nHost=vhay\n  Hostname = localhost\n  Port=2288\n\nHost *\n  ServerAliveInterval 30\n  Include extra.conf\n";

    #[test]
    fn parses_ssh_config_host_blocks() {
        let cfg = parse_ssh_config(SSH_CONFIG);
        let names: Vec<&str> = cfg.hosts.iter().map(|h| h.name.as_str()).collect();
        assert_eq!(names, vec!["agu_admin", "agu_draco", "draco", "vhay"]);
        let admin = &cfg.hosts[0];
        assert_eq!((admin.user.as_str(), admin.hostname.as_str()), ("root", ""));
        let draco = &cfg.hosts[2];
        assert_eq!(draco.hostname, "draco.example", "first HostName wins");
        assert_eq!(draco.port, "45652");
        let vhay = &cfg.hosts[3];
        assert_eq!(
            (vhay.hostname.as_str(), vhay.port.as_str()),
            ("localhost", "2288")
        );
        assert_eq!(cfg.includes, vec!["extra.conf"]);
    }

    #[test]
    fn falls_back_to_ssh_config_with_includes() {
        let dir = std::env::temp_dir().join(format!("mshell-ssh-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("conf.d")).unwrap();
        std::fs::write(
            dir.join("config"),
            "Include conf.d/*.conf\nHost main\n  HostName m.example\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("conf.d/a.conf"),
            "Host inc_a\n  User u\nInclude ../config\n",
        )
        .unwrap();
        std::fs::write(dir.join("conf.d/b.txt"), "Host not_included\n").unwrap();
        let p = SshProvider::with_paths(dir.join("assh.yml"), dir.join("config"));
        let names: Vec<String> = p.search("ssh").into_iter().map(|i| i.name).collect();
        assert_eq!(names, vec!["ssh inc_a", "ssh main"]);
        let a = p.search("ssh inc_a");
        assert_eq!(
            a[0].description, "u@inc_a",
            "alias stands in for missing HostName"
        );
        // assh hosts take precedence when present.
        std::fs::write(
            dir.join("assh.yml"),
            "hosts:\n  yhost:\n    Hostname: y.example\n",
        )
        .unwrap();
        let p = SshProvider::with_paths(dir.join("assh.yml"), dir.join("config"));
        let names: Vec<String> = p.search("ssh").into_iter().map(|i| i.name).collect();
        assert_eq!(names, vec!["ssh yhost"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn glob_matches_single_component() {
        assert!(glob_match("*.conf", "a.conf"));
        assert!(!glob_match("*.conf", "a.txt"));
        assert!(glob_match("h?st", "host"));
    }

    #[test]
    fn handles_command_only_for_ssh_prefix() {
        let p = SshProvider::new();
        assert!(p.handles_command("ssh"));
        assert!(p.handles_command("ssh foo"));
        assert!(!p.handles_command("sshfs"));
        assert!(!p.handles_command(":ssh"));
    }
}
