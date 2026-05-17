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

use crate::{item::LauncherItem, notify::toast, provider::Provider};
use serde::Deserialize;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;
use std::rc::Rc;

/// One host snapshot, projected from the YAML map for cheap
/// search-time access.
#[derive(Debug, Clone)]
struct Host {
    /// The map key — `kenan_e14u7`, `vhay`, etc. Passed to `ssh`
    /// verbatim.
    name: String,
    /// `Hostname:` field. Hosts without this are pure templates
    /// (inherited via `Inherits:`) and the provider filters
    /// them out.
    hostname: String,
    /// `User:` field, empty when unset.
    user: String,
    /// `Port:` field as a string, empty when unset.
    port: String,
}

/// Minimal YAML schema — just enough to identify hosts and pull
/// their display fields. `serde_yaml::Value` for everything else
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
    port: Option<serde_yaml::Value>,
}

pub struct SshProvider {
    config_path: PathBuf,
    hosts: RefCell<Vec<Host>>,
    terminal: String,
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
        let terminal = std::env::var("TERMINAL")
            .ok()
            .or_else(|| {
                ["kitty", "alacritty", "foot", "wezterm"]
                    .iter()
                    .find(|t| which_exists(t))
                    .map(|t| t.to_string())
            })
            .unwrap_or_else(|| "kitty".into());
        let me = Self {
            config_path: path,
            hosts: RefCell::new(Vec::new()),
            terminal,
        };
        me.refresh();
        me
    }

    pub fn refresh(&self) {
        let hosts = std::fs::read_to_string(&self.config_path)
            .ok()
            .and_then(|src| parse_assh(&src).ok())
            .unwrap_or_default();
        *self.hosts.borrow_mut() = hosts;
    }
}

impl Default for SshProvider {
    fn default() -> Self {
        Self::new()
    }
}

fn which_exists(bin: &str) -> bool {
    Command::new("which")
        .arg(bin)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Parse the YAML and project each `hosts.<name>` entry into a
/// `Host` snapshot. Hosts without a `Hostname:` field are
/// dropped — they're inheritance templates (`kenan_base:`) that
/// can't be SSH'd into directly. Returns the host list sorted
/// alphabetically by name so the empty-query browse view is
/// predictable.
fn parse_assh(src: &str) -> Result<Vec<Host>, serde_yaml::Error> {
    let file: AsshFile = serde_yaml::from_str(src)?;
    let mut out: Vec<Host> = file
        .hosts
        .into_iter()
        .filter_map(|(name, entry)| {
            let hostname = entry.hostname?;
            let user = entry.user.unwrap_or_default();
            let port = entry
                .port
                .map(|v| match v {
                    serde_yaml::Value::Number(n) => n.to_string(),
                    serde_yaml::Value::String(s) => s,
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
            description: "Connect to a host defined in ~/.ssh/assh.yml".into(),
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
                    "Expected hosts: <name>: ... in {}",
                    self.config_path.display()
                ),
                icon: "dialog-warning-symbolic".into(),
                icon_is_path: false,
                score: 100.0,
                provider_name: "SSH".into(),
                usage_key: None,
                on_activate: Rc::new(|| {}),
            }];
        }

        let terminal = self.terminal.clone();
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
                let label_host = if h.user.is_empty() {
                    h.hostname.clone()
                } else {
                    format!("{}@{}", h.user, h.hostname)
                };
                let description = if h.port.is_empty() {
                    label_host.clone()
                } else {
                    format!("{label_host}:{}", h.port)
                };
                let terminal_clone = terminal.clone();
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
                        spawn_terminal_ssh(&terminal_clone, &name);
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

/// Spawn `<terminal> -e ssh <host>`. wezterm wants `start --`,
/// the rest accept `-e` — mirrors the convention in
/// ArchLinuxPkgsProvider's terminal spawn.
fn spawn_terminal_ssh(terminal: &str, host: &str) {
    let result = if terminal == "wezterm" {
        Command::new(terminal).args(["start", "--", "ssh", host]).spawn()
    } else {
        Command::new(terminal).args(["-e", "ssh", host]).spawn()
    };
    if let Err(err) = result {
        tracing::warn!(?err, terminal, host, "ssh provider spawn failed");
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
        assert!(p.search("ssh").iter().any(|i| i.name == "No SSH hosts found"));
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
