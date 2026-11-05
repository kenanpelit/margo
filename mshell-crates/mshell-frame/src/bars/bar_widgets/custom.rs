//! Custom (user-defined) bar pill.
//!
//! A config-driven button: an icon or image + an optional label, with
//! left / right click commands and an optional `exec` poller whose stdout
//! fills the label via a `{output}` template. Defined under
//! `bars.widgets.custom_widgets` and placed in a bar slot via
//! `!Custom <name>`. (Inspired by VibePanel's custom widgets.)
//!
//! With `json: true` the exec prints one JSON object per poll that drives
//! visibility, label, glyph, theme-role colours and a multi-row tooltip
//! (port of the noctalia `stdout` plugin's JSON mode, see [`JsonOutput`]).
//! Right-click (when `on_click_right` is empty) or `mshellctl
//! custom-refresh [name]` re-runs the exec at once.
//!
//! The exec runs only while the pill is mapped (not in a collapsed drawer
//! or on a hidden bar); a pill its own JSON hid keeps polling so it can
//! come back. Pills with the same exec source (one per monitor) share one
//! loop (see [`exec`]). Art images are decoded off the GTK thread, and only
//! when the file (path or mtime) changed.

mod exec;

use crate::plugin_cmd::{run_capture, run_detached};
use exec::{ExecHub, ExecKey, ExecSubscription, Producer};
use mshell_config::schema::config::{CustomMenuRow, CustomWidgetConfig};
use relm4::gtk::prelude::{
    BoxExt, ButtonExt, Cast, GestureExt, GestureSingleExt, OrientableExt, WidgetExt,
};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

/// Leading icon / image pixel size in the pill.
const ICON_SIZE: i32 = 16;

/// The exec sources of every bar's custom pills.
static EXEC_HUB: ExecHub<ExecKey, ExecOutput> = ExecHub::new();

pub(crate) struct CustomWidgetModel {
    label: String,
    /// Fallback leading icon name (used when `art` has no current image).
    icon: String,
    /// When true, the exec poller's first stdout line is a live image path.
    art: bool,
    /// Currently-applied state CSS class from a `#<state>` exec first line
    /// (e.g. a VPN pill emits `#active` to tint its icon with the accent).
    /// Tracked so the previous class is removed before the new one is added.
    state_class: Option<String>,
    /// JSON mode: the theme-role / extra classes currently applied.
    json_classes: Vec<String>,
    /// JSON mode: the glyph currently shown (to skip needless rebuilds).
    json_glyph: Option<String>,
    /// The configured static tooltip (restored when JSON sends none).
    tooltip: String,
    /// The pill's handle on its shared exec source (`None` without an
    /// `exec`). Active while the exec should run: the pill is mapped, or
    /// its own JSON hid it (so the next poll can show it again).
    exec: Option<Rc<ExecSubscription<ExecOutput>>>,
    /// Art mode: the image widget showing the current art, kept across
    /// polls (`None` while the fallback icon shows).
    art_image: Option<gtk::Image>,
}

#[derive(Debug)]
pub(crate) enum CustomWidgetInput {}

#[derive(Debug)]
pub(crate) enum CustomWidgetOutput {
    /// A panel pill (plugin with a WASM panel) was clicked — ask the frame to
    /// open the first-class plugin-panel menu hosting it. Carries the compiled
    /// component path + resolved settings (JSON). Only emitted on a
    /// `wasm-plugins` build.
    #[cfg_attr(not(feature = "wasm-plugins"), allow(dead_code))]
    OpenPanel {
        name: String,
        entry: String,
        settings: String,
        capabilities: String,
        min_width: i32,
        max_height: i32,
    },
    /// A pill with a declarative `[[widget.menu]]` was clicked — ask the frame
    /// to open its command rows in the first-class plugin menu (layer-shell),
    /// instead of a pill-anchored popover.
    OpenMenu {
        name: String,
        rows: Vec<CustomMenuRow>,
        min_width: i32,
        max_height: i32,
    },
}

pub(crate) struct CustomWidgetInit {
    pub config: CustomWidgetConfig,
}

#[derive(Debug)]
pub(crate) enum CustomWidgetCommandOutput {
    /// The latest result of the pill's exec source.
    Exec(ExecOutput),
}

