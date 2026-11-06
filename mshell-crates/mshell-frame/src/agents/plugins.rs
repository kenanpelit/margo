//! Installed Claude Code plugins and bare skills, READ-ONLY.
//!
//! Everything comes straight from each account's config dir, never from
//! the `claude plugin` CLI (which can install, update and remove):
//! - `plugins/installed_plugins.json`: id (`name@marketplace`), scope,
//!   version, install path (and the project of a project / local install);
//! - `plugins/known_marketplaces.json`: where each marketplace comes from
//!   and its local clone, whose `.claude-plugin/marketplace.json` catalog
//!   gives the newest version on offer;
//! - `settings.json` (`enabledPlugins`), or the project's
//!   `.claude/settings{,.local}.json` for a project / local install;
//! - the plugin's `.claude-plugin/plugin.json` (description, repository)
//!   and its `skills/**/SKILL.md` count;
//! - `skills/<name>/SKILL.md`: bare skill folders (claude.ai's own
//!   `skills/synced` bucket stays hidden, as Claude Code's /skills does).
//!
//! Changing any of it stays with `claude plugin` / `/plugin`.

use super::accounts::Account;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One installed plugin (one per install: a plugin installed for the
/// user and for a project is listed twice).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Plugin {
    pub(crate) name: String,
    pub(crate) marketplace: String,
    /// Account label; empty with only one account.
    pub(crate) account: String,
    pub(crate) version: String,
    /// `user`, `project` or `local`.
    pub(crate) scope: String,
    /// The project of a `project` / `local` install.
    pub(crate) project: Option<String>,
    /// `enabledPlugins` entry; `None` when the settings don't say.
    pub(crate) enabled: Option<bool>,
    /// Newer version the marketplace clone offers.
    pub(crate) update: Option<String>,
    pub(crate) skills: usize,
    pub(crate) description: String,
    /// http(s) page of the plugin (repository, homepage or marketplace).
    pub(crate) url: Option<String>,
}

/// One bare skill folder under `<config>/skills`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Skill {
    pub(crate) name: String,
    pub(crate) account: String,
    pub(crate) description: String,
    /// `SKILL.md`, or the folder when it has none.
    pub(crate) path: PathBuf,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Inventory {
    /// Sorted by marketplace, then name.
    pub(crate) plugins: Vec<Plugin>,
    /// Sorted by name.
    pub(crate) skills: Vec<Skill>,
}

/// Plugin / marketplace / skill folder names Claude Code accepts; also
/// keeps odd records (newlines, a leading `-`) out of the menu.
pub(crate) fn is_name(s: &str) -> bool {
    let mut chars = s.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// A plain http(s) URL (`git+` and `.git` dropped), else `None`: only
/// those are ever opened.
pub(crate) fn web_url(s: &str) -> Option<String> {
    let s = s.trim();
    let s = s.strip_prefix("git+").unwrap_or(s);
    let s = s.strip_suffix(".git").unwrap_or(s);
    let ok = (s.starts_with("https://") || s.starts_with("http://"))
        && s.len() > "https://".len()
        && !s.chars().any(|c| c.is_whitespace() || c.is_control());
    ok.then(|| s.to_string())
}

fn str_of<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

/// A marketplace from `known_marketplaces.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Marketplace {
    url: Option<String>,
    clone: Option<PathBuf>,
}

fn marketplace_url(source: &Value) -> Option<String> {
    match str_of(source, "source") {
        Some("github") => {
            let repo = str_of(source, "repo")?;
            let mut parts = repo.split('/');
            let ok = matches!((parts.next(), parts.next(), parts.next()), (Some(o), Some(r), None) if is_name(o) && is_name(r));
            ok.then(|| format!("https://github.com/{repo}"))
        }
        _ => str_of(source, "url").and_then(web_url),
    }
}

