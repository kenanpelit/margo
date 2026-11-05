//! `pass <query>` — GNU pass (password-store) launcher provider.
//!
//! Ported from noctalia's `launcher-pass` plugin, rebuilt native: scans
//! the password store directly (no `find` subprocess), AND-token fuzzy
//! filtering ("spaces act as wildcards"), and two actions per entry:
//!
//! | Key | Action |
//! |---|---|
//! | Enter | **Copy** the password — `wl-copy --sensitive` (kept out of clipboard history), cleared after 45 s if still on the clipboard. |
//! | Ctrl+Enter | **Type** the password via `wtype`, so the secret never touches the clipboard. |
//!
//! ## Entry fields (`pass <entry>:`)
//!
//! Typing a colon after an entry name (`pass web/github:`) opens that
//! entry's detail view — the same Enter = copy / Ctrl+Enter = type pair
//! for every value in it, filtered by whatever follows the colon
//! (`pass web/github: user`):
//!
//! * **Password** — the first line, exactly as stored (what `pass -c`
//!   would copy).
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
//! like `pass -c`) or `wtype`; the plain entry row does the same for the
//! password.
//!
//! Store location follows pass itself: `$PASSWORD_STORE_DIR`, else
//! `~/.password-store`. Entries are listed by their store-relative path
//! without the `.gpg` suffix (e.g. `web/github`, `email/work`).
//!
//! The password is copied the same way (never with `pass -c`, whose bare
//! `wl-copy` has no password-manager hint): mshell's clipboard history
//! skips `--sensitive` copies while `clipboard.skip_sensitive` is on (the
//! default). Ctrl+Enter types instead and never touches the clipboard.
//!
//! The detail view also carries two whole-entry actions:
//!
//! * **Autotype login** (when `wtype` is installed) — types username, a
//!   separator key (Tab, or Enter for two-step logins), the password, the
//!   OTP code when the entry has one, then Enter to submit; see
//!   [`super::pass_autotype`].
//! * **Edit entry** (when a terminal resolves) — `pass edit <entry>` in
//!   the user's terminal.
//!
//! Clipboard timeout, type delay, keystroke delay and the autotype
//! separator / submit key come from Settings → Launcher → Password store
//! ([`PassOptions`]), re-read on every launcher open.

