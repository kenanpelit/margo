//! Custom (user-defined) bar pill.
//!
//! A config-driven button: an icon or image + an optional label, with
//! left / right click commands and an optional `exec` poller whose stdout
//! fills the label via a `{output}` template. Defined under
//! `bars.widgets.custom_widgets` and placed in a bar slot via
//! `!Custom <name>`. (Inspired by VibePanel's custom widgets.)

use mshell_config::schema::config::{CustomMenuRow, CustomWidgetConfig};
use relm4::gtk::prelude::{
    BoxExt, ButtonExt, GestureExt, GestureSingleExt, OrientableExt, PopoverExt, WidgetExt,
};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::Duration;

/// Leading icon / image pixel size in the pill.
const ICON_SIZE: i32 = 16;

pub(crate) struct CustomWidgetModel {
    label: String,
}

#[derive(Debug)]
pub(crate) enum CustomWidgetInput {}

#[derive(Debug)]
pub(crate) enum CustomWidgetOutput {}

pub(crate) struct CustomWidgetInit {
    pub config: CustomWidgetConfig,
}

#[derive(Debug)]
pub(crate) enum CustomWidgetCommandOutput {
    /// New rendered label text from the `exec` poller.
    ExecResult(String),
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

        let model = CustomWidgetModel { label };

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
            wire_panel(&widgets.root, &config);
        } else if !config.menu.is_empty() {
            let popover = build_menu_popover(&config.menu);
            popover.set_parent(&widgets.root);
            widgets
                .root
                .connect_clicked(move |_| popover.popup());
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
            sender.command(move |out, shutdown| async move {
                let shutdown_fut = shutdown.wait();
                tokio::pin!(shutdown_fut);
                loop {
                    if let Some(stdout) = run_capture(&exec).await {
                        let rendered = truncate(&render(&stdout, &template), max_chars);
                        let _ = out.send(CustomWidgetCommandOutput::ExecResult(rendered));
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
            CustomWidgetCommandOutput::ExecResult(text) => {
                self.label = text;
            }
        }
        self.update_view(widgets, sender);
    }
}

/// Wire a left-click on this pill to open the plugin's sandboxed WASM panel in
/// a popover anchored to the pill. The panel is instantiated once and kept
/// alive by the click closure; its own internal event loop drives streaming.
#[cfg(feature = "wasm-plugins")]
fn wire_panel(button: &gtk::Button, config: &CustomWidgetConfig) {
    use mshell_plugin_ui::{PluginPanel, PluginRuntime};
    use std::collections::HashMap;
    use std::path::Path;

    let settings: HashMap<String, String> =
        serde_json::from_str(config.panel_settings.trim()).unwrap_or_default();

    let runtime = match PluginRuntime::new() {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("plugin panel: runtime init failed: {e}");
            return;
        }
    };
    let panel = match PluginPanel::new(
        &runtime,
        &config.name,
        Path::new(config.panel_entry.trim()),
        settings,
    ) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("plugin panel `{}`: load failed: {e}", config.name);
            return;
        }
    };

    let popover = gtk::Popover::new();
    popover.add_css_class("plugin-panel-popover");
    let content = panel.widget().clone();
    content.set_size_request(360, 480);
    popover.set_child(Some(&content));
    popover.set_parent(button);

    // Move `panel` into the closure so the instance (and its event loop) lives
    // as long as the pill does.
    button.connect_clicked(move |_| {
        let _keep = &panel;
        popover.popup();
    });
}

/// Without the `wasm-plugins` build, a panel pill can't run — hint at the
/// rebuild and do nothing on click.
#[cfg(not(feature = "wasm-plugins"))]
fn wire_panel(button: &gtk::Button, _config: &CustomWidgetConfig) {
    button.set_tooltip_text(Some(
        "This plugin needs an mshell built with --features wasm-plugins",
    ));
}

/// Build the click-dropdown popover from the widget's declarative menu rows.
/// Each row is an icon + label button that runs its `exec` and closes.
fn build_menu_popover(menu: &[CustomMenuRow]) -> gtk::Popover {
    let popover = gtk::Popover::new();
    popover.add_css_class("custom-widget-menu");
    let list = gtk::Box::new(gtk::Orientation::Vertical, 2);
    for row in menu {
        let label = row.label.trim();
        if label.is_empty() && row.exec.trim().is_empty() {
            continue;
        }
        let btn = gtk::Button::new();
        btn.add_css_class("custom-widget-menu-row");
        btn.set_has_frame(false);

        let hb = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        if !row.icon.trim().is_empty() {
            let img = gtk::Image::from_icon_name(row.icon.trim());
            img.set_pixel_size(ICON_SIZE);
            hb.append(&img);
        }
        let text = if label.is_empty() { row.exec.trim() } else { label };
        let lbl = gtk::Label::new(Some(text));
        lbl.set_halign(gtk::Align::Start);
        lbl.set_hexpand(true);
        hb.append(&lbl);
        btn.set_child(Some(&hb));

        let cmd = row.exec.clone();
        let pop = popover.clone();
        btn.connect_clicked(move |_| {
            run_cmd(&cmd);
            pop.popdown();
        });
        list.append(&btn);
    }
    popover.set_child(Some(&list));
    popover
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
    use super::{render, truncate};

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
