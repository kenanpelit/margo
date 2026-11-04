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

use crate::plugin_cmd::{run_capture, run_detached};
use mshell_config::schema::config::{CustomMenuRow, CustomWidgetConfig};
use relm4::gtk::prelude::{
    BoxExt, ButtonExt, GestureExt, GestureSingleExt, OrientableExt, WidgetExt,
};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::Duration;

/// Leading icon / image pixel size in the pill.
const ICON_SIZE: i32 = 16;

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
    /// New rendered label + optional live image path + paused flag from the
    /// `exec` poller. `paused` dims the pill (media-style) and is derived from
    /// the helper's optional status line (`art` widgets only).
    ExecResult {
        art: Option<String>,
        label: String,
        paused: bool,
        /// Optional state CSS class parsed from a `#<state>` first line.
        state_class: Option<String>,
    },
    /// A JSON-mode poll result (`None` = the output wasn't a JSON object).
    JsonResult(Option<JsonOutput>),
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
        // its command right away (force refresh).
        let refresh = std::sync::Arc::new(tokio::sync::Notify::new());
        if config.on_click_right.trim().is_empty() && !config.exec.trim().is_empty() {
            let gesture = gtk::GestureClick::new();
            gesture.set_button(gtk::gdk::BUTTON_SECONDARY);
            let refresh = refresh.clone();
            gesture.connect_pressed(move |g, _, _, _| {
                g.set_state(gtk::EventSequenceState::Claimed);
                refresh.notify_one();
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

        // `exec` poller fills the label from command stdout.
        if !config.exec.trim().is_empty() {
            let exec = config.exec.clone();
            let env = config.secret_env.clone();
            let template = config.template.clone();
            let max_chars = config.max_chars;
            let interval = config.interval;
            let art = config.art;
            let json = config.json;
            let name = config.name.clone();
            let mut ipc_refresh = mshell_common::custom_refresh::subscribe();
            sender.command(move |out, shutdown| async move {
                let shutdown_fut = shutdown.wait();
                tokio::pin!(shutdown_fut);
                loop {
                    if json {
                        if let Some(stdout) = run_capture(&exec, &env).await {
                            let parsed = JsonOutput::parse(&stdout).map(|mut j| {
                                j.text = truncate(&render(&j.text, &template), max_chars);
                                j
                            });
                            let _ = out.send(CustomWidgetCommandOutput::JsonResult(parsed));
                        }
                    } else if let Some(stdout) = run_capture(&exec, &env).await {
                        let (art_path, body, paused, state_class) = if art {
                            let (cand, label, paused) = split_art_output(&stdout);
                            // Only show the image if the file actually exists.
                            let path = cand.filter(|p| std::path::Path::new(p).exists());
                            (path, label, paused, None)
                        } else {
                            let (state, label) = split_state_output(&stdout);
                            (None, label, false, state)
                        };
                        let rendered = truncate(&render(&body, &template), max_chars);
                        let _ = out.send(CustomWidgetCommandOutput::ExecResult {
                            art: art_path,
                            label: rendered,
                            paused,
                            state_class,
                        });
                    }
                    // Wait for the next interval, a right-click, or an IPC
                    // refresh aimed at this pill. `interval == 0` runs once
                    // and then only on a forced refresh.
                    let sleep = async {
                        if interval == 0 {
                            std::future::pending::<()>().await;
                        } else {
                            tokio::time::sleep(Duration::from_secs(interval)).await;
                        }
                    };
                    tokio::pin!(sleep);
                    loop {
                        tokio::select! {
                            () = &mut shutdown_fut => return,
                            () = &mut sleep => break,
                            () = refresh.notified() => break,
                            msg = ipc_refresh.recv() => match msg {
                                Ok(target) if mshell_common::custom_refresh::targets(&target, &name) => break,
                                // Lagged: skip. Closed can't happen (the
                                // sender is a static), but don't spin on it.
                                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                    ipc_refresh = mshell_common::custom_refresh::subscribe();
                                }
                            },
                        }
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
        match message {
            CustomWidgetCommandOutput::ExecResult {
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
                    // Reload the leading image (e.g. album art) — rebuild the
                    // icon box so a changed file on disk actually re-renders.
                    while let Some(child) = widgets.icon_box.first_child() {
                        widgets.icon_box.remove(&child);
                    }
                    if let Some(path) = art {
                        let img = gtk::Image::from_file(&path);
                        img.set_pixel_size(ICON_SIZE);
                        widgets.icon_box.append(&img);
                    } else if !self.icon.trim().is_empty() {
                        let img = gtk::Image::from_icon_name(self.icon.trim());
                        img.set_pixel_size(ICON_SIZE);
                        widgets.icon_box.append(&img);
                    }
                }
            }
            CustomWidgetCommandOutput::JsonResult(parsed) => {
                self.apply_json(widgets, parsed);
            }
        }
        self.update_view(widgets, sender);
    }
}

impl CustomWidgetModel {
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
    use super::{JsonOutput, render, split_art_output, truncate};

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
