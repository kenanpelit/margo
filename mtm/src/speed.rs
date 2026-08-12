//! `mtm speed ...` — category-aware fzf command launcher with pin +
//! recency scoring. Rust port of `tm.sh`'s SPEED MODE section.
//!
//! Where `tm.sh` injects bash helper functions into a temp file so fzf's
//! `--bind execute()/reload()` hooks can call back into them, `mtm`
//! exposes the same two operations as hidden subcommands on itself
//! (`speed __list`, `speed __pin`) — fzf shells out to `mtm speed __list`
//! / `mtm speed __pin {1}` the same way it shelled out to the sourced
//! bash functions, just against our own binary instead of a temp file.

use crate::config::{Config, fzf_dir};
use crate::fzf;
use anyhow::{Result, bail};
use clap::Subcommand;
use std::collections::{BTreeSet, HashMap};
use std::fs::OpenOptions;
use std::io::Write;
use std::process::Command;

const HISTORY_LIMIT: usize = 100;

#[derive(Subcommand, Debug)]
pub enum SpeedCommands {
    /// Open the picker (the default)
    #[command(aliases = ["s"])]
    Show,
    /// List every speed command, grouped by category
    #[command(aliases = ["l", "ls"])]
    List,
    /// Write out a starter set of example commands
    #[command(aliases = ["i"])]
    Init,
    /// Add a new speed command
    #[command(aliases = ["a"])]
    Add { name: String, command: String },
    /// Remove a speed command
    #[command(aliases = ["rm", "r"])]
    Remove { name: String },
    /// Edit a speed command in `$EDITOR`
    #[command(aliases = ["e"])]
    Edit { name: String },
    /// Open the speed-command directory in a shell
    #[command(aliases = ["d", "o", "open"])]
    Dir,
    /// Print the picker's candidate list (used by fzf's `reload`)
    #[command(name = "__list", hide = true)]
    InternalList,
    /// Toggle a base's pinned state (used by fzf's `ctrl-p` bind)
    #[command(name = "__pin", hide = true)]
    InternalPin { base: String },
}

pub fn run(cmd: Option<SpeedCommands>, cfg: &Config) -> Result<()> {
    match cmd.unwrap_or(SpeedCommands::Show) {
        SpeedCommands::Show => show(cfg),
        SpeedCommands::List => list(),
        SpeedCommands::Init => init(),
        SpeedCommands::Add { name, command } => add(&name, &command),
        SpeedCommands::Remove { name } => remove(&name),
        SpeedCommands::Edit { name } => edit(&name),
        SpeedCommands::Dir => open_dir(),
        SpeedCommands::InternalList => {
            print!("{}", build_list());
            Ok(())
        }
        SpeedCommands::InternalPin { base } => {
            toggle_pin(&base);
            Ok(())
        }
    }
}

fn cache_file() -> std::path::PathBuf {
    fzf_dir().join(".fzf_cache")
}
fn pins_file() -> std::path::PathBuf {
    fzf_dir().join(".fzf_pins")
}

/// Every `_*` file directly under `fzf_dir()`.
fn scripts() -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(fzf_dir()) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with('_'))
        .collect();
    names.sort();
    names
}

/// `<icon> <5-char category>`, matching `tm.sh speed_category`'s glob
/// patterns (on the comma-stripped, leading-`_`-stripped base).
fn category(base: &str) -> &'static str {
    if base.starts_with("ssh_") && base.contains("vpn") {
        "󰒃 VPN  "
    } else if base.starts_with("ssh_") && base.contains("podman") {
        " POD  "
    } else if base.starts_with("ssh_") {
        "󰣀 SSH  "
    } else if base.ends_with("-history") {
        "󰋖 HIST "
    } else if base.starts_with("translate_") {
        "󰊿 I18N "
    } else if matches!(base, "emoji" | "compose" | "zinger" | "snippets") {
        "󰅍 CLIP "
    } else if matches!(base, "ipwebtv" | "ytfzf") {
        "󰕧 MEDIA"
    } else if matches!(base, "playerctl" | "pulseaudio" | "volume_mute") {
        "󰕾 AUDIO"
    } else if matches!(base, "anote" | "notes") {
        "󰠮 NOTE "
    } else if base == "yazi_locate" {
        "󰉋 FILE "
    } else if base == "wpaperctl" {
        "󰸉 WALL "
    } else if base.ends_with("window-switcher") {
        "󰓩 TMUX "
    } else if base == "fman" {
        "󰈙 DOCS "
    } else if base == "applauncher" {
        "󰀻 APP  "
    } else if base == "fkill" {
        "󰜺 KILL "
    } else if base == "trash" {
        "󰩹 TRASH"
    } else if base == "calculator" {
        "󰪚 MATH "
    } else {
        " MISC "
    }
}

