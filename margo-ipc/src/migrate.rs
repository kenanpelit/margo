//! W4.4 — config migration from hyprland/sway → margo.
//!
//! `mctl migrate` reads a Hyprland or Sway/i3 config file and
//! emits an equivalent margo `config.conf` to stdout (or
//! `--output PATH`). Niri's KDL config is intentionally
//! out-of-scope: niri's design (workspaces + scrolling columns,
//! no tags, no per-tag layouts) doesn't map cleanly onto
//! margo's tag-based model — a translator would invent a
//! semantics niri users wouldn't expect.
//!
//! Scope is deliberately narrow: **keybinds + spawn lines**.
//! The most valuable part of any compositor config to carry
//! over. Window rules / animations / monitor topology are
//! compositor-specific enough that auto-translation would
//! produce more noise than signal — the migrator emits a
//! comment block at the top pointing the user at the relevant
//! margo sections to re-author.
//!
//! ## Modifier translation
//!
//! Every supported source uses different conventions for "the
//! Super key":
//!
//! | Source    | Super       | Alt    | Ctrl   | Shift   |
//! |-----------|-------------|--------|--------|---------|
//! | Hyprland  | `SUPER`     | `ALT`  | `CTRL` | `SHIFT` |
//! | Sway/i3   | `Mod4`      | `Mod1` | `Ctrl` | `Shift` |
//! | margo     | `super`     | `alt`  | `ctrl` | `shift` |
//!
//! Unknown modifiers (e.g. `Mod3` for hyper, `Mod5` for AltGr)
//! pass through as warnings — the user has to map them by
//! hand. Same for compositor-specific actions (Hyprland's
//! `pseudo`, Sway's `mode`-switches, etc.).

use std::path::Path;

/// Which source compositor's config we're translating.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceFormat {
    Hyprland,
    Sway,
}

impl SourceFormat {
    pub fn parse_name(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "hyprland" | "hypr" | "h" => Some(SourceFormat::Hyprland),
            "sway" | "i3" | "s" => Some(SourceFormat::Sway),
            _ => None,
        }
    }

    /// Heuristic detection from file path / contents. Used when
    /// the user passes `mctl migrate <PATH>` without a `--from`
    /// flag.
    pub fn detect(path: &Path, contents: &str) -> Option<Self> {
        let path_str = path.to_string_lossy().to_ascii_lowercase();
        if path_str.contains("hypr") {
            return Some(SourceFormat::Hyprland);
        }
        if path_str.contains("sway") || path_str.contains("/i3/") {
            return Some(SourceFormat::Sway);
        }
        // Content sniff: hyprland uses `bind = MOD, KEY, ...`;
        // sway uses `bindsym MOD+KEY ...`.
        for line in contents.lines().take(200) {
            let l = line.trim_start();
            if l.starts_with("bind = ") || l.starts_with("bind=") {
                return Some(SourceFormat::Hyprland);
            }
            if l.starts_with("bindsym ") {
                return Some(SourceFormat::Sway);
            }
        }
        None
    }
}

/// Result of running the migrator. `output` is the emitted
/// margo config; `warnings` is one line per unconvertible
/// source line (action margo doesn't have an analogue for,
/// unrecognized modifier, etc.) with the source line number.
#[derive(Debug, Default)]
pub struct MigrationResult {
    pub output: String,
    pub warnings: Vec<String>,
}

/// Run the migration. `src` is the raw text of the source
/// config; the result's `output` is ready to be written to
/// `~/.config/margo/config.conf` (after the user reviews it).
pub fn migrate(format: SourceFormat, src: &str) -> MigrationResult {
    let mut result = MigrationResult::default();
    write_header(format, &mut result.output);

    for (lineno, raw) in src.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        match format {
            SourceFormat::Hyprland => translate_hyprland_line(line, lineno + 1, &mut result),
            SourceFormat::Sway => translate_sway_line(line, lineno + 1, &mut result),
        }
    }

    if result.warnings.is_empty() {
        result.output.push_str(
            "\n# Migration finished without warnings. Review the binds above\n\
             # and tune as needed.\n",
        );
    } else {
        result.output.push_str(
            "\n# Migration finished with warnings (printed to stderr).\n\
             # The binds above translated cleanly; lines mentioned in the\n\
             # warnings need manual attention.\n",
        );
    }
    result
}

