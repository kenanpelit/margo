//! Night-light quick-action button.
//!
//! Drives margo's built-in **twilight** blue-light filter via
//! `mctl twilight` rather than running a second, independent gamma
//! controller in the shell:
//!   * click  → `mctl twilight toggle` (flips the schedule on/off)
//!   * state  → `mctl twilight status --json`, polled every few
//!     seconds plus an immediate re-probe after each toggle.
//!
//! margo owns the output gamma ramps itself (geo schedule, day /
//! night phases, smooth transitions), so going through `mctl`
//! keeps a single source of truth — no two writers fighting over
//! `zwlr_gamma_control` the way the old `mshell-gamma` path did.

use relm4::gtk::prelude::{ButtonExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::Duration;
use tracing::warn;

const POLL_INTERVAL: Duration = Duration::from_secs(5);
const STARTUP_DELAY: Duration = Duration::from_millis(200);
const POST_TOGGLE_DELAY: Duration = Duration::from_millis(150);

#[derive(Debug)]
pub(crate) struct NightLightModel {
    enabled: bool,
}

#[derive(Debug)]
pub(crate) enum NightLightInput {
    Clicked,
}

#[derive(Debug)]
pub(crate) enum NightLightOutput {}

pub(crate) struct NightLightInit {}

#[derive(Debug)]
pub(crate) enum NightLightCommandOutput {
    EnabledChanged(bool),
}

#[relm4::component(pub)]
impl Component for NightLightModel {
    type CommandOutput = NightLightCommandOutput;
    type Input = NightLightInput;
    type Output = NightLightOutput;
    type Init = NightLightInit;

    view! {
        #[root]
        gtk::Box {
            #[name = "button"]
            gtk::Button {
                #[watch]
                set_css_classes: if model.enabled {
                    &["ok-button-surface", "ok-button-medium", "selected"]
                } else {
                    &["ok-button-surface", "ok-button-medium"]
                },
                set_hexpand: false,
                set_vexpand: false,
                connect_clicked[sender] => move |_| {
                    sender.input(NightLightInput::Clicked);
                },

                #[name = "action_icon_image"]
                gtk::Image {
                    set_hexpand: true,
                    set_vexpand: true,
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,
                    set_icon_name: Some("nightlight-symbolic"),
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        // Poll `mctl twilight status` so the button tracks the
        // schedule (and any external `mctl twilight` calls), not
        // just our own clicks.
        sender.command(|out, shutdown| {
            async move {
                let shutdown_fut = shutdown.wait();
                tokio::pin!(shutdown_fut);
                let mut first = true;
                loop {
                    let delay = if first { STARTUP_DELAY } else { POLL_INTERVAL };
                    first = false;
                    tokio::select! {
                        () = &mut shutdown_fut => break,
                        _ = tokio::time::sleep(delay) => {}
                    }
                    if let Some(enabled) = probe_twilight_enabled().await {
                        let _ = out.send(NightLightCommandOutput::EnabledChanged(enabled));
                    }
                }
            }
        });

        let model = NightLightModel { enabled: false };
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            NightLightInput::Clicked => {
                sender.command(|out, _shutdown| async move {
                    run_twilight_toggle().await;
                    tokio::time::sleep(POST_TOGGLE_DELAY).await;
                    if let Some(enabled) = probe_twilight_enabled().await {
                        let _ = out.send(NightLightCommandOutput::EnabledChanged(enabled));
                    }
                });
            }
        }
    }

    fn update_cmd(
        &mut self,
        message: Self::CommandOutput,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            NightLightCommandOutput::EnabledChanged(enabled) => {
                self.enabled = enabled;
            }
        }
    }
}

/// `mctl twilight status --json` → the `enabled` flag. `None` when
/// `mctl` is missing, the compositor isn't reachable, or the JSON
/// doesn't parse.
async fn probe_twilight_enabled() -> Option<bool> {
    let out = tokio::process::Command::new("mctl")
        .args(["twilight", "status", "--json"])
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    v.get("enabled")?.as_bool()
}

async fn run_twilight_toggle() {
    match tokio::process::Command::new("mctl")
        .args(["twilight", "toggle"])
        .status()
        .await
    {
        Ok(s) if s.success() => {}
        Ok(s) => warn!(?s, "mctl twilight toggle returned non-zero"),
        Err(e) => warn!(error = %e, "mctl twilight toggle spawn failed"),
    }
}
