//! GitHub: the notification inbox and the user's open pull requests,
//! through the `gh` CLI and its saved login (no token handling here — `gh`
//! keeps it in the keyring).
//!
//! - Inbox: `gh api notifications` (unread threads), mark one read with
//!   `PATCH notifications/threads/<id>`, all with `PUT notifications`.
//! - My PRs: one `gh api graphql` search (`is:pr is:open author:@me`) with
//!   each PR's review decision and the CI rollup of its last commit.
//!
//! `gh` may be missing or logged out; both are reported, never fatal.

use serde_json::Value;

/// The poll cadence: GitHub's `X-Poll-Interval` for notifications is 60 s;
/// two minutes keeps well inside it.
pub(crate) const POLL_SECS: u64 = 120;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GhError {
    /// `gh` is not installed.
    Missing,
    /// `gh` is installed but not logged in.
    LoggedOut,
    Failed(String),
}

impl GhError {
    pub(crate) fn message(&self) -> String {
        match self {
            GhError::Missing => {
                "The GitHub CLI (gh) is not installed — install the github-cli package".into()
            }
            GhError::LoggedOut => "gh is not logged in — run `gh auth login`".into(),
            GhError::Failed(e) => format!("gh failed: {e}"),
        }
    }

    /// Classify a failed `gh` run from its stderr.
    pub(crate) fn from_stderr(stderr: &str) -> Self {
        let s = stderr.to_ascii_lowercase();
        if s.contains("gh auth login") || s.contains("not logged") || s.contains("authentication") {
            GhError::LoggedOut
        } else {
            GhError::Failed(stderr.lines().next().unwrap_or("").trim().to_string())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Notification {
    /// Thread id (digits only — validated, it goes into an API path).
    pub(crate) id: String,
    pub(crate) unread: bool,
    pub(crate) reason: String,
    pub(crate) title: String,
    /// `PullRequest`, `Issue`, `Release`, `CheckSuite`, …
    pub(crate) kind: String,
    pub(crate) repo: String,
    pub(crate) updated_ms: i64,
    /// Browser URL for the thread's subject.
    pub(crate) url: String,
}

impl Notification {
    pub(crate) fn icon(&self) -> &'static str {
        match self.kind.as_str() {
            "PullRequest" => "vcs-pull-request-symbolic",
            "Issue" => "dialog-information-symbolic",
            "Release" => "software-update-available-symbolic",
            "CheckSuite" | "WorkflowRun" => "system-run-symbolic",
            "Discussion" => "chat-message-new-symbolic",
            "RepositoryVulnerabilityAlert" => "dialog-warning-symbolic",
            _ => "notification-symbolic",
        }
    }
}

/// `review_requested` → `Review requested`.
pub(crate) fn reason_label(reason: &str) -> String {
    match reason {
        "ci_activity" => "CI activity".into(),
        "review_requested" => "Review requested".into(),
        "security_alert" => "Security alert".into(),
        "state_change" => "State changed".into(),
        "assign" => "Assigned".into(),
        "author" => "Your thread".into(),
        "mention" => "Mentioned".into(),
        "team_mention" => "Team mentioned".into(),
        "comment" => "Comment".into(),
        "subscribed" => "Watching".into(),
        other => {
            let s = other.replace('_', " ");
            let mut c = s.chars();
            c.next()
                .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                .unwrap_or_default()
        }
    }
}

fn valid_segment(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// The browser URL for a notification subject. `subject.url` is an API
/// URL (`https://api.github.com/repos/o/r/pulls/94`); anything that doesn't
/// map lands on the repository (or its Actions tab for CI threads), and
/// failing that on the inbox.
pub(crate) fn html_url(subject_url: Option<&str>, kind: &str, repo_html: Option<&str>) -> String {
    if let Some(rest) = subject_url.and_then(|u| u.strip_prefix("https://api.github.com/repos/")) {
        let parts: Vec<&str> = rest.split('/').collect();
        if let [owner, repo, what, id] = parts[..]
            && valid_segment(owner)
            && valid_segment(repo)
            && valid_segment(id)
        {
            let path = match what {
                "pulls" => Some(format!("pull/{id}")),
                "issues" => Some(format!("issues/{id}")),
                "discussions" => Some(format!("discussions/{id}")),
                "commits" => Some(format!("commit/{id}")),
                "releases" => Some("releases".to_string()),
                _ => None,
            };
            if let Some(path) = path {
                return format!("https://github.com/{owner}/{repo}/{path}");
            }
        }
    }
    if let Some(repo) = repo_html.filter(|u| is_github_url(u)) {
        if kind == "CheckSuite" || kind == "WorkflowRun" {
            return format!("{repo}/actions");
        }
        return repo.to_string();
    }
    "https://github.com/notifications".into()
}

/// Only `https://github.com/…` URLs are handed to `xdg-open`.
pub(crate) fn is_github_url(url: &str) -> bool {
    url.strip_prefix("https://github.com/").is_some_and(|rest| {
        !rest.starts_with('/')
            && rest
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_./?=&#%".contains(&b))
    })
}

fn iso_ms(v: Option<&Value>) -> i64 {
    v.and_then(Value::as_str)
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map_or(0, |d| d.timestamp_millis())
}

/// Parse a `GET /notifications` body. Malformed threads are dropped.
pub(crate) fn parse_notifications(body: &str) -> Result<Vec<Notification>, GhError> {
    let v: Value = serde_json::from_str(body).map_err(|e| GhError::Failed(e.to_string()))?;
    let arr = v
        .as_array()
        .ok_or_else(|| GhError::Failed("unexpected notifications response".into()))?;
    let mut out: Vec<Notification> = arr
        .iter()
        .filter_map(|n| {
            let id = n.get("id").and_then(Value::as_str)?;
            if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let subject = n.get("subject");
            let kind = subject
                .and_then(|s| s.get("type"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let repo = n.get("repository");
            Some(Notification {
                id: id.to_string(),
                unread: n.get("unread").and_then(Value::as_bool).unwrap_or(false),
                reason: n
                    .get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                title: subject
                    .and_then(|s| s.get("title"))
                    .and_then(Value::as_str)
                    .unwrap_or("Untitled")
                    .to_string(),
                repo: repo
                    .and_then(|r| r.get("full_name"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                updated_ms: iso_ms(n.get("updated_at")),
                url: html_url(
                    subject.and_then(|s| s.get("url")).and_then(Value::as_str),
                    &kind,
                    repo.and_then(|r| r.get("html_url")).and_then(Value::as_str),
                ),
                kind,
            })
        })
        .collect();
    out.sort_by_key(|e| std::cmp::Reverse(e.updated_ms));
    Ok(out)
}

// ── Pull requests ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CiState {
    None,
    Pending,
    Success,
    Failure,
}

impl CiState {
    fn parse(s: Option<&str>) -> Self {
        match s {
            Some("SUCCESS") => CiState::Success,
            Some("FAILURE" | "ERROR") => CiState::Failure,
            Some("PENDING" | "EXPECTED") => CiState::Pending,
            _ => CiState::None,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            CiState::None => "",
            CiState::Pending => "CI running",
            CiState::Success => "CI passed",
            CiState::Failure => "CI failing",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Review {
    None,
    Required,
    Approved,
    ChangesRequested,
}

impl Review {
    fn parse(s: Option<&str>) -> Self {
        match s {
            Some("APPROVED") => Review::Approved,
            Some("CHANGES_REQUESTED") => Review::ChangesRequested,
            Some("REVIEW_REQUIRED") => Review::Required,
            _ => Review::None,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Review::None => "",
            Review::Required => "Review required",
            Review::Approved => "Approved",
            Review::ChangesRequested => "Changes requested",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PullRequest {
    pub(crate) number: u64,
    pub(crate) title: String,
    pub(crate) url: String,
    pub(crate) repo: String,
    pub(crate) draft: bool,
    pub(crate) review: Review,
    pub(crate) ci: CiState,
    pub(crate) updated_ms: i64,
}

/// The GraphQL search for the user's open PRs.
pub(crate) const PR_QUERY: &str = "query($q: String!) { search(query: $q, type: ISSUE, first: 50) { nodes { ... on PullRequest { number title url isDraft reviewDecision updatedAt repository { nameWithOwner } commits(last: 1) { nodes { commit { statusCheckRollup { state } } } } } } } }";
pub(crate) const PR_SEARCH: &str = "is:pr is:open author:@me archived:false";

/// Parse the `gh api graphql` response for [`PR_QUERY`].
pub(crate) fn parse_prs(body: &str) -> Result<Vec<PullRequest>, GhError> {
    let v: Value = serde_json::from_str(body).map_err(|e| GhError::Failed(e.to_string()))?;
    if let Some(msg) = v.pointer("/errors/0/message").and_then(Value::as_str) {
        return Err(GhError::Failed(msg.to_string()));
    }
    let nodes = v
        .pointer("/data/search/nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| GhError::Failed("unexpected search response".into()))?;
    let mut out: Vec<PullRequest> = nodes
        .iter()
        .filter_map(|n| {
            let url = n.get("url").and_then(Value::as_str)?;
            Some(PullRequest {
                number: n.get("number").and_then(Value::as_u64).unwrap_or(0),
                title: n
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                url: url.to_string(),
                repo: n
                    .pointer("/repository/nameWithOwner")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                draft: n.get("isDraft").and_then(Value::as_bool).unwrap_or(false),
                review: Review::parse(n.get("reviewDecision").and_then(Value::as_str)),
                ci: CiState::parse(
                    n.pointer("/commits/nodes/0/commit/statusCheckRollup/state")
                        .and_then(Value::as_str),
                ),
                updated_ms: iso_ms(n.get("updatedAt")),
            })
        })
        .collect();
    out.sort_by_key(|e| std::cmp::Reverse(e.updated_ms));
    Ok(out)
}

// ── gh calls ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GithubSnapshot {
    pub(crate) notifications: Result<Vec<Notification>, GhError>,
    pub(crate) prs: Result<Vec<PullRequest>, GhError>,
}

impl GithubSnapshot {
    pub(crate) fn unread(&self) -> usize {
        self.notifications
            .as_ref()
            .map_or(0, |n| n.iter().filter(|n| n.unread).count())
    }
}

async fn gh(args: &[&str]) -> Result<String, GhError> {
    let out = tokio::process::Command::new("gh")
        .args(args)
        .env("GH_PROMPT_DISABLED", "1")
        .env("NO_COLOR", "1")
        .output()
        .await
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                GhError::Missing
            } else {
                GhError::Failed(e.to_string())
            }
        })?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(GhError::from_stderr(&String::from_utf8_lossy(&out.stderr)))
    }
}

pub(crate) async fn fetch() -> GithubSnapshot {
    let notifications = match gh(&["api", "notifications?per_page=50"]).await {
        Ok(body) => parse_notifications(&body),
        Err(e) => Err(e),
    };
    // Don't spawn a second doomed call when gh is missing / logged out.
    let prs = match &notifications {
        Err(e @ (GhError::Missing | GhError::LoggedOut)) => Err(e.clone()),
        _ => match gh(&[
            "api",
            "graphql",
            "-f",
            &format!("q={PR_SEARCH}"),
            "-f",
            &format!("query={PR_QUERY}"),
        ])
        .await
        {
            Ok(body) => parse_prs(&body),
            Err(e) => Err(e),
        },
    };
    GithubSnapshot { notifications, prs }
}

/// Mark one thread read. `id` must be the digits-only thread id.
pub(crate) async fn mark_read(id: &str) -> Result<(), GhError> {
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return Err(GhError::Failed("bad thread id".into()));
    }
    gh(&[
        "api",
        "--method",
        "PATCH",
        &format!("notifications/threads/{id}"),
    ])
    .await
    .map(|_| ())
}

/// Mark every notification read.
pub(crate) async fn mark_all_read() -> Result<(), GhError> {
    gh(&["api", "--method", "PUT", "notifications", "-F", "read=true"])
        .await
        .map(|_| ())
}

/// "5m", "3h", "2d" ago.
pub(crate) fn ago(now_ms: i64, then_ms: i64) -> String {
    let m = ((now_ms - then_ms) / 60_000).max(0);
    if m < 60 {
        format!("{m}m")
    } else if m < 1440 {
        format!("{}h", m / 60)
    } else {
        format!("{}d", m / 1440)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real `gh api 'notifications?all=true'` threads from this machine,
    /// trimmed to the fields GitHub sends that the parser reads, plus an
    /// unread one.
    const NOTIFICATIONS: &str = r#"[
      {"id":"25499561872","unread":false,"reason":"state_change","updated_at":"2026-09-07T14:32:49Z",
       "subject":{"title":"Mosaic layout","url":"https://api.github.com/repos/kenanpelit/margo/issues/2","type":"Issue"},
       "repository":{"full_name":"kenanpelit/margo","html_url":"https://github.com/kenanpelit/margo"}},
      {"id":"24246896453","unread":true,"reason":"author","updated_at":"2026-08-07T17:47:17Z",
       "subject":{"title":"Add margo compositor","url":"https://api.github.com/repos/rcalixte/awesome-wayland/pulls/94","type":"PullRequest"},
       "repository":{"full_name":"rcalixte/awesome-wayland","html_url":"https://github.com/rcalixte/awesome-wayland"}},
      {"id":"1","unread":true,"reason":"ci_activity","updated_at":"2026-09-08T10:00:00Z",
       "subject":{"title":"CI failed","url":null,"type":"CheckSuite"},
       "repository":{"full_name":"kenanpelit/margo","html_url":"https://github.com/kenanpelit/margo"}},
      {"id":"../evil","unread":true,"subject":{"title":"x"}}
    ]"#;

    #[test]
    fn notifications_map_to_browser_urls_newest_first() {
        let n = parse_notifications(NOTIFICATIONS).unwrap();
        assert_eq!(n.len(), 3, "the non-numeric id is dropped");
        assert_eq!(n[0].url, "https://github.com/kenanpelit/margo/actions");
        assert_eq!(n[1].url, "https://github.com/kenanpelit/margo/issues/2");
        assert_eq!(
            n[2].url,
            "https://github.com/rcalixte/awesome-wayland/pull/94"
        );
        assert_eq!(n[2].kind, "PullRequest");
        assert!(n[2].unread && !n[1].unread);
        let snap = GithubSnapshot {
            notifications: Ok(n),
            prs: Ok(vec![]),
        };
        assert_eq!(snap.unread(), 2);
        assert_eq!(parse_notifications("[]").unwrap(), vec![]);
        assert!(parse_notifications(r#"{"message":"Bad credentials"}"#).is_err());
    }

    #[test]
    fn html_url_fallbacks() {
        assert_eq!(
            html_url(
                Some("https://api.github.com/repos/o/r/releases/123"),
                "Release",
                None
            ),
            "https://github.com/o/r/releases"
        );
        assert_eq!(
            html_url(
                Some("https://api.github.com/repos/o/r/commits/abc123"),
                "Commit",
                None
            ),
            "https://github.com/o/r/commit/abc123"
        );
        assert_eq!(
            html_url(None, "Issue", Some("https://evil.example/x")),
            "https://github.com/notifications"
        );
        assert!(is_github_url("https://github.com/o/r/pull/1"));
        assert!(!is_github_url("https://github.com.evil/x"));
        assert!(!is_github_url("http://github.com/o"));
        assert!(!is_github_url("https://github.com/o r"));
    }

    #[test]
    fn reasons_read_as_words() {
        assert_eq!(reason_label("review_requested"), "Review requested");
        assert_eq!(reason_label("new_reason_kind"), "New reason kind");
    }

    #[test]
    fn prs_with_ci_and_review_state() {
        // Real `gh api graphql` response for PR_QUERY on this machine
        // (searching closed PRs too, since the account has no open one),
        // with a third PR showing the populated states.
        let body = r#"{"data":{"search":{"issueCount":3,"nodes":[
          {"number":94,"title":"Add margo compositor","url":"https://github.com/rcalixte/awesome-wayland/pull/94","isDraft":true,"state":"CLOSED","reviewDecision":null,"updatedAt":"2026-08-07T17:47:14Z","repository":{"nameWithOwner":"rcalixte/awesome-wayland"},"author":{"login":"kenanpelit"},"commits":{"nodes":[{"commit":{"statusCheckRollup":null}}]}},
          {"number":1,"title":"feat(transmission): Add transmission torrent client configuration","url":"https://github.com/kenanpelit/nixosc/pull/1","isDraft":false,"state":"MERGED","reviewDecision":null,"updatedAt":"2025-01-15T14:57:14Z","repository":{"nameWithOwner":"kenanpelit/nixosc"},"author":{"login":"kenanpelit"},"commits":{"nodes":[{"commit":{"statusCheckRollup":null}}]}},
          {"number":7,"title":"x","url":"https://github.com/o/r/pull/7","isDraft":false,"reviewDecision":"CHANGES_REQUESTED","updatedAt":"2026-09-01T00:00:00Z","repository":{"nameWithOwner":"o/r"},"commits":{"nodes":[{"commit":{"statusCheckRollup":{"state":"FAILURE"}}}]}},
          {}
        ]}}}"#;
        let prs = parse_prs(body).unwrap();
        assert_eq!(prs.len(), 3, "an empty node (non-PR hit) is dropped");
        assert_eq!(prs[0].number, 7);
        assert_eq!(
            (prs[0].ci, prs[0].review),
            (CiState::Failure, Review::ChangesRequested)
        );
        assert!(prs[1].draft);
        assert_eq!((prs[1].ci, prs[1].review), (CiState::None, Review::None));
        assert_eq!(prs[1].repo, "rcalixte/awesome-wayland");
        assert!(matches!(
            parse_prs(r#"{"errors":[{"message":"Bad credentials"}]}"#),
            Err(GhError::Failed(m)) if m == "Bad credentials"
        ));
    }

    #[test]
    fn gh_errors_are_classified() {
        assert_eq!(
            GhError::from_stderr("To get started with GitHub CLI, please run:  gh auth login\n"),
            GhError::LoggedOut
        );
        assert_eq!(
            GhError::from_stderr("HTTP 502: Bad Gateway\nmore"),
            GhError::Failed("HTTP 502: Bad Gateway".into())
        );
    }

    #[test]
    fn ago_text() {
        assert_eq!(ago(3_600_000, 0), "1h");
        assert_eq!(ago(0, 1000), "0m");
        assert_eq!(ago(3 * 86_400_000, 0), "3d");
    }
}
