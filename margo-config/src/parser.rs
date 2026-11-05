use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// `var=name,value` definitions accumulated while parsing — shared across
/// `source`/`include`'d files (a file can use a variable defined before its
/// `source` line). Threaded through the recursive parse calls the same way
/// `visited` already is.
pub(crate) type Vars = HashMap<String, String>;

use anyhow::{Result, anyhow, bail};
use tracing::{error, warn};

use crate::types::*;

mod binds;
mod options;
mod rules;
mod values;

pub(crate) use binds::parse_key;
use binds::{
    inject_default_chvt_bindings, parse_axisbind, parse_bind, parse_gesturebind, parse_mousebind,
    parse_switchbind, parse_touchgesturebind,
};
pub use binds::{keysym_raw_from_name, parse_modifiers};
pub use options::OPTION_KEYS;
use options::{parse_env, parse_option};
pub use rules::{
    LAYERRULE_KEYS, LAYERRULE_MATCHERS, MONITORRULE_KEYS, TAGRULE_KEYS, WINDOWRULE_KEYS,
    WINDOWRULE_MATCHERS,
};
use rules::{parse_layerrule, parse_monitorrule, parse_tagrule, parse_windowrule};

// ── Public entry point ───────────────────────────────────────────────────────

/// Parse the user's mango config file (auto-discovers path if none given).
pub fn parse_config(path: Option<&Path>) -> Result<Config> {
    parse_config_tracked(path).map(|(cfg, _)| cfg)
}

fn parse_config_tracked(path: Option<&Path>) -> Result<(Config, Vec<PathBuf>)> {
    let resolved = resolve_config_path(path)?;
    let mut cfg = Config::default();
    let mut visited = HashSet::new();
    parse_file(&mut cfg, &resolved, true, &mut visited, &mut Vars::new())?;
    inject_default_chvt_bindings(&mut cfg);
    let mut sources: Vec<PathBuf> = visited.into_iter().collect();
    sources.sort();
    Ok((cfg, sources))
}

/// Every file this config currently reads, transitively through
/// `include`/`source` (main file first) — the watch-list for margo's
/// config auto-reload (`auto_reload_config`). A config's source tree can
/// change across edits (a `source` line added/removed), so callers
/// re-fetch this after every reload rather than caching it once.
pub fn config_source_files(path: Option<&Path>) -> Result<Vec<PathBuf>> {
    parse_config_tracked(path).map(|(_, sources)| sources)
}

/// Like [`parse_config`] but also applies margo's built-in first-party
/// defaults. The compositor uses this; plain [`parse_config`] stays free
/// of them so tooling / tests see a faithful view of the user's file.
pub fn parse_config_with_defaults(path: Option<&Path>) -> Result<Config> {
    let mut cfg = parse_config(path)?;
    apply_first_party_defaults(&mut cfg);
    Ok(cfg)
}

/// [`parse_config_with_defaults`] plus the file list of
/// [`config_source_files`] from the same single parse — what a reload
/// needs to both apply the config and re-sync its watch set.
pub fn parse_config_with_defaults_and_sources(
    path: Option<&Path>,
) -> Result<(Config, Vec<PathBuf>)> {
    let (mut cfg, sources) = parse_config_tracked(path)?;
    apply_first_party_defaults(&mut cfg);
    Ok((cfg, sources))
}

/// Like [`parse_config`] but the top-level content comes from `content`
/// instead of reading `path` (or the default config location) from
/// disk — `path` is still used as the *origin* for resolving relative
/// `source`/`include` directives found inside `content`, which are read
/// from disk exactly as they are for the path-based parser. Used to
/// validate config content that isn't (yet, or any longer) the live
/// file on disk — a saved generation being considered for the boot
/// fallback, or a rollback candidate being checked before it's written.
pub fn parse_config_str(content: &str, path: Option<&Path>) -> Result<Config> {
    let origin = resolve_config_path(path)?;
    let mut cfg = Config::default();
    let mut visited = HashSet::new();
    // Pre-mark `origin` visited so a pathological self-referential
    // `source = <origin's own filename>` line is treated as a cycle
    // (matching parse_file's own guard) instead of re-reading whatever
    // is currently on disk at `origin`, which may not match `content`.
    visited.insert(std::fs::canonicalize(&origin).unwrap_or_else(|_| origin.clone()));
    parse_lines(&mut cfg, content, &origin, &mut visited, &mut Vars::new());
    inject_default_chvt_bindings(&mut cfg);
    Ok(cfg)
}

