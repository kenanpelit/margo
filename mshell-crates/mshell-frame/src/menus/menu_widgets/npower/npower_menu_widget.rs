//! Power profile menu widget — content surface for
//! `MenuType::Npower`.
//!
//! Layout:
//!   * **Hero** — battery ring (percentage + charge status) +
//!     power-source line, or just the profile name when there's
//!     no battery (desktop).
//!   * **Profile switcher** — three big buttons (Power Saver /
//!     Balanced / Performance). The active one carries the
//!     `.selected` class plus its colour-state class
//!     (`.profile-saver` green / `.profile-performance` red /
//!     `.profile-balanced` neutral).
//!
//! Switching shells out to `powerprofilesctl set <id>` — an
//! unprivileged call against the per-session power-profiles-
//! daemon, no pkexec needed. Each switch triggers an immediate
//! re-probe so the highlight tracks reality.

use crate::bars::bar_widgets::npower::{PowerState, Profile, probe_power_state};
use relm4::gtk::prelude::{BoxExt, ButtonExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::Duration;
use tracing::warn;

const REFRESH_INTERVAL: Duration = Duration::from_secs(8);
const STARTUP_DELAY: Duration = Duration::from_millis(200);
const POST_ACTION_DELAY: Duration = Duration::from_millis(400);

pub(crate) struct NpowerMenuWidgetModel {
    state: PowerState,
    hero_icon: gtk::Image,
    hero_title: gtk::Label,
    hero_subtitle: gtk::Label,
    /// Profile buttons keyed by their Profile so `sync_view` can
    /// flip `.selected` + the colour-state class onto the
    /// active one.
    profile_buttons: Vec<(Profile, gtk::Button)>,
}

impl std::fmt::Debug for NpowerMenuWidgetModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NpowerMenuWidgetModel")
            .field("state", &self.state)
            .finish()
    }
}

#[derive(Debug)]
pub(crate) enum NpowerMenuWidgetInput {
    SetProfile(Profile),
    RefreshNow,
}

#[derive(Debug)]
pub(crate) enum NpowerMenuWidgetOutput {}

pub(crate) struct NpowerMenuWidgetInit {}

#[derive(Debug)]
pub(crate) enum NpowerMenuWidgetCommandOutput {
    Refreshed(PowerState),
}

#[relm4::component(pub(crate))]
impl Component for NpowerMenuWidgetModel {
    type CommandOutput = NpowerMenuWidgetCommandOutput;
    type Input = NpowerMenuWidgetInput;
    type Output = NpowerMenuWidgetOutput;
    type Init = NpowerMenuWidgetInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "npower-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 10,

            // ── Hero ────────────────────────────────────────────
            gtk::Box {
                add_css_class: "npower-hero",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 10,

                #[local_ref]
                hero_icon_widget -> gtk::Image {
                    set_pixel_size: 32,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_hexpand: true,

                    #[local_ref]
                    hero_title_widget -> gtk::Label {
                        add_css_class: "label-large-bold",
                        set_xalign: 0.0,
                    },
                    #[local_ref]
                    hero_subtitle_widget -> gtk::Label {
                        add_css_class: "label-small",
                        set_xalign: 0.0,
                    },
                },
            },

            gtk::Separator { set_orientation: gtk::Orientation::Horizontal },

            gtk::Label {
                add_css_class: "label-medium-bold",
                set_label: "Power profile",
                set_xalign: 0.0,
            },

            // ── Profile switcher ────────────────────────────────
            #[local_ref]
            profile_box -> gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 6,
                set_homogeneous: true,
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let hero_icon_widget = gtk::Image::from_icon_name("power-profile-balanced-symbolic");
        let hero_title_widget = gtk::Label::new(Some("Power"));
        let hero_subtitle_widget = gtk::Label::new(Some("…"));

        let profile_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let mut profile_buttons: Vec<(Profile, gtk::Button)> = Vec::with_capacity(3);
        for profile in [
            Profile::PowerSaver,
            Profile::Balanced,
            Profile::Performance,
        ] {
            let btn = make_profile_button(profile);
            let s = sender.clone();
            btn.connect_clicked(move |_| s.input(NpowerMenuWidgetInput::SetProfile(profile)));
            profile_box.append(&btn);
            profile_buttons.push((profile, btn));
        }

        sender.command(|out, shutdown| {
            async move {
                let shutdown_fut = shutdown.wait();
                tokio::pin!(shutdown_fut);
                let mut first = true;
                loop {
                    let delay = if first { STARTUP_DELAY } else { REFRESH_INTERVAL };
                    first = false;
                    tokio::select! {
                        () = &mut shutdown_fut => break,
                        _ = tokio::time::sleep(delay) => {}
                    }
                    let s = probe_power_state().await;
                    let _ = out.send(NpowerMenuWidgetCommandOutput::Refreshed(s));
                }
            }
        });

        let model = NpowerMenuWidgetModel {
            state: PowerState::default(),
            hero_icon: hero_icon_widget.clone(),
            hero_title: hero_title_widget.clone(),
            hero_subtitle: hero_subtitle_widget.clone(),
            profile_buttons,
        };

