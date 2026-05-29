//! W2 runtime: load a component, link the `log` capability, and drive the UI
//! model — `view()` for the initial render and `update(event)` after each
//! interaction. Both return a flat node list (see `world.wit`).
//!
//! This module stays **GTK-free**: it exposes plain Rust types ([`UiNode`],
//! [`UiEvent`]) so the GTK renderer (in the shell frame) can consume them
//! without coupling wasmtime to gtk4.
//!
//! Compiled only under `--features wasm`.

use anyhow::Result;
use std::path::Path;
use wasmtime::component::{Component, Linker};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiView};

wasmtime::component::bindgen!({
    path: "wit",
    world: "plugin",
});

// `Node` and `Event` are brought into scope by the world's `use types.{…}`;
// the enums + the host capability trait need importing explicitly.
use margo::plugin::host::Host;
use margo::plugin::types::{EventKind, NodeKind};

// ── Public, GTK-free UI types ────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiKind {
    VBox,
    HBox,
    Label,
    Button,
    Entry,
}

/// One node of a guest-rendered UI. Children are referenced by id; the tree is
/// rebuilt by the renderer from the flat list, rooted at id `"root"`.
#[derive(Debug, Clone)]
pub struct UiNode {
    pub id: String,
    pub kind: UiKind,
    pub text: String,
    pub children: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiEventKind {
    Click,
    Input,
    Submit,
}

/// An interaction routed back to the guest's `update`.
#[derive(Debug, Clone)]
pub struct UiEvent {
    pub id: String,
    pub kind: UiEventKind,
    pub value: String,
}

// ── Host ─────────────────────────────────────────────────────────────────────

struct HostState {
    plugin_id: String,
    wasi: WasiCtx,
    table: ResourceTable,
}

impl WasiView for HostState {
    fn ctx(&mut self) -> &mut WasiCtx {
        &mut self.wasi
    }
    fn table(&mut self) -> &mut ResourceTable {
        &mut self.table
    }
}

impl Host for HostState {
    fn log(&mut self, level: u32, message: String) {
        let id = self.plugin_id.as_str();
        match level {
            0 => tracing::trace!(plugin = id, "{message}"),
            1 => tracing::debug!(plugin = id, "{message}"),
            3 => tracing::warn!(plugin = id, "{message}"),
            4 => tracing::error!(plugin = id, "{message}"),
            _ => tracing::info!(plugin = id, "{message}"),
        }
    }
}

// The `types` interface is types-only, but the generated linker bound still
// requires its (empty) host trait.
impl margo::plugin::types::Host for HostState {}

/// A wasmtime engine, reused across plugin instantiations.
pub struct PluginRuntime {
    engine: Engine,
}

impl PluginRuntime {
    pub fn new() -> Result<Self> {
        let mut config = Config::new();
        config.wasm_component_model(true);
        Ok(Self {
            engine: Engine::new(&config)?,
        })
    }

    /// Instantiate a plugin component, ready to `view` / `update`.
    pub fn instantiate(&self, plugin_id: &str, wasm_path: &Path) -> Result<PluginInstance> {
        let component = Component::from_file(&self.engine, wasm_path)?;
        let mut linker = Linker::new(&self.engine);
        // wasip2 guests link the WASI std interfaces; provide them.
        wasmtime_wasi::add_to_linker_sync(&mut linker)?;
        Plugin::add_to_linker(&mut linker, |s: &mut HostState| s)?;
        let mut store = Store::new(
            &self.engine,
            HostState {
                plugin_id: plugin_id.to_string(),
                wasi: WasiCtxBuilder::new().build(),
                table: ResourceTable::new(),
            },
        );
        let bindings = Plugin::instantiate(&mut store, &component, &linker)?;
        Ok(PluginInstance { store, bindings })
    }
}

/// A live plugin instance. Holds the wasm store, so it is single-threaded and
/// lives on the GTK main thread alongside its rendered surface.
pub struct PluginInstance {
    store: Store<HostState>,
    bindings: Plugin,
}

impl PluginInstance {
    /// Initial render.
    pub fn view(&mut self) -> Result<Vec<UiNode>> {
        let nodes = self.bindings.call_view(&mut self.store)?;
        Ok(nodes.into_iter().map(to_ui_node).collect())
    }

    /// Re-render after an interaction.
    pub fn update(&mut self, event: &UiEvent) -> Result<Vec<UiNode>> {
        let nodes = self.bindings.call_update(&mut self.store, &to_wit_event(event))?;
        Ok(nodes.into_iter().map(to_ui_node).collect())
    }
}

// ── Conversions between the generated component types and the public types ───

fn to_ui_node(n: Node) -> UiNode {
    UiNode {
        id: n.id,
        kind: match n.kind {
            NodeKind::Vbox => UiKind::VBox,
            NodeKind::Hbox => UiKind::HBox,
            NodeKind::Label => UiKind::Label,
            NodeKind::Button => UiKind::Button,
            NodeKind::Entry => UiKind::Entry,
        },
        text: n.text,
        children: n.children,
    }
}

fn to_wit_event(e: &UiEvent) -> Event {
    Event {
        id: e.id.clone(),
        kind: match e.kind {
            UiEventKind::Click => EventKind::Click,
            UiEventKind::Input => EventKind::Input,
            UiEventKind::Submit => EventKind::Submit,
        },
        value: e.value.clone(),
    }
}
