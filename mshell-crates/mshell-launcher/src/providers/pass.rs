//! `pass <query>` — GNU pass (password-store) launcher provider.
//!
//! Ported from noctalia's `launcher-pass` plugin, rebuilt native: scans
//! the password store directly (no `find` subprocess), AND-token fuzzy
//! filtering ("spaces act as wildcards"), and two actions per entry:
//!
//! | Key | Action |
//! |---|---|
//! | Enter | **Copy** the password — `pass -c`, which auto-clears the clipboard after `PASSWORD_STORE_CLIP_TIME` (45 s default). |
//! | Ctrl+Enter | **Type** the password — `pass show … \| wtype`, so the secret never touches the clipboard (no history trail). |
//!
//! ## Entry fields (`pass <entry>:`)
//!
//! Typing a colon after an entry name (`pass web/github:`) opens that
//! entry's detail view — the same Enter = copy / Ctrl+Enter = type pair
//! for every value in it, filtered by whatever follows the colon
//! (`pass web/github: user`):
//!
//! * **Password** — the first line.
//! * **OTP code** — when the entry has an `otpauth://` line and
//!   `pass-otp` is installed (`pass otp -c` / `pass otp | wtype`); without
//!   pass-otp a hint row says so.
//! * **Username** — the first `login:` / `user:` / `username:` field,
//!   else the entry's basename (the pass convention).
//! * every other `key: value` line.
//!
//! The entry is decrypted once off the GTK thread to learn which fields
//! exist; only the field *names* are kept in memory. Each activation
//! decrypts again on a worker thread and hands the single value to
//! `wl-copy --sensitive` (cleared after 45 s if still on the clipboard,
//! like `pass -c`) or `wtype`.
//!
//! Store location follows pass itself: `$PASSWORD_STORE_DIR`, else
//! `~/.password-store`. Entries are listed by their store-relative path
//! without the `.gpg` suffix (e.g. `web/github`, `email/work`).
//!
//! Security note: `pass -c` does land the secret in the wl-clipboard,
//! which margo's own clipboard history (mshell-clipboard) watches — use
//! Ctrl+Enter (type) when you'd rather keep it out of history, or add
//! `pass` to the clipboard's sensitive-skip list.

use crate::providers::tools::shell_quote;
use crate::{
    item::LauncherItem,
    notify::toast,
    provider::{Provider, RefreshNotifier},
};
use mshell_process::SpawnReaped;
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Cap the rows built per query so a multi-hundred-entry store stays
/// responsive on the empty/browse view; a filter narrows well below this.
const MAX_RESULTS: usize = 100;

pub struct PassProvider {
    /// Current store directory; re-resolved from `resolver` on each open
    /// so a Settings change to the path applies without a restart.
    store_dir: RefCell<PathBuf>,
    /// Live path resolver (the shell closes over config + env). `None`
    /// for fixed-path construction (standalone / tests).
    resolver: Option<Box<dyn Fn() -> PathBuf>>,
    /// Store-relative entry names (sans `.gpg`), alphabetically sorted.
    entries: RefCell<Vec<String>>,
    /// Field *names* of decrypted entries (never the values), filled off
    /// thread for the `pass <entry>:` detail view; dropped on open.
    details: Arc<Mutex<HashMap<String, DetailState>>>,
    notifier: Arc<Mutex<Option<RefreshNotifier>>>,
}