fn parse_marketplaces(text: &str) -> BTreeMap<String, Marketplace> {
    let Ok(Value::Object(map)) = serde_json::from_str::<Value>(text) else {
        return BTreeMap::new();
    };
    map.into_iter()
        .filter(|(name, _)| is_name(name))
        .map(|(name, m)| {
            let market = Marketplace {
                url: m.get("source").and_then(marketplace_url),
                clone: str_of(&m, "installLocation").map(PathBuf::from),
            };
            (name, market)
        })
        .collect()
}

/// One install record of `installed_plugins.json` (version 2: id → list).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Install {
    name: String,
    marketplace: String,
    scope: String,
    project: Option<String>,
    version: String,
    path: Option<PathBuf>,
}

fn parse_installed(text: &str) -> Vec<Install> {
    let Ok(v) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    let Some(map) = v.get("plugins").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (id, records) in map {
        let Some((name, marketplace)) = id.split_once('@') else {
            continue;
        };
        if !is_name(name) || !is_name(marketplace) {
            continue;
        }
        // Version 1 kept one object per id; version 2 a list of installs.
        let records: Vec<&Value> = match records {
            Value::Array(a) => a.iter().collect(),
            o @ Value::Object(_) => vec![o],
            _ => continue,
        };
        for r in records {
            let scope = str_of(r, "scope").filter(|s| is_name(s)).unwrap_or("user");
            out.push(Install {
                name: name.into(),
                marketplace: marketplace.into(),
                scope: scope.into(),
                project: str_of(r, "projectPath")
                    .filter(|p| Path::new(p).is_absolute())
                    .map(Into::into),
                version: str_of(r, "version").unwrap_or_default().into(),
                path: str_of(r, "installPath").map(PathBuf::from),
            });
        }
    }
    out
}

/// `enabledPlugins[id]` of one settings file.
fn enabled_in(settings: &str, id: &str) -> Option<bool> {
    serde_json::from_str::<Value>(settings)
        .ok()?
        .get("enabledPlugins")?
        .get(id)?
        .as_bool()
}

/// The marketplace catalog's entry for `name`.
fn catalog_entry(catalog: &Value, name: &str) -> Option<Value> {
    catalog
        .get("plugins")?
        .as_array()?
        .iter()
        .find(|e| str_of(e, "name") == Some(name))
        .cloned()
}

/// `./plugins/x` → the plugin's folder inside the marketplace clone.
fn local_source(entry: &Value, clone: &Path) -> Option<PathBuf> {
    let rel = str_of(entry, "source")?.strip_prefix("./")?;
    let rel = Path::new(rel.trim_end_matches('/'));
    rel.components()
        .all(|c| matches!(c, std::path::Component::Normal(_)))
        .then(|| clone.join(rel))
}

