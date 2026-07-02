//! Custom (user-defined) bar pill.
//!
//! A config-driven button: an icon or image + an optional label, with
//! left / right click commands and an optional `exec` poller whose stdout
//! fills the label via a `{output}` template. Defined under
//! `bars.widgets.custom_widgets` and placed in a bar slot via
//! `!Custom <name>`. (Inspired by VibePanel's custom widgets.)

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
            widgets.root.connect_clicked(move |_| run_cmd(&cmd));
        }

        // Right click → on_click_right.
        if !config.on_click_right.trim().is_empty() {
            let gesture = gtk::GestureClick::new();
            gesture.set_button(gtk::gdk::BUTTON_SECONDARY);
            let cmd = config.on_click_right.clone();
            gesture.connect_pressed(move |g, _, _, _| {
                g.set_state(gtk::EventSequenceState::Claimed);
                run_cmd(&cmd);
            });
            widgets.root.add_controller(gesture);
        }

        // `exec` poller fills the label from command stdout.
        if !config.exec.trim().is_empty() {
            let exec = config.exec.clone();
            let template = config.template.clone();
            let max_chars = config.max_chars;
            let interval = config.interval;
            let art = config.art;
            sender.command(move |out, shutdown| async move {
                let shutdown_fut = shutdown.wait();
                tokio::pin!(shutdown_fut);
                loop {
                    if let Some(stdout) = run_capture(&exec).await {
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
                    if interval == 0 {
                        break;
                    }
                    tokio::select! {
                        () = &mut shutdown_fut => break,
                        _ = tokio::time::sleep(Duration::from_secs(interval)) => {}
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
        }
        self.update_view(widgets, sender);
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
        button.connect_clicked(move |_| run_cmd(&cmd));
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

/// Fire-and-forget a shell command (`sh -c`). Reaped to avoid zombies.
fn run_cmd(cmd: &str) {
    let cmd = cmd.trim().to_string();
    if cmd.is_empty() {
        return;
    }
    relm4::spawn(async move {
        let _ = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(&cmd)
            .status()
            .await;
    });
}

/// Run a shell command (`sh -c`) and capture its stdout. `None` on spawn
/// failure or a non-zero exit.
async fn run_capture(cmd: &str) -> Option<String> {
    let out = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::{render, split_art_output, truncate};

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
}
