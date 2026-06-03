//! Settings → Keybinds — a full editor for margo's keyboard shortcuts.
//!
//! margo keeps binds as `bind = MODS,KEY,ACTION[,ARGS…]` lines in
//! `config.conf`. Editing them in place means line-precise surgery on a
//! hand-maintained (often dotfiles-symlinked) file. Instead this editor
//! **owns a dedicated `binds.conf`**: on first edit we migrate every inline
//! `bind*` line out of `config.conf` into `binds.conf` (backing up the
//! original) and leave a single `source = binds.conf` behind. From then on
//! every add / edit / delete is a clean full rewrite of `binds.conf`, grouped
//! by category — no fragile in-place patching, zero risk to the rest of the
//! config. `mctl reload` applies the change live.
//!
//! The list is searchable (filter-func, never rebuilt per keystroke); a row
//! opens an inline editor with modifier chips, a press-to-capture key field,
//! a searchable action picker, and a contextual argument field.

use relm4::gtk::prelude::*;
use relm4::gtk::{gdk, glib};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::cell::RefCell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;

// ── Known dispatch actions (name, argument hint) ────────────────────────────
// Curated from margo/src/dispatch/mod.rs. Covers everything a config uses; an
// unknown action on an existing bind is added to the picker on the fly.
const ACTIONS: &[(&str, &str)] = &[
    ("spawn", "Command to run — e.g. kitty"),
    ("killclient", "no arguments"),
    ("togglefloating", "no arguments"),
    ("togglefullscreen", "no arguments"),
    ("togglefullscreen_exclusive", "no arguments"),
    ("sticky_window", "no arguments"),
    ("zoom", "no arguments"),
    ("focusdir", "Direction — left | right | up | down"),
    ("focusstack", "Offset — 1 (next) or -1 (prev)"),
    ("focuswindow", "App-id / title regex to focus"),
    ("exchange_client", "Direction — left | right | up | down"),
    ("movewin", "Direction — left | right | up | down"),
    ("resizewin", "Direction — left | right | up | down"),
    ("view", "Tag number (1, 2, 3 …)"),
    ("tag", "Tag number to move the window to"),
    ("tagview", "Tag number — move window there and follow"),
    ("toggleview", "Tag number to toggle into view"),
    ("toggletag", "Tag number to toggle on the window"),
    ("tagall", "no arguments"),
    ("viewtoleft", "no arguments"),
    ("viewtoright", "no arguments"),
    ("tagtoleft", "no arguments"),
    ("tagtoright", "no arguments"),
    ("focusmon", "Direction — left | right | up | down"),
    ("tagmon", "Direction — left | right | up | down"),
    (
        "setlayout",
        "Layout — tile | monocle | scroller | grid | deck | dwindle …",
    ),
    ("switch_layout", "no arguments"),
    ("incnmaster", "Master count delta — 1 or -1"),
    ("setmfact", "Master ratio delta — e.g. 0.05 or -0.05"),
    ("set_proportion", "Master ratio — 0.0 … 1.0 (e.g. 0.5)"),
    ("switch_proportion_preset", "no arguments"),
    ("togglegaps", "no arguments"),
    ("incgaps", "Gap delta — e.g. 5 or -5"),
    ("toggle_scratchpad", "no arguments"),
    (
        "toggle_named_scratchpad",
        "app-id regex , title|none , spawn command",
    ),
    ("unscratchpad", "no arguments"),
    ("summon", "match regex , exclude|none , spawn command"),
    ("toggle_overview", "no arguments"),
    ("overview_focus_next", "no arguments"),
    ("overview_focus_prev", "no arguments"),
    ("overview_activate", "no arguments"),
    ("canvas_pan", "Direction — left | right | up | down"),
    ("canvas_reset", "no arguments"),
    ("screenshot", "no arguments"),
    ("screenshot-window", "no arguments"),
    ("screenshot-region", "no arguments"),
    ("screenshot-region-ui", "no arguments"),
    ("screenshot-output", "no arguments"),
    ("theme", "Theme preset name"),
    ("twilight_toggle", "no arguments"),
    ("twilight_set", "key value (e.g. mode geo)"),
    ("session_save", "Session name"),
    ("session_load", "Session name"),
    ("setkeymode", "Key-mode name"),
    ("reload", "no arguments"),
    ("quit", "no arguments"),
    ("force_unlock", "no arguments"),
    ("run_script", "Rhai script path or inline code"),
];

/// Cheatsheet-style category order — also the section order in `binds.conf`.
const CATEGORIES: &[&str] = &[
    "Launch",
    "Windows",
    "Workspaces",
    "Layout",
    "Scratchpad",
    "Media",
    "Shell",
    "System",
    "General",
];

// ── One bind ────────────────────────────────────────────────────────────────
#[derive(Clone, Debug, Default)]
pub(crate) struct Bind {
    /// Suffix chars after `bind` (`s`=sym, `l`=lock, `r`=release, `p`=pass).
    flags: String,
    m_super: bool,
    m_ctrl: bool,
    m_shift: bool,
    m_alt: bool,
    /// Raw keysym name (`Return`, `d`, `XF86AudioRaiseVolume`).
    key: String,
    action: String,
    /// Raw comma-joined argument tail, exactly as written after the action.
    args: String,
    /// Optional human description (trailing `#"…"` comment).
    desc: String,
}

impl Bind {
    /// `super+ctrl` … or `NONE`.
    fn mods_str(&self) -> String {
        let mut v: Vec<&str> = Vec::new();
        if self.m_super {
            v.push("super");
        }
        if self.m_ctrl {
            v.push("ctrl");
        }
        if self.m_alt {
            v.push("alt");
        }
        if self.m_shift {
            v.push("shift");
        }
        if v.is_empty() {
            "NONE".to_string()
        } else {
            v.join("+")
        }
    }

    /// Regenerate the canonical `bind … = …` line.
    fn to_line(&self) -> String {
        let mut s = format!(
            "bind{} = {},{},{}",
            self.flags,
            self.mods_str(),
            self.key.trim(),
            self.action.trim()
        );
        let args = self.args.trim();
        if !args.is_empty() {
            s.push(',');
            s.push_str(args);
        }
        let desc = self.desc.trim();
        if !desc.is_empty() {
            // Space-prefixed so the parser's inline-comment stripper drops it.
            s.push_str(&format!("   #\"{}\"", desc.replace('"', "'")));
        }
        s
    }

    /// Pretty trigger key for the list chips.
    fn pretty_key(&self) -> String {
        format_key(&self.key)
    }

