//! `webapp` — create, list and remove Chromium `--app` web apps.
//!
//! Port of noctalia's `webapp-maker` panel as a launcher action. A web app
//! is a `.desktop` entry whose `Exec` opens the site in a Chromium-family
//! browser's app mode (`--app=<url>`: own window, no tabs or URL bar), with
//! the site's favicon as its icon. It then shows up in the Apps provider,
//! the dock and any other menu like a normal application.
//!
//! | Query | Result |
//! |---|---|
//! | `webapp` | every web app created here — Enter launches, Delete removes |
//! | `webapp <Name> <url>` | one "Create" row per installed Chromium browser (Helium, Chrome, Chromium, Brave, …) |
//! | `webapp <url>` | same, named after the domain |
//!
//! Files: `~/.local/share/applications/margo-webapp-<slug>.desktop` and the
//! icon in `~/.local/share/margo/webapps/<slug>.png` (referenced by
//! absolute path, so no icon-theme cache needs refreshing). Entries carry
//! `X-Margo-WebApp=true`; listing and removal only ever touch files with
//! that marker, so a name clash can never delete a real application.

use crate::providers::tools::find_in_path;
use crate::providers::websearch::{download_png, favicon_url_sized, url_domain};
use crate::{item::LauncherItem, notify::toast, provider::Provider};
use mshell_process::SpawnReaped;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;

const NAME: &str = "Web apps";
const PREFIX: &str = "webapp";
const MARKER: &str = "X-Margo-WebApp";
const FILE_PREFIX: &str = "margo-webapp-";

/// Chromium-family browsers with an `--app` mode, in preference order
/// (binary, display name).
pub const BROWSERS: &[(&str, &str)] = &[
    ("helium-browser", "Helium"),
    ("helium", "Helium"),
    ("google-chrome-stable", "Chrome"),
    ("google-chrome", "Chrome"),
    ("chromium", "Chromium"),
    ("brave", "Brave"),
    ("brave-browser", "Brave"),
    ("vivaldi-stable", "Vivaldi"),
    ("microsoft-edge-stable", "Edge"),
];

/// Installed browsers: (absolute binary path, display name), one per
/// display name, preference order.
pub fn installed_browsers(
    lookup: impl Fn(&str) -> Option<PathBuf>,
) -> Vec<(PathBuf, &'static str)> {
    let mut out: Vec<(PathBuf, &'static str)> = Vec::new();
    for (bin, label) in BROWSERS {
        if out.iter().any(|(_, l)| l == label) {
            continue;
        }
        if let Some(path) = lookup(bin) {
            out.push((path, label));
        }
    }
    out
}

/// A parsed `webapp <Name> <url>` request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub name: String,
    pub url: String,
}

/// Parse the text after `webapp`: the last token is the URL (`https://`
/// assumed without a scheme), everything before it the name; a lone URL
/// is named after its domain. Only http(s) URLs without whitespace pass —
/// Chromium's `--app=` would run `javascript:`/`file:`/`data:` documents.
pub fn parse_request(rest: &str) -> Option<Request> {
    let rest = rest.trim();
    let (name, url) = match rest.rsplit_once(char::is_whitespace) {
        Some((n, u)) => (n.trim(), u.trim()),
        None => ("", rest),
    };
    let lower = url.to_ascii_lowercase();
    let url = if lower.starts_with("http://") || lower.starts_with("https://") {
        url.to_string()
    } else if url.contains("://") || url.contains(':') && !url.contains('.') {
        return None;
    } else {
        format!("https://{url}")
    };
    let domain = url_domain(&url)?;
    let name = if name.is_empty() {
        let d = domain.trim_start_matches("www.");
        let base = d.split('.').next().unwrap_or(d);
        let mut chars = base.chars();
        chars
            .next()
            .map(|c| c.to_uppercase().collect::<String>() + chars.as_str())
            .unwrap_or_default()
    } else {
        name.to_string()
    };
    if name.contains(['/', '\n', '\r']) {
        return None;
    }
    Some(Request { name, url })
}

