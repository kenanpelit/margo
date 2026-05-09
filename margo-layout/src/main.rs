//! `margo-layout` — quick-switch helper for margo's monitor layout.
//!
//! Drop one `layout_<name>.conf` file per setup into the margo
//! config directory (each containing the `monitorrule` lines that
//! describe that arrangement), point your main `config.conf` at
//! `source = margo-layout.conf`, and use this binary to flip the
//! `margo-layout.conf` symlink between the available files. A
//! `mctl reload` fires automatically after the swap so the change
//! lands without a logout.
//!
//! See `parser.rs` for the meta-directive grammar (`#@ name`,
//! `#@ shortcut`, `#@ output_name`, `#@ color`).
//!
//! ## Why a symlink, not a writeable file?
//!
//! The user's source-of-truth lives in the `layout_*.conf` files,
//! which they edit by hand. Mutating the active layout file
//! directly would conflate "what setups are available" with
//! "which one is active right now" — the former wants version
//! control, the latter is per-machine state. The symlink
//! approach keeps the two cleanly separate: the layout files are
//! a static catalogue, the symlink is the runtime selection.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand};

mod parser;
mod preview;

use parser::Layout;

/// Default link basename inside the margo config directory. The
/// user's `config.conf` should `source = margo-layout.conf` so this
/// path gets pulled into the active config on every reload.
const ACTIVE_LINK: &str = "margo-layout.conf";

#[derive(Parser, Debug)]
#[command(
    name = "margo-layout",
    version,
    about = "Switch margo's monitor layout between named profiles",
    long_about = "Maintain a catalogue of named monitor arrangements as \
        layout_<name>.conf files in margo's config directory and flip \
        between them with one command. Drives `mctl reload` automatically."
)]
struct Cli {
    /// Margo config directory. Defaults to `$XDG_CONFIG_HOME/margo`
    /// or `~/.config/margo`.
    #[arg(short, long)]
    config_dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// List every available layout, with shortcuts and an inline
    /// colour-coded summary of its output rectangles.
    List {
        /// Render a multi-line ASCII preview under each layout.
        #[arg(long)]
        preview: bool,
        /// Emit machine-readable JSON instead of the human view.
        #[arg(long)]
        json: bool,
    },

    /// Print the slug of the currently-active layout (whatever
    /// `margo-layout.conf` symlinks to). Exit non-zero if no
    /// active layout is set.
    Current,

    /// Switch to layout `<name>` (matched against the file slug,
    /// the `#@ name` directive, or any `#@ shortcut`). Re-runs
    /// `mctl reload` after the swap so margo picks up the change
    /// without a logout.
    Set {
        name: String,
        /// Skip the `mctl reload` trigger. Useful when scripting a
        /// layout switch alongside other config edits that should
        /// land in the same reload pass.
        #[arg(long)]
        no_reload: bool,
    },

    /// Cycle to the next layout (alphabetical order, wraps).
    Next {
        #[arg(long)]
        no_reload: bool,
    },

    /// Cycle to the previous layout (alphabetical order, wraps).
    Prev {
        #[arg(long)]
        no_reload: bool,
    },

    /// Render an ASCII preview of `<name>` to stdout — useful for
    /// sanity-checking the geometry without activating the layout.
    Preview { name: String },

    /// Interactive picker. Renders the layout list with previews
    /// and reads a single line from stdin (slug, name, or
    /// shortcut). On match, activates the layout and reloads. If
    /// `wofi` / `fuzzel` / `rofi` is installed, hands off to that
    /// instead of the inline prompt.
    Pick {
        /// Force the inline TTY prompt; skip the auto-detected
        /// graphical picker.
        #[arg(long)]
        no_gui: bool,
        #[arg(long)]
        no_reload: bool,
    },
}