use crate::providers::pass_autotype::{PassOptions, TypeStep, autotype_steps, run_steps};
use crate::{
    item::LauncherItem,
    notify::toast,
    provider::{Provider, RefreshNotifier},
};
use mshell_process::SpawnReaped;
use std::cell::{Cell, RefCell};
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
    /// Typing / clipboard behaviour; re-read from `options_resolver` on
    /// each open.
    options: Cell<PassOptions>,
    options_resolver: Option<Box<dyn Fn() -> PassOptions>>,
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
            options: Cell::new(PassOptions::default()),
            options_resolver: None,
        };
        me.refresh();
        me
    }

    /// Follow live typing / clipboard options (re-read on every open).
    pub fn with_options(mut self, resolver: Box<dyn Fn() -> PassOptions>) -> Self {
        self.options.set(resolver());
        self.options_resolver = Some(resolver);
        self
    }

    /// Construct with a live path resolver, re-read on every launcher open.
    pub fn with_resolver(resolver: Box<dyn Fn() -> PathBuf>) -> Self {
        let me = Self {
            store_dir: RefCell::new(resolver()),
            resolver: Some(resolver),
            entries: RefCell::new(Vec::new()),
            details: Arc::default(),
            notifier: Arc::default(),
            options: Cell::new(PassOptions::default()),
            options_resolver: None,
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
        let opts = self.options.get();
        let actions = EntryActions {
            autotype: margo_fs::path::on_path("wtype"),
            edit: mshell_process::terminal::pick().is_some(),
        };
        let mut rows: Vec<LauncherItem> = detail_rows_order(&detail, otp_ok, actions)
            .into_iter()
            .filter(|r| needle.is_empty() || r.label().to_lowercase().contains(&needle))
            .enumerate()
            .map(|(idx, row)| {
                let field = match row {
                    DetailRow::Field(f) => f,
                    DetailRow::Autotype => {
                        return action_row(
                            entry,
                            "autotype",
                            format!("Autotype login — {entry}"),
                            autotype_description(&opts, detail.has_otp && otp_ok),
                            "input-keyboard-symbolic",
                            200.0 - idx as f64,
                            {
                                let (store, entry) = (store.clone(), entry.to_string());
                                Rc::new(move || run_autotype(store.clone(), entry.clone(), opts))
                            },
                        );
                    }
                    DetailRow::Edit => {
                        return action_row(
                            entry,
                            "edit",
                            format!("Edit entry — {entry}"),
                            "Open `pass edit` in the terminal".into(),
                            "document-edit-symbolic",
                            200.0 - idx as f64,
                            {
                                let (store, entry) = (store.clone(), entry.to_string());
                                Rc::new(move || spawn_edit(&store, &entry))
                            },
                        );
                    }
                };
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
                            opts,
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

// ── Entry fields ─────────────────────────────────────────────────────────

/// Keys whose value is the entry's username (case-insensitive).
const USERNAME_KEYS: &[&str] = &["login", "user", "username"];
/// Separator between the field tag and the entry in detail-row ids
/// (entry names may contain `:` and `/`, never a unit separator).
const ID_SEP: char = '\u{1f}';

/// A decrypted entry: the first line is the password, later
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
    let mut lines = body.lines();
    // The literal first line (what `pass -c` copies): no trimming, and an
    // empty first line is an empty password, not the next line.
    out.password = lines.next().unwrap_or_default().to_string();
    for line in lines.map(str::trim).filter(|l| !l.is_empty()) {
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

/// Whole-entry actions available on this machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntryActions {
    /// `wtype` installed.
    pub autotype: bool,
    /// A terminal resolves for `pass edit`.
    pub edit: bool,
}

/// One row of the detail view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetailRow {
    Field(Field),
    Autotype,
    Edit,
}

impl DetailRow {
    fn label(&self) -> String {
        match self {
            DetailRow::Field(f) => f.label(),
            DetailRow::Autotype => "Autotype login".into(),
            DetailRow::Edit => "Edit entry".into(),
        }
    }
}

/// Detail rows in display order: password, OTP, username, autotype (right
/// after the username, like the plugin), the other fields, then edit.
pub fn detail_rows_order(
    detail: &Detail,
    otp_available: bool,
    actions: EntryActions,
) -> Vec<DetailRow> {
    let mut rows = Vec::new();
    for field in detail.fields(otp_available) {
        let after = field == Field::Username;
        rows.push(DetailRow::Field(field));
        if after && actions.autotype {
            rows.push(DetailRow::Autotype);
        }
    }
    if actions.edit {
        rows.push(DetailRow::Edit);
    }
    rows
}

fn autotype_description(opts: &PassOptions, otp: bool) -> String {
    let sep = match opts.separator {
        crate::providers::pass_autotype::Separator::Tab => "Tab",
        crate::providers::pass_autotype::Separator::Enter => "Enter",
    };
    let mut s = format!("Types username · {sep} · password");
    if otp {
        s.push_str(" · Enter · OTP");
    }
    if opts.submit {
        s.push_str(" · Enter");
    }
    s
}

fn action_row(
    entry: &str,
    tag: &str,
    name: String,
    description: String,
    icon: &str,
    score: f64,
    on_activate: Rc<dyn Fn() + 'static>,
) -> LauncherItem {
    LauncherItem {
        id: format!("pass:{tag}{ID_SEP}{entry}"),
        name,
        description,
        icon: icon.into(),
        icon_is_path: false,
        score,
        provider_name: "Pass".into(),
        usage_key: None,
        on_activate,
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
/// clear it after `clip_secs` unless something else was copied since.
fn copy_secret(value: &str, clip_secs: u32) -> Result<(), String> {
    pipe_to(&["wl-copy", "--sensitive", "--trim-newline"], value)?;
    let value = value.to_string();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(u64::from(clip_secs)));
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
fn type_secret(value: &str, opts: &PassOptions) -> Result<(), String> {
    run_steps(&[TypeStep::Text(value.to_string())], opts)
}

/// The autotype login for `entry`: decrypt once (and fetch the OTP code
/// when the entry has one and pass-otp is installed) on a worker thread,
/// then type the sequence. Values never leave the worker thread.
fn run_autotype(store: PathBuf, entry: String, opts: PassOptions) {
    std::thread::spawn(move || {
        let result = pass_show(&store, &entry).and_then(|body| {
            let parsed = parse_entry(&body);
            let password = field_value(&parsed, &entry, &Field::Password)
                .ok_or_else(|| format!("{entry} has no password"))?;
            let username = field_value(&parsed, &entry, &Field::Username).unwrap_or_default();
            let otp = if parsed.has_otp && otp_extension_installed() {
                Some(pass_otp(&store, &entry)?)
            } else {
                None
            };
            let steps = autotype_steps(&username, &password, otp.as_deref(), &opts);
            run_steps(&steps, &opts)
        });
        match result {
            Ok(()) => toast("Pass", format!("Autotyped login for {entry}")),
            Err(err) => {
                tracing::warn!(entry, %err, "pass: autotype failed");
                toast("Pass", format!("Autotype {entry}: {err}"));
            }
        }
    });
}

/// `pass edit <entry>` in the user's terminal (pass re-encrypts on save).
fn spawn_edit(store: &Path, entry: &str) {
    match mshell_process::terminal::command(&["pass", "edit", entry], "pass-edit") {
        Ok((terminal, mut cmd)) => {
            cmd.env("PASSWORD_STORE_DIR", store);
            if let Err(err) = cmd.spawn_reaped() {
                tracing::warn!(?err, %terminal, entry, "pass: edit terminal spawn failed");
                toast("Pass", format!("Could not start {terminal}"));
            }
        }
        Err(err) => toast("Pass", err.to_string()),
    }
}

/// Copy (`typing == false`) or type one field, the password included:
/// decrypt on a worker thread, then `copy_secret` (`wl-copy --sensitive`,
/// which mshell's clipboard history skips) or `type_secret`. Never
/// `pass -c`: it copies with a bare `wl-copy`, so the password stayed in
/// clipboard history after pass restored the old clipboard.
///
/// `PASSWORD_STORE_DIR` is forced to the store the provider scanned
/// (`pass_show`): mshell runs as a systemd user service and usually does
/// NOT inherit the shell's `$PASSWORD_STORE_DIR`.
fn run_field_action(store: PathBuf, entry: String, field: Field, typing: bool, opts: PassOptions) {
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
                type_secret(&v, &opts)
            } else {
                copy_secret(&v, opts.clip_timeout_secs)
            }
        });
        match result {
            Ok(()) if typing => toast("Pass", format!("Typed {label} of {entry}")),
            Ok(()) => toast(
                "Pass",
                format!(
                    "Copied {label} of {entry} (clears in {}s)",
                    opts.clip_timeout_secs
                ),
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
        let opts = self.options.get();
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
                        run_field_action(store.clone(), entry.clone(), Field::Password, false, opts)
                    }),
                }
            })
            .collect()
    }

    /// Ctrl+Enter on an entry → type it via wtype (no clipboard).
    fn alt_action(&self, item: &LauncherItem) -> Option<Rc<dyn Fn() + 'static>> {
        let opts = self.options.get();
        if let Some((entry, field)) = parse_detail_id(&item.id) {
            let store = self.store_dir.borrow().clone();
            return Some(Rc::new(move || {
                run_field_action(store.clone(), entry.clone(), field.clone(), true, opts)
            }));
        }
        let entry = item.id.strip_prefix("pass:entry:")?.to_string();
        if entry.is_empty() {
            return None;
        }
        let store = self.store_dir.borrow().clone();
        Some(Rc::new(move || {
            run_field_action(store.clone(), entry.clone(), Field::Password, true, opts)
        }))
    }

    fn on_opened(&mut self) {
        // Re-resolve the store path (a Settings change applies here) and
        // pick up new/removed entries. Cheap.
        if let Some(resolver) = &self.resolver {
            *self.store_dir.borrow_mut() = resolver();
        }
        if let Some(resolver) = &self.options_resolver {
            self.options.set(resolver());
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

    #[test]
    fn every_secret_goes_through_the_sensitive_copy_or_wtype() {
        // `pass -c` copies with a bare `wl-copy` (no password-manager hint),
        // so mshell's clipboard history kept the password after pass
        // restored the old clipboard. Every field, the password included,
        // must be decrypted here and handed to `copy_secret`
        // (`wl-copy --sensitive`) or `type_secret`, never to a `-c` mode or a
        // shell pipeline.
        let src = include_str!("pass.rs");
        let prod = src.split("#[cfg(test)]").next().unwrap();
        assert!(
            !prod.contains(".arg(\"-c\")"),
            "pass.rs still runs a `-c` copy/shell"
        );
        assert!(
            !prod.contains("Command::new(\"sh\")"),
            "pass.rs still pipes a secret through sh"
        );
        assert!(prod.contains("\"wl-copy\", \"--sensitive\""));
    }

    #[test]
    fn autotype_sits_after_username_and_edit_comes_last() {
        let d = Detail::from_parsed(&parse_entry(BODY));
        let both = EntryActions {
            autotype: true,
            edit: true,
        };
        assert_eq!(
            detail_rows_order(&d, true, both),
            vec![
                DetailRow::Field(Field::Password),
                DetailRow::Field(Field::Otp),
                DetailRow::Field(Field::Username),
                DetailRow::Autotype,
                DetailRow::Field(Field::Key("url".into())),
                DetailRow::Field(Field::Key("recovery".into())),
                DetailRow::Edit,
            ]
        );
        // No wtype / no terminal → the action rows are hidden.
        let none = EntryActions {
            autotype: false,
            edit: false,
        };
        let rows = detail_rows_order(&d, true, none);
        assert!(!rows.contains(&DetailRow::Autotype) && !rows.contains(&DetailRow::Edit));
    }

    #[test]
    fn autotype_description_follows_the_options() {
        use crate::providers::pass_autotype::Separator;
        let mut o = PassOptions::default();
        assert_eq!(
            autotype_description(&o, false),
            "Types username · Tab · password · Enter"
        );
        o.separator = Separator::Enter;
        o.submit = false;
        assert_eq!(
            autotype_description(&o, true),
            "Types username · Enter · password · Enter · OTP"
        );
    }

    #[test]
    fn options_are_re_read_on_open() {
        use std::sync::atomic::{AtomicU32, Ordering};
        static CLIP: AtomicU32 = AtomicU32::new(10);
        let mut p =
            PassProvider::with_dir(PathBuf::from("/nonexistent")).with_options(Box::new(|| {
                PassOptions {
                    clip_timeout_secs: CLIP.load(Ordering::Relaxed),
                    ..PassOptions::default()
                }
            }));
        assert_eq!(p.options.get().clip_timeout_secs, 10);
        CLIP.store(90, Ordering::Relaxed);
        p.on_opened();
        assert_eq!(p.options.get().clip_timeout_secs, 90);
    }

    #[test]
    fn action_rows_have_no_type_alt_action() {
        // Ctrl+Enter on Autotype / Edit falls back to Enter (no "type" of
        // an action row).
        let p = PassProvider::with_dir(PathBuf::from("/nonexistent"));
        let row = action_row(
            "web/github",
            "autotype",
            "x".into(),
            String::new(),
            ICON,
            1.0,
            Rc::new(|| {}),
        );
        assert!(p.alt_action(&row).is_none());
        assert_eq!(parse_detail_id(&row.id), None);
    }

    #[test]
    fn the_password_is_the_first_line_exactly_as_stored() {
        // The plain row now copies through parse_entry, so it must match
        // what `pass -c` copied: the literal first line.
        let p = parse_entry("  pw with spaces \t\nlogin: me\n");
        assert_eq!(p.password, "  pw with spaces \t");
        assert_eq!(p.fields, vec![("login".to_string(), "me".to_string())]);
        assert_eq!(parse_entry("pw\r\nurl: x\r\n").password, "pw");
        // An empty first line is an empty password, not the next line.
        let blank = parse_entry("\nlogin: me\n");
        assert_eq!(blank.password, "");
        assert_eq!(field_value(&blank, "e", &Field::Password), None);
    }
}