/// [`parse_config_str`] plus margo's first-party defaults — the
/// string-content counterpart to [`parse_config_with_defaults`].
pub fn parse_config_str_with_defaults(content: &str, path: Option<&Path>) -> Result<Config> {
    let mut cfg = parse_config_str(content, path)?;
    apply_first_party_defaults(&mut cfg);
    Ok(cfg)
}

/// Defaults that are part of the margo suite itself rather than the
/// user's file. Currently none: the setup wizard used to be floated +
/// `exec-once`'d here, but it is now an in-shell layer-shell menu
/// (mshell auto-opens it on first launch, or `mshellctl wizard`), so the
/// compositor no longer launches any wizard window. Kept as the hook for
/// future first-party defaults.
pub fn apply_first_party_defaults(_cfg: &mut Config) {}

// ── Path resolution ──────────────────────────────────────────────────────────

pub fn resolve_config_path(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = explicit {
        return Ok(p.to_path_buf());
    }
    // The shared XDG rule (margo_fs::xdg), so margo reads the config.conf
    // mshell's Settings pages write.
    Ok(margo_fs::xdg::margo_config_dir().join("config.conf"))
}

pub(crate) fn resolve_include_path(include: &str, relative_to: &Path) -> PathBuf {
    // ~/-relative — substitute $HOME.
    let path = if let Some(rest) = include.strip_prefix("~/") {
        let home = std::env::var("HOME").unwrap_or_default();
        PathBuf::from(home).join(rest)
    } else if let Some(rest) = include.strip_prefix("./") {
        // Explicit "./foo" — relative to the parent dir of the
        // file currently being parsed.
        let dir = relative_to.parent().unwrap_or(Path::new("."));
        dir.join(rest)
    } else {
        PathBuf::from(include)
    };

    if path.is_absolute() {
        return path;
    }

    // Bare relative paths (`source = foo.conf`) — resolve
    // against the parent directory of the file being parsed,
    // NOT the process CWD. Without this, `source = mlayout
    // .conf` from `~/.config/margo/config.conf` would look for
    // `./mlayout.conf` relative to wherever margo was
    // launched (typically `/`), silently load nothing, and the
    // sourced rules disappear into the void.
    let dir = relative_to.parent().unwrap_or(Path::new("."));
    dir.join(path)
}

/// What a `source`/`include` value points at.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum IncludeTarget {
    /// Files to load, in order. Empty when a wildcard matched nothing —
    /// a fragments directory with no fragments yet is not an error.
    Files(Vec<PathBuf>),
    /// A plain path that doesn't exist (the resolved path).
    Missing(PathBuf),
}

/// Resolve a `source`/`include` value to the files it loads. A `*` or `?`
/// in the last path component is a wildcard: every regular file in that
/// directory whose name matches is loaded, in lexical order (`*` doesn't
/// match a leading `.`, as in a shell). Wildcards in directory components
/// are not expanded. Shared by the parser and the validator so both read
/// the same files.
pub(crate) fn include_targets(include: &str, relative_to: &Path) -> IncludeTarget {
    let path = resolve_include_path(include, relative_to);
    let pattern = path
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| n.contains(['*', '?']))
        .map(str::to_owned);
    let Some(pattern) = pattern else {
        return if path.exists() {
            IncludeTarget::Files(vec![path])
        } else {
            IncludeTarget::Missing(path)
        };
    };
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|name| wildcard_match(&pattern, name))
        })
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    files.sort();
    IncludeTarget::Files(files)
}

