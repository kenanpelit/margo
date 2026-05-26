use std::process::Stdio;
use tokio::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermEntry {
    pub table: String,
    pub object: String,
    pub app: String,
    pub perms: String,
}

/// Parse `flatpak permissions` output. The header row (starts with "Table")
/// and blank lines are skipped. Columns are whitespace/tab separated.
pub(crate) fn parse_permissions(out: &str) -> Vec<PermEntry> {
    out.lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with("Table"))
        .filter_map(|l| {
            let cols: Vec<&str> = l.split_whitespace().collect();
            if cols.len() < 3 {
                return None;
            }
            Some(PermEntry {
                table: cols[0].to_string(),
                object: cols[1].to_string(),
                app: cols[2].to_string(),
                perms: cols.get(3).copied().unwrap_or("").to_string(),
            })
        })
        .collect()
}

async fn fp(args: &[&str]) -> Result<String, String> {
    let o = Command::new("flatpak")
        .env("LC_ALL", "C")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|e| format!("flatpak unavailable: {e}"))?;
    if o.status.success() {
        Ok(String::from_utf8_lossy(&o.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&o.stderr).trim().to_owned())
    }
}

/// True if the `flatpak` CLI exists.
pub async fn available() -> bool {
    Command::new("flatpak")
        .arg("--version")
        .stdin(Stdio::null())
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub async fn list() -> Result<Vec<PermEntry>, String> {
    Ok(parse_permissions(&fp(&["permissions"]).await?))
}

pub async fn revoke(table: &str, object: &str, app: &str) -> Result<(), String> {
    fp(&["permission-remove", table, object, app])
        .await
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_permissions_table() {
        let out = "Table\tObject\tApp\tPermissions\n\
                   devices\tcamera\torg.example.App\tyes\n\
                   location\tlocation\torg.foo.Bar\tEXACT,0\n";
        let rows = parse_permissions(out);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].table, "devices");
        assert_eq!(rows[0].object, "camera");
        assert_eq!(rows[0].app, "org.example.App");
        assert_eq!(rows[1].app, "org.foo.Bar");
    }
    #[test]
    fn skips_header_and_blanks() {
        let rows = parse_permissions("Table\tObject\tApp\tPermissions\n\n");
        assert!(rows.is_empty());
    }
}