    /// Lower-cased search haystack: mods + key + action + args + desc + category.
    fn haystack(&self) -> String {
        format!(
            "{} {} {} {} {} {}",
            self.mods_str(),
            self.key,
            self.action,
            self.args,
            self.desc,
            self.category()
        )
        .to_lowercase()
    }

    fn category(&self) -> &'static str {
        categorise(&self.action, &self.args)
    }

    // ── Summon helpers (used by the dedicated Tag Apps page) ──────────
    pub(crate) fn is_summon(&self) -> bool {
        self.action.eq_ignore_ascii_case("summon")
    }

    /// `super+alt` / `NONE` — the modifier string (re-exported for the
    /// Tag Apps row display).
    pub(crate) fn mods(&self) -> String {
        self.mods_str()
    }

    pub(crate) fn key_name(&self) -> &str {
        self.key.trim()
    }

    /// Split a summon bind's arg tail into `(app_id, title, spawn)`. A
    /// `none`/empty title comes back as an empty string.
    pub(crate) fn summon_parts(&self) -> (String, String, String) {
        let p: Vec<&str> = self.args.splitn(3, ',').collect();
        let appid = p.first().map(|s| s.trim().to_string()).unwrap_or_default();
        let title = p
            .get(1)
            .map(|s| s.trim())
            .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("none"))
            .unwrap_or("")
            .to_string();
        let spawn = p.get(2).map(|s| s.trim().to_string()).unwrap_or_default();
        (appid, title, spawn)
    }

    /// Build a `summon` bind from friendly fields. An empty title is
    /// written as `none` so the spawn command stays the third arg.
    pub(crate) fn new_summon(mods: &str, key: &str, appid: &str, title: &str, spawn: &str) -> Bind {
        let mut b = Bind {
            key: key.trim().to_string(),
            action: "summon".to_string(),
            ..Default::default()
        };
        b.set_mods(mods);
        let title = if title.trim().is_empty() {
            "none"
        } else {
            title.trim()
        };
        b.args = format!("{},{},{}", appid.trim(), title, spawn.trim());
        b
    }

    fn set_mods(&mut self, mods: &str) {
        for m in mods.split('+') {
            match m.trim().to_ascii_lowercase().as_str() {
                "super" | "mod" | "mod4" | "logo" | "win" => self.m_super = true,
                "ctrl" | "control" => self.m_ctrl = true,
                "shift" => self.m_shift = true,
                "alt" | "mod1" => self.m_alt = true,
                _ => {}
            }
        }
    }

    // ── Tag-key helpers (move / toggle the focused window's tags) ─────
    pub(crate) fn action_str(&self) -> &str {
        self.action.trim()
    }

    /// `tag` / `toggletag` — moves or toggles the focused window's tag set.
    pub(crate) fn is_tag_key(&self) -> bool {
        let a = self.action.to_ascii_lowercase();
        a == "tag" || a == "toggletag"
    }

    /// The raw tag bitmask argument (`tag,16` → 16). Binds carry a raw
    /// mask, not a 1-based index.
    pub(crate) fn tag_mask(&self) -> u32 {
        self.args.trim().parse().unwrap_or(0)
    }

    /// A tag-key the friendly Tags page can represent: a single tag bit or
    /// the all-tags mask. Multi-bit masks (e.g. `tag,6`) are left to the
    /// raw keybinds editor and preserved untouched.
    pub(crate) fn is_simple_tag_key(&self) -> bool {
        if !self.is_tag_key() {
            return false;
        }
        let m = self.tag_mask();
        m == u32::MAX || (m != 0 && m.is_power_of_two())
    }

    /// Build a `tag` / `toggletag` bind. `action` is `"tag"` or
    /// `"toggletag"`; `mask` is the raw tag bitmask.
    pub(crate) fn new_tag(mods: &str, key: &str, action: &str, mask: u32) -> Bind {
        let mut b = Bind {
            key: key.trim().to_string(),
            action: action.trim().to_string(),
            args: mask.to_string(),
            ..Default::default()
        };
        b.set_mods(mods);
        b
    }
}

// ── Paths ─────────────────────────────────────────────────────────────────
fn config_dir() -> PathBuf {
    let base = if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        PathBuf::from(xdg)
    } else if let Some(home) = std::env::var_os("HOME") {
        PathBuf::from(home).join(".config")
    } else {
        PathBuf::from(".config")
    };
    base.join("margo")
}
fn config_path() -> PathBuf {
    config_dir().join("config.conf")
}
fn binds_path() -> PathBuf {
    config_dir().join("binds.conf")
}

// ── Reading ─────────────────────────────────────────────────────────────────
fn expand(path: &str, base_dir: &Path) -> PathBuf {
    let p = path.trim().trim_matches('"');
    if let Some(rest) = p.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    let pb = PathBuf::from(p);
    if pb.is_absolute() {
        pb
    } else {
        base_dir.join(pb)
    }
}

/// Read a config file plus every `source =` it pulls in (depth-first, guarded).
fn read_all_lines(path: &Path, visited: &mut HashSet<PathBuf>, out: &mut Vec<String>) {
    let canon = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if !visited.insert(canon) {
        return;
    }
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    let base_dir = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("source")
            && let Some(val) = rest.trim_start().strip_prefix('=')
        {
            read_all_lines(&expand(val, &base_dir), visited, out);
            continue;
        }
        out.push(line.to_string());
    }
}

/// Every bind reachable from `config.conf` (inline + sourced), parsed.
pub(crate) fn load_binds() -> Vec<Bind> {
    let mut lines = Vec::new();
    read_all_lines(&config_path(), &mut HashSet::new(), &mut lines);
    let mut binds: Vec<Bind> = lines.iter().filter_map(|l| parse_bind_line(l)).collect();
    binds.sort_by(sort_key);
    binds
}

/// Is `key` a `bind` variant (`bind`, `binds`, `bindr`, `bindl`, `bindp`, …)?
fn is_bind_key(k: &str) -> bool {
    k.starts_with("bind") && k[4..].chars().all(|c| matches!(c, 's' | 'l' | 'r' | 'p'))
}