/// Shell-style match of `name` against `pattern` with `*` (any run, not a
/// leading `.`) and `?` (one char). No character classes.
fn wildcard_match(pattern: &str, name: &str) -> bool {
    if name.starts_with('.') && !pattern.starts_with('.') {
        return false;
    }
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    // Iterative matcher with single-star backtracking.
    let (mut pi, mut ni) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some((pi, ni));
            pi += 1;
        } else if let Some((sp, sn)) = star {
            pi = sp + 1;
            ni = sn + 1;
            star = Some((sp, sn + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

// ── File-level parsing ───────────────────────────────────────────────────────

fn parse_file(
    cfg: &mut Config,
    path: &Path,
    required: bool,
    visited: &mut HashSet<PathBuf>,
    vars: &mut Vars,
) -> Result<()> {
    // Guard against include/source cycles (A→B→A, or a file that
    // includes itself) and accidental double-loads of the same file
    // (which would duplicate its binds/rules/exec entries). Key on the
    // canonicalized path so `./foo` and `foo` resolve to one identity.
    let canon = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if !visited.insert(canon) {
        warn!(
            "{}: include/source already parsed (cycle or duplicate) — \
             skipping to avoid infinite recursion",
            path.display()
        );
        return Ok(());
    }
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            if required {
                bail!("cannot open config file {}: {}", path.display(), e);
            }
            // A sourced path that exists but can't be read (a directory, no
            // permission): say so, the validator reports it as E000.
            warn!(
                "cannot read sourced config file {}: {e} — skipping",
                path.display()
            );
            return Ok(());
        }
    };
    parse_lines(cfg, &text, path, visited, vars);
    Ok(())
}

/// Walk `text` line by line, dispatching each non-empty/non-comment line
/// to [`parse_line`]. `origin` is the path `source`/`include` directives
/// in `text` resolve relative to (and, for the path-based caller, is
/// `text`'s own real location; for [`parse_config_str`] it's the
/// location `content` is *pretending* to be). Parse errors on individual
/// lines are logged and skipped, never propagated — matches the
/// permissive-parser contract the rest of the crate already documents.
fn parse_lines(
    cfg: &mut Config,
    text: &str,
    origin: &Path,
    visited: &mut HashSet<PathBuf>,
    vars: &mut Vars,
) {
    for (lineno, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Every warning the line's value parsing emits (a malformed number
        // falling back to 0, an over-long keyword being clamped, an unknown
        // rule option, …) carries the file and line through this span, so the
        // log points at the culprit even inside a `source`d fragment.
        let _location = tracing::warn_span!(
            "config",
            file = %origin.display(),
            line = lineno + 1
        )
        .entered();
        if let Err(e) = parse_line(cfg, line, origin, visited, vars) {
            error!("{}:{}: {} — {:?}", origin.display(), lineno + 1, e, line);
        }
    }
}

/// Strip an inline `#`-comment that begins at a whitespace boundary
/// (`<space|tab>#…` to end of line) — same convention as
/// sshd_config / sysctl.conf. A `#` flush-against text is preserved
/// so we don't eat hex colours (`0x1e1e2eff`), regex anchors
/// (`^foo#bar$`), or URL fragments embedded in spawn commands.
///
/// Whole-line comments (`#` at column 0) are filtered upstream in
/// `parse_file`; this helper handles the inline case only.
///
/// Caveat: a literal `<space>#` inside a single-line shell command
/// (`bind = …,spawn,sh -c 'echo # foo'`) would also be stripped.
/// In practice that pattern doesn't appear in real configs — shell
/// inline-comments-after-space are rare; users wanting them
/// literal can quote-escape (`#` → `\#` in their shell, which the
/// stripper leaves alone since it's not a config-level `#`) or use
/// a wrapper script.
pub(crate) fn strip_inline_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut prev_ws = false;
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'#' && prev_ws && !hex_color_follows(&line[i + 1..]) {
            return line[..i].trim_end();
        }
        prev_ws = b == b' ' || b == b'\t';
    }
    line
}

