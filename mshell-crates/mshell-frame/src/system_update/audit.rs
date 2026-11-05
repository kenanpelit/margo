//! Arch Security Team advisories via `arch-audit --json` (package
//! `arch-audit`). Read-only: it lists advisories affecting installed
//! packages; the fix is always a full `-Syu`, never a partial update.
//! When the binary is missing the panel shows an install hint and the
//! pill shows no badge.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) enum Severity {
    Unknown,
    Low,
    Medium,
    High,
    Critical,
}

impl Severity {
    fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "critical" => Severity::Critical,
            "high" => Severity::High,
            "medium" => Severity::Medium,
            "low" => Severity::Low,
            _ => Severity::Unknown,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Severity::Critical => "Critical",
            Severity::High => "High",
            Severity::Medium => "Medium",
            Severity::Low => "Low",
            Severity::Unknown => "Unknown",
        }
    }

    /// DESIGN.md §2 ladder class.
    pub(crate) fn css_class(self) -> &'static str {
        match self {
            Severity::Critical | Severity::High => "danger",
            Severity::Medium => "warn",
            Severity::Low | Severity::Unknown => "calm",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Advisory {
    /// Tracker group id (`AVG-2901`).
    pub(crate) id: String,
    pub(crate) packages: Vec<String>,
    pub(crate) severity: Severity,
    /// `arbitrary filesystem access`, `denial of service`, ...
    pub(crate) kind: String,
    /// Fixed version once the tracker has one.
    pub(crate) fixed: Option<String>,
    pub(crate) issues: Vec<String>,
}

impl Advisory {
    /// Only tracker-shaped ids become links.
    pub(crate) fn url(&self) -> Option<String> {
        let (prefix, num) = self.id.split_once('-')?;
        (prefix.chars().all(|c| c.is_ascii_uppercase())
            && !prefix.is_empty()
            && !num.is_empty()
            && num.chars().all(|c| c.is_ascii_digit()))
        .then(|| format!("https://security.archlinux.org/{}", self.id))
    }
}

#[derive(Deserialize)]
struct Raw {
    name: Option<String>,
    #[serde(default)]
    packages: Vec<String>,
    severity: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
    fixed: Option<String>,
    #[serde(default)]
    issues: Vec<String>,
}

/// Parse `arch-audit --json`, worst severity first. Entries without a
/// name or packages are dropped rather than failing the whole list.
pub(crate) fn parse(json: &str) -> Result<Vec<Advisory>, String> {
    let raw: Vec<serde_json::Value> =
        serde_json::from_str(json).map_err(|e| format!("arch-audit json: {e}"))?;
    let mut out: Vec<Advisory> = raw
        .into_iter()
        .filter_map(|v| serde_json::from_value::<Raw>(v).ok())
        .filter_map(|r| {
            let id = r.name.filter(|n| !n.is_empty())?;
            if r.packages.is_empty() {
                return None;
            }
            Some(Advisory {
                id,
                packages: r.packages,
                severity: Severity::parse(r.severity.as_deref().unwrap_or("")),
                kind: r.kind.unwrap_or_else(|| "unknown".into()),
                fixed: r.fixed.filter(|f| !f.is_empty()),
                issues: r.issues,
            })
        })
        .collect();
    out.sort_by(|a, b| {
        b.severity
            .cmp(&a.severity)
            .then_with(|| a.packages[0].cmp(&b.packages[0]))
            .then_with(|| a.id.cmp(&b.id))
    });
    out.dedup_by(|a, b| a.id == b.id);
    Ok(out)
}

/// Worst severity present (drives the pill badge tint).
pub(crate) fn worst(advisories: &[Advisory]) -> Option<Severity> {
    advisories.iter().map(|a| a.severity).max()
}

/// Result of an audit probe, cached with the update report.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum AuditState {
    /// Audit disabled in config.
    #[default]
    Off,
    /// `arch-audit` isn't installed.
    Missing,
    Failed(String),
    Ok(Vec<Advisory>),
}

impl AuditState {
    pub(crate) fn advisories(&self) -> &[Advisory] {
        match self {
            AuditState::Ok(v) => v,
            _ => &[],
        }
    }
}

pub(crate) async fn probe() -> AuditState {
    if !margo_fs::path::on_path("arch-audit") {
        return AuditState::Missing;
    }
    match tokio::process::Command::new("arch-audit")
        .arg("--json")
        .output()
        .await
    {
        Ok(o) if o.status.success() => match parse(&String::from_utf8_lossy(&o.stdout)) {
            Ok(v) => AuditState::Ok(v),
            Err(e) => AuditState::Failed(e),
        },
        Ok(o) => AuditState::Failed(format!(
            "arch-audit exit {}: {}",
            o.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&o.stderr).trim()
        )),
        Err(e) => AuditState::Failed(format!("arch-audit: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real `arch-audit --json` (arch-audit 0.2.0) from this machine.
    const REAL: &str = include_str!("testdata/arch-audit.json");

    #[test]
    fn parses_real_output() {
        let v = parse(REAL).unwrap();
        assert_eq!(v.len(), 24);
        // Worst first: the four High ones, ordered by package name.
        assert_eq!(v[0].severity, Severity::High);
        let highs: Vec<_> = v
            .iter()
            .filter(|a| a.severity == Severity::High)
            .map(|a| a.packages[0].as_str())
            .collect();
        assert!(highs.contains(&"pam") && highs.contains(&"libxml2") && highs.contains(&"grub"));
        assert!(highs.windows(2).all(|w| w[0] <= w[1]));
        assert_eq!(v.last().unwrap().severity, Severity::Unknown);
        let pam = v.iter().find(|a| a.id == "AVG-2901").unwrap();
        assert_eq!(pam.kind, "arbitrary filesystem access");
        assert_eq!(pam.issues, ["CVE-2025-6020"]);
        assert_eq!(pam.fixed, None);
        assert_eq!(
            pam.url().as_deref(),
            Some("https://security.archlinux.org/AVG-2901")
        );
        assert_eq!(worst(&v), Some(Severity::High));
    }

    #[test]
    fn tolerates_bad_entries_and_rejects_non_json() {
        let v = parse(
            r#"[{"name":"AVG-1","packages":["a"],"severity":"Critical","type":"x","fixed":"2-1","issues":[]},
                {"packages":["b"]}, {"name":"AVG-2","packages":[]}, 7,
                {"name":"AVG-1","packages":["a"],"severity":"Critical"}]"#,
        )
        .unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].fixed.as_deref(), Some("2-1"));
        assert_eq!(v[0].severity.css_class(), "danger");
        assert!(parse("arch-audit: error").is_err());
        assert!(parse("[]").unwrap().is_empty());
        assert_eq!(worst(&[]), None);
    }

    #[test]
    fn only_tracker_ids_link() {
        let mut a = parse(REAL).unwrap().remove(0);
        a.id = "javascript:alert(1)".into();
        assert_eq!(a.url(), None);
        a.id = "AVG-".into();
        assert_eq!(a.url(), None);
    }

    #[test]
    fn severity_ladder() {
        assert_eq!(Severity::Medium.css_class(), "warn");
        assert_eq!(Severity::Low.css_class(), "calm");
        assert!(Severity::Critical > Severity::High);
    }
}
