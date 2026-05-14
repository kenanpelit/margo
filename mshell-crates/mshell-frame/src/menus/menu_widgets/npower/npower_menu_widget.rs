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
//!   * **Power Controls** — three secondary actions ported from
//!     the noctalia `npower` panel:
//!       - **Cycle** — step to the next profile.
//!       - **Lock Auto** — toggle the `ppd-auto-profile` lock
//!         file (`~/.local/state/ppd-auto-profile/lock`), the
//!         same flag noctalia's auto-profile timer honours.
//!       - **Idle Toggle** — flip margo's idle inhibitor via the
//!         shared `mshell_idle::IdleInhibitor` (same path the
//!         quick-action coffee button uses).
//!
//! Profile switching goes through `PowerProfilesService::
//! set_active_profile` (power-profiles-daemon over D-Bus); the
//! `active_profile` watcher then fires `StateChanged` so the
//! highlight tracks reality with no re-probe.

use crate::bars::bar_widgets::npower::{PowerState, Profile, read_power_state};
use mshell_idle::inhibitor::IdleInhibitor;
use mshell_services::power_profile_service;
use mshell_utils::battery::{spawn_battery_online_watcher, spawn_battery_watcher};
use mshell_utils::idle::spawn_idle_inhibitor_watcher;
use mshell_utils::power_profile::spawn_active_profile_watcher;
use relm4::gtk::prelude::{BoxExt, ButtonExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::path::PathBuf;
use tracing::warn;

pub(crate) struct NpowerMenuWidgetModel {
    state: PowerState,
    /// `ppd-auto-profile` lock-file present — auto-profile
    /// switching is pinned off.
    auto_locked: bool,
    /// margo idle inhibitor currently engaged.
    idle_inhibited: bool,
    hero_icon: gtk::Image,
    hero_title: gtk::Label,
    hero_subtitle: gtk::Label,
    /// Profile buttons keyed by their Profile so `sync_view` can
    /// flip `.selected` + the colour-state class onto the
    /// active one.
    profile_buttons: Vec<(Profile, gtk::Button)>,
    /// Power-control buttons whose label / state tracks runtime
    /// state — kept as refs so `sync_view` can re-style them.
    lock_auto_button: gtk::Button,
    lock_auto_icon: gtk::Image,
    lock_auto_label: gtk::Label,
    idle_button: gtk::Button,
}

impl std::fmt::Debug for NpowerMenuWidgetModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NpowerMenuWidgetModel")
            .field("state", &self.state)
            .field("auto_locked", &self.auto_locked)
            .field("idle_inhibited", &self.idle_inhibited)
            .finish()
    }
}

#[derive(Debug)]
pub(crate) enum NpowerMenuWidgetInput {
    SetProfile(Profile),
    CycleProfile,
    ToggleAutoLock,
    ToggleIdleInhibit,
}

#[derive(Debug)]
pub(crate) enum NpowerMenuWidgetOutput {}

pub(crate) struct NpowerMenuWidgetInit {}

#[derive(Debug)]
pub(crate) enum NpowerMenuWidgetCommandOutput {
    /// Profile or battery state changed (a D-Bus watcher fired).
    StateChanged,
    /// The `ppd-auto-profile` lock-file state changed — re-read
    /// after our own toggle (and once at startup).
    AutoLockChanged(bool),
    /// The shared idle inhibitor flipped (watcher or our own
    /// toggle) — re-read `IdleInhibitor::global()`.
    IdleStateChanged,
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

            gtk::Separator { set_orientation: gtk::Orientation::Horizontal },

            gtk::Label {
                add_css_class: "label-medium-bold",
                set_label: "Power controls",
                set_xalign: 0.0,
            },

            // ── Power controls ──────────────────────────────────
            #[local_ref]
            controls_box -> gtk::Box {
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

        // ── Power-control buttons ───────────────────────────────
        let controls_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);

