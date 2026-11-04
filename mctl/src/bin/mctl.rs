//! mctl — margo compositor control tool (replaces mmsg)
//!
//! Talks to margo over its Unix-socket IPC (get / watch / dispatch).
//! Connects to the compositor via the standard WAYLAND_DISPLAY socket.

use anyhow::{Context, Result, bail};
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::Shell;

use mctl::actions::{ACTIONS, Group};

// ── CLI ───────────────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(
    name = "mctl",
    version,
    about = "margo compositor control",
    long_about = "Query and control the margo Wayland compositor over its Unix-socket IPC.\n\
                  \n\
                  EXAMPLES:\n  \
                    mctl doctor                         # one-shot health check (config, socket, GPU, services)\n  \
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
        display_order = 20,
        long_about = "Dispatch a compositor command by name.\n\
                      \n\
                      The action <NAME> is the same string used in `bind = MODS,KEY,<NAME>,<args>` \
                      lines in `config.conf`. Up to 5 trailing args fill the dispatch slots:\n\
                      \n  \
                        ARGS[0] → slot 1 → arg.i   (i32, parsed; empty / non-numeric ⇒ 0)\n  \
                        ARGS[1] → slot 2 → arg.i2  (i32, parsed; rarely used)\n  \
                        ARGS[2] → slot 3 → arg.f   (f32, parsed; empty / non-numeric ⇒ 0.0)\n  \
                        ARGS[3] → slot 4 → arg.v   (string; primary string payload)\n  \
                        ARGS[4] → slot 5 → arg.v2  (string; secondary)\n\
                      \n  \
                      String-taking actions (`spawn`, `theme`, `run_script`, scratchpad commands) \
                      read from slot 4, so the string goes in position 3 (with slots 1-3 left empty). \
                      See `margo_config::Arg` doc comment for the full wire-↔-field table.\n\
                      \n\
                      EXAMPLES:\n  \
                        mctl dispatch killclient                       # close focused window\n  \
                        mctl dispatch togglefullscreen\n  \
                        mctl dispatch focusdir right                   # focus next window to the right\n  \
                        mctl dispatch view 128                         # arg.i = 128 → tag 8\n  \
                        mctl dispatch tagview 4                        # arg.i = 4 → tag 3 + follow\n  \
                        mctl dispatch setlayout 2                      # arg.i = 2 (layout index)\n  \
                        mctl dispatch movewin 40 0                     # arg.i = 40, arg.i2 = 0\n  \
                        mctl dispatch spawn '' '' '' 'firefox …'       # string in slot 4 (arg.v)\n  \
                        mctl dispatch theme '' '' '' default           # preset name in slot 4\n  \
                      \n\
                      Run `mctl actions --verbose` for per-action arg shapes."
    )]
    Dispatch {
        /// Dispatch action name (e.g. `view`, `togglefloating`, `setlayout`, `killclient`).
        /// `mctl actions` prints every accepted name.
        name: String,
        /// Up to 5 positional arguments → dispatch slots 1-5 in order.
        /// See the long help (`mctl dispatch --help`) for the slot ↔
        /// field mapping; the common pitfall is putting a string in
        /// slot 1 (numeric) instead of slot 4 (string).
        #[arg(trailing_var_arg = true)]
        args: Vec<String>,
    },

    /// File-logging controls (~/.local/state/margo/logs/margo-*.log).
    #[command(long_about = "Control margo's file logging. Logs live flat in \
                      ~/.local/state/margo/logs as margo-YYYYMMDD-HHMMSS.log \
                      (last `log_keep_sessions` kept; a margo-latest.log \
                      symlink points at the current one).\n\
                      \n\
                      `level`/`enable`/`disable` retune the *running* \
                      compositor live (no restart); they map to the \
                      `loglevel` / `logenabled` dispatch actions. `path` and \
                      `open` are local conveniences.\n\
                      \n\
                      EXAMPLES:\n  \
                        mctl log level debug    # go deep, live\n  \
                        mctl log level info     # back to normal\n  \
                        mctl log path           # print the log dir + current file\n  \
                        mctl log open           # open the log dir")]
    Log {
        #[command(subcommand)]
        action: LogCmd,
    },

    /// List / enable / disable plugins (~/.config/margo/plugins/<name>/).
    #[command(long_about = "Inspect and toggle plugins discovered under \
                      ~/.config/margo/plugins/<name>/plugin.toml. `list` shows each \
                      plugin's name / version / enabled flag (on-disk state). \
                      `enable` / `disable` flip the manifest's `enabled` key; the \
                      change takes effect on the next margo start (plugins load once \
                      at startup, so there is no live toggle).\n\
                      \n\
                      EXAMPLES:\n  \
                        mctl plugin list\n  \
                        mctl plugin disable my-plugin\n  \
                        mctl plugin enable my-plugin")]
    Plugin {
        #[command(subcommand)]
        action: PluginCmd,
    },

    /// One-shot Rhai script execution against the live compositor (W3.2).
    #[command(
        display_order = 22,
        long_about = "Evaluate a Rhai script once against the live margo \
                      session. Same binding surface + sandbox as \
                      ~/.config/margo/init.rhai; works whether or not \
                      init.rhai exists. Hook registrations inside the \
                      script (on_focus_change(...) etc.) persist after \
                      the run.\n\
                      \n\
                      EXAMPLES:\n  \
                        mctl run /tmp/test.rhai      # ad-hoc test\n  \
                        mctl run plugin.rhai         # register a hook live"
    )]
    Run {
        /// Path to the Rhai script file. Resolved relative to the
        /// caller's CWD (mctl client side); the absolute path is
        /// what gets sent to the compositor.
        file: std::path::PathBuf,
    },

    /// Translate a Hyprland or Sway/i3 config to margo (W4.4).
    #[command(
        display_order = 23,
        long_about = "Translate a Hyprland or Sway/i3 config file to a margo \
                      `config.conf`. Reads <FILE>, emits the translated config \
                      to stdout (or `-o OUT`). Only keybinds + spawn lines are \
                      auto-translated; window rules / animations / monitor \
                      topology need manual re-authoring (`config.example.conf` \
                      walks every section).\n\
                      \n\
                      Format is auto-detected from the file path or first 200 \
                      lines (`bind = ...` → hyprland, `bindsym ...` → sway). \
                      Override with `--from hyprland|sway`.\n\
                      \n\
                      Niri's KDL config is intentionally out-of-scope: niri's \
                      workspaces+scrolling model doesn't map cleanly onto \
                      margo's tag-based one — auto-translating it would \
                      invent semantics niri users wouldn't expect.\n\
                      \n\
                      EXAMPLES:\n  \
                        mctl migrate ~/.config/hypr/hyprland.conf > ~/.config/margo/config.conf\n  \
                        mctl migrate --from sway ~/.config/sway/config -o /tmp/margo.conf\n  \
                        mctl check-config /tmp/margo.conf  # validate the result"
    )]
    Migrate {
        /// Source config file.
        file: std::path::PathBuf,
        /// Force a specific source format (otherwise auto-detected).
        #[arg(long, value_name = "FORMAT")]
        from: Option<String>,
        /// Write output to a file instead of stdout.
        #[arg(short, long, value_name = "PATH")]
        output: Option<std::path::PathBuf>,
    },

    /// Set the active tagset on an output (raw tag bitmask)
    #[command(
        display_order = 21,
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
        display_order = 23,
        long_about = "Mutate the focused client's tag bitmask.\n\
                      \n\
                      Applies `(tags & AND_MASK) ^ XOR_MASK`. Almost no one calls this \
                      directly — `mctl dispatch tag <MASK>` (replace) and \
                      `mctl dispatch toggletag <MASK>` (toggle one bit) cover the user-side \
                      cases. This raw form exists so external clients (status bars) can \
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
        display_order = 22,
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
    #[command(display_order = 31)]
    Quit,

    /// Reload `~/.config/margo/config.conf`
    ///
    /// By default runs `check-config` first and refuses to reload when
    /// the file has errors; margo checks again and keeps the running
    /// config too. `--force` applies it anyway.
    #[command(display_order = 30)]
    Reload {
        /// Apply the config even when it has errors: skip mctl's pre-check
        /// and ask margo to load what it can. margo still refuses a file
        /// it can't read, keeps the error banner up, and doesn't save the
        /// config as the last-good generation (the boot fallback).
        #[arg(long)]
        force: bool,
    },

    /// Live-swap the visual theme preset
    ///
    /// Built-ins: `default`, `minimal`, `gaudy`. The first preset
    /// switch captures a baseline of the on-disk theme values;
    /// `theme default` restores them. `mctl reload` invalidates
    /// the baseline so reload + `default` lands the freshly-parsed
    /// values.
    #[command(display_order = 32)]
    Theme {
        /// Preset name: `default`, `minimal`, or `gaudy`.
        preset: String,
    },

    /// Save per-monitor tag/layout state to session.json
    ///
    /// Writes to `$XDG_STATE_HOME/margo/session.json` (defaults to
    /// `~/.local/state/margo/session.json`). Captures every
    /// monitor's seltags, tagset, per-tag layout/mfact/nmaster, and
    /// currently-parked scratchpad entries. Open windows are not
    /// captured — clients are bound to processes, the spawn line
    /// lives in user-space.
    #[command(name = "session-save", display_order = 33, alias = "session_save")]
    SessionSave,

    /// Restore per-monitor tag/layout state from session.json
    ///
    /// Matches monitors by output name; entries for absent
    /// monitors get logged + skipped. Scratchpad presence is
    /// re-flagged on live clients matching by `app_id`.
    #[command(name = "session-load", display_order = 34, alias = "session_load")]
    SessionLoad,

    /// Raw IPC query — print one JSON reply and exit.
    #[command(
        display_order = 2,
        long_about = "Send a `get <topic> [args…]` request over margo's IPC socket and \
                      pretty-print the single JSON reply.\n\n\
                      TOPICS: state | clients | client <id> | monitors | monitor <name> | \
                      tags <monitor> | focused | layouts | keyboard-layout | twilight | \
                      config-errors | layers\n\n\
                      EXAMPLES:\n  \
                        mctl get state\n  \
                        mctl get clients\n  \
                        mctl get tags DP-3"
    )]
    Get {
        /// Topic and any args (e.g. `state`, `client 3`, `tags DP-3`).
        #[arg(required = true, num_args = 1..)]
        topic: Vec<String>,
    },

    /// Stream state updates from margo (runs until Ctrl-C)
    #[command(
        display_order = 2,
        long_about = "Subscribe to a `watch <topic>` stream over margo's IPC socket and \
                      print a fresh JSON frame on every change. With no topic this watches \
                      `state` (the full snapshot) and renders the human status block; with \
                      a topic it prints raw JSON frames.\n\n\
                      EXAMPLES:\n  \
                        mctl watch                 # live status (state)\n  \
                        mctl watch tags DP-3       # raw tag frames for one monitor\n  \
                        mctl watch focused"
    )]
    Watch {
        /// Topic to watch (omit for `state` + human status rendering).
        #[arg(num_args = 0..)]
        topic: Vec<String>,
    },

    /// Print current status (one shot)
    #[command(
        display_order = 1,
        long_about = "Print the focused output's current state once and exit.\n\
                      \n\
                      Default output is the human-readable `output= … tag[N] …` block.\n\
                      `--json` emits a machine-readable JSON document with every output, \
                      every tag, the announced layout list and the per-output focused-client \
                      details — designed for status-bar widgets and `jq` pipelines that don't \
                      want to scrape the text format.\n\
                      \n\
                      EXAMPLES:\n  \
                        mctl status                    # human-readable\n  \
                        mctl status --json             # full state as JSON\n  \
                        mctl status --json | jq '.outputs[] | select(.active) | .focused.appid'"
    )]
    Status {
        /// Emit JSON instead of the default `output=… tag[N]=…` text
        /// format. Schema is stable: `{ outputs: [{ name, active,
        /// layout, layout_idx, focused: { appid, title, fullscreen,
        /// floating, x, y, width, height }, tags: [{ index, state,
        /// clients, focused }] }], layouts: [..] }`.
        #[arg(long)]
        json: bool,
    },

    /// List every dispatch action margo accepts (for binds and `mctl dispatch`)
    #[command(
        display_order = 40,
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

    /// Twilight (built-in blue-light filter) control.
    ///
    /// Subcommands:
    ///
    ///   status              # current temp / gamma / phase / source
    ///   preview <K> [pct]   # pin a temperature until `reset`
    ///   test [seconds]      # animate day→night across `seconds` (default 5)
    ///   set <field>=<value> # live tweak (day_temp, night_temp, day_gamma, …)
    ///   reset               # clear preview/test, resume schedule
    ///
    /// `mctl twilight status --json` prints the raw state snapshot
    /// `twilight` object for scripting consumption.
    #[command(display_order = 37, alias = "tl")]
    Twilight {
        #[command(subcommand)]
        action: TwilightCmd,
    },

    /// Config generation history — list, diff, or roll back
    /// `~/.config/margo/config.conf`
    ///
    /// A copy of config.conf is saved every time it successfully takes
    /// effect (compositor boot, or a successful `mctl reload`). `list`
    /// shows the saved history, `diff` compares one against the live
    /// file, `rollback` restores one and reloads.
    #[command(display_order = 43)]
    Config {
        #[command(subcommand)]
        action: ConfigCmd,
    },

    /// Show the running compositor's config diagnostics (niri-style)
    ///
    /// Hyprland's `hyprctl configerrors` analogue: what margo's config
    /// check found at startup, then after each reload. Empty when the
    /// config is clean. Reads from the live compositor (via
    /// the same state snapshot IPC `mctl status` uses), so it reflects
    /// what the compositor actually applied — not what
    /// `check-config` thinks about the file on disk right now.
    #[command(display_order = 38)]
    ConfigErrors,

    /// Validate the user's config (niri-style diagnostics)
    ///
    /// Catches unknown keys, regex errors, duplicate binds, missing
    /// source files, lone-mango leftovers. `--help` prints the full
    /// rule list + exit-code semantics.
    #[command(
        long_about = "Sanity-check a margo config file without launching the compositor.\n\
                      \n\
                      Catches:\n  \
                        * Unknown top-level keys (not in the documented schema).\n  \
                        * Unknown windowrule / layerrule / monitorrule / tagrule fields.\n  \
                        * Invalid regex patterns in `appid:`, `title:`, `exclude_*:`, `layer_name:` slots.\n  \
                        * Duplicate `bind = MODS,KEY,…` lines in the same keymode (the first one wins).\n  \
                        * Unresolvable `source = …` / `include = …` includes.\n  \
                        * Lone-mango option carry-overs that margo doesn't yet implement (warning).\n  \
                      \n\
                      Exits 0 on a clean parse, 1 on errors (or on any warning with --strict), \
                      2 if the file itself can't be read.\n\
                      \n\
                      EXAMPLES:\n  \
                        mctl check-config\n  \
                        mctl check-config --config ~/dotfiles/margo/config.conf\n  \
                        mctl check-config --strict      # CI: warnings fail too\n  \
                        mctl check-config 2>&1 | grep ERROR"
    )]
    #[command(display_order = 42)]
    CheckConfig {
        /// Path to inspect. Defaults to `~/.config/margo/config.conf`.
        #[arg(long)]
        config: Option<std::path::PathBuf>,
        /// Treat warnings as errors: exit 1 when there is any diagnostic
        /// (for CI and pre-commit checks of a config).
        #[arg(long)]
        strict: bool,
    },

    /// Dry-run windowrules against an app_id / title pair
    ///
    /// Pure config introspection — doesn't query the running
    /// compositor. Useful for sanity-checking `windowrule` patterns
    /// without launching the app.
    #[command(
        long_about = "Walk `~/.config/margo/config.conf` (or the file passed via \
                      `--config`) and print the windowrules that match a given \
                      app_id / title.\n\
                      \n\
                      EXAMPLES:\n  \
                        mctl rules --appid Spotify\n  \
                        mctl rules --appid Kenp --title 'Helium'\n  \
                        mctl rules --config ~/work/test.conf --appid clipse\n  \
                      \n\
                      Output groups matching rules first (with the fields each rule \
                      sets), then non-matching rules with the reason they were \
                      rejected (positive pattern miss / exclude pattern hit).\n\
                      \n\
                      Useful when a rule isn't firing — pinpoints the regex problem \
                      without needing to launch the app and watch journalctl."
    )]
    #[command(display_order = 41)]
    Rules {
        /// Path to the config to inspect. Defaults to
        /// `$XDG_CONFIG_HOME/margo/config.conf`, falling back to
        /// `~/.config/margo/config.conf`.
        #[arg(long)]
        config: Option<std::path::PathBuf>,
        /// app_id pattern to test rules against.
        #[arg(long, default_value = "")]
        appid: String,
        /// Window title to test against. Empty = match-anything.
        #[arg(long, default_value = "")]
        title: String,
        /// Show non-matching rules too, with the reason they didn't fire.
        #[arg(short, long)]
        verbose: bool,
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
                      The same output is checked in under `contrib/completions/` (regenerated \
                      by `just completions`) and installed system-wide by the package. These are \
                      static completions — `mctl dispatch <NAME>` is free text; list the actions \
                      with `mctl actions --names`."
    )]
    #[command(display_order = 43)]
    Completions {
        /// Shell to generate for.
        #[arg(value_enum)]
        shell: Shell,
    },

    /// Health check — config, socket, version sync, GPU, services in one pass.
    #[command(
        display_order = 2,
        long_about = "Run a one-shot health check over a margo install and print a \
                      ✓ / ⚠ / ✗ line per check, then a summary.\n\
                      \n\
                      Unifies the scattered diagnostics — config validation \
                      (`check-config`), the running compositor's rejected lines \
                      (`config-errors`), socket reachability, margo↔mctl version \
                      sync (catches 'installed a new margo but haven't re-logged \
                      in'), GPU render nodes, matugen, and the mshell / portal \
                      D-Bus services.\n\
                      \n\
                      Exits non-zero if any check is an outright error, so it \
                      works as a scriptable smoke test. Optional pieces (matugen, \
                      busctl) degrade to warnings, never failures."
    )]
    Doctor,

    /// List every open window with tag, monitor, app_id, title.
    ///
    /// Reads `the margo IPC socket` (margo refreshes
    /// it on every arrange/focus/output-change event). Same data
    /// you'd get from triggering `pkill -USR1 margo` and grepping
    /// the journal, but live and parseable.
    #[command(
        alias = "client",
        display_order = 3,
        long_about = "List every open window the compositor knows about — tag, \
                      monitor, app_id, title, geometry, focus, floating/fullscreen \
                      state. Reads the compositor state snapshot over the IPC socket which margo \
                      refreshes on every relevant event.\n\
                      \n\
                      EXAMPLES:\n  \
                        mctl clients                        # all windows, table\n  \
                        mctl clients --json                 # full JSON\n  \
                        mctl clients --tag 2                # only tag 2 windows\n  \
                        mctl clients --monitor DP-3         # only DP-3 windows\n  \
                        mctl clients --app-id helium        # match app_id substring\n  \
                        mctl clients --json | jq '.[] | .app_id'\n  \
                      \n\
                      Output columns: TAG  MON  APP-ID  TITLE  (+ markers for \
                      focused/floating/fullscreen). Pass `--wide` for the \
                      full geometry column."
    )]
    Clients {
        /// JSON dump of the full client list.
        #[arg(long)]
        json: bool,
        /// Filter by tag number (1-based; e.g. `--tag 2`).
        #[arg(long)]
        tag: Option<u32>,
        /// Filter by monitor connector name (e.g. `--monitor DP-3`).
        #[arg(long)]
        monitor: Option<String>,
        /// Filter by app_id substring (case-insensitive).
        #[arg(long)]
        app_id: Option<String>,
        /// Include the geometry column (`x,y wxh`).
        #[arg(long)]
        wide: bool,
    },

    /// List every connected output with mode, position, scale, layout.
    #[command(
        alias = "monitors",
        display_order = 4,
        long_about = "List every connected output: connector name, position in \
                      the global compositor coordinate space, mode, scale, \
                      transform, current layout, active-tag mask. Reads the \
                      same state file as `mctl clients`.\n\
                      \n\
                      EXAMPLES:\n  \
                        mctl outputs\n  \
                        mctl outputs --json\n  \
                        mctl outputs --json | jq '.[].name'"
    )]
    Outputs {
        #[arg(long)]
        json: bool,
    },

    /// Per-output frame counters: renders, empties, empty ratio, queue errors.
    #[command(
        display_order = 5,
        long_about = "Per-output render performance, read live from the \
                      compositor (the `perf` IPC topic — kept out of the hot \
                      state stream). The table shows the last-60s rolling window: \
                      windowed frame rate (1s + 60s), the recent empty (no-damage) \
                      ratio, the render-latency tail (p50/p95/p99 ms) — where \
                      jank hides that a mean would average away — the direct-\
                      scanout share of presented frames (SCAN%: fullscreen \
                      video on the primary plane, zero composite cost), and \
                      the composited damage area (DMG p50/p95, megapixels per \
                      frame; large values mean full-screen redraws). \
                      A high empty ratio is healthy (the compositor skipping \
                      redundant work); rising queue errors point at a flaky \
                      modeset/plane. `--json` also carries the lifetime totals \
                      (renders / queued / empties) and every window (fps_1s / \
                      fps_10s / fps_60s).\n\
                      \n\
                      EXAMPLES:\n  \
                        mctl perf                       # one-shot table\n  \
                        mctl perf --watch               # live, refreshes every 1s\n  \
                        mctl perf --json | jq '.outputs[] | {name, fps_60s, render_us_p99}'"
    )]
    Perf {
        #[arg(long)]
        json: bool,
        /// Refresh the table once a second until interrupted (ignored with --json).
        #[arg(long)]
        watch: bool,
    },

    /// Print the focused window's app_id + title (terse, scriptable).
    #[command(
        alias = "active",
        display_order = 6,
        long_about = "Print the focused window's app_id + title in a single \
                      line — designed for status-bar scripts that just need \
                      `who has focus right now`. `--json` for the full \
                      client object.\n\
                      \n\
                      EXAMPLES:\n  \
                        mctl focused                    # `app_id · title`\n  \
                        mctl focused --json | jq .title"
    )]
    Focused {
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand, Debug)]
enum LogCmd {
    /// Set the file-log level live: error | warn | info | debug | trace.
    Level {
        /// One of error, warn, info, debug, trace.
        level: String,
    },
    /// Turn file logging on, live.
    Enable,
    /// Turn file logging off, live.
    Disable,
    /// Print the log directory and the current-session file path.
    Path,
    /// Open the log directory in the default file manager.
    Open,
}

