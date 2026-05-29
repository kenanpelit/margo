# mplugins WASM tier — design

The declarative tier (pill + label/exec + menus + settings) is feature-complete.
This document designs the **WASM tier**: sandboxed, in-shell plugin **UI**
(the assistant *panel*, dashboards, etc.) authored in Rust (or any language)
and compiled to WebAssembly.

> **Status:** design / not yet implemented. This is a multi-stage framework,
> not a one-shot change. Milestones below are built and shipped in order.

## Why WASM (not native, not Lua)

- Native `.so` plugins need a stable ABI margo doesn't have (gtk4/relm4/glib +
  rustc version lock) and run unsandboxed — rejected.
- WASM is **sandboxed** (the host grants only the capabilities it exposes),
  **language-agnostic** (Rust → wasm primarily), and ABI-stable via the
  **component model**. This is how authors get "real Rust plugins" safely.

## Architecture

```
plugin.wasm (guest, sandboxed)            mshell (host, GTK)
  exports:  init(settings)                 wasmtime component runtime
            update(event) -> Ui            renders Ui tree → GTK in a
  imports:  log / http / notify / …        layer-shell panel; routes events
```

- **Runtime:** `wasmtime` + the **component model**, typed interfaces in **WIT**.
- **UI model:** the guest returns a **declarative UI tree** (`Ui`) — a small node
  set: `vbox/hbox`, `label`, `button`, `entry`, `scroll`, `list`, `markdown`.
  The host renders it to GTK and **diffs** on each `update`. The host is the
  only thing that touches GTK; the guest never does.
- **Events:** GTK interactions (`click(id)`, `input(id, text)`, `submit(id)`)
  are delivered to the guest's `update(event)`, which returns a new `Ui`.
- **Capabilities (host imports = the sandbox boundary):**
  - `get_setting(key)` — reuse the declarative `[[setting]]` tier (API keys, …).
  - `http(request) -> stream` — outbound HTTP with **streaming** chunks
    (delivered back as repeated `update(StreamChunk)` events) for token streams.
  - `notify(summary, body)`, `log(level, msg)`, `clipboard`, `open(url)`.
  - Nothing else — no raw filesystem / process by default.
- **Surface:** the rendered tree lives in a layer-shell panel (same machinery
  as the existing menus), opened from the plugin's bar pill.
- **SDK:** an `mplugin-sdk` crate wraps the WIT bindings; authors
  `cargo build --target wasm32-wasip2` and ship `plugin.wasm` in the plugin
  folder. Manifest gains `entry = "plugin.wasm"` + `entryKind = "wasm"`.

## Milestones (built in order)

- **W1 — Runtime foundation:** ✅ `mshell-plugin-host` crate; loads a component,
  links a `log` host import, calls a guest export. wasmtime is feature-gated
  (`wasm`) so non-WASM builds stay lean. Verified: `--features wasm` compiles
  (wasmtime 27 component model).
- **W2 — UI model + view/update:** ✅ host/guest contract carries a flat node
  list (`vbox/hbox/label/button/entry`, children by id, rooted at "root");
  `view()` + `update(event)`. `mshell-plugin-host` exposes GTK-free `UiNode` /
  `UiEvent` + a `PluginInstance`. Verified to compile under `--features wasm`.
  *(Remaining for W2b: the GTK renderer + layer-shell panel in the frame, and a
  guest component to runtime-verify the loop end-to-end.)*
- **W3 — Capabilities:** `get_setting`, `notify`, non-streaming `http`.
- **W4 — Streaming + rich nodes:** streaming `http`, `entry`, scrollable `list`,
  `markdown` (message bubbles) — enough for a chat panel.
- **W5 — SDK + docs + real port:** `mplugin-sdk` crate, author guide, and port
  `assistant-panel`'s actual chat panel as the proving ground.

## Risks / open decisions

- **Build cost:** wasmtime is large (compile time + binary size). Feature-gate
  it so the default shell build is unaffected unless WASM plugins are enabled.
- **The UI protocol is the hard part** — the `Ui`/event/diff model is the core
  design; start minimal (W2) and grow node types only as a real plugin needs
  them (driven by the assistant-panel port).
- **Async/streaming across the boundary** — map host async (reqwest stream) to
  guest `update(StreamChunk)` events on the GTK main loop.
- **Component model vs hand-rolled ABI** — use the component model (typed WIT,
  first-class in wasmtime) over a brittle hand-rolled ABI.
- **Trust:** capabilities ARE the sandbox. Only expose vetted host functions;
  `http` egress is the main thing to surface to users at install/enable.

## Relationship to the declarative tier

WASM plugins **reuse** the declarative pieces: the same `[[setting]]` form (so
API keys/config use the existing UI + `0600` storage) and the same bar-pill
placement. WASM only adds the *panel UI* the declarative tier can't express.