/// File-name slug: lowercase alphanumerics, runs of anything else → `-`.
pub fn slugify(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() { "app".into() } else { out }
}

/// Escape a desktop-entry string value (`\`, newline, tab, CR).
fn escape_value(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\t', "\\t")
        .replace('\r', "\\r")
}

/// Quote one `Exec` argument per the Desktop Entry spec: double-quote it
/// when it holds a reserved character, escaping `"`, `` ` ``, `$` and `\`
/// inside; `%` becomes `%%` (field codes). The result is then a string
/// value, so [`escape_value`] applies on top.
pub fn exec_quote(arg: &str) -> String {
    let arg = arg.replace('%', "%%");
    let reserved = arg.chars().any(|c| {
        c.is_whitespace()
            || matches!(
                c,
                '"' | '\''
                    | '\\'
                    | '>'
                    | '<'
                    | '~'
                    | '|'
                    | '&'
                    | ';'
                    | '$'
                    | '*'
                    | '?'
                    | '#'
                    | '('
                    | ')'
                    | '`'
            )
    });
    if !reserved {
        return arg;
    }
    let mut out = String::from("\"");
    for c in arg.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// The `.desktop` contents for a web app.
pub fn desktop_entry(
    req: &Request,
    browser: &Path,
    browser_label: &str,
    icon: Option<&Path>,
) -> String {
    let exec = format!(
        "{} {}",
        exec_quote(&browser.to_string_lossy()),
        exec_quote(&format!("--app={}", req.url))
    );
    let icon = icon
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "web-browser".into());
    format!(
        "[Desktop Entry]\nType=Application\nVersion=1.0\nName={name}\nComment={comment}\nExec={exec}\nIcon={icon}\nTerminal=false\nCategories=Network;WebBrowser;\nStartupNotify=true\n{MARKER}=true\n{MARKER}-Url={url}\n{MARKER}-Browser={label}\n",
        name = escape_value(&req.name),
        comment = escape_value(&format!("Web app for {}", req.url)),
        exec = escape_value(&exec),
        icon = escape_value(&icon),
        url = escape_value(&req.url),
        label = escape_value(browser_label),
    )
}

/// A web app found on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebApp {
    pub name: String,
    pub url: String,
    pub browser: String,
    pub icon: String,
    pub desktop: PathBuf,
}

/// Parse a `.desktop` file created here; `None` without our marker.
pub fn parse_webapp(content: &str, desktop: PathBuf) -> Option<WebApp> {
    let mut in_main = false;
    let (mut name, mut url, mut browser, mut icon, mut marked) =
        (None, None, String::new(), String::new(), false);
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_main = line == "[Desktop Entry]";
            continue;
        }
        if !in_main {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        match k.trim() {
            "Name" => name = Some(v.trim().to_string()),
            "Icon" => icon = v.trim().to_string(),
            k if k == MARKER => marked = v.trim() == "true",
            k if k == format!("{MARKER}-Url") => url = Some(v.trim().to_string()),
            k if k == format!("{MARKER}-Browser") => browser = v.trim().to_string(),
            _ => {}
        }
    }
    marked.then_some(())?;
    Some(WebApp {
        name: name?,
        url: url?,
        browser,
        icon,
        desktop,
    })
}

fn applications_dir() -> PathBuf {
    margo_fs::xdg::data_home().join("applications")
}

fn icons_dir() -> PathBuf {
    margo_fs::xdg::data_home().join("margo").join("webapps")
}

/// Every web app under `dir` (sorted by name).
pub fn list_webapps(dir: &Path) -> Vec<WebApp> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<WebApp> = rd
        .flatten()
        .filter(|e| {
            let n = e.file_name();
            let n = n.to_string_lossy();
            n.starts_with(FILE_PREFIX) && n.ends_with(".desktop")
        })
        .filter_map(|e| {
            let content = std::fs::read_to_string(e.path()).ok()?;
            parse_webapp(&content, e.path())
        })
        .collect();
    out.sort_by_key(|a| a.name.to_lowercase());
    out
}

