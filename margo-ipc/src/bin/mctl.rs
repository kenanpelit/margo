//! mctl — margo compositor control tool (replaces mmsg)
//!
//! Uses the zdwl_ipc_unstable_v2 Wayland protocol to query and control margo.
//! Connects to the compositor via the standard WAYLAND_DISPLAY socket.

use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use wayland_client::{
    globals::{registry_queue_init, GlobalListContents},
    protocol::{wl_output, wl_registry},
    Connection, Dispatch, EventQueue, QueueHandle,
};

use margo_ipc::protocols::dwl_ipc::{
    zdwl_ipc_manager_v2::{self, ZdwlIpcManagerV2},
    zdwl_ipc_output_v2::{self, ZdwlIpcOutputV2},
};

// ── CLI ───────────────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(
    name = "mctl",
    about = "margo compositor control",
    long_about = "Query and control the margo Wayland compositor via the dwl-ipc-v2 protocol."
)]
struct Args {
    /// Output to target (default: first / focused)
    #[arg(short, long)]
    output: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Dispatch a compositor command by name
    #[command(alias = "d")]
    Dispatch {
        /// Dispatch function name (e.g. focusstack, view, setlayout, killclient)
        name: String,
        /// Optional arguments (up to 5)
        #[arg(trailing_var_arg = true)]
        args: Vec<String>,
    },

    /// Set active tags on an output (bitmask)
    Tags {
        /// Tag bitmask (1-indexed, e.g. 1 for tag 1, 4 for tag 3)
        mask: u32,
        /// Toggle tagset (1 to toggle, 0 to set)
        #[arg(default_value = "0")]
        toggle: u32,
    },

    /// Set the focused client's tags
    ClientTags {
        /// AND mask
        and_mask: u32,
        /// XOR mask
        xor_mask: u32,
    },

    /// Set layout by index
    Layout {
        /// Layout index (0-based, matches compositor layout list)
        index: u32,
    },

    /// Quit the compositor
    Quit,

    /// Reload the compositor config
    Reload,

    /// Watch for state updates (runs until interrupted)
    Watch,

    /// Print current status (one shot)
    Status,
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
    }

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
