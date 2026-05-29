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
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{mpsc, Arc};
use wasmtime::component::{Component, Linker};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiView};

wasmtime::component::bindgen!({
    path: "wit",
    world: "plugin",
});

// The protocol types live in the `types` interface; the host capability trait
// + its records in `host`.
use margo::plugin::host::{Host, HttpRequest, HttpResponse, ProcessOutput};
use margo::plugin::types::{Event, EventKind, Node, NodeKind};

// ── Public, GTK-free UI types ────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiKind {
    VBox,
    HBox,
    Label,
    Button,
    Entry,
    /// Vertically-scrolling container for its children (e.g. a chat log).
    Scroll,
    /// Markdown-rendered label, styled as a message bubble.
    Markdown,
    /// File path or freedesktop icon name → `gtk::Image` / `gtk::Picture`.
    Image,
    /// `gtk::Switch`. State in `properties["on"]`; toggle echoes a click.
    Switch,
    /// `gtk::Scale`. `min`/`max`/`value`/`step` in `properties`.
    Slider,
    /// `gtk::ProgressBar`. `properties["fraction"]` is 0.0–1.0.
    Progress,
    /// `gtk::Separator`.
    Separator,
    /// `gtk::Grid`. `properties["columns"]` sets the column count.
    Grid,
    /// `gtk::Revealer` wrapping one child; `properties["revealed"]` toggles.
    Revealer,
    /// `gtk::Stack`; `properties["visible-child"]` is the active child id.
    Stack,
}

/// One node of a guest-rendered UI. Children are referenced by id; the tree is
/// rebuilt by the renderer from the flat list, rooted at id `"root"`.
#[derive(Debug, Clone)]
pub struct UiNode {
    pub id: String,
    pub kind: UiKind,
    pub text: String,
    pub children: Vec<String>,
    /// Space-separated CSS classes to add to the rendered widget.
    pub class: String,
    /// Extensible property bag (layout + per-kind knobs). See `world.wit`.
    pub properties: HashMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiEventKind {
    Click,
    Input,
    Submit,
    /// A chunk of an `http-start` response body (host-originated).
    StreamChunk,
    /// An `http-start` stream completed (host-originated).
    StreamEnd,
}

/// An interaction routed back to the guest's `update`.
#[derive(Debug, Clone)]
pub struct UiEvent {
    pub id: String,
    pub kind: UiEventKind,
    pub value: String,
}

// ── Host ─────────────────────────────────────────────────────────────────────

/// A piece of an in-flight `http-start` response, sent from the worker thread
/// to the main loop's pump.
struct StreamMsg {
    /// The request id `http-start` returned.
    req_id: String,
    /// A body chunk (or an `error: …` message); empty on the terminal message.
    chunk: String,
    /// True for the final message of a stream.
    done: bool,
}

struct HostState {
    plugin_id: String,
    /// Values the user set for this plugin (declarative `[[setting]]` tier),
    /// exposed to the guest via `get-setting`.
    settings: HashMap<String, String>,
    /// Per-plugin data dir for `read-file`/`write-file`. Scoped so the guest
    /// can't escape it via `..`. Created lazily on first write.
    data_dir: PathBuf,
    /// Worker threads send response chunks here; `PluginInstance::pump` drains
    /// the matching receiver on the UI thread.
    stream_tx: Sender<StreamMsg>,
    /// Count of in-flight `http-start` requests — the pump keeps running while
    /// this is non-zero.
    inflight: Arc<AtomicUsize>,
    /// Monotonic source of `http-start` request ids.
    next_req: AtomicU64,
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

    fn copy(&mut self, text: String) {
        use std::io::Write as _;
        use std::process::{Command, Stdio};
        match Command::new("wl-copy").stdin(Stdio::piped()).spawn() {
            Ok(mut child) => {
                if let Some(mut stdin) = child.stdin.take() {
                    let _ = stdin.write_all(text.as_bytes());
                }
                // stdin dropped → wl-copy reads EOF + forks to hold the
                // selection; its front process exits quickly, so reap it.
                let _ = child.wait();
            }
            Err(e) => tracing::warn!(plugin = self.plugin_id, "copy failed: {e}"),
        }
    }

    fn run(&mut self, program: String, args: Vec<String>) -> ProcessOutput {
        match std::process::Command::new(&program).args(&args).output() {
            Ok(out) => ProcessOutput {
                stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
                code: out.status.code().unwrap_or(-1),
            },
            Err(e) => {
                tracing::warn!(plugin = self.plugin_id, "run `{program}` failed: {e}");
                ProcessOutput {
                    stdout: String::new(),
                    stderr: e.to_string(),
                    code: -1,
                }
            }
        }
    }

    fn http(&mut self, req: HttpRequest) -> Result<HttpResponse, String> {
        host_http(req)
    }