/// True when the text right after a `#` is a bare hex-colour run
/// (`RGB`/`RGBA`/`RRGGBB`/`RRGGBBAA`) terminated by end-of-line or
/// whitespace. Guards `focuscolor = #c66b25` (matugen-style `#`-hex,
/// space after `=`) from being swallowed as an inline comment, while
/// a real `… # note` (space after `#`, run length 0) still strips.
fn hex_color_follows(rest: &str) -> bool {
    let run = rest
        .as_bytes()
        .iter()
        .take_while(|b| b.is_ascii_hexdigit())
        .count();
    matches!(run, 3 | 4 | 6 | 8) && rest[run..].chars().next().is_none_or(char::is_whitespace)
}

// ── Line-level dispatch ──────────────────────────────────────────────────────

fn parse_line(
    cfg: &mut Config,
    line: &str,
    origin: &Path,
    visited: &mut HashSet<PathBuf>,
    vars: &mut Vars,
) -> Result<()> {
    // Strip inline comments before parsing so users can write e.g.
    //   xkb_rules_options = ctrl:nocaps   # CapsLock → Ctrl
    // without the comment becoming part of the value.
    let line = strip_inline_comment(line);
    let (raw_key, raw_val) = split_kv(line).ok_or_else(|| anyhow!("invalid line format"))?;
    let key = raw_key.trim();
    // Expand `$name`/`${name}` to already-defined `var=` values before any
    // key-specific handling — matches mango: expansion applies uniformly
    // to every line's value, `var=` lines included (so a later var can
    // reference an earlier one), not just plain option values.
    // Quotes come off first (mango `strip_quotes`, before expansion), so
    // `focuscolor = "0xc66b25ff"` and `default_layout = 'tile'` parse.
    let val = expand_vars(strip_enclosing_quotes(raw_val.trim()), vars);
    let val = val.as_str();

    if key == "var" {
        return parse_var(val, vars);
    }

    // include/source directive (a wildcard in the file name loads every
    // match in lexical order)
    if key == "include" || key == "source" {
        match include_targets(val, origin) {
            IncludeTarget::Missing(p) => {
                warn!(
                    "{}: source/include `{}` resolved to `{}` which doesn't exist — \
                     skipping (rules / binds in that file won't load)",
                    origin.display(),
                    val,
                    p.display()
                );
            }
            IncludeTarget::Files(files) => {
                for p in files {
                    parse_file(cfg, &p, false, visited, vars)?;
                }
            }
        }
        return Ok(());
    }

    // bind variants
    if is_bind_key(key) {
        return parse_bind(cfg, key, val);
    }

    match key {
        "mousebind" => return parse_mousebind(cfg, val),
        "axisbind" => return parse_axisbind(cfg, val),
        "switchbind" => return parse_switchbind(cfg, val),
        "gesturebind" => return parse_gesturebind(cfg, val),
        "touchgesturebind" => return parse_touchgesturebind(cfg, val),
        "windowrule" => return parse_windowrule(cfg, val, false),
        "windowrule-once" => return parse_windowrule(cfg, val, true),
        "monitorrule" => return parse_monitorrule(cfg, val),
        "tagrule" => return parse_tagrule(cfg, val),
        "layerrule" => return parse_layerrule(cfg, val),
        "env" => return parse_env(cfg, val),
        "exec" => {
            cfg.exec.push(val.to_string());
            return Ok(());
        }
        "exec-once" => {
            cfg.exec_once.push(val.to_string());
            return Ok(());
        }
        _ => {}
    }

    parse_option(cfg, key, val)
}

/// `var=name,value` — define (or overwrite) a config variable. Names must
/// match `[A-Za-z_][A-Za-z0-9_]*`; anything else is a parse error (reported
/// and skipped, per the permissive-parser contract — this `var=` line is
/// simply dropped, not fatal to the rest of the file).
pub(crate) fn parse_var(val: &str, vars: &mut Vars) -> Result<()> {
    let (name, value) = val
        .split_once(',')
        .ok_or_else(|| anyhow!("invalid var format (expected name,value): {val}"))?;
    let name = name.trim();
    let value = value.trim();
    if !is_valid_var_name(name) {
        bail!("invalid var name (allowed: [A-Za-z_][A-Za-z0-9_]*): {name}");
    }
    vars.insert(name.to_string(), value.to_string());
    Ok(())
}