fn write_header(format: SourceFormat, out: &mut String) {
    out.push_str(&format!(
        "# margo config — migrated from {} via `mctl migrate`.\n",
        match format {
            SourceFormat::Hyprland => "Hyprland",
            SourceFormat::Sway => "Sway / i3",
        }
    ));
    out.push_str(
        "#\n\
         # Only keybinds and spawn lines were auto-translated. Window rules,\n\
         # animations, and monitor topology are compositor-specific and need\n\
         # to be re-authored from `margo/src/config.example.conf`. Read that\n\
         # file end-to-end before tweaking — every section explains the\n\
         # margo-side semantics and trade-offs.\n\
         #\n\
         # `mctl check-config <this-file>` validates without applying.\n\
         # `mctl reload` (or Super+Ctrl+R) re-reads it without logout.\n\n",
    );
}

// ── Hyprland ────────────────────────────────────────────────────────────────
//
// Format: `bind = MODS, KEY, ACTION [, ARG1 [, ARG2]]`
// Mods are SUPER / ALT / CTRL / SHIFT joined with whitespace or
// nothing. Actions: `exec`, `killactive`, `togglefloating`,
// `fullscreen`, `movefocus l/r/u/d`, `workspace N`,
// `movetoworkspace N`, `pseudo`, `togglesplit`, etc.

fn translate_hyprland_line(line: &str, lineno: usize, result: &mut MigrationResult) {
    if let Some(rest) = line.strip_prefix("bind").and_then(|s| s.trim_start().strip_prefix('=')) {
        translate_hyprland_bind(rest.trim(), lineno, result);
        return;
    }
    if let Some(rest) = line.strip_prefix("exec-once") {
        // hyprland writes `exec-once = foo`; strip the
        // optional whitespace-and-equals prefix.
        let cmd = rest.trim().trim_start_matches('=').trim();
        if !cmd.is_empty() {
            result.output.push_str(&format!("exec-once = {cmd}\n"));
        }
        return;
    }
    // Hyprland's `bindm` / `bindl` / `binde` are flag-suffixed
    // bind variants. Treat them like plain `bind` for now;
    // their semantics (mouse / locked / repeat) don't all map
    // cleanly. Notes the variant in a warning.
    for suffix in ["bindm", "bindl", "binde", "bindle", "bindel", "bindr"] {
        if let Some(rest) = line.strip_prefix(suffix).and_then(|s| s.trim_start().strip_prefix('=')) {
            result.warnings.push(format!(
                "L{lineno}: hyprland `{suffix}` flag-bind treated as plain bind \
                 (mouse / locked / repeat semantics not fully translated)"
            ));
            translate_hyprland_bind(rest.trim(), lineno, result);
            return;
        }
    }
}

fn translate_hyprland_bind(spec: &str, lineno: usize, result: &mut MigrationResult) {
    let parts: Vec<&str> = spec.splitn(4, ',').map(str::trim).collect();
    if parts.len() < 3 {
        result.warnings.push(format!(
            "L{lineno}: bind missing fields — got `{spec}`"
        ));
        return;
    }
    let mods = parts[0];
    let key = parts[1];
    let action = parts[2];
    let arg = parts.get(3).copied().unwrap_or("");

    let margo_mods = translate_mods_hyprland(mods);
    let margo_key = translate_key(key);
    let Some((margo_action, margo_arg)) = map_action(action, arg, lineno, result) else {
        return;
    };

    if margo_arg.is_empty() {
        result.output.push_str(&format!(
            "bind = {margo_mods},{margo_key},{margo_action}\n"
        ));
    } else {
        result.output.push_str(&format!(
            "bind = {margo_mods},{margo_key},{margo_action},{margo_arg}\n"
        ));
    }
}

fn translate_mods_hyprland(mods: &str) -> String {
    let lower = mods.to_ascii_lowercase();
    let tokens: Vec<&str> = lower.split_whitespace().collect();
    let mut out = Vec::new();
    for t in tokens {
        match t {
            "super" | "mod4" | "win" | "meta" => out.push("super"),
            "alt" | "mod1" => out.push("alt"),
            "ctrl" | "control" => out.push("ctrl"),
            "shift" => out.push("shift"),
            "" => {}
            _ => out.push(t),
        }
    }
    if out.is_empty() {
        "NONE".to_string()
    } else {
        out.join("+")
    }
}