/// Parse one `bind* = …` line into a [`Bind`]. `None` for non-bind lines.
fn parse_bind_line(line: &str) -> Option<Bind> {
    let t = line.trim();
    if !t.starts_with("bind") {
        return None;
    }
    let eq = t.find('=')?;
    let key = t[..eq].trim();
    if !is_bind_key(key) {
        return None;
    }
    let flags = key[4..].to_string();
    let body = t[eq + 1..].trim();

    let (body, desc) = split_comment(body);

    // Same splitn(8) shape the compositor parser uses: the 8th field keeps
    // any remaining commas (so spawn commands survive intact).
    let parts: Vec<&str> = body.splitn(8, ',').map(|p| p.trim()).collect();
    if parts.len() < 3 {
        return None;
    }
    let mods = parts[0];
    let key_raw = parts[1];
    let action = parts[2];
    let args = parts[3..].join(",");

    let mut b = Bind {
        flags,
        key: key_raw.to_string(),
        action: action.to_string(),
        args,
        desc: desc.unwrap_or_default(),
        ..Default::default()
    };
    for m in mods.split('+') {
        match m.trim().to_ascii_lowercase().as_str() {
            "super" | "mod" | "mod4" | "logo" | "win" => b.m_super = true,
            "ctrl" | "control" => b.m_ctrl = true,
            "shift" => b.m_shift = true,
            "alt" | "mod1" => b.m_alt = true,
            _ => {}
        }
    }
    Some(b)
}

/// Pull a trailing `#"quoted"` or ` # plain` description off a bind body.
fn split_comment(body: &str) -> (&str, Option<String>) {
    if let Some(start) = body.rfind("#\"")
        && let Some(end) = body[start + 2..].find('"')
    {
        let desc = body[start + 2..start + 2 + end].trim().to_string();
        return (body[..start].trim_end(), Some(desc));
    }
    let bytes = body.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'#' && i > 0 && bytes[i - 1].is_ascii_whitespace() {
            let desc = body[i + 1..].trim().to_string();
            return (
                body[..i].trim_end(),
                if desc.is_empty() { None } else { Some(desc) },
            );
        }
    }
    (body, None)
}

fn sort_key(a: &Bind, b: &Bind) -> std::cmp::Ordering {
    let ca = CATEGORIES
        .iter()
        .position(|c| *c == a.category())
        .unwrap_or(usize::MAX);
    let cb = CATEGORIES
        .iter()
        .position(|c| *c == b.category())
        .unwrap_or(usize::MAX);
    ca.cmp(&cb)
        .then_with(|| a.key.to_lowercase().cmp(&b.key.to_lowercase()))
        .then_with(|| a.action.cmp(&b.action))
}

fn format_key(key: &str) -> String {
    let k = key.trim();
    match k {
        "XF86AudioRaiseVolume" => "Vol+".into(),
        "XF86AudioLowerVolume" => "Vol−".into(),
        "XF86AudioMute" => "Mute".into(),
        "XF86AudioMicMute" => "MicMute".into(),
        "XF86AudioPlay" | "XF86AudioPlayPause" => "Play".into(),
        "XF86AudioNext" => "Next".into(),
        "XF86AudioPrev" => "Prev".into(),
        "XF86MonBrightnessUp" => "Bright+".into(),
        "XF86MonBrightnessDown" => "Bright−".into(),
        "Return" => "Enter".into(),
        "Escape" => "Esc".into(),
        "space" => "Space".into(),
        "Print" => "PrtSc".into(),
        other if other.chars().count() == 1 => other.to_ascii_uppercase(),
        other => other.to_string(),
    }
}

fn categorise(action: &str, args: &str) -> &'static str {
    let a = action.to_ascii_lowercase();
    let cmd = args.to_ascii_lowercase();
    if a.contains("scratchpad") || a == "summon" || a == "unscratchpad" {
        return "Scratchpad";
    }
    match a.as_str() {
        "spawn" | "exec" => {
            if cmd.contains("mshellctl") || cmd.contains("mctl") || cmd.contains("rofi") {
                "Shell"
            } else if cmd.contains("pamixer")
                || cmd.contains("wpctl")
                || cmd.contains("playerctl")
                || cmd.contains("brightness")
                || cmd.contains("soundctl")
                || cmd.contains("osc-")
            {
                "Media"
            } else if cmd.contains("mlock")
                || cmd.contains("systemctl")
                || cmd.contains("shutdown")
                || cmd.contains("reboot")
            {
                "System"
            } else {
                "Launch"
            }
        }
        "view"
        | "tag"
        | "tagview"
        | "toggleview"
        | "toggletag"
        | "tagall"
        | "viewtoleft"
        | "viewtoright"
        | "tagtoleft"
        | "tagtoright"
        | "focusmon"
        | "tagmon"
        | "toggle_overview"
        | "overview_focus_next"
        | "overview_focus_prev"
        | "overview_activate" => "Workspaces",
        "setlayout"
        | "switch_layout"
        | "incnmaster"
        | "setmfact"
        | "set_proportion"
        | "switch_proportion_preset"
        | "incgaps"
        | "togglegaps"
        | "canvas_pan"
        | "canvas_reset" => "Layout",
        "killclient"
        | "togglefloating"
        | "togglefullscreen"
        | "togglefullscreen_exclusive"
        | "movewin"
        | "resizewin"
        | "zoom"
        | "focusdir"
        | "focusstack"
        | "focuswindow"
        | "exchange_client"
        | "sticky_window" => "Windows",
        "screenshot"
        | "screenshot-window"
        | "screenshot-region"
        | "screenshot-region-ui"
        | "screenshot-output"
        | "theme"
        | "twilight_toggle"
        | "twilight_set" => "Shell",
        "reload" | "quit" | "force_unlock" | "session_save" | "session_load" | "setkeymode"
        | "run_script" => "System",
        _ => "General",
    }
}

// ── Migration + writing ─────────────────────────────────────────────────────
/// `binds.conf` exists and `config.conf` already sources it.
fn is_migrated() -> bool {
    if !binds_path().exists() {
        return false;
    }
    let Ok(text) = std::fs::read_to_string(config_path()) else {
        return false;
    };
    text.lines().any(|l| {
        let t = l.trim();
        (t.starts_with("source") || t.starts_with("include"))
            && t.contains("binds.conf")
            && !t.starts_with('#')
    })
}

/// Move every inline `bind*` line out of `config.conf` into a fresh
/// `binds.conf`, leaving a `source = binds.conf` behind. Backs the original up
/// to `config.conf.bak`. Idempotent: a no-op once migrated.
fn migrate(all: &[Bind]) -> std::io::Result<()> {
    if is_migrated() {
        return Ok(());
    }
    let cfg = config_path();
    let text = std::fs::read_to_string(&cfg)?;

    // Back up the original (follows the symlink → copies target content).
    let _ = std::fs::copy(&cfg, cfg.with_extension("conf.bak"));

    write_binds_conf(all)?;
    std::fs::write(&cfg, plan_config_rewrite(&text))
}