/// One `exec` run, as every pill on the source shows it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ExecOutput {
    /// Rendered label + the art image + paused flag. `paused` dims the pill
    /// (media-style) and is derived from the helper's optional status line
    /// (`art` widgets only).
    Plain {
        /// `art` mode: the decoded image (`None`: show the fallback icon).
        art: Option<gtk::gdk::Texture>,
        label: String,
        paused: bool,
        /// Optional state CSS class parsed from a `#<state>` first line.
        state_class: Option<String>,
    },
    /// A JSON-mode poll result (`None` = the output wasn't a JSON object).
    Json(Option<JsonOutput>),
}

#[relm4::component(pub)]
impl Component for CustomWidgetModel {
    type CommandOutput = CustomWidgetCommandOutput;
    type Input = CustomWidgetInput;
    type Output = CustomWidgetOutput;
    type Init = CustomWidgetInit;

    view! {
        #[root]
        #[name = "root"]
        gtk::Button {
            set_css_classes: &["custom-bar-widget", "ok-button-surface", "ok-bar-widget"],

            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 6,
                set_halign: gtk::Align::Center,
                set_valign: gtk::Align::Center,

                #[name = "icon_box"]
                gtk::Box {
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,
                },

                #[name = "label_widget"]
                gtk::Label {
                    add_css_class: "custom-bar-label",
                    #[watch]
                    set_label: model.label.as_str(),
                    #[watch]
                    set_visible: !model.label.is_empty(),
                },
            },
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let config = params.config;

        // Static label (truncated) until/unless an `exec` poller overrides it.
        let label = if config.exec.trim().is_empty() {
            truncate(&config.label, config.max_chars)
        } else {
            String::new()
        };

        let model = CustomWidgetModel {
            label,
            icon: config.icon.clone(),
            art: config.art,
            state_class: None,
            json_classes: Vec::new(),
            json_glyph: None,
            tooltip: config.tooltip.clone(),
            exec: (!config.exec.trim().is_empty()).then(|| {
                let key = ExecKey::of(&config);
                let sub = EXEC_HUB.subscribe(
                    key.clone(),
                    || exec_producer(key),
                    |fut| mshell_services::tokio_rt().spawn(fut).abort_handle(),
                );
                sub.set_active(root.is_mapped());
                Rc::new(sub)
            }),
            art_image: None,
        };

        let widgets = view_output!();

        if !config.tooltip.trim().is_empty() {
            widgets.root.set_tooltip_text(Some(&config.tooltip));
        }

        // Leading image (file) takes precedence over a named icon.
        if !config.image.trim().is_empty() {
            let img = gtk::Image::from_file(config.image.trim());
            img.set_pixel_size(ICON_SIZE);
            widgets.icon_box.append(&img);
        } else if !config.icon.trim().is_empty() {
            let img = gtk::Image::from_icon_name(config.icon.trim());
            img.set_pixel_size(ICON_SIZE);
            widgets.icon_box.append(&img);
        }

        // Left click priority: a WASM panel (in-shell sandboxed UI) > a
        // declarative dropdown menu > the `on_click` command.
        if !config.panel_entry.trim().is_empty() {
            wire_panel(&widgets.root, &config, &sender);
        } else if !config.menu.is_empty() {
            // Open the declarative menu rows as a first-class layer-shell menu
            // (via the frame), not a pill-anchored popover.
            let name = config.name.clone();
            let rows = config.menu.clone();
            let min_width = config.panel_min_width;
            let max_height = config.panel_max_height;
            let sender = sender.clone();
            widgets.root.connect_clicked(move |_| {
                let _ = sender.output(CustomWidgetOutput::OpenMenu {
                    name: name.clone(),
                    rows: rows.clone(),
                    min_width,
                    max_height,
                });
            });
        } else {
            let cmd = config.on_click.clone();
            let env = config.secret_env.clone();
            widgets
                .root
                .connect_clicked(move |_| run_detached(&cmd, &env));
        }

        // Right click → on_click_right; without one, an `exec` pill re-runs
        // its command right away, for its pills on every monitor.
        if let (true, Some(sub)) = (config.on_click_right.trim().is_empty(), &model.exec) {
            let gesture = gtk::GestureClick::new();
            gesture.set_button(gtk::gdk::BUTTON_SECONDARY);
            let sub = Rc::downgrade(sub);
            gesture.connect_pressed(move |g, _, _, _| {
                g.set_state(gtk::EventSequenceState::Claimed);
                with_sub(&sub, ExecSubscription::refresh);
            });
            widgets.root.add_controller(gesture);
        }
        if !config.on_click_right.trim().is_empty() {
            let gesture = gtk::GestureClick::new();
            gesture.set_button(gtk::gdk::BUTTON_SECONDARY);
            let cmd = config.on_click_right.clone();
            let env = config.secret_env.clone();
            gesture.connect_pressed(move |g, _, _, _| {
                g.set_state(gtk::EventSequenceState::Claimed);
                run_detached(&cmd, &env);
            });
            widgets.root.add_controller(gesture);
        }

        // The exec runs while the pill is mapped. An unmap because the
        // pill's own JSON hid it keeps it running (see `apply_json`).
        if let Some(sub) = &model.exec {
            let weak = Rc::downgrade(sub);
            widgets
                .root
                .connect_map(move |_| with_sub(&weak, |s| s.set_active(true)));
            let weak = Rc::downgrade(sub);
            widgets
                .root
                .connect_unmap(move |w| with_sub(&weak, |s| s.set_active(!w.is_visible())));

            // Show every result of the shared source (the current one at
            // once, when another monitor's pill already ran it).
            let mut results = sub.watch();
            sender.command(move |out, shutdown| async move {
                let shutdown_fut = shutdown.wait();
                tokio::pin!(shutdown_fut);
                loop {
                    let latest = results.borrow_and_update().clone();
                    if let Some(latest) = latest {
                        let _ = out.send(CustomWidgetCommandOutput::Exec(latest));
                    }
                    tokio::select! {
                        () = &mut shutdown_fut => return,
                        changed = results.changed() => if changed.is_err() { return },
                    }
                }
            });
        }

        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        _widgets: &mut Self::Widgets,
        message: Self::Input,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {}
    }

