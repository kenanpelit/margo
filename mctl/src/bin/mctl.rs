//! mctl — margo compositor control tool (replaces mmsg)
//!
//! Talks to margo over its Unix-socket IPC (get / watch / dispatch).
//! Connects to the compositor via the standard WAYLAND_DISPLAY socket.

// Submodules live in src/bin/mctl/: a bin root's own `mod x;` would resolve
// to src/bin/x.rs, which Cargo builds as another binary.
#[path = "mctl/cli.rs"]
mod cli;
#[path = "mctl/config.rs"]
mod config;
#[path = "mctl/plugin.rs"]
mod plugin;
#[path = "mctl/query.rs"]
mod query;
#[path = "mctl/rules.rs"]
mod rules;
#[path = "mctl/twilight.rs"]
mod twilight;

use anyhow::{Result, bail};
use clap::{CommandFactory, Parser};
use clap_complete::Shell;

use cli::{Args, Command, ConfigCmd, LogCmd, TwilightCmd};
use config::{
    cmd_check_config, cmd_config_diff, cmd_config_errors, cmd_config_list, cmd_config_rollback,
};
use mctl::actions::ACTIONS;
use plugin::cmd_plugin;
use query::{
    cmd_clients, cmd_focused, cmd_mirror, cmd_outputs, cmd_perf, print_status_rich, read_state_file,
};
use rules::cmd_rules;
use twilight::{cmd_twilight_preset, cmd_twilight_status};

// ── IPC state machine ─────────────────────────────────────────────────────────

// ── Dispatch impls ────────────────────────────────────────────────────────────

// ── Main ──────────────────────────────────────────────────────────────────────