/// First free `margo-webapp-<slug>[-N].desktop` path in `dir`.
fn free_desktop_path(dir: &Path, slug: &str) -> PathBuf {
    let mut n = 1;
    loop {
        let name = if n == 1 {
            format!("{FILE_PREFIX}{slug}.desktop")
        } else {
            format!("{FILE_PREFIX}{slug}-{n}.desktop")
        };
        let p = dir.join(name);
        if !p.exists() {
            return p;
        }
        n += 1;
    }
}

/// Create the web app (blocking: favicon download + file writes).
pub fn create_webapp(
    req: &Request,
    browser: &Path,
    browser_label: &str,
    apps_dir: &Path,
    icons_dir: &Path,
    fetch_icon: bool,
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(apps_dir).map_err(|e| e.to_string())?;
    let desktop = free_desktop_path(apps_dir, &slugify(&req.name));
    let stem = desktop
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "webapp".into());
    let icon = if fetch_icon {
        url_domain(&req.url).and_then(|domain| {
            let path = icons_dir.join(format!("{stem}.png"));
            match download_png(&favicon_url_sized(&domain, 256), &path) {
                Ok(()) => Some(path),
                Err(err) => {
                    tracing::warn!(%domain, %err, "webapp: icon fetch failed");
                    None
                }
            }
        })
    } else {
        None
    };
    let content = desktop_entry(req, browser, browser_label, icon.as_deref());
    let tmp = desktop.with_extension("desktop.tmp");
    std::fs::write(&tmp, content).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &desktop).map_err(|e| e.to_string())?;
    Ok(desktop)
}

/// Remove a web app: its `.desktop` (only if it carries our marker) and
/// its icon when the icon lives in our icon directory.
pub fn remove_webapp(desktop: &Path, icons_dir: &Path) -> Result<(), String> {
    let content = std::fs::read_to_string(desktop).map_err(|e| e.to_string())?;
    let app = parse_webapp(&content, desktop.to_path_buf())
        .ok_or_else(|| format!("{} is not a margo web app", desktop.display()))?;
    std::fs::remove_file(desktop).map_err(|e| e.to_string())?;
    let icon = Path::new(&app.icon);
    if icon.starts_with(icons_dir) {
        let _ = std::fs::remove_file(icon);
    }
    Ok(())
}

