//! Keybind cheatsheet source — parses margo's `config.conf` `bind =`
//! lines into grouped, display-ready shortcuts. Port of the noctalia
//! `keybind-cheatsheet` plugin adapted to margo's Hyprland-shaped
//! config: `bind = MODS,KEY,ACTION,ARGS  # optional description`.
//!
//! `source = <path>` includes are followed (tilde + relative
//! resolution, loop-guarded). Descriptions come from a trailing
//! `#"quoted"` or plain `# comment`; otherwise we humanise the
//! action/command. Binds are grouped into a few action-type
//! categories since margo configs don't carry category headers.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// One parsed shortcut.
#[derive(Debug, Clone)]
pub(crate) struct Keybind {
    /// Canonical modifier names in order: `Super` / `Ctrl` / `Shift`
    /// / `Alt`.
    pub mods: Vec<&'static str>,
    /// The (formatted) trigger key.
    pub key: String,
    /// Human description.
    pub desc: String,
}

/// A titled group of binds.
#[derive(Debug, Clone)]
pub(crate) struct Section {
    pub title: &'static str,
    pub binds: Vec<Keybind>,
}

/// Category order in the cheatsheet.
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

/// `~/.config/margo/config.conf` (or `$XDG_CONFIG_HOME/margo/...`).
fn config_path() -> PathBuf {
    let base = if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        PathBuf::from(xdg)
    } else if let Some(home) = std::env::var_os("HOME") {
        PathBuf::from(home).join(".config")
    } else {
        PathBuf::from(".config")
    };
    base.join("margo").join("config.conf")
}

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

/// Read a config file and every `source =` it pulls in (depth-first,
/// visited-guarded), returning all lines in order.
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

/// Parse all binds, grouped into ordered sections (empty sections
/// dropped).
pub(crate) fn load_sections() -> Vec<Section> {
    load_sections_from(&config_path())
}

/// [`load_sections`] for the config at `path` (testable seam).
fn load_sections_from(path: &Path) -> Vec<Section> {
    let mut lines = Vec::new();
    read_all_lines(path, &mut HashSet::new(), &mut lines);

    let mut by_cat: std::collections::HashMap<&'static str, Vec<Keybind>> =
        std::collections::HashMap::new();
    for line in &lines {
        if let Some((cat, kb)) = parse_bind(line) {
            by_cat.entry(cat).or_default().push(kb);
        }
    }

    CATEGORIES
        .iter()
        .filter_map(|&title| {
            by_cat
                .remove(title)
                .filter(|b| !b.is_empty())
                .map(|binds| Section { title, binds })
        })
        .collect()
}

/// Parse one `bind = …` line → (category, Keybind). `None` for
/// non-bind lines.
fn parse_bind(line: &str) -> Option<(&'static str, Keybind)> {
    let t = line.trim();
    // `bind` plus margo's flag suffixes (`bindr`, `bindl`, `bindsp`, …).
    let rest = t.strip_prefix("bind")?;
    let flags_end = rest
        .find(|c: char| !c.is_ascii_alphabetic())
        .unwrap_or(rest.len());
    let (flags, rest) = rest.split_at(flags_end);
    // Only the flags margo's parser accepts (`is_bind_key`: s/l/r/p/c).
    // Hyprland's `binde`/`bindm` are not margo binds — margo skips them, so
    // the cheatsheet must not list them.
    if !flags
        .chars()
        .all(|c| matches!(c, 's' | 'l' | 'r' | 'p' | 'c'))
    {
        return None;
    }
    let body = rest.trim_start().strip_prefix('=')?.trim();

    // Split off a trailing description comment.
    let (body, desc_comment) = split_comment(body);

    let mut parts = body.splitn(3, ',');
    let mod_part = parts.next()?.trim();
    let key_raw = parts.next()?.trim();
    let action_rest = parts.next().unwrap_or("").trim();
    if key_raw.is_empty() && mod_part.is_empty() {
        return None;
    }

    let mods = parse_mods(mod_part);
    let key = format_key(key_raw);

    // action_rest = "ACTION,ARG1,ARG2,…"
    let mut ar = action_rest.splitn(2, ',');
    let action = ar.next().unwrap_or("").trim();
    let args = ar.next().unwrap_or("").trim();

    let desc = desc_comment.unwrap_or_else(|| humanise(action, args));
    let cat = categorise(action, args);

    Some((cat, Keybind { mods, key, desc }))
}

