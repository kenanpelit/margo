//! mctl — margo compositor control tool (replaces mmsg)
//!
//! Uses the zdwl_ipc_unstable_v2 Wayland protocol to query and control margo.
//! Connects to the compositor via the standard WAYLAND_DISPLAY socket.

use anyhow::{bail, Result};
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::Shell;
use wayland_client::{
    globals::{registry_queue_init, GlobalListContents},
    protocol::{wl_output, wl_registry},
    Connection, Dispatch, EventQueue, QueueHandle,
};

use margo_ipc::actions::{ACTIONS, Group};
use margo_ipc::protocols::dwl_ipc::{
    zdwl_ipc_manager_v2::{self, ZdwlIpcManagerV2},
    zdwl_ipc_output_v2::{self, ZdwlIpcOutputV2},
};

// ── CLI ───────────────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(
    name = "mctl",
    about = "margo compositor control",
    long_about = "Query and control the margo Wayland compositor via the dwl-ipc-v2 protocol.\n\
                  \n\
                  EXAMPLES:\n  \
                    mctl status                         # current focused-client / tag state\n  \
                    mctl watch                          # stream state updates (Ctrl-C to stop)\n  \
                    mctl tags 128                       # switch active tagset to tag 8 (1<<7)\n  \
                    mctl layout 1                       # switch to layout #1 (scroller)\n  \
                    mctl dispatch togglefloating        # toggle focused window's float state\n  \
                    mctl dispatch view 4                # switch to tag 3 (1<<2)\n  \
                    mctl dispatch spawn 'kitty -e htop' # run shell command\n  \
                    mctl actions                        # list every dispatch action\n  \
                    mctl completions bash               # emit bash completion script\n  \
                  \n\
                  Bind-line equivalent in `~/.config/margo/config.conf`:\n  \
                    bind = super+ctrl,Escape,unscratchpad\n  \
                    bind = super,Return,spawn,kitty\n  \
                    bind = super,1,view,1\n  \
                  \n\
                  Tag bitmask convention: tag N corresponds to `1 << (N - 1)`.\n  \
                  Tag 1 = 1, tag 2 = 2, tag 3 = 4, tag 4 = 8, … tag 8 = 128, tag 9 = 256.\n  \
                  Use `mctl actions --group Tag` for the full tag-action reference."
)]
struct Args {
    /// Output to target (default: focused, falls back to first)
    #[arg(short, long)]
    output: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Dispatch a compositor command by name (margo's internal dispatch table)
    #[command(
        alias = "d",
        long_about = "Dispatch a compositor command by name.\n\
                      \n\
                      The action <NAME> is the same string used in `bind = MODS,KEY,<NAME>,<args>` \
                      lines in `config.conf`. Up to 5 trailing args are forwarded as the bind's \
                      arg slots (v, v2, v3, i, i2 — see `mctl actions --verbose` for shapes).\n\
                      \n\
                      EXAMPLES:\n  \
                        mctl dispatch killclient            # close focused window\n  \
                        mctl dispatch togglefullscreen\n  \
                        mctl dispatch focusdir right        # focus next window to the right\n  \
                        mctl dispatch view 128              # switch to tag 8\n  \
                        mctl dispatch tagview 4             # move focused to tag 3 + follow\n  \
                        mctl dispatch setlayout scroller\n  \
                        mctl dispatch movewin 40 0          # move floating window 40 px right\n  \
                        mctl dispatch spawn 'firefox --new-window https://…'\n  \
                      \n\
                      Run `mctl actions` for the complete list."
    )]
    Dispatch {
        /// Dispatch action name (e.g. `view`, `togglefloating`, `setlayout`, `killclient`).
        /// `mctl actions` prints every accepted name.
        name: String,
        /// Up to 5 positional arguments forwarded to the action.
        #[arg(trailing_var_arg = true)]
        args: Vec<String>,
    },

    /// Set the active tagset on an output (raw tag bitmask)
    #[command(
        long_about = "Set the active tagset on an output (raw tag bitmask).\n\
                      \n\
                      Bitmask is 1-indexed: tag 1 = 1, tag 2 = 2, tag 3 = 4, tag N = 1 << (N - 1). \
                      To view multiple tags simultaneously, OR the bits (`5` = tags 1+3).\n\
                      \n\
                      EXAMPLES:\n  \
                        mctl tags 1            # show tag 1 only\n  \
                        mctl tags 128          # show tag 8 only (Spotify in the user's setup)\n  \
                        mctl tags 5            # show tags 1 + 3 simultaneously\n  \
                        mctl tags 128 1        # toggle tag 8 in the active set (don't replace)\n  \
                        mctl -o eDP-1 tags 64  # change tagset on a specific output"
    )]
    Tags {
        /// Tag bitmask. Tag N → `1 << (N - 1)`.
        mask: u32,
        /// 1 = toggle tag in the active set, 0 = replace active set entirely.
        #[arg(default_value = "0")]
        toggle: u32,
    },

    /// Mutate the focused client's tag bitmask (advanced)
    #[command(
        long_about = "Mutate the focused client's tag bitmask.\n\
                      \n\
                      Applies `(tags & AND_MASK) ^ XOR_MASK`. Almost no one calls this \
                      directly — `mctl dispatch tag <MASK>` (replace) and \
                      `mctl dispatch toggletag <MASK>` (toggle one bit) cover the user-side \
                      cases. This raw form exists so dwl-ipc-v2 clients (status bars) can \
                      build their own tag-manipulation UI without going through dispatch."
    )]
    ClientTags {
        /// Bitmask AND'd with the client's current tags.
        and_mask: u32,
        /// Bitmask XOR'd with the result.
        xor_mask: u32,
    },

    /// Set layout by index (0-based, matches the compositor's layout list)
    #[command(
        long_about = "Set the layout for the focused tag by 0-based index.\n\
                      \n\
                      The index ordering matches the list announced by the compositor at \
                      registry-bind time — see the `layouts:` line in `mctl status`. \
                      Prefer `mctl dispatch setlayout <name>` for stability across config \
                      changes (the index moves when the layout list reorders)."
    )]
    Layout {
        /// 0-based layout index.
        index: u32,
    },

    /// Quit the compositor cleanly
    Quit,

    /// Reload `~/.config/margo/config.conf`
    Reload,

    /// Stream state updates from margo (runs until Ctrl-C)
    #[command(
        long_about = "Stream a fresh status block every time the compositor publishes a \
                      `frame` event on the targeted output. Useful for watching focus / \
                      tag / layout changes live, or for piping into `awk`/`jq` in shell \
                      scripts that react to compositor state."
    )]
    Watch,

    /// Print current status (one shot)
    #[command(
        long_about = "Print the focused output's current state once and exit. \
                      Includes: active flag, layout symbol, focused window's title / appid / \
                      geom, and per-tag occupancy / focus."
    )]
    Status,

    /// List every dispatch action margo accepts (for binds and `mctl dispatch`)
    #[command(
        long_about = "Print every dispatch action margo accepts, grouped by purpose, with \
                      argument-shape hints. Use `--verbose` for inline detail / examples, \
                      `--group <name>` to filter to a single section, `--names` for a flat \
                      newline-separated list (drives shell completion).\n\
                      \n\
                      EXAMPLES:\n  \
                        mctl actions                       # full grouped list\n  \
                        mctl actions --verbose             # with detail / examples\n  \
                        mctl actions --group Scratchpad    # one section\n  \
                        mctl actions --names               # newline list, completion-friendly"
    )]
    Actions {
        /// Print the optional `detail` block under each action.
        #[arg(short, long)]
        verbose: bool,
        /// Filter to a single group (Tag, Focus, Layout, Scroller, Window, Scratchpad, Overview, System).
        #[arg(short, long)]
        group: Option<String>,
        /// Flat newline list of every accepted spelling (canonical + aliases).
        #[arg(long, conflicts_with_all = ["verbose", "group"])]
        names: bool,
    },

    /// Generate a shell-completion script (bash / zsh / fish / elvish / powershell)
    #[command(
        long_about = "Emit a shell-completion script to stdout for the requested shell.\n\
                      \n\
                      INSTALL:\n  \
                        bash:  mctl completions bash > ~/.local/share/bash-completion/completions/mctl\n  \
                        zsh:   mctl completions zsh  > ~/.local/share/zsh/site-functions/_mctl\n  \
                        fish:  mctl completions fish > ~/.config/fish/completions/mctl.fish\n  \
                      \n\
                      Hand-curated completion scripts shipped under `contrib/completions/` in \
                      the source tree complete `mctl dispatch <NAME>` with the action list \
                      from `mctl actions --names` — clap-generated completions only cover the \
                      subcommand layer, so prefer the contrib scripts where possible."
    )]
    Completions {
        /// Shell to generate for.
        #[arg(value_enum)]
        shell: Shell,
    },
}