/// Pure core of the migration: strip every inline `bind*` line and ensure a
/// single `source = binds.conf` is present (inserted after the last existing
/// `source`/`include`, else where the first bind was). No IO — testable.
fn plan_config_rewrite(text: &str) -> String {
    let mut kept: Vec<String> = Vec::new();
    let mut first_bind_at: Option<usize> = None;
    let mut last_source_at: Option<usize> = None;
    let mut already_sources = false;
    for line in text.lines() {
        let t = line.trim();
        let is_bind = t.find('=').is_some_and(|eq| is_bind_key(t[..eq].trim()));
        if is_bind {
            first_bind_at.get_or_insert(kept.len());
            continue;
        }
        if (t.starts_with("source") || t.starts_with("include")) && !t.starts_with('#') {
            last_source_at = Some(kept.len());
            if t.contains("binds.conf") {
                already_sources = true;
            }
        }
        kept.push(line.to_string());
    }

    if !already_sources {
        let at = last_source_at
            .map(|i| i + 1)
            .or(first_bind_at)
            .unwrap_or(kept.len());
        kept.insert(at.min(kept.len()), "source = binds.conf".to_string());
    }

    let mut joined = kept.join("\n");
    if !joined.ends_with('\n') {
        joined.push('\n');
    }
    joined
}

/// Render `binds.conf`, grouped by category with section headers. Pure.
fn render_binds_conf(binds: &[Bind]) -> String {
    let mut sorted = binds.to_vec();
    sorted.sort_by(sort_key);

    let mut out = String::new();
    out.push_str("# Managed by mshell Settings → Keybinds.\n");
    out.push_str("# Edits here are overwritten by the editor; the syntax is the same as\n");
    out.push_str("# config.conf: bind = MODS,KEY,ACTION[,ARGS]   #\"description\"\n");

    let mut last_cat = "";
    for b in &sorted {
        let cat = b.category();
        if cat != last_cat {
            out.push_str(&format!("\n# {cat}\n"));
            last_cat = cat;
        }
        out.push_str(&b.to_line());
        out.push('\n');
    }
    out
}

/// Full rewrite of `binds.conf` (atomic via temp + rename).
fn write_binds_conf(binds: &[Bind]) -> std::io::Result<()> {
    let path = binds_path();
    let tmp = path.with_extension("conf.tmp");
    std::fs::write(&tmp, render_binds_conf(binds))?;
    std::fs::rename(&tmp, &path)
}

/// Migrate-if-needed, write `binds.conf`, reload the compositor.
pub(crate) fn persist(binds: &[Bind]) {
    if let Err(e) = migrate(binds).and_then(|_| write_binds_conf(binds)) {
        tracing::warn!(error = %e, "keybinds: failed to write binds.conf");
        return;
    }
    reload();
}

fn reload() {
    match std::process::Command::new("mctl").args(["reload"]).spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(e) => tracing::warn!(error = %e, "keybinds: `mctl reload` failed to spawn"),
    }
}

// ── Component ────────────────────────────────────────────────────────────────
#[derive(Clone, Copy, Debug, PartialEq)]
enum Mode {
    List,
    /// Editing an existing bind (index into `binds`) or adding a new one.
    Edit(Option<usize>),
}

pub(crate) struct KeybindsSettingsModel {
    binds: Vec<Bind>,
    mode: Mode,
    /// Cached: editor already owns `binds.conf`. Refreshed after a migration.
    migrated: bool,
    /// Cached hero subtitle (count + migration state).
    subtitle: String,
    /// Search text, shared with the list filter-func closure.
    filter: Rc<RefCell<String>>,
    /// Per-row search haystacks, indexed in list-build (row) order.
    haystacks: Rc<RefCell<Vec<String>>>,
    /// Row position → `binds` index (`None` for category headers).
    row_map: Rc<RefCell<Vec<Option<usize>>>>,
    /// Whether key-capture is currently armed (shared with the controller).
    capturing: Rc<RefCell<bool>>,
    /// Action picker entries (name + hint), incl. any fly-in unknown action.
    actions: Vec<(String, String)>,

    // widget refs
    list_box: gtk::ListBox,
    stack: gtk::Stack,
    super_btn: gtk::ToggleButton,
    ctrl_btn: gtk::ToggleButton,
    shift_btn: gtk::ToggleButton,
    alt_btn: gtk::ToggleButton,
    key_entry: gtk::Entry,
    capture_btn: gtk::ToggleButton,
    action_dd: gtk::DropDown,
    action_model: gtk::StringList,
    args_entry: gtk::Entry,
    args_hint: gtk::Label,
    desc_entry: gtk::Entry,
    delete_btn: gtk::Button,
    edit_title: gtk::Label,
}

impl std::fmt::Debug for KeybindsSettingsModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeybindsSettingsModel")
            .field("binds", &self.binds.len())
            .field("mode", &self.mode)
            .finish()
    }
}

#[derive(Debug)]
pub(crate) enum KeybindsSettingsInput {
    Search(String),
    AddNew,
    Edit(usize),
    ActionChanged,
    StartCapture(bool),
    KeyCaptured {
        name: String,
        sup: bool,
        ctrl: bool,
        shift: bool,
        alt: bool,
    },
    Save,
    Cancel,
    Delete,
}

pub(crate) struct KeybindsSettingsInit {}

#[derive(Debug)]
pub(crate) enum KeybindsSettingsCommandOutput {}

#[relm4::component(pub)]
impl Component for KeybindsSettingsModel {
    type CommandOutput = KeybindsSettingsCommandOutput;
    type Input = KeybindsSettingsInput;
    type Output = ();
    type Init = KeybindsSettingsInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "settings-page",
            set_orientation: gtk::Orientation::Vertical,
            set_hexpand: true,
            set_vexpand: true,
            set_spacing: 12,

