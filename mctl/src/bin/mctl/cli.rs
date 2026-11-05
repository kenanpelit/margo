//! The clap command line: `Args`, the `Command` subcommand tree and its
//! nested action enums.

use clap::{Parser, Subcommand};
use clap_complete::Shell;

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
pub(super) struct Args {
    /// Output to target (default: focused, falls back to first)
    #[arg(short, long)]
    pub(super) output: Option<String>,

    #[command(subcommand)]
    pub(super) command: Command,
}

#[derive(Subcommand, Debug)]
pub(super) enum Command {
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

    /// Mirror one output onto another (presentation mode)
    ///
    /// Shows SOURCE's picture on TARGET, scaled to fit and letterboxed.
    /// TARGET leaves the layout (its windows move to another output and
    /// the pointer can't enter it) until `mctl mirror stop` or either
    /// output unplugs. Without names: the laptop panel onto the first
    /// external output.
    ///
    ///   mctl mirror start                 # eDP-1 → first external
    ///   mctl mirror start eDP-1 DP-3
    ///   mctl mirror toggle
    ///   mctl mirror stop
    ///   mctl mirror status [--json]
    #[command(display_order = 38)]
    Mirror {
        #[command(subcommand)]
        action: MirrorCmd,
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
pub(super) enum LogCmd {
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
pub(super) enum PluginCmd {
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
pub(super) enum ConfigCmd {
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
pub(super) enum MirrorCmd {
    /// Start mirroring SOURCE onto TARGET (defaults: laptop → external).
    Start {
        source: Option<String>,
        target: Option<String>,
    },
    /// Stop mirroring if active, else start like `start`.
    Toggle {
        source: Option<String>,
        target: Option<String>,
    },
    /// Stop mirroring; the target returns to the layout.
    Stop,
    /// Print the running mirror (`--json`: the `mirror` IPC object).
    Status {
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand, Debug)]
pub(super) enum TwilightCmd {
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
pub(super) enum TwilightPresetCmd {
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
pub(super) enum TwilightScheduleCmd {
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

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn mirror_args_parse() {
        use super::MirrorCmd;
        let args = Args::try_parse_from(["mctl", "mirror", "start", "eDP-1", "DP-3"]).unwrap();
        let Command::Mirror {
            action: MirrorCmd::Start { source, target },
        } = args.command
        else {
            panic!("expected mirror start");
        };
        assert_eq!(
            (source.as_deref(), target.as_deref()),
            (Some("eDP-1"), Some("DP-3"))
        );
        assert!(Args::try_parse_from(["mctl", "mirror", "stop"]).is_ok());
        assert!(Args::try_parse_from(["mctl", "mirror", "status", "--json"]).is_ok());
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