        let (cycle_button, _, _) =
            make_control_button("media-playlist-shuffle-symbolic", "Cycle");
        {
            let s = sender.clone();
            cycle_button.connect_clicked(move |_| s.input(NpowerMenuWidgetInput::CycleProfile));
        }
        controls_box.append(&cycle_button);

        let (lock_auto_button, lock_auto_icon, lock_auto_label) =
            make_control_button("changes-allow-symbolic", "Lock Auto");
        {
            let s = sender.clone();
            lock_auto_button
                .connect_clicked(move |_| s.input(NpowerMenuWidgetInput::ToggleAutoLock));
        }
        controls_box.append(&lock_auto_button);

        let (idle_button, _, _) = make_control_button("coffee-symbolic", "Idle Toggle");
        {
            let s = sender.clone();
            idle_button.connect_clicked(move |_| s.input(NpowerMenuWidgetInput::ToggleIdleInhibit));
        }
        controls_box.append(&idle_button);

        // Reactive — profile from power-profiles-daemon, battery
        // from UPower, both over D-Bus. No polling.
        spawn_active_profile_watcher(&sender, None, || {
            NpowerMenuWidgetCommandOutput::StateChanged
        });
        spawn_battery_watcher(&sender, || NpowerMenuWidgetCommandOutput::StateChanged);
        spawn_battery_online_watcher(&sender, || NpowerMenuWidgetCommandOutput::StateChanged);

        // The `ppd-auto-profile` lock-file has no change signal;
        // read it once at startup and again after our own toggle.
        sender.command(|out, _shutdown| async move {
            let _ = out.send(NpowerMenuWidgetCommandOutput::AutoLockChanged(
                probe_auto_locked().await,
            ));
        });

        // Idle inhibitor watcher — same shared global the
        // quick-action coffee button drives.
        spawn_idle_inhibitor_watcher(&sender, || NpowerMenuWidgetCommandOutput::IdleStateChanged);