// ── Sway / i3 ───────────────────────────────────────────────────────────────
//
// Format: `bindsym MOD+KEY ACTION [ARGS]`
// Mods join with `+`. Actions: `exec`, `kill`,
// `floating toggle`, `fullscreen`, `focus left/right/up/down`,
// `workspace N` / `workspace number N`,
// `move container to workspace N`, `mode <name>`, etc.

fn translate_sway_line(line: &str, lineno: usize, result: &mut MigrationResult) {
    if let Some(rest) = line.strip_prefix("bindsym ") {
        translate_sway_bind(rest, lineno, result);
        return;
    }
    if let Some(rest) = line.strip_prefix("exec ") {
        let cmd = rest.trim();
        if !cmd.is_empty() {
            result.output.push_str(&format!("exec-once = {cmd}\n"));
        }
        return;
    }
    if let Some(rest) = line.strip_prefix("exec_always ") {
        result.warnings.push(format!(
            "L{lineno}: sway `exec_always` (re-runs on reload) → margo `exec-once`; \
             margo's reload doesn't re-run exec; consider a systemd user unit"
        ));
        let cmd = rest.trim();
        if !cmd.is_empty() {
            result.output.push_str(&format!("exec-once = {cmd}\n"));
        }
    }
}

fn translate_sway_bind(spec: &str, lineno: usize, result: &mut MigrationResult) {
    // Optional flags before the keysym (--release, --no-repeat,
    // --to-code, etc.). Skip them; warn on any that change
    // semantics.
    let mut spec = spec.trim();
    while let Some(rest) = spec.strip_prefix("--") {
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let flag = &rest[..end];
        result.warnings.push(format!(
            "L{lineno}: sway bindsym flag `--{flag}` ignored (no margo equivalent)"
        ));
        spec = rest[end..].trim_start();
    }

    let split = spec.splitn(2, char::is_whitespace).collect::<Vec<_>>();
    if split.len() < 2 {
        result.warnings.push(format!(
            "L{lineno}: bindsym missing action — got `{spec}`"
        ));
        return;
    }
    let key_combo = split[0];
    let action_part = split[1].trim();

    let mut combo_parts: Vec<&str> = key_combo.split('+').collect();
    let key = combo_parts.pop().unwrap_or("");
    let mods_joined = combo_parts.join(" ");
    let margo_mods = translate_mods_hyprland(&mods_joined); // same mod table
    let margo_key = translate_key(key);

    let (action, arg) = split_first_word(action_part);
    let Some((margo_action, margo_arg)) = map_action(action, arg, lineno, result) else {
        return;
    };

    if margo_arg.is_empty() {
        result.output.push_str(&format!(
            "bind = {margo_mods},{margo_key},{margo_action}\n"
        ));
    } else {
        result.output.push_str(&format!(
            "bind = {margo_mods},{margo_key},{margo_action},{margo_arg}\n"
        ));
    }
}

// ── Action mapping (canonical → margo) ──────────────────────────────────────
//
// Many compositors share the same ideas under different names.
// Build a single canonical map and route hyprland / sway
// vocabulary through it. Returns `None` when there's no margo
// equivalent (in which case a warning has been emitted).