#[derive(Subcommand, Debug)]
enum PluginCmd {
    /// List discovered plugins + their enabled state (on-disk).
    List,
    /// Enable a plugin by name (takes effect on the next margo start).
    Enable {
        /// Plugin directory name or manifest `name`.
        name: String,
    },
    /// Disable a plugin by name (takes effect on the next margo start).
    Disable {
        /// Plugin directory name or manifest `name`.
        name: String,
    },
}

#[derive(Subcommand, Debug)]
enum ConfigCmd {
    /// List saved config.conf generations, newest first (index 0 = most recent).
    List {
        /// Path to the live config to compare each generation against.
        /// Defaults to `~/.config/margo/config.conf`.
        #[arg(long)]
        config: Option<std::path::PathBuf>,
    },
    /// Show a unified diff between a saved generation and the live file.
    Diff {
        /// Generation index (0 = most recent). Default: 0.
        n: Option<usize>,
        /// Path to the live config. Defaults to `~/.config/margo/config.conf`.
        #[arg(long)]
        config: Option<std::path::PathBuf>,
    },
    /// Restore a saved generation to config.conf and reload.
    Rollback {
        /// Generation index (0 = most recent). Default: index 0 (the
        /// last known-good) if the live file has unsaved changes
        /// (e.g. after a failed reload), else 1 (one before the
        /// current file).
        n: Option<usize>,
        /// Skip the symlink/overwrite confirmation prompt.
        #[arg(long)]
        yes: bool,
        /// Path to the live config. Defaults to `~/.config/margo/config.conf`.
        #[arg(long)]
        config: Option<std::path::PathBuf>,
    },
}