        let widgets = view_output!();
        sync_view(&model);

        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            NpowerMenuWidgetInput::SetProfile(profile) => {
                let id = profile.ppd_id().to_string();
                sender.command(move |out, _shutdown| async move {
                    let status = tokio::process::Command::new("powerprofilesctl")
                        .arg("set")
                        .arg(&id)
                        .status()
                        .await;
                    match status {
                        Ok(s) if s.success() => {}
                        Ok(s) => warn!(?s, id, "powerprofilesctl set returned non-zero"),
                        Err(e) => warn!(error = %e, id, "powerprofilesctl spawn failed"),
                    }
                    tokio::time::sleep(POST_ACTION_DELAY).await;
                    let s = probe_power_state().await;
                    let _ = out.send(NpowerMenuWidgetCommandOutput::Refreshed(s));
                });
            }
            NpowerMenuWidgetInput::RefreshNow => {
                sender.command(|out, _shutdown| async move {
                    let s = probe_power_state().await;
                    let _ = out.send(NpowerMenuWidgetCommandOutput::Refreshed(s));
                });
            }
        }
    }

    fn update_cmd_with_view(
        &mut self,
        _widgets: &mut Self::Widgets,
        message: Self::CommandOutput,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            NpowerMenuWidgetCommandOutput::Refreshed(state) => {
                if self.state != state {
                    self.state = state;
                    sync_view(self);
                }
            }
        }
    }
}

fn make_profile_button(profile: Profile) -> gtk::Button {
    let inner = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .halign(gtk::Align::Center)
        .build();
    let img = gtk::Image::from_icon_name(profile.icon());
    img.set_pixel_size(22);
    inner.append(&img);
    let label = gtk::Label::new(Some(profile.label()));
    label.add_css_class("label-small-bold");
    inner.append(&label);
    gtk::Button::builder()
        .child(&inner)
        .css_classes(vec!["ok-button-surface", "npower-profile-button"])
        .hexpand(true)
        .build()
}

fn sync_view(model: &NpowerMenuWidgetModel) {
    let s = &model.state;
    let profile = s.profile.unwrap_or(Profile::Unknown);

    // Hero: battery-centric on a laptop, profile-centric on a
    // desktop with no battery.
    if s.battery_available {
        if let Some(pct) = s.battery_percent {
            model
                .hero_icon
                .set_icon_name(Some(battery_icon(pct, &s.battery_status)));
            model.hero_title.set_label(&format!("{pct}%"));
            model.hero_subtitle.set_label(&format!(
                "{}  ·  {}",
                if s.battery_status.is_empty() {
                    "unknown"
                } else {
                    &s.battery_status
                },
                match s.power_source.as_str() {
                    "ac" => "on AC",
                    "battery" => "on battery",
                    _ => "unknown source",
                }
            ));
        }
    } else {
        model.hero_icon.set_icon_name(Some(profile.icon()));
        model.hero_title.set_label(profile.label());
        model.hero_subtitle.set_label("Desktop · no battery");
    }

    // Profile buttons — active one gets `.selected` + its
    // colour-state class.
    for (p, btn) in &model.profile_buttons {
        let mut classes = vec!["ok-button-surface", "npower-profile-button"];
        if Some(*p) == s.profile {
            classes.push("selected");
            classes.push(p.css_class());
        }
        btn.set_css_classes(&classes);
    }
}

/// Pick a battery glyph from the OkMaterial `battery-level-*`
/// set — 0/10/20…100, with the `-charging` variant when the
/// status says so.
fn battery_icon(pct: u8, status: &str) -> &'static str {
    let charging = status.eq_ignore_ascii_case("charging")
        || status.eq_ignore_ascii_case("full");
    let bucket = match pct {
        0..=4 => 0,
        5..=14 => 10,
        15..=24 => 20,
        25..=34 => 30,
        35..=44 => 40,
        45..=54 => 50,
        55..=64 => 60,
        65..=74 => 70,
        75..=84 => 80,
        85..=94 => 90,
        _ => 100,
    };
    // OkMaterial ships `battery-level-{N}[-charging]-symbolic`.
    match (bucket, charging) {
        (0, false) => "battery-level-0-symbolic",
        (0, true) => "battery-level-0-charging-symbolic",
        (10, false) => "battery-level-10-symbolic",
        (10, true) => "battery-level-10-charging-symbolic",
        (20, false) => "battery-level-20-symbolic",
        (20, true) => "battery-level-20-charging-symbolic",
        (30, false) => "battery-level-30-symbolic",
        (30, true) => "battery-level-30-charging-symbolic",
        (40, false) => "battery-level-40-symbolic",
        (40, true) => "battery-level-40-charging-symbolic",
        (50, false) => "battery-level-50-symbolic",
        (50, true) => "battery-level-50-charging-symbolic",
        (60, false) => "battery-level-60-symbolic",
        (60, true) => "battery-level-60-charging-symbolic",
        (70, false) => "battery-level-70-symbolic",
        (70, true) => "battery-level-70-charging-symbolic",
        (80, false) => "battery-level-80-symbolic",
        (80, true) => "battery-level-80-charging-symbolic",
        (90, false) => "battery-level-90-symbolic",
        (90, true) => "battery-level-90-charging-symbolic",
        (_, false) => "battery-level-100-symbolic",
        (_, true) => "battery-level-100-charging-symbolic",
    }
}