            gtk::Box {
                add_css_class: "settings-hero",
                set_orientation: gtk::Orientation::Horizontal,
                set_halign: gtk::Align::Start,
                set_spacing: 16,
                gtk::Image {
                    add_css_class: "settings-hero-icon",
                    set_icon_name: Some("input-keyboard-symbolic"),
                    set_valign: gtk::Align::Center,
                },
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_valign: gtk::Align::Center,
                    gtk::Label {
                        add_css_class: "settings-hero-title",
                        set_label: "Keybinds",
                        set_halign: gtk::Align::Start,
                    },
                    gtk::Label {
                        add_css_class: "settings-hero-subtitle",
                        set_halign: gtk::Align::Start,
                        set_xalign: 0.0,
                        set_wrap: true,
                        #[watch]
                        set_label: model.subtitle.as_str(),
                    },
                },
            },

            #[local_ref]
            stack -> gtk::Stack {
                set_vexpand: true,
                set_transition_type: gtk::StackTransitionType::Crossfade,

                // ── List ──
                add_named[Some("list")] = &gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 10,
                    set_vexpand: true,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 8,
                        gtk::SearchEntry {
                            add_css_class: "keybinds-search",
                            set_hexpand: true,
                            set_placeholder_text: Some("Search shortcuts…"),
                            connect_search_changed[sender] => move |e| {
                                sender.input(KeybindsSettingsInput::Search(e.text().to_string()));
                            },
                        },
                        gtk::Button {
                            add_css_class: "ok-button-primary",
                            set_label: "Add",
                            connect_clicked[sender] => move |_| {
                                sender.input(KeybindsSettingsInput::AddNew);
                            },
                        },
                    },

                    gtk::ScrolledWindow {
                        set_vscrollbar_policy: gtk::PolicyType::Automatic,
                        set_hscrollbar_policy: gtk::PolicyType::Never,
                        set_vexpand: true,
                        #[local_ref]
                        list_box -> gtk::ListBox {
                            add_css_class: "keybinds-list",
                            set_selection_mode: gtk::SelectionMode::None,
                            connect_row_activated[sender] => move |_, row| {
                                sender.input(KeybindsSettingsInput::Edit(row.index() as usize));
                            },
                        },
                    },
                },

                // ── Editor ──
                add_named[Some("edit")] = &gtk::ScrolledWindow {
                    set_vscrollbar_policy: gtk::PolicyType::Automatic,
                    set_hscrollbar_policy: gtk::PolicyType::Never,
                    set_vexpand: true,
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_spacing: 14,

                        #[local_ref]
                        edit_title -> gtk::Label {
                            add_css_class: "label-large-bold",
                            set_halign: gtk::Align::Start,
                            set_label: "Edit shortcut",
                        },

                        gtk::Label {
                            add_css_class: "label-small",
                            set_label: "Modifiers",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Box {
                            set_orientation: gtk::Orientation::Horizontal,
                            set_spacing: 8,
                            #[local_ref] super_btn -> gtk::ToggleButton {
                                add_css_class: "ok-button-surface", set_label: "Super",
                            },
                            #[local_ref] ctrl_btn -> gtk::ToggleButton {
                                add_css_class: "ok-button-surface", set_label: "Ctrl",
                            },
                            #[local_ref] alt_btn -> gtk::ToggleButton {
                                add_css_class: "ok-button-surface", set_label: "Alt",
                            },
                            #[local_ref] shift_btn -> gtk::ToggleButton {
                                add_css_class: "ok-button-surface", set_label: "Shift",
                            },
                        },

                        gtk::Label {
                            add_css_class: "label-small",
                            set_label: "Key",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Box {
                            set_orientation: gtk::Orientation::Horizontal,
                            set_spacing: 8,
                            #[local_ref] key_entry -> gtk::Entry {
                                set_hexpand: true,
                                set_placeholder_text: Some("e.g. Return, d, Print, XF86AudioPlay"),
                            },
                            #[local_ref] capture_btn -> gtk::ToggleButton {
                                add_css_class: "ok-button-surface",
                                set_label: "Press a key…",
                                connect_toggled[sender] => move |b| {
                                    sender.input(KeybindsSettingsInput::StartCapture(b.is_active()));
                                },
                            },
                        },

                        gtk::Label {
                            add_css_class: "label-small",
                            set_label: "Action",
                            set_halign: gtk::Align::Start,
                        },
                        #[local_ref] action_dd -> gtk::DropDown {
                            set_enable_search: true,
                            connect_selected_notify[sender] => move |_| {
                                sender.input(KeybindsSettingsInput::ActionChanged);
                            },
                        },

                        gtk::Label {
                            add_css_class: "label-small",
                            set_label: "Arguments",
                            set_halign: gtk::Align::Start,
                        },
                        #[local_ref] args_entry -> gtk::Entry {
                            set_hexpand: true,
                        },
                        #[local_ref] args_hint -> gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },

                        gtk::Label {
                            add_css_class: "label-small",
                            set_label: "Description (optional)",
                            set_halign: gtk::Align::Start,
                        },
                        #[local_ref] desc_entry -> gtk::Entry {
                            set_hexpand: true,
                            set_placeholder_text: Some("Shown in the keybinds cheatsheet"),
                        },

                        gtk::Box {
                            set_orientation: gtk::Orientation::Horizontal,
                            set_spacing: 8,
                            set_margin_top: 6,
                            gtk::Button {
                                add_css_class: "ok-button-primary",
                                set_label: "Save",
                                connect_clicked[sender] => move |_| {
                                    sender.input(KeybindsSettingsInput::Save);
                                },
                            },
                            gtk::Button {
                                add_css_class: "ok-button-surface",
                                set_label: "Cancel",
                                connect_clicked[sender] => move |_| {
                                    sender.input(KeybindsSettingsInput::Cancel);
                                },
                            },
                            gtk::Box { set_hexpand: true },
                            #[local_ref] delete_btn -> gtk::Button {
                                add_css_class: "ok-button-surface",
                                add_css_class: "destructive",
                                set_label: "Delete",
                                connect_clicked[sender] => move |_| {
                                    sender.input(KeybindsSettingsInput::Delete);
                                },
                            },
                        },
                    },
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let actions: Vec<(String, String)> = ACTIONS
            .iter()
            .map(|(n, h)| (n.to_string(), h.to_string()))
            .collect();
        let action_model = gtk::StringList::new(&[]);
        for (n, _) in &actions {
            action_model.append(n);
        }

        let binds = load_binds();
        let migrated = is_migrated();
        let model = KeybindsSettingsModel {
            subtitle: make_subtitle(migrated, binds.len()),
            binds,
            mode: Mode::List,
            migrated,
            filter: Rc::new(RefCell::new(String::new())),
            haystacks: Rc::new(RefCell::new(Vec::new())),
            row_map: Rc::new(RefCell::new(Vec::new())),
            capturing: Rc::new(RefCell::new(false)),
            actions,
            list_box: gtk::ListBox::new(),
            stack: gtk::Stack::new(),
            super_btn: gtk::ToggleButton::new(),
            ctrl_btn: gtk::ToggleButton::new(),
            shift_btn: gtk::ToggleButton::new(),
            alt_btn: gtk::ToggleButton::new(),
            key_entry: gtk::Entry::new(),
            capture_btn: gtk::ToggleButton::new(),
            action_dd: gtk::DropDown::new(Some(action_model.clone()), None::<gtk::Expression>),
            action_model,
            args_entry: gtk::Entry::new(),
            args_hint: gtk::Label::new(None),
            desc_entry: gtk::Entry::new(),
            delete_btn: gtk::Button::new(),
            edit_title: gtk::Label::new(None),
        };

        let list_box = &model.list_box;
        let stack = &model.stack;
        let super_btn = &model.super_btn;
        let ctrl_btn = &model.ctrl_btn;
        let shift_btn = &model.shift_btn;
        let alt_btn = &model.alt_btn;
        let key_entry = &model.key_entry;
        let capture_btn = &model.capture_btn;
        let action_dd = &model.action_dd;
        let args_entry = &model.args_entry;
        let args_hint = &model.args_hint;
        let desc_entry = &model.desc_entry;
        let delete_btn = &model.delete_btn;
        let edit_title = &model.edit_title;

        let widgets = view_output!();

        // Filter-func: read shared search text + per-row haystack by index.
        {
            let filter = model.filter.clone();
            let haystacks = model.haystacks.clone();
            model.list_box.set_filter_func(move |row| {
                let q = filter.borrow();
                if q.is_empty() {
                    return true;
                }
                let idx = row.index() as usize;
                haystacks
                    .borrow()
                    .get(idx)
                    .map(|h| h.contains(&*q))
                    .unwrap_or(true)
            });
        }

        // Key-capture controller (capture phase, so it beats the entry).
        {
            let capturing = model.capturing.clone();
            let s = sender.clone();
            let ctrl = gtk::EventControllerKey::new();
            ctrl.set_propagation_phase(gtk::PropagationPhase::Capture);
            ctrl.connect_key_pressed(move |_, keyval, _, state| {
                if !*capturing.borrow() {
                    return glib::Propagation::Proceed;
                }
                let Some(name) = keyval.name() else {
                    return glib::Propagation::Stop;
                };
                let n = name.to_string();
                // Ignore bare modifier presses — wait for a real trigger key.
                if n.starts_with("Super")
                    || n.starts_with("Control")
                    || n.starts_with("Shift")
                    || n.starts_with("Alt")
                    || n.starts_with("Meta")
                    || n.starts_with("Hyper")
                    || n.starts_with("ISO_")
                    || n == "Mode_switch"
                {
                    return glib::Propagation::Stop;
                }
                s.input(KeybindsSettingsInput::KeyCaptured {
                    name: n,
                    sup: state.contains(gdk::ModifierType::SUPER_MASK),
                    ctrl: state.contains(gdk::ModifierType::CONTROL_MASK),
                    shift: state.contains(gdk::ModifierType::SHIFT_MASK),
                    alt: state.contains(gdk::ModifierType::ALT_MASK),
                });
                glib::Propagation::Stop
            });
            model.stack.add_controller(ctrl);
        }

        rebuild_list(&model);
        sync_action_hint(&model);
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            KeybindsSettingsInput::Search(q) => {
                *self.filter.borrow_mut() = q.to_lowercase();
                self.list_box.invalidate_filter();
            }
            KeybindsSettingsInput::AddNew => {
                self.mode = Mode::Edit(None);
                self.load_draft(&Bind {
                    action: "spawn".into(),
                    ..Default::default()
                });
                self.delete_btn.set_visible(false);
                self.edit_title.set_label("New shortcut");
                self.stack.set_visible_child_name("edit");
            }
            KeybindsSettingsInput::Edit(row_pos) => {
                // The activated row position counts category headers; resolve
                // it to the `binds` index via the row map.
                let bind_idx = self.row_map.borrow().get(row_pos).copied().flatten();
                if let Some(idx) = bind_idx
                    && let Some(b) = self.binds.get(idx).cloned()
                {
                    self.mode = Mode::Edit(Some(idx));
                    self.load_draft(&b);
                    self.delete_btn.set_visible(true);
                    self.edit_title.set_label("Edit shortcut");
                    self.stack.set_visible_child_name("edit");
                }
            }
            KeybindsSettingsInput::ActionChanged => sync_action_hint(self),
            KeybindsSettingsInput::StartCapture(on) => {
                *self.capturing.borrow_mut() = on;
                if on {
                    self.capture_btn.set_label("Press now…");
                } else {
                    self.capture_btn.set_label("Press a key…");
                }
            }
            KeybindsSettingsInput::KeyCaptured {
                name,
                sup,
                ctrl,
                shift,
                alt,
            } => {
                self.key_entry.set_text(&name);
                self.super_btn.set_active(sup);
                self.ctrl_btn.set_active(ctrl);
                self.shift_btn.set_active(shift);
                self.alt_btn.set_active(alt);
                *self.capturing.borrow_mut() = false;
                self.capture_btn.set_active(false);
                self.capture_btn.set_label("Press a key…");
            }
            KeybindsSettingsInput::Save => {
                let Some(bind) = self.read_draft() else {
                    return; // invalid (no key/action) — keep editing
                };
                match self.mode {
                    Mode::Edit(Some(idx)) if idx < self.binds.len() => self.binds[idx] = bind,
                    _ => self.binds.push(bind),
                }
                self.binds.sort_by(sort_key);
                persist(&self.binds);
                self.migrated = true;
                self.subtitle = make_subtitle(true, self.binds.len());
                self.mode = Mode::List;
                rebuild_list(self);
                self.stack.set_visible_child_name("list");
            }
            KeybindsSettingsInput::Cancel => {
                *self.capturing.borrow_mut() = false;
                self.capture_btn.set_active(false);
                self.mode = Mode::List;
                self.stack.set_visible_child_name("list");
            }
            KeybindsSettingsInput::Delete => {
                if let Mode::Edit(Some(idx)) = self.mode
                    && idx < self.binds.len()
                {
                    self.binds.remove(idx);
                    persist(&self.binds);
                    self.migrated = true;
                    self.subtitle = make_subtitle(true, self.binds.len());
                }
                self.mode = Mode::List;
                rebuild_list(self);
                self.stack.set_visible_child_name("list");
            }
        }
    }
}