fn is_valid_var_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Expand `$name` / `${name}` references to already-defined `var=`
/// variables. Only names actually defined via `var=` are touched —
/// anything else (`$HOME`, `$(date …)`, `$1`, `$$`, an undefined `$foo`)
/// is copied through byte-for-byte, so shell commands in
/// `spawn`/`spawn_shell` keep working exactly as before.
pub(crate) fn expand_vars(value: &str, vars: &Vars) -> String {
    if vars.is_empty() || !value.contains('$') {
        return value.to_string();
    }
    let bytes = value.as_bytes();
    let mut out = String::with_capacity(value.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'$' {
            if bytes.get(i + 1) == Some(&b'{') {
                if let Some(close) = value[i + 2..].find('}') {
                    let name = &value[i + 2..i + 2 + close];
                    if let Some(v) = vars.get(name) {
                        out.push_str(v);
                        i += 2 + close + 1;
                        continue;
                    }
                }
            } else if matches!(bytes.get(i + 1), Some(b) if b.is_ascii_alphabetic() || *b == b'_') {
                let start = i + 1;
                let mut end = start;
                while end < bytes.len()
                    && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_')
                {
                    end += 1;
                }
                if let Some(v) = vars.get(&value[start..end]) {
                    out.push_str(v);
                    i = end;
                    continue;
                }
            }
        }
        // Not a recognised/defined variable reference — copy one *char*
        // (not byte) through untouched, so this stays UTF-8 safe for any
        // multibyte text sitting next to an unrelated '$'.
        let ch_len = value[i..].chars().next().map_or(1, char::len_utf8);
        out.push_str(&value[i..i + ch_len]);
        i += ch_len;
    }
    out
}

/// Is `k` a keyboard-bind key margo loads: `bind` plus any of the flag
/// suffixes `s` (keysym), `l` (lock), `r` (release), `p` (pass), `c` (allow
/// conflict)? Public so editors of the config text classify lines the same way.
pub fn is_bind_key(k: &str) -> bool {
    if !k.starts_with("bind") {
        return false;
    }
    k[4..]
        .chars()
        .all(|c| matches!(c, 's' | 'l' | 'r' | 'p' | 'c'))
}

/// Drop one pair of matching enclosing quotes (`"…"` or `'…'`) from an
/// already-trimmed value, then re-trim. Anything else is returned as-is,
/// so a quote inside a value (`spawn,sh -c "x"`) is untouched.
pub(crate) fn strip_enclosing_quotes(val: &str) -> &str {
    let b = val.as_bytes();
    if b.len() >= 2 && (b[0] == b'"' || b[0] == b'\'') && b[b.len() - 1] == b[0] {
        val[1..val.len() - 1].trim()
    } else {
        val
    }
}

fn split_kv(line: &str) -> Option<(&str, &str)> {
    let pos = line.find('=')?;
    Some((&line[..pos], &line[pos + 1..]))
}

#[cfg(test)]
mod tests {
    use super::{
        config_source_files, parse_config, parse_config_str, parse_config_str_with_defaults,
        strip_inline_comment,
    };

    #[test]
    fn var_expands_in_later_lines_and_across_definitions() {
        let c = parse_config_str(
            "var=term,kitty\nvar=mod,super\nenv=TESTVAR,${term}-$mod\n",
            None,
        )
        .unwrap();
        assert_eq!(c.envs.last().map(|(_, v)| v.as_str()), Some("kitty-super"));
    }

    #[test]
    fn enclosing_quotes_are_stripped_from_values() {
        let c = parse_config_str(
            "default_layout = \"scroller\"\nborderpx = ' 7 '\nexec='kitty'\n",
            None,
        )
        .unwrap();
        assert_eq!(c.default_layout, "scroller");
        assert_eq!(c.borderpx, 7);
        assert_eq!(c.exec.first().map(String::as_str), Some("kitty"));
        // Mismatched or inner quotes are left alone.
        let c = parse_config_str("exec=\"a' \nexec=sh -c \"x\"\n", None).unwrap();
        assert_eq!(c.exec, vec!["\"a'".to_string(), "sh -c \"x\"".to_string()]);
    }