fn main() -> Result<()> {
    let args = Args::parse();

    // Subcommands that don't need (or want) a Wayland connection —
    // documentation / scripting / config-introspection helpers that
    // should work outside a margo session, e.g. when the user is
    // generating completions during a package install or
    // sanity-checking a windowrule pattern in their editor.
    match &args.command {
        Command::Actions {
            verbose,
            group,
            names,
        } => {
            return cmd_actions(*verbose, group.as_deref(), *names);
        }
        Command::Completions { shell } => {
            return cmd_completions(*shell);
        }
        // doctor manages its own socket connection (and degrades when
        // margo isn't running), so it lives among the no-preconnect group.
        Command::Doctor => {
            return mctl::doctor::run();
        }
        Command::Rules {
            config,
            appid,
            title,
            verbose,
        } => {
            return cmd_rules(config.as_deref(), appid, title, *verbose);
        }
        Command::CheckConfig { config, strict } => {
            return cmd_check_config(config.as_deref(), *strict);
        }
        // The state-file commands don't need a Wayland connection.
        // They read whatever margo last wrote out.
        Command::Clients {
            json,
            tag,
            monitor,
            app_id,
            wide,
        } => {
            return cmd_clients(*json, *tag, monitor.as_deref(), app_id.as_deref(), *wide);
        }
        Command::Outputs { json } => {
            return cmd_outputs(*json);
        }
        Command::Perf { json, watch } => {
            return cmd_perf(*json, *watch);
        }
        Command::Focused { json } => {
            return cmd_focused(*json);
        }
        Command::ConfigErrors => {
            return cmd_config_errors();
        }
        Command::Config {
            action: ConfigCmd::List { config },
        } => {
            return cmd_config_list(config.as_deref());
        }
        Command::Config {
            action: ConfigCmd::Diff { n, config },
        } => {
            return cmd_config_diff(*n, config.as_deref());
        }
        Command::Twilight {
            action: TwilightCmd::Status { json },
        } => {
            return cmd_twilight_status(*json);
        }
        Command::Mirror { action } => {
            return cmd_mirror(action);
        }
        Command::Twilight {
            action: TwilightCmd::Preset { action },
        } => {
            return cmd_twilight_preset(action);
        }
        // Plugins live as files under ~/.config/margo/plugins/; list/toggle is
        // pure filesystem, no compositor connection needed.
        Command::Plugin { action } => {
            return cmd_plugin(action);
        }
        _ => {}
    }

    match args.command {
        Command::Dispatch {
            name,
            args: cmd_args,
        } => {
            let a: Vec<&str> = cmd_args.iter().map(String::as_str).collect();
            send_dispatch(&name, &a)?;
        }
        Command::Log { action } => match action {
            LogCmd::Level { level } => send_dispatch("loglevel", &[&level])?,
            LogCmd::Enable => send_dispatch("logenabled", &["true"])?,
            LogCmd::Disable => send_dispatch("logenabled", &["false"])?,
            LogCmd::Path => {
                let dir = margo_logging::logs_dir();
                println!("dir:     {}", dir.display());
                println!("current: {}", dir.join("margo-latest.log").display());
            }
            LogCmd::Open => {
                let dir = margo_logging::logs_dir();
                let _ = std::fs::create_dir_all(&dir);
                std::process::Command::new("xdg-open")
                    .arg(&dir)
                    .spawn()
                    .map(|_| ())
                    .unwrap_or_else(|e| eprintln!("could not open {}: {e}", dir.display()));
            }
        },
        Command::Run { file } => {
            // W3.2 — resolve to an absolute path so the
            // compositor (different CWD) reads the same file.
            let abs = match std::fs::canonicalize(&file) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("mctl run: cannot find {}: {e}", file.display());
                    std::process::exit(1);
                }
            };
            let abs_str = abs.to_string_lossy().to_string();
            // Single non-numeric arg lands in `arg.v` (the path), which
            // run_script reads.
            send_dispatch("run_script", &[&abs_str])?;
        }
        Command::Migrate { file, from, output } => {
            // W4.4 — pure-function translation, no compositor IPC
            // involved. We literally just read the source, run it
            // through `mctl::migrate`, write the result.
            let contents = match std::fs::read_to_string(&file) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("mctl migrate: read {}: {e}", file.display());
                    std::process::exit(1);
                }
            };
            let format = match from
                .as_deref()
                .and_then(mctl::migrate::SourceFormat::parse_name)
                .or_else(|| mctl::migrate::SourceFormat::detect(&file, &contents))
            {
                Some(f) => f,
                None => {
                    eprintln!(
                        "mctl migrate: cannot detect source format for {}\n  \
                         override with `--from hyprland` or `--from sway`",
                        file.display()
                    );
                    std::process::exit(2);
                }
            };
            let result = mctl::migrate::migrate(format, &contents);
            // Warnings → stderr so the user sees them but stdout
            // is clean if they pipe directly into config.conf.
            for w in &result.warnings {
                eprintln!("warning: {w}");
            }
            match output {
                Some(path) => {
                    // Back up an existing file rather than silently
                    // clobbering it — a second `migrate` run, or a
                    // copy-pasted command pointed at the wrong `-o`, must
                    // never destroy a config the user has since hand-edited
                    // with no way to get it back. Non-interactive on
                    // purpose: this command is also used in scripts/install
                    // pipelines, so back up rather than prompt.
                    if path.exists() {
                        let mut backup_name = path
                            .file_name()
                            .map(|n| n.to_os_string())
                            .unwrap_or_else(|| std::ffi::OsString::from("output"));
                        backup_name.push(".bak");
                        let backup = path.with_file_name(&backup_name);
                        if let Err(e) = std::fs::rename(&path, &backup) {
                            eprintln!(
                                "mctl migrate: {} exists and could not be backed up ({e}); refusing to overwrite",
                                path.display()
                            );
                            std::process::exit(1);
                        }
                        eprintln!(
                            "mctl migrate: backed up existing {} to {}",
                            path.display(),
                            backup.display()
                        );
                    }
                    if let Err(e) = std::fs::write(&path, &result.output) {
                        eprintln!("mctl migrate: write {}: {e}", path.display());
                        std::process::exit(1);
                    }
                    eprintln!(
                        "mctl migrate: wrote {} ({} warning{})",
                        path.display(),
                        result.warnings.len(),
                        if result.warnings.len() == 1 { "" } else { "s" }
                    );
                }
                None => {
                    print!("{}", result.output);
                }
            }
            // Skip the IPC roundtrip — this command is offline.
            return Ok(());
        }
        Command::Tags { mask, toggle } => {
            send_dispatch(
                "settagset",
                &[&mask.to_string(), if toggle != 0 { "1" } else { "0" }],
            )?;
        }
        Command::ClientTags { and_mask, xor_mask } => {
            send_dispatch(
                "setclienttags",
                &[&and_mask.to_string(), &xor_mask.to_string()],
            )?;
        }
        Command::Layout { index } => {
            send_dispatch("setlayoutindex", &[&index.to_string()])?;
        }
        Command::Quit => {
            send_dispatch("quit", &[])?;
        }
        Command::Reload { force } => {
            // Pre-flight: validate before we touch IPC. The compositor
            // also fail-soft loads (it'll keep the previous config if
            // the new one parses to garbage), but stopping here is far
            // friendlier — the user sees a coloured caret-and-line
            // diagnostic in the terminal rather than a tracing log
            // line buried in journalctl. `--force` keeps the old
            // permissive behaviour.
            if !force {
                let report = margo_config::validator::validate_config(None).unwrap_or_else(|e| {
                    eprintln!("error: validator could not read config: {e}");
                    std::process::exit(2);
                });
                if report.has_errors() {
                    let colored = std::io::IsTerminal::is_terminal(&std::io::stderr());
                    for d in &report.diagnostics {
                        if d.is_error() {
                            eprint!("{}", d.render(colored));
                            eprintln!();
                        }
                    }
                    let err_count = report.errors().count();
                    eprintln!(
                        "mctl reload: refusing to reload — {} error{} in config. \
                         Fix them and retry, or pass `--force` to reload anyway.",
                        err_count,
                        if err_count == 1 { "" } else { "s" }
                    );
                    std::process::exit(1);
                }
                if report.has_warnings() {
                    let colored = std::io::IsTerminal::is_terminal(&std::io::stderr());
                    for d in &report.diagnostics {
                        if !d.is_error() {
                            eprint!("{}", d.render(colored));
                            eprintln!();
                        }
                    }
                    eprintln!(
                        "mctl reload: proceeding with {} warning{}.",
                        report.warnings().count(),
                        if report.warnings().count() == 1 {
                            ""
                        } else {
                            "s"
                        }
                    );
                }
            }
            send_dispatch("reload_config", if force { &["force"] } else { &[] })?;
        }
        Command::Twilight { action } => {
            // Status is handled higher up (no IPC needed). The
            // remaining four actions are dispatched to the
            // compositor. Slot mapping per `Arg` doc:
            //   preview → slot 1 = Kelvin, slot 2 = gamma %
            //   test    → slot 1 = duration_s
            //   set     → slot 4 = "field=value"
            //   reset   → no args
            match action {
                TwilightCmd::Status { .. } => {
                    unreachable!("Status is handled before this match arm")
                }
                TwilightCmd::Preset { .. } => {
                    unreachable!("Preset is handled before this match arm")
                }
                TwilightCmd::Preview { kelvin, gamma } => {
                    send_dispatch(
                        "twilight_preview",
                        &[&kelvin.to_string(), &gamma.to_string()],
                    )?;
                }
                TwilightCmd::Test { seconds } => {
                    let s = seconds.clamp(1, 60);
                    send_dispatch("twilight_test", &[&s.to_string()])?;
                }
                TwilightCmd::Set { spec } => {
                    let Some(spec) = spec else {
                        eprintln!(
                            "error: `mctl twilight set` needs a `field=value` argument\n\
                             \n\
                             SUPPORTED FIELDS:\n  \
                               day_temp=<1000-25000>      Daytime colour temperature (K)\n  \
                               night_temp=<1000-25000>    Night colour temperature (K)\n  \
                               day_gamma=<10-200>         Daytime brightness (%)\n  \
                               night_gamma=<10-200>       Night brightness (%)\n  \
                               transition_s=<30-7200>     Day-night fade duration (s)\n  \
                               enabled=<0|1>              Master on/off (alias: twilight)\n  \
                               mode=<geo|manual|static|schedule>\n\
                             \n\
                             EXAMPLE: mctl twilight set mode=schedule\n\
                             See `mctl twilight set --help` for full details."
                        );
                        std::process::exit(2);
                    };
                    // Single non-numeric arg → `arg.v` (the spec).
                    send_dispatch("twilight_set", &[&spec])?;
                }
                TwilightCmd::Reset => {
                    send_dispatch("twilight_reset", &[])?;
                }
                TwilightCmd::Toggle => {
                    send_dispatch("twilight_toggle", &[])?;
                }
            }
        }
        Command::Config { action } => match action {
            ConfigCmd::List { .. } | ConfigCmd::Diff { .. } => {
                unreachable!("List/Diff return early in main's first match block")
            }
            ConfigCmd::Rollback { n, yes, config } => {
                cmd_config_rollback(n, yes, config.as_deref())?;
            }
        },
        Command::Theme { preset } => {
            // Single non-numeric arg lands in `arg.v` (the preset name),
            // which the `theme` action reads.
            send_dispatch("theme", &[&preset])?;
        }
        Command::SessionSave => {
            send_dispatch("session_save", &[])?;
        }
        Command::SessionLoad => {
            send_dispatch("session_load", &[])?;
        }
        Command::Status { json } => {
            let snap = read_state_file()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&snap)?);
            } else {
                print_status_rich(&snap, args.output.as_deref());
            }
        }
        Command::Get { topic } => {
            let reply = mctl::ipc_client::request_once(&format!("get {}", topic.join(" ")))?;
            println!("{}", serde_json::to_string_pretty(&reply)?);
        }
        Command::Watch { topic } => {
            if topic.is_empty() {
                // Bare `watch`: live status stream with human rendering.
                mctl::ipc_client::watch_stream("watch state", |frame| {
                    print_status_rich(&frame, args.output.as_deref());
                    true
                })?;
            } else {
                // `watch <topic>`: raw JSON frames.
                mctl::ipc_client::watch_stream(&format!("watch {}", topic.join(" ")), |frame| {
                    println!("{frame}");
                    true
                })?;
            }
        }
        Command::Actions { .. }
        | Command::Completions { .. }
        | Command::Doctor
        | Command::Rules { .. }
        | Command::CheckConfig { .. }
        | Command::ConfigErrors
        | Command::Clients { .. }
        | Command::Outputs { .. }
        | Command::Perf { .. }
        | Command::Focused { .. }
        | Command::Mirror { .. }
        | Command::Plugin { .. } => {
            // These return early at the top of `main`; this arm only
            // exists to keep the match exhaustive.
            unreachable!("offline commands return early in main");
        }
    }

    Ok(())
}