/// `description:` of a SKILL.md front matter (quoted, plain or a folded
/// `>` / `|` block), whitespace collapsed.
pub(crate) fn skill_description(text: &str) -> String {
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("---") {
        return String::new();
    }
    let mut out = String::new();
    let mut in_block = false;
    for line in lines {
        if line.trim() == "---" {
            break;
        }
        if in_block {
            if line.starts_with(' ') || line.starts_with('\t') || line.trim().is_empty() {
                out.push(' ');
                out.push_str(line.trim());
                continue;
            }
            break;
        }
        if let Some(v) = line.strip_prefix("description:") {
            let v = v.trim();
            if matches!(v, "" | ">" | "|" | ">-" | "|-") {
                in_block = true;
                continue;
            }
            let v = v
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .or_else(|| v.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
                .unwrap_or(v);
            out = v.replace("\\\"", "\"");
            break;
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// SKILL.md files under `dir`, up to `depth` levels (vendored
/// `node_modules` and dot dirs skipped).
fn count_skills(dir: &Path, depth: u8) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut n = 0;
    for e in entries.flatten() {
        let name = e.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || name == "node_modules" {
            continue;
        }
        let path = e.path();
        if !path.is_dir() {
            continue;
        }
        if path.join("SKILL.md").is_file() {
            n += 1;
        } else if depth > 1 {
            n += count_skills(&path, depth - 1);
        }
    }
    n
}

fn read(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&read(path)?).ok()
}

/// Everything installed for one account (config dir `dir`).
fn account_inventory(dir: &Path, account: &str, out: &mut Inventory) {
    let plugins_dir = dir.join("plugins");
    let markets = read(&plugins_dir.join("known_marketplaces.json"))
        .map(|t| parse_marketplaces(&t))
        .unwrap_or_default();
    let user_settings = read(&dir.join("settings.json")).unwrap_or_default();
    let mut catalogs: BTreeMap<String, Value> = BTreeMap::new();
    let installs = read(&plugins_dir.join("installed_plugins.json"))
        .map(|t| parse_installed(&t))
        .unwrap_or_default();
    for i in installs {
        let market = markets.get(&i.marketplace).cloned().unwrap_or_default();
        let catalog = catalogs.entry(i.marketplace.clone()).or_insert_with(|| {
            market
                .clone
                .as_deref()
                .and_then(|c| read_json(&c.join(".claude-plugin/marketplace.json")))
                .unwrap_or(Value::Null)
        });
        let entry = catalog_entry(catalog, &i.name);
        let manifest = i
            .path
            .as_deref()
            .and_then(|p| read_json(&p.join(".claude-plugin/plugin.json")));
        // Newest version on offer: the catalog's own, else the plugin.json
        // in the clone (a plugin living in another repo has neither).
        let latest = entry.as_ref().and_then(|e| {
            str_of(e, "version").map(str::to_string).or_else(|| {
                let dir = local_source(e, market.clone.as_deref()?)?;
                let m = read_json(&dir.join(".claude-plugin/plugin.json"))?;
                str_of(&m, "version").map(str::to_string)
            })
        });
        let id = format!("{}@{}", i.name, i.marketplace);
        let enabled = match (i.scope.as_str(), i.project.as_deref()) {
            ("project", Some(p)) => {
                read(&Path::new(p).join(".claude/settings.json")).and_then(|s| enabled_in(&s, &id))
            }
            ("local", Some(p)) => read(&Path::new(p).join(".claude/settings.local.json"))
                .and_then(|s| enabled_in(&s, &id)),
            _ => enabled_in(&user_settings, &id),
        };
        let url = manifest
            .as_ref()
            .and_then(|m| {
                let repo = m.get("repository")?;
                repo.as_str()
                    .or_else(|| str_of(repo, "url"))
                    .and_then(web_url)
            })
            .or_else(|| {
                manifest
                    .as_ref()
                    .and_then(|m| str_of(m, "homepage"))
                    .and_then(web_url)
            })
            .or_else(|| {
                entry
                    .as_ref()
                    .and_then(|e| str_of(e, "homepage"))
                    .and_then(web_url)
            })
            .or_else(|| {
                entry
                    .as_ref()
                    .and_then(|e| e.get("source"))
                    .and_then(marketplace_url)
            })
            .or(market.url);
        let description = manifest
            .as_ref()
            .and_then(|m| str_of(m, "description"))
            .or_else(|| entry.as_ref().and_then(|e| str_of(e, "description")))
            .unwrap_or_default()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        out.plugins.push(Plugin {
            update: latest.filter(|l| !l.is_empty() && *l != i.version),
            skills: i
                .path
                .as_deref()
                .map_or(0, |p| count_skills(&p.join("skills"), 3)),
            name: i.name,
            marketplace: i.marketplace,
            account: account.into(),
            version: i.version,
            scope: i.scope,
            project: i.project,
            enabled,
            description,
            url,
        });
    }

    let Ok(entries) = std::fs::read_dir(dir.join("skills")) else {
        return;
    };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name == "synced" || !is_name(&name) || !e.path().is_dir() {
            continue;
        }
        let md = e.path().join("SKILL.md");
        let (description, path) = match read(&md) {
            Some(t) => (skill_description(&t), md),
            None => (String::new(), e.path()),
        };
        out.skills.push(Skill {
            name,
            account: account.into(),
            description,
            path,
        });
    }
}

/// Plugins and bare skills of every account.
pub(crate) fn inventory(accounts: &[Account]) -> Inventory {
    let mut out = Inventory::default();
    let multi = accounts.len() > 1;
    for a in accounts {
        let label = if multi { a.label.as_str() } else { "" };
        account_inventory(&a.dir, label, &mut out);
    }
    out.plugins.sort_by(|a, b| {
        (&a.marketplace, &a.name, &a.account, &a.scope).cmp(&(
            &b.marketplace,
            &b.name,
            &b.account,
            &b.scope,
        ))
    });
    out.skills
        .sort_by(|a, b| (&a.name, &a.account).cmp(&(&b.name, &b.account)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Captured from this machine (home path shortened), plus a project
    /// install and a disabled plugin.
    const INSTALLED: &str = r#"{
 "version": 2,
 "plugins": {
  "rust-analyzer-lsp@claude-plugins-official": [
   {"scope": "user", "installPath": "@C/rust-analyzer-lsp/1.0.0", "version": "1.0.0",
    "installedAt": "2026-05-02T18:23:02.937Z", "lastUpdated": "2026-05-02T18:23:02.937Z"}
  ],
  "superpowers@claude-plugins-official": [
   {"scope": "user", "installPath": "@C/superpowers/7.0.0", "version": "7.0.0",
    "gitCommitSha": "bb92a77741419a4ab5f06e711a283343f1ada0c3"},
   {"scope": "project", "projectPath": "@P", "installPath": "@C/superpowers/7.0.0", "version": "7.0.0"}
  ],
  "claude-code-setup@claude-plugins-official": [
   {"scope": "user", "installPath": "@C/claude-code-setup/1.0.0", "version": "1.0.0"}
  ],
  "bad name@x": [{"scope": "user"}],
  "-flag@x": [{"scope": "user"}],
  "noat": [{"scope": "user"}]
 }
}"#;

    const MARKETS: &str = r#"{
 "claude-plugins-official": {
  "source": {"source": "github", "repo": "anthropics/claude-plugins-official"},
  "installLocation": "@M",
  "lastUpdated": "2026-11-05T20:06:11.249Z"
 },
 "evil": {"source": {"source": "github", "repo": "a/b/../c"}}
}"#;

    /// Real entries of the official marketplace.json (trimmed).
    const CATALOG: &str = r#"{"name": "claude-plugins-official", "plugins": [
 {"name": "claude-code-setup", "description": "Analyze codebases and recommend tailored Claude Code automations.", "source": "./plugins/claude-code-setup", "homepage": "https://github.com/anthropics/claude-plugins-official/tree/main/plugins/claude-code-setup"},
 {"name": "rust-analyzer-lsp", "description": "Rust language server for code intelligence and analysis", "version": "1.1.0", "source": "./plugins/rust-analyzer-lsp"},
 {"name": "superpowers", "description": "Superpowers teaches Claude brainstorming.", "source": {"source": "url", "url": "https://github.com/obra/superpowers.git", "sha": "bb92a77"}, "homepage": "https://github.com/obra/superpowers.git"}
]}"#;

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn fixture() -> (tempfile::TempDir, PathBuf) {
        let t = tempfile::tempdir().unwrap();
        let root = t.path();
        let cfg = root.join("claude");
        let cache = cfg.join("plugins/cache/claude-plugins-official");
        let market = cfg.join("plugins/marketplaces/claude-plugins-official");
        let project = root.join("proj");
        let sub = |s: &str| {
            s.replace("@C", cache.to_str().unwrap())
                .replace("@M", market.to_str().unwrap())
                .replace("@P", project.to_str().unwrap())
        };
        write(&cfg.join("plugins/installed_plugins.json"), &sub(INSTALLED));
        write(&cfg.join("plugins/known_marketplaces.json"), &sub(MARKETS));
        write(&market.join(".claude-plugin/marketplace.json"), CATALOG);
        // The clone's claude-code-setup is newer than the installed one.
        write(
            &market.join("plugins/claude-code-setup/.claude-plugin/plugin.json"),
            r#"{"name": "claude-code-setup", "version": "1.2.0"}"#,
        );
        write(
            &cfg.join("settings.json"),
            r#"{"enabledPlugins": {"rust-analyzer-lsp@claude-plugins-official": true,
                "superpowers@claude-plugins-official": true,
                "claude-code-setup@claude-plugins-official": false}}"#,
        );
        write(
            &project.join(".claude/settings.json"),
            r#"{"enabledPlugins": {"superpowers@claude-plugins-official": false}}"#,
        );
        let sp = cache.join("superpowers/7.0.0");
        write(
            &sp.join(".claude-plugin/plugin.json"),
            r#"{"name": "superpowers", "description": "Core skills library for Claude Code:\n TDD, debugging", "version": "7.0.0", "repository": "https://github.com/obra/superpowers"}"#,
        );
        for s in ["brainstorming", "systematic-debugging"] {
            write(&sp.join(format!("skills/{s}/SKILL.md")), "---\n---\n");
        }
        // Nested like mattpocock-skills; vendored ones never count.
        write(&sp.join("skills/engineering/tdd/SKILL.md"), "");
        write(&sp.join("skills/node_modules/x/SKILL.md"), "");
        write(
            &cfg.join("skills/graphify/SKILL.md"),
            "---\nname: graphify\ndescription: \"Use for any question about a codebase — a \\\"graph\\\".\"\n---\n# /graphify\n",
        );
        write(
            &cfg.join("skills/gstack/SKILL.md"),
            "---\nname: gstack\npreamble-tier: 1\nversion: 1.2.0\ndescription: Router for the gstack skill suite. (gstack)\n---\n",
        );
        write(&cfg.join("skills/synced/x/SKILL.md"), "---\n---\n");
        std::fs::create_dir_all(cfg.join("skills/empty")).unwrap();
        (t, cfg)
    }

    #[test]
    fn lists_plugins_with_scope_state_updates_and_links() {
        let (_t, cfg) = fixture();
        let mut inv = Inventory::default();
        account_inventory(&cfg, "", &mut inv);
        inv.plugins
            .sort_by(|a, b| (&a.name, &a.scope).cmp(&(&b.name, &b.scope)));
        let got: Vec<_> = inv
            .plugins
            .iter()
            .map(|p| {
                (
                    p.name.as_str(),
                    p.scope.as_str(),
                    p.enabled,
                    p.update.as_deref(),
                    p.skills,
                    p.url.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            got,
            [
                (
                    "claude-code-setup",
                    "user",
                    Some(false),
                    Some("1.2.0"),
                    0,
                    Some(
                        "https://github.com/anthropics/claude-plugins-official/tree/main/plugins/claude-code-setup"
                    )
                ),
                (
                    "rust-analyzer-lsp",
                    "user",
                    Some(true),
                    Some("1.1.0"),
                    0,
                    Some("https://github.com/anthropics/claude-plugins-official")
                ),
                (
                    "superpowers",
                    "project",
                    Some(false),
                    None,
                    3,
                    Some("https://github.com/obra/superpowers")
                ),
                (
                    "superpowers",
                    "user",
                    Some(true),
                    None,
                    3,
                    Some("https://github.com/obra/superpowers")
                ),
            ]
        );
        let sp = &inv.plugins[3];
        assert_eq!(
            sp.description,
            "Core skills library for Claude Code: TDD, debugging"
        );
        assert_eq!(sp.marketplace, "claude-plugins-official");
        assert!(
            inv.plugins[2]
                .project
                .as_deref()
                .is_some_and(|p| p.ends_with("proj"))
        );
        assert_eq!(
            inv.plugins[0].description,
            "Analyze codebases and recommend tailored Claude Code automations."
        );
    }

    #[test]
    fn lists_bare_skills_but_not_the_synced_bucket() {
        let (_t, cfg) = fixture();
        let mut inv = Inventory::default();
        account_inventory(&cfg, "work", &mut inv);
        inv.skills.sort_by(|a, b| a.name.cmp(&b.name));
        let got: Vec<_> = inv
            .skills
            .iter()
            .map(|s| (s.name.as_str(), s.description.as_str(), s.account.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                ("empty", "", "work"),
                (
                    "graphify",
                    "Use for any question about a codebase — a \"graph\".",
                    "work"
                ),
                (
                    "gstack",
                    "Router for the gstack skill suite. (gstack)",
                    "work"
                ),
            ]
        );
        assert!(inv.skills[0].path.ends_with("skills/empty"));
        assert!(inv.skills[1].path.ends_with("graphify/SKILL.md"));
    }

    #[test]
    fn a_missing_config_dir_is_simply_empty() {
        let t = tempfile::tempdir().unwrap();
        let mut inv = Inventory::default();
        account_inventory(&t.path().join("nope"), "", &mut inv);
        assert_eq!(inv, Inventory::default());
        assert!(parse_installed("not json").is_empty());
        assert!(parse_installed(r#"{"plugins": 3}"#).is_empty());
        assert!(parse_marketplaces("[]").is_empty());
    }

    #[test]
    fn version_one_records_and_odd_ids() {
        let v = parse_installed(
            r#"{"version": 1, "plugins": {"a@m": {"version": "2", "installPath": "/x"},
                "b@m": [{"scope": "local", "projectPath": "rel"}], "c@": [], "@m": []}}"#,
        );
        assert_eq!(v.len(), 2);
        assert_eq!((v[0].name.as_str(), v[0].scope.as_str()), ("a", "user"));
        assert_eq!(
            (v[1].scope.as_str(), v[1].project.as_deref()),
            ("local", None)
        );
    }

    #[test]
    fn marketplace_sources() {
        let m = parse_marketplaces(MARKETS);
        assert_eq!(
            m["claude-plugins-official"].url.as_deref(),
            Some("https://github.com/anthropics/claude-plugins-official")
        );
        assert_eq!(m["evil"].url, None);
        let url = serde_json::json!({"source": "git", "url": "git+https://example.com/x.git"});
        assert_eq!(
            marketplace_url(&url).as_deref(),
            Some("https://example.com/x")
        );
        let local = serde_json::json!({"source": "directory", "path": "/home/x"});
        assert_eq!(marketplace_url(&local), None);
    }

    #[test]
    fn only_plain_web_urls_open() {
        assert_eq!(
            web_url("https://github.com/obra/superpowers.git").as_deref(),
            Some("https://github.com/obra/superpowers")
        );
        for bad in [
            "file:///etc/passwd",
            "javascript:x",
            "https://",
            "https://a b",
            "-x",
        ] {
            assert_eq!(web_url(bad), None, "{bad}");
        }
    }

    #[test]
    fn names_and_traversal() {
        assert!(is_name("rust-analyzer-lsp") && is_name("a.b_c") && is_name("_x"));
        for bad in ["", "-x", ".x", "a b", "a/b", "a\nb"] {
            assert!(!is_name(bad), "{bad:?}");
        }
        let clone = Path::new("/m");
        let e = serde_json::json!({"source": "./plugins/x/"});
        assert_eq!(local_source(&e, clone), Some(PathBuf::from("/m/plugins/x")));
        let e = serde_json::json!({"source": "./../etc"});
        assert_eq!(local_source(&e, clone), None);
    }

    #[test]
    fn folded_and_plain_skill_descriptions() {
        assert_eq!(
            skill_description("---\nname: x\ndescription: >\n  Two\n  lines.\nother: 1\n---\n"),
            "Two lines."
        );
        assert_eq!(
            skill_description("---\ndescription: 'Single'\n---\n"),
            "Single"
        );
        assert_eq!(skill_description("# no front matter\ndescription: x\n"), "");
        assert_eq!(
            skill_description("---\nname: x\n---\ndescription: body\n"),
            ""
        );
    }
}