/// `$PASSWORD_STORE_DIR` (if absolute), else `~/.password-store` — pass's
/// own resolution order, with no config layer on top.
pub fn default_store_dir() -> PathBuf {
    std::env::var_os("PASSWORD_STORE_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| dirs::home_dir().map(|h| h.join(".password-store")))
        .unwrap_or_else(|| PathBuf::from("/home/nobody/.password-store"))
}

impl PassProvider {
    pub fn new() -> Self {
        Self::with_dir(default_store_dir())
    }

    pub fn with_dir(store_dir: PathBuf) -> Self {
        let me = Self {
            store_dir: RefCell::new(store_dir),
            resolver: None,
            entries: RefCell::new(Vec::new()),
            details: Arc::default(),
            notifier: Arc::default(),
        };
        me.refresh();
        me
    }

    /// Construct with a live path resolver, re-read on every launcher open.
    pub fn with_resolver(resolver: Box<dyn Fn() -> PathBuf>) -> Self {
        let me = Self {
            store_dir: RefCell::new(resolver()),
            resolver: Some(resolver),
            entries: RefCell::new(Vec::new()),
            details: Arc::default(),
            notifier: Arc::default(),
        };
        me.refresh();
        me
    }

    pub fn refresh(&self) {
        let dir = self.store_dir.borrow().clone();
        let mut out = Vec::new();
        scan(&dir, &dir, &mut out, 0);
        out.sort();
        *self.entries.borrow_mut() = out;
    }
}

impl PassProvider {
    /// Field names of `entry`, decrypting off-thread on first request.
    /// Returns `Loading` until the worker lands (then the notifier repaints).
    fn detail_state(&self, entry: &str) -> DetailState {
        let Ok(mut map) = self.details.lock() else {
            return DetailState::Failed("internal lock poisoned".into());
        };
        if let Some(state) = map.get(entry) {
            return state.clone();
        }
        map.insert(entry.to_string(), DetailState::Loading);
        drop(map);
        let details = Arc::clone(&self.details);
        let notifier = Arc::clone(&self.notifier);
        let store = self.store_dir.borrow().clone();
        let entry = entry.to_string();
        std::thread::spawn(move || {
            let state = match pass_show(&store, &entry) {
                Ok(body) => DetailState::Ready(Detail::from_parsed(&parse_entry(&body))),
                Err(err) => DetailState::Failed(err),
            };
            if let Ok(mut map) = details.lock() {
                map.insert(entry, state);
            }
            if let Ok(slot) = notifier.lock()
                && let Some(notify) = slot.as_ref()
            {
                notify();
            }
        });
        DetailState::Loading
    }

    fn info_row(id: &str, name: String, description: String, icon: &str) -> LauncherItem {
        LauncherItem {
            id: format!("pass:{id}"),
            name,
            description,
            icon: icon.into(),
            icon_is_path: false,
            score: 100.0,
            provider_name: "Pass".into(),
            usage_key: None,
            on_activate: Rc::new(|| {}),
        }
    }

    /// The `pass <entry>:<filter>` view: one row per field, filtered by
    /// case-insensitive substring of the field label.
    fn detail_rows(&self, entry: &str, filter: &str) -> Vec<LauncherItem> {
        let detail = match self.detail_state(entry) {
            DetailState::Loading => {
                return vec![Self::info_row(
                    "loading",
                    format!("Decrypting {entry}…"),
                    "Waiting for gpg (pinentry may ask for your passphrase)".into(),
                    "content-loading-symbolic",
                )];
            }
            DetailState::Failed(err) => {
                return vec![Self::info_row(
                    "error",
                    format!("Could not decrypt {entry}"),
                    err,
                    "dialog-warning-symbolic",
                )];
            }
            DetailState::Ready(d) => d,
        };
        let otp_ok = otp_extension_installed();
        let needle = filter.to_lowercase();
        let store = self.store_dir.borrow().clone();
        let mut rows: Vec<LauncherItem> = detail
            .fields(otp_ok)
            .into_iter()
            .filter(|f| needle.is_empty() || f.label().to_lowercase().contains(&needle))
            .enumerate()
            .map(|(idx, field)| {
                let label = field.label();
                let store = store.clone();
                let entry_owned = entry.to_string();
                let field_for_action = field.clone();
                LauncherItem {
                    id: detail_id(entry, &field),
                    name: format!("{label} — {entry}"),
                    description: "Enter: copy · Ctrl+Enter: type".into(),
                    icon: if field == Field::Otp {
                        "appointment-soon-symbolic"
                    } else if field == Field::Username {
                        "avatar-default-symbolic"
                    } else {
                        ICON
                    }
                    .into(),
                    icon_is_path: false,
                    score: 200.0 - idx as f64,
                    provider_name: "Pass".into(),
                    usage_key: None,
                    on_activate: Rc::new(move || {
                        run_field_action(
                            store.clone(),
                            entry_owned.clone(),
                            field_for_action.clone(),
                            false,
                        )
                    }),
                }
            })
            .collect();
        if detail.has_otp && !otp_ok && (needle.is_empty() || "otp".contains(&needle)) {
            rows.push(Self::info_row(
                "otp-missing",
                "OTP code unavailable".into(),
                "This entry has an otpauth:// line — install pass-otp".into(),
                "dialog-information-symbolic",
            ));
        }
        rows
    }
}

impl Default for PassProvider {
    fn default() -> Self {
        Self::new()
    }
}

/// Recursively collect `*.gpg` entries as store-relative paths without the
/// `.gpg` suffix. Skips dotfiles/dirs (`.git`, `.gpg-id`, …) and bounds the
/// recursion depth so a symlink loop or pathological tree can't hang.
fn scan(root: &Path, dir: &Path, out: &mut Vec<String>, depth: usize) {
    if depth > 16 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();
        if name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if is_dir {
            scan(root, &path, out, depth + 1);
        } else if let Some(stem) = name.strip_suffix(".gpg") {
            // Rebuild the relative path with the stripped stem so nested
            // entries read as `web/github`, not `web/github.gpg`.
            let rel = path
                .strip_prefix(root)
                .ok()
                .and_then(|r| r.parent().map(|p| p.to_path_buf()))
                .filter(|p| !p.as_os_str().is_empty())
                .map(|parent| format!("{}/{stem}", parent.to_string_lossy()))
                .unwrap_or_else(|| stem.to_string());
            out.push(rel);
        }
    }
}