/// `(name, description)` from a filename `<base>[,<description>]` —
/// `tm.sh speed_display_name`: strip the leading `_`, turn `_`/`.` into
/// spaces in the name, turn `.` into spaces (and drop a leading `-- `)
/// in the description.
fn display_name(filename: &str) -> (String, String) {
    let base = filename.split(',').next().unwrap_or(filename);
    let desc = filename.split_once(',').map(|(_, d)| d).unwrap_or("");
    let name = base
        .strip_prefix('_')
        .unwrap_or(base)
        .replace(['_', '.'], " ");
    let desc = desc.replace('.', " ");
    let desc = desc.trim_start().trim_start_matches("-- ").to_string();
    (name, desc)
}

/// Resolve a `base` (as stored in the pins/cache files, `_ssh.server1`
/// style, leading underscore included) back to the real filename on
/// disk — it may carry a `,<description>` suffix the base doesn't.
fn resolve_base(base: &str) -> Option<String> {
    scripts()
        .into_iter()
        .find(|f| f == base || f.split(',').next() == Some(base))
}

fn format_row(filename: &str) -> String {
    let base = filename.split(',').next().unwrap_or(filename);
    let cat = category(base.strip_prefix('_').unwrap_or(base));
    let (name, desc) = display_name(filename);
    if desc.is_empty() {
        format!("{cat}  {name}")
    } else {
        format!("{cat}  {name:<24} · {desc}")
    }
}

fn read_lines(path: &std::path::Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

fn toggle_pin(base: &str) {
    let base = base.split(',').next().unwrap_or(base).to_string();
    let mut pins: Vec<String> = read_lines(&pins_file());
    if let Some(pos) = pins.iter().position(|p| p == &base) {
        pins.remove(pos);
    } else {
        pins.push(base);
    }
    let _ = std::fs::create_dir_all(fzf_dir());
    let _ = std::fs::write(pins_file(), pins.join("\n") + "\n");
}

/// Pinned first (alpha), then by frequency+recency score (desc), then
/// alpha — `tm.sh build_speed_list`. Score = usage count + a 0..10
/// recency bonus weighted toward the end of the cache file (most recent
/// runs).
fn build_list() -> String {
    let pinned: BTreeSet<String> = read_lines(&pins_file()).into_iter().collect();
    let recent = read_lines(&cache_file());
    let n = recent.len();

    let mut score: HashMap<String, i64> = HashMap::new();
    for (i, base) in recent.iter().enumerate() {
        let bonus = ((i as i64 + 1) * 10) / (n as i64 + 1);
        *score.entry(base.clone()).or_insert(0) += 1 + bonus;
    }

    let mut out = String::new();

    let mut pinned_sorted: Vec<&String> = pinned.iter().collect();
    pinned_sorted.sort();
    for base in pinned_sorted {
        if let Some(filename) = resolve_base(base) {
            out.push_str(&format!("{filename}\t⭐ {}\n", format_row(&filename)));
        }
    }

    let mut rest: Vec<String> = scripts()
        .into_iter()
        .filter(|f| {
            let base = f.split(',').next().unwrap_or(f);
            !pinned.contains(base)
        })
        .collect();
    rest.sort_by(|a, b| {
        let base_a = a.split(',').next().unwrap_or(a);
        let base_b = b.split(',').next().unwrap_or(b);
        let sa = *score.get(base_a).unwrap_or(&0);
        let sb = *score.get(base_b).unwrap_or(&0);
        sb.cmp(&sa).then_with(|| a.cmp(b))
    });
    for filename in rest {
        out.push_str(&format!("{filename}\t  {}\n", format_row(&filename)));
    }

    out
}

fn show(cfg: &Config) -> Result<()> {
    if !which("fzf") {
        bail!("fzf is not installed");
    }
    std::fs::create_dir_all(fzf_dir())?;
    let _ = OpenOptions::new()
        .create(true)
        .append(true)
        .open(cache_file());
    let _ = OpenOptions::new()
        .create(true)
        .append(true)
        .open(pins_file());

    let all = scripts();
    let total = all.len();
    if total == 0 {
        println!("No speed commands found");
        println!("Run `mtm speed init` to create example commands");
        return Ok(());
    }
    let ssh_count = all.iter().filter(|f| f.starts_with("_ssh")).count();
    let vpn_count = all
        .iter()
        .filter(|f| f.starts_with("_ssh") && f.contains("vpn"))
        .count();

    let self_exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "mtm".to_string());
    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "nvim".to_string());
    let fzf_dir_str = fzf_dir().display().to_string();

    let header = format!(
        "Total {total} · SSH {ssh_count} · VPN {vpn_count} │ ↵ run · ⌃p pin · ⌃e edit · ⌃o files · esc"
    );
    let ctrl_p = format!(
        "ctrl-p:execute-silent({self_exe} speed __pin {{1}})+reload({self_exe} speed __list)"
    );
    let ctrl_e =
        format!("ctrl-e:execute({editor} '{fzf_dir_str}'/{{1}})+reload({self_exe} speed __list)");
    let ctrl_o = format!("ctrl-o:execute(yazi '{fzf_dir_str}')");

    let selection = fzf::pick(
        &cfg.fzf_theme,
        "Speed",
        &header,
        &[
            "--delimiter=\t",
            "--with-nth=2..",
            "--no-sort",
            "--bind",
            &ctrl_p,
            "--bind",
            &ctrl_e,
            "--bind",
            &ctrl_o,
        ],
        &build_list(),
    )?;

    let Some(selection) = selection else {
        println!("Cancelled");
        return Ok(());
    };
    let selected = selection
        .split('\t')
        .next()
        .unwrap_or(&selection)
        .to_string();
    if selected.is_empty() {
        println!("Cancelled");
        return Ok(());
    }

    record_usage(&selected);

    let script_path = fzf_dir().join(&selected);
    if !script_path.is_file() {
        bail!("script not found: {selected}");
    }
    make_executable(&script_path);
    println!("Running: {selected}");
    let status = Command::new(&script_path).status()?;
    if status.success() {
        println!("Done");
        Ok(())
    } else {
        bail!("command failed: {selected}")
    }
}

