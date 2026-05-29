//! Renders a WASM plugin's UI tree into GTK and drives its event loop.

use gtk4 as gtk;
use gtk::prelude::*;
use mshell_plugin_host::{PluginInstance, PluginRuntime, UiEvent, UiEventKind, UiKind, UiNode};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

/// A live WASM-plugin surface: owns the instance and re-renders its UI tree
/// into a container whenever the guest returns a new tree.
pub struct PluginPanel {
    root: gtk::Box,
    inner: Rc<RefCell<Inner>>,
}

struct Inner {
    instance: PluginInstance,
    container: gtk::Box,
}

impl PluginPanel {
    /// Instantiate a plugin component and render its initial `view`.
    pub fn new(runtime: &PluginRuntime, plugin_id: &str, wasm_path: &Path) -> anyhow::Result<Self> {
        let instance = runtime.instantiate(plugin_id, wasm_path)?;
        let container = gtk::Box::new(gtk::Orientation::Vertical, 6);
        container.add_css_class("plugin-panel");
        let inner = Rc::new(RefCell::new(Inner {
            instance,
            container: container.clone(),
        }));

        let nodes = inner.borrow_mut().instance.view()?;
        render(&inner, nodes);

        Ok(Self {
            root: container,
            inner,
        })
    }

    /// The widget to embed in a panel surface.
    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    /// Re-run `view` and re-render — e.g. after the plugin's settings change.
    pub fn refresh(&self) -> anyhow::Result<()> {
        let nodes = self.inner.borrow_mut().instance.view()?;
        render(&self.inner, nodes);
        Ok(())
    }
}

/// Apply an event: ask the guest to `update`, then re-render the new tree.
fn dispatch(inner: &Rc<RefCell<Inner>>, event: UiEvent) {
    let result = inner.borrow_mut().instance.update(&event);
    match result {
        Ok(nodes) => render(inner, nodes),
        Err(e) => tracing::warn!("plugin update failed: {e}"),
    }
}

/// Rebuild the container from a flat node list (rooted at id "root").
fn render(inner: &Rc<RefCell<Inner>>, nodes: Vec<UiNode>) {
    // Clone the container handle and drop the borrow before building widgets —
    // their click closures re-borrow `inner` on later activation, which must
    // not overlap a live borrow held here.
    let container = inner.borrow().container.clone();
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
    let by_id: HashMap<&str, &UiNode> = nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    if let Some(root) = by_id.get("root") {
        container.append(&build(root, &by_id, inner));
    }
}

/// Build one node, recursing into children referenced by id.
fn build(node: &UiNode, by_id: &HashMap<&str, &UiNode>, inner: &Rc<RefCell<Inner>>) -> gtk::Widget {
    match node.kind {
        UiKind::VBox | UiKind::HBox => {
            let orient = if node.kind == UiKind::HBox {
                gtk::Orientation::Horizontal
            } else {
                gtk::Orientation::Vertical
            };
            let b = gtk::Box::new(orient, 6);
            for child_id in &node.children {
                if let Some(child) = by_id.get(child_id.as_str()) {
                    b.append(&build(child, by_id, inner));
                }
            }
            b.upcast()
        }
        UiKind::Label => {
            let label = gtk::Label::new(Some(&node.text));
            label.set_halign(gtk::Align::Start);
            label.set_wrap(true);
            label.upcast()
        }
        UiKind::Button => {
            let btn = gtk::Button::with_label(&node.text);
            let inner = inner.clone();
            let id = node.id.clone();
            btn.connect_clicked(move |_| {
                dispatch(
                    &inner,
                    UiEvent {
                        id: id.clone(),
                        kind: UiEventKind::Click,
                        value: String::new(),
                    },
                );
            });
            btn.upcast()
        }
        UiKind::Entry => {
            let entry = gtk::Entry::new();
            entry.set_text(&node.text);
            let inner = inner.clone();
            let id = node.id.clone();
            entry.connect_activate(move |e| {
                dispatch(
                    &inner,
                    UiEvent {
                        id: id.clone(),
                        kind: UiEventKind::Submit,
                        value: e.text().to_string(),
                    },
                );
            });
            entry.upcast()
        }
    }
}