#[derive(Subcommand, Debug)]
enum TwilightCmd {
    /// Print the current twilight state (temperature, gamma, phase,
    /// source). Reads `state snapshot` — works even when the compositor
    /// is paused on a heavy frame.
    Status {
        /// Emit the raw `state snapshot` `twilight` object as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Pin a temperature (and optional gamma %) until `reset`. The
    /// schedule is bypassed for the duration.
    Preview {
        /// Colour temperature in Kelvin (1000–25000).
        kelvin: u32,
        /// Brightness percentage (10–200). Defaults to 100.
        #[arg(default_value_t = 100)]
        gamma: u32,
    },
    /// Sweep day → night over `seconds`. Falls back to the
    /// schedule when done.
    Test {
        /// Sweep duration in seconds (1–60). Defaults to 5.
        #[arg(default_value_t = 5)]
        seconds: u32,
    },
    /// Live-tweak one config field. Persists until reload — the
    /// on-disk config isn't touched.
    #[command(long_about = "Live-tweak one twilight config field. The change \
                      persists until the next `mctl reload` — the on-disk \
                      config file is NOT modified.\n\
                      \n\
                      SUPPORTED FIELDS:\n  \
                        day_temp=<1000–25000>      Daytime colour temperature (K)\n  \
                        night_temp=<1000–25000>    Night colour temperature (K)\n  \
                        day_gamma=<10–200>         Daytime brightness (%)\n  \
                        night_gamma=<10–200>       Night brightness (%)\n  \
                        transition_s=<30–7200>     Day↔night fade duration (s)\n  \
                        enabled=<0|1>              Master on/off (alias: `twilight`)\n  \
                        mode=<geo|manual|static|schedule>  Source of truth for temp/gamma\n\
                      \n\
                      EXAMPLES:\n  \
                        mctl twilight set mode=schedule       # switch to preset schedule\n  \
                        mctl twilight set night_temp=2200     # warmer nights\n  \
                        mctl twilight set enabled=0           # disable until reload")]
    Set {
        /// `field=value`. Supported fields: `day_temp`,
        /// `night_temp`, `day_gamma`, `night_gamma`, `transition_s`,
        /// `enabled` (alias `twilight`, 0/1), `mode`
        /// (geo|manual|static|schedule). Run `mctl twilight set --help`
        /// for full details + examples.
        spec: Option<String>,
    },
    /// Clear any preview / test override, resume the schedule.
    Reset,
    /// Toggle the schedule on/off. Equivalent to
    /// `mctl twilight set enabled=0` / `enabled=1` but stateful —
    /// uses the current `state.config.twilight` flag.
    Toggle,
    /// Manage Schedule-mode presets (the TOML files under
    /// `~/.config/margo/twilight/presets/`) and the schedule
    /// table that maps `HH:MM` to preset names. Writes the
    /// on-disk files directly, then asks the compositor to
    /// reload so the change is live.
    Preset {
        #[command(subcommand)]
        action: TwilightPresetCmd,
    },
}

#[derive(Subcommand, Debug)]
enum TwilightPresetCmd {
    /// List all preset files plus the current time → preset schedule.
    List,
    /// Print one preset's Kelvin temperature and gamma percent.
    Show {
        /// Preset name (no `.toml`).
        name: String,
    },
    /// Create or update a preset file. Existing files are rewritten.
    Set {
        /// Preset name (no `.toml`). Used as the filename.
        name: String,
        /// Colour temperature in Kelvin (1000–25000).
        temp: u32,
        /// Brightness percent (10–200). Defaults to 100.
        #[arg(default_value_t = 100)]
        gamma: u32,
    },
    /// Delete a preset file. Also strips any `schedule.conf` line
    /// that referenced it so the schedule never points at a
    /// missing preset.
    Remove {
        /// Preset name to delete.
        name: String,
    },
    /// Edit the `HH:MM → preset` schedule.
    Schedule {
        #[command(subcommand)]
        action: TwilightScheduleCmd,
    },
}

#[derive(Subcommand, Debug)]
enum TwilightScheduleCmd {
    /// Add or update a schedule line.
    Set {
        /// Time of day in `HH:MM` (24-hour).
        time: String,
        /// Preset name to apply at that time. The preset file
        /// must already exist — run `mctl twilight preset set`
        /// first if you're creating a brand-new entry.
        name: String,
    },
    /// Remove the schedule line at a given `HH:MM`.
    Remove {
        /// Time of day in `HH:MM` (24-hour) to remove.
        time: String,
    },
}

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
        | Command::Plugin { .. } => {
            // These return early at the top of `main`; this arm only
            // exists to keep the match exhaustive.
            unreachable!("offline commands return early in main");
        }
    }

    Ok(())
}

/// `~/.config/margo/plugins` if it exists.
fn mctl_plugins_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(|h| std::path::PathBuf::from(h).join("margo/plugins"))
        .or_else(|| {
            std::env::var_os("HOME")
                .map(|h| std::path::PathBuf::from(h).join(".config/margo/plugins"))
        })
        .filter(|p| p.is_dir())
}

/// Read a bare `key = value` field from a plugin.toml manifest (same flat
/// grammar margo's loader uses — no nested tables, and everything after a
/// `#` is a comment, inline ones included).
fn manifest_field(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let line = line.split('#').next().unwrap_or("").trim();
        let (k, v) = line.split_once('=')?;
        (k.trim() == key).then(|| v.trim().trim_matches('"').trim_matches('\'').to_string())
    })
}

fn manifest_enabled(text: &str) -> bool {
    // Default true (matches the loader): absent / unparseable ⇒ enabled.
    manifest_field(text, "enabled")
        .map(|v| matches!(v.as_str(), "true" | "yes" | "1" | "on"))
        .unwrap_or(true)
}

fn cmd_plugin(action: &PluginCmd) -> Result<()> {
    let Some(dir) = mctl_plugins_dir() else {
        // No `~/.config/margo/plugins/` — the common case. Not an error;
        // explain the three separate plugin/scripting worlds so the user
        // isn't left thinking something is broken.
        match action {
            PluginCmd::List => {
                println!("No compositor plugins.");
                println!();
                println!(
                    "  `mctl plugin` manages PACKAGED Rhai plugins in\n  \
                     ~/.config/margo/plugins/<name>/ (plugin.toml + init.rhai).\n\n  \
                     Personal scripting lives in ~/.config/margo/init.rhai and loads\n  \
                     automatically — no packaging needed.\n  \
                     Shell (WASM) plugins are a separate system: Settings → Plugins\n  \
                     (or `mshellctl`)."
                );
            }
            PluginCmd::Enable { .. } | PluginCmd::Disable { .. } => {
                println!(
                    "No ~/.config/margo/plugins/ directory yet — nothing to enable/disable.\n\
                     Create ~/.config/margo/plugins/<name>/ with a plugin.toml + init.rhai first."
                );
            }
        }
        return Ok(());
    };
    match action {
        PluginCmd::List => {
            let mut found = false;
            let mut entries: Vec<_> = std::fs::read_dir(&dir)?.flatten().collect();
            entries.sort_by_key(|e| e.file_name());
            for entry in entries {
                let manifest = entry.path().join("plugin.toml");
                let Ok(text) = std::fs::read_to_string(&manifest) else {
                    continue;
                };
                found = true;
                let dirname = entry.file_name().to_string_lossy().to_string();
                let name = manifest_field(&text, "name").unwrap_or_else(|| dirname.clone());
                let version = manifest_field(&text, "version").unwrap_or_default();
                let mark = if manifest_enabled(&text) {
                    "●"
                } else {
                    "○"
                };
                println!("{mark}  {name:<24} {version}");
            }
            if !found {
                println!("(no plugins in {})", dir.display());
            }
        }
        PluginCmd::Enable { name } => set_plugin_enabled(&dir, name, true)?,
        PluginCmd::Disable { name } => set_plugin_enabled(&dir, name, false)?,
    }
    Ok(())
}

/// Flip the `enabled` key of the plugin matching `name` (by directory name or
/// manifest `name`). Rewrites the `enabled = ...` line, or appends it.
fn set_plugin_enabled(dir: &std::path::Path, name: &str, enabled: bool) -> Result<()> {
    for entry in std::fs::read_dir(dir)?.flatten() {
        let manifest = entry.path().join("plugin.toml");
        let Ok(text) = std::fs::read_to_string(&manifest) else {
            continue;
        };
        let dirname = entry.file_name().to_string_lossy().to_string();
        let mname = manifest_field(&text, "name").unwrap_or_else(|| dirname.clone());
        if dirname == name || mname == name {
            std::fs::write(&manifest, with_enabled_line(&text, enabled))?;
            println!(
                "{} plugin '{name}'. Takes effect on the next margo start.",
                if enabled { "Enabled" } else { "Disabled" }
            );
            return Ok(());
        }
    }
    bail!("no plugin named '{name}' in {}", dir.display());
}