fn cmd_actions(verbose: bool, group_filter: Option<&str>, names_only: bool) -> Result<()> {
    use std::io::Write;
    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    if names_only {
        // Newline-separated dump of every accepted spelling
        // (canonical names + aliases). Drives shell-completion
        // generators (`compgen -W "$(mctl actions --names)"`).
        for n in mctl::actions::all_names() {
            writeln!(out, "{n}")?;
        }
        return Ok(());
    }

    // Validate the optional group filter case-insensitively against
    // the labels declared on `Group::label`.
    let filter_label = group_filter.map(|s| s.to_lowercase());
    let mut matched_any = false;

    for g in mctl::actions::GROUPS {
        if let Some(filter) = filter_label.as_deref() {
            let label_lc = g.label().to_lowercase();
            if !label_lc.contains(filter) {
                continue;
            }
        }
        let group_actions: Vec<_> = ACTIONS.iter().filter(|a| a.group == g).collect();
        if group_actions.is_empty() {
            continue;
        }
        matched_any = true;
        writeln!(out)?;
        writeln!(
            out,
            "── {} ─────────────────────────────────────",
            g.label()
        )?;
        for action in group_actions {
            let mut spellings = String::from(action.name);
            for alias in action.aliases {
                spellings.push_str(", ");
                spellings.push_str(alias);
            }
            if action.args.is_empty() {
                writeln!(out, "  {spellings}")?;
            } else {
                writeln!(out, "  {spellings}  {}", action.args)?;
            }
            writeln!(out, "      {}", action.summary)?;
            if verbose && !action.detail.is_empty() {
                for line in action.detail.split('\n') {
                    writeln!(out, "      {}", line.trim_start())?;
                }
            }
        }
    }

    if !matched_any && let Some(filter) = group_filter {
        bail!(
            "no group matches '{filter}'. Available: \
             Tag, Focus, Layout, Scroller, Window, Scratchpad, Overview, System"
        );
    }
    Ok(())
}