fn record_usage(filename: &str) {
    let base = filename.split(',').next().unwrap_or(filename);
    let mut lines = read_lines(&cache_file());
    lines.push(base.to_string());
    if lines.len() > HISTORY_LIMIT {
        let drop = lines.len() - HISTORY_LIMIT;
        lines.drain(0..drop);
    }
    let _ = std::fs::write(cache_file(), lines.join("\n") + "\n");
}

#[cfg(unix)]
fn make_executable(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = std::fs::metadata(path) {
        let mut perms = meta.permissions();
        if perms.mode() & 0o111 == 0 {
            perms.set_mode(perms.mode() | 0o755);
            let _ = std::fs::set_permissions(path, perms);
        }
    }
}

fn which(bin: &str) -> bool {
    Command::new("which")
        .arg(bin)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn list() -> Result<()> {
    let all = scripts();
    if all.is_empty() {
        println!("No speed commands found");
        println!("Run `mtm speed init` to create example commands");
        return Ok(());
    }
    println!("Speed commands (total: {})", all.len());
    for filename in all {
        let (name, desc) = display_name(&filename);
        if desc.is_empty() {
            println!("  • {name}");
        } else {
            println!("  • {name} - {desc}");
        }
    }
    Ok(())
}

fn add(name: &str, command: &str) -> Result<()> {
    std::fs::create_dir_all(fzf_dir())?;
    let path = fzf_dir().join(format!("_{name}"));
    if path.exists() {
        print!("Command '{name}' already exists. Overwrite? (y/N): ");
        std::io::stdout().flush().ok();
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer).ok();
        if !answer.trim().eq_ignore_ascii_case("y") {
            println!("Cancelled");
            return Ok(());
        }
    }
    std::fs::write(&path, format!("#!/usr/bin/env bash\n# {name}\n{command}\n"))?;
    make_executable(&path);
    println!("Speed command added: {name}");
    Ok(())
}

fn remove(name: &str) -> Result<()> {
    let Some(filename) = resolve_base(&format!("_{name}")) else {
        bail!("command not found: {name}");
    };
    print!("Delete '{filename}'? (y/N): ");
    std::io::stdout().flush().ok();
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer).ok();
    if answer.trim().eq_ignore_ascii_case("y") {
        std::fs::remove_file(fzf_dir().join(&filename))?;
        println!("Removed: {name}");
    } else {
        println!("Cancelled");
    }
    Ok(())
}

