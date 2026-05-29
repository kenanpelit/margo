//! Authoring SDK for **margo WASM plugins** (the mplugins WASM tier).
//!
//! The host/guest contract ([`wit/world.wit`]) speaks a *flat* node list — every
//! node names its children by id, because the component model has no recursive
//! types. Hand-writing that list is tedious and error-prone. This SDK lets you
//! build a normal **nested tree** with [`El`] and flattens it for you, and
//! re-exports the host [`host`] capabilities and the protocol [`Event`] type.
//!
//! ## Writing a plugin
//!
//! ```ignore
//! use mplugin_sdk::{export_component, host, Component, El, Event, EventKind};
//!
//! struct Hello;
//!
//! impl Component for Hello {
//!     fn view() -> El {
//!         El::vbox(vec![
//!             El::markdown("**hi** from a wasm plugin"),
//!             El::button("go", "Click me"),
//!         ])
//!     }
//!     fn update(ev: Event) -> El {
//!         if ev.kind == EventKind::Click {
//!             host::notify("hello", &format!("clicked {}", ev.id));
//!         }
//!         Self::view()
//!     }
//! }
//!
//! export_component!(Hello);
//! ```
//!
//! Build with `cargo build --target wasm32-wasip2 --release` and ship the
//! resulting `*.wasm` in your plugin folder (manifest `entry = "plugin.wasm"`).

wit_bindgen::generate!({
    world: "plugin",
    path: "wit",
    // Make the generated `export!` macro public so author crates can call it
    // (via `export_component!`).
    pub_export_macro: true,
});

// Surface the protocol types + the generated guest trait so authors (and the
// `export_component!` macro) can name them as `mplugin_sdk::…`.
pub use crate::exports::margo::plugin::guest::Guest;
pub use crate::margo::plugin::types::{Event, EventKind, Node, NodeKind};

/// Host capabilities — the sandbox boundary. Everything a plugin can reach
/// outside its own memory goes through one of these.
pub mod host {
    pub use crate::margo::plugin::host::{
        clipboard_read, copy, get_setting, http, http_start, log, notify, read_file, run,
        write_file, HttpRequest, HttpResponse, ProcessOutput,
    };
}

/// A node in a UI tree you build by nesting. Construct with the builders
/// ([`El::vbox`], [`El::markdown`], [`El::button`], …) and hand the root to
/// the framework via [`Component`]; the SDK flattens it to the wire format.
///
/// Interactive nodes ([`El::button`], [`El::entry`]) take an explicit `id` —
/// that id comes back on the [`Event`] they emit. Layout/leaf nodes get a
/// stable auto-id unless you set one with [`El::with_id`].
pub struct El {
    id: Option<String>,
    kind: NodeKind,
    text: String,
    children: Vec<El>,
    class: String,
    properties: Vec<(String, String)>,
}

impl El {
    fn leaf(kind: NodeKind, text: impl Into<String>) -> El {
        El {
            id: None,
            kind,
            text: text.into(),
            children: Vec::new(),
            class: String::new(),
            properties: Vec::new(),
        }
    }

    fn container(kind: NodeKind, children: Vec<El>) -> El {
        El {
            id: None,
            kind,
            text: String::new(),
            children,
            class: String::new(),
            properties: Vec::new(),
        }
    }

    /// A vertical stack.
    pub fn vbox(children: Vec<El>) -> El {
        Self::container(NodeKind::Vbox, children)
    }

    /// A horizontal row.
    pub fn hbox(children: Vec<El>) -> El {
        Self::container(NodeKind::Hbox, children)
    }

    /// A vertically-scrolling container — e.g. a chat log.
    pub fn scroll(children: Vec<El>) -> El {
        Self::container(NodeKind::Scroll, children)
    }

    /// A plain text label.
    pub fn label(text: impl Into<String>) -> El {
        Self::leaf(NodeKind::Label, text)
    }

    /// A markdown message bubble (bold / italic / `code`).
    pub fn markdown(text: impl Into<String>) -> El {
        Self::leaf(NodeKind::Markdown, text)
    }

    /// An image — either a freedesktop icon name (e.g. `audio-volume-high-symbolic`)
    /// or an absolute file path. The renderer picks the right widget.
    pub fn image(src: impl Into<String>) -> El {
        Self::leaf(NodeKind::Image, src)
    }

    /// A bool switch. `id` is echoed on toggle; `on` sets the initial state.
    pub fn switch(id: impl Into<String>, on: bool) -> El {
        El {
            id: Some(id.into()),
            kind: NodeKind::Switch,
            text: String::new(),
            children: Vec::new(),
            class: String::new(),
            properties: Vec::new(),
        }
        .prop("on", if on { "true" } else { "false" })
    }

    /// A numeric slider. `id` is echoed as `input` events whose `value` is the
    /// new number; `min`/`max`/`value`/`step` configure the range.
    pub fn slider(id: impl Into<String>, min: f64, max: f64, value: f64) -> El {
        El {
            id: Some(id.into()),
            kind: NodeKind::Slider,
            text: String::new(),
            children: Vec::new(),
            class: String::new(),
            properties: Vec::new(),
        }
        .prop("min", min.to_string())
        .prop("max", max.to_string())
        .prop("value", value.to_string())
    }