    fn update_cmd_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        let CustomWidgetCommandOutput::Exec(result) = message;
        match result {
            ExecOutput::Plain {
                art,
                label,
                paused,
                state_class,
            } => {
                self.label = label;
                if paused {
                    widgets.root.add_css_class("paused");
                } else {
                    widgets.root.remove_css_class("paused");
                }
                // Swap the `#<state>` CSS class (remove the old, add the new).
                if self.state_class != state_class {
                    if let Some(old) = self.state_class.take() {
                        widgets.root.remove_css_class(&old);
                    }
                    if let Some(new) = &state_class {
                        widgets.root.add_css_class(new);
                    }
                    self.state_class = state_class;
                }
                if self.art {
                    self.apply_art(widgets, art);
                }
            }
            ExecOutput::Json(parsed) => {
                self.apply_json(widgets, parsed);
            }
        }
        self.update_view(widgets, sender);
    }
}

impl CustomWidgetModel {
    /// Show the art image (already decoded off the GTK thread) or the
    /// fallback icon; the image already shown costs nothing.
    fn apply_art(&mut self, widgets: &CustomWidgetModelWidgets, art: Option<gtk::gdk::Texture>) {
        match art {
            Some(texture) => {
                if self.art_image.is_none() {
                    clear_box(&widgets.icon_box);
                    let img = gtk::Image::new();
                    img.set_pixel_size(ICON_SIZE);
                    widgets.icon_box.append(&img);
                    self.art_image = Some(img);
                }
                if let Some(img) = &self.art_image
                    && img.paintable().as_ref() != Some(texture.upcast_ref())
                {
                    img.set_paintable(Some(&texture));
                }
            }
            None => {
                if self.art_image.take().is_some() || widgets.icon_box.first_child().is_none() {
                    clear_box(&widgets.icon_box);
                    if !self.icon.trim().is_empty() {
                        let img = gtk::Image::from_icon_name(self.icon.trim());
                        img.set_pixel_size(ICON_SIZE);
                        widgets.icon_box.append(&img);
                    }
                }
            }
        }
    }

    /// Apply one JSON-mode poll to the pill.
    fn apply_json(&mut self, widgets: &CustomWidgetModelWidgets, parsed: Option<JsonOutput>) {
        let root = &widgets.root;
        let Some(j) = parsed else {
            // Not a JSON object: keep the pill visible with its configured
            // icon and say why, rather than silently vanishing.
            root.set_visible(true);
            self.label.clear();
            self.set_json_glyph(widgets, None);
            self.swap_json_classes(root, Vec::new());
            root.set_tooltip_text(Some("This widget's exec did not print a JSON object"));
            return;
        };
        root.set_visible(j.visible);
        if !j.visible
            && let Some(sub) = &self.exec
        {
            // Hidden by its own output: keep polling so it can come back.
            sub.set_active(true);
        }
        self.label = j.text.clone();
        self.set_json_glyph(widgets, j.glyph.clone());
        self.swap_json_classes(root, j.css_classes());
        match j.tooltip_markup() {
            Some(markup) => root.set_tooltip_markup(Some(&markup)),
            None if self.tooltip.trim().is_empty() => root.set_tooltip_text(None),
            None => root.set_tooltip_text(Some(&self.tooltip)),
        }
    }