fn refresh_desktop_database(dir: &Path) {
    if find_in_path("update-desktop-database").is_some() {
        let _ = Command::new("update-desktop-database")
            .arg(dir)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

pub struct WebappProvider {
    apps_dir: PathBuf,
    icons_dir: PathBuf,
    browsers: Vec<(PathBuf, &'static str)>,
    apps: RefCell<Vec<WebApp>>,
    fetch_icons: bool,
}

impl WebappProvider {
    pub fn new() -> Self {
        Self::with_dirs(
            applications_dir(),
            icons_dir(),
            installed_browsers(find_in_path),
            true,
        )
    }

    pub fn with_dirs(
        apps_dir: PathBuf,
        icons_dir: PathBuf,
        browsers: Vec<(PathBuf, &'static str)>,
        fetch_icons: bool,
    ) -> Self {
        let me = Self {
            apps: RefCell::new(list_webapps(&apps_dir)),
            apps_dir,
            icons_dir,
            browsers,
            fetch_icons,
        };
        me.refresh();
        me
    }

    fn refresh(&self) {
        *self.apps.borrow_mut() = list_webapps(&self.apps_dir);
    }

    fn info(id: &str, name: &str, description: String, icon: &str) -> LauncherItem {
        LauncherItem {
            id: format!("webapp:{id}"),
            name: name.into(),
            description,
            icon: icon.into(),
            icon_is_path: false,
            score: 100.0,
            provider_name: NAME.into(),
            usage_key: None,
            on_activate: Rc::new(|| {}),
        }
    }

    fn create_rows(&self, req: Request) -> Vec<LauncherItem> {
        if self.browsers.is_empty() {
            return vec![Self::info(
                "nobrowser",
                "No Chromium-family browser found",
                "Web apps need app mode: install helium-browser-bin, google-chrome or chromium"
                    .into(),
                "dialog-warning-symbolic",
            )];
        }
        self.browsers
            .iter()
            .enumerate()
            .map(|(idx, (bin, label))| {
                let req = req.clone();
                let bin = bin.clone();
                let label: &'static str = label;
                let apps_dir = self.apps_dir.clone();
                let icons_dir = self.icons_dir.clone();
                let fetch = self.fetch_icons;
                LauncherItem {
                    id: format!("webapp:create:{label}"),
                    name: format!("Create web app “{}” in {label}", req.name),
                    description: req.url.clone(),
                    icon: "list-add-symbolic".into(),
                    icon_is_path: false,
                    score: 300.0 - idx as f64,
                    provider_name: NAME.into(),
                    usage_key: None,
                    on_activate: Rc::new(move || {
                        let (req, bin, apps_dir, icons_dir) = (
                            req.clone(),
                            bin.clone(),
                            apps_dir.clone(),
                            icons_dir.clone(),
                        );
                        std::thread::spawn(move || {
                            match create_webapp(&req, &bin, label, &apps_dir, &icons_dir, fetch) {
                                Ok(_) => {
                                    refresh_desktop_database(&apps_dir);
                                    toast("Web app created", format!("{} ({label})", req.name));
                                }
                                Err(err) => toast("Web app failed", err),
                            }
                        });
                    }),
                }
            })
            .collect()
    }

    fn list_rows(&self) -> Vec<LauncherItem> {
        let apps = self.apps.borrow();
        let mut rows: Vec<LauncherItem> = apps
            .iter()
            .enumerate()
            .map(|(idx, app)| {
                let desktop = app.desktop.clone();
                let has_icon = Path::new(&app.icon).is_file();
                LauncherItem {
                    id: format!("webapp:app:{}", app.desktop.display()),
                    name: app.name.clone(),
                    description: if app.browser.is_empty() {
                        format!("{} · Del: remove", app.url)
                    } else {
                        format!("{} · {} · Del: remove", app.url, app.browser)
                    },
                    icon: if has_icon {
                        app.icon.clone()
                    } else {
                        "web-browser-symbolic".into()
                    },
                    icon_is_path: has_icon,
                    score: 200.0 - idx as f64,
                    provider_name: NAME.into(),
                    usage_key: Some(format!("webapp:{}", app.desktop.display())),
                    on_activate: Rc::new(move || {
                        // gtk4-launch resolves the desktop id like a menu.
                        let id = desktop
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        if let Err(err) = Command::new("gtk4-launch").arg(&id).spawn_reaped() {
                            tracing::warn!(?err, id, "webapp: gtk4-launch failed");
                        }
                    }),
                }
            })
            .collect();
        rows.push(Self::info(
            "hint",
            "New web app: webapp <Name> <url>",
            "Opens the site in its own Chromium/Helium app window".into(),
            "list-add-symbolic",
        ));
        rows
    }
}

impl Default for WebappProvider {
    fn default() -> Self {
        Self::new()
    }
}

fn strip_prefix(query: &str) -> Option<&str> {
    let q = query.trim_start();
    if q == PREFIX {
        Some("")
    } else {
        q.strip_prefix(PREFIX)
            .filter(|r| r.starts_with(char::is_whitespace))
    }
}

impl Provider for WebappProvider {
    fn name(&self) -> &str {
        NAME
    }

    fn handles_search(&self) -> bool {
        false
    }

    fn handles_command(&self, query: &str) -> bool {
        strip_prefix(query).is_some()
    }

    fn commands(&self) -> Vec<LauncherItem> {
        vec![Self::info(
            "palette",
            "webapp <Name> <url>",
            "Create / list / remove Chromium --app web apps".into(),
            "web-browser-symbolic",
        )]
    }

    fn search(&self, query: &str) -> Vec<LauncherItem> {
        let Some(rest) = strip_prefix(query) else {
            return Vec::new();
        };
        let rest = rest.trim();
        if rest.is_empty() {
            return self.list_rows();
        }
        match parse_request(rest) {
            Some(req) => self.create_rows(req),
            None => {
                // Not a URL yet: filter the existing apps by name.
                let needle = rest.to_lowercase();
                let rows: Vec<LauncherItem> = self
                    .list_rows()
                    .into_iter()
                    .filter(|r| {
                        r.id.starts_with("webapp:app:") && r.name.to_lowercase().contains(&needle)
                    })
                    .collect();
                if rows.is_empty() {
                    vec![Self::info(
                        "needurl",
                        "Add a URL to create a web app",
                        format!("webapp {rest} https://…"),
                        "dialog-information-symbolic",
                    )]
                } else {
                    rows
                }
            }
        }
    }

    fn can_delete(&self, item: &LauncherItem) -> bool {
        item.id.starts_with("webapp:app:")
    }

    /// Delete key on an app row removes the web app.
    fn delete_item(&mut self, item: &LauncherItem) {
        let Some(path) = item.id.strip_prefix("webapp:app:") else {
            return;
        };
        match remove_webapp(Path::new(path), &self.icons_dir) {
            Ok(()) => {
                let dir = self.apps_dir.clone();
                std::thread::spawn(move || refresh_desktop_database(&dir));
                toast("Web app removed", item.name.clone());
            }
            Err(err) => toast("Web app not removed", err),
        }
        self.refresh();
    }

    fn on_opened(&mut self) {
        self.refresh();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("mshell-webapp-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn parses_requests() {
        assert_eq!(
            parse_request("My Mail https://mail.google.com/mail/u/0"),
            Some(Request {
                name: "My Mail".into(),
                url: "https://mail.google.com/mail/u/0".into()
            })
        );
        assert_eq!(
            parse_request("github.com"),
            Some(Request {
                name: "Github".into(),
                url: "https://github.com".into()
            })
        );
        assert_eq!(
            parse_request("www.youtube.com/feed"),
            Some(Request {
                name: "Youtube".into(),
                url: "https://www.youtube.com/feed".into()
            })
        );
        assert_eq!(parse_request("Evil javascript:alert(1)"), None);
        assert_eq!(parse_request("x file:///etc/passwd"), None);
        assert_eq!(parse_request("Bad/Name https://a.com"), None);
        assert_eq!(parse_request("just words"), None);
    }

    #[test]
    fn slug_and_exec_quoting() {
        assert_eq!(slugify("My Mail!"), "my-mail");
        assert_eq!(slugify("Ünï Côde"), "ünï-côde");
        assert_eq!(slugify("///"), "app");
        assert_eq!(
            exec_quote("/usr/bin/helium-browser"),
            "/usr/bin/helium-browser"
        );
        assert_eq!(
            exec_quote("--app=https://a.com/?q=1&x=%20"),
            "\"--app=https://a.com/?q=1&x=%%20\""
        );
        assert_eq!(exec_quote("a\"b$c"), "\"a\\\"b\\$c\"");
    }

    #[test]
    fn desktop_entry_round_trips_and_carries_marker() {
        let req = Request {
            name: "Git Hub".into(),
            url: "https://github.com/?tab=1".into(),
        };
        let content = desktop_entry(
            &req,
            Path::new("/usr/bin/helium-browser"),
            "Helium",
            Some(Path::new("/home/u/.local/share/margo/webapps/x.png")),
        );
        assert!(
            content.contains("Exec=/usr/bin/helium-browser \"--app=https://github.com/?tab=1\"\n")
        );
        assert!(content.contains("Icon=/home/u/.local/share/margo/webapps/x.png\n"));
        let app = parse_webapp(&content, PathBuf::from("/x.desktop")).unwrap();
        assert_eq!(app.name, "Git Hub");
        assert_eq!(app.url, "https://github.com/?tab=1");
        assert_eq!(app.browser, "Helium");
        // A normal application never parses as a web app.
        assert!(
            parse_webapp(
                "[Desktop Entry]\nName=Firefox\nExec=firefox\n",
                PathBuf::new()
            )
            .is_none()
        );
    }

    #[test]
    fn create_list_remove_cycle() {
        let apps = temp("apps");
        let icons = temp("icons");
        std::fs::write(
            apps.join("margo-webapp-foreign.desktop"),
            "[Desktop Entry]\nName=Real\nExec=real\n",
        )
        .unwrap();
        let req = parse_request("Docs https://docs.rs").unwrap();
        let bin = Path::new("/usr/bin/chromium");
        let first = create_webapp(&req, bin, "Chromium", &apps, &icons, false).unwrap();
        let second = create_webapp(&req, bin, "Chromium", &apps, &icons, false).unwrap();
        assert_eq!(first.file_name().unwrap(), "margo-webapp-docs.desktop");
        assert_eq!(second.file_name().unwrap(), "margo-webapp-docs-2.desktop");
        let listed = list_webapps(&apps);
        assert_eq!(listed.len(), 2, "foreign file without marker is ignored");
        // Removal refuses files without the marker.
        assert!(remove_webapp(&apps.join("margo-webapp-foreign.desktop"), &icons).is_err());
        assert!(apps.join("margo-webapp-foreign.desktop").exists());
        remove_webapp(&first, &icons).unwrap();
        assert!(!first.exists());
        assert_eq!(list_webapps(&apps).len(), 1);
        let _ = std::fs::remove_dir_all(&apps);
        let _ = std::fs::remove_dir_all(&icons);
    }

    #[test]
    fn browser_detection_prefers_helium_and_dedupes() {
        let have = [
            "google-chrome-stable",
            "chromium",
            "helium-browser",
            "google-chrome",
        ];
        let found = installed_browsers(|b| {
            have.contains(&b)
                .then(|| PathBuf::from(format!("/usr/bin/{b}")))
        });
        let labels: Vec<&str> = found.iter().map(|(_, l)| *l).collect();
        assert_eq!(labels, vec!["Helium", "Chrome", "Chromium"]);
        assert_eq!(found[1].0, PathBuf::from("/usr/bin/google-chrome-stable"));
    }

    #[test]
    fn provider_rows() {
        let apps = temp("rows");
        let icons = temp("rowicons");
        let p = WebappProvider::with_dirs(
            apps.clone(),
            icons.clone(),
            vec![(PathBuf::from("/usr/bin/helium-browser"), "Helium")],
            false,
        );
        assert!(p.handles_command("webapp"));
        assert!(!p.handles_command("webapps"));
        let rows = p.search("webapp Mail https://mail.google.com");
        assert_eq!(rows[0].name, "Create web app “Mail” in Helium");
        assert_eq!(p.search("webapp")[0].id, "webapp:hint");
        create_webapp(
            &parse_request("Mail mail.google.com").unwrap(),
            Path::new("/usr/bin/helium-browser"),
            "Helium",
            &apps,
            &icons,
            false,
        )
        .unwrap();
        let mut p = p;
        p.on_opened();
        let rows = p.search("webapp");
        assert_eq!(rows[0].name, "Mail");
        assert!(p.can_delete(&rows[0]));
        p.delete_item(&rows[0]);
        assert_eq!(p.search("webapp")[0].id, "webapp:hint");
        let none = WebappProvider::with_dirs(apps.clone(), icons.clone(), vec![], false);
        assert_eq!(none.search("webapp a.com")[0].id, "webapp:nobrowser");
        let _ = std::fs::remove_dir_all(&apps);
        let _ = std::fs::remove_dir_all(&icons);
    }
}