fn map_action(
    raw_action: &str,
    raw_arg: &str,
    lineno: usize,
    result: &mut MigrationResult,
) -> Option<(String, String)> {
    let action = raw_action.trim();
    let arg = raw_arg.trim();

    // exec/spawn — pass through.
    if action == "exec" || action == "spawn" {
        return Some(("spawn".to_string(), arg.to_string()));
    }
    // killclient / close.
    if matches!(action, "killactive" | "close-window" | "kill") {
        return Some(("killclient".to_string(), String::new()));
    }
    // Floating + fullscreen toggles.
    if action == "togglefloating"
        || action == "toggle-window-floating"
        || (action == "floating" && arg == "toggle")
    {
        return Some(("togglefloating".to_string(), String::new()));
    }
    if action == "fullscreen" || action == "fullscreen-window" || action == "fullscreentoggle" {
        return Some(("togglefullscreen".to_string(), String::new()));
    }
    // Focus directions: hyprland `movefocus l|r|u|d`,
    // sway `focus left|right|up|down`.
    if action == "movefocus" || action == "focus" {
        if let Some(dir) = focus_direction(arg) {
            return Some(("focusdir".to_string(), dir.to_string()));
        }
    }
    // Workspace / view: hyprland `workspace N`,
    // sway `workspace N` / `workspace number N`.
    if action == "workspace" {
        let num = arg.trim_start_matches("number").trim();
        if let Some(mask) = workspace_to_tag_mask(num) {
            return Some(("view".to_string(), mask.to_string()));
        }
    }
    // Move container/window to workspace.
    if action == "movetoworkspace"
        || action == "move-window-to-workspace"
        || (action == "move" && arg.starts_with("container to workspace"))
        || (action == "move" && arg.starts_with("window to workspace"))
    {
        // Last whitespace-separated token of the arg is always
        // the workspace number across hyprland / sway / i3
        // variants. Avoids fighting the prefix-stripping
        // chain when the format has optional words like
        // "number" between the verb and the digit.
        if let Some(last) = arg.split_whitespace().last() {
            if let Some(mask) = workspace_to_tag_mask(last) {
                return Some(("tag".to_string(), mask.to_string()));
            }
        }
    }
    if action == "togglesplit" || action == "toggle-split" {
        return Some(("switch_layout".to_string(), String::new()));
    }
    if action == "exit" || action == "exit-niri" {
        return Some(("quit".to_string(), String::new()));
    }
    if action == "reload" {
        return Some(("reload_config".to_string(), String::new()));
    }

    // Compositor-specific actions we have no analogue for.
    result.warnings.push(format!(
        "L{lineno}: action `{action} {arg}` has no margo equivalent — keep, drop, or remap manually"
    ));
    None
}

fn focus_direction(arg: &str) -> Option<&'static str> {
    match arg.trim().chars().next()? {
        'l' | 'L' => Some("left"),
        'r' | 'R' => Some("right"),
        'u' | 'U' => Some("up"),
        'd' | 'D' => Some("down"),
        _ => None,
    }
}

fn workspace_to_tag_mask(s: &str) -> Option<u32> {
    let s = s.trim();
    let n: u32 = s.parse().ok()?;
    if !(1..=32).contains(&n) {
        return None;
    }
    Some(1u32 << (n - 1))
}

// ── Key name normalization ──────────────────────────────────────────────────

fn translate_key(key: &str) -> String {
    // Both source compositors use xkbcommon names — pass
    // through. We special-case a couple of common rename
    // patterns: hyprland `code:`-prefixed scancodes (skip),
    // sway `Mod4` mistakenly placed in the key slot (rare).
    let key = key.trim();
    if let Some(code) = key.strip_prefix("code:") {
        return format!("Code{code}");
    }
    // Aliases that differ between xkb naming and what users
    // commonly type. Compare case-insensitively so RETURN /
    // Return / return all hit the same branch.
    match key.to_ascii_lowercase().as_str() {
        "return" | "ret" | "enter" | "kp_enter" => "Return".to_string(),
        "esc" | "escape" => "Escape".to_string(),
        "space" => "space".to_string(),
        "tab" => "Tab".to_string(),
        "print" => "Print".to_string(),
        _ => key.to_string(),
    }
}