/// Pull a trailing `#"quoted"` or ` # plain` comment off a bind body.
fn split_comment(body: &str) -> (&str, Option<String>) {
    if let Some(start) = body.rfind("#\"")
        && let Some(end) = body[start + 2..].find('"')
    {
        let desc = body[start + 2..start + 2 + end].trim().to_string();
        return (body[..start].trim_end(), Some(desc));
    }
    // Plain ` # comment` (hash preceded by whitespace).
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

fn parse_mods(mod_part: &str) -> Vec<&'static str> {
    let mut mods = Vec::new();
    for m in mod_part.split('+') {
        match m.trim().to_ascii_lowercase().as_str() {
            "super" | "mod" | "mod4" | "logo" | "win" => push_unique(&mut mods, "Super"),
            "ctrl" | "control" => push_unique(&mut mods, "Ctrl"),
            "shift" => push_unique(&mut mods, "Shift"),
            "alt" | "mod1" => push_unique(&mut mods, "Alt"),
            _ => {}
        }
    }
    mods
}

fn push_unique(v: &mut Vec<&'static str>, s: &'static str) {
    if !v.contains(&s) {
        v.push(s);
    }
}

/// XF86 + a few special keys → readable; single letters upper-cased.
fn format_key(key: &str) -> String {
    let k = key.trim();
    let pretty = match k {
        "XF86AudioRaiseVolume" => "Vol Up",
        "XF86AudioLowerVolume" => "Vol Down",
        "XF86AudioMute" => "Mute",
        "XF86AudioMicMute" => "Mic Mute",
        "XF86AudioPlay" | "XF86AudioPlayPause" => "Play/Pause",
        "XF86AudioNext" => "Next",
        "XF86AudioPrev" => "Prev",
        "XF86AudioStop" => "Stop",
        "XF86MonBrightnessUp" => "Bright Up",
        "XF86MonBrightnessDown" => "Bright Down",
        "Return" => "Enter",
        "Escape" => "Esc",
        "space" => "Space",
        "Print" => "PrtSc",
        "" => "",
        other => {
            // Single letter → upper-case; otherwise pass through.
            if other.chars().count() == 1 {
                return other.to_ascii_uppercase();
            }
            other
        }
    };
    pretty.to_string()
}

/// Humanise an action+args when there's no comment.
fn humanise(action: &str, args: &str) -> String {
    match action {
        "spawn" | "exec" => launched_command(args)
            .split_whitespace()
            .take(4)
            .collect::<Vec<_>>()
            .join(" "),
        "" => "—".to_string(),
        other => {
            // dispatch action name → "Title Case".
            let words: Vec<String> = other
                .split(['_', '-'])
                .filter(|w| !w.is_empty())
                .map(|w| {
                    let mut c = w.chars();
                    match c.next() {
                        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                        None => String::new(),
                    }
                })
                .collect();
            words.join(" ")
        }
    }
}

/// The command a `spawn` bind launches, without a `uwsm app` launcher
/// wrapper (`uwsm app -a -- foot` → `foot`). Only the wrapper's own `--`
/// separator is stripped: a command's `--flags` are part of what it runs.
fn launched_command(args: &str) -> &str {
    let args = args.trim();
    if !args.starts_with("uwsm ") {
        return args;
    }
    if let Some((_, cmd)) = args.split_once(" -- ") {
        return cmd.trim();
    }
    let cmd = args
        .strip_prefix("uwsm app -a ")
        .or_else(|| args.strip_prefix("uwsm app "))
        .unwrap_or(args)
        .trim();
    if cmd.is_empty() { args } else { cmd }
}