    #[test]
    fn var_defined_from_an_earlier_var() {
        let c =
            parse_config_str("var=base,/x\nvar=full,$base/y\nenv=TESTVAR,$full\n", None).unwrap();
        assert_eq!(c.envs.last().map(|(_, v)| v.as_str()), Some("/x/y"));
    }

    #[test]
    fn var_leaves_shell_syntax_and_undefined_names_untouched() {
        // $HOME, $(date …), $1 and $$ are never config variables, and an
        // undefined $foo is left exactly as written — only names actually
        // defined via `var=` get substituted.
        let c = parse_config_str("exec=echo $HOME $(date +%s) $1 $$ $undefined\n", None).unwrap();
        assert_eq!(
            c.exec.first().map(String::as_str),
            Some("echo $HOME $(date +%s) $1 $$ $undefined")
        );
    }

    #[test]
    fn var_bad_name_is_rejected_without_poisoning_later_lines() {
        // "1bad" isn't a valid name ([A-Za-z_][A-Za-z0-9_]*) — the var=
        // line is dropped (permissive-parser contract: an error on one
        // line never aborts the rest of the file), so nothing named
        // "1bad" ever gets defined and `${1bad}` later has nothing to
        // expand to — left exactly as written.
        let c = parse_config_str("var=1bad,oops\nexec=echo ${1bad}\n", None).unwrap();
        assert_eq!(c.exec.first().map(String::as_str), Some("echo ${1bad}"));
    }

    #[test]
    fn inline_comments_after_whitespace_are_stripped() {
        // Common case the bug surfaced for: option list with trailing comment.
        assert_eq!(
            strip_inline_comment("xkb_rules_options = ctrl:nocaps   # CapsLock → Ctrl"),
            "xkb_rules_options = ctrl:nocaps"
        );
        // Tab before `#` should also count.
        assert_eq!(
            strip_inline_comment("repeat_rate = 35\t# faster keys"),
            "repeat_rate = 35"
        );
    }

    #[test]
    fn hex_colour_values_are_not_stripped_as_comments() {
        // Regression: `focuscolor = #c66b25` (matugen-style `#`-hex with a
        // space after `=`) had its whole value eaten as an inline comment,
        // silently resetting the colour to default on every reload.
        for val in [
            "focuscolor = #c66b25",    // RRGGBB
            "bordercolor = #1e1e2eff", // RRGGBBAA
            "col = #abc",              // RGB
            "col = #abcd",             // RGBA
        ] {
            assert_eq!(strip_inline_comment(val), val, "stripped {val:?}");
        }
        // A hex colour followed by a genuine comment keeps the colour, drops
        // the note.
        assert_eq!(
            strip_inline_comment("focuscolor = #c66b25   # accent"),
            "focuscolor = #c66b25"
        );
        // A non-colour token after a space is still a comment.
        assert_eq!(strip_inline_comment("x = 1 # note"), "x = 1");
    }