    /// Show `glyph` (or fall back to the configured icon) as the leading icon.
    fn set_json_glyph(&mut self, widgets: &CustomWidgetModelWidgets, glyph: Option<String>) {
        if self.json_glyph == glyph && widgets.icon_box.first_child().is_some() {
            return;
        }
        while let Some(child) = widgets.icon_box.first_child() {
            widgets.icon_box.remove(&child);
        }
        let name = glyph
            .clone()
            .unwrap_or_else(|| self.icon.trim().to_string());
        if !name.is_empty() {
            let img = gtk::Image::from_icon_name(&name);
            img.set_pixel_size(ICON_SIZE);
            widgets.icon_box.append(&img);
        }
        self.json_glyph = glyph;
    }

    /// Replace the JSON-driven CSS classes on the pill.
    fn swap_json_classes(&mut self, root: &gtk::Button, next: Vec<String>) {
        if self.json_classes == next {
            return;
        }
        for c in &self.json_classes {
            root.remove_css_class(c);
        }
        for c in &next {
            root.add_css_class(c);
        }
        self.json_classes = next;
    }
}

/// Theme roles a JSON pill may colour its text / glyph with. Raw hex is
/// deliberately not accepted: every colour follows the matugen theme
/// (DESIGN.md §1); the SCSS maps each class to its `var(--role)`.
const COLOR_ROLES: &[&str] = &["primary", "secondary", "tertiary", "error", "muted"];

/// One JSON-mode poll, parsed. Keys follow the noctalia `stdout` plugin
/// (camelCase `textColor` / `glyphColor` are accepted too).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct JsonOutput {
    pub(crate) visible: bool,
    pub(crate) text: String,
    pub(crate) glyph: Option<String>,
    pub(crate) text_color: Option<&'static str>,
    pub(crate) glyph_color: Option<&'static str>,
    pub(crate) class: Option<String>,
    pub(crate) tooltip: Option<String>,
    pub(crate) tips: Vec<(String, String)>,
}