impl KeybindsSettingsModel {
    /// Push a bind into the editor widgets.
    fn load_draft(&mut self, b: &Bind) {
        self.super_btn.set_active(b.m_super);
        self.ctrl_btn.set_active(b.m_ctrl);
        self.shift_btn.set_active(b.m_shift);
        self.alt_btn.set_active(b.m_alt);
        self.key_entry.set_text(&b.key);
        self.args_entry.set_text(&b.args);
        self.desc_entry.set_text(&b.desc);

        // Select the action, adding it to the picker if unfamiliar.
        let pos = self.actions.iter().position(|(n, _)| n == &b.action);
        let idx = match pos {
            Some(i) => i,
            None if !b.action.is_empty() => {
                self.actions
                    .push((b.action.clone(), "custom action".into()));
                self.action_model.append(&b.action);
                self.actions.len() - 1
            }
            None => 0,
        };
        self.action_dd.set_selected(idx as u32);
        sync_action_hint(self);
    }

    /// Read the editor widgets back into a [`Bind`]. `None` if incomplete.
    fn read_draft(&self) -> Option<Bind> {
        let key = self.key_entry.text().trim().to_string();
        let aidx = self.action_dd.selected() as usize;
        let action = self
            .actions
            .get(aidx)
            .map(|(n, _)| n.clone())
            .unwrap_or_default();
        if key.is_empty() || action.is_empty() {
            return None;
        }
        Some(Bind {
            flags: String::new(),
            m_super: self.super_btn.is_active(),
            m_ctrl: self.ctrl_btn.is_active(),
            m_shift: self.shift_btn.is_active(),
            m_alt: self.alt_btn.is_active(),
            key,
            action,
            args: self.args_entry.text().trim().to_string(),
            desc: self.desc_entry.text().trim().to_string(),
        })
    }
}

