// These public async fns (modify, up, down, delete, wifi_connect, import_vpn,
// get_field, list_connections) are not called yet — they will be consumed by
// the Network settings page and connection editor added in later tasks.
#![allow(dead_code)]

use std::process::Stdio;
use tokio::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnRow {
    pub name: String,
    pub uuid: String,
    pub kind: String,   // 802-3-ethernet, 802-11-wireless, vpn, wireguard, …
    pub device: String, // "" if not active
    pub active: bool,
}

/// Split one `nmcli -t` line on unescaped ':' and unescape `\:` and `\\`.
pub(crate) fn split_terse(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut cur = String::new();
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(&n) = chars.peek() {
                    cur.push(n);
                    chars.next();
                }
            }
            ':' => fields.push(std::mem::take(&mut cur)),
            other => cur.push(other),
        }
    }
    fields.push(cur);
    fields
}

pub(crate) fn parse_connections(out: &str) -> Vec<ConnRow> {
    out.lines()
        .filter(|l| !l.is_empty())
        .filter_map(|l| {
            let f = split_terse(l);
            if f.len() < 5 {
                return None;
            }
            Some(ConnRow {
                name: f[0].clone(),
                uuid: f[1].clone(),
                kind: f[2].clone(),
                device: f[3].clone(),
                active: f[4] == "yes",
            })
        })
        .collect()
}

/// Run nmcli with `LC_ALL=C` for stable parsing. Returns stdout on success,
/// Err(stderr) on non-zero exit (e.g. a polkit denial).
async fn run(args: &[&str]) -> Result<String, String> {
    let output = Command::new("nmcli")
        .env("LC_ALL", "C")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|e| format!("failed to spawn nmcli: {e}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}

pub async fn list_connections() -> Result<Vec<ConnRow>, String> {
    let out = run(&[
        "-t",
        "-f",
        "NAME,UUID,TYPE,DEVICE,ACTIVE",
        "connection",
        "show",
    ])
    .await?;
    Ok(parse_connections(&out))
}

pub async fn modify(uuid: &str, kv: &[(&str, &str)]) -> Result<(), String> {
    let mut args = vec!["connection", "modify", uuid];
    for (k, v) in kv {
        args.push(k);
        args.push(v);
    }
    run(&args).await.map(|_| ())
}

pub async fn up(uuid: &str) -> Result<(), String> {
    run(&["connection", "up", uuid]).await.map(|_| ())
}
pub async fn down(uuid: &str) -> Result<(), String> {
    run(&["connection", "down", uuid]).await.map(|_| ())
}
pub async fn delete(uuid: &str) -> Result<(), String> {
    run(&["connection", "delete", uuid]).await.map(|_| ())
}
pub async fn wifi_rescan() -> Result<(), String> {
    run(&["device", "wifi", "rescan"]).await.map(|_| ())
}

pub async fn wifi_connect(ssid: &str, password: Option<&str>) -> Result<(), String> {
    let mut args = vec!["device", "wifi", "connect", ssid];
    if let Some(p) = password {
        args.push("password");
        args.push(p);
    }
    run(&args).await.map(|_| ())
}

pub async fn import_vpn(path: &str, kind: &str) -> Result<(), String> {
    run(&["connection", "import", "type", kind, "file", path])
        .await
        .map(|_| ())
}

/// Read one `connection.*`/`ipv4.*`/`ipv6.*` field via terse single-field show.
pub async fn get_field(uuid: &str, field: &str) -> Result<String, String> {
    let out = run(&["-t", "-f", field, "connection", "show", uuid]).await?;
    // single-field terse output is `field:value`; take the value
    Ok(split_terse(out.trim())
        .into_iter()
        .nth(1)
        .unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_escaped_terse_fields() {
        // nmcli -t escapes ':' as '\:' and '\' as '\\'
        let line = r"Wired connection 1:abc-123:802-3-ethernet:eth0";
        assert_eq!(
            split_terse(line),
            vec!["Wired connection 1", "abc-123", "802-3-ethernet", "eth0"]
        );
    }

    #[test]
    fn unescapes_colons_in_field() {
        let line = r"My\:SSID:uuid:wifi";
        assert_eq!(split_terse(line), vec!["My:SSID", "uuid", "wifi"]);
    }

    #[test]
    fn parses_connection_rows() {
        let out = "Wired connection 1:abc-123:802-3-ethernet:eth0:yes\n\
                   home-wifi:def-456:802-11-wireless::no\n";
        let rows = parse_connections(out);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, "Wired connection 1");
        assert_eq!(rows[0].uuid, "abc-123");
        assert!(rows[0].active);
        assert_eq!(rows[1].name, "home-wifi");
        assert_eq!(rows[1].device, "");
        assert!(!rows[1].active);
    }
}