/// `text` (a plugin.toml) with its `enabled = …` line set to `enabled`,
/// appended when there is none.
fn with_enabled_line(text: &str, enabled: bool) -> String {
    let val = if enabled { "true" } else { "false" };
    let mut out = String::new();
    let mut rewrote = false;
    for line in text.lines() {
        let is_enabled = line
            .split_once('=')
            .map(|(k, _)| k.trim() == "enabled")
            .unwrap_or(false);
        if is_enabled {
            out.push_str(&format!("enabled = {val}\n"));
            rewrote = true;
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    if !rewrote {
        out.push_str(&format!("enabled = {val}\n"));
    }
    out
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

fn cmd_rules(
    config_override: Option<&std::path::Path>,
    appid: &str,
    title: &str,
    verbose: bool,
) -> Result<()> {
    use margo_config::{WindowRule, parse_config};

    let cfg_path = config_override
        .map(|p| p.to_path_buf())
        .or_else(|| {
            std::env::var_os("XDG_CONFIG_HOME")
                .map(|h| std::path::PathBuf::from(h).join("margo/config.conf"))
        })
        .unwrap_or_else(|| {
            std::path::PathBuf::from(format!(
                "{}/.config/margo/config.conf",
                std::env::var("HOME").unwrap_or_else(|_| ".".to_string())
            ))
        });

    if !cfg_path.exists() {
        bail!("config file not found: {}", cfg_path.display());
    }
    let cfg = parse_config(Some(&cfg_path))
        .map_err(|e| anyhow::anyhow!("parse {}: {e}", cfg_path.display()))?;

    // No filter args provided → behave like `niri msg windows` /
    // `hyprctl rules`: just dump every defined windowrule. Test
    // mode (with --appid/--title) only when the user explicitly
    // asks.
    let no_filter = appid.is_empty() && title.is_empty();
    println!("config: {}", cfg_path.display());
    if no_filter {
        println!(
            "─── all windowrules ({}) ─────────────────────",
            cfg.window_rules.len()
        );
        if cfg.window_rules.is_empty() {
            println!("  (no `windowrule = ...` lines defined)");
        } else {
            for r in &cfg.window_rules {
                print_rule(r);
            }
        }
        if verbose {
            println!(
                "\nTip: pass `--appid X` and/or `--title Y` to see which \
                 rules WOULD apply to a hypothetical window."
            );
        }
        return Ok(());
    }

    println!("query:  appid='{}' title='{}'\n", appid, title);

    // Re-implement the matcher locally — `WindowRule` matching lives
    // in `margo`'s state module, but the rules / patterns themselves
    // live in `margo-config`, which we have here. Reusing the same
    // regex semantics (`regex::Regex` with empty-pattern → match-all,
    // unanchored otherwise) keeps the verdict in lockstep with the
    // compositor's runtime decision.
    let mut matched: Vec<&WindowRule> = Vec::new();
    let mut rejected: Vec<(&WindowRule, &'static str)> = Vec::new();

    for rule in &cfg.window_rules {
        match classify_rule(rule, appid, title) {
            Verdict::Match => matched.push(rule),
            Verdict::Reject(reason) => rejected.push((rule, reason)),
        }
    }

    println!(
        "── matching ({} rule{}) ───────────────────────",
        matched.len(),
        if matched.len() == 1 { "" } else { "s" },
    );
    if matched.is_empty() {
        println!("  (none)");
    } else {
        for r in &matched {
            print_rule(r);
        }
    }

    if verbose && !rejected.is_empty() {
        println!(
            "\n── rejected ({} rule{}) ───────────────────────",
            rejected.len(),
            if rejected.len() == 1 { "" } else { "s" },
        );
        for (r, reason) in rejected {
            println!("  ✗ {}", reason);
            print_rule(r);
        }
    }
    Ok(())
}

enum Verdict {
    Match,
    Reject(&'static str),
}

fn classify_rule(rule: &margo_config::WindowRule, appid: &str, title: &str) -> Verdict {
    let pattern_match = |pat: &str, value: &str| -> bool {
        if pat.is_empty() {
            return true;
        }
        if value.is_empty() {
            return false;
        }
        match regex::Regex::new(pat) {
            Ok(rx) => rx.is_match(value),
            Err(_) => {
                // Fall back to substring match if the pattern won't
                // compile — same behaviour as the compositor's
                // `matches_rule_text`.
                let trimmed = pat.trim_start_matches('^').trim_end_matches('$');
                value.contains(trimmed)
            }
        }
    };

    if !pattern_match(rule.id.as_deref().unwrap_or(""), appid) {
        return Verdict::Reject("appid pattern miss");
    }
    if !pattern_match(rule.title.as_deref().unwrap_or(""), title) {
        return Verdict::Reject("title pattern miss");
    }
    if let Some(p) = rule.exclude_id.as_deref().filter(|p| !p.is_empty())
        && pattern_match(p, appid)
    {
        return Verdict::Reject("exclude_id matched");
    }
    if let Some(p) = rule.exclude_title.as_deref().filter(|p| !p.is_empty())
        && pattern_match(p, title)
    {
        return Verdict::Reject("exclude_title matched");
    }
    Verdict::Match
}

fn print_rule(rule: &margo_config::WindowRule) {
    let id = rule.id.as_deref().unwrap_or("");
    let title = rule.title.as_deref().unwrap_or("");
    let mut bits: Vec<String> = Vec::new();
    if !id.is_empty() {
        bits.push(format!("appid={}", id));
    }
    if !title.is_empty() {
        bits.push(format!("title={}", title));
    }
    if rule.tags != 0 {
        bits.push(format!("tags=0x{:x}", rule.tags));
    }
    if rule.width > 0 || rule.height > 0 {
        bits.push(format!("size={}x{}", rule.width, rule.height));
    }
    if rule.offset_x != 0 || rule.offset_y != 0 {
        bits.push(format!("offset={}+{}", rule.offset_x, rule.offset_y));
    }
    macro_rules! flag {
        ($field:ident, $name:literal) => {
            if let Some(v) = rule.$field {
                bits.push(format!("{}={}", $name, v));
            }
        };
    }
    flag!(is_floating, "isfloating");
    flag!(is_fullscreen, "isfullscreen");
    flag!(is_named_scratchpad, "isnamedscratchpad");
    flag!(no_border, "isnoborder");
    flag!(no_animation, "isnoanimation");
    flag!(no_blur, "noblur");
    flag!(no_focus, "nofocus");
    flag!(allow_csd, "allow_csd");
    flag!(block_out_from_screencast, "block_out_from_screencast");
    if let Some(v) = rule.scroller_proportion {
        bits.push(format!("scroller_proportion={}", v));
    }
    if !bits.is_empty() {
        println!("    {}", bits.join("  "));
    }
}

fn cmd_twilight_status(as_json: bool) -> Result<()> {
    let state = read_state_file().map_err(|e| anyhow::anyhow!("cannot query margo IPC: {e}"))?;
    let tw = state
        .get("twilight")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    if tw.is_null() {
        eprintln!("twilight: not advertised by this compositor (older margo?)");
        std::process::exit(1);
    }
    if as_json {
        println!("{}", serde_json::to_string_pretty(&tw)?);
        return Ok(());
    }
    let enabled = tw.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false);
    let mode = tw.get("mode").and_then(|v| v.as_str()).unwrap_or("?");
    let phase = tw.get("phase").and_then(|v| v.as_str()).unwrap_or("?");
    let source = tw.get("source").and_then(|v| v.as_str()).unwrap_or("?");
    let cur_k = tw.get("current_temp_k").and_then(|v| v.as_u64());
    let cur_g = tw.get("current_gamma_pct").and_then(|v| v.as_u64());
    let day_k = tw.get("day_temp_k").and_then(|v| v.as_u64()).unwrap_or(0);
    let nig_k = tw.get("night_temp_k").and_then(|v| v.as_u64()).unwrap_or(0);
    let day_g = tw
        .get("day_gamma_pct")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let nig_g = tw
        .get("night_gamma_pct")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let colored = std::io::IsTerminal::is_terminal(&std::io::stdout());
    let (b, dim, r) = if colored {
        ("\x1b[1m", "\x1b[2m", "\x1b[0m")
    } else {
        ("", "", "")
    };
    println!(
        "{b}twilight{r}: {} ({dim}{}{r})",
        if enabled { "ON" } else { "off" },
        mode
    );
    println!("  {dim}phase{r}   = {phase}    {dim}source{r} = {source}");
    match (cur_k, cur_g) {
        (Some(k), Some(g)) => {
            println!("  {dim}current{r} = {b}{k}K{r} @ {b}{g}%{r}");
        }
        _ => {
            println!("  {dim}current{r} = (none — gamma untouched)");
        }
    }
    println!("  {dim}day{r}     = {day_k}K @ {day_g}%");
    println!("  {dim}night{r}   = {nig_k}K @ {nig_g}%");
    Ok(())
}

// ── twilight preset management ────────────────────────────────────────────────
//
// The on-disk layout (`margo/src/twilight/preset.rs`) is:
//   <dir>/
//     schedule.conf          # `HH:MM <preset-name>` lines, # for comments
//     presets/<name>.toml    # `static_temp = K`, `static_gamma = %`
//
// All preset subcommands work on these files directly — no IPC dance
// for reads, just a best-effort `reload_config` dispatch at the end of
// writes so the compositor picks up the change without `mctl reload`.

fn twilight_dir() -> std::path::PathBuf {
    // Match the compositor's `default_dir()` from
    // `margo/src/twilight/preset.rs`. If the user has overridden
    // `twilight_schedule_dir` in their config, state snapshot doesn't
    // currently expose it — they'll need `mctl reload` after manual
    // edits in that custom location (uncommon enough to defer).
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return std::path::PathBuf::from(xdg).join("margo").join("twilight");
    }
    if let Ok(home) = std::env::var("HOME") {
        return std::path::PathBuf::from(home)
            .join(".config")
            .join("margo")
            .join("twilight");
    }
    std::path::PathBuf::from(".config/margo/twilight")
}

fn cmd_twilight_preset(action: &TwilightPresetCmd) -> Result<()> {
    let dir = twilight_dir();
    match action {
        TwilightPresetCmd::List => cmd_twilight_preset_list(&dir),
        TwilightPresetCmd::Show { name } => cmd_twilight_preset_show(&dir, name),
        TwilightPresetCmd::Set { name, temp, gamma } => {
            cmd_twilight_preset_set(&dir, name, *temp, *gamma)
        }
        TwilightPresetCmd::Remove { name } => cmd_twilight_preset_remove(&dir, name),
        TwilightPresetCmd::Schedule { action } => match action {
            TwilightScheduleCmd::Set { time, name } => cmd_twilight_schedule_set(&dir, time, name),
            TwilightScheduleCmd::Remove { time } => cmd_twilight_schedule_remove(&dir, time),
        },
    }
}

fn cmd_twilight_preset_list(dir: &std::path::Path) -> Result<()> {
    let presets_dir = dir.join("presets");
    let schedule_path = dir.join("schedule.conf");
    let colored = std::io::IsTerminal::is_terminal(&std::io::stdout());
    let (b, dim, r) = if colored {
        ("\x1b[1m", "\x1b[2m", "\x1b[0m")
    } else {
        ("", "", "")
    };

    println!("{b}presets{r} ({dim}{}{r})", presets_dir.display());
    let mut names: Vec<(String, Option<(u32, u32)>)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&presets_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().and_then(|s| s.to_str()) != Some("toml") {
                continue;
            }
            let name = p
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            let parsed = read_preset_file(&p);
            names.push((name, parsed));
        }
    }
    names.sort_by(|a, b| a.0.cmp(&b.0));
    if names.is_empty() {
        println!("  {dim}(none — run `mctl twilight preset set <name> <K>` to create one){r}");
    } else {
        for (name, parsed) in &names {
            match parsed {
                Some((k, g)) => println!("  {b}{name:<16}{r}  {k}K @ {g}%"),
                None => println!("  {b}{name:<16}{r}  {dim}(unreadable){r}"),
            }
        }
    }

    println!();
    println!("{b}schedule{r} ({dim}{}{r})", schedule_path.display());
    match std::fs::read_to_string(&schedule_path) {
        Ok(text) => {
            let mut lines: Vec<(String, String)> = Vec::new();
            for line in text.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with('#') {
                    continue;
                }
                let mut parts = trimmed.split_whitespace();
                let Some(time) = parts.next() else { continue };
                let Some(name) = parts.next() else { continue };
                lines.push((time.to_string(), name.to_string()));
            }
            if lines.is_empty() {
                println!("  {dim}(empty){r}");
            } else {
                for (time, name) in &lines {
                    println!("  {b}{time}{r}  {name}");
                }
            }
        }
        Err(_) => {
            println!("  {dim}(no schedule.conf){r}");
        }
    }
    Ok(())
}

fn cmd_twilight_preset_show(dir: &std::path::Path, name: &str) -> Result<()> {
    let path = dir.join("presets").join(format!("{name}.toml"));
    if !path.exists() {
        anyhow::bail!("preset {name:?} not found at {}", path.display());
    }
    let (temp, gamma) = read_preset_file(&path)
        .ok_or_else(|| anyhow::anyhow!("preset {name:?} unparseable at {}", path.display()))?;
    println!("{name}: {temp}K @ {gamma}%");
    println!("  path = {}", path.display());
    Ok(())
}

fn cmd_twilight_preset_set(dir: &std::path::Path, name: &str, temp: u32, gamma: u32) -> Result<()> {
    validate_preset_name(name)?;
    if !(1000..=25000).contains(&temp) {
        anyhow::bail!("temp must be 1000–25000, got {temp}");
    }
    if !(10..=200).contains(&gamma) {
        anyhow::bail!("gamma must be 10–200, got {gamma}");
    }
    let presets_dir = dir.join("presets");
    std::fs::create_dir_all(&presets_dir)
        .map_err(|e| anyhow::anyhow!("cannot create {}: {e}", presets_dir.display()))?;
    let path = presets_dir.join(format!("{name}.toml"));
    let body = format!(
        "# margo twilight preset\n\
         # static_temp: Kelvin (1000–25000)\n\
         # static_gamma: brightness % (10–200)\n\
         static_temp = {temp}\n\
         static_gamma = {gamma}\n"
    );
    std::fs::write(&path, body)
        .map_err(|e| anyhow::anyhow!("cannot write {}: {e}", path.display()))?;
    println!("wrote {} ({temp}K @ {gamma}%)", path.display());
    request_reload();
    Ok(())
}