    fn clipboard_read(&mut self) -> String {
        match std::process::Command::new("wl-paste").arg("-n").output() {
            Ok(out) if out.status.success() => {
                String::from_utf8_lossy(&out.stdout).into_owned()
            }
            Ok(_) => String::new(),
            Err(e) => {
                tracing::warn!(plugin = self.plugin_id, "clipboard-read failed: {e}");
                String::new()
            }
        }
    }

    fn read_file(&mut self, rel_path: String) -> Result<Vec<u8>, String> {
        let path = resolve_scoped(&self.data_dir, &rel_path)?;
        std::fs::read(&path).map_err(|e| e.to_string())
    }

    fn write_file(&mut self, rel_path: String, bytes: Vec<u8>) -> Result<(), String> {
        let path = resolve_scoped(&self.data_dir, &rel_path)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        // Atomic-ish: write tmp + rename, so a crash mid-write doesn't corrupt
        // the existing file.
        let tmp = path.with_extension("mplugin-tmp");
        std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn process_start(&mut self, program: String, args: Vec<String>) -> String {
        let req_id = format!("p{}", self.next_req.fetch_add(1, Ordering::Relaxed));
        let tx = self.stream_tx.clone();
        let inflight = self.inflight.clone();
        let plugin = self.plugin_id.clone();
        inflight.fetch_add(1, Ordering::SeqCst);
        let id = req_id.clone();
        std::thread::spawn(move || {
            host_process_stream(&id, &plugin, &program, &args, &tx);
            inflight.fetch_sub(1, Ordering::SeqCst);
        });
        req_id
    }

    fn http_start(&mut self, req: HttpRequest) -> String {
        let req_id = format!("r{}", self.next_req.fetch_add(1, Ordering::Relaxed));
        let tx = self.stream_tx.clone();
        let inflight = self.inflight.clone();
        inflight.fetch_add(1, Ordering::SeqCst);
        let id = req_id.clone();
        // The blocking read runs off the UI thread; chunks flow back through the
        // channel to `pump`. The id lets the guest correlate chunks to requests.
        std::thread::spawn(move || {
            host_http_stream(&id, req, &tx);
            inflight.fetch_sub(1, Ordering::SeqCst);
        });
        req_id
    }
}

/// Stream a response body to the pump as `StreamMsg` chunks, then a terminal
/// `done` message. Runs on a worker thread.
fn host_http_stream(req_id: &str, req: HttpRequest, tx: &Sender<StreamMsg>) {
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
    // Both 2xx and HTTP-error statuses carry a readable body; only a transport
    // error has none.
    let reader = match result {
        Ok(resp) => resp.into_reader(),
        Err(ureq::Error::Status(_, resp)) => resp.into_reader(),
        Err(e) => {
            let _ = tx.send(StreamMsg {
                req_id: req_id.to_string(),
                chunk: format!("error: {e}"),
                done: true,
            });
            return;
        }
    };
    let mut reader = reader;
    let mut buf = [0u8; 4096];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                let chunk = String::from_utf8_lossy(&buf[..n]).into_owned();
                if tx
                    .send(StreamMsg {
                        req_id: req_id.to_string(),
                        chunk,
                        done: false,
                    })
                    .is_err()
                {
                    return; // pump (and its instance) is gone — stop.
                }
            }
            Err(_) => break,
        }
    }
    let _ = tx.send(StreamMsg {
        req_id: req_id.to_string(),
        chunk: String::new(),
        done: true,
    });
}

/// Stream a subprocess's stdout to the pump as `StreamMsg` chunks, then a
/// terminal `done` message. Runs on a worker thread; the child is dropped
/// (and so SIGTERM'd on most systems) when this scope exits.
fn host_process_stream(
    req_id: &str,
    plugin: &str,
    program: &str,
    args: &[String],
    tx: &Sender<StreamMsg>,
) {
    use std::process::{Command, Stdio};
    let mut child = match Command::new(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(plugin, "process-start `{program}` failed: {e}");
            let _ = tx.send(StreamMsg {
                req_id: req_id.to_string(),
                chunk: format!("error: {e}"),
                done: true,
            });
            return;
        }
    };
    let Some(stdout) = child.stdout.take() else {
        let _ = tx.send(StreamMsg {
            req_id: req_id.to_string(),
            chunk: String::new(),
            done: true,
        });
        return;
    };
    let mut reader = stdout;
    let mut buf = [0u8; 4096];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                let chunk = String::from_utf8_lossy(&buf[..n]).into_owned();
                if tx
                    .send(StreamMsg {
                        req_id: req_id.to_string(),
                        chunk,
                        done: false,
                    })
                    .is_err()
                {
                    let _ = child.kill();
                    return; // pump (and its instance) is gone — stop.
                }
            }
            Err(_) => break,
        }
    }
    let _ = child.wait();
    let _ = tx.send(StreamMsg {
        req_id: req_id.to_string(),
        chunk: String::new(),
        done: true,
    });
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