        let model = NpowerMenuWidgetModel {
            state: read_power_state(),
            auto_locked: false,
            idle_inhibited: IdleInhibitor::global().get(),
            hero_icon: hero_icon_widget.clone(),
            hero_title: hero_title_widget.clone(),
            hero_subtitle: hero_subtitle_widget.clone(),
            profile_buttons,
            lock_auto_button: lock_auto_button.clone(),
            lock_auto_icon,
            lock_auto_label,
            idle_button: idle_button.clone(),
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
                // The `active_profile` watcher fires `StateChanged`
                // once the daemon applies it — no re-probe here.
                tokio::spawn(async move {
                    set_profile(profile).await;
                });
            }
            NpowerMenuWidgetInput::CycleProfile => {
                let next = cycle_next(self.state.profile.unwrap_or(Profile::Unknown));
                tokio::spawn(async move {
                    set_profile(next).await;
                });
            }
            NpowerMenuWidgetInput::ToggleAutoLock => {
                sender.command(move |out, _shutdown| async move {
                    toggle_auto_lock().await;
                    let _ = out.send(NpowerMenuWidgetCommandOutput::AutoLockChanged(
                        probe_auto_locked().await,
                    ));
                });
            }
            NpowerMenuWidgetInput::ToggleIdleInhibit => {
                // The shared watcher reports the new state back as
                // `IdleStateChanged`, so nothing to send here.
                tokio::spawn(async move {
                    let _ = IdleInhibitor::global().toggle().await;
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
            NpowerMenuWidgetCommandOutput::StateChanged => {
                let state = read_power_state();
                if self.state != state {
                    self.state = state;
                    sync_view(self);
                }
            }
            NpowerMenuWidgetCommandOutput::AutoLockChanged(auto_locked) => {
                if self.auto_locked != auto_locked {
                    self.auto_locked = auto_locked;
                    sync_view(self);
                }
            }
            NpowerMenuWidgetCommandOutput::IdleStateChanged => {
                let inhibited = IdleInhibitor::global().get();
                if self.idle_inhibited != inhibited {
                    self.idle_inhibited = inhibited;
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

/// A secondary "Power controls" button (Cycle / Lock Auto / Idle
/// Toggle). Returns the button plus its icon + label so callers
/// whose state changes at runtime can re-style them.
fn make_control_button(icon: &str, text: &str) -> (gtk::Button, gtk::Image, gtk::Label) {
    let inner = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .halign(gtk::Align::Center)
        .build();
    let img = gtk::Image::from_icon_name(icon);
    img.set_pixel_size(20);
    inner.append(&img);
    let label = gtk::Label::new(Some(text));
    label.add_css_class("label-small-bold");
    inner.append(&label);
    let button = gtk::Button::builder()
        .child(&inner)
        .css_classes(vec!["ok-button-surface", "npower-control-button"])
        .hexpand(true)
        .build();
    (button, img, label)
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

    // Lock Auto — label + icon + `.selected` flip on the lock
    // state.
    if model.auto_locked {
        model.lock_auto_label.set_label("Unlock Auto");
        model
            .lock_auto_icon
            .set_icon_name(Some("changes-prevent-symbolic"));
        model
            .lock_auto_button
            .set_css_classes(&["ok-button-surface", "npower-control-button", "selected"]);
    } else {
        model.lock_auto_label.set_label("Lock Auto");
        model
            .lock_auto_icon
            .set_icon_name(Some("changes-allow-symbolic"));
        model
            .lock_auto_button
            .set_css_classes(&["ok-button-surface", "npower-control-button"]);
    }

    // Idle Toggle — `.selected` while the inhibitor is engaged.
    if model.idle_inhibited {
        model
            .idle_button
            .set_css_classes(&["ok-button-surface", "npower-control-button", "selected"]);
    } else {
        model
            .idle_button
            .set_css_classes(&["ok-button-surface", "npower-control-button"]);
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

/// Next profile in the Saver → Balanced → Performance → Saver
/// cycle. `Unknown` falls into the cycle at Balanced.
fn cycle_next(current: Profile) -> Profile {
    match current {
        Profile::PowerSaver => Profile::Balanced,
        Profile::Balanced => Profile::Performance,
        Profile::Performance => Profile::PowerSaver,
        Profile::Unknown => Profile::Balanced,
    }
}

/// `~/.local/state/ppd-auto-profile/lock` — the flag noctalia's
/// `npower` shares with its auto-profile timer.
fn auto_lock_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(
        PathBuf::from(home)
            .join(".local/state/ppd-auto-profile")
            .join("lock"),
    )
}

async fn probe_auto_locked() -> bool {
    match auto_lock_path() {
        Some(p) => tokio::fs::try_exists(&p).await.unwrap_or(false),
        None => false,
    }
}

/// Create the lock file when absent, remove it when present —
/// the same create/remove toggle noctalia's `action.sh` does.
async fn toggle_auto_lock() {
    let Some(path) = auto_lock_path() else {
        warn!("npower: $HOME unset, cannot toggle auto-profile lock");
        return;
    };
    match tokio::fs::try_exists(&path).await {
        Ok(true) => {
            if let Err(e) = tokio::fs::remove_file(&path).await {
                warn!(error = %e, "npower: failed to clear auto-profile lock");
            }
        }
        Ok(false) => {
            if let Some(parent) = path.parent() {
                if let Err(e) = tokio::fs::create_dir_all(parent).await {
                    warn!(error = %e, "npower: failed to create auto-profile lock dir");
                    return;
                }
            }
            if let Err(e) = tokio::fs::write(&path, b"").await {
                warn!(error = %e, "npower: failed to set auto-profile lock");
            }
        }
        Err(e) => warn!(error = %e, "npower: cannot stat auto-profile lock file"),
    }
}

/// Apply a profile through power-profiles-daemon over D-Bus.
async fn set_profile(profile: Profile) {
    if let Err(e) = power_profile_service()
        .power_profiles
        .set_active_profile(profile.to_wayle())
        .await
    {
        warn!(error = %e, ?profile, "npower: set_active_profile failed");
    }
}