fn cmd_completions(shell: Shell) -> Result<()> {
    // This is the single source of truth for mctl's completions: the
    // checked-in scripts under `contrib/completions/` are just this
    // output captured by `just completions` (contrib/completions/
    // generate.sh). Static only — dispatch action names are free text
    // (enumerate them with `mctl actions --names`).
    let mut cmd = Args::command();
    let bin_name = "mctl";
    clap_complete::generate(shell, &mut cmd, bin_name, &mut std::io::stdout());
    Ok(())
}

// ── State-file consumers ────────────────────────────────────────

/// Send a `dispatch <action> [args…]` request over the IPC socket and
/// surface any error frame. Empty args are skipped (the line protocol
/// is whitespace-split); the value-bearing actions read their single
/// non-numeric arg as `arg.v`.
fn send_dispatch(action: &str, args: &[&str]) -> Result<()> {
    let req = dispatch_request(action, args);
    let reply = mctl::ipc_client::request_once(&req)?;
    check_dispatch_reply(action, &reply)
}

/// The `dispatch <action> [args…]` request line (without the newline).
/// Empty args are dropped: the server tokenises on whitespace, so they
/// could not be told apart anyway.
fn dispatch_request(action: &str, args: &[&str]) -> String {
    let mut req = String::from("dispatch ");
    req.push_str(action);
    for a in args {
        if a.is_empty() {
            continue;
        }
        req.push(' ');
        req.push_str(a);
    }
    req
}