fn main() {
    if let Err(err) = run() {
        eprintln!("margo-layout: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let config_dir = resolve_config_dir(cli.config_dir.as_deref())?;
    let layouts = parser::gather_layouts(&config_dir)?;

    match cli.command {
        Cmd::List { preview, json } => cmd_list(&config_dir, &layouts, preview, json),
        Cmd::Current => cmd_current(&config_dir, &layouts),
        Cmd::Set { name, no_reload } => cmd_set(&config_dir, &layouts, &name, no_reload),
        Cmd::Next { no_reload } => cmd_cycle(&config_dir, &layouts, 1, no_reload),
        Cmd::Prev { no_reload } => cmd_cycle(&config_dir, &layouts, -1, no_reload),
        Cmd::Preview { name } => cmd_preview(&layouts, &name),
        Cmd::Pick { no_gui, no_reload } => cmd_pick(&config_dir, &layouts, no_gui, no_reload),
    }
}

fn resolve_config_dir(arg: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = arg {
        return Ok(path.to_path_buf());
    }
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        let p = PathBuf::from(xdg).join("margo");
        if p.exists() {
            return Ok(p);
        }
    }
    let home = std::env::var_os("HOME").ok_or_else(|| anyhow!("$HOME not set"))?;
    Ok(PathBuf::from(home).join(".config").join("margo"))
}

fn cmd_list(
    config_dir: &Path,
    layouts: &[Layout],
    show_preview: bool,
    as_json: bool,
) -> Result<()> {
    if as_json {
        let active = current_slug(config_dir);
        let entries: Vec<_> = layouts
            .iter()
            .map(|l| {
                serde_json::json!({
                    "slug": l.slug,
                    "name": l.name,
                    "shortcuts": l.shortcuts,
                    "active": Some(&l.slug) == active.as_ref(),
                    "outputs": l.outputs.iter().map(|o| serde_json::json!({
                        "connector": o.connector,
                        "label": o.label,
                        "color": o.color,
                        "x": o.x,
                        "y": o.y,
                        "width": o.width,
                        "height": o.height,
                    })).collect::<Vec<_>>(),
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "config_dir": config_dir.display().to_string(),
                "active": active,
                "layouts": entries,
            }))?
        );
        return Ok(());
    }

    if layouts.is_empty() {
        println!("No layouts found in {}.", config_dir.display());
        println!(
            "Create one or more layout_<name>.conf files there to use margo-layout."
        );
        return Ok(());
    }

    let active = current_slug(config_dir);
    for layout in layouts {
        let active_marker = if Some(&layout.slug) == active.as_ref() {
            "● "
        } else {
            "  "
        };
        let shortcuts = if layout.shortcuts.is_empty() {
            String::new()
        } else {
            format!(" [{}]", layout.shortcuts.join(", "))
        };
        println!(
            "{}{}{} — {}",
            active_marker,
            layout.name,
            shortcuts,
            preview::render_inline(layout)
        );
        if show_preview {
            for line in preview::render_ascii(layout, 60).lines() {
                println!("    {}", line);
            }
            println!();
        }
    }
    Ok(())
}

fn cmd_current(config_dir: &Path, layouts: &[Layout]) -> Result<()> {
    let Some(slug) = current_slug(config_dir) else {
        bail!("no active layout (run `margo-layout set <name>` to pick one)");
    };
    let layout = layouts
        .iter()
        .find(|l| l.slug == slug)
        .ok_or_else(|| anyhow!("active link points to unknown slug `{}`", slug))?;
    if layout.shortcuts.is_empty() {
        println!("{} ({})", layout.name, layout.slug);
    } else {
        println!(
            "{} ({}) — shortcut(s): {}",
            layout.name,
            layout.slug,
            layout.shortcuts.join(", ")
        );
    }
    Ok(())
}

fn cmd_set(
    config_dir: &Path,
    layouts: &[Layout],
    needle: &str,
    no_reload: bool,
) -> Result<()> {
    let layout = match_layout(layouts, needle)?;
    activate(config_dir, layout)?;
    println!("Activated layout `{}`.", layout.name);
    if !no_reload {
        trigger_reload();
    }
    Ok(())
}

fn cmd_cycle(
    config_dir: &Path,
    layouts: &[Layout],
    step: i32,
    no_reload: bool,
) -> Result<()> {
    if layouts.is_empty() {
        bail!("no layouts available to cycle");
    }
    let current = current_slug(config_dir);
    let idx = current
        .as_ref()
        .and_then(|slug| layouts.iter().position(|l| &l.slug == slug))
        .unwrap_or(0);
    let n = layouts.len() as i32;
    let next = ((idx as i32 + step).rem_euclid(n)) as usize;
    let layout = &layouts[next];
    activate(config_dir, layout)?;
    println!("Activated layout `{}`.", layout.name);
    if !no_reload {
        trigger_reload();
    }
    Ok(())
}

fn cmd_preview(layouts: &[Layout], needle: &str) -> Result<()> {
    let layout = match_layout(layouts, needle)?;
    println!("{}", layout.name);
    println!("{}", preview::render_ascii(layout, 80));
    Ok(())
}

fn cmd_pick(
    config_dir: &Path,
    layouts: &[Layout],
    no_gui: bool,
    no_reload: bool,
) -> Result<()> {
    if layouts.is_empty() {
        bail!("no layouts found in {}", config_dir.display());
    }

    if !no_gui {
        if let Some(picker) = detect_graphical_picker() {
            return run_graphical_pick(&picker, config_dir, layouts, no_reload);
        }
    }
    run_inline_pick(config_dir, layouts, no_reload)
}

fn run_inline_pick(
    config_dir: &Path,
    layouts: &[Layout],
    no_reload: bool,
) -> Result<()> {
    println!();
    let active = current_slug(config_dir);
    for (i, layout) in layouts.iter().enumerate() {
        let active_marker = if Some(&layout.slug) == active.as_ref() {
            "●"
        } else {
            " "
        };
        let shortcuts = if layout.shortcuts.is_empty() {
            String::new()
        } else {
            format!(" [{}]", layout.shortcuts.join(", "))
        };
        println!(
            " {} {}. {}{}  {}",
            active_marker,
            i + 1,
            layout.name,
            shortcuts,
            preview::render_inline(layout)
        );
    }
    println!();
    eprint!("Pick layout (number, name, or shortcut): ");
    let mut input = String::new();
    std::io::stdin()
        .read_line(&mut input)
        .context("reading stdin")?;
    let input = input.trim();
    if input.is_empty() {
        bail!("no selection given");
    }

    let layout = if let Ok(n) = input.parse::<usize>() {
        if n == 0 || n > layouts.len() {
            bail!("number out of range");
        }
        &layouts[n - 1]
    } else {
        match_layout(layouts, input)?
    };
    activate(config_dir, layout)?;
    println!("Activated layout `{}`.", layout.name);
    if !no_reload {
        trigger_reload();
    }
    Ok(())
}

fn run_graphical_pick(
    picker: &PickerCmd,
    config_dir: &Path,
    layouts: &[Layout],
    no_reload: bool,
) -> Result<()> {
    use std::io::Write;

    let mut menu = String::new();
    for layout in layouts {
        let shortcuts = if layout.shortcuts.is_empty() {
            String::new()
        } else {
            format!(" [{}]", layout.shortcuts.join(","))
        };
        menu.push_str(&format!("{}{}\n", layout.name, shortcuts));
    }

    let mut child = Command::new(&picker.binary)
        .args(&picker.args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .with_context(|| format!("spawn {}", picker.binary))?;
    {
        let stdin = child.stdin.as_mut().context("picker stdin")?;
        stdin.write_all(menu.as_bytes())?;
    }
    let output = child.wait_with_output()?;
    if !output.status.success() {
        // User cancelled — exit cleanly.
        return Ok(());
    }
    let chosen = String::from_utf8_lossy(&output.stdout);
    // The picker echoes back the entire line including the
    // ` [shortcut]` suffix; strip it off before matching.
    let line = chosen.lines().next().unwrap_or("").trim();
    let needle = line.split_once(" [").map(|(a, _)| a).unwrap_or(line);
    let needle = needle.trim();
    if needle.is_empty() {
        return Ok(());
    }

    let layout = match_layout(layouts, needle)?;
    activate(config_dir, layout)?;
    println!("Activated layout `{}`.", layout.name);
    if !no_reload {
        trigger_reload();
    }
    Ok(())
}

struct PickerCmd {
    binary: &'static str,
    args: Vec<String>,
}

/// Find a graphical menu picker on $PATH. Order: fuzzel (Wayland-
/// native, fast) → wofi (also Wayland) → rofi (X11/Wayland via
/// xdg-portal). First match wins.
fn detect_graphical_picker() -> Option<PickerCmd> {
    if which("fuzzel") {
        return Some(PickerCmd {
            binary: "fuzzel",
            args: vec![
                "--dmenu".into(),
                "--prompt".into(),
                "layout: ".into(),
            ],
        });
    }
    if which("wofi") {
        return Some(PickerCmd {
            binary: "wofi",
            args: vec![
                "--dmenu".into(),
                "--prompt".into(),
                "layout".into(),
                "--insensitive".into(),
            ],
        });
    }
    if which("rofi") {
        return Some(PickerCmd {
            binary: "rofi",
            args: vec![
                "-dmenu".into(),
                "-p".into(),
                "layout".into(),
                "-i".into(),
            ],
        });
    }
    None
}

fn which(binary: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| {
        let candidate = dir.join(binary);
        std::fs::metadata(&candidate)
            .map(|m| m.is_file())
            .unwrap_or(false)
    })
}

/// Match a layout by slug → name → shortcut, in that order. Each
/// match is exact; we don't fuzzy-match because the picker UI
/// passes whatever the user clicked (which is exact by
/// construction) and CLI users can complete via shell.
fn match_layout<'a>(layouts: &'a [Layout], needle: &str) -> Result<&'a Layout> {
    if let Some(l) = layouts.iter().find(|l| l.slug == needle) {
        return Ok(l);
    }
    if let Some(l) = layouts.iter().find(|l| l.name == needle) {
        return Ok(l);
    }
    if let Some(l) = layouts
        .iter()
        .find(|l| l.shortcuts.iter().any(|s| s == needle))
    {
        return Ok(l);
    }
    bail!(
        "no layout matches `{}` — known: {}",
        needle,
        layouts
            .iter()
            .map(|l| l.slug.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
}

/// Atomically swap the `margo-layout.conf` symlink to point at the
/// chosen layout. We write a fresh symlink at a unique sibling
/// path then `rename` it over the live target — the rename is
/// atomic on every Unix file system, so a `mctl reload` racing
/// with us can never see a half-updated link.
fn activate(config_dir: &Path, layout: &Layout) -> Result<()> {
    let active = config_dir.join(ACTIVE_LINK);
    let temp = config_dir.join(format!(
        "{}.{}.tmp",
        ACTIVE_LINK,
        std::process::id()
    ));

    // Clean any leftover from a previous crash.
    let _ = std::fs::remove_file(&temp);

    std::os::unix::fs::symlink(&layout.path, &temp)
        .with_context(|| format!("create symlink at {}", temp.display()))?;
    std::fs::rename(&temp, &active)
        .with_context(|| format!("rename {} → {}", temp.display(), active.display()))?;
    Ok(())
}

fn current_slug(config_dir: &Path) -> Option<String> {
    let target = std::fs::read_link(config_dir.join(ACTIVE_LINK)).ok()?;
    let file_name = target.file_name()?.to_string_lossy().to_string();
    let slug = file_name
        .strip_prefix("layout_")
        .and_then(|s| s.strip_suffix(".conf"))?;
    Some(slug.to_string())
}

/// Best-effort `mctl reload`. Failure is non-fatal — the user
/// might be running `margo-layout` outside a margo session, or
/// they may have set `--no-reload` for scripting reasons.
fn trigger_reload() {
    match Command::new("mctl")
        .arg("reload")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
    {
        Ok(status) if status.success() => {
            // Quiet success — `mctl reload` already echoes its own
            // confirmation line to stderr if it has anything to
            // say. Adding "Reloaded margo." here would just be
            // noise.
        }
        Ok(_) => {
            eprintln!("(mctl reload exited non-zero — margo may need a manual reload)");
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("(mctl not on PATH — run `mctl reload` manually to apply)");
        }
        Err(e) => {
            eprintln!("(mctl reload failed: {e})");
        }
    }
}
