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
use std::collections::HashMap;
use std::path::Path;
use wasmtime::component::{Component, Linker};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiView};

wasmtime::component::bindgen!({
    path: "wit",
    world: "plugin",
});

// `Node` and `Event` are brought into scope by the world's `use types.{…}`;
// the enums + the host capability trait + its records need importing explicitly.
use margo::plugin::host::{Host, HttpRequest, HttpResponse};
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
    /// Values the user set for this plugin (declarative `[[setting]]` tier),
    /// exposed to the guest via `get-setting`.
    settings: HashMap<String, String>,
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

    fn get_setting(&mut self, key: String) -> String {
        self.settings.get(&key).cloned().unwrap_or_default()
    }

    fn notify(&mut self, summary: String, body: String) {
        // `--` guards against a guest-supplied summary that starts with `-`.
        if let Err(e) = std::process::Command::new("notify-send")
            .arg("--")
            .arg(&summary)
            .arg(&body)
            .spawn()
        {
            tracing::warn!(plugin = self.plugin_id, "notify failed: {e}");
        }
    }

    fn http(&mut self, req: HttpRequest) -> Result<HttpResponse, String> {
        host_http(req)
    }
}

/// Blocking one-shot HTTP, run on the host's behalf (the guest never touches
/// the network directly). W3 is synchronous; W4 replaces this with an async,
/// streaming path so token streams don't block the UI.
fn host_http(req: HttpRequest) -> Result<HttpResponse, String> {
    let method = if req.method.is_empty() {
        "GET"
    } else {
        req.method.as_str()
    };
    let mut request = ureq::request(method, &req.url);
    for (name, value) in &req.headers {
        request = request.set(name, value);
    }
    let result = if req.body.is_empty() {
        request.call()
    } else {
        request.send_string(&req.body)
    };
    match result {
        Ok(resp) => {
            let status = resp.status();
            let body = resp.into_string().map_err(|e| e.to_string())?;
            Ok(HttpResponse { status, body })
        }
        // A non-2xx status is still a real response (e.g. a 4xx JSON error from
        // an API) — hand it back to the guest rather than dropping it.
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            Ok(HttpResponse { status: code, body })
        }
        Err(e) => Err(e.to_string()),
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

    /// Instantiate a plugin component, ready to `view` / `update`. `settings`
    /// are the user's values for this plugin's declarative `[[setting]]`s,
    /// surfaced to the guest through the `get-setting` capability.
    pub fn instantiate(
        &self,
        plugin_id: &str,
        wasm_path: &Path,
        settings: HashMap<String, String>,
    ) -> Result<PluginInstance> {
        let component = Component::from_file(&self.engine, wasm_path)?;
        let mut linker = Linker::new(&self.engine);
        // wasip2 guests link the WASI std interfaces; provide them.
        wasmtime_wasi::add_to_linker_sync(&mut linker)?;
        Plugin::add_to_linker(&mut linker, |s: &mut HostState| s)?;
        let mut store = Store::new(
            &self.engine,
            HostState {
                plugin_id: plugin_id.to_string(),
                settings,
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