/// Subtitle reflects count + whether the editor owns `binds.conf` yet.
fn make_subtitle(migrated: bool, count: usize) -> String {
    if migrated {
        format!("{count} shortcuts · editing binds.conf")
    } else {
        format!(
            "{count} shortcuts · the first edit moves them into binds.conf (config.conf backed up)"
        )
    }
}

/// Update the argument hint + placeholder from the selected action.
fn sync_action_hint(model: &KeybindsSettingsModel) {
    let idx = model.action_dd.selected() as usize;
    if let Some((name, hint)) = model.actions.get(idx) {
        model.args_hint.set_label(hint);
        let no_args = hint.starts_with("no argument");
        model.args_entry.set_sensitive(!no_args);
        model.args_entry.set_placeholder_text(if no_args {
            Some("—")
        } else {
            Some(hint.as_str())
        });
        if name == "spawn" || name == "exec" || name == "run_script" {
            model
                .args_entry
                .set_placeholder_text(Some("Command — e.g. kitty"));
        }
    }
}

/// Rebuild every list row from `model.binds` (only on CRUD, never on search).
fn rebuild_list(model: &KeybindsSettingsModel) {
    while let Some(child) = model.list_box.first_child() {
        model.list_box.remove(&child);
    }
    let mut haystacks = Vec::with_capacity(model.binds.len());
    let mut row_map: Vec<Option<usize>> = Vec::with_capacity(model.binds.len());
    let mut last_cat = "";
    for (i, b) in model.binds.iter().enumerate() {
        // A non-selectable category header row when the category changes.
        let cat = b.category();
        if cat != last_cat {
            let header = gtk::ListBoxRow::new();
            header.set_selectable(false);
            header.set_activatable(false);
            let lbl = gtk::Label::new(Some(cat));
            lbl.add_css_class("keybinds-section-label");
            lbl.set_halign(gtk::Align::Start);
            header.set_child(Some(&lbl));
            model.list_box.append(&header);
            haystacks.push(String::new()); // headers stay hidden while searching
            row_map.push(None);
            last_cat = cat;
        }
        model.list_box.append(&bind_row(b));
        haystacks.push(b.haystack());
        row_map.push(Some(i));
    }
    *model.haystacks.borrow_mut() = haystacks;
    *model.row_map.borrow_mut() = row_map;
    model.list_box.invalidate_filter();
}

/// One shortcut row: modifier + key chips on the left, action + description on
/// the right. Reuses the cheatsheet's `.keybind-*` chip styling.
fn bind_row(b: &Bind) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.add_css_class("keybinds-row");

    let hbox = gtk::Box::new(gtk::Orientation::Horizontal, 12);

    let combo = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    combo.set_valign(gtk::Align::Center);
    let add_chip = |text: &str, mod_class: Option<&str>| {
        let l = gtk::Label::new(Some(text));
        l.add_css_class("keybind-chip");
        match mod_class {
            Some(m) => {
                l.add_css_class("keybind-mod");
                l.add_css_class(m);
            }
            None => l.add_css_class("keybind-key"),
        }
        combo.append(&l);
    };
    if b.m_super {
        add_chip("Super", Some("mod-super"));
    }
    if b.m_ctrl {
        add_chip("Ctrl", Some("mod-ctrl"));
    }
    if b.m_alt {
        add_chip("Alt", Some("mod-alt"));
    }
    if b.m_shift {
        add_chip("Shift", Some("mod-shift"));
    }
    add_chip(&b.pretty_key(), None);
    // Fixed-ish width so the action column lines up.
    combo.set_size_request(190, -1);
    combo.set_halign(gtk::Align::Start);
    hbox.append(&combo);

    let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    let action_lbl = gtk::Label::new(Some(&action_summary(b)));
    action_lbl.add_css_class("label-medium-bold");
    action_lbl.set_halign(gtk::Align::Start);
    action_lbl.set_xalign(0.0);
    action_lbl.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text.append(&action_lbl);
    if !b.desc.is_empty() {
        let d = gtk::Label::new(Some(&b.desc));
        d.add_css_class("keybinds-desc");
        d.set_halign(gtk::Align::Start);
        d.set_xalign(0.0);
        d.set_ellipsize(gtk::pango::EllipsizeMode::End);
        text.append(&d);
    }
    hbox.append(&text);

    let chevron = gtk::Image::from_icon_name("go-next-symbolic");
    chevron.add_css_class("dim-label");
    chevron.set_valign(gtk::Align::Center);
    hbox.append(&chevron);

    row.set_child(Some(&hbox));
    row
}