fn cmd_twilight_preset_remove(dir: &std::path::Path, name: &str) -> Result<()> {
    validate_preset_name(name)?;
    let path = dir.join("presets").join(format!("{name}.toml"));
    if !path.exists() {
        anyhow::bail!("preset {name:?} not found at {}", path.display());
    }
    std::fs::remove_file(&path)
        .map_err(|e| anyhow::anyhow!("cannot delete {}: {e}", path.display()))?;
    println!("removed {}", path.display());

    // Strip any schedule lines that referenced this preset so the
    // schedule never points at a missing file (the compositor would
    // skip them with a warn, but a clean schedule file is friendlier).
    let schedule_path = dir.join("schedule.conf");
    if let Ok(text) = std::fs::read_to_string(&schedule_path) {
        let mut kept: Vec<String> = Vec::new();
        let mut dropped = 0;
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                kept.push(line.to_string());
                continue;
            }
            let mut parts = trimmed.split_whitespace();
            let _time = parts.next();
            let ref_name = parts.next().unwrap_or("");
            if ref_name == name {
                dropped += 1;
            } else {
                kept.push(line.to_string());
            }
        }
        if dropped > 0 {
            let mut out = kept.join("\n");
            if !out.ends_with('\n') {
                out.push('\n');
            }
            std::fs::write(&schedule_path, out)
                .map_err(|e| anyhow::anyhow!("cannot rewrite {}: {e}", schedule_path.display()))?;
            println!("removed {dropped} schedule line(s) referencing {name:?}");
        }
    }
    request_reload();
    Ok(())
}

fn cmd_twilight_schedule_set(dir: &std::path::Path, time: &str, name: &str) -> Result<()> {
    let normalized = normalize_hhmm(time)?;
    validate_preset_name(name)?;
    // Guard against pointing the schedule at a preset that doesn't
    // exist yet — the compositor would just warn-and-skip but it's
    // a friendlier error if mctl catches it.
    let preset_path = dir.join("presets").join(format!("{name}.toml"));
    if !preset_path.exists() {
        anyhow::bail!(
            "preset {name:?} not found at {}\n\
             create it first: mctl twilight preset set {name} <K>",
            preset_path.display()
        );
    }
    let schedule_path = dir.join("schedule.conf");
    std::fs::create_dir_all(dir)
        .map_err(|e| anyhow::anyhow!("cannot create {}: {e}", dir.display()))?;
    let existing = std::fs::read_to_string(&schedule_path).unwrap_or_default();
    let mut kept: Vec<String> = Vec::new();
    let mut replaced = false;
    for line in existing.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            kept.push(line.to_string());
            continue;
        }
        let mut parts = trimmed.split_whitespace();
        let Some(t) = parts.next() else {
            kept.push(line.to_string());
            continue;
        };
        if normalize_hhmm(t).ok().as_deref() == Some(&normalized) {
            kept.push(format!("{normalized} {name}"));
            replaced = true;
        } else {
            kept.push(line.to_string());
        }
    }
    if !replaced {
        kept.push(format!("{normalized} {name}"));
    }
    let mut out = kept.join("\n");
    if !out.ends_with('\n') {
        out.push('\n');
    }
    std::fs::write(&schedule_path, out)
        .map_err(|e| anyhow::anyhow!("cannot write {}: {e}", schedule_path.display()))?;
    if replaced {
        println!("updated {normalized} → {name}");
    } else {
        println!("added {normalized} → {name}");
    }
    request_reload();
    Ok(())
}

fn cmd_twilight_schedule_remove(dir: &std::path::Path, time: &str) -> Result<()> {
    let normalized = normalize_hhmm(time)?;
    let schedule_path = dir.join("schedule.conf");
    let existing = std::fs::read_to_string(&schedule_path)
        .map_err(|e| anyhow::anyhow!("cannot read {}: {e}", schedule_path.display()))?;
    let mut kept: Vec<String> = Vec::new();
    let mut dropped = false;
    for line in existing.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            kept.push(line.to_string());
            continue;
        }
        let mut parts = trimmed.split_whitespace();
        let Some(t) = parts.next() else {
            kept.push(line.to_string());
            continue;
        };
        if normalize_hhmm(t).ok().as_deref() == Some(&normalized) {
            dropped = true;
        } else {
            kept.push(line.to_string());
        }
    }
    if !dropped {
        anyhow::bail!("no schedule line found for {normalized}");
    }
    let mut out = kept.join("\n");
    if !out.ends_with('\n') {
        out.push('\n');
    }
    std::fs::write(&schedule_path, out)
        .map_err(|e| anyhow::anyhow!("cannot rewrite {}: {e}", schedule_path.display()))?;
    println!("removed schedule line at {normalized}");
    request_reload();
    Ok(())
}

fn read_preset_file(path: &std::path::Path) -> Option<(u32, u32)> {
    // Mirror the compositor-side parser in
    // `margo/src/twilight/preset.rs::load_preset_file` so the two
    // stay in sync — same two keys, same clamps.
    let text = std::fs::read_to_string(path).ok()?;
    let mut temp_k: Option<u32> = None;
    let mut gamma_pct: Option<u32> = None;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let key = k.trim();
        let val = v.trim().trim_matches('"');
        match key {
            "static_temp" => {
                if let Ok(n) = val.parse::<u32>() {
                    temp_k = Some(n.clamp(1000, 25000));
                }
            }
            "static_gamma" => {
                if let Ok(n) = val.parse::<u32>() {
                    gamma_pct = Some(n.clamp(10, 200));
                }
            }
            _ => {}
        }
    }
    Some((temp_k?, gamma_pct?))
}

fn validate_preset_name(name: &str) -> Result<()> {
    if name.is_empty() {
        anyhow::bail!("preset name cannot be empty");
    }
    if name.contains('/') || name.contains('\\') || name.starts_with('.') {
        anyhow::bail!("preset name {name:?} is invalid — no slashes, no leading dot");
    }
    Ok(())
}

fn normalize_hhmm(s: &str) -> Result<String> {
    let (h, m) = s
        .split_once(':')
        .ok_or_else(|| anyhow::anyhow!("time must be HH:MM, got {s:?}"))?;
    let h: u32 = h
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("bad hour in {s:?}"))?;
    let m: u32 = m
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("bad minute in {s:?}"))?;
    if h >= 24 || m >= 60 {
        anyhow::bail!("HH must be 0–23 and MM 0–59, got {s:?}");
    }
    Ok(format!("{h:02}:{m:02}"))
}

/// Best-effort `reload_config` dispatch so a preset edit takes
/// effect without the user having to run `mctl reload`. Failures
/// are silent (compositor not running, no Wayland socket, etc.) —
/// the file write already succeeded, and `mctl reload` later will
/// finish the job.
fn request_reload() {
    // Best-effort fire-and-forget reload over the IPC socket. Any
    // failure (compositor not running, no socket) is ignored — the
    // file write already happened and `mctl reload` later closes the loop.
    let _ = send_dispatch("reload_config", &[]);
}

fn cmd_config_errors() -> Result<()> {
    use margo_config::diagnostics::{ConfigDiagnostic, Severity};
    let state = read_state_file().map_err(|e| anyhow::anyhow!("cannot query margo IPC: {e}"))?;
    let entries = state
        .get("config_errors")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if entries.is_empty() {
        println!("no config errors");
        return Ok(());
    }
    let colored = std::io::IsTerminal::is_terminal(&std::io::stderr());
    let mut errs = 0usize;
    let mut warns = 0usize;
    for entry in &entries {
        let severity = match entry.get("severity").and_then(|v| v.as_str()) {
            Some("error") => {
                errs += 1;
                Severity::Error
            }
            _ => {
                warns += 1;
                Severity::Warning
            }
        };
        let d = ConfigDiagnostic {
            path: entry
                .get("path")
                .and_then(|v| v.as_str())
                .map(std::path::PathBuf::from)
                .unwrap_or_default(),
            line: entry.get("line").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
            col: entry.get("col").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
            end_col: entry.get("end_col").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
            severity,
            code: entry
                .get("code")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            message: entry
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            line_text: entry
                .get("line_text")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        };
        eprint!("{}", d.render(colored));
        eprintln!();
    }
    println!(
        "result: {} error{}, {} warning{}",
        errs,
        if errs == 1 { "" } else { "s" },
        warns,
        if warns == 1 { "" } else { "s" },
    );
    if errs > 0 {
        std::process::exit(1);
    }
    Ok(())
}