/// Bucket a bind into a cheatsheet category from its action/command.
fn categorise(action: &str, args: &str) -> &'static str {
    let a = action.to_ascii_lowercase();
    let cmd = args.to_ascii_lowercase();
    if a.contains("scratchpad") {
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
                || cmd.contains("hyprlock")
            {
                "System"
            } else {
                "Launch"
            }
        }
        "view" | "tag" | "toggleview" | "toggletag" | "viewall" | "focusstack" | "focusmon"
        | "tagmon" => "Workspaces",
        "setlayout" | "cyclelayout" | "incnmaster" | "setmfact" | "incgaps" | "togglegaps"
        | "decgaps" => "Layout",
        "killclient" | "togglefloating" | "togglefullscreen" | "fullscreen" | "movewindow"
        | "resizewindow" | "zoom" | "swap" | "movestack" | "togglealwaysontop" | "summon"
        | "unscratchpad" => "Windows",
        _ => "General",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A config tree exercising `source =` the way margo resolves it: a bare
    /// relative path, a nested include, a self-include loop, a commented-out
    /// source, plus bind flag variants and descriptions.
    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("binds.d")).unwrap();
        std::fs::write(
            root.join("config.conf"),
            "# main\n\
             bind = SUPER,Return,spawn,kitty #\"Terminal\"\n\
             binde = SUPER,equal,incgaps,1\n\
             # source = binds.d/ignored.conf\n\
             source = binds.d/apps.conf\n\
             bind = SUPER+SHIFT,q,killclient\n",
        )
        .unwrap();
        std::fs::write(
            root.join("binds.d/apps.conf"),
            "bind = SUPER,b,spawn,firefox\nsource = nested.conf\nsource = apps.conf\n",
        )
        .unwrap();
        std::fs::write(
            root.join("binds.d/nested.conf"),
            "bindr = ALT,space,spawn,mshellctl menu app-launcher # launcher\n",
        )
        .unwrap();
        std::fs::write(root.join("binds.d/ignored.conf"), "bind = SUPER,x,quit\n").unwrap();
        dir
    }

    fn all_binds(sections: &[Section]) -> Vec<&Keybind> {
        sections.iter().flat_map(|s| s.binds.iter()).collect()
    }

    #[test]
    fn follows_sources_and_reads_descriptions() {
        let dir = fixture();
        let sections = load_sections_from(&dir.path().join("config.conf"));
        let binds = all_binds(&sections);
        // 6 bind lines in the tree, one is a commented-out source's and one
        // is a Hyprland-only `binde` margo doesn't load.
        assert_eq!(binds.len(), 4, "{binds:#?}");
        assert!(binds.iter().any(|b| b.desc == "Terminal"));
        assert!(binds.iter().any(|b| b.desc == "launcher"));
    }

    /// Golden check against margo's own parser: the cheatsheet must see
    /// exactly the key binds margo loads from the same include tree (minus
    /// the chvt binds margo injects into every config).
    #[test]
    fn sees_the_same_binds_as_margo() {
        let dir = fixture();
        let injected = {
            let empty = dir.path().join("empty.conf");
            std::fs::write(&empty, "").unwrap();
            margo_config::parse_config(Some(&empty))
                .unwrap()
                .key_bindings
                .len()
        };
        let margo = margo_config::parse_config(Some(&dir.path().join("config.conf")))
            .unwrap()
            .key_bindings
            .len()
            - injected;
        let ours = all_binds(&load_sections_from(&dir.path().join("config.conf"))).len();
        assert_eq!(ours, margo);
    }
    fn bind(line: &str) -> Keybind {
        parse_bind(line)
            .unwrap_or_else(|| panic!("not a bind: {line}"))
            .1
    }

    #[test]
    fn bind_flag_suffixes_follow_margo() {
        for ok in [
            "bind", "binds", "bindl", "bindr", "bindp", "bindc", "bindsl", "bindlr",
        ] {
            assert!(
                parse_bind(&format!("{ok} = SUPER,q,killclient")).is_some(),
                "{ok}"
            );
        }
        for bad in ["binde", "bindm", "bindsym", "binding", "bindx"] {
            assert!(
                parse_bind(&format!("{bad} = SUPER,q,killclient")).is_none(),
                "{bad}"
            );
        }
        // Spacing around `=` doesn't matter; the key must still be first.
        assert!(parse_bind("bind=SUPER,q,killclient").is_some());
        assert!(parse_bind("   bind   =   SUPER , q , killclient").is_some());
        assert!(parse_bind("mousebind = SUPER,btn_left,moveresize").is_none());
        assert!(parse_bind("bind SUPER,q,killclient").is_none());
    }

    #[test]
    fn commented_out_binds_are_skipped() {
        assert!(parse_bind("# bind = SUPER,q,killclient").is_none());
        assert!(parse_bind("   #bind = SUPER,q,killclient").is_none());
    }

    #[test]
    fn modifier_names_are_case_insensitive_and_deduplicated() {
        assert_eq!(
            bind("bind = super+SHIFT,q,killclient").mods,
            ["Super", "Shift"]
        );
        assert_eq!(bind("bind = Ctrl+ctrl,q,killclient").mods, ["Ctrl"]);
        assert_eq!(
            bind("bind = NONE,Print,spawn,grim").mods,
            Vec::<&str>::new()
        );
        assert_eq!(bind("bind = ,Print,spawn,grim").mods, Vec::<&str>::new());
    }

    #[test]
    fn keys_are_prettified() {
        assert_eq!(bind("bind = SUPER,q,killclient").key, "Q");
        assert_eq!(bind("bind = SUPER,Return,spawn,kitty").key, "Enter");
        assert_eq!(bind("bind = SUPER,space,spawn,x").key, "Space");
        assert_eq!(
            bind("bind = NONE,XF86AudioRaiseVolume,spawn,wpctl").key,
            "Vol Up"
        );
        assert_eq!(
            bind("bind = NONE,XF86AudioPlayPause,spawn,playerctl").key,
            "Play/Pause"
        );
        assert_eq!(bind("bind = SUPER,F1,spawn,x").key, "F1");
        assert_eq!(bind("bind = SUPER,code:24,killclient").key, "code:24");
        // Non-ASCII single characters pass through unchanged.
        assert_eq!(bind("bind = SUPER,ö,spawn,x").key, "ö");
    }

    #[test]
    fn descriptions_prefer_comments_then_humanise() {
        assert_eq!(
            bind(r#"bind = SUPER,q,killclient #"Close window""#).desc,
            "Close window"
        );
        assert_eq!(
            bind("bind = SUPER,q,killclient   # close it").desc,
            "close it"
        );
        // Only a launcher wrapper's `--` is stripped; a command's own
        // `--flags` stay.
        assert_eq!(
            bind("bind = SUPER,c,spawn,picker --color=#ff0000").desc,
            "picker --color=#ff0000"
        );
        assert_eq!(
            bind("bind = SUPER,k,spawn,kitty --class scratch").desc,
            "kitty --class scratch"
        );
        assert_eq!(
            bind("bind = SUPER,b,spawn,uwsm app -a -- firefox --new-window https://x.y z w").desc,
            "firefox --new-window https://x.y z"
        );
        assert_eq!(
            bind("bind = SUPER,b,spawn,uwsm app -a firefox").desc,
            "firefox"
        );
        assert_eq!(bind("bind = SUPER,b,spawn,uwsm app -- foot").desc, "foot");
        // An empty plain comment falls back to the humanised action.
        assert_eq!(bind("bind = SUPER,q,killclient #").desc, "Killclient");
        assert_eq!(
            bind("bind = SUPER,f,toggle_floating-window").desc,
            "Toggle Floating Window"
        );
        assert_eq!(bind("bind = SUPER,b,spawn,").desc, "");
    }

    #[test]
    fn categories_follow_the_action() {
        let cat = |line: &str| parse_bind(line).unwrap().0;
        assert_eq!(cat("bind = SUPER,1,view,1"), "Workspaces");
        assert_eq!(cat("bind = SUPER,q,killclient"), "Windows");
        assert_eq!(cat("bind = SUPER,t,setlayout,tile"), "Layout");
        assert_eq!(
            cat("bind = SUPER,s,toggle_named_scratchpad,term"),
            "Scratchpad"
        );
        assert_eq!(
            cat("bind = SUPER,d,spawn,mshellctl menu app-launcher"),
            "Shell"
        );
        assert_eq!(
            cat("bind = NONE,XF86AudioMute,spawn,wpctl set-mute @DEFAULT_SINK@ toggle"),
            "Media"
        );
        assert_eq!(cat("bind = SUPER,l,spawn,mlock"), "System");
        assert_eq!(cat("bind = SUPER,Return,spawn,kitty"), "Launch");
        assert_eq!(cat("bind = SUPER,r,reload_config"), "General");
        // Case-insensitive on the action.
        assert_eq!(cat("bind = SUPER,1,VIEW,1"), "Workspaces");
    }

    #[test]
    fn sections_come_out_in_category_order_without_empties() {
        let dir = tempfile::tempdir().unwrap();
        let conf = dir.path().join("config.conf");
        std::fs::write(
            &conf,
            "bind = SUPER,r,reload_config\n\
             bind = SUPER,Return,spawn,kitty\n\
             bind = SUPER,1,view,1\n\
             bind = SUPER,2,view,2\n",
        )
        .unwrap();
        let sections = load_sections_from(&conf);
        let titles: Vec<_> = sections.iter().map(|s| s.title).collect();
        assert_eq!(titles, ["Launch", "Workspaces", "General"]);
        // Binds keep config order within a section.
        let ws: Vec<_> = sections[1].binds.iter().map(|b| b.key.as_str()).collect();
        assert_eq!(ws, ["1", "2"]);
    }

    #[test]
    fn missing_config_or_source_is_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_sections_from(&dir.path().join("nope.conf")).is_empty());
        let conf = dir.path().join("config.conf");
        std::fs::write(
            &conf,
            "source = gone.conf\nsource = ~/definitely/not/here.conf\nbind = SUPER,q,killclient\n",
        )
        .unwrap();
        assert_eq!(all_binds(&load_sections_from(&conf)).len(), 1);
    }

    #[test]
    fn mutual_source_cycle_terminates_and_loads_each_file_once() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.conf");
        std::fs::write(&a, "bind = SUPER,a,killclient\nsource = b.conf\n").unwrap();
        std::fs::write(
            dir.path().join("b.conf"),
            "bind = SUPER,b,killclient\nsource = ./a.conf\nsource = \"a.conf\"\n",
        )
        .unwrap();
        let keys: Vec<String> = all_binds(&load_sections_from(&a))
            .iter()
            .map(|b| b.key.clone())
            .collect();
        assert_eq!(keys, ["A", "B"]);
    }

    #[test]
    fn source_lines_are_followed_in_place() {
        // An include's binds land where the `source` line is, not at the end.
        let dir = tempfile::tempdir().unwrap();
        let conf = dir.path().join("config.conf");
        std::fs::write(
            &conf,
            "bind = SUPER,a,killclient\nsource=inc.conf\nbind = SUPER,c,killclient\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("inc.conf"), "bind = SUPER,b,killclient\n").unwrap();
        let keys: Vec<String> = all_binds(&load_sections_from(&conf))
            .iter()
            .map(|b| b.key.clone())
            .collect();
        assert_eq!(keys, ["A", "B", "C"]);
    }
}