/// Resolve `rel_path` against `root` and reject any traversal: rejects empty,
/// absolute, or `..`-bearing paths. The returned path is always inside `root`.
fn resolve_scoped(root: &Path, rel_path: &str) -> Result<PathBuf, String> {
    if rel_path.is_empty() {
        return Err("path is empty".to_string());
    }
    let candidate = Path::new(rel_path);
    if candidate.is_absolute() {
        return Err("absolute paths are not allowed".to_string());
    }
    for component in candidate.components() {
        match component {
            std::path::Component::Normal(_) => {}
            _ => return Err(format!("disallowed path component in `{rel_path}`")),
        }
    }
    Ok(root.join(candidate))
}

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
        let (stream_tx, stream_rx) = mpsc::channel::<StreamMsg>();
        let inflight = Arc::new(AtomicUsize::new(0));
        let data_dir = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("mshell")
            .join("plugins")
            .join(plugin_id);
        let mut store = Store::new(
            &self.engine,
            HostState {
                plugin_id: plugin_id.to_string(),
                settings,
                data_dir,
                stream_tx,
                inflight: inflight.clone(),
                next_req: AtomicU64::new(0),
                wasi: WasiCtxBuilder::new().build(),
                table: ResourceTable::new(),
            },
        );
        let bindings = Plugin::instantiate(&mut store, &component, &linker)?;
        Ok(PluginInstance {
            store,
            bindings,
            stream_rx,
            inflight,
        })
    }
}

/// A live plugin instance. Holds the wasm store, so it is single-threaded and
/// lives on the GTK main thread alongside its rendered surface.
pub struct PluginInstance {
    store: Store<HostState>,
    bindings: Plugin,
    /// Response chunks from `http-start` worker threads.
    stream_rx: Receiver<StreamMsg>,
    /// In-flight `http-start` count, shared with the workers.
    inflight: Arc<AtomicUsize>,
}

impl PluginInstance {
    /// Initial render.
    pub fn view(&mut self) -> Result<Vec<UiNode>> {
        let nodes = self
            .bindings
            .margo_plugin_guest()
            .call_view(&mut self.store)?;
        Ok(nodes.into_iter().map(to_ui_node).collect())
    }

    /// Re-render after an interaction.
    pub fn update(&mut self, event: &UiEvent) -> Result<Vec<UiNode>> {
        let nodes = self
            .bindings
            .margo_plugin_guest()
            .call_update(&mut self.store, &to_wit_event(event))?;
        Ok(nodes.into_iter().map(to_ui_node).collect())
    }

    /// Whether any `http-start` request is still running. The renderer keeps
    /// pumping while this is true.
    pub fn streams_active(&self) -> bool {
        self.inflight.load(Ordering::SeqCst) > 0
    }

    /// Drain all response chunks delivered since the last call, feeding each to
    /// the guest's `update` as a `stream-chunk`/`stream-end` event. Returns the
    /// **last** re-rendered tree (the guest accumulates state, so earlier trees
    /// are superseded), or `None` if nothing was pending.
    ///
    /// Call this from the UI main loop (e.g. a short glib timeout) while
    /// [`streams_active`](Self::streams_active) holds — and once more after it
    /// clears, to flush the terminal chunks.
    pub fn pump(&mut self) -> Result<Option<Vec<UiNode>>> {
        let mut last = None;
        while let Ok(msg) = self.stream_rx.try_recv() {
            let kind = if msg.done {
                UiEventKind::StreamEnd
            } else {
                UiEventKind::StreamChunk
            };
            let event = UiEvent {
                id: msg.req_id,
                kind,
                value: msg.chunk,
            };
            last = Some(self.update(&event)?);
        }
        Ok(last)
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
            NodeKind::Scroll => UiKind::Scroll,
            NodeKind::Markdown => UiKind::Markdown,
            NodeKind::Image => UiKind::Image,
            NodeKind::Switch => UiKind::Switch,
            NodeKind::Slider => UiKind::Slider,
            NodeKind::Progress => UiKind::Progress,
            NodeKind::Separator => UiKind::Separator,
            NodeKind::Grid => UiKind::Grid,
            NodeKind::Revealer => UiKind::Revealer,
            NodeKind::Stack => UiKind::Stack,
        },
        text: n.text,
        children: n.children,
        class: n.class,
        properties: n.properties.into_iter().collect(),
    }
}

fn to_wit_event(e: &UiEvent) -> Event {
    Event {
        id: e.id.clone(),
        kind: match e.kind {
            UiEventKind::Click => EventKind::Click,
            UiEventKind::Input => EventKind::Input,
            UiEventKind::Submit => EventKind::Submit,
            UiEventKind::StreamChunk => EventKind::StreamChunk,
            UiEventKind::StreamEnd => EventKind::StreamEnd,
        },
        value: e.value.clone(),
    }
}