impl JsonOutput {
    /// Parse the exec's stdout. `None` unless it is a JSON object.
    pub(crate) fn parse(stdout: &str) -> Option<Self> {
        let v: serde_json::Value = serde_json::from_str(stdout.trim()).ok()?;
        let o = v.as_object()?;
        let str_of = |keys: &[&str]| {
            keys.iter()
                .find_map(|k| o.get(*k).and_then(|v| v.as_str()))
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        let role = |keys: &[&str]| {
            let want = str_of(keys)?.to_ascii_lowercase();
            COLOR_ROLES.iter().copied().find(|r| *r == want)
        };
        let tips = o
            .get("tips")
            .and_then(|t| t.as_array())
            .map(|rows| {
                rows.iter()
                    .filter_map(|row| {
                        let row = row.as_array()?;
                        let cell = |i: usize| match row.get(i) {
                            Some(serde_json::Value::String(s)) => s.clone(),
                            Some(serde_json::Value::Null) | None => String::new(),
                            Some(other) => other.to_string(),
                        };
                        let (k, v) = (cell(0), cell(1));
                        (!k.is_empty() || !v.is_empty()).then_some((k, v))
                    })
                    .collect()
            })
            .unwrap_or_default();
        Some(Self {
            visible: o.get("visible").and_then(|v| v.as_bool()).unwrap_or(true),
            text: match o.get("text") {
                Some(serde_json::Value::String(s)) => s.clone(),
                Some(serde_json::Value::Number(n)) => n.to_string(),
                _ => String::new(),
            },
            glyph: str_of(&["glyph", "icon"]),
            text_color: role(&["text_color", "textColor"]),
            glyph_color: role(&["glyph_color", "glyphColor"]),
            class: str_of(&["class"]).filter(|c| {
                c.chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
            }),
            tooltip: str_of(&["tooltip"]),
            tips,
        })
    }

    /// CSS classes for the pill: `custom-text-<role>`, `custom-glyph-<role>`
    /// and the sanitised `class`.
    pub(crate) fn css_classes(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(r) = self.text_color {
            out.push(format!("custom-text-{r}"));
        }
        if let Some(r) = self.glyph_color {
            out.push(format!("custom-glyph-{r}"));
        }
        if let Some(c) = &self.class {
            out.push(c.clone());
        }
        out
    }

    /// Tooltip as Pango markup: the `tooltip` line, then one
    /// "<b>label</b> value" row per tip. `None` when there's neither.
    pub(crate) fn tooltip_markup(&self) -> Option<String> {
        let esc = |s: &str| gtk::glib::markup_escape_text(s).to_string();
        let mut lines = Vec::new();
        if let Some(t) = &self.tooltip {
            lines.push(esc(t));
        }
        for (k, v) in &self.tips {
            lines.push(match (k.is_empty(), v.is_empty()) {
                (false, false) => format!("<b>{}</b> {}", esc(k), esc(v)),
                (false, true) => format!("<b>{}</b>", esc(k)),
                _ => esc(v),
            });
        }
        (!lines.is_empty()).then(|| lines.join("\n"))
    }
}

/// On click, ask the frame to open the first-class plugin-panel menu hosting
/// this plugin's WASM panel. The frame owns the wasm runtime + the panel, so
/// the panel gets the same position/size config as any built-in menu.
#[cfg(feature = "wasm-plugins")]
fn wire_panel(
    button: &gtk::Button,
    config: &CustomWidgetConfig,
    sender: &ComponentSender<CustomWidgetModel>,
) {
    let name = config.name.clone();
    let entry = config.panel_entry.clone();
    let settings = config.panel_settings.clone();
    let capabilities = config.panel_capabilities.clone();
    let min_width = config.panel_min_width;
    let max_height = config.panel_max_height;
    let sender = sender.clone();
    button.connect_clicked(move |_| {
        let _ = sender.output(CustomWidgetOutput::OpenPanel {
            name: name.clone(),
            entry: entry.clone(),
            settings: settings.clone(),
            capabilities: capabilities.clone(),
            min_width,
            max_height,
        });
    });
}

/// Without the `wasm-plugins` build there's no WASM runtime, so a panel pill
/// falls back to its `on_click` (e.g. a terminal chat) — or hints at the
/// rebuild if the plugin offers no fallback command.
#[cfg(not(feature = "wasm-plugins"))]
fn wire_panel(
    button: &gtk::Button,
    config: &CustomWidgetConfig,
    _sender: &ComponentSender<CustomWidgetModel>,
) {
    let cmd = config.on_click.clone();
    if cmd.trim().is_empty() {
        button.set_tooltip_text(Some(
            "This plugin's panel needs an mshell built with --features wasm-plugins",
        ));
    } else {
        let env = config.secret_env.clone();
        button.connect_clicked(move |_| run_detached(&cmd, &env));
    }
}

/// Run `f` on the subscription while the pill still holds it.
fn with_sub(
    sub: &Weak<ExecSubscription<ExecOutput>>,
    f: impl FnOnce(&ExecSubscription<ExecOutput>),
) {
    if let Some(sub) = sub.upgrade() {
        f(&sub);
    }
}

/// The shared loop's run for the source `key`: the command once, its
/// stdout parsed the way the pills' config says. The art file last decoded
/// is remembered across runs, so an unchanged one isn't decoded again.
fn exec_producer(key: ExecKey) -> Producer<ExecOutput> {
    let name = key.name.clone();
    let period = (key.interval > 0).then(|| Duration::from_secs(key.interval));
    let key = Arc::new(key);
    let art: Arc<ArtState> = Arc::default();
    Producer {
        probe: Arc::new(move || {
            let (key, art) = (key.clone(), art.clone());
            Box::pin(async move { run_exec(&key, &art).await })
        }),
        period,
        name,
    }
}

/// The art tracker and the image it last decoded.
type ArtState = tokio::sync::Mutex<(ArtTracker, Option<gtk::gdk::Texture>)>;

async fn run_exec(key: &ExecKey, art: &ArtState) -> Option<ExecOutput> {
    let stdout = run_capture(&key.exec, &key.secret_env).await?;
    if key.json {
        let parsed = JsonOutput::parse(&stdout).map(|mut j| {
            j.text = truncate(&render(&j.text, &key.template), key.max_chars);
            j
        });
        return Some(ExecOutput::Json(parsed));
    }
    let (texture, body, paused, state_class) = if key.art {
        let (cand, label, paused) = split_art_output(&stdout);
        let mut art = art.lock().await;
        let (seen, shown) = &mut *art;
        match art_update(seen, cand).await {
            ArtUpdate::Unchanged => {}
            ArtUpdate::Image(t) => *shown = Some(t),
            ArtUpdate::Fallback => *shown = None,
        }
        (shown.clone(), label, paused, None)
    } else {
        let (state, label) = split_state_output(&stdout);
        (None, label, false, state)
    };
    Some(ExecOutput::Plain {
        art: texture,
        label: truncate(&render(&body, &key.template), key.max_chars),
        paused,
        state_class,
    })
}

/// Split an `art` widget's exec stdout into its three fields:
/// `(image-path candidate, label, paused)`. Line 1 is the image path (empty →
/// `None`), line 2 the label, line 3 the status (`paused`/`stopped` → dim).
/// Pure — the caller decides whether the path actually exists.
/// Non-`art` exec convention: an optional leading `#<state>` line becomes a CSS
/// class on the pill (and is stripped from the label). Lets a status pill tint
/// itself — e.g. a VPN pill emits `#active` when connected so `.custom-bar-widget
/// .active` can recolour the icon with the accent. The state token is sanitised
/// to a CSS-safe identifier; anything else leaves the output as the plain label.
fn split_state_output(stdout: &str) -> (Option<String>, String) {
    let mut lines = stdout.lines();
    let first = lines.next().unwrap_or("");
    if let Some(tok) = first.trim().strip_prefix('#') {
        let tok = tok.trim();
        let safe = !tok.is_empty()
            && tok
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        let class = safe.then(|| tok.to_string());
        let label = lines.collect::<Vec<_>>().join("\n");
        return (class, label);
    }
    (None, stdout.to_string())
}

/// What a poll does to the art image.
#[derive(Debug)]
enum ArtUpdate {
    /// Same file as shown (path and mtime): keep it.
    Unchanged,
    /// A new image, decoded on a blocking thread.
    Image(gtk::gdk::Texture),
    /// No (readable) art: show the configured icon.
    Fallback,
}

/// The art file last shown, by path and modification time.
#[derive(Debug, Default)]
pub(crate) struct ArtTracker {
    shown: Option<Option<(String, SystemTime)>>,
}

impl ArtTracker {
    /// What to do for `path`: `None` when it is what is shown already,
    /// else the file to load (`Some(None)`: no existing file, show the
    /// fallback icon).
    pub(crate) fn next(&mut self, path: Option<&str>) -> Option<Option<String>> {
        let key = path.and_then(|p| {
            let mtime = std::fs::metadata(p).and_then(|m| m.modified()).ok()?;
            Some((p.to_string(), mtime))
        });
        if self.shown.as_ref() == Some(&key) {
            return None;
        }
        let load = key.as_ref().map(|(p, _)| p.clone());
        self.shown = Some(key);
        Some(load)
    }