/// Spawn `pass -c <entry>` (decrypt + copy + auto-clear). Detached; gpg
/// pinentry, if needed, is handled by the agent.
///
/// `PASSWORD_STORE_DIR` is forced to the store the provider scanned —
/// mshell runs as a systemd user service and usually does NOT inherit
/// the shell's `$PASSWORD_STORE_DIR`, so a bare `pass -c` would look in
/// `~/.password-store`, not find the entry, and silently copy nothing.
fn copy_password(store_dir: &Path, entry: &str) {
    if let Err(err) = Command::new("pass")
        .env("PASSWORD_STORE_DIR", store_dir)
        .arg("-c")
        .arg(entry)
        .spawn_reaped()
    {
        tracing::warn!(?err, entry, "pass: copy spawn failed");
    }
}

/// Type the password into the focused surface via wtype, never touching
/// the clipboard. The short sleep lets the launcher close and keyboard
/// focus return to the target window first. `PASSWORD_STORE_DIR` is
/// forced for the same reason as `copy_password`.
fn type_password(store_dir: &Path, entry: &str) {
    let cmd = format!(
        "sleep 0.25; pass show {} 2>/dev/null | head -n1 | tr -d '\\n' | wtype -",
        shell_quote(entry)
    );
    if let Err(err) = Command::new("sh")
        .env("PASSWORD_STORE_DIR", store_dir)
        .arg("-c")
        .arg(cmd)
        .spawn_reaped()
    {
        tracing::warn!(?err, entry, "pass: type spawn failed");
    }
}

// ── Entry fields ─────────────────────────────────────────────────────────

/// Seconds a copied field stays on the clipboard (pass's own default).
const CLIP_SECS: u64 = 45;
/// Keys whose value is the entry's username (case-insensitive).
const USERNAME_KEYS: &[&str] = &["login", "user", "username"];
/// Separator between the field tag and the entry in detail-row ids
/// (entry names may contain `:` and `/`, never a unit separator).
const ID_SEP: char = '\u{1f}';

/// A decrypted entry: first non-empty line is the password, later
/// `key: value` lines are fields (file order), any `otpauth://` line marks
/// it OTP-capable. The pass / pass-otp conventions.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ParsedEntry {
    pub password: String,
    pub has_otp: bool,
    pub fields: Vec<(String, String)>,
}

