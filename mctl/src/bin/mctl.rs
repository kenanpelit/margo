//! mctl — margo compositor control tool (replaces mmsg)
//!
//! Uses the zdwl_ipc_unstable_v2 Wayland protocol to query and control margo.
//! Connects to the compositor via the standard WAYLAND_DISPLAY socket.

use anyhow::{bail, Context, Result};
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::Shell;
use wayland_client::{
    globals::{registry_queue_init, GlobalListContents},
    protocol::{wl_output, wl_registry},
    Connection, Dispatch, EventQueue, QueueHandle,
};

use mctl::actions::{ACTIONS, Group};
use mctl::protocols::dwl_ipc::{
    zdwl_ipc_manager_v2::{self, ZdwlIpcManagerV2},
    zdwl_ipc_output_v2::{self, ZdwlIpcOutputV2},
};

// ── CLI ───────────────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(
    name = "mctl",
    version,
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
        display_order = 20,
        long_about = "Dispatch a compositor command by name.\n\
                      \n\
                      The action <NAME> is the same string used in `bind = MODS,KEY,<NAME>,<args>` \
                      lines in `config.conf`. Up to 5 trailing args fill the dwl-ipc dispatch slots:\n\
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
    /// By default runs `check-config` first; refuses to reload when
    /// the file has errors (so the compositor doesn't try to apply a
    /// broken config). Pass `--force` to skip the pre-check.
    #[command(display_order = 30)]
    Reload {
        /// Skip the pre-reload validation pass and dispatch the IPC
        /// reload unconditionally. Useful if the validator is wrong
        /// or the user wants to test the parser's recovery behaviour
        /// directly.
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
    /// monitor's seltags, tagset, per-tag layout/mfact/nmaster,
    /// canvas-pan, and currently-parked scratchpad entries. Open
    /// windows are not captured — clients are bound to processes,
    /// the spawn line lives in user-space.
    #[command(name = "session-save", display_order = 33, alias = "session_save")]
    SessionSave,

    /// Restore per-monitor tag/layout state from session.json
    ///
    /// Matches monitors by output name; entries for absent
    /// monitors get logged + skipped. Scratchpad presence is
    /// re-flagged on live clients matching by `app_id`.
    #[command(name = "session-load", display_order = 34, alias = "session_load")]
    SessionLoad,

    /// Stream state updates from margo (runs until Ctrl-C)
    #[command(
        display_order = 2,
        long_about = "Stream a fresh status block every time the compositor publishes a \
                      `frame` event on the targeted output. Useful for watching focus / \
                      tag / layout changes live, or for piping into `awk`/`jq` in shell \
                      scripts that react to compositor state."
    )]
    Watch,

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
    /// `mctl twilight status --json` prints the raw state.json
    /// `twilight` object for scripting consumption.
    #[command(display_order = 37, alias = "tl")]
    Twilight {
        #[command(subcommand)]
        action: TwilightCmd,
    },

    /// Show config diagnostics from the last reload (niri-style)
    ///
    /// Hyprland's `hyprctl configerrors` analogue. Empty when the
    /// last reload was clean. Reads from the live compositor (via
    /// the same state.json IPC `mctl status` uses), so it reflects
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
                        * Invalid regex patterns in `appid:`, `title:`, `exclude_*:` slots.\n  \
                        * Duplicate `bind = MODS,KEY,…` lines (one bind silently shadows the other).\n  \
                        * Unresolvable `source = …` / `include = …` includes.\n  \
                        * Lone-mango option carry-overs that margo doesn't yet implement (warning).\n  \
                      \n\
                      Exits 0 on a clean parse, 1 on errors, 2 if the file itself can't be read.\n\
                      \n\
                      EXAMPLES:\n  \
                        mctl check-config\n  \
                        mctl check-config --config ~/dotfiles/margo/config.conf\n  \
                        mctl check-config 2>&1 | grep ERROR"
    )]
    #[command(display_order = 42)]
    CheckConfig {
        /// Path to inspect. Defaults to `~/.config/margo/config.conf`.
        #[arg(long)]
        config: Option<std::path::PathBuf>,
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
                      Hand-curated completion scripts shipped under `contrib/completions/` in \
                      the source tree complete `mctl dispatch <NAME>` with the action list \
                      from `mctl actions --names` — clap-generated completions only cover the \
                      subcommand layer, so prefer the contrib scripts where possible."
    )]
    #[command(display_order = 43)]
    Completions {
        /// Shell to generate for.
        #[arg(value_enum)]
        shell: Shell,
    },

    /// List every open window with tag, monitor, app_id, title.
    ///
    /// Reads `$XDG_RUNTIME_DIR/margo/state.json` (margo refreshes
    /// it on every arrange/focus/output-change event). Same data
    /// you'd get from triggering `pkill -USR1 margo` and grepping
    /// the journal, but live and parseable.
    #[command(
        alias = "client",
        display_order = 3,
        long_about = "List every open window the compositor knows about — tag, \
                      monitor, app_id, title, geometry, focus, floating/fullscreen \
                      state. Reads `$XDG_RUNTIME_DIR/margo/state.json` which margo \
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

    /// Print the focused window's app_id + title (terse, scriptable).
    #[command(
        alias = "active",
        display_order = 5,
        long_about = "Print the focused window's app_id + title in a single \
                      line — designed for status-bar scripts that just need \
                      `who has focus right now`. `--json` for the full \
                      ClientInfo struct.\n\
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
enum TwilightCmd {
    /// Print the current twilight state (temperature, gamma, phase,
    /// source). Reads `state.json` — works even when the compositor
    /// is paused on a heavy frame.
    Status {
        /// Emit the raw `state.json` `twilight` object as JSON.
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
    Set {
        /// `field=value`. Supported fields: `day_temp`,
        /// `night_temp`, `day_gamma`, `night_gamma`, `transition_s`,
        /// `enabled` (alias `twilight`, 0/1).
        spec: String,
    },
    /// Clear any preview / test override, resume the schedule.
    Reset,
    /// Toggle the schedule on/off. Equivalent to
    /// `mctl twilight set enabled=0` / `enabled=1` but stateful —
    /// uses the current `state.config.twilight` flag.
    Toggle,
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
        if let wl_output::Event::Name { name } = event
            && let Some(o) = state.outputs.iter_mut().find(|o| o.wl_output.as_ref() == Some(proxy))
        {
            o.name = name;
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

    // Subcommands that don't need (or want) a Wayland connection —
    // documentation / scripting / config-introspection helpers that
    // should work outside a margo session, e.g. when the user is
    // generating completions during a package install or
    // sanity-checking a windowrule pattern in their editor.
    match &args.command {
        Command::Actions { verbose, group, names } => {
            return cmd_actions(*verbose, group.as_deref(), *names);
        }
        Command::Completions { shell } => {
            return cmd_completions(*shell);
        }
        Command::Rules { config, appid, title, verbose } => {
            return cmd_rules(config.as_deref(), appid, title, *verbose);
        }
        Command::CheckConfig { config } => {
            return cmd_check_config(config.as_deref());
        }
        // The state-file commands don't need a Wayland connection.
        // They read whatever margo last wrote out.
        Command::Clients { json, tag, monitor, app_id, wide } => {
            return cmd_clients(*json, *tag, monitor.as_deref(), app_id.as_deref(), *wide);
        }
        Command::Outputs { json } => {
            return cmd_outputs(*json);
        }
        Command::Focused { json } => {
            return cmd_focused(*json);
        }
        Command::ConfigErrors => {
            return cmd_config_errors();
        }
        Command::Twilight {
            action: TwilightCmd::Status { json },
        } => {
            return cmd_twilight_status(*json);
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
                        // Bind to v4+ so the connector `Name`
                        // event fires (`wl_output.name` was added
                        // in protocol version 4). Older v3 only
                        // carries Geometry / Mode / Done / Scale
                        // — no connector name, which is why
                        // `mctl status`'s `output=` field used to
                        // print empty.
                        let wl_out: wl_output::WlOutput = globals
                            .registry()
                            .bind(global.name, global.version.min(4), &qh, global.name);
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
            // arg.v is fed from slot 4 (see dwl_ipc.rs Dispatch
            // handler). Slots 1-3 are numeric-parsed and dropped
            // on a non-numeric path, so the script path goes here.
            ipc_out.dispatch(
                "run_script".to_string(),
                String::new(),
                String::new(),
                String::new(),
                abs_str,
                String::new(),
            );
            eq.roundtrip(&mut state)?;
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
        Command::Reload { force } => {
            // Pre-flight: validate before we touch IPC. The compositor
            // also fail-soft loads (it'll keep the previous config if
            // the new one parses to garbage), but stopping here is far
            // friendlier — the user sees a coloured caret-and-line
            // diagnostic in the terminal rather than a tracing log
            // line buried in journalctl. `--force` keeps the old
            // permissive behaviour.
            if !force {
                let report = margo_config::validator::validate_config(None)
                    .unwrap_or_else(|e| {
                        eprintln!("error: validator could not read config: {e}");
                        std::process::exit(2);
                    });
                if report.has_errors() {
                    let colored =
                        std::io::IsTerminal::is_terminal(&std::io::stderr());
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
                    let colored =
                        std::io::IsTerminal::is_terminal(&std::io::stderr());
                    for d in &report.diagnostics {
                        if !d.is_error() {
                            eprint!("{}", d.render(colored));
                            eprintln!();
                        }
                    }
                    eprintln!(
                        "mctl reload: proceeding with {} warning{}.",
                        report.warnings().count(),
                        if report.warnings().count() == 1 { "" } else { "s" }
                    );
                }
            }
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
        Command::Twilight { action } => {
            // Status is handled higher up (no IPC needed). The
            // remaining four actions are dispatched to the
            // compositor. Slot mapping per `Arg` doc:
            //   preview → slot 1 = Kelvin, slot 2 = gamma %
            //   test    → slot 1 = duration_s
            //   set     → slot 4 = "field=value"
            //   reset   → no args
            match action {
                TwilightCmd::Status { .. } => unreachable!(),
                TwilightCmd::Preview { kelvin, gamma } => {
                    ipc_out.dispatch(
                        "twilight_preview".to_string(),
                        kelvin.to_string(),
                        gamma.to_string(),
                        String::new(),
                        String::new(),
                        String::new(),
                    );
                }
                TwilightCmd::Test { seconds } => {
                    let s = seconds.clamp(1, 60);
                    ipc_out.dispatch(
                        "twilight_test".to_string(),
                        s.to_string(),
                        String::new(),
                        String::new(),
                        String::new(),
                        String::new(),
                    );
                }
                TwilightCmd::Set { spec } => {
                    ipc_out.dispatch(
                        "twilight_set".to_string(),
                        String::new(),
                        String::new(),
                        String::new(),
                        spec,
                        String::new(),
                    );
                }
                TwilightCmd::Reset => {
                    ipc_out.dispatch(
                        "twilight_reset".to_string(),
                        String::new(),
                        String::new(),
                        String::new(),
                        String::new(),
                        String::new(),
                    );
                }
                TwilightCmd::Toggle => {
                    ipc_out.dispatch(
                        "twilight_toggle".to_string(),
                        String::new(),
                        String::new(),
                        String::new(),
                        String::new(),
                        String::new(),
                    );
                }
            }
            eq.roundtrip(&mut state)?;
        }
        Command::Theme { preset } => {
            // dwl-ipc-v2 dispatch maps the 5 string slots as:
            //   arg1=i  arg2=i2  arg3=f  arg4=v  arg5=v2
            // The handler reads `arg.v`, so the preset string has to
            // land in slot 4. Slots 1-3 stay empty (they'd be
            // numeric-parsed and dropped on a non-numeric value).
            ipc_out.dispatch(
                "theme".to_string(),
                String::new(),
                String::new(),
                String::new(),
                preset,
                String::new(),
            );
            eq.roundtrip(&mut state)?;
        }
        Command::SessionSave => {
            ipc_out.dispatch(
                "session_save".to_string(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
            );
            eq.roundtrip(&mut state)?;
        }
        Command::SessionLoad => {
            ipc_out.dispatch(
                "session_load".to_string(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
            );
            eq.roundtrip(&mut state)?;
        }
        Command::Status { json } => {
            // Prefer the rich state.json (post-r143) which carries
            // the output's connector name plus tag-mask info that
            // dwl-ipc-v2 doesn't broadcast in a single event. Fall
            // back to the dwl-ipc snapshot if the file isn't there
            // (margo-version mismatch, race on boot, etc.).
            let used_state_file = if json {
                match read_state_file() { Ok(rich) => {
                    println!("{}", serde_json::to_string_pretty(&rich)?);
                    true
                } _ => {
                    print_status_json(&state)?;
                    true
                }}
            } else { match read_state_file() { Ok(rich) => {
                print_status_rich(&rich, args.output.as_deref());
                true
            } _ => {
                false
            }}};
            if !used_state_file {
                print_status(&state, target_idx);
            }
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
        Command::Actions { .. }
        | Command::Completions { .. }
        | Command::Rules { .. }
        | Command::CheckConfig { .. }
        | Command::ConfigErrors
        | Command::Clients { .. }
        | Command::Outputs { .. }
        | Command::Focused { .. } => {
            // Both branches return early at the top of `main`;
            // this arm only exists to keep the match exhaustive.
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

    if !matched_any
        && let Some(filter) = group_filter
    {
        bail!(
            "no group matches '{filter}'. Available: \
             Tag, Focus, Layout, Scroller, Window, Scratchpad, Overview, System"
        );
    }
    Ok(())
}

fn print_status_json(state: &IpcState) -> Result<()> {
    use serde_json::json;
    let outputs: Vec<_> = state
        .outputs
        .iter()
        .map(|out| {
            let layout_name = state
                .layouts
                .get(out.layout_idx as usize)
                .cloned()
                .unwrap_or_default();
            let tags: Vec<_> = out
                .tags
                .iter()
                .enumerate()
                .take(state.tag_count as usize)
                .map(|(i, t)| {
                    let state_str = match t.state {
                        0 => "none",
                        1 => "active",
                        2 => "urgent",
                        _ => "unknown",
                    };
                    json!({
                        "index": i + 1,
                        "state": state_str,
                        "clients": t.clients,
                        "focused": t.focused,
                    })
                })
                .collect();
            json!({
                "name": out.name,
                "active": out.active,
                "layout": out.layout_symbol,
                "layout_name": layout_name,
                "layout_idx": out.layout_idx,
                "focused": {
                    "appid": out.appid,
                    "title": out.title,
                    "fullscreen": out.fullscreen,
                    "floating": out.floating,
                    "x": out.x,
                    "y": out.y,
                    "width": out.width,
                    "height": out.height,
                },
                "tags": tags,
            })
        })
        .collect();
    // Stable JSON schema. Bump `version` on any breaking change
    // (field renamed or removed); additive changes (new fields,
    // new enum variants on existing fields) keep the version
    // unchanged and consumers should ignore unknown fields.
    let document = json!({
        "version": 1,
        "tag_count": state.tag_count,
        "layouts": state.layouts,
        "outputs": outputs,
    });
    println!("{}", serde_json::to_string_pretty(&document)?);
    Ok(())
}

fn cmd_rules(
    config_override: Option<&std::path::Path>,
    appid: &str,
    title: &str,
    verbose: bool,
) -> Result<()> {
    use margo_config::{parse_config, WindowRule};

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
    let state = read_state_file()
        .map_err(|e| anyhow::anyhow!("cannot read margo state.json: {e}"))?;
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
    let day_g = tw.get("day_gamma_pct").and_then(|v| v.as_u64()).unwrap_or(0);
    let nig_g = tw.get("night_gamma_pct").and_then(|v| v.as_u64()).unwrap_or(0);

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
    println!(
        "  {dim}phase{r}   = {phase}    {dim}source{r} = {source}"
    );
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

fn cmd_config_errors() -> Result<()> {
    use margo_config::diagnostics::{ConfigDiagnostic, Severity};
    let state = read_state_file()
        .map_err(|e| anyhow::anyhow!("cannot read margo state.json: {e}"))?;
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

fn cmd_check_config(config_override: Option<&std::path::Path>) -> Result<()> {
    use margo_config::diagnostics::{ConfigDiagnostic, Severity};
    use std::collections::HashMap;

    let cfg_path = config_override
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| {
            std::path::PathBuf::from(format!(
                "{}/.config/margo/config.conf",
                std::env::var("HOME").unwrap_or_else(|_| ".".to_string())
            ))
        });

    if !cfg_path.exists() {
        eprintln!("error: config file not found: {}", cfg_path.display());
        std::process::exit(2);
    }

    // Pass 1 — structured validator. Picks up trailing/leading/doubled
    // commas in CSV-shaped values, missing `=`, unknown top-level keys,
    // unresolved include/source paths.
    let mut report = margo_config::validator::validate_config(Some(&cfg_path))
        .unwrap_or_else(|e| {
            eprintln!("error: validator could not read config: {e}");
            std::process::exit(2);
        });

    // Pass 2 — try to fully parse for the summary line. If the parser
    // itself bails (rare; it's permissive) record an E000 diagnostic
    // and keep going so the rest of the diagnostic stream still prints.
    let cfg = margo_config::parse_config(Some(&cfg_path)).ok();

    // Pass 3 — duplicate-bind and regex-pattern checks. These need
    // semantic understanding the line-walker doesn't have, so we keep
    // them here but emit structured `ConfigDiagnostic`s into the same
    // report stream as Pass 1 so the renderer is uniform.
    let text = std::fs::read_to_string(&cfg_path)?;
    let mut bind_seen: HashMap<(String, String), Vec<(usize, String)>> = HashMap::new();

    for (lineno_zero, raw) in text.lines().enumerate() {
        let lineno = lineno_zero + 1;
        let line = raw.trim_start();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some(eq_pos) = raw.find('=') else { continue };
        let key = raw[..eq_pos].trim();
        let val = raw[eq_pos + 1..].trim();

        if key == "bind" {
            let mut parts = val.splitn(3, ',');
            let mods = parts.next().unwrap_or("").trim().to_lowercase();
            let keysym = parts.next().unwrap_or("").trim().to_lowercase();
            if !mods.is_empty() && !keysym.is_empty() {
                bind_seen
                    .entry((mods, keysym))
                    .or_default()
                    .push((lineno, raw.to_string()));
            }
        }

        if key == "windowrule" || key == "layerrule" {
            for (k_raw, v_raw) in val.split(',').filter_map(|p| p.split_once(':')) {
                let k = k_raw.trim();
                if matches!(
                    k,
                    "appid"
                        | "app_id"
                        | "title"
                        | "exclude_appid"
                        | "exclude_id"
                        | "exclude_title"
                        | "not_appid"
                        | "not_title"
                        | "layer_name"
                ) && let Err(e) = regex::Regex::new(v_raw.trim())
                {
                    let pat_col = raw
                        .find(v_raw.trim())
                        .map(|i| i + 1)
                        .unwrap_or(eq_pos + 2);
                    report.push(ConfigDiagnostic {
                        path: cfg_path.clone(),
                        line: lineno,
                        col: pat_col,
                        end_col: pat_col + v_raw.trim().len(),
                        severity: Severity::Error,
                        code: "E004".into(),
                        message: format!("regex compile error in `{k}:{}` — {e}", v_raw.trim()),
                        line_text: raw.to_string(),
                    });
                }
            }
        }
    }

    for (key, hits) in &bind_seen {
        if hits.len() > 1 {
            // Report on the *later* line(s); the first one is the one
            // that's getting shadowed. niri-style: the diagnostic
            // points at the line that "won" so the user knows which
            // entry is the active one.
            let (lineno, line_text) = hits.last().unwrap();
            report.push(ConfigDiagnostic {
                path: cfg_path.clone(),
                line: *lineno,
                col: 1,
                end_col: line_text.len() + 1,
                severity: Severity::Warning,
                code: "W002".into(),
                message: format!(
                    "duplicate bind `{}+{}` (also defined on line{} {}; this definition wins)",
                    key.0,
                    key.1,
                    if hits.len() > 2 { "s" } else { "" },
                    hits.iter()
                        .take(hits.len() - 1)
                        .map(|(l, _)| l.to_string())
                        .collect::<Vec<_>>()
                        .join(", "),
                ),
                line_text: line_text.clone(),
            });
        }
    }

    // Stable render order: by line, then severity (errors first).
    report.diagnostics.sort_by(|a, b| {
        a.line
            .cmp(&b.line)
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
    if err_count > 0 {
        std::process::exit(1);
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
    use std::io::IsTerminal;
    let tty = std::io::stdout().is_terminal();
    let bold = if tty { "\x1b[1m" } else { "" };
    let dim = if tty { "\x1b[2m" } else { "" };
    let cyan = if tty { "\x1b[36m" } else { "" };
    let yellow = if tty { "\x1b[33m" } else { "" };
    let green = if tty { "\x1b[32m" } else { "" };
    let red = if tty { "\x1b[31m" } else { "" };
    let reset = if tty { "\x1b[0m" } else { "" };

    let out = &state.outputs[idx];

    // Header line — output name + active/inactive marker + layout symbol.
    let active_marker = if out.active {
        format!("{green}●{reset} ")
    } else {
        "  ".to_string()
    };
    println!(
        "{active_marker}{bold}{}{reset}  {dim}layout {reset}{cyan}{}{reset}",
        out.name, out.layout_symbol
    );

    // Focused window line.
    if out.appid.is_empty() && out.title.is_empty() {
        println!("    {dim}focused{reset}: (none)");
    } else {
        let mut flags = Vec::new();
        if out.fullscreen {
            flags.push(format!("{red}FULLSCREEN{reset}"));
        }
        if out.floating {
            flags.push(format!("{yellow}FLOAT{reset}"));
        }
        let flags_str = if flags.is_empty() {
            String::new()
        } else {
            format!("  {}", flags.join(" "))
        };
        println!(
            "    {dim}focused{reset}: {bold}{}{reset} · {}{flags_str}",
            out.appid, out.title
        );
        if out.width > 0 && out.height > 0 {
            println!(
                "    {dim}geometry{reset}: {}×{} @ {},{}",
                out.width, out.height, out.x, out.y
            );
        }
    }

    // Tags row — compact one-line summary.
    let mut row = String::new();
    for (i, tag) in out.tags.iter().enumerate().take(state.tag_count as usize) {
        let n = i + 1;
        let label = format!("{n}·{}", tag.clients);
        let cell = if tag.focused {
            format!("{green}[{label}]●{reset}")
        } else if tag.state == 1 {
            // active but not the focused tag — multi-tag-view case.
            format!("{cyan}[{label}]{reset}")
        } else if tag.state == 2 {
            format!("{red}{label}!{reset}")
        } else if tag.clients > 0 {
            label.to_string()
        } else {
            format!("{dim}{label}{reset}")
        };
        if !row.is_empty() {
            row.push(' ');
            row.push(' ');
        }
        row.push_str(&cell);
    }
    println!("    {dim}tags{reset}: {row}");
    println!();
    println!(
        "{dim}layouts:{reset} {}",
        state.layouts
            .iter()
            .enumerate()
            .map(|(i, l)| format!("{i}:{l}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
}

/// Render `mctl status` from the richer state.json snapshot —
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
    let clients = state["clients"].as_array().map(|v| v.as_slice()).unwrap_or(&[]);
    let layout_names: Vec<String> = state["layouts"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
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
        println!(
            "{active_marker}{bold}{name}{reset}  {dim}layout {reset}{cyan}{layout}{reset}"
        );

        // Focused window on this output (find from clients array).
        let mon_idx = out["x"].as_i64(); // unused — we match by name
        let _ = mon_idx;
        let focused = clients
            .iter()
            .find(|c| {
                c["focused"].as_bool() == Some(true)
                    && c["monitor"].as_str() == Some(name)
            });
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
            println!(
                "    {dim}focused{reset}: {bold}{app}{reset} · {title}{flags_str}"
            );
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

fn read_state_file() -> Result<serde_json::Value> {
    let path = state_file_path();
    let body = std::fs::read_to_string(&path).with_context(|| {
        format!(
            "read {}: margo writes this file on every layout/focus change. \
             Is margo running? (You can poke it with `pkill -USR1 margo` or \
             toggle a tag to force a refresh.)",
            path.display()
        )
    })?;
    let json: serde_json::Value = serde_json::from_str(&body)
        .with_context(|| format!("parse {}", path.display()))?;
    Ok(json)
}

fn state_file_path() -> std::path::PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            let uid = unsafe { libc::getuid() };
            std::path::PathBuf::from(format!("/run/user/{uid}"))
        });
    dir.join("margo").join("state.json")
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
        let app_disp = if app.len() > max_app {
            format!("{}…", &app[..max_app.saturating_sub(1)])
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

fn cmd_focused(json_out: bool) -> Result<()> {
    let state = read_state_file()?;
    let clients = state["clients"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("state file missing `clients` array"))?;
    let Some(focused) = clients.iter().find(|c| c["focused"].as_bool() == Some(true)) else {
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
