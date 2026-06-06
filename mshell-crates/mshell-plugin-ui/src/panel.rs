//! Renders a WASM plugin's UI tree into GTK and drives its event loop.

use gtk::prelude::*;
use gtk4 as gtk;
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

        // When the panel becomes visible (menu shown), focus the first entry
        // so the user can start typing immediately — the assistant chat case
        // is the obvious one, but every entry-bearing plugin wants this.
        // Done only on `map` (not on every re-render) so a button click in the
        // middle of a session doesn't yank focus back to the entry.
        let focus_root = container.clone();
        container.connect_map(move |_| {
            let Some(entry) = find_first_entry(focus_root.upcast_ref()) else {
                return;
            };
            // Defer until after GTK finishes mapping — focusing inside the
            // map handler itself is too early for the focus chain to settle.
            gtk::glib::idle_add_local_once(move || {
                entry.grab_focus();
            });
        });

        Ok(Self {
            root: container,
            inner,
        })
    }

    /// The widget to embed in a panel surface.
    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    /// Inject a host-originated event (e.g. a fired keybind) into the guest,
    /// then re-render. Used by the frame's `FirePluginKeybind` handler so a
    /// registered hotkey reaches the plugin's `update` regardless of whether
    /// the panel is currently visible.
    pub fn fire_event(&self, event: UiEvent) {
        dispatch(&self.inner, event);
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

/// Apply the node's property bag (layout knobs the renderer reads — see
/// `world.wit`) to a built widget. Unknown keys are ignored; per-kind keys
/// like `on`/`fraction` are consumed by their own arms above and harmless
/// here.
fn apply_node_properties(widget: &gtk::Widget, props: &HashMap<String, String>) {
    if let Some(px) = props.get("padding").and_then(|s| s.parse::<i32>().ok()) {
        widget.set_margin_start(px);
        widget.set_margin_end(px);
        widget.set_margin_top(px);
        widget.set_margin_bottom(px);
    }
    if let Some(px) = props.get("margin").and_then(|s| s.parse::<i32>().ok()) {
        widget.set_margin_start(px);
        widget.set_margin_end(px);
        widget.set_margin_top(px);
        widget.set_margin_bottom(px);
    }
    if let Some(align) = props.get("halign").and_then(|s| parse_align(s)) {
        widget.set_halign(align);
    }
    if let Some(align) = props.get("valign").and_then(|s| parse_align(s)) {
        widget.set_valign(align);
    }
    if let Some(on) = props.get("hexpand").map(|s| s == "true") {
        widget.set_hexpand(on);
    }
    if let Some(on) = props.get("vexpand").map(|s| s == "true") {
        widget.set_vexpand(on);
    }
    if let Some(px) = props.get("width").and_then(|s| s.parse::<i32>().ok()) {
        widget.set_size_request(px, widget.height_request());
    }
    if let Some(px) = props.get("height").and_then(|s| s.parse::<i32>().ok()) {
        widget.set_size_request(widget.width_request(), px);
    }
    if let Some(px) = props.get("spacing").and_then(|s| s.parse::<i32>().ok()) {
        if let Some(b) = widget.downcast_ref::<gtk::Box>() {
            b.set_spacing(px);
        }
    }
}

/// Accept hex colour strings in either web (`#rrggbb` / `#rrggbbaa`) or
/// margo's compositor-conf (`0xrrggbbaa`) form. Returns `(r, g, b, a)` in
/// the unit range; bogus input → opaque black so the swatch still renders.
fn parse_hex_rgba(input: &str) -> (f64, f64, f64, f64) {
    let s = input.trim();
    let hex = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix('#'))
        .unwrap_or(s);
    let bytes: Vec<u8> = (0..hex.len())
        .step_by(2)
        .filter_map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok())
        .collect();
    let to_unit = |b: u8| b as f64 / 255.0;
    match bytes.len() {
        3 => (to_unit(bytes[0]), to_unit(bytes[1]), to_unit(bytes[2]), 1.0),
        4 => (
            to_unit(bytes[0]),
            to_unit(bytes[1]),
            to_unit(bytes[2]),
            to_unit(bytes[3]),
        ),
        _ => (0.0, 0.0, 0.0, 1.0),
    }
}