pub fn parse_entry(body: &str) -> ParsedEntry {
    let mut out = ParsedEntry::default();
    let mut first = true;
    for line in body.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if first {
            out.password = line.to_string();
            first = false;
            continue;
        }
        if line.contains("otpauth://") {
            out.has_otp = true;
            continue;
        }
        if let Some((k, v)) = line.split_once(": ") {
            let k = k.trim();
            if !k.is_empty() {
                out.fields.push((k.to_string(), v.trim().to_string()));
            }
        }
    }
    out
}

/// Index of the username field, if any.
fn username_index(fields: &[(String, String)]) -> Option<usize> {
    fields
        .iter()
        .position(|(k, _)| USERNAME_KEYS.contains(&k.to_ascii_lowercase().as_str()))
}

/// One copy/type-able value of an entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Field {
    Password,
    Otp,
    Username,
    Key(String),
}

impl Field {
    fn tag(&self) -> String {
        match self {
            Field::Password => "password".into(),
            Field::Otp => "otp".into(),
            Field::Username => "username".into(),
            Field::Key(k) => format!("key:{k}"),
        }
    }

    fn from_tag(tag: &str) -> Option<Self> {
        match tag {
            "password" => Some(Field::Password),
            "otp" => Some(Field::Otp),
            "username" => Some(Field::Username),
            _ => tag.strip_prefix("key:").map(|k| Field::Key(k.to_string())),
        }
    }

    fn label(&self) -> String {
        match self {
            Field::Password => "Password".into(),
            Field::Otp => "OTP code".into(),
            Field::Username => "Username".into(),
            Field::Key(k) => k.clone(),
        }
    }
}

/// Value of `field` in a decrypted entry. Username falls back to the
/// entry's basename when no login/user/username field exists.
pub fn field_value(parsed: &ParsedEntry, entry: &str, field: &Field) -> Option<String> {
    match field {
        Field::Password => Some(parsed.password.clone()).filter(|p| !p.is_empty()),
        Field::Otp => None, // generated by pass-otp, not stored
        Field::Username => Some(
            username_index(&parsed.fields)
                .map(|i| parsed.fields[i].1.clone())
                .unwrap_or_else(|| entry.rsplit('/').next().unwrap_or(entry).to_string()),
        ),
        Field::Key(k) => parsed
            .fields
            .iter()
            .find(|(key, _)| key == k)
            .map(|(_, v)| v.clone()),
    }
}

/// What the detail view knows about an entry: field names only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detail {
    pub has_otp: bool,
    /// Non-username field keys, file order, deduplicated.
    pub keys: Vec<String>,
}

impl Detail {
    pub fn from_parsed(parsed: &ParsedEntry) -> Self {
        let user = username_index(&parsed.fields);
        let mut keys: Vec<String> = Vec::new();
        for (i, (k, _)) in parsed.fields.iter().enumerate() {
            if Some(i) != user && !keys.contains(k) {
                keys.push(k.clone());
            }
        }
        Self {
            has_otp: parsed.has_otp,
            keys,
        }
    }

    /// Rows in display order: password, OTP (when usable), username, keys.
    pub fn fields(&self, otp_available: bool) -> Vec<Field> {
        let mut out = vec![Field::Password];
        if self.has_otp && otp_available {
            out.push(Field::Otp);
        }
        out.push(Field::Username);
        out.extend(self.keys.iter().cloned().map(Field::Key));
        out
    }
}

#[derive(Debug, Clone)]
enum DetailState {
    Loading,
    Ready(Detail),
    Failed(String),
}

/// `pass <entry>:<filter>` → `(entry, filter)` when `entry` exists.
pub fn parse_detail_query(rest: &str, is_entry: impl Fn(&str) -> bool) -> Option<(&str, &str)> {
    let (entry, filter) = rest.split_once(':')?;
    let entry = entry.trim();
    is_entry(entry).then(|| (entry, filter.trim()))
}

