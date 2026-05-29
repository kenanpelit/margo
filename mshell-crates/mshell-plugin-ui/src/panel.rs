//! Renders a WASM plugin's UI tree into GTK and drives its event loop.

use gtk4 as gtk;
use gtk::prelude::*;
use mshell_plugin_host::{PluginInstance, PluginRuntime, UiEvent, UiEventKind, UiKind, UiNode};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

/// The user's values for a plugin's declarative settings, handed to the guest
/// via the `get-setting` capability (API keys, model choices, …).
pub type PluginSettings = HashMap<String, String>;

/// A live WASM-plugin surface: owns the instance and re-renders its UI tree
/// into a container whenever the guest returns a new tree.
pub struct PluginPanel {
    root: gtk::Box,
    inner: Rc<RefCell<Inner>>,
}

struct Inner {
    instance: PluginInstance,
    container: gtk::Box,
    /// Whether a pump timeout is currently installed (so we don't stack them).
    pumping: bool,
}

impl PluginPanel {
    /// Instantiate a plugin component and render its initial `view`. `settings`
    /// are the user's values for the plugin's declarative `[[setting]]`s,
    /// surfaced to the guest through `get-setting`.
    pub fn new(
        runtime: &PluginRuntime,
        plugin_id: &str,
        wasm_path: &Path,
        settings: PluginSettings,
    ) -> anyhow::Result<Self> {
        let instance = runtime.instantiate(plugin_id, wasm_path, settings)?;
        let container = gtk::Box::new(gtk::Orientation::Vertical, 6);
        container.add_css_class("plugin-panel");

        // Ctrl+C anywhere in the panel copies text — robustly, since drag-select
        // + the focused-widget Ctrl+C path is unreliable in a layer-shell
        // surface. Capture phase so it fires before a focused entry consumes it.
        // If a label has an active selection, copy that; otherwise copy the
        // whole conversation (every label/markdown line).
        let key = gtk::EventControllerKey::new();
        key.set_propagation_phase(gtk::PropagationPhase::Capture);
        let copy_root = container.clone();
        key.connect_key_pressed(move |_, keyval, _, state| {
            if state.contains(gtk::gdk::ModifierType::CONTROL_MASK)
                && matches!(keyval, gtk::gdk::Key::c | gtk::gdk::Key::C)
            {
                let text = panel_copy_text(copy_root.upcast_ref());
                if !text.is_empty() {
                    if let Some(display) = gtk::gdk::Display::default() {
                        display.clipboard().set_text(&text);
                    }
                    return gtk::glib::Propagation::Stop;
                }
            }
            gtk::glib::Propagation::Proceed
        });
        container.add_controller(key);

        let inner = Rc::new(RefCell::new(Inner {
            instance,
            container: container.clone(),
            pumping: false,
        }));

        let nodes = inner.borrow_mut().instance.view()?;
        render(&inner, nodes);
        ensure_pump(&inner);

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
        ensure_pump(&self.inner);
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
    // The event may have kicked off an `http-start` stream — start draining it.
    ensure_pump(inner);
}

/// If the guest has an `http-start` stream in flight and no pump is running,
/// install a short glib timeout that drains response chunks into the guest's
/// `update` and re-renders, until the stream completes.
fn ensure_pump(inner: &Rc<RefCell<Inner>>) {
    {
        let mut guard = inner.borrow_mut();
        if guard.pumping || !guard.instance.streams_active() {
            return;
        }
        guard.pumping = true;
    }
    let inner = inner.clone();
    gtk::glib::timeout_add_local(Duration::from_millis(30), move || {
        let tree = match inner.borrow_mut().instance.pump() {
            Ok(tree) => tree,
            Err(e) => {
                tracing::warn!("plugin pump failed: {e}");
                None
            }
        };
        if let Some(nodes) = tree {
            render(&inner, nodes);
        }
        if inner.borrow().instance.streams_active() {
            return gtk::glib::ControlFlow::Continue;
        }
        // Stream done — flush any terminal chunks, then stop the timer.
        let tail = inner.borrow_mut().instance.pump().ok().flatten();
        if let Some(nodes) = tail {
            render(&inner, nodes);
        }
        inner.borrow_mut().pumping = false;
        gtk::glib::ControlFlow::Break
    });
}

/// Put `text` on the system clipboard (no-op if there's no display).
fn copy_to_clipboard(text: &str) {
    if let Some(display) = gtk::gdk::Display::default() {
        display.clipboard().set_text(text);
    }
}

/// Text to copy when Ctrl+C is pressed in a panel: the active selection in
/// any label if there is one, otherwise the whole conversation (every label's
/// text, joined by blank lines). Walks the widget tree under `root`.
fn panel_copy_text(root: &gtk::Widget) -> String {
    let mut labels = Vec::new();
    collect_labels(root, &mut labels);
    // 1) Prefer an active selection (user selected one bubble + Ctrl+C).
    for label in &labels {
        if let Some((a, b)) = label.selection_bounds() {
            let (a, b) = (a.min(b) as usize, a.max(b) as usize);
            let full = label.text();
            let sel: String = full.chars().skip(a).take(b - a).collect();
            if !sel.trim().is_empty() {
                return sel;
            }
        }
    }
    // 2) Fall back to the entire conversation.
    labels
        .iter()
        .map(|l| l.text().to_string())
        .filter(|t| !t.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Depth-first collect every [`gtk::Label`] descendant of `widget` (inclusive).
fn collect_labels(widget: &gtk::Widget, out: &mut Vec<gtk::Label>) {
    if let Some(label) = widget.downcast_ref::<gtk::Label>() {
        out.push(label.clone());
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        collect_labels(&c, out);
        child = c.next_sibling();
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
    let widget: gtk::Widget = match node.kind {
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
            label.set_selectable(true);
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
        UiKind::Scroll => {
            let vbox = gtk::Box::new(gtk::Orientation::Vertical, 6);
            for child_id in &node.children {
                if let Some(child) = by_id.get(child_id.as_str()) {
                    vbox.append(&build(child, by_id, inner));
                }
            }
            let scroller = gtk::ScrolledWindow::new();
            scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
            scroller.set_vexpand(true);
            // Kinetic scrolling steals the click-drag, so let it select text.
            scroller.set_kinetic_scrolling(false);
            // Floor the height: inside the menu's `propagate_natural_height`
            // scroll a `vexpand`-only child reports 0 and collapses.
            scroller.set_min_content_height(300);
            scroller.set_child(Some(&vbox));
            scroller.add_css_class("plugin-scroll");
            scroller.upcast()
        }
        UiKind::Markdown => {
            let label = gtk::Label::new(None);
            label.set_markup(&markdown_to_pango(&node.text));
            label.set_halign(gtk::Align::Start);
            label.set_xalign(0.0);
            label.set_wrap(true);
            label.set_selectable(true);
            label.add_css_class("plugin-markdown");
            // Right-click also copies the whole message (belt-and-suspenders).
            let gesture = gtk::GestureClick::new();
            gesture.set_button(gtk::gdk::BUTTON_SECONDARY);
            gesture.set_propagation_phase(gtk::PropagationPhase::Capture);
            let lbl = label.clone();
            gesture.connect_pressed(move |g, _, _, _| {
                copy_to_clipboard(&lbl.text());
                g.set_state(gtk::EventSequenceState::Claimed);
            });
            label.add_controller(gesture);

            // Hero/status markdown (e.g. mullvad's) stays plain; conversation
            // bubbles get a corner "copy" button — the reliable, obvious way to
            // copy a message in a layer-shell surface.
            if node.class.split_whitespace().any(|c| c == "plugin-hero") {
                label.upcast()
            } else {
                let copy = gtk::Button::from_icon_name("edit-copy-symbolic");
                copy.add_css_class("plugin-bubble-copy");
                copy.set_halign(gtk::Align::End);
                copy.set_valign(gtk::Align::Start);
                copy.set_tooltip_text(Some("Copy message"));
                let lbl = label.clone();
                copy.connect_clicked(move |_| copy_to_clipboard(&lbl.text()));

                let overlay = gtk::Overlay::new();
                overlay.set_child(Some(&label));
                overlay.add_overlay(&copy);
                overlay.upcast()
            }
        }
    };
    // Apply the plugin's design-language classes (plugin-hero, plugin-action,
    // plugin-toggle, …) so the panel can match the native widgets. The special
    // `plugin-expand` class also sets `hexpand` (CSS can't), so siblings in a
    // row share the width evenly.
    for class in node.class.split_whitespace() {
        widget.add_css_class(class);
        if class == "plugin-expand" {
            widget.set_hexpand(true);
        }
    }
    widget
}

/// Convert lightweight markdown to Pango markup for a `markdown` node. Handles
/// `` `code` ``, `**bold**`, and `*italic*`; everything else is escaped and
/// passed through. Unpaired markers are emitted literally (never panics).
fn markdown_to_pango(src: &str) -> String {
    let escaped = src
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    // Order matters: code first (so its contents aren't re-styled), then the
    // two-char `**` before the one-char `*`.
    let s = replace_paired(&escaped, "`", "<tt>", "</tt>");
    let s = replace_paired(&s, "**", "<b>", "</b>");
    replace_paired(&s, "*", "<i>", "</i>")
}

/// Wrap text between matched pairs of `marker` with `open`/`close`. An unpaired
/// trailing marker is left as-is.
fn replace_paired(s: &str, marker: &str, open: &str, close: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    loop {
        let Some(i) = rest.find(marker) else {
            out.push_str(rest);
            break;
        };
        let after = &rest[i + marker.len()..];
        let Some(j) = after.find(marker) else {
            out.push_str(rest);
            break;
        };
        out.push_str(&rest[..i]);
        out.push_str(open);
        out.push_str(&after[..j]);
        out.push_str(close);
        rest = &after[j + marker.len()..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::markdown_to_pango;

    #[test]
    fn renders_inline_styles() {
        assert_eq!(markdown_to_pango("**b**"), "<b>b</b>");
        assert_eq!(markdown_to_pango("*i*"), "<i>i</i>");
        assert_eq!(markdown_to_pango("`c`"), "<tt>c</tt>");
    }

    #[test]
    fn escapes_and_tolerates_unpaired() {
        assert_eq!(markdown_to_pango("a < b & c"), "a &lt; b &amp; c");
        assert_eq!(markdown_to_pango("lone * marker"), "lone * marker");
    }
}