fn parse_align(s: &str) -> Option<gtk::Align> {
    match s {
        "start" => Some(gtk::Align::Start),
        "center" => Some(gtk::Align::Center),
        "end" => Some(gtk::Align::End),
        "fill" => Some(gtk::Align::Fill),
        _ => None,
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

/// Depth-first find the first [`gtk::Entry`] descendant of `widget`.
fn find_first_entry(widget: &gtk::Widget) -> Option<gtk::Entry> {
    if let Some(entry) = widget.downcast_ref::<gtk::Entry>() {
        return Some(entry.clone());
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        if let Some(found) = find_first_entry(&c) {
            return Some(found);
        }
        child = c.next_sibling();
    }
    None
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
            let btn = gtk::Button::new();
            // Plugins own their button design via their own classes
            // (`plugin-action`/`-primary`/`-danger`, `plugin-tile`,
            // `plugin-panel-action`, `plugin-segment`, … in `_plugins.scss`) —
            // a coherent kind system: filled-primary vs flat-secondary actions,
            // card tiles, icon pills. We deliberately do NOT inject the shell's
            // own `.ok-button-surface`/`.ok-button-cell` here: that flattened
            // the plugin's primary/secondary hierarchy and pinned an 84px floor
            // onto compact icon buttons. The author classes do the styling.
            //
            // `properties["icon"]` lets a plugin author build icon-only or
            // leading-icon buttons (e.g. the §12 circular refresh action).
            // text-only stays the default.
            match (
                node.properties.get("icon").map(String::as_str),
                node.text.is_empty(),
            ) {
                (Some(icon), true) => btn.set_icon_name(icon),
                (Some(icon), false) => {
                    let h = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                    h.append(&gtk::Image::from_icon_name(icon));
                    h.append(&gtk::Label::new(Some(&node.text)));
                    btn.set_child(Some(&h));
                }
                (None, _) => btn.set_label(&node.text),
            }
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
        UiKind::Image => {
            let src = node.text.as_str();
            // Filesystem paths render as a Picture; everything else (icon names
            // like `audio-volume-high-symbolic`) goes through the icon theme.
            if src.starts_with('/') || src.starts_with("./") || src.starts_with("../") {
                let pic = gtk::Picture::for_filename(src);
                pic.set_can_shrink(true);
                // `properties["fit"]` chooses how the image fills its area —
                // mirrors `gtk::ContentFit`. Defaults to `contain` (aspect-fit).
                pic.set_content_fit(
                    match node
                        .properties
                        .get("fit")
                        .map(String::as_str)
                        .unwrap_or("contain")
                    {
                        "fill" => gtk::ContentFit::Fill,
                        "cover" => gtk::ContentFit::Cover,
                        "scale-down" => gtk::ContentFit::ScaleDown,
                        _ => gtk::ContentFit::Contain,
                    },
                );
                pic.upcast()
            } else {
                let img = gtk::Image::from_icon_name(src);
                img.add_css_class("plugin-image");
                img.upcast()
            }
        }
        UiKind::Switch => {
            let sw = gtk::Switch::new();
            let on = node
                .properties
                .get("on")
                .map(|s| s == "true")
                .unwrap_or(false);
            sw.set_active(on);
            sw.set_valign(gtk::Align::Center);
            let inner = inner.clone();
            let id = node.id.clone();
            sw.connect_state_set(move |_, new_state| {
                dispatch(
                    &inner,
                    UiEvent {
                        id: id.clone(),
                        kind: UiEventKind::Click,
                        value: new_state.to_string(),
                    },
                );
                gtk::glib::Propagation::Proceed
            });
            sw.upcast()
        }
        UiKind::Slider => {
            let min: f64 = node
                .properties
                .get("min")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0.0);
            let max: f64 = node
                .properties
                .get("max")
                .and_then(|s| s.parse().ok())
                .unwrap_or(100.0);
            let step: f64 = node
                .properties
                .get("step")
                .and_then(|s| s.parse().ok())
                .unwrap_or(1.0);
            let value: f64 = node
                .properties
                .get("value")
                .and_then(|s| s.parse().ok())
                .unwrap_or(min);
            let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, min, max, step);
            scale.set_value(value);
            scale.set_hexpand(true);
            scale.set_draw_value(false);
            let inner = inner.clone();
            let id = node.id.clone();
            scale.connect_value_changed(move |s| {
                dispatch(
                    &inner,
                    UiEvent {
                        id: id.clone(),
                        kind: UiEventKind::Input,
                        value: format!("{:.4}", s.value()),
                    },
                );
            });
            scale.upcast()
        }
        UiKind::Progress => {
            let bar = gtk::ProgressBar::new();
            let f: f64 = node
                .properties
                .get("fraction")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0.0);
            bar.set_fraction(f.clamp(0.0, 1.0));
            bar.set_hexpand(true);
            bar.upcast()
        }
        UiKind::Separator => gtk::Separator::new(gtk::Orientation::Horizontal).upcast(),
        UiKind::Grid => {
            let cols: i32 = node
                .properties
                .get("columns")
                .and_then(|s| s.parse().ok())
                .unwrap_or(2)
                .max(1);
            let grid = gtk::Grid::new();
            grid.set_row_spacing(6);
            grid.set_column_spacing(6);
            // Equal-width columns that fill the grid's width, so tile grids
            // (sounds, techniques, …) span the panel instead of hugging the
            // left. Children that opt into `hexpand` also stretch their cell.
            grid.set_column_homogeneous(true);
            for (i, child_id) in node.children.iter().enumerate() {
                if let Some(child) = by_id.get(child_id.as_str()) {
                    let row = (i as i32) / cols;
                    let col = (i as i32) % cols;
                    grid.attach(&build(child, by_id, inner), col, row, 1, 1);
                }
            }
            grid.upcast()
        }
        UiKind::Revealer => {
            let revealer = gtk::Revealer::new();
            let revealed = node
                .properties
                .get("revealed")
                .map(|s| s == "true")
                .unwrap_or(true);
            let transition = match node
                .properties
                .get("transition")
                .map(String::as_str)
                .unwrap_or("crossfade")
            {
                "slide-down" => gtk::RevealerTransitionType::SlideDown,
                "slide-up" => gtk::RevealerTransitionType::SlideUp,
                "slide-left" => gtk::RevealerTransitionType::SlideLeft,
                "slide-right" => gtk::RevealerTransitionType::SlideRight,
                "none" => gtk::RevealerTransitionType::None,
                _ => gtk::RevealerTransitionType::Crossfade,
            };
            revealer.set_transition_type(transition);
            revealer.set_reveal_child(revealed);
            if let Some(child_id) = node.children.first() {
                if let Some(child) = by_id.get(child_id.as_str()) {
                    revealer.set_child(Some(&build(child, by_id, inner)));
                }
            }
            revealer.upcast()
        }
        UiKind::Stack => {
            let stack = gtk::Stack::new();
            stack.set_transition_type(gtk::StackTransitionType::Crossfade);
            for child_id in &node.children {
                if let Some(child) = by_id.get(child_id.as_str()) {
                    stack.add_named(&build(child, by_id, inner), Some(child_id));
                }
            }
            if let Some(visible) = node.properties.get("visible-child") {
                stack.set_visible_child_name(visible);
            }
            stack.upcast()
        }
        UiKind::Extended => {
            // The extension's actual identity lives in `properties["kind"]` —
            // dispatch on that. Unknown extensions render as an empty vbox so
            // a future host with new kinds doesn't crash older renderers; the
            // pre-compiled plugin just sees a transparent placeholder.
            let kind = node
                .properties
                .get("kind")
                .map(String::as_str)
                .unwrap_or("");
            let inner_widget: gtk::Widget = match kind {
                // A filled-rectangle colour swatch driven by `properties["color"]`
                // (hex `#rrggbb` / `#rrggbbaa`, or margo's `0xrrggbbaa`) and an
                // optional `properties["size"]`. Used by the colour-scheme
                // editor + any plugin that wants to render arbitrary user
                // colours without piercing the design language.
                "color-swatch" => {
                    let hex = node
                        .properties
                        .get("color")
                        .map(String::as_str)
                        .unwrap_or("#00000000");
                    let size: i32 = node
                        .properties
                        .get("size")
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(28);
                    let rgba = parse_hex_rgba(hex);
                    let area = gtk::DrawingArea::new();
                    area.set_content_width(size);
                    area.set_content_height(size);
                    area.set_draw_func(move |a, cr, w, h| {
                        let (w, h) = (w as f64, h as f64);
                        let radius = (w.min(h) * 0.22).min(6.0);
                        // Rounded-rect path, inset 0.5px so the stroke stays crisp.
                        let (x0, y0, x1, y1) = (0.5, 0.5, w - 0.5, h - 0.5);
                        let pi = std::f64::consts::PI;
                        cr.new_sub_path();
                        cr.arc(x1 - radius, y0 + radius, radius, -0.5 * pi, 0.0);
                        cr.arc(x1 - radius, y1 - radius, radius, 0.0, 0.5 * pi);
                        cr.arc(x0 + radius, y1 - radius, radius, 0.5 * pi, pi);
                        cr.arc(x0 + radius, y0 + radius, radius, pi, 1.5 * pi);
                        cr.close_path();
                        // A faint neutral track first, so a translucent or very
                        // dark colour still reads as a filled box (not blank).
                        let ink = a.color();
                        cr.set_source_rgba(
                            ink.red() as f64,
                            ink.green() as f64,
                            ink.blue() as f64,
                            0.10,
                        );
                        let _ = cr.fill_preserve();
                        // The colour itself.
                        cr.set_source_rgba(rgba.0, rgba.1, rgba.2, rgba.3);
                        let _ = cr.fill_preserve();
                        // A subtle border so the swatch is delineated on any
                        // surface, even when the colour matches the background.
                        cr.set_source_rgba(
                            ink.red() as f64,
                            ink.green() as f64,
                            ink.blue() as f64,
                            0.35,
                        );
                        cr.set_line_width(1.0);
                        let _ = cr.stroke();
                    });
                    area.upcast()
                }
                // A breathing orb: concentric circles whose radius scales with
                // `properties["fraction"]` (0..1 = lungs empty..full). The
                // colour is read from the widget's CSS `color` at draw time, so
                // a plugin tints the phase by setting a class (e.g.
                // `breath-orb-inhale` → `color: var(--primary)`) and never
                // hardcodes a colour. Used by the breathing-exercise plugin.
                "breath-orb" => {
                    let fraction: f64 = node
                        .properties
                        .get("fraction")
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0.0_f64)
                        .clamp(0.0, 1.0);
                    let size: i32 = node
                        .properties
                        .get("size")
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(180);
                    let area = gtk::DrawingArea::new();
                    area.set_content_width(size);
                    area.set_content_height(size);
                    area.set_draw_func(move |a, cr, w, h| {
                        let c = a.color();
                        let (r, g, b) = (c.red() as f64, c.green() as f64, c.blue() as f64);
                        let cx = w as f64 / 2.0;
                        let cy = h as f64 / 2.0;
                        let max_r = (w.min(h) as f64 / 2.0) - 4.0;
                        if max_r <= 0.0 {
                            return;
                        }
                        let tau = std::f64::consts::TAU;
                        // Faint full-size track ring.
                        cr.set_source_rgba(r, g, b, 0.16);
                        cr.arc(cx, cy, max_r, 0.0, tau);
                        let _ = cr.fill();
                        // Breathing body: radius scales 0.34..1.0 of the track.
                        let rr = max_r * (0.34 + 0.66 * fraction);
                        cr.set_source_rgba(r, g, b, 0.45);
                        cr.arc(cx, cy, rr, 0.0, tau);
                        let _ = cr.fill();
                        // Solid core for a clear focal point.
                        cr.set_source_rgba(r, g, b, 0.92);
                        cr.arc(cx, cy, rr * 0.5, 0.0, tau);
                        let _ = cr.fill();
                    });
                    area.upcast()
                }
                // Future extension kinds slot in here without ever growing the WIT enum.
                _ => {
                    let b = gtk::Box::new(gtk::Orientation::Vertical, 6);
                    for child_id in &node.children {
                        if let Some(child) = by_id.get(child_id.as_str()) {
                            b.append(&build(child, by_id, inner));
                        }
                    }
                    b.upcast()
                }
            };
            inner_widget
        }
    };
    apply_node_properties(&widget, &node.properties);
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
    use super::{markdown_to_pango, parse_hex_rgba};

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn parses_hex_colours() {
        // #rrggbb → opaque.
        let (r, g, b, a) = parse_hex_rgba("#ff0000");
        assert!(close(r, 1.0) && close(g, 0.0) && close(b, 0.0) && close(a, 1.0));
        // margo's 0xrrggbbaa form, with alpha.
        let (r, g, b, a) = parse_hex_rgba("0x00ff0080");
        assert!(close(r, 0.0) && close(g, 1.0) && close(b, 0.0) && close(a, 128.0 / 255.0));
        // #rrggbbaa web form.
        let (.., a) = parse_hex_rgba("#11223344");
        assert!(close(a, 0x44 as f64 / 255.0));
        // A near-transparent dark token (the colour-scheme case) keeps low alpha.
        let (.., a) = parse_hex_rgba("0x44475a14");
        assert!(close(a, 0x14 as f64 / 255.0));
    }

    #[test]
    fn bogus_hex_is_opaque_black() {
        assert_eq!(parse_hex_rgba("garbage"), (0.0, 0.0, 0.0, 1.0));
        assert_eq!(parse_hex_rgba(""), (0.0, 0.0, 0.0, 1.0));
    }

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