    #[test]
    fn include_cycle_does_not_recurse_forever() {
        // Regression: a self-including (or A→B→A) config caused unbounded
        // recursion → stack overflow, killing the compositor at startup /
        // `mctl reload`. The visited-set must break the cycle and return Ok.
        let dir = std::env::temp_dir().join(format!("margo-inc-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("config.conf");
        let b = dir.join("b.conf");
        std::fs::write(&a, "include = b.conf\nfocuscolor = #c66b25\n").unwrap();
        std::fs::write(&b, "include = config.conf\n").unwrap();
        // Must not stack-overflow; parse completes and the real option loads.
        let cfg = parse_config(Some(&a)).unwrap();
        assert_eq!(cfg.focuscolor.0[0], 0xc6 as f32 / 255.0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn config_source_files_lists_the_whole_include_tree_once_each() {
        // The config auto-reload watch-list (margo/src/config_watcher.rs)
        // needs every file the config tree actually touches, main file
        // included, each exactly once — even through an A→B→A cycle.
        let dir = std::env::temp_dir().join(format!("margo-srcfiles-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("config.conf");
        let b = dir.join("b.conf");
        std::fs::write(&a, "include = b.conf\nfocuscolor = #c66b25\n").unwrap();
        std::fs::write(&b, "include = config.conf\n").unwrap();

        let files = config_source_files(Some(&a)).unwrap();
        let canon_a = std::fs::canonicalize(&a).unwrap();
        let canon_b = std::fs::canonicalize(&b).unwrap();
        assert_eq!(files.len(), 2, "{files:?}");
        assert!(files.contains(&canon_a));
        assert!(files.contains(&canon_b));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn flush_hash_is_preserved() {
        // Regex `#` flush against neighbour char — common in
        // window-rule title patterns. The space before `=` is
        // followed by `title:^foo#bar$`, no whitespace right
        // before the `#`, so it stays.
        assert_eq!(
            strip_inline_comment("windowrule = title:^foo#bar$"),
            "windowrule = title:^foo#bar$"
        );
        // URL fragment in spawn argument.
        assert_eq!(
            strip_inline_comment("bind = super,d,spawn,xdg-open https://x.org/page#anchor"),
            "bind = super,d,spawn,xdg-open https://x.org/page#anchor"
        );
    }

    #[test]
    fn no_hash_means_unchanged() {
        assert_eq!(
            strip_inline_comment("default_layout = scroller"),
            "default_layout = scroller"
        );
    }

    /// Write `content` to a temp config and return the parsed `Config`.
    /// `unique` keeps parallel test dirs from colliding (tests run on
    /// separate threads inside one process, so the PID alone isn't).
    pub(super) fn parse_conf(unique: &str, content: &str) -> super::Config {
        let dir = std::env::temp_dir().join(format!("margo-cfg-{unique}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let main = dir.join("config.conf");
        std::fs::write(&main, content).unwrap();
        let cfg = parse_config(Some(&main)).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        cfg
    }

    #[test]
    fn parse_config_str_parses_scalar_keys() {
        let cfg = parse_config_str("borderpx = 7\n", None).unwrap();
        assert_eq!(cfg.borderpx, 7);
    }

    #[test]
    fn parse_config_str_with_defaults_matches_plain_variant() {
        // apply_first_party_defaults is currently a no-op, but the
        // `_with_defaults` entry point must still exist and behave
        // identically to plain parse_config_str until that changes.
        let plain = parse_config_str("borderpx = 7\n", None).unwrap();
        let defaulted = parse_config_str_with_defaults("borderpx = 7\n", None).unwrap();
        assert_eq!(plain.borderpx, defaulted.borderpx);
    }

    #[test]
    fn parse_config_str_resolves_relative_source_against_given_origin() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("conf.d")).unwrap();
        std::fs::write(dir.path().join("conf.d/frag.conf"), "borderpx = 9\n").unwrap();
        // `origin` doesn't need to exist on disk itself — only its
        // parent directory is used to resolve the relative `source =`.
        let origin = dir.path().join("config.conf");
        let cfg = parse_config_str("source = conf.d/frag.conf\n", Some(&origin)).unwrap();
        assert_eq!(
            cfg.borderpx, 9,
            "relative source= must resolve against `origin`'s directory"
        );
    }

    #[test]
    fn parse_config_str_self_referential_source_does_not_recurse_forever() {
        // A pathological `source = config.conf` line pointing back at
        // `origin` itself must not infinite-loop or double-parse.
        let dir = tempfile::tempdir().unwrap();
        let origin = dir.path().join("config.conf");
        std::fs::write(&origin, "borderpx = 1\n").unwrap(); // real file, so the cycle path is exercised
        let cfg = parse_config_str("borderpx = 5\nsource = config.conf\n", Some(&origin)).unwrap();
        // `borderpx = 5` applies first; the trailing self-source is then
        // skipped as an already-visited origin, so the on-disk fragment's
        // `borderpx = 1` never overwrites it. Without the pre-seeded
        // `visited` guard, the source line would successfully re-read the
        // fragment and this would fail with borderpx == 1.
        assert_eq!(cfg.borderpx, 5);
    }
}