    /// Forget the shown file (its load failed), so the next poll retries.
    fn forget(&mut self) {
        self.shown = None;
    }
}

/// Turn the exec's art path into an [`ArtUpdate`], decoding a changed
/// file on a blocking thread.
async fn art_update(seen: &mut ArtTracker, path: Option<String>) -> ArtUpdate {
    match seen.next(path.as_deref()) {
        None => ArtUpdate::Unchanged,
        Some(None) => ArtUpdate::Fallback,
        Some(Some(file)) => {
            let decoded =
                tokio::task::spawn_blocking(move || gtk::gdk::Texture::from_filename(file)).await;
            match decoded {
                Ok(Ok(texture)) => ArtUpdate::Image(texture),
                _ => {
                    seen.forget();
                    ArtUpdate::Fallback
                }
            }
        }
    }
}

fn clear_box(b: &gtk::Box) {
    while let Some(child) = b.first_child() {
        b.remove(&child);
    }
}

fn split_art_output(stdout: &str) -> (Option<String>, String, bool) {
    let mut lines = stdout.lines();
    let first = lines.next().unwrap_or("").trim();
    let label = lines.next().unwrap_or("").to_string();
    let status = lines.next().unwrap_or("").trim().to_lowercase();
    let art = if first.is_empty() {
        None
    } else {
        Some(first.to_string())
    };
    let paused = status == "paused" || status == "stopped";
    (art, label, paused)
}

/// Render the `exec` output through the `{output}` template.
fn render(output: &str, template: &str) -> String {
    let output = output.trim();
    if template.trim().is_empty() {
        output.to_string()
    } else {
        template.replace("{output}", output)
    }
}

/// Truncate to `max` characters (0 = no cap).
fn truncate(s: &str, max: u32) -> String {
    if max == 0 {
        s.to_string()
    } else {
        s.chars().take(max as usize).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::exec::{ExecHub, ExecKey};
    use super::{
        ArtTracker, ExecOutput, JsonOutput, exec_producer, render, split_art_output, truncate,
    };
    use mshell_config::schema::config::CustomWidgetConfig;
    use std::time::{Duration, SystemTime};

    /// One pill per monitor, a fake command that counts its runs: one
    /// spawn per poll or refresh, whatever the number of monitors.
    #[tokio::test]
    async fn monitors_share_one_spawn_of_the_command() {
        let dir = tempfile::tempdir().expect("tempdir");
        let runs = dir.path().join("runs");
        let config = CustomWidgetConfig {
            name: "custom-exec-share-test".into(),
            exec: format!(
                "echo x >> '{f}'; echo '#on'; wc -l < '{f}'",
                f = runs.display()
            ),
            template: "<{output}>".into(),
            ..Default::default()
        };
        let count = || {
            std::fs::read_to_string(&runs)
                .map(|s| s.lines().count())
                .unwrap_or(0)
        };
        let hub: ExecHub<ExecKey, ExecOutput> = ExecHub::new();
        let spawn = |fut| tokio::spawn(fut).abort_handle();
        let pills: Vec<_> = (0..3)
            .map(|_| {
                let key = ExecKey::of(&config);
                hub.subscribe(key.clone(), || exec_producer(key), spawn)
            })
            .collect();
        for p in &pills {
            p.set_active(true);
        }
        let label = |n: usize| ExecOutput::Plain {
            art: None,
            label: format!("<{n}>"),
            paused: false,
            state_class: Some("on".into()),
        };
        let mut rx = pills[2].watch();
        let first = label(1);
        tokio::time::timeout(
            Duration::from_secs(10),
            rx.wait_for(|v| v.as_ref() == Some(&first)),
        )
        .await
        .expect("first run")
        .expect("source alive");
        assert_eq!(count(), 1);
        // Right-click on another monitor: one more run, seen by every pill.
        pills[0].refresh();
        let second = label(2);
        tokio::time::timeout(
            Duration::from_secs(10),
            rx.wait_for(|v| v.as_ref() == Some(&second)),
        )
        .await
        .expect("refresh")
        .expect("source alive");
        assert_eq!(*pills[1].watch().borrow(), Some(label(2)));
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(count(), 2);
        // Another template is another source: its own run.
        let other = CustomWidgetConfig {
            template: "{output}!".into(),
            ..config.clone()
        };
        let key = ExecKey::of(&other);
        let p = hub.subscribe(key.clone(), || exec_producer(key), spawn);
        p.set_active(true);
        let mut rx = p.watch();
        tokio::time::timeout(Duration::from_secs(10), rx.wait_for(Option::is_some))
            .await
            .expect("other run")
            .expect("source alive");
        assert_eq!(count(), 3);
    }

    #[test]
    fn art_reloads_only_when_the_file_changes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let a = dir.path().join("a.png");
        std::fs::write(&a, b"x").expect("write");
        let a = a.to_string_lossy().into_owned();
        let mut t = ArtTracker::default();
        assert_eq!(t.next(Some(&a)), Some(Some(a.clone())), "first: load");
        assert_eq!(t.next(Some(&a)), None, "same file: skip");
        // Rewritten in place (same path, new mtime): load again.
        let f = std::fs::File::options().write(true).open(&a).expect("open");
        f.set_modified(SystemTime::now() + Duration::from_secs(5))
            .expect("mtime");
        assert_eq!(t.next(Some(&a)), Some(Some(a.clone())));
        assert_eq!(t.next(Some(&a)), None);
        // No art (or a missing file): the fallback once, then nothing.
        assert_eq!(t.next(None), Some(None));
        assert_eq!(t.next(Some("/nonexistent/art.png")), None);
        assert_eq!(t.next(Some(&a)), Some(Some(a.clone())));
    }

    #[test]
    fn art_output_splits_three_fields() {
        let (art, label, paused) = split_art_output("/tmp/cover.png\nSong — Artist\nplaying");
        assert_eq!(art.as_deref(), Some("/tmp/cover.png"));
        assert_eq!(label, "Song — Artist");
        assert!(!paused);
    }

    #[test]
    fn art_output_paused_and_stopped_dim() {
        assert!(split_art_output("/a\nL\npaused").2);
        assert!(split_art_output("/a\nL\nStopped").2); // case-insensitive
        assert!(!split_art_output("/a\nL\nPlaying").2);
    }

    #[test]
    fn art_output_tolerates_missing_lines() {
        // Empty art line → no image; missing status → not paused.
        let (art, label, paused) = split_art_output("\nJust a label");
        assert_eq!(art, None);
        assert_eq!(label, "Just a label");
        assert!(!paused);
        // Nothing at all.
        let (art, label, paused) = split_art_output("");
        assert_eq!(art, None);
        assert_eq!(label, "");
        assert!(!paused);
    }

    #[test]
    fn render_uses_template_placeholder() {
        assert_eq!(render("42", " {output}\u{b0}"), " 42\u{b0}");
        assert_eq!(render("  hi  ", "[{output}]"), "[hi]");
    }

    #[test]
    fn render_empty_template_is_trimmed_output() {
        assert_eq!(render(" 1.2M ", ""), "1.2M");
        assert_eq!(render("x", "   "), "x");
    }

    #[test]
    fn truncate_caps_by_chars() {
        assert_eq!(truncate("hello", 0), "hello"); // 0 = no cap
        assert_eq!(truncate("hello", 3), "hel");
        assert_eq!(truncate("hi", 5), "hi");
        assert_eq!(truncate("h\u{e9}llo", 3), "h\u{e9}l"); // char-based, not byte
    }

    #[test]
    fn json_output_parses_the_stdout_plugin_example() {
        // The noctalia `stdout` plugin README's example (bt battery).
        let j = JsonOutput::parse(
            r##"{"visible": true, "glyph": "input-mouse-symbolic", "text": "26%",
                "fontWeight": "bold", "textColor": "#00FF00", "glyphColor": "error",
                "tips": [["Name:", "TX Track"], ["Paired:", "yes"]]}"##,
        )
        .unwrap();
        assert!(j.visible);
        assert_eq!(j.text, "26%");
        assert_eq!(j.glyph.as_deref(), Some("input-mouse-symbolic"));
        // Hex isn't a theme role: ignored. `error` is.
        assert_eq!(j.text_color, None);
        assert_eq!(j.glyph_color, Some("error"));
        assert_eq!(j.css_classes(), vec!["custom-glyph-error".to_string()]);
        assert_eq!(
            j.tooltip_markup().as_deref(),
            Some("<b>Name:</b> TX Track\n<b>Paired:</b> yes")
        );
    }