fn split_first_word(s: &str) -> (&str, &str) {
    s.find(char::is_whitespace)
        .map(|i| (s[..i].trim(), s[i..].trim()))
        .unwrap_or((s, ""))
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn migrate_str(format: SourceFormat, src: &str) -> MigrationResult {
        migrate(format, src)
    }

    #[test]
    fn hyprland_basic_binds() {
        let src = "\
            bind = SUPER, Q, killactive\n\
            bind = SUPER, RETURN, exec, kitty\n\
            bind = SUPER+SHIFT, F, togglefloating\n\
            bind = SUPER, 1, workspace, 1\n\
        ";
        let r = migrate_str(SourceFormat::Hyprland, src);
        assert!(r.output.contains("bind = super,Q,killclient"), "{}", r.output);
        assert!(r.output.contains("bind = super,Return,spawn,kitty"), "{}", r.output);
        assert!(r.output.contains("bind = super+shift,F,togglefloating"), "{}", r.output);
        assert!(r.output.contains("bind = super,1,view,1"), "{}", r.output);
        assert!(r.warnings.is_empty(), "unexpected warnings: {:?}", r.warnings);
    }

    #[test]
    fn hyprland_movetoworkspace_becomes_tag() {
        let src = "bind = SUPER+SHIFT, 5, movetoworkspace, 5\n";
        let r = migrate_str(SourceFormat::Hyprland, src);
        // workspace 5 → 1 << 4 = 16.
        assert!(r.output.contains("bind = super+shift,5,tag,16"), "{}", r.output);
    }

    #[test]
    fn hyprland_focusdir() {
        let src = "\
            bind = SUPER, h, movefocus, l\n\
            bind = SUPER, l, movefocus, r\n\
        ";
        let r = migrate_str(SourceFormat::Hyprland, src);
        assert!(r.output.contains("bind = super,h,focusdir,left"), "{}", r.output);
        assert!(r.output.contains("bind = super,l,focusdir,right"), "{}", r.output);
    }

    #[test]
    fn hyprland_unconvertible_emits_warning() {
        let src = "bind = SUPER, P, pseudo,\n";
        let r = migrate_str(SourceFormat::Hyprland, src);
        assert!(r.warnings.iter().any(|w| w.contains("pseudo")));
        // Should NOT emit a half-broken bind for it.
        assert!(!r.output.contains(",pseudo"), "{}", r.output);
    }

    #[test]
    fn sway_basic_binds() {
        let src = "\
            bindsym Mod4+Return exec kitty\n\
            bindsym Mod4+q kill\n\
            bindsym Mod4+Shift+f floating toggle\n\
            bindsym Mod4+1 workspace 1\n\
        ";
        let r = migrate_str(SourceFormat::Sway, src);
        assert!(r.output.contains("bind = super,Return,spawn,kitty"), "{}", r.output);
        assert!(r.output.contains("bind = super,q,killclient"), "{}", r.output);
        assert!(r.output.contains("bind = super+shift,f,togglefloating"), "{}", r.output);
        assert!(r.output.contains("bind = super,1,view,1"), "{}", r.output);
    }

    #[test]
    fn sway_move_container_to_workspace() {
        let src = "bindsym Mod4+Shift+3 move container to workspace number 3\n";
        let r = migrate_str(SourceFormat::Sway, src);
        // workspace 3 → 1 << 2 = 4.
        assert!(r.output.contains("bind = super+shift,3,tag,4"), "{}", r.output);
    }

    #[test]
    fn sway_release_flag_warns_but_translates() {
        let src = "bindsym --release Mod4+q kill\n";
        let r = migrate_str(SourceFormat::Sway, src);
        assert!(r.warnings.iter().any(|w| w.contains("--release")));
        assert!(r.output.contains("bind = super,q,killclient"), "{}", r.output);
    }

    #[test]
    fn detect_format_by_path_and_content() {
        assert_eq!(
            SourceFormat::detect(Path::new("/home/u/.config/hypr/hyprland.conf"), ""),
            Some(SourceFormat::Hyprland)
        );
        assert_eq!(
            SourceFormat::detect(Path::new("/home/u/.config/sway/config"), ""),
            Some(SourceFormat::Sway)
        );
        assert_eq!(
            SourceFormat::detect(Path::new("/tmp/x.txt"), "bindsym Mod4+q kill\n"),
            Some(SourceFormat::Sway)
        );
        assert_eq!(
            SourceFormat::detect(Path::new("/tmp/x.txt"), "bind = SUPER, q, killactive\n"),
            Some(SourceFormat::Hyprland)
        );
    }

    #[test]
    fn workspace_mask_arithmetic() {
        assert_eq!(workspace_to_tag_mask("1"), Some(1));
        assert_eq!(workspace_to_tag_mask("9"), Some(256));
        assert_eq!(workspace_to_tag_mask("0"), None);
        assert_eq!(workspace_to_tag_mask("33"), None);
        assert_eq!(workspace_to_tag_mask("abc"), None);
    }
}