/// pass-otp installed? It is a pass extension script, not a binary.
pub fn otp_extension_installed() -> bool {
    [
        "/usr/lib/password-store/extensions/otp.bash",
        "/usr/local/lib/password-store/extensions/otp.bash",
    ]
    .iter()
    .any(|p| Path::new(p).is_file())
}

fn detail_id(entry: &str, field: &Field) -> String {
    format!("pass:field:{}{ID_SEP}{entry}", field.tag())
}

fn parse_detail_id(id: &str) -> Option<(String, Field)> {
    let rest = id.strip_prefix("pass:field:")?;
    let (tag, entry) = rest.split_once(ID_SEP)?;
    Some((entry.to_string(), Field::from_tag(tag)?))
}

/// Decrypt `entry` (blocking — worker threads only).
fn pass_show(store_dir: &Path, entry: &str) -> Result<String, String> {
    let out = Command::new("pass")
        .env("PASSWORD_STORE_DIR", store_dir)
        .args(["show", entry])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("pass: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(err
            .lines()
            .last()
            .unwrap_or("pass show failed")
            .trim()
            .to_string())
    }
}

/// Current OTP code for `entry` via pass-otp (blocking).
fn pass_otp(store_dir: &Path, entry: &str) -> Result<String, String> {
    let out = Command::new("pass")
        .env("PASSWORD_STORE_DIR", store_dir)
        .args(["otp", entry])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("pass otp: {e}"))?;
    let code = String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    if out.status.success() && !code.is_empty() {
        Ok(code)
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// Pipe `value` into `argv`'s stdin and wait.
fn pipe_to(argv: &[&str], value: &str) -> Result<(), String> {
    let mut child = Command::new(argv[0])
        .args(&argv[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .map_err(|e| format!("{}: {e}", argv[0]))?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(value.as_bytes());
    }
    child
        .wait()
        .map_err(|e| e.to_string())?
        .success()
        .then_some(())
        .ok_or_else(|| format!("{} failed", argv[0]))
}

/// Copy `value` (sensitive hint, so clipboard managers may skip it) and
/// clear it after `CLIP_SECS` unless something else was copied since.
fn copy_secret(value: &str) -> Result<(), String> {
    pipe_to(&["wl-copy", "--sensitive", "--trim-newline"], value)?;
    let value = value.to_string();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(CLIP_SECS));
        let current = Command::new("wl-paste")
            .args(["--no-newline"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default();
        if current == value {
            let _ = Command::new("wl-copy").arg("--clear").status();
        }
    });
    Ok(())
}

/// Type `value` into the focused window (after the launcher closes).
fn type_secret(value: &str) -> Result<(), String> {
    std::thread::sleep(Duration::from_millis(250));
    pipe_to(&["wtype", "-"], value)
}

/// Copy (`typing == false`) or type one field. Password copy keeps
/// `pass -c`; everything else decrypts on a worker thread.
fn run_field_action(store: PathBuf, entry: String, field: Field, typing: bool) {
    match (&field, typing) {
        (Field::Password, false) => {
            copy_password(&store, &entry);
            toast("Pass", format!("Copied {entry} (clears in {CLIP_SECS}s)"));
            return;
        }
        (Field::Password, true) => {
            type_password(&store, &entry);
            toast("Pass", format!("Typing {entry}"));
            return;
        }
        _ => {}
    }
    std::thread::spawn(move || {
        let value = match &field {
            Field::Otp => pass_otp(&store, &entry),
            _ => pass_show(&store, &entry).and_then(|body| {
                field_value(&parse_entry(&body), &entry, &field)
                    .ok_or_else(|| format!("{} has no {}", entry, field.label()))
            }),
        };
        let label = field.label();
        let result = value.and_then(|v| {
            if typing {
                type_secret(&v)
            } else {
                copy_secret(&v)
            }
        });
        match result {
            Ok(()) if typing => toast("Pass", format!("Typed {label} of {entry}")),
            Ok(()) => toast(
                "Pass",
                format!("Copied {label} of {entry} (clears in {CLIP_SECS}s)"),
            ),
            Err(err) => {
                tracing::warn!(entry, %label, %err, "pass: field action failed");
                toast("Pass", format!("{label} of {entry}: {err}"));
            }
        }
    });
}

const ICON: &str = "dialog-password-symbolic";

impl Provider for PassProvider {
    fn name(&self) -> &str {
        "Pass"
    }

    fn category(&self) -> &str {
        "Connect"
    }

    fn handles_search(&self) -> bool {
        // Password entry names must never surface in the regular,
        // unprefixed search — only via the explicit `pass ` invocation.
        false
    }

    fn handles_command(&self, query: &str) -> bool {
        let q = query.trim_start();
        q == "pass" || q.starts_with("pass ")
    }

    fn commands(&self) -> Vec<LauncherItem> {
        vec![LauncherItem {
            id: "pass:palette".into(),
            name: "pass".into(),
            description: "Copy (Enter) or type (Ctrl+Enter) a password from your store".into(),
            icon: ICON.into(),
            icon_is_path: false,
            score: 0.0,
            provider_name: "Pass".into(),
            usage_key: None,
            on_activate: Rc::new(|| {}),
        }]
    }

    fn search(&self, query: &str) -> Vec<LauncherItem> {
        let q = query.trim_start();
        if !(q == "pass" || q.starts_with("pass ")) {
            return Vec::new();
        }
        let rest = q.trim_start_matches("pass");
        {
            let entries = self.entries.borrow();
            if let Some((entry, filter)) = parse_detail_query(rest, |e| {
                entries.binary_search_by(|x| x.as_str().cmp(e)).is_ok()
            }) {
                return self.detail_rows(entry, filter);
            }
        }
        let filter = rest.trim().to_ascii_lowercase();
        let tokens: Vec<&str> = filter.split_whitespace().collect();

        let entries = self.entries.borrow();
        if entries.is_empty() {
            return vec![LauncherItem {
                id: "pass:none".into(),
                name: "No password entries found".into(),
                description: format!("Looked in {}", self.store_dir.borrow().display()),
                icon: "dialog-warning-symbolic".into(),
                icon_is_path: false,
                score: 100.0,
                provider_name: "Pass".into(),
                usage_key: None,
                on_activate: Rc::new(|| {}),
            }];
        }

        let store_dir = self.store_dir.borrow().clone();
        entries
            .iter()
            .filter(|e| {
                // AND-token match: every space-separated token must appear
                // (the noctalia "spaces as wildcards" behaviour).
                let lower = e.to_ascii_lowercase();
                tokens.iter().all(|t| lower.contains(t))
            })
            .take(MAX_RESULTS)
            .enumerate()
            .map(|(idx, e)| {
                let entry = e.clone();
                let entry_for_toast = e.clone();
                let store = store_dir.clone();
                LauncherItem {
                    id: format!("pass:entry:{e}"),
                    name: e.clone(),
                    description: "Enter: copy · Ctrl+Enter: type · add ':' for fields".into(),
                    icon: ICON.into(),
                    icon_is_path: false,
                    score: 180.0 - idx as f64,
                    provider_name: "Pass".into(),
                    usage_key: Some(format!("pass:{e}")),
                    on_activate: Rc::new(move || {
                        copy_password(&store, &entry);
                        toast("Pass", format!("Copied {entry_for_toast} (clears in 45s)"));
                    }),
                }
            })
            .collect()
    }

    /// Ctrl+Enter on an entry → type it via wtype (no clipboard).
    fn alt_action(&self, item: &LauncherItem) -> Option<Rc<dyn Fn() + 'static>> {
        if let Some((entry, field)) = parse_detail_id(&item.id) {
            let store = self.store_dir.borrow().clone();
            return Some(Rc::new(move || {
                run_field_action(store.clone(), entry.clone(), field.clone(), true)
            }));
        }
        let entry = item.id.strip_prefix("pass:entry:")?.to_string();
        if entry.is_empty() {
            return None;
        }
        let entry_for_toast = entry.clone();
        let store = self.store_dir.borrow().clone();
        Some(Rc::new(move || {
            type_password(&store, &entry);
            toast("Pass", format!("Typing {entry_for_toast}"));
        }))
    }

    fn on_opened(&mut self) {
        // Re-resolve the store path (a Settings change applies here) and
        // pick up new/removed entries. Cheap.
        if let Some(resolver) = &self.resolver {
            *self.store_dir.borrow_mut() = resolver();
        }
        self.refresh();
        // Field lists are re-learnt per session (entries may have changed).
        if let Ok(mut d) = self.details.lock() {
            d.clear();
        }
    }

    fn set_refresh_notifier(&mut self, notifier: RefreshNotifier) {
        if let Ok(mut slot) = self.notifier.lock() {
            *slot = Some(notifier);
        }
    }

    /// Connect tab — list every entry without the `pass ` prefix;
    /// `filter` narrows by AND-token substring.
    fn browse(&self, filter: &str) -> Vec<LauncherItem> {
        if filter.is_empty() {
            self.search("pass")
        } else {
            self.search(&format!("pass {filter}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> PathBuf {
        // Unique per call: cargo runs tests as parallel threads of one
        // process, so keying the dir on `process::id()` alone made the
        // two store-scanning tests share a path and race each other's
        // remove_dir_all/create_dir_all (ENOENT mid-create). A monotonic
        // counter gives every call its own directory.
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("dcli-pass-test-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("web")).unwrap();
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(dir.join("email.gpg"), b"x").unwrap();
        std::fs::write(dir.join("web/github.gpg"), b"x").unwrap();
        std::fs::write(dir.join("web/gitlab.gpg"), b"x").unwrap();
        std::fs::write(dir.join(".git/HEAD"), b"x").unwrap();
        std::fs::write(dir.join(".gpg-id"), b"KEY").unwrap();
        std::fs::write(dir.join("notes.txt"), b"x").unwrap();
        dir
    }

    #[test]
    fn scans_entries_relative_without_gpg_skipping_dotdirs() {
        let dir = temp_store();
        let p = PassProvider::with_dir(dir.clone());
        let entries = p.entries.borrow().clone();
        assert!(entries.contains(&"email".to_string()));
        assert!(entries.contains(&"web/github".to_string()));
        assert!(entries.contains(&"web/gitlab".to_string()));
        // .git/ contents and non-.gpg files are excluded.
        assert!(!entries.iter().any(|e| e.contains("HEAD")));
        assert!(!entries.iter().any(|e| e.contains("notes")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn and_token_filter_treats_spaces_as_wildcards() {
        let dir = temp_store();
        let p = PassProvider::with_dir(dir.clone());
        let names: Vec<String> = p
            .search("pass web git")
            .iter()
            .map(|i| i.name.clone())
            .collect();
        // "web" + "git" both present in web/github and web/gitlab.
        assert!(names.contains(&"web/github".to_string()));
        assert!(names.contains(&"web/gitlab".to_string()));
        assert!(!names.contains(&"email".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn handles_command_only_for_pass_prefix() {
        let p = PassProvider::with_dir(PathBuf::from("/nonexistent"));
        assert!(p.handles_command("pass"));
        assert!(p.handles_command("pass github"));
        assert!(!p.handles_command("passwd"));
        assert!(!p.handles_command(":pass"));
    }

    #[test]
    fn missing_store_yields_warning_row() {
        let p = PassProvider::with_dir(PathBuf::from("/nonexistent/store"));
        assert!(
            p.search("pass")
                .iter()
                .any(|i| i.name == "No password entries found")
        );
    }

    // Shape of a real pass entry (values made up).
    const BODY: &str = "s3cr3t-pw\nlogin: kenan@example.com\nurl: https://github.com/login\notpauth://totp/GitHub:kenan?secret=JBSWY3DPEHPK3PXP&issuer=GitHub\nrecovery: a: b\n\nnotes without colon\n";

    #[test]
    fn parses_password_otp_and_fields() {
        let p = parse_entry(BODY);
        assert_eq!(p.password, "s3cr3t-pw");
        assert!(p.has_otp);
        assert_eq!(
            p.fields,
            vec![
                ("login".to_string(), "kenan@example.com".to_string()),
                ("url".to_string(), "https://github.com/login".to_string()),
                ("recovery".to_string(), "a: b".to_string()),
            ]
        );
        assert!(!parse_entry("pw-only\n").has_otp);
    }

    #[test]
    fn username_from_login_field_else_basename() {
        let p = parse_entry(BODY);
        assert_eq!(
            field_value(&p, "web/github", &Field::Username).as_deref(),
            Some("kenan@example.com")
        );
        let bare = parse_entry("pw\nurl: x\n");
        assert_eq!(
            field_value(&bare, "web/github", &Field::Username).as_deref(),
            Some("github")
        );
        assert_eq!(
            field_value(&p, "web/github", &Field::Key("url".into())).as_deref(),
            Some("https://github.com/login")
        );
        assert_eq!(
            field_value(&p, "e", &Field::Password).as_deref(),
            Some("s3cr3t-pw")
        );
    }

    #[test]
    fn detail_keeps_names_only_in_order() {
        let d = Detail::from_parsed(&parse_entry(BODY));
        assert_eq!(d.keys, vec!["url", "recovery"]);
        assert_eq!(
            d.fields(true),
            vec![
                Field::Password,
                Field::Otp,
                Field::Username,
                Field::Key("url".into()),
                Field::Key("recovery".into()),
            ]
        );
        // No pass-otp → no OTP row.
        assert!(!d.fields(false).contains(&Field::Otp));
        // Debug output of the cached detail carries no secret value.
        let dbg = format!("{d:?}");
        assert!(!dbg.contains("s3cr3t") && !dbg.contains("kenan@"));
    }

    #[test]
    fn colon_opens_detail_view_for_known_entries_only() {
        let known = |e: &str| e == "web/github";
        assert_eq!(
            parse_detail_query(" web/github:", known),
            Some(("web/github", ""))
        );
        assert_eq!(
            parse_detail_query(" web/github: us", known),
            Some(("web/github", "us"))
        );
        assert_eq!(parse_detail_query(" web/gh:", known), None);
        assert_eq!(parse_detail_query(" web github", known), None);
    }

    #[test]
    fn detail_ids_round_trip() {
        for f in [
            Field::Password,
            Field::Otp,
            Field::Username,
            Field::Key("a:b".into()),
        ] {
            let id = detail_id("dir/my:entry", &f);
            assert_eq!(parse_detail_id(&id), Some(("dir/my:entry".to_string(), f)));
        }
    }

    #[test]
    fn detail_rows_filter_by_label_once_decrypted() {
        let dir = temp_store();
        let p = PassProvider::with_dir(dir.clone());
        p.details.lock().unwrap().insert(
            "web/github".into(),
            DetailState::Ready(Detail::from_parsed(&parse_entry(BODY))),
        );
        let names: Vec<String> = p
            .search("pass web/github:")
            .into_iter()
            .map(|i| i.name)
            .collect();
        assert_eq!(
            names.first().map(String::as_str),
            Some("Password — web/github")
        );
        assert!(names.iter().any(|n| n == "Username — web/github"));
        assert!(names.iter().any(|n| n == "url — web/github"));
        let filtered: Vec<String> = p
            .search("pass web/github: ur")
            .into_iter()
            .map(|i| i.name)
            .collect();
        assert_eq!(filtered, vec!["url — web/github"]);
        let rows = p.search("pass web/github: user");
        assert!(p.alt_action(&rows[0]).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn alt_action_only_for_real_entries() {
        let p = PassProvider::with_dir(PathBuf::from("/nonexistent"));
        let mut palette = p.commands();
        assert!(p.alt_action(&palette.remove(0)).is_none());
    }
}