fn cmd_config_list(config_override: Option<&std::path::Path>) -> Result<()> {
    let gens = margo_config::generations::list()?;
    if gens.is_empty() {
        println!(
            "no saved generations yet — one is created on the next successful boot or `mctl reload`"
        );
        return Ok(());
    }
    // Best-effort: an unreadable live file just means the "vs current"
    // column is blank for this run, not a hard error for the whole
    // command.
    let live_content = std::fs::read_to_string(resolve_live_config_path(config_override)).ok();
    for (idx, r#gen) in gens.iter().enumerate() {
        let when: chrono::DateTime<chrono::Local> = r#gen.timestamp.into();
        let vs_current = live_content
            .as_deref()
            .and_then(|live| {
                margo_config::generations::read(&r#gen.id)
                    .ok()
                    .map(|gen_content| diff_stat_vs_current(&gen_content, live))
            })
            .unwrap_or_default();
        println!(
            "{idx:>3}  {}  {}  {vs_current}",
            when.format("%Y-%m-%d %H:%M:%S"),
            r#gen.id
        );
    }
    Ok(())
}

/// Short "vs current file" marker for one `mctl config list` row:
/// `(current)` on an exact match, else a `+added/-removed` line-count
/// diff-stat (relative to `gen_content`, matching `cmd_config_diff`'s
/// old=generation / new=live direction).
fn diff_stat_vs_current(gen_content: &str, live_content: &str) -> String {
    if gen_content == live_content {
        return "(current)".to_string();
    }
    let diff = similar::TextDiff::from_lines(gen_content, live_content);
    let mut added = 0usize;
    let mut removed = 0usize;
    for change in diff.iter_all_changes() {
        match change.tag() {
            similar::ChangeTag::Insert => added += 1,
            similar::ChangeTag::Delete => removed += 1,
            similar::ChangeTag::Equal => {}
        }
    }
    format!("+{added}/-{removed}")
}

fn resolve_live_config_path(config_override: Option<&std::path::Path>) -> std::path::PathBuf {
    config_override.map(|p| p.to_path_buf()).unwrap_or_else(|| {
        std::path::PathBuf::from(format!(
            "{}/.config/margo/config.conf",
            std::env::var("HOME").unwrap_or_else(|_| ".".to_string())
        ))
    })
}

fn cmd_config_diff(n: Option<usize>, config_override: Option<&std::path::Path>) -> Result<()> {
    let gens = margo_config::generations::list()?;
    let idx = n.unwrap_or(0);
    let r#gen = gens
        .get(idx)
        .ok_or_else(|| anyhow::anyhow!("no generation at index {idx} (have {})", gens.len()))?;
    let old = margo_config::generations::read(&r#gen.id)?;
    let live_path = resolve_live_config_path(config_override);
    let new = std::fs::read_to_string(&live_path)
        .with_context(|| format!("reading {}", live_path.display()))?;
    let diff = similar::TextDiff::from_lines(&old, &new);
    print!(
        "{}",
        diff.unified_diff().header(
            &format!("generation {}", r#gen.id),
            &live_path.display().to_string()
        )
    );
    Ok(())
}

/// Default rollback index when the user didn't pass `N` explicitly.
///
/// `0` (the last known-good generation) if the live file's content
/// differs from the newest saved generation's — i.e. the live file
/// carries an edit that was never itself successfully saved as a
/// generation (a failed reload, or a boot-time parse failure): that
/// newest generation IS the last-known-good state, so the default
/// must not skip past it. `1` (one generation before the newest) if
/// they match — the classic "reloaded fine, now I dislike it" case.
/// Falls back to `1` (today's pre-fix behavior) whenever there isn't
/// enough information to compare — `get(1)` errors clearly if that's
/// out of range, same as before this fix.
fn default_rollback_index(
    live_content: Option<&str>,
    newest_generation_content: Option<&str>,
) -> usize {
    match (live_content, newest_generation_content) {
        (Some(live), Some(newest)) if live != newest => 0,
        _ => 1,
    }
}

fn cmd_config_rollback(
    n: Option<usize>,
    yes: bool,
    config_override: Option<&std::path::Path>,
) -> Result<()> {
    let gens = margo_config::generations::list()?;
    let live_path = resolve_live_config_path(config_override);

    let idx = match n {
        Some(n) => n,
        None => {
            let live_content = std::fs::read_to_string(&live_path).ok();
            let newest_content = gens
                .first()
                .and_then(|g| margo_config::generations::read(&g.id).ok());
            default_rollback_index(live_content.as_deref(), newest_content.as_deref())
        }
    };
    let r#gen = gens
        .get(idx)
        .ok_or_else(|| anyhow::anyhow!("no generation at index {idx} (have {})", gens.len()))?;
    let content = margo_config::generations::read(&r#gen.id)?;

    // Safety check: run the real validator (not the permissive parser)
    // against the candidate content before writing it — a generation
    // can be parseable-but-validator-rejected (the boot-path save
    // never runs the validator, only the parser). Written to a temp
    // file *in the same directory* as the live path so relative
    // `source = ...` lines resolve exactly as they would for the live
    // file itself.
    let tmp_path = live_path
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .join(format!(".mctl-rollback-check-{}.conf", std::process::id()));
    std::fs::write(&tmp_path, &content)
        .with_context(|| format!("writing temp file {}", tmp_path.display()))?;
    let validated = margo_config::validator::validate_config(Some(&tmp_path));
    let _ = std::fs::remove_file(&tmp_path);
    let report =
        validated.with_context(|| format!("validating generation {} before rollback", r#gen.id))?;
    if report.has_errors() {
        let err_count = report.errors().count();
        bail!(
            "generation {} fails validation ({err_count} error{}); refusing to roll back to it.",
            r#gen.id,
            if err_count == 1 { "" } else { "s" },
        );
    }

    let is_symlink = std::fs::symlink_metadata(&live_path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false);
    if is_symlink && !yes {
        let target = std::fs::read_link(&live_path).unwrap_or_default();
        eprintln!(
            "{} is a symlink to {} — rollback writes through it, changing that file.",
            live_path.display(),
            target.display()
        );
        eprint!("Continue? [y/N] ");
        use std::io::Write;
        std::io::stdout().flush().ok();
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        if !answer.trim().eq_ignore_ascii_case("y") {
            println!("rollback cancelled");
            return Ok(());
        }
    }

    // Back up whatever's currently on disk before overwriting it. This
    // is the one write path that can destroy content that was never
    // itself saved as a generation (a failed reload, or a boot-time
    // fallback edit) — deliberately NOT via `generations::save()`:
    // that store is the boot-fallback's trusted "known-good" source,
    // and this content is exactly what's about to be replaced
    // (possibly for being broken); seeding it here would poison the
    // next fallback.
    if let Ok(current) = std::fs::read_to_string(&live_path) {
        let mut backup_name = live_path.as_os_str().to_owned();
        backup_name.push(".bak");
        let backup_path = std::path::PathBuf::from(backup_name);
        std::fs::write(&backup_path, &current)
            .with_context(|| format!("writing backup {}", backup_path.display()))?;
        println!("backed up current config to {}", backup_path.display());
    }

    std::fs::write(&live_path, &content)
        .with_context(|| format!("writing {}", live_path.display()))?;
    println!(
        "restored generation {} to {}",
        r#gen.id,
        live_path.display()
    );
    send_dispatch("reload", &[])?;
    println!("reloaded");
    Ok(())
}

fn cmd_check_config(config_override: Option<&std::path::Path>, strict: bool) -> Result<()> {
    use margo_config::diagnostics::Severity;

    let cfg_path = config_override.map(|p| p.to_path_buf()).unwrap_or_else(|| {
        std::path::PathBuf::from(format!(
            "{}/.config/margo/config.conf",
            std::env::var("HOME").unwrap_or_else(|_| ".".to_string())
        ))
    });

    if !cfg_path.exists() {
        // A missing config is NOT an error — margo falls back to built-in
        // defaults. Treat it as a clean pass so the start-margo preflight
        // doesn't refuse to launch on a first login (before any config or
        // dotfiles exist). Only a config that *exists but is broken* fails.
        println!(
            "config OK: no file at {} — margo will use built-in defaults",
            cfg_path.display()
        );
        return Ok(());
    }

    // Pass 1 — the structured validator over the whole `source` tree:
    // unknown keys and rule options, bad values, stray commas, unresolved
    // includes, rule regexes, duplicate binds (keymode-aware, first wins).
    let mut report =
        margo_config::validator::validate_config(Some(&cfg_path)).unwrap_or_else(|e| {
            eprintln!("error: validator could not read config: {e}");
            std::process::exit(2);
        });

    // Pass 2 — try to fully parse for the summary line. If the parser
    // itself bails (rare; it's permissive) record an E000 diagnostic
    // and keep going so the rest of the diagnostic stream still prints.
    let cfg = margo_config::parse_config(Some(&cfg_path)).ok();

    // Stable render order: by file, line, then severity (errors first).
    report.diagnostics.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then(a.line.cmp(&b.line))
            .then(a.col.cmp(&b.col))
            .then_with(|| match (a.severity, b.severity) {
                (Severity::Error, Severity::Warning) => std::cmp::Ordering::Less,
                (Severity::Warning, Severity::Error) => std::cmp::Ordering::Greater,
                _ => std::cmp::Ordering::Equal,
            })
    });

    let colored = std::io::IsTerminal::is_terminal(&std::io::stderr());
    let err_count = report.errors().count();
    let warn_count = report.warnings().count();

    for d in &report.diagnostics {
        eprint!("{}", d.render(colored));
        eprintln!();
    }

    println!("config: {}", cfg_path.display());
    if let Some(cfg) = cfg.as_ref() {
        println!(
            "summary: binds={} windowrules={} layerrules={} monitorrules={} tagrules={}",
            cfg.key_bindings.len(),
            cfg.window_rules.len(),
            cfg.layer_rules.len(),
            cfg.monitor_rules.len(),
            cfg.tag_rules.len(),
        );
    }

    if err_count == 0 && warn_count == 0 {
        println!("✓ no problems detected");
        return Ok(());
    }
    println!(
        "result: {} error{}, {} warning{}",
        err_count,
        if err_count == 1 { "" } else { "s" },
        warn_count,
        if warn_count == 1 { "" } else { "s" },
    );
    if err_count > 0 || (strict && warn_count > 0) {
        if err_count == 0 {
            println!("--strict: warnings count as failures");
        }
        std::process::exit(1);
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

/// Render `mctl status` from the richer state snapshot snapshot —
/// per-output blocks with proper connector names + tag client
/// counts. Used when the file exists (post-r143 margo).
fn print_status_rich(state: &serde_json::Value, output_filter: Option<&str>) {
    use std::io::IsTerminal;
    let tty = std::io::stdout().is_terminal();
    let bold = if tty { "\x1b[1m" } else { "" };
    let dim = if tty { "\x1b[2m" } else { "" };
    let cyan = if tty { "\x1b[36m" } else { "" };
    let yellow = if tty { "\x1b[33m" } else { "" };
    let green = if tty { "\x1b[32m" } else { "" };
    let red = if tty { "\x1b[31m" } else { "" };
    let reset = if tty { "\x1b[0m" } else { "" };

    let outputs = match state["outputs"].as_array() {
        Some(o) => o,
        None => return,
    };
    let clients = state["clients"]
        .as_array()
        .map(|v| v.as_slice())
        .unwrap_or(&[]);
    let layout_names: Vec<String> = state["layouts"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let tag_count = state["tag_count"].as_u64().unwrap_or(9) as usize;

    let mut printed_any = false;
    for out in outputs {
        let name = out["name"].as_str().unwrap_or("");
        if let Some(filter) = output_filter
            && name != filter
        {
            continue;
        }
        let active = out["active"].as_bool().unwrap_or(false);
        let layout_idx = out["layout_idx"].as_u64().unwrap_or(0) as usize;
        let layout = layout_names
            .get(layout_idx)
            .map(String::as_str)
            .unwrap_or("?");

        let active_marker = if active {
            format!("{green}●{reset} ")
        } else {
            "  ".to_string()
        };
        if printed_any {
            println!();
        }
        printed_any = true;
        println!("{active_marker}{bold}{name}{reset}  {dim}layout {reset}{cyan}{layout}{reset}");

        // Focused window on this output (find from clients array).
        let mon_idx = out["x"].as_i64(); // unused — we match by name
        let _ = mon_idx;
        let focused = clients
            .iter()
            .find(|c| c["focused"].as_bool() == Some(true) && c["monitor"].as_str() == Some(name));
        if let Some(c) = focused {
            let app = c["app_id"].as_str().unwrap_or("");
            let title = c["title"].as_str().unwrap_or("");
            let mut flags = Vec::new();
            if c["fullscreen"].as_bool().unwrap_or(false) {
                flags.push(format!("{red}FULLSCREEN{reset}"));
            }
            if c["floating"].as_bool().unwrap_or(false) {
                flags.push(format!("{yellow}FLOAT{reset}"));
            }
            let flags_str = if flags.is_empty() {
                String::new()
            } else {
                format!("  {}", flags.join(" "))
            };
            println!("    {dim}focused{reset}: {bold}{app}{reset} · {title}{flags_str}");
            let w = c["width"].as_i64().unwrap_or(0);
            let h = c["height"].as_i64().unwrap_or(0);
            if w > 0 && h > 0 {
                let x = c["x"].as_i64().unwrap_or(0);
                let y = c["y"].as_i64().unwrap_or(0);
                println!("    {dim}geometry{reset}: {w}×{h} @ {x},{y}");
            }
        } else {
            println!("    {dim}focused{reset}: (none)");
        }

        // Tag row: count clients per tag on this output.
        let active_tag = out["active_tag_mask"].as_u64().unwrap_or(0) as u32;
        let mut counts = vec![0u32; tag_count];
        for c in clients {
            if c["monitor"].as_str() != Some(name) {
                continue;
            }
            let tags = c["tags"].as_u64().unwrap_or(0) as u32;
            for (i, slot) in counts.iter_mut().enumerate() {
                if tags & (1 << i) != 0 {
                    *slot += 1;
                }
            }
        }
        let mut row = String::new();
        for (i, count) in counts.iter().enumerate() {
            let n = i + 1;
            let on = active_tag & (1 << i) != 0;
            let label = format!("{n}·{}", count);
            let cell = if on {
                let any_focused_here = clients.iter().any(|c| {
                    c["focused"].as_bool() == Some(true)
                        && c["monitor"].as_str() == Some(name)
                        && (c["tags"].as_u64().unwrap_or(0) as u32 & (1 << i)) != 0
                });
                if any_focused_here {
                    format!("{green}[{label}]●{reset}")
                } else {
                    format!("{cyan}[{label}]{reset}")
                }
            } else if counts[i] > 0 {
                label.clone()
            } else {
                format!("{dim}{label}{reset}")
            };
            if !row.is_empty() {
                row.push_str("  ");
            }
            row.push_str(&cell);
        }
        println!("    {dim}tags{reset}: {row}");
    }

    if printed_any {
        println!();
        println!(
            "{dim}layouts:{reset} {}",
            layout_names
                .iter()
                .enumerate()
                .map(|(i, l)| format!("{i}:{l}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
    } else if let Some(f) = output_filter {
        eprintln!("(no output named `{f}`)");
    }
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

fn read_state_file() -> Result<serde_json::Value> {
    // Query the compositor's IPC socket for the full state snapshot.
    // (Same document margo used to dump to state snapshot — the function
    // name is kept so every caller stays unchanged.)
    mctl::ipc_client::request_once("get state").with_context(|| {
        format!(
            "query margo IPC socket {}: is margo running?",
            mctl::ipc_client::socket_path().display()
        )
    })
}

fn cmd_clients(
    json_out: bool,
    tag_filter: Option<u32>,
    monitor_filter: Option<&str>,
    appid_filter: Option<&str>,
    wide: bool,
) -> Result<()> {
    use std::io::IsTerminal;
    let state = read_state_file()?;
    let clients = state["clients"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("state file missing `clients` array"))?;

    let want_tag_mask = tag_filter.map(|n| 1u32 << (n.saturating_sub(1).min(31)));

    let filtered: Vec<&serde_json::Value> = clients
        .iter()
        .filter(|c| {
            if let Some(mask) = want_tag_mask {
                let tags = c["tags"].as_u64().unwrap_or(0) as u32;
                if tags & mask == 0 {
                    return false;
                }
            }
            if let Some(mon) = monitor_filter
                && c["monitor"].as_str().unwrap_or("") != mon
            {
                return false;
            }
            if let Some(needle) = appid_filter {
                let needle = needle.to_lowercase();
                let app = c["app_id"].as_str().unwrap_or("").to_lowercase();
                if !app.contains(&needle) {
                    return false;
                }
            }
            true
        })
        .collect();

    if json_out {
        let arr: Vec<_> = filtered.iter().cloned().cloned().collect();
        println!("{}", serde_json::to_string_pretty(&arr)?);
        return Ok(());
    }

    if filtered.is_empty() {
        println!("(no matching clients)");
        return Ok(());
    }

    let tty = std::io::stdout().is_terminal();
    let bold = if tty { "\x1b[1m" } else { "" };
    let dim = if tty { "\x1b[2m" } else { "" };
    let green = if tty { "\x1b[32m" } else { "" };
    let yellow = if tty { "\x1b[33m" } else { "" };
    let red = if tty { "\x1b[31m" } else { "" };
    let reset = if tty { "\x1b[0m" } else { "" };

    // Decode tag bitmask → comma list of tag numbers.
    let decode_tags = |mask: u32| -> String {
        let mut v = Vec::new();
        for i in 0..9 {
            if mask & (1 << i) != 0 {
                v.push(format!("{}", i + 1));
            }
        }
        if v.is_empty() {
            "—".to_string()
        } else {
            v.join(",")
        }
    };

    // Compute column widths.
    let max_tag = filtered
        .iter()
        .map(|c| decode_tags(c["tags"].as_u64().unwrap_or(0) as u32).len())
        .max()
        .unwrap_or(3)
        .max(3);
    let max_mon = filtered
        .iter()
        .map(|c| c["monitor"].as_str().unwrap_or("").len())
        .max()
        .unwrap_or(3)
        .max(3);
    let max_app = filtered
        .iter()
        .map(|c| c["app_id"].as_str().unwrap_or("").len())
        .max()
        .unwrap_or(6)
        .clamp(6, 28);

    // Header.
    if wide {
        println!(
            "{bold}{:<w_tag$}  {:<w_mon$}  {:<w_app$}  {:<22}  TITLE{reset}",
            "TAG",
            "MON",
            "APP-ID",
            "GEOMETRY",
            w_tag = max_tag,
            w_mon = max_mon,
            w_app = max_app,
        );
    } else {
        println!(
            "{bold}{:<w_tag$}  {:<w_mon$}  {:<w_app$}  TITLE{reset}",
            "TAG",
            "MON",
            "APP-ID",
            w_tag = max_tag,
            w_mon = max_mon,
            w_app = max_app,
        );
    }

    for c in &filtered {
        let tags = decode_tags(c["tags"].as_u64().unwrap_or(0) as u32);
        let mon = c["monitor"].as_str().unwrap_or("");
        let app = c["app_id"].as_str().unwrap_or("");
        let title = c["title"].as_str().unwrap_or("");
        let geom = format!(
            "{}×{}+{}+{}",
            c["width"].as_i64().unwrap_or(0),
            c["height"].as_i64().unwrap_or(0),
            c["x"].as_i64().unwrap_or(0),
            c["y"].as_i64().unwrap_or(0),
        );
        let mut markers = String::new();
        if c["focused"].as_bool().unwrap_or(false) {
            markers.push_str(&format!("{green}●{reset} "));
        }
        if c["fullscreen"].as_bool().unwrap_or(false) {
            markers.push_str(&format!("{red}⛶{reset} "));
        }
        if c["floating"].as_bool().unwrap_or(false) {
            markers.push_str(&format!("{yellow}⬚{reset} "));
        }
        if c["minimized"].as_bool().unwrap_or(false) {
            markers.push_str(&format!("{dim}↓{reset} "));
        }
        // ★ marker: this client is currently being scanned out
        // directly from a primary/overlay plane (zero-copy). Cheap
        // signal that compositor blending overhead is bypassed.
        if c["scanout"].as_bool().unwrap_or(false) {
            markers.push_str(&format!("{green}★{reset} "));
        }
        // Count/slice by chars, not bytes: a byte slice panics when
        // `max_app-1` lands mid-UTF-8 (app-ids can be non-ASCII), and
        // byte-length over-counts a short multibyte name as too wide.
        let app_disp = if app.chars().count() > max_app {
            let keep = max_app.saturating_sub(1);
            format!("{}…", app.chars().take(keep).collect::<String>())
        } else {
            app.to_string()
        };
        if wide {
            println!(
                "{:<w_tag$}  {:<w_mon$}  {:<w_app$}  {:<22}  {markers}{title}",
                tags,
                mon,
                app_disp,
                geom,
                w_tag = max_tag,
                w_mon = max_mon,
                w_app = max_app,
            );
        } else {
            println!(
                "{:<w_tag$}  {:<w_mon$}  {:<w_app$}  {markers}{title}",
                tags,
                mon,
                app_disp,
                w_tag = max_tag,
                w_mon = max_mon,
                w_app = max_app,
            );
        }
    }

    Ok(())
}

fn cmd_outputs(json_out: bool) -> Result<()> {
    use std::io::IsTerminal;
    let state = read_state_file()?;
    let outputs = state["outputs"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("state file missing `outputs` array"))?;

    if json_out {
        println!("{}", serde_json::to_string_pretty(outputs)?);
        return Ok(());
    }

    let tty = std::io::stdout().is_terminal();
    let bold = if tty { "\x1b[1m" } else { "" };
    let dim = if tty { "\x1b[2m" } else { "" };
    let green = if tty { "\x1b[32m" } else { "" };
    let cyan = if tty { "\x1b[36m" } else { "" };
    let reset = if tty { "\x1b[0m" } else { "" };

    println!(
        "{bold}{:<10}  {:<11}  {:<6}  {:<10}  ACTIVE-TAGS{reset}",
        "NAME", "POSITION", "SCALE", "MODE",
    );
    for o in outputs {
        let name = o["name"].as_str().unwrap_or("");
        let x = o["x"].as_i64().unwrap_or(0);
        let y = o["y"].as_i64().unwrap_or(0);
        let w = o["width"].as_i64().unwrap_or(0);
        let h = o["height"].as_i64().unwrap_or(0);
        let scale = o["scale"].as_f64().unwrap_or(1.0);
        let mode = format!(
            "{}×{}",
            o["mode"]["physical_width"].as_i64().unwrap_or(0),
            o["mode"]["physical_height"].as_i64().unwrap_or(0),
        );
        let active = o["active"].as_bool().unwrap_or(false);
        let active_mark = if active {
            format!("{green}●{reset} ")
        } else {
            "  ".to_string()
        };
        let active_tag = o["active_tag_mask"].as_u64().unwrap_or(0) as u32;
        let mut tags = Vec::new();
        for i in 0..9 {
            if active_tag & (1 << i) != 0 {
                tags.push(format!("{}", i + 1));
            }
        }
        let tag_str = if tags.is_empty() {
            "—".to_string()
        } else {
            tags.join(",")
        };
        println!(
            "{active_mark}{bold}{name:<8}{reset}  {dim}{x:>4},{y:<4}{reset}  {dim}{scale:<6.2}{reset}  {cyan}{mode:<10}{reset}  {tag_str} {dim}({w}×{h} logical){reset}",
        );
    }

    Ok(())
}

fn cmd_perf(json_out: bool, watch: bool) -> Result<()> {
    if watch && !json_out {
        // Live-refresh mode: clear + redraw once a second. The compositor keeps
        // a rolling 60 s sample window, so the windowed columns are meaningful
        // even on the very first frame. Loop exits (returns Err) if margo goes
        // away — request_once fails and the `?` propagates out.
        loop {
            // Clear screen + home the cursor, then draw one snapshot.
            print!("\x1b[2J\x1b[H");
            perf_print_once(false)?;
            use std::io::Write;
            std::io::stdout().flush().ok();
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }
    perf_print_once(json_out)
}

/// Fetch the `perf` topic once and print it (table or `--json`).
fn perf_print_once(json_out: bool) -> Result<()> {
    use std::io::IsTerminal;
    let perf = mctl::ipc_client::request_once("get perf").with_context(|| {
        format!(
            "query margo IPC socket {}: is margo running?",
            mctl::ipc_client::socket_path().display()
        )
    })?;
    let outputs = perf["outputs"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("perf reply missing `outputs` array"))?;

    if json_out {
        println!("{}", serde_json::to_string_pretty(outputs)?);
        return Ok(());
    }

    if outputs.is_empty() {
        // The winit/nested backend keeps no per-output DRM counters.
        println!("no per-output counters (winit/nested backend, or nothing rendered yet)");
        return Ok(());
    }

    let tty = std::io::stdout().is_terminal();
    let bold = if tty { "\x1b[1m" } else { "" };
    let dim = if tty { "\x1b[2m" } else { "" };
    let cyan = if tty { "\x1b[36m" } else { "" };
    let yellow = if tty { "\x1b[33m" } else { "" };
    let reset = if tty { "\x1b[0m" } else { "" };

    // Live health first (windowed FPS + recent empty ratio + render-latency
    // tail + direct-scanout share + composited damage area), then lifetime
    // queue-error total. Cumulative renders/queued/empties stay available
    // via `--json`.
    println!(
        "{bold}{:<10}  {:>7}  {:>7}  {:>7}  {:>22}  {:>6}  {:>17}  {:>6}{reset}",
        "OUTPUT",
        "FPS 1s",
        "FPS 60s",
        "EMPTY%",
        "RENDER p50/p95/p99 ms",
        "SCAN%",
        "DMG p50/p95 Mpx",
        "Q-ERR",
    );
    for o in outputs {
        let name = o["name"].as_str().unwrap_or("");
        let q_err = o["queue_errors"].as_u64().unwrap_or(0);
        // Queue errors are the one counter that's bad when non-zero.
        let err_col = if q_err > 0 { yellow } else { dim };
        let n = o["window_samples"].as_u64().unwrap_or(0);
        if n == 0 {
            // Output present but no frames in the window yet — don't invent 0.0s.
            println!(
                "{bold}{name:<10}{reset}  {dim}{:>7}  {:>7}  {:>7}  {:>22}  {:>6}  {:>17}{reset}  {err_col}{q_err:>6}{reset}",
                "—", "—", "—", "—", "—", "—",
            );
            continue;
        }
        let fps1 = o["fps_1s"].as_f64().unwrap_or(0.0);
        let fps60 = o["fps_60s"].as_f64().unwrap_or(0.0);
        let empty_pct = o["empty_ratio_10s"].as_f64().unwrap_or(0.0) * 100.0;
        let p50 = o["render_us_p50"].as_u64().unwrap_or(0) as f64 / 1000.0;
        let p95 = o["render_us_p95"].as_u64().unwrap_or(0) as f64 / 1000.0;
        let p99 = o["render_us_p99"].as_u64().unwrap_or(0) as f64 / 1000.0;
        let lat = format!("{p50:.2} / {p95:.2} / {p99:.2}");
        let scan_pct = o["scanout_ratio_10s"].as_f64().unwrap_or(0.0) * 100.0;
        let dmg50 = o["damage_px_p50"].as_u64().unwrap_or(0) as f64 / 1_000_000.0;
        let dmg95 = o["damage_px_p95"].as_u64().unwrap_or(0) as f64 / 1_000_000.0;
        let dmg = format!("{dmg50:.2} / {dmg95:.2}");
        println!(
            "{bold}{name:<10}{reset}  {cyan}{fps1:>7.1}{reset}  {fps60:>7.1}  {cyan}{empty_pct:>6.1}%{reset}  {lat:>22}  {cyan}{scan_pct:>5.1}%{reset}  {dmg:>17}  {err_col}{q_err:>6}{reset}",
        );
    }

    Ok(())
}

fn cmd_focused(json_out: bool) -> Result<()> {
    let state = read_state_file()?;
    let clients = state["clients"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("state file missing `clients` array"))?;
    let Some(focused) = clients
        .iter()
        .find(|c| c["focused"].as_bool() == Some(true))
    else {
        if json_out {
            println!("null");
        } else {
            println!("(no focused window)");
        }
        return Ok(());
    };
    if json_out {
        println!("{}", serde_json::to_string_pretty(focused)?);
        return Ok(());
    }
    let app = focused["app_id"].as_str().unwrap_or("");
    let title = focused["title"].as_str().unwrap_or("");
    println!("{app} · {title}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Args, Command, ConfigCmd, default_rollback_index, diff_stat_vs_current};
    use clap::Parser;

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

    #[test]
    fn config_list_parses() {
        let args = Args::try_parse_from(["mctl", "config", "list"]).unwrap();
        let Command::Config {
            action: ConfigCmd::List { config },
        } = args.command
        else {
            panic!("expected ConfigCmd::List");
        };
        assert_eq!(config, None);
    }

    #[test]
    fn config_list_accepts_config_override() {
        let args =
            Args::try_parse_from(["mctl", "config", "list", "--config", "/tmp/x.conf"]).unwrap();
        let Command::Config {
            action: ConfigCmd::List { config },
        } = args.command
        else {
            panic!("expected ConfigCmd::List");
        };
        assert_eq!(config, Some(std::path::PathBuf::from("/tmp/x.conf")));
    }

    #[test]
    fn config_diff_defaults_n_to_none() {
        let args = Args::try_parse_from(["mctl", "config", "diff"]).unwrap();
        let Command::Config {
            action: ConfigCmd::Diff { n, config },
        } = args.command
        else {
            panic!("expected ConfigCmd::Diff");
        };
        assert_eq!(n, None);
        assert_eq!(config, None);
    }

    #[test]
    fn config_diff_accepts_explicit_n() {
        let args = Args::try_parse_from(["mctl", "config", "diff", "2"]).unwrap();
        let Command::Config {
            action: ConfigCmd::Diff { n, config },
        } = args.command
        else {
            panic!("expected ConfigCmd::Diff");
        };
        assert_eq!(n, Some(2));
        assert_eq!(config, None);
    }

    #[test]
    fn config_diff_accepts_config_override() {
        let args = Args::try_parse_from(["mctl", "config", "diff", "2", "--config", "/tmp/x.conf"])
            .unwrap();
        let Command::Config {
            action: ConfigCmd::Diff { n, config },
        } = args.command
        else {
            panic!("expected ConfigCmd::Diff");
        };
        assert_eq!(n, Some(2));
        assert_eq!(config, Some(std::path::PathBuf::from("/tmp/x.conf")));
    }

    #[test]
    fn config_rollback_defaults_n_and_yes() {
        let args = Args::try_parse_from(["mctl", "config", "rollback"]).unwrap();
        let Command::Config {
            action: ConfigCmd::Rollback { n, yes, config },
        } = args.command
        else {
            panic!("expected ConfigCmd::Rollback");
        };
        assert_eq!(n, None);
        assert!(!yes);
        assert_eq!(config, None);
    }

    #[test]
    fn config_rollback_accepts_n_and_yes_flag() {
        let args = Args::try_parse_from(["mctl", "config", "rollback", "3", "--yes"]).unwrap();
        let Command::Config {
            action: ConfigCmd::Rollback { n, yes, config },
        } = args.command
        else {
            panic!("expected ConfigCmd::Rollback");
        };
        assert_eq!(n, Some(3));
        assert!(yes);
        assert_eq!(config, None);
    }

    #[test]
    fn config_rollback_accepts_config_override() {
        let args = Args::try_parse_from([
            "mctl",
            "config",
            "rollback",
            "--yes",
            "--config",
            "/tmp/x.conf",
        ])
        .unwrap();
        let Command::Config {
            action: ConfigCmd::Rollback { n, yes, config },
        } = args.command
        else {
            panic!("expected ConfigCmd::Rollback");
        };
        assert_eq!(n, None);
        assert!(yes);
        assert_eq!(config, Some(std::path::PathBuf::from("/tmp/x.conf")));
    }

    // ── default_rollback_index (Fix 1) ──────────────────────────────

    #[test]
    fn default_rollback_index_is_zero_when_live_differs_from_newest() {
        // The live file carries an unsaved edit (failed reload, or
        // boot fallback) — the newest generation IS the last
        // known-good state, so the default must not skip past it.
        assert_eq!(default_rollback_index(Some("a"), Some("b")), 0);
    }

    #[test]
    fn default_rollback_index_is_one_when_live_matches_newest() {
        // Nothing's changed since the last successful save — go back
        // one further, past the state the user says they dislike.
        assert_eq!(default_rollback_index(Some("a"), Some("a")), 1);
    }

    #[test]
    fn default_rollback_index_falls_back_to_one_when_live_unreadable() {
        assert_eq!(default_rollback_index(None, Some("a")), 1);
    }

    #[test]
    fn default_rollback_index_falls_back_to_one_when_no_generations() {
        assert_eq!(default_rollback_index(Some("a"), None), 1);
    }

    // ── diff_stat_vs_current (Fix 6) ────────────────────────────────

    #[test]
    fn diff_stat_vs_current_marks_exact_match() {
        assert_eq!(diff_stat_vs_current("a\nb\n", "a\nb\n"), "(current)");
    }

    #[test]
    fn diff_stat_vs_current_counts_added_and_removed_lines() {
        let stat = diff_stat_vs_current("a\nb\n", "a\nc\n");
        assert_eq!(stat, "+1/-1");
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

    #[test]
    fn manifest_fields() {
        let text = "# a plugin\nname = \"clock\"\nversion='1.2'\ndescription = a=b\n";
        assert_eq!(manifest_field(text, "name").as_deref(), Some("clock"));
        assert_eq!(manifest_field(text, "version").as_deref(), Some("1.2"));
        assert_eq!(manifest_field(text, "description").as_deref(), Some("a=b"));
        assert_eq!(manifest_field(text, "enabled"), None);
        assert_eq!(
            manifest_field("# name = x\n", "name"),
            None,
            "commented out"
        );
    }

    #[test]
    fn manifest_inline_comments_are_stripped_like_the_loader() {
        // margo's plugin loader drops everything after `#`; mctl read
        // `true # …` as a non-true value and listed the plugin disabled
        // (the loader's own doc example has such a comment).
        assert!(manifest_enabled(
            "name = clock\nenabled = true                # default true; flip to disable\n"
        ));
        assert!(!manifest_enabled("enabled = false # off for now\n"));
        assert_eq!(
            manifest_field("name = \"clock\"  # shown in lists\n", "name").as_deref(),
            Some("clock")
        );
        assert_eq!(
            manifest_field("   # name = x\n", "name"),
            None,
            "indented comment"
        );
    }

    #[test]
    fn manifest_enabled_defaults_to_true() {
        assert!(manifest_enabled("name = x\n"));
        assert!(manifest_enabled("enabled = yes\n"));
        assert!(manifest_enabled("enabled = on"));
        assert!(!manifest_enabled("enabled = false\n"));
        assert!(!manifest_enabled("enabled = 0\n"));
        assert!(
            !manifest_enabled("enabled = nah\n"),
            "unknown values disable"
        );
    }

    #[test]
    fn enabled_line_is_rewritten_or_appended() {
        assert_eq!(
            with_enabled_line("name = x\nenabled = true\nversion = 1\n", false),
            "name = x\nenabled = false\nversion = 1\n"
        );
        assert_eq!(
            with_enabled_line("name = x\n", true),
            "name = x\nenabled = true\n"
        );
        assert_eq!(
            with_enabled_line("name = x\n  enabled=false", true),
            "name = x\nenabled = true\n"
        );
        assert_eq!(
            with_enabled_line("# enabled = false\nname = x", false),
            "# enabled = false\nname = x\nenabled = false\n",
            "a commented-out key is left alone"
        );
    }

    #[test]
    fn hhmm_is_normalised_and_range_checked() {
        assert_eq!(normalize_hhmm("7:5").unwrap(), "07:05");
        assert_eq!(normalize_hhmm(" 23 : 59 ").unwrap(), "23:59");
        assert_eq!(normalize_hhmm("00:00").unwrap(), "00:00");
        for bad in ["24:00", "12:60", "1200", "", ":", "a:b", "-1:00", "12:5:00"] {
            assert!(normalize_hhmm(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn preset_names_cannot_escape_the_directory() {
        assert!(validate_preset_name("night").is_ok());
        assert!(validate_preset_name("warm-2").is_ok());
        for bad in ["", "../x", "a/b", "a\\b", ".hidden", "..", "."] {
            assert!(validate_preset_name(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn preset_file_needs_both_keys_and_clamps() {
        let dir = std::env::temp_dir().join(format!("mctl-preset-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let write = |name: &str, body: &str| {
            let p = dir.join(name);
            std::fs::write(&p, body).unwrap();
            p
        };
        let p = write(
            "ok.conf",
            "# night\n[preset]\nstatic_temp = 3400\nstatic_gamma = \"90\"\n",
        );
        assert_eq!(read_preset_file(&p), Some((3400, 90)));
        let p = write("clamp.conf", "static_temp=100\nstatic_gamma=999\n");
        assert_eq!(read_preset_file(&p), Some((1000, 200)));
        let p = write("half.conf", "static_temp=4000\n");
        assert_eq!(read_preset_file(&p), None);
        let p = write("bad.conf", "static_temp=warm\nstatic_gamma=90\n");
        assert_eq!(read_preset_file(&p), None);
        assert_eq!(read_preset_file(&dir.join("missing.conf")), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn rule(id: &str, title: &str) -> margo_config::WindowRule {
        margo_config::WindowRule {
            id: Some(id.into()),
            title: Some(title.into()),
            ..Default::default()
        }
    }

    fn verdict(r: &margo_config::WindowRule, appid: &str, title: &str) -> Result<(), &'static str> {
        match classify_rule(r, appid, title) {
            Verdict::Match => Ok(()),
            Verdict::Reject(why) => Err(why),
        }
    }

    #[test]
    fn rule_classification() {
        assert_eq!(verdict(&rule("^firefox$", ""), "firefox", "x"), Ok(()));
        assert_eq!(
            verdict(&rule("^firefox$", ""), "firefox-dev", "x"),
            Err("appid pattern miss")
        );
        assert_eq!(
            verdict(
                &rule("", "Picture-in-Picture"),
                "firefox",
                "Picture-in-Picture"
            ),
            Ok(())
        );
        assert_eq!(
            verdict(&rule("firefox", "^PiP$"), "firefox", "Mozilla"),
            Err("title pattern miss")
        );
        assert_eq!(
            verdict(&rule("kitty", ""), "", ""),
            Err("appid pattern miss"),
            "a pattern never matches an empty value"
        );
        // An invalid regex falls back to substring matching.
        assert_eq!(verdict(&rule("^foo(", ""), "xfoo(y", ""), Ok(()));
    }

    #[test]
    fn rule_exclusions() {
        let mut r = rule("firefox", "");
        r.exclude_title = Some("Private".into());
        assert_eq!(verdict(&r, "firefox", "Mozilla Firefox"), Ok(()));
        assert_eq!(
            verdict(&r, "firefox", "Private Browsing"),
            Err("exclude_title matched")
        );
        let mut r = rule("", "");
        r.exclude_id = Some("^steam".into());
        assert_eq!(verdict(&r, "steam_app_1", ""), Err("exclude_id matched"));
        r.exclude_id = Some(String::new());
        assert_eq!(
            verdict(&r, "steam_app_1", ""),
            Ok(()),
            "empty exclusion is ignored"
        );
    }

    #[test]
    fn twilight_preview_args_parse() {
        let args = Args::try_parse_from(["mctl", "twilight", "preview", "3500", "80"]).unwrap();
        assert!(matches!(
            args.command,
            Command::Twilight {
                action: TwilightCmd::Preview {
                    kelvin: 3500,
                    gamma: 80
                }
            }
        ));
        assert!(Args::try_parse_from(["mctl", "twilight", "preview", "warm"]).is_err());
    }
}