/// "spawn → kitty" / "view 2" / "Toggle Fullscreen" — a compact action label.
fn action_summary(b: &Bind) -> String {
    let args = b.args.trim();
    match b.action.as_str() {
        "spawn" | "exec" | "run_script" if !args.is_empty() => {
            let cmd = args
                .split_whitespace()
                .take(5)
                .collect::<Vec<_>>()
                .join(" ");
            format!("{} → {}", b.action, cmd)
        }
        _ if !args.is_empty() => format!("{} {}", b.action, args),
        _ => b.action.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bind_key_matcher() {
        assert!(is_bind_key("bind"));
        assert!(is_bind_key("binds"));
        assert!(is_bind_key("bindr"));
        assert!(is_bind_key("bindlrp"));
        assert!(!is_bind_key("bindx"));
        assert!(!is_bind_key("mousebind"));
        assert!(!is_bind_key("gesturebind"));
        assert!(!is_bind_key("windowrule"));
        assert!(!is_bind_key("animations"));
    }

    #[test]
    fn parses_a_plain_spawn() {
        let b = parse_bind_line("bind = super,Return,spawn,uwsm app -a kitty -- /usr/bin/kitty")
            .expect("parse");
        assert!(b.m_super && !b.m_ctrl && !b.m_alt && !b.m_shift);
        assert_eq!(b.key, "Return");
        assert_eq!(b.action, "spawn");
        assert_eq!(b.args, "uwsm app -a kitty -- /usr/bin/kitty");
        assert_eq!(b.mods_str(), "super");
    }

    #[test]
    fn no_mods_become_none() {
        let b = parse_bind_line("bind = NONE,Print,screenshot-region-ui").expect("parse");
        assert_eq!(b.mods_str(), "NONE");
        assert!(b.args.is_empty());
        assert_eq!(b.to_line(), "bind = NONE,Print,screenshot-region-ui");
    }

    #[test]
    fn multi_arg_summon_survives_commas() {
        // The 8th split field keeps embedded commas; regex args stay intact.
        let line =
            "bind = alt,2,summon,^(TmuxKenp|kitty)$,^Tmux$,uwsm app -a TmuxKenp -- start-kkenp";
        let b = parse_bind_line(line).expect("parse");
        assert_eq!(b.action, "summon");
        assert_eq!(
            b.args,
            "^(TmuxKenp|kitty)$,^Tmux$,uwsm app -a TmuxKenp -- start-kkenp"
        );
        // Round-trips byte-for-byte (input already in canonical comma form).
        assert_eq!(b.to_line(), line);
    }

    #[test]
    fn round_trip_is_stable() {
        // Parse → regenerate → re-parse must be a fixed point.
        for line in [
            "bind = super+ctrl,f,spawn,nautilus",
            "bind = super,q,killclient",
            "bind = super+shift,j,exchange_client,down",
            "bind = super+alt,v,toggle_named_scratchpad,^clipse$,none,kitty --class clipse -e clipse",
        ] {
            let a = parse_bind_line(line).expect("parse");
            let regen = a.to_line();
            let b = parse_bind_line(&regen).expect("re-parse");
            assert_eq!(a.mods_str(), b.mods_str());
            assert_eq!(a.key, b.key);
            assert_eq!(a.action, b.action);
            assert_eq!(a.args, b.args);
            assert_eq!(regen, b.to_line(), "second regen differs for {line}");
        }
    }

    #[test]
    fn description_comment_extracted_and_reemitted() {
        let b = parse_bind_line("bind = super,d,spawn,fuzzel  #\"App launcher\"").expect("parse");
        assert_eq!(b.desc, "App launcher");
        assert!(b.to_line().contains("#\"App launcher\""));
        // The action body before the comment is clean.
        assert!(b.to_line().starts_with("bind = super,d,spawn,fuzzel"));
    }

    /// A config shaped like the real one: header, options, a `source`, and a
    /// run of binds incl. a tricky multi-arg scratchpad + a description.
    const SAMPLE: &str = "\
# margo config
gaps_out = 8
source = colors.conf

bind = super,Return,spawn,kitty
bind = super+ctrl,f,spawn,nautilus
bind = super,q,killclient
bind = alt,2,summon,^(TmuxKenp|kitty)$,^Tmux$,uwsm app -a TmuxKenp -- start-kkenp
bind = super,d,spawn,fuzzel  #\"App launcher\"
bind = NONE,Print,screenshot-region-ui

# layouts
bind = super,t,setlayout,tile
windowrule = float,^(pavucontrol)$
env = FOO,bar
";

    #[test]
    fn migration_moves_binds_and_keeps_everything_else() {
        let binds: Vec<Bind> = SAMPLE.lines().filter_map(parse_bind_line).collect();
        assert_eq!(binds.len(), 7, "all bind lines parsed");

        let new_cfg = plan_config_rewrite(SAMPLE);

        // No bind line remains in config.conf …
        assert!(
            !new_cfg.lines().any(|l| {
                l.trim()
                    .find('=')
                    .is_some_and(|eq| is_bind_key(l.trim()[..eq].trim()))
            }),
            "config still has a bind line:\n{new_cfg}"
        );
        // … exactly one `source = binds.conf`, placed after the colors source …
        assert_eq!(new_cfg.matches("source = binds.conf").count(), 1);
        let so_colors = new_cfg.find("source = colors.conf").unwrap();
        let so_binds = new_cfg.find("source = binds.conf").unwrap();
        assert!(so_binds > so_colors, "binds.conf sourced after colors.conf");
        // … and every non-bind line survived verbatim.
        for keep in [
            "gaps_out = 8",
            "source = colors.conf",
            "windowrule = float,^(pavucontrol)$",
            "env = FOO,bar",
            "# layouts",
        ] {
            assert!(new_cfg.contains(keep), "lost line: {keep}");
        }

        // binds.conf re-parses to the same set, grouped under headers.
        let rendered = render_binds_conf(&binds);
        let reparsed: Vec<Bind> = rendered.lines().filter_map(parse_bind_line).collect();
        assert_eq!(reparsed.len(), binds.len(), "no bind lost in binds.conf");
        assert!(rendered.contains("# Scratchpad"));
        assert!(rendered.contains("summon,^(TmuxKenp|kitty)$,^Tmux$,"));
    }

    #[test]
    fn rewrite_is_idempotent_once_sourced() {
        let once = plan_config_rewrite(SAMPLE);
        let twice = plan_config_rewrite(&once);
        assert_eq!(once.matches("source = binds.conf").count(), 1);
        assert_eq!(
            twice.matches("source = binds.conf").count(),
            1,
            "no duplicate source on re-run"
        );
    }

    #[test]
    fn categories_match_actions() {
        assert_eq!(categorise("killclient", ""), "Windows");
        assert_eq!(categorise("view", "2"), "Workspaces");
        assert_eq!(categorise("setlayout", "tile"), "Layout");
        assert_eq!(categorise("summon", "^x$"), "Scratchpad");
        assert_eq!(
            categorise("spawn", "wpctl set-volume @DEFAULT_AUDIO_SINK@ 5%+"),
            "Media"
        );
        assert_eq!(categorise("spawn", "kitty"), "Launch");
    }
}
