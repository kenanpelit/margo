//! Custom (user-defined) bar pill.
//!
//! A config-driven button: an icon or image + an optional label, with
//! left / right click commands and an optional `exec` poller whose stdout
//! fills the label via a `{output}` template. Defined under
//! `bars.widgets.custom_widgets` and placed in a bar slot via
//! `!Custom <name>`. (Inspired by VibePanel's custom widgets.)

use mshell_config::schema::config::CustomWidgetConfig;
use relm4::gtk::prelude::{
    BoxExt, ButtonExt, GestureExt, GestureSingleExt, OrientableExt, WidgetExt,
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

        // Left click → on_click.
        {
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