// ── IPC state machine ─────────────────────────────────────────────────────────

#[derive(Debug, Default)]
struct OutputInfo {
    name: String,
    wl_output: Option<wl_output::WlOutput>,
    ipc_output: Option<ZdwlIpcOutputV2>,
    active: bool,
    tags: [TagInfo; 9],
    layout_idx: u32,
    layout_symbol: String,
    title: String,
    appid: String,
    fullscreen: bool,
    floating: bool,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

#[derive(Debug, Default, Clone)]
struct TagInfo {
    state: u32,
    clients: u32,
    focused: bool,
}

#[derive(Default)]
struct IpcState {
    manager: Option<ZdwlIpcManagerV2>,
    outputs: Vec<OutputInfo>,
    layouts: Vec<String>,
    tag_count: u32,
    ready: bool,
}

// ── Dispatch impls ────────────────────────────────────────────────────────────

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for IpcState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global { name, interface, version } = event {
            match interface.as_str() {
                "zdwl_ipc_manager_v2" => {
                    let mgr: ZdwlIpcManagerV2 =
                        registry.bind(name, version.min(2), qh, ());
                    state.manager = Some(mgr);
                }
                "wl_output" => {
                    let wl_out: wl_output::WlOutput =
                        registry.bind(name, version.min(3), qh, name);
                    let idx = state.outputs.len();
                    state.outputs.push(OutputInfo {
                        wl_output: Some(wl_out.clone()),
                        ..Default::default()
                    });
                    let _ = (idx, wl_out);
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<wl_output::WlOutput, u32> for IpcState {
    fn event(
        state: &mut Self,
        proxy: &wl_output::WlOutput,
        event: wl_output::Event,
        _name: &u32,
        _: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Name { name } = event {
            if let Some(o) = state.outputs.iter_mut().find(|o| o.wl_output.as_ref() == Some(proxy)) {
                o.name = name;
            }
        }
    }
}

impl Dispatch<ZdwlIpcManagerV2, ()> for IpcState {
    fn event(
        state: &mut Self,
        _: &ZdwlIpcManagerV2,
        event: zdwl_ipc_manager_v2::Event,
        _: &(),
        _: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            zdwl_ipc_manager_v2::Event::Tags { amount } => state.tag_count = amount,
            zdwl_ipc_manager_v2::Event::Layout { name } => state.layouts.push(name),
            _ => {}
        }
    }
}

impl Dispatch<ZdwlIpcOutputV2, usize> for IpcState {
    fn event(
        state: &mut Self,
        _proxy: &ZdwlIpcOutputV2,
        event: zdwl_ipc_output_v2::Event,
        idx: &usize,
        _: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        use zdwl_ipc_output_v2::Event;
        let out = match state.outputs.get_mut(*idx) {
            Some(o) => o,
            None => return,
        };
        match event {
            Event::Active { active } => out.active = active != 0,
            Event::Tag { tag, state: tag_state, clients, focused } => {
                if let Some(t) = out.tags.get_mut(tag as usize) {
                    t.state = match tag_state {
                        wayland_client::WEnum::Value(v) => v as u32,
                        wayland_client::WEnum::Unknown(v) => v,
                    };
                    t.clients = clients;
                    t.focused = focused != 0;
                }
            }
            Event::Layout { layout } => out.layout_idx = layout,
            Event::LayoutSymbol { layout } => out.layout_symbol = layout,
            Event::Title { title } => out.title = title,
            Event::Appid { appid } => out.appid = appid,
            Event::Fullscreen { is_fullscreen } => out.fullscreen = is_fullscreen != 0,
            Event::Floating { is_floating } => out.floating = is_floating != 0,
            Event::X { x } => out.x = x,
            Event::Y { y } => out.y = y,
            Event::Width { width } => out.width = width,
            Event::Height { height } => out.height = height,
            Event::Frame => {
                state.ready = true;
            }
            _ => {}
        }
    }
}

// ── Main ──────────────────────────────────────────────────────────────────────

fn main() -> Result<()> {
    let args = Args::parse();

    // Two subcommands don't need (or want) a Wayland connection —
    // they're documentation / scripting helpers that should work
    // outside a margo session, e.g. when the user is generating
    // completions during a package install.
    match &args.command {
        Command::Actions { verbose, group, names } => {
            return cmd_actions(*verbose, group.as_deref(), *names);
        }
        Command::Completions { shell } => {
            return cmd_completions(*shell);
        }
        _ => {}
    }

    let conn = Connection::connect_to_env()
        .map_err(|e| anyhow::anyhow!("cannot connect to Wayland display: {e}"))?;

    let (globals, mut eq): (_, EventQueue<IpcState>) = registry_queue_init(&conn)?;
    let qh = eq.handle();

    let mut state = IpcState::default();

    // Bind manager + outputs
    globals
        .contents()
        .with_list(|list| {
            for global in list {
                match global.interface.as_str() {
                    "zdwl_ipc_manager_v2" => {
                        let mgr: ZdwlIpcManagerV2 =
                            globals.registry().bind(global.name, global.version.min(2), &qh, ());
                        state.manager = Some(mgr);
                    }
                    "wl_output" => {
                        let wl_out: wl_output::WlOutput = globals
                            .registry()
                            .bind(global.name, global.version.min(3), &qh, global.name);
                        state.outputs.push(OutputInfo {
                            wl_output: Some(wl_out),
                            ..Default::default()
                        });
                    }
                    _ => {}
                }
            }
        });

    if state.manager.is_none() {
        bail!("compositor does not support zdwl_ipc_manager_v2 — is margo running?");
    }

    // Roundtrip to receive tags + layout announcements
    eq.roundtrip(&mut state)?;

    let mgr = state.manager.as_ref().unwrap().clone();

    // Bind ipc_output for each wl_output
    for (idx, out) in state.outputs.iter_mut().enumerate() {
        if let Some(wl_out) = &out.wl_output {
            let ipc_out = mgr.get_output(wl_out, &qh, idx);
            out.ipc_output = Some(ipc_out);
        }
    }

    // Wait for frame events (initial state flush)
    eq.roundtrip(&mut state)?;
    eq.roundtrip(&mut state)?;

    // Select target output
    let target_idx = select_output(&state, args.output.as_deref())?;

    let ipc_out = state.outputs[target_idx]
        .ipc_output
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("no ipc output for target"))?
        .clone();

    // Execute command
    match args.command {
        Command::Dispatch { name, args: cmd_args } => {
            let a: Vec<&str> = cmd_args.iter().map(String::as_str).collect();
            let get = |i: usize| a.get(i).copied().unwrap_or("").to_string();
            ipc_out.dispatch(name, get(0), get(1), get(2), get(3), get(4));
            eq.roundtrip(&mut state)?;
        }
        Command::Tags { mask, toggle } => {
            ipc_out.set_tags(mask, toggle);
            eq.roundtrip(&mut state)?;
        }
        Command::ClientTags { and_mask, xor_mask } => {
            ipc_out.set_client_tags(and_mask, xor_mask);
            eq.roundtrip(&mut state)?;
        }
        Command::Layout { index } => {
            ipc_out.set_layout(index);
            eq.roundtrip(&mut state)?;
        }
        Command::Quit => {
            ipc_out.quit();
            eq.roundtrip(&mut state)?;
        }
        Command::Reload => {
            ipc_out.dispatch(
                "reload_config".to_string(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
            );
            eq.roundtrip(&mut state)?;
        }
        Command::Status => {
            print_status(&state, target_idx);
        }
        Command::Watch => {
            println!("Watching output '{}' (Ctrl-C to stop)…", state.outputs[target_idx].name);
            loop {
                eq.blocking_dispatch(&mut state)?;
                if state.ready {
                    state.ready = false;
                    print_status(&state, target_idx);
                }
            }
        }
        Command::Actions { .. } | Command::Completions { .. } => {
            // Both branches return early at the top of `main`; this
            // arm only exists to keep the match exhaustive.
            unreachable!();
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
        for n in margo_ipc::actions::all_names() {
            writeln!(out, "{n}")?;
        }
        return Ok(());
    }

    // Validate the optional group filter case-insensitively against
    // the labels declared on `Group::label`.
    let filter_label = group_filter.map(|s| s.to_lowercase());
    let mut matched_any = false;

    let groups = [
        Group::Tag,
        Group::Focus,
        Group::Layout,
        Group::Scroller,
        Group::Window,
        Group::Scratchpad,
        Group::Overview,
        Group::System,
    ];

    for g in groups {
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
        writeln!(out, "── {} ─────────────────────────────────────", g.label())?;
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

    if !matched_any {
        if let Some(filter) = group_filter {
            bail!(
                "no group matches '{filter}'. Available: \
                 Tag, Focus, Layout, Scroller, Window, Scratchpad, Overview, System"
            );
        }
    }
    Ok(())
}

fn cmd_completions(shell: Shell) -> Result<()> {
    // clap_complete only knows about the subcommand layer of mctl —
    // it can't enumerate dispatch action names. The hand-written
    // scripts in `contrib/completions/` extend the generated output
    // with the action list from `mctl actions --names`. For an
    // ad-hoc one-off (e.g. installing into a fresh shell), this
    // generator is "good enough"; for distributions, prefer the
    // contrib scripts.
    let mut cmd = Args::command();
    let bin_name = "mctl";
    clap_complete::generate(shell, &mut cmd, bin_name, &mut std::io::stdout());
    Ok(())
}

fn select_output(state: &IpcState, name: Option<&str>) -> Result<usize> {
    if state.outputs.is_empty() {
        bail!("no outputs found");
    }
    match name {
        Some(n) => state
            .outputs
            .iter()
            .position(|o| o.name == n)
            .ok_or_else(|| anyhow::anyhow!("output '{n}' not found")),
        None => {
            // Prefer the active (focused) output
            state
                .outputs
                .iter()
                .position(|o| o.active)
                .or(Some(0))
                .ok_or_else(|| unreachable!())
        }
    }
}

fn print_status(state: &IpcState, idx: usize) {
    let out = &state.outputs[idx];
    println!(
        "output={} active={} layout={} title={:?} appid={:?} fullscreen={} floating={} x={} y={} width={} height={}",
        out.name,
        out.active,
        out.layout_symbol,
        out.title,
        out.appid,
        out.fullscreen,
        out.floating,
        out.x,
        out.y,
        out.width,
        out.height,
    );
    for (i, tag) in out.tags.iter().enumerate().take(state.tag_count as usize) {
        println!(
            "  tag[{}] state={} clients={} focused={}",
            i + 1,
            match tag.state {
                0 => "none",
                1 => "active",
                2 => "urgent",
                _ => "?",
            },
            tag.clients,
            tag.focused,
        );
    }
    println!("layouts: {}", state.layouts.join(", "));
}