    /// A determinate progress bar (`0.0` … `1.0`).
    pub fn progress(fraction: f64) -> El {
        Self::leaf(NodeKind::Progress, "").prop("fraction", fraction.to_string())
    }

    /// A visual divider that respects the design language's spacing.
    pub fn separator() -> El {
        Self::leaf(NodeKind::Separator, "")
    }

    /// A button. `id` is echoed back on the click [`Event`].
    pub fn button(id: impl Into<String>, text: impl Into<String>) -> El {
        El {
            id: Some(id.into()),
            kind: NodeKind::Button,
            text: text.into(),
            children: Vec::new(),
            class: String::new(),
            properties: Vec::new(),
        }
    }

    /// A text entry. `id` is echoed back on its input/submit [`Event`]s; `text`
    /// is the initial value.
    pub fn entry(id: impl Into<String>, text: impl Into<String>) -> El {
        El {
            id: Some(id.into()),
            kind: NodeKind::Entry,
            text: text.into(),
            children: Vec::new(),
            class: String::new(),
            properties: Vec::new(),
        }
    }

    /// Pin this node's id (useful to keep a layout node stable across renders).
    pub fn with_id(mut self, id: impl Into<String>) -> El {
        self.id = Some(id.into());
        self
    }

    /// Add space-separated CSS class(es) so this node picks up the design
    /// language's styling (e.g. `plugin-hero`, `plugin-action`, `plugin-toggle`).
    pub fn class(mut self, class: impl Into<String>) -> El {
        self.class = class.into();
        self
    }

    /// Set one entry in the node's property bag. Layout (`padding`/`margin`/…),
    /// per-kind state (`on`/`fraction`/`value`), and any future extension goes
    /// through here — so adding a property never breaks pre-compiled plugins.
    pub fn prop(mut self, key: impl Into<String>, value: impl Into<String>) -> El {
        self.properties.push((key.into(), value.into()));
        self
    }

    // ── Layout shortcuts (set typed values via `prop`) ──────────────────────

    /// Padding around the node's content, in px.
    pub fn padding(self, px: i32) -> El {
        self.prop("padding", px.to_string())
    }

    /// Margin outside the node, in px.
    pub fn margin(self, px: i32) -> El {
        self.prop("margin", px.to_string())
    }

    /// Spacing between children (`vbox`/`hbox`/`scroll`), in px.
    pub fn spacing(self, px: i32) -> El {
        self.prop("spacing", px.to_string())
    }

    /// Horizontal alignment: `start | center | end | fill`.
    pub fn halign(self, align: impl Into<String>) -> El {
        self.prop("halign", align)
    }

    /// Vertical alignment: `start | center | end | fill`.
    pub fn valign(self, align: impl Into<String>) -> El {
        self.prop("valign", align)
    }

    /// Whether the node should expand horizontally to fill its parent.
    pub fn hexpand(self, expand: bool) -> El {
        self.prop("hexpand", if expand { "true" } else { "false" })
    }

    /// Whether the node should expand vertically to fill its parent.
    pub fn vexpand(self, expand: bool) -> El {
        self.prop("vexpand", if expand { "true" } else { "false" })
    }
}

/// Flatten a nested [`El`] tree into the protocol's flat node list, rooted at
/// `"root"`. Nodes without an explicit id get a stable auto id (`n1`, `n2`, …).
pub fn render(root: El) -> Vec<Node> {
    let mut out = Vec::new();
    let mut counter = 0usize;
    flatten(root, Some("root".to_string()), &mut counter, &mut out);
    out
}

fn flatten(el: El, forced_id: Option<String>, counter: &mut usize, out: &mut Vec<Node>) -> String {
    let id = forced_id.or(el.id).unwrap_or_else(|| {
        *counter += 1;
        format!("n{counter}")
    });
    let children: Vec<String> = el
        .children
        .into_iter()
        .map(|child| flatten(child, None, counter, out))
        .collect();
    out.push(Node {
        id: id.clone(),
        kind: el.kind,
        text: el.text,
        children,
        class: el.class,
        properties: el.properties,
    });
    id
}

/// Implement this for your plugin type, then call [`export_component!`]. You
/// return nested [`El`] trees; the SDK handles the wire format and the export
/// glue.
pub trait Component {
    /// Initial UI.
    fn view() -> El;
    /// Re-render after an [`Event`] (a click/submit, or a host-originated
    /// stream chunk).
    fn update(ev: Event) -> El;
}

/// Wire a [`Component`] up as the plugin's WASM exports. Call once at the top
/// level of your crate: `export_component!(MyPlugin);`.
#[macro_export]
macro_rules! export_component {
    ($component:ty) => {
        struct __MarGoPluginGuest;
        impl $crate::Guest for __MarGoPluginGuest {
            fn view() -> ::std::vec::Vec<$crate::Node> {
                $crate::render(<$component as $crate::Component>::view())
            }
            fn update(ev: $crate::Event) -> ::std::vec::Vec<$crate::Node> {
                $crate::render(<$component as $crate::Component>::update(ev))
            }
        }
        $crate::export!(__MarGoPluginGuest with_types_in $crate);
    };
}
