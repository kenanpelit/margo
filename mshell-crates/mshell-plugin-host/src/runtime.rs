//! W1 runtime: load a WebAssembly **component**, link the `log` host
//! capability, and call the guest's `run` export.
//!
//! NOTE: the wasmtime component-model wiring here is written against the
//! crate's current API but is **only compiled under `--features wasm`** (a
//! heavy build). Treat the exact binding names (`PluginImports`,
//! `Plugin::instantiate`, `call_run`) as provisional until a `--features wasm`
//! build confirms them; they may need small adjustments to track the pinned
//! wasmtime version. The `world.wit` contract is the stable part.

use anyhow::Result;
use std::path::Path;
use wasmtime::component::{Component, Linker};
use wasmtime::{Config, Engine, Store};

wasmtime::component::bindgen!({
    path: "wit",
    world: "plugin",
});

/// Host state handed to capability implementations.
struct HostState {
    plugin_id: String,
}

// World-level function imports are implemented on the host state.
impl PluginImports for HostState {
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

/// A wasmtime engine, reused across plugin loads.
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

    /// Instantiate a plugin component and call its `run` export. W1 smoke
    /// path — W2 swaps `run` for the `Ui`/event loop and a rendered panel.
    pub fn run(&self, plugin_id: &str, wasm_path: &Path) -> Result<()> {
        let component = Component::from_file(&self.engine, wasm_path)?;
        let mut linker = Linker::new(&self.engine);
        Plugin::add_to_linker(&mut linker, |s: &mut HostState| s)?;
        let mut store = Store::new(
            &self.engine,
            HostState {
                plugin_id: plugin_id.to_string(),
            },
        );
        let bindings = Plugin::instantiate(&mut store, &component, &linker)?;
        bindings.call_run(&mut store)?;
        Ok(())
    }
}
