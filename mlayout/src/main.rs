//! `mlayout` — quick-switch helper for margo's monitor layout.
//!
//! Drop one `layout_<name>.conf` file per setup into the margo
//! config directory (each containing the `monitorrule` lines that
//! describe that arrangement), point your main `config.conf` at
//! `source = mlayout.conf`, and use this binary to flip the
//! `mlayout.conf` symlink between the available files. A
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

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand};

mod apply;
mod capture;
mod parser;
mod presets;
mod preview;

use parser::Layout;

/// Default link basename inside the margo config directory. The
/// user's `config.conf` should `source = mlayout.conf` so this
/// path gets pulled into the active config on every reload.
const ACTIVE_LINK: &str = "mlayout.conf";

#[derive(Parser, Debug)]
#[command(
    name = "mlayout",
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
    /// First-time setup. Reads the live monitor configuration via
    /// `wlr-randr`, writes it as a layout file (`layout_<name>.conf`,
    /// default name `default`), checks the main `config.conf` for a
    /// `source = mlayout.conf` line and offers to add one if
    /// it's missing, then activates the new layout. Idempotent —
    /// safe to re-run; existing layout files aren't overwritten
    /// unless `--force` is passed.
    Init {
        /// Slug for the captured layout file (becomes
        /// `layout_<name>.conf`). Default: `default`.
        #[arg(long, default_value = "default")]
        name: String,
        /// Don't ask before adding `source = mlayout.conf` to
        /// `config.conf`.
        #[arg(short, long)]
        yes: bool,
        /// Overwrite an existing layout file with the same slug.
        #[arg(long)]
        force: bool,
        /// Skip the trailing `mctl reload`.
        #[arg(long)]
        no_reload: bool,
    },

    /// Generate preset layout files for the detected monitor
    /// arrangement. For a typical laptop+monitor setup this
    /// writes `layout_vertical-ext-top.conf`,
    /// `layout_horizontal-ext-left.conf`, etc. — the common
    /// arrangements you'd otherwise hand-write. Then prompts
    /// for which preset to activate (or use `--activate <slug>`
    /// to pick non-interactively).
    ///
    /// Idempotent: existing layout files are skipped unless
    /// `--force` is passed. Re-run after plugging in a new
    /// monitor to refresh the catalogue.
    Suggest {
        /// Activate the named preset immediately after
        /// generating. When omitted, you're prompted to pick
        /// from the freshly-written list.
        #[arg(long)]
        activate: Option<String>,
        /// Overwrite existing `layout_<preset>.conf` files
        /// (default: skip ones that already exist so hand
        /// edits aren't lost).
        #[arg(long)]
        force: bool,
        /// Also wire `source = mlayout.conf` into
        /// `config.conf` if it isn't already.
        #[arg(short, long)]
        yes: bool,
    },

    /// Snapshot the current monitor configuration as a new named
    /// layout file. Use this when you've physically rearranged
    /// monitors and want to bookmark the new setup as a profile —
    /// e.g. plug in a projector, run `mlayout new meeting`,
    /// later run `mlayout set meeting` to flip back.
    New {
        /// Slug for the new layout file (`layout_<name>.conf`).
        name: String,
        /// Display title (`#@ name`). Defaults to a Title-Cased
        /// version of the slug.
        #[arg(long)]
        title: Option<String>,
        /// One or more single-key shortcuts (`#@ shortcut`).
        #[arg(long)]
        shortcut: Vec<String>,
        /// Overwrite an existing layout file with the same slug.
        #[arg(long)]
        force: bool,
        /// Activate the captured layout immediately and reload.
        #[arg(long)]
        activate: bool,
        /// When `--activate` is passed, skip the trailing reload.
        #[arg(long)]
        no_reload: bool,
    },

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
    /// `mlayout.conf` symlinks to). Exit non-zero if no
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
    /// Omit `<name>` to preview every layout in the catalogue.
    Preview {
        /// Layout slug, name, or shortcut. If omitted, every
        /// layout is previewed in turn (active marked with `●`).
        name: Option<String>,
    },

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
        eprintln!("mlayout: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let config_dir = resolve_config_dir(cli.config_dir.as_deref())?;

    // `init` / `new` / `suggest` need to run BEFORE we gather
    // layouts — they're the bootstrap paths used when zero
    // layouts exist.
    match cli.command {
        Cmd::Init {
            name,
            yes,
            force,
            no_reload,
        } => return cmd_init(&config_dir, &name, yes, force, no_reload),
        Cmd::Suggest {
            activate,
            force,
            yes,
        } => return cmd_suggest(&config_dir, activate.as_deref(), force, yes),
        Cmd::New {
            name,
            title,
            shortcut,
            force,
            activate,
            no_reload,
        } => {
            return cmd_new(
                &config_dir,
                &name,
                title.as_deref(),
                &shortcut,
                force,
                activate,
                no_reload,
            )
        }
        _ => {}
    }

    let layouts = parser::gather_layouts(&config_dir)?;

    match cli.command {
        Cmd::List { preview, json } => cmd_list(&config_dir, &layouts, preview, json),
        Cmd::Current => cmd_current(&config_dir, &layouts),
        Cmd::Set { name, no_reload } => cmd_set(&config_dir, &layouts, &name, no_reload),
        Cmd::Next { no_reload } => cmd_cycle(&config_dir, &layouts, 1, no_reload),
        Cmd::Prev { no_reload } => cmd_cycle(&config_dir, &layouts, -1, no_reload),
        Cmd::Preview { name } => cmd_preview(&config_dir, &layouts, name.as_deref()),
        Cmd::Pick { no_gui, no_reload } => cmd_pick(&config_dir, &layouts, no_gui, no_reload),
        // Already handled above.
        Cmd::Init { .. } | Cmd::New { .. } | Cmd::Suggest { .. } => unreachable!("Init/New/Suggest are short-circuited above before this match"),
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

/// First-time setup. The user has the binary installed but no
/// layouts and no `source` line yet — we capture their current
/// monitor state, write it as a layout, wire `config.conf`, and
/// activate. Idempotent enough to re-run safely.
fn cmd_init(
    config_dir: &Path,
    name: &str,
    yes: bool,
    force: bool,
    no_reload: bool,
) -> Result<()> {
    ensure_config_dir(config_dir)?;

    let outputs = capture::capture_via_wlr_randr()?;
    println!("Detected {} active output(s):", outputs.len());
    for o in &outputs {
        println!(
            "  • {} — {}x{}@{}Hz at ({}, {}) scale {}",
            o.connector,
            o.width,
            o.height,
            o.refresh.round() as i32,
            o.x,
            o.y,
            o.scale
        );
    }

    let target = config_dir.join(format!("layout_{}.conf", name));
    if target.exists() && !force {
        bail!(
            "{} already exists — pass `--force` to overwrite, or pick \
             another name with `--name`.",
            target.display()
        );
    }

    let body = render_layout_file(name, &outputs, None, &[]);
    std::fs::write(&target, body)
        .with_context(|| format!("write {}", target.display()))?;
    println!("\nWrote {}.", target.display());

    // ── config.conf wiring ────────────────────────────────────
    let config_file = config_dir.join("config.conf");
    let needs_wire = !config_file.exists() || !config_already_sourced(&config_file)?;
    if needs_wire {
        if !yes && !confirm_prompt(&format!(
            "Add `source = mlayout.conf` to {}? [Y/n] ",
            config_file.display()
        ))? {
            println!(
                "Skipped wiring config.conf. Add this line yourself when \
                 ready:\n\n    source = mlayout.conf\n"
            );
        } else {
            wire_config_file(&config_file)?;
            println!("Wired {}.", config_file.display());
        }
    }

    // ── activate ──────────────────────────────────────────────
    let layout = parser::parse_file(&target)?;
    activate(config_dir, &layout)?;
    println!(
        "\nActivated layout `{}` ({} output(s)).",
        layout.name,
        layout.outputs.len()
    );

    if !no_reload {
        trigger_reload();
    }

    println!(
        "\nNext steps:\n  • `mlayout list`            — view your catalogue\n  • `mlayout new <name>`      — capture more setups\n  • `mlayout pick`            — interactive switcher\n  • Bind `mlayout next` to a hotkey for one-press cycling"
    );
    Ok(())
}

/// Generate preset arrangements for the detected monitor set,
/// show each one with an ASCII preview, and prompt the user to
/// pick. Only the picked preset gets written to disk — the
/// others stay in-memory and disappear when the command exits,
/// so the config directory doesn't get cluttered with seven
/// candidate files when the user only wants one.
///
/// Auto-cleanup: at the start of each run, delete any
/// `layout_<slug>.conf` whose first non-empty line is the
/// `presets::AUTOGEN_MARKER`. Hand-edited layouts (which the
/// user has stripped the marker from, or never had) survive.
fn cmd_suggest(
    config_dir: &Path,
    activate_slug: Option<&str>,
    _force: bool,
    yes: bool,
) -> Result<()> {
    use std::io::Write;

    ensure_config_dir(config_dir)?;

    // 1. Detect connected outputs.
    let outputs = capture::capture_via_wlr_randr()?;
    println!("Detected {} active output(s):", outputs.len());
    for o in &outputs {
        println!(
            "  • {} — {}x{}@{}Hz scale {}",
            o.connector,
            o.width,
            o.height,
            o.refresh.round() as i32,
            o.scale
        );
    }
    println!();

    // 2. Sweep up auto-generated presets from previous runs so
    //    re-running suggest doesn't accumulate stale files.
    let removed = cleanup_auto_generated_presets(config_dir)?;
    if !removed.is_empty() {
        println!(
            "Removed {} stale preset file(s) from previous suggest run(s).",
            removed.len()
        );
    }

    // 3. Generate the preset catalogue (in memory only — no
    //    disk yet).
    let presets = presets::generate(&outputs);
    if presets.is_empty() {
        bail!("no preset arrangements available for this output set");
    }

    // 4. config.conf wiring (same logic as `init`).
    let config_file = config_dir.join("config.conf");
    let needs_wire = !config_file.exists() || !config_already_sourced(&config_file)?;
    if needs_wire {
        let yes_to_wire = yes
            || confirm_prompt(&format!(
                "Add `source = mlayout.conf` to {}? [Y/n] ",
                config_file.display()
            ))?;
        if yes_to_wire {
            wire_config_file(&config_file)?;
            println!("Wired {}.", config_file.display());
        } else {
            println!(
                "Skipped wiring config.conf. Add this line yourself when ready:\n    source = mlayout.conf"
            );
        }
    }

    // 5. Show each preset with a preview, numbered. Drives the
    //    pick prompt below.
    let preview_layouts: Vec<parser::Layout> = presets
        .iter()
        .map(preset_to_layout)
        .collect();
    println!("\nAvailable arrangements:");
    for (i, (preset, layout)) in
        presets.iter().zip(preview_layouts.iter()).enumerate()
    {
        println!(
            "\n  {}. {} ({})",
            i + 1,
            preset.name,
            preset.slug
        );
        println!("     {}", preview::render_inline(layout));
    }
    println!();

    // 6. Pick.
    let chosen_idx = if let Some(needle) = activate_slug {
        presets
            .iter()
            .position(|p| p.slug == needle)
            .with_context(|| {
                format!(
                    "no preset matches `{}` — known: {}",
                    needle,
                    presets
                        .iter()
                        .map(|p| p.slug)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })?
    } else {
        eprint!("Pick a preset (number or slug, blank to abort): ");
        std::io::stderr().flush().ok();
        let mut input = String::new();
        std::io::stdin().read_line(&mut input).context("read stdin")?;
        let trimmed = input.trim();
        if trimmed.is_empty() {
            println!("(no selection — nothing written.)");
            return Ok(());
        }
        if let Ok(n) = trimmed.parse::<usize>() {
            if n == 0 || n > presets.len() {
                bail!("number out of range (1..={})", presets.len());
            }
            n - 1
        } else {
            presets
                .iter()
                .position(|p| p.slug == trimmed)
                .with_context(|| format!("no preset matches `{}`", trimmed))?
        }
    };

    let chosen = &presets[chosen_idx];
    let chosen_layout = &preview_layouts[chosen_idx];
    println!("\nWriting layout_{}.conf …", chosen.slug);

    // 7. Write ONLY the picked preset.
    let target = config_dir.join(format!("layout_{}.conf", chosen.slug));
    let shortcut = presets::shortcut_for(chosen.slug, &[]);
    let body = presets::render(chosen, shortcut);
    std::fs::write(&target, body)
        .with_context(|| format!("write {}", target.display()))?;

    // 8. Activate. Use the freshly-parsed file (not the
    //    in-memory preview layout) so the symlink target is
    //    the real layout file path on disk.
    let layout = parser::parse_file(&target)?;
    let _ = chosen_layout; // preview-only; don't symlink against the fake preset path
    activate(config_dir, &layout)?;
    println!("Activated layout `{}`.", chosen.name);
    trigger_reload();

    println!(
        "\nNext steps:\n  • Re-run `mlayout suggest` to switch arrangement\n  • `mlayout new <name>` to capture a custom one\n  • `mlayout pick` for an interactive switcher across hand-edited layouts"
    );
    Ok(())
}

/// Convert a [`presets::Preset`] into a [`parser::Layout`] so the
/// rest of the code (preview render, activate symlink) can treat
/// it like a parsed layout file. Cheap; the data is already here.
fn preset_to_layout(preset: &presets::Preset) -> parser::Layout {
    let outputs = preset
        .outputs
        .iter()
        .map(|o| parser::LayoutOutput {
            connector: o.connector.clone(),
            label: o.label.map(str::to_string),
            color: o.color,
            x: o.x,
            y: o.y,
            has_position: true,
            width: o.width,
            height: o.height,
            transform: 0,
        })
        .collect();
    parser::Layout {
        path: PathBuf::from(format!("(preset:{})", preset.slug)),
        slug: preset.slug.to_string(),
        name: preset.name.clone(),
        shortcuts: Vec::new(),
        outputs,
    }
}

/// Delete every `layout_<slug>.conf` in `config_dir` whose body
/// starts with the [`presets::AUTOGEN_MARKER`] line. Returns the
/// list of slugs removed so the caller can summarise. Does NOT
/// delete the active-layout symlink target — even if it's
/// auto-generated, removing it under the user's feet would
/// break the running session until the next `set`.
fn cleanup_auto_generated_presets(config_dir: &Path) -> Result<Vec<String>> {
    let active = current_slug(config_dir);
    let active_target = active.as_ref().map(|s| format!("layout_{}.conf", s));

    let mut removed = Vec::new();
    let entries = match std::fs::read_dir(config_dir) {
        Ok(e) => e,
        Err(_) => return Ok(removed),
    };
    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let name = entry.file_name();
        let name_str = name.to_string_lossy().to_string();
        if !name_str.starts_with("layout_") || !name_str.ends_with(".conf") {
            continue;
        }
        if Some(&name_str) == active_target.as_ref() {
            continue;
        }
        let path = entry.path();
        let Ok(body) = std::fs::read_to_string(&path) else {
            continue;
        };
        // New format → first non-empty line is the marker.
        // Legacy format (pre-marker, suggest v0) → starts with
        // "# Generated by `mlayout suggest`". Both count as
        // auto-generated for cleanup purposes; hand-edited
        // layouts (`init` output) start with "# Generated by
        // `mlayout` from the live monitor configuration"
        // which we deliberately don't sweep.
        let is_autogen = body
            .lines()
            .take(3)
            .any(|l| {
                let t = l.trim();
                t == presets::AUTOGEN_MARKER.trim()
                    || t.starts_with("# Generated by `mlayout suggest`")
            });
        if is_autogen
            && std::fs::remove_file(&path).is_ok() {
                let slug = name_str
                    .trim_start_matches("layout_")
                    .trim_end_matches(".conf")
                    .to_string();
                removed.push(slug);
            }
    }
    Ok(removed)
}

/// Snapshot the current monitor state into a new named layout
/// file. The user runs this AFTER physically rearranging
/// monitors: "I just plugged in the projector, let me bookmark
/// this as `meeting` so I can come back to it."
fn cmd_new(
    config_dir: &Path,
    name: &str,
    title: Option<&str>,
    shortcuts: &[String],
    force: bool,
    activate_now: bool,
    no_reload: bool,
) -> Result<()> {
    ensure_config_dir(config_dir)?;

    let target = config_dir.join(format!("layout_{}.conf", name));
    if target.exists() && !force {
        bail!(
            "{} already exists — pass `--force` to overwrite.",
            target.display()
        );
    }

    let outputs = capture::capture_via_wlr_randr()?;
    let body = render_layout_file(name, &outputs, title, shortcuts);
    std::fs::write(&target, body)
        .with_context(|| format!("write {}", target.display()))?;

    println!("Wrote {}:", target.display());
    for o in &outputs {
        println!(
            "  • {} — {}x{}@{}Hz at ({}, {}) scale {}",
            o.connector,
            o.width,
            o.height,
            o.refresh.round() as i32,
            o.x,
            o.y,
            o.scale
        );
    }

    if activate_now {
        let layout = parser::parse_file(&target)?;
        activate(config_dir, &layout)?;
        println!("\nActivated layout `{}`.", layout.name);
        if !no_reload {
            trigger_reload();
        }
    } else {
        println!(
            "\nRun `mlayout set {}` to activate this layout.",
            name
        );
    }
    Ok(())
}

/// Build the body of a `layout_<name>.conf` file from a captured
/// snapshot. Mirrors the hand-written format users would produce
/// themselves — headers, blank-line separators, `#@` directives
/// in the same shape the README documents.
fn render_layout_file(
    slug: &str,
    outputs: &[capture::CapturedOutput],
    title: Option<&str>,
    shortcuts: &[String],
) -> String {
    let mut buf = String::new();
    buf.push_str("# Generated by `mlayout` from the live monitor configuration.\n");
    buf.push_str("# Edit freely — `mlayout` will preserve your edits on\n");
    buf.push_str("# subsequent reads. Re-capture with `mlayout new ");
    buf.push_str(slug);
    buf.push_str(" --force` if you rearrange.\n\n");
    let display_title = title.map(str::to_string).unwrap_or_else(|| title_case(slug));
    buf.push_str(&format!("#@ name = \"{}\"\n", display_title));
    for s in shortcuts {
        buf.push_str(&format!("#@ shortcut = {}\n", s));
    }
    // Trailing blank line — same regardless of whether
    // shortcuts existed (used to be branched but the two arms
    // were identical; clippy caught the dead branch).
    buf.push('\n');

    for o in outputs {
        if let Some(label) = capture::auto_label_for_connector(&o.connector) {
            buf.push_str(&format!("#@ output_name = \"{}\"\n", label));
        }
        if let Some(c) = capture::auto_color_for_connector(&o.connector) {
            buf.push_str(&format!("#@ color = {}\n", c));
        }
        buf.push_str(&format!(
            "monitorrule = {}\n\n",
            capture::to_monitorrule_value(o)
        ));
    }
    buf
}

fn title_case(slug: &str) -> String {
    let parts: Vec<String> = slug
        .split(['_', '-'])
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect();
    parts.join(" ")
}

/// Marker comment used to identify the line `mlayout` adds
/// to `config.conf`. Lets us check on re-run whether wiring is
/// already in place without false positives from user-added
/// `source =` lines pointing somewhere else.
const WIRE_MARKER: &str = "# managed by mlayout";

fn config_already_sourced(path: &Path) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let body = std::fs::read_to_string(path)
        .with_context(|| format!("read {}", path.display()))?;
    Ok(body
        .lines()
        .any(|l| is_layout_source_line(l.trim_start())))
}

fn is_layout_source_line(line: &str) -> bool {
    if !line.starts_with("source") {
        return false;
    }
    // `source = mlayout.conf` — accept with or without
    // surrounding whitespace, with optional quotes.
    let Some(rest) = line.strip_prefix("source") else {
        return false;
    };
    let rest = rest.trim_start();
    let Some(rest) = rest.strip_prefix('=') else {
        return false;
    };
    let rest = rest.trim();
    let rest = rest.trim_matches('"').trim_matches('\'');
    rest == "mlayout.conf"
}

fn wire_config_file(path: &Path) -> Result<()> {
    let mut existing = if path.exists() {
        std::fs::read_to_string(path)
            .with_context(|| format!("read {}", path.display()))?
    } else {
        String::new()
    };
    if !existing.is_empty() && !existing.ends_with('\n') {
        existing.push('\n');
    }
    existing.push('\n');
    existing.push_str(WIRE_MARKER);
    existing.push('\n');
    existing.push_str("source = mlayout.conf\n");

    std::fs::write(path, existing)
        .with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

fn ensure_config_dir(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir)
        .with_context(|| format!("create config dir {}", dir.display()))?;
    Ok(())
}

fn confirm_prompt(prompt: &str) -> Result<bool> {
    eprint!("{prompt}");
    std::io::stderr().flush().ok();
    let mut input = String::new();
    std::io::stdin()
        .read_line(&mut input)
        .context("read stdin")?;
    let answer = input.trim().to_lowercase();
    Ok(answer.is_empty() || answer == "y" || answer == "yes")
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
            "Create one or more layout_<name>.conf files there to use mlayout."
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
        bail!("no active layout (run `mlayout set <name>` to pick one)");
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

fn cmd_preview(
    config_dir: &Path,
    layouts: &[Layout],
    needle: Option<&str>,
) -> Result<()> {
    if let Some(needle) = needle {
        let layout = match_layout(layouts, needle)?;
        println!("{}", layout.name);
        println!("{}", preview::render_ascii(layout, 80));
        return Ok(());
    }

    if layouts.is_empty() {
        bail!(
            "no layouts in {} — run `mlayout init` first",
            config_dir.display()
        );
    }
    let active = current_slug(config_dir);
    for (i, layout) in layouts.iter().enumerate() {
        if i > 0 {
            println!();
        }
        let marker = if Some(&layout.slug) == active.as_ref() {
            "● "
        } else {
            "  "
        };
        let shortcuts = if layout.shortcuts.is_empty() {
            String::new()
        } else {
            format!(" [{}]", layout.shortcuts.join(", "))
        };
        println!("{}{}{}", marker, layout.name, shortcuts);
        println!("{}", preview::render_ascii(layout, 80));
    }
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

    if !no_gui
        && let Some(picker) = detect_graphical_picker()
    {
        return run_graphical_pick(&picker, config_dir, layouts, no_reload);
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

    let mut child = Command::new(picker.binary)
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

/// Atomically swap the `mlayout.conf` symlink to point at the
/// chosen layout AND push the geometry to the live session via
/// `wlr-randr` so outputs reposition immediately. The symlink
/// makes the choice durable; the wlr-randr call makes it visible.
///
/// Three steps, in order:
///   1. write fresh symlink at a unique sibling path
///   2. atomic `rename` over the live `mlayout.conf` target
///      (so a racing `mctl reload` can never see a half-updated link)
///   3. spawn `wlr-randr` with all positions / modes from the
///      layout file — atomic transaction inside wlr-randr, applies
///      to all outputs in one configure event
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

    // Push geometry to the live session. Best-effort — failure
    // here just means the user has to log out / back in to see
    // the new arrangement; the layout file is still in place.
    if let Err(err) = apply::apply(layout) {
        eprintln!("(layout activated but `wlr-randr` apply failed: {err:#})");
    }
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
/// might be running `mlayout` outside a margo session, or
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