    #[test]
    fn json_output_defaults_and_hiding() {
        let j = JsonOutput::parse(r#"{"visible": false}"#).unwrap();
        assert!(!j.visible);
        assert_eq!(j.text, "");
        assert_eq!(j.glyph, None);
        assert_eq!(j.tooltip_markup(), None);
        // Missing `visible` means visible; numbers are fine as text.
        let j = JsonOutput::parse(r#"{"text": 42}"#).unwrap();
        assert!(j.visible);
        assert_eq!(j.text, "42");
    }

    #[test]
    fn json_output_rejects_non_objects_and_garbage() {
        assert!(JsonOutput::parse("").is_none());
        assert!(JsonOutput::parse("26%").is_none());
        assert!(JsonOutput::parse("[1, 2]").is_none());
        assert!(JsonOutput::parse("{\"text\": ").is_none());
    }

    #[test]
    fn json_output_escapes_markup_and_sanitises_class() {
        let j = JsonOutput::parse(
            r#"{"tooltip": "a <b> & c", "tips": [["k<", 3], ["", "only value"], []],
                "class": "warn ing", "text_color": "Primary"}"#,
        )
        .unwrap();
        assert_eq!(
            j.tooltip_markup().as_deref(),
            Some("a &lt;b&gt; &amp; c\n<b>k&lt;</b> 3\nonly value")
        );
        assert_eq!(j.class, None, "a class with a space is not CSS-safe");
        assert_eq!(j.text_color, Some("primary"));
        let j = JsonOutput::parse(r#"{"class": "active"}"#).unwrap();
        assert_eq!(j.css_classes(), vec!["active".to_string()]);
    }
}
