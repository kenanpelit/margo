//! `bt` palette — connect / disconnect Bluetooth devices.
//!
//! Subprocess wrapper around `bluetoothctl`. We enumerate paired
//! devices via `bluetoothctl paired-devices` (one MAC + name per
//! line), then run `bluetoothctl info <mac>` for each to detect
//! the current connection state.
//!
//! Activation toggles: connected → disconnect, otherwise →
//! connect. Pair / trust flows are out of scope for the launcher
//! — those are GUI-heavy enough that the system's bluetooth
//! settings panel is the right tool.

use crate::{item::LauncherItem, notify::toast, provider::Provider};
use std::process::Command;
use std::rc::Rc;

pub struct BluetoothProvider;

impl BluetoothProvider {
    pub fn new() -> Self {
        Self
    }
}

impl Default for BluetoothProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Device {
    /// MAC address (`AA:BB:CC:DD:EE:FF`).
    mac: String,
    name: String,
    connected: bool,
}

/// Parse `bluetoothctl paired-devices` output. Each line is:
///     Device AA:BB:CC:DD:EE:FF Some Name With Spaces
fn parse_paired(stdout: &str) -> Vec<(String, String)> {
    stdout
        .lines()
        .filter_map(|line| {
            let mut tokens = line.split_whitespace();
            // Header word ("Device") then MAC then name…
            if tokens.next()? != "Device" {
                return None;
            }
            let mac = tokens.next()?.to_string();
            let name: String = tokens.collect::<Vec<_>>().join(" ");
            if name.is_empty() {
                None
            } else {
                Some((mac, name))
            }
        })
        .collect()
}

/// Probe a single device's connection state via
/// `bluetoothctl info <mac>` — looking for `Connected: yes/no`.
fn is_connected(mac: &str) -> bool {
    Command::new("bluetoothctl")
        .args(["info", mac])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .any(|l| l.trim().starts_with("Connected: yes"))
        })
        .unwrap_or(false)
}

fn snapshot() -> Vec<Device> {
    // bluez 5.65+ removed the standalone `paired-devices`
    // subcommand in favour of `devices <filter>`. We try the
    // new form first and fall back to the old one for older
    // installs. Output format is identical: `Device <MAC> <name>`
    // one per line, so the parser handles both transparently.
    let out = Command::new("bluetoothctl")
        .args(["devices", "Paired"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .or_else(|| {
            Command::new("bluetoothctl")
                .args(["paired-devices"])
                .output()
                .ok()
                .filter(|o| o.status.success())
        });
    let Some(out) = out else {
        return Vec::new();
    };
    parse_paired(&String::from_utf8_lossy(&out.stdout))
        .into_iter()
        .map(|(mac, name)| {
            let connected = is_connected(&mac);
            Device {
                mac,
                name,
                connected,
            }
        })
        .collect()
}

impl Provider for BluetoothProvider {
    fn name(&self) -> &str {
        "Bluetooth"
    }

    fn category(&self) -> &str {
        "System"
    }

    fn handles_search(&self) -> bool {
        false
    }

    fn handles_command(&self, query: &str) -> bool {
        let q = query.trim_start();
        q == "bt"
            || q.starts_with("bt ")
            || q == "bluetooth"
            || q.starts_with("bluetooth ")
    }

    fn commands(&self) -> Vec<LauncherItem> {
        vec![LauncherItem {
            id: "bt:palette".into(),
            name: "bt".into(),
            description: "Connect / disconnect paired Bluetooth devices".into(),
            icon: "bluetooth-symbolic".into(),
            icon_is_path: false,
            score: 0.0,
            provider_name: "Bluetooth".into(),
            usage_key: None,
            on_activate: Rc::new(|| {}),
        }]
    }

    fn search(&self, query: &str) -> Vec<LauncherItem> {
        let q = query.trim_start();
        if !(q == "bt"
            || q.starts_with("bt ")
            || q == "bluetooth"
            || q.starts_with("bluetooth "))
        {
            return Vec::new();
        }
        let filter = q
            .trim_start_matches("bluetooth")
            .trim_start_matches("bt")
            .trim()
            .to_ascii_lowercase();

        let mut devices = snapshot();
        if devices.is_empty() {
            return vec![LauncherItem {
                id: "bt:none".into(),
                name: "No paired Bluetooth devices".into(),
                description: "Pair a device first via the system settings panel".into(),
                icon: "bluetooth-disabled-symbolic".into(),
                icon_is_path: false,
                score: 100.0,
                provider_name: "Bluetooth".into(),
                usage_key: None,
                on_activate: Rc::new(|| {}),
            }];
        }
        if !filter.is_empty() {
            devices.retain(|d| d.name.to_ascii_lowercase().contains(&filter));
        }
        // Connected first.
        devices.sort_by(|a, b| b.connected.cmp(&a.connected).then(a.name.cmp(&b.name)));

        devices
            .into_iter()
            .enumerate()
            .map(|(idx, dev)| {
                let mac = dev.mac.clone();
                let name = dev.name.clone();
                let toast_name = name.clone();
                let connected = dev.connected;
                let action_label = if connected {
                    "Disconnect"
                } else {
                    "Connect"
                };
                LauncherItem {
                    id: format!("bt:{}", dev.mac),
                    name: if connected {
                        format!("● {}", dev.name)
                    } else {
                        format!("○ {}", dev.name)
                    },
                    description: format!("{} · {action_label} on Enter", dev.mac),
                    icon: if connected {
                        "bluetooth-active-symbolic".into()
                    } else {
                        "bluetooth-symbolic".into()
                    },
                    icon_is_path: false,
                    score: if connected {
                        200.0 - idx as f64
                    } else {
                        180.0 - idx as f64
                    },
                    provider_name: "Bluetooth".into(),
                    usage_key: Some(format!("bt:{}", dev.mac)),
                    on_activate: Rc::new(move || {
                        let sub = if connected { "disconnect" } else { "connect" };
                        if let Err(err) = Command::new("bluetoothctl")
                            .args([sub, &mac])
                            .spawn()
                        {
                            tracing::warn!(?err, mac, sub, "bluetoothctl failed");
                        } else {
                            toast(action_label, toast_name.clone());
                        }
                    }),
                }
            })
            .collect()
    }

    /// System tab — surface the paired devices list without the
    /// `bt` prefix. Re-uses the search path so connect / disconnect
    /// behaviour and the connected-first ordering are identical.
    fn browse(&self) -> Vec<LauncherItem> {
        self.search("bt")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn does_not_handle_regular_search() {
        let p = BluetoothProvider::new();
        assert!(p.search("firefox").is_empty());
    }

    #[test]
    fn parse_paired_handles_basic_lines() {
        let sample = "Device AA:BB:CC:DD:EE:FF JBL Tune\nDevice 11:22:33:44:55:66 Logitech MX Master 3\n";
        let parsed = parse_paired(sample);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].0, "AA:BB:CC:DD:EE:FF");
        assert_eq!(parsed[0].1, "JBL Tune");
        assert_eq!(parsed[1].1, "Logitech MX Master 3");
    }

    #[test]
    fn parse_paired_skips_garbage_lines() {
        let sample = "Garbage line\nDevice AA:BB:CC:DD:EE:FF Phone\n";
        let parsed = parse_paired(sample);
        assert_eq!(parsed.len(), 1);
    }
}