fn edit(name: &str) -> Result<()> {
    let Some(filename) = resolve_base(&format!("_{name}")) else {
        bail!("command not found: {name}");
    };
    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vim".to_string());
    let status = Command::new(editor)
        .arg(fzf_dir().join(filename))
        .status()?;
    if status.success() {
        Ok(())
    } else {
        bail!("editor exited with an error")
    }
}

fn open_dir() -> Result<()> {
    std::fs::create_dir_all(fzf_dir())?;
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "bash".to_string());
    println!("Speed directory: {}", fzf_dir().display());
    let status = Command::new(shell).current_dir(fzf_dir()).status()?;
    if status.success() {
        Ok(())
    } else {
        bail!("shell exited with an error")
    }
}

fn init() -> Result<()> {
    std::fs::create_dir_all(fzf_dir())?;
    let samples: &[(&str, &str)] = &[
        (
            "_ssh.server1",
            "#!/usr/bin/env bash\n# SSH to server1\nssh user@server1.example.com\n",
        ),
        (
            "_ssh.server2",
            "#!/usr/bin/env bash\n# SSH to server2\nssh user@server2.example.com\n",
        ),
        (
            "_tmux.list",
            "#!/usr/bin/env bash\n# List all tmux sessions\ntmux list-sessions\n",
        ),
        (
            "_tmux.kill-all",
            "#!/usr/bin/env bash\n# Kill all tmux sessions\nread -p \"Kill every tmux session? (y/N): \" -n 1 -r\necho\nif [[ $REPLY =~ ^[Yy]$ ]]; then\n    tmux kill-server\n    echo \"All sessions killed\"\nfi\n",
        ),
        (
            "_tmux.attach",
            "#!/usr/bin/env bash\n# Attach to last tmux session\ntmux attach || tmux new-session\n",
        ),
        (
            "_git.status",
            "#!/usr/bin/env bash\n# Git status with color\ngit status\n",
        ),
        (
            "_git.pull",
            "#!/usr/bin/env bash\n# Git pull with rebase\ngit pull --rebase\n",
        ),
        (
            "_git.push",
            "#!/usr/bin/env bash\n# Git push current branch\ncurrent_branch=$(git branch --show-current)\ngit push origin \"$current_branch\"\n",
        ),
        (
            "_system.update",
            "#!/usr/bin/env bash\n# System update (Arch Linux)\nif command -v yay &>/dev/null; then\n    yay -Syu\nelif command -v pacman &>/dev/null; then\n    sudo pacman -Syu\nfi\n",
        ),
        (
            "_system.clean",
            "#!/usr/bin/env bash\n# Clean package cache\nif command -v yay &>/dev/null; then\n    yay -Sc\nelif command -v pacman &>/dev/null; then\n    sudo pacman -Sc\nfi\n",
        ),
        (
            "_docker.ps",
            "#!/usr/bin/env bash\n# List running containers\ndocker ps\n",
        ),
        (
            "_docker.clean",
            "#!/usr/bin/env bash\n# Clean docker system\ndocker system prune -af\n",
        ),
    ];
    for (name, body) in samples {
        let path = fzf_dir().join(name);
        std::fs::write(&path, body)?;
        make_executable(&path);
    }
    println!("Example commands created: {}", fzf_dir().display());
    println!("Total {} example commands", samples.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categorizes_ssh_vpn_before_generic_ssh() {
        assert_eq!(category("ssh_office-vpn"), "󰒃 VPN  ");
        assert_eq!(category("ssh_server1"), "󰣀 SSH  ");
    }

    #[test]
    fn categorizes_history_and_i18n() {
        assert_eq!(category("cmd-history"), "󰋖 HIST ");
        assert_eq!(category("translate_en"), "󰊿 I18N ");
    }

    #[test]
    fn falls_back_to_misc() {
        assert_eq!(category("something_unrecognised"), " MISC ");
    }

    #[test]
    fn display_name_splits_description_and_replaces_separators() {
        let (name, desc) = display_name("_ssh.server1");
        assert_eq!(name, "ssh server1");
        assert_eq!(desc, "");

        let (name, desc) = display_name("_deploy,--.Deploy.to.prod");
        assert_eq!(name, "deploy");
        assert_eq!(desc, "Deploy to prod");
    }

    #[test]
    fn resolve_base_matches_comma_suffixed_files() {
        // Pure logic test, no filesystem: verify the matching predicate
        // a resolve_base-style search would use.
        let files = ["_deploy,--.Deploy.to.prod".to_string()];
        let hit = files
            .iter()
            .find(|f| f.split(',').next() == Some("_deploy"));
        assert!(hit.is_some());
    }
}