/// A dispatch succeeded only if margo said `ok: true`; anything else is a
/// failure, with margo's `error` text when it sent one.
fn check_dispatch_reply(action: &str, reply: &serde_json::Value) -> Result<()> {
    match mctl::ipc_client::dispatch_reply_error(reply) {
        None => Ok(()),
        Some(err) => bail!("dispatch {action}: {err}"),
    }
}

#[cfg(test)]
#[path = "../man_page_tests.rs"]
mod man_page_tests;

#[cfg(test)]
mod tests {
    #[test]
    fn dispatch_reply_needs_an_explicit_ok() {
        use super::check_dispatch_reply;
        use serde_json::json;
        assert!(check_dispatch_reply("view", &json!({ "ok": true })).is_ok());
        let err = check_dispatch_reply("frob", &json!({ "error": "unknown action: frob" }))
            .unwrap_err()
            .to_string();
        assert!(err.contains("unknown action"), "{err}");
        // `ok:false` with no `error` used to be reported as success.
        assert!(check_dispatch_reply("view", &json!({ "ok": false })).is_err());
        assert!(check_dispatch_reply("view", &json!({})).is_err());
    }
}

#[cfg(test)]
mod helper_tests {
    use super::*;

    #[test]
    fn dispatch_request_joins_action_and_args() {
        assert_eq!(dispatch_request("quit", &[]), "dispatch quit");
        assert_eq!(dispatch_request("view", &["4"]), "dispatch view 4");
        assert_eq!(
            dispatch_request("settagset", &["5", "0"]),
            "dispatch settagset 5 0"
        );
        assert_eq!(
            dispatch_request("spawn", &["", "foot", ""]),
            "dispatch spawn foot",
            "empty args are dropped"
        );
    }
}
