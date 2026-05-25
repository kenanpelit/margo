//! Twilight menu widget — the panel content for `MenuType::Twilight`.
//!
//! Quick controls for margo's blue-light filter: a master on/off, the
//! live temperature / phase / mode readout, a source-mode selector
//! (Auto / Manual / Static / Schedule), a temperature slider that
//! previews live, and "Resume schedule" to drop any preview override.
//! Everything routes through `mctl twilight …` (see [`crate::twilight`])
//! — the rich schedule-preset editor lives in Settings → Display.

use crate::twilight::{self, TwilightStatus};
use relm4::gtk::glib;
use relm4::gtk::prelude::{
    BoxExt, ButtonExt, FlowBoxChildExt, OrientableExt, RangeExt, ScaleExt, WidgetExt,
};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::{Duration, Instant};

/// Poll cadence while the panel is open.
const POLL: Duration = Duration::from_secs(2);
/// Trailing debounce on the temperature slider so a drag lands one
/// `preview`, not one per pixel.
const TEMP_DEBOUNCE_MS: u64 = 120;
/// After a user action (mode / toggle), ignore poll results this long
/// so the optimistic UI isn't clobbered by an in-flight stale read
/// before margo has applied the change + refreshed state.json.
const SETTLE: Duration = Duration::from_millis(1200);

pub(crate) struct TwilightMenuWidgetModel {
    status: TwilightStatus,
    temp_debounce: Option<glib::JoinHandle<()>>,
    /// Polls landing before this instant are dropped (settle window).
    settle_until: Instant,
    /// Schedule presets in time order (from `schedule.conf`).
    presets: Vec<twilight::Preset>,
    /// Chip buttons, parallel to `presets`, so the currently-scheduled
    /// one can be tinted active without rebuilding the grid.
    preset_buttons: Vec<gtk::Button>,
    /// Source-mode tiles keyed by mode id (`geo`/`manual`/`static`/
    /// `schedule`), so the active one gets `.selected` without a rebuild —
    /// same pattern as the power-profile buttons.
    mode_buttons: Vec<(&'static str, gtk::Button)>,
}

#[derive(Debug)]
pub(crate) enum TwilightMenuWidgetInput {
    /// Poll result.
    Refresh(TwilightStatus),
    /// Header toggle — flip the filter on/off.
    Toggle,
    /// Switch the source mode (`geo` / `manual` / `static` / `schedule`).
    SetMode(&'static str),
    /// Slider moved — debounce into a live `preview`.
    TempChanged(u32),
    /// Debounce fired — pin the previewed temperature.
    TempCommit(u32),
    /// Drop any preview/test override and resume the schedule.
    Reset,
    /// Apply a schedule preset's look immediately (live preview).
    ApplyPreset { temp: u32, gamma: u32 },
}

#[derive(Debug)]
pub(crate) enum TwilightMenuWidgetOutput {}

pub(crate) struct TwilightMenuWidgetInit {}

#[derive(Debug)]
pub(crate) enum TwilightMenuWidgetCommandOutput {
    Polled(TwilightStatus),
}

#[relm4::component(pub(crate))]
impl Component for TwilightMenuWidgetModel {
    type CommandOutput = TwilightMenuWidgetCommandOutput;
    type Input = TwilightMenuWidgetInput;
    type Output = TwilightMenuWidgetOutput;
    type Init = TwilightMenuWidgetInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "twilight-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 12,

            // ── §12 panel header ──
            gtk::Box {
                add_css_class: "panel-header",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 12,
                gtk::Image {
                    add_css_class: "panel-header-icon",
                    set_valign: gtk::Align::Center,
                    #[watch]
                    set_icon_name: Some(model.status.icon()),
                },
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_hexpand: true,
                    gtk::Label {
                        add_css_class: "panel-title",
                        set_halign: gtk::Align::Start,
                        set_label: "Twilight",
                    },
                    gtk::Label {
                        add_css_class: "label-small",
                        set_halign: gtk::Align::Start,
                        set_xalign: 0.0,
                        #[watch]
                        set_label: &status_line(&model.status),
                    },
                },
                gtk::Button {
                    set_valign: gtk::Align::Center,
                    #[watch]
                    set_css_classes: if model.status.enabled {
                        &["ok-button-surface", "selected"]
                    } else {
                        &["ok-button-surface"]
                    },
                    #[watch]
                    set_label: if model.status.enabled { "On" } else { "Off" },
                    connect_clicked[sender] => move |_| {
                        sender.input(TwilightMenuWidgetInput::Toggle);
                    },
                },
            },

            // Source-mode selector — power-profile-style icon tiles
            // (built imperatively in `init`, kept in `mode_buttons`).
            #[local_ref]
            mode_box -> gtk::Box {
                add_css_class: "twilight-mode-row",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 6,
                set_homogeneous: true,
            },

            // Temperature slider — previews live (pins until Reset).
            gtk::Label {
                add_css_class: "twilight-section-label",
                set_halign: gtk::Align::Start,
                set_label: "Temperature",
            },
            gtk::Scale {
                add_css_class: "twilight-temp-scale",
                set_orientation: gtk::Orientation::Horizontal,
                set_hexpand: true,
                // Set range/value BEFORE wiring the handler so seeding
                // the slider on open doesn't fire a spurious preview.
                set_range: (1000.0, 6500.0),
                set_increments: (100.0, 500.0),
                set_value: 4000.0,
                set_draw_value: true,
                // Whole kelvin only — no spurious "3035.7" decimal.
                set_digits: 0,
                set_value_pos: gtk::PositionType::Right,
                connect_value_changed[sender] => move |s| {
                    sender.input(TwilightMenuWidgetInput::TempChanged(s.value() as u32));
                },
            },

            // Schedule presets — populated imperatively in `init` (a
            // chip per preset that previews its temperature/gamma).
            // Hidden when there are no presets on disk.
            #[name = "presets_section"]
            gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 6,
                set_visible: false,

                gtk::Label {
                    add_css_class: "twilight-section-label",
                    set_halign: gtk::Align::Start,
                    set_label: "Presets",
                },
                #[name = "presets_grid"]
                gtk::FlowBox {
                    add_css_class: "twilight-preset-grid",
                    set_selection_mode: gtk::SelectionMode::None,
                    set_homogeneous: true,
                    // Two-line value tiles need more width than the old
                    // name-only pills, so cap at 3 per line.
                    set_min_children_per_line: 2,
                    set_max_children_per_line: 3,
                    set_row_spacing: 6,
                    set_column_spacing: 6,
                },
            },

            gtk::Button {
                add_css_class: "ok-button-surface",
                add_css_class: "ok-button-cell",
                set_label: "Resume schedule",
                set_tooltip_text: Some("Drop any preview and follow the schedule again"),
                connect_clicked[sender] => move |_| {
                    sender.input(TwilightMenuWidgetInput::Reset);
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        // Poll `mctl twilight status` while the panel is open so the
        // readout + selected mode track the schedule and our actions.
        sender.command(|out, shutdown| async move {
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            let mut first = true;
            loop {
                let delay = if first {
                    Duration::from_millis(50)
                } else {
                    POLL
                };
                first = false;
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    _ = tokio::time::sleep(delay) => {}
                }
                if let Some(s) = twilight::probe().await {
                    let _ = out.send(TwilightMenuWidgetCommandOutput::Polled(s));
                }
            }
        });

        // Source-mode tiles (icon + label), built like the power-profile
        // buttons so each carries an icon; `mode_box` is consumed by the
        // `#[local_ref]` in the view.
        let mode_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let mut mode_buttons: Vec<(&'static str, gtk::Button)> = Vec::with_capacity(4);
        for (mode, icon, label) in [
            ("geo", "weather-sunset-symbolic", "Auto"),
            ("manual", "document-edit-symbolic", "Manual"),
            ("static", "nightlight-symbolic", "Static"),
            ("schedule", "timer-symbolic", "Schedule"),
        ] {
            let btn = make_mode_button(icon, label);
            let s = sender.clone();
            btn.connect_clicked(move |_| s.input(TwilightMenuWidgetInput::SetMode(mode)));
            mode_box.append(&btn);
            mode_buttons.push((mode, btn));
        }

        let mut model = TwilightMenuWidgetModel {
            status: TwilightStatus::default(),
            temp_debounce: None,
            settle_until: Instant::now(),
            presets: Vec::new(),
            preset_buttons: Vec::new(),
            mode_buttons,
        };
        let widgets = view_output!();
        model.sync_modes();

        // Fill the preset grid from disk; hide the section if empty.
        let presets = twilight::load_presets();
        let mut buttons = Vec::with_capacity(presets.len());
        for p in &presets {
            let (child, btn) = preset_chip(p, &sender);
            widgets.presets_grid.insert(&child, -1);
            buttons.push(btn);
        }
        widgets.presets_section.set_visible(!presets.is_empty());
        model.presets = presets;
        model.preset_buttons = buttons;

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            TwilightMenuWidgetInput::Refresh(s) => {
                self.status = s;
            }
            TwilightMenuWidgetInput::Toggle => {
                let on = !self.status.enabled;
                self.status.enabled = on;
                self.settle_until = Instant::now() + SETTLE;
                twilight::run(vec![
                    "twilight".into(),
                    "set".into(),
                    format!("enabled={}", if on { 1 } else { 0 }),
                ]);
            }
            TwilightMenuWidgetInput::SetMode(mode) => {
                self.status.mode = mode.to_string();
                self.settle_until = Instant::now() + SETTLE;
                twilight::run(vec!["twilight".into(), "set".into(), format!("mode={mode}")]);
            }
            TwilightMenuWidgetInput::TempChanged(k) => {
                if let Some(h) = self.temp_debounce.take() {
                    h.abort();
                }
                let sender_clone = sender.clone();
                self.temp_debounce = Some(glib::spawn_future_local(async move {
                    glib::timeout_future(Duration::from_millis(TEMP_DEBOUNCE_MS)).await;
                    sender_clone.input(TwilightMenuWidgetInput::TempCommit(k));
                }));
            }
            TwilightMenuWidgetInput::TempCommit(k) => {
                self.temp_debounce = None;
                twilight::run(vec!["twilight".into(), "preview".into(), k.to_string()]);
            }
            TwilightMenuWidgetInput::Reset => {
                twilight::run(vec!["twilight".into(), "reset".into()]);
            }
            TwilightMenuWidgetInput::ApplyPreset { temp, gamma } => {
                // Live-preview the preset's look. Optimistically reflect
                // the temperature so the readout updates immediately.
                self.status.current_temp_k = Some(temp);
                self.status.current_gamma_pct = Some(gamma);
                self.settle_until = Instant::now() + SETTLE;
                twilight::run(vec![
                    "twilight".into(),
                    "preview".into(),
                    temp.to_string(),
                    gamma.to_string(),
                ]);
            }
        }
        self.refresh_active_chip();
        self.sync_modes();
    }

    fn update_cmd(
        &mut self,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            TwilightMenuWidgetCommandOutput::Polled(s) => {
                // Drop polls that land inside the settle window so an
                // in-flight stale read can't revert a just-made change.
                if Instant::now() >= self.settle_until {
                    sender.input(TwilightMenuWidgetInput::Refresh(s));
                }
            }
        }
    }
}

impl TwilightMenuWidgetModel {
    /// Tint the chip whose schedule slot is live right now — but only
    /// in schedule mode (see [`active_preset_index`]); clears the
    /// highlight in every other mode.
    fn refresh_active_chip(&self) {
        let active = active_preset_index(&self.presets, &self.status.mode);
        for (i, btn) in self.preset_buttons.iter().enumerate() {
            if Some(i) == active {
                btn.add_css_class("active");
            } else {
                btn.remove_css_class("active");
            }
        }
    }

    /// Flip `.selected` onto the active source-mode tile (the others stay
    /// on the plain surface).
    fn sync_modes(&self) {
        for (mode, btn) in &self.mode_buttons {
            if *mode == self.status.mode {
                btn.set_css_classes(&["ok-button-surface", "twilight-mode-button", "selected"]);
            } else {
                btn.set_css_classes(&["ok-button-surface", "twilight-mode-button"]);
            }
        }
    }
}

/// Index of the preset whose schedule slot contains "now" — only
/// meaningful in `schedule` mode and when every preset carries a time.
/// Presets are time-sorted; the active slot is the last one whose start
/// time has passed, wrapping to the final slot before the first time
/// (it runs from the previous evening through midnight).
fn active_preset_index(presets: &[twilight::Preset], mode: &str) -> Option<usize> {
    if mode != "schedule" || presets.is_empty() {
        return None;
    }
    let times: Vec<u32> = presets
        .iter()
        .map(|p| p.time.as_deref().and_then(parse_hhmm))
        .collect::<Option<Vec<_>>>()?;
    let now = glib::DateTime::now_local().ok()?;
    let now_min = now.hour() as u32 * 60 + now.minute() as u32;
    let mut active = times.len() - 1;
    for (i, &t) in times.iter().enumerate() {
        if t <= now_min {
            active = i;
        } else {
            break;
        }
    }
    Some(active)
}

/// `"HH:MM"` → minutes since midnight.
fn parse_hhmm(s: &str) -> Option<u32> {
    let (h, m) = s.trim().split_once(':')?;
    Some(h.trim().parse::<u32>().ok()? * 60 + m.trim().parse::<u32>().ok()?)
}

/// One preset chip — a two-line tile that surfaces the preset's actual
/// values (name on top, `<temp> K · <time>` below) instead of hiding
/// them in a tooltip, and previews the preset's temperature/gamma on
/// click. Returns the grid child plus the inner button (kept so the
/// active slot can be tinted later).
fn preset_chip(
    p: &twilight::Preset,
    sender: &ComponentSender<TwilightMenuWidgetModel>,
) -> (gtk::FlowBoxChild, gtk::Button) {
    let btn = gtk::Button::new();
    btn.add_css_class("twilight-preset-chip");
    btn.set_hexpand(true);

    let inner = gtk::Box::new(gtk::Orientation::Vertical, 0);
    inner.set_halign(gtk::Align::Center);

    let name = gtk::Label::new(Some(&p.name));
    name.add_css_class("twilight-preset-name");
    inner.append(&name);

    // The value line: temperature always, plus the schedule time when
    // the preset carries one (schedule presets do). This is what the
    // user reads to pick a slot — no longer tooltip-only.
    let value_text = match p.time.as_deref() {
        Some(t) if !t.is_empty() => format!("{} K · {}", p.temp_k, t),
        _ => format!("{} K", p.temp_k),
    };
    let value = gtk::Label::new(Some(&value_text));
    value.add_css_class("twilight-preset-value");
    inner.append(&value);

    btn.set_child(Some(&inner));
    btn.set_tooltip_text(Some(&format!(
        "{} K · {}% gamma{}",
        p.temp_k,
        p.gamma_pct,
        p.time
            .as_deref()
            .filter(|t| !t.is_empty())
            .map(|t| format!(" · {t}"))
            .unwrap_or_default(),
    )));
    {
        let sender = sender.clone();
        let (temp, gamma) = (p.temp_k, p.gamma_pct);
        btn.connect_clicked(move |_| {
            sender.input(TwilightMenuWidgetInput::ApplyPreset { temp, gamma });
        });
    }
    let child = gtk::FlowBoxChild::new();
    child.set_child(Some(&btn));
    child.set_focusable(false);
    (child, btn)
}

/// A source-mode tile — a vertical icon + label button matching the
/// power-profile buttons. The `.selected` state is applied later by
/// [`TwilightMenuWidgetModel::sync_modes`].
fn make_mode_button(icon: &str, label: &str) -> gtk::Button {
    let inner = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .halign(gtk::Align::Center)
        .build();
    let img = gtk::Image::from_icon_name(icon);
    img.set_pixel_size(22);
    inner.append(&img);
    let lbl = gtk::Label::new(Some(label));
    lbl.add_css_class("label-small-bold");
    inner.append(&lbl);
    gtk::Button::builder()
        .child(&inner)
        .css_classes(vec!["ok-button-surface", "twilight-mode-button"])
        .hexpand(true)
        .build()
}

/// "4200 K · Night · Schedule" — non-empty parts joined with " · ".
fn status_line(s: &TwilightStatus) -> String {
    if !s.enabled {
        return "Off — colours unfiltered".to_string();
    }
    let mut parts: Vec<String> = Vec::new();
    if let Some(k) = s.current_temp_k {
        parts.push(format!("{k} K"));
    }
    let phase = s.phase_label();
    if !phase.is_empty() {
        parts.push(phase.to_string());
    }
    parts.push(mode_label(&s.mode).to_string());
    parts.join(" · ")
}

fn mode_label(mode: &str) -> &'static str {
    match mode {
        "geo" => "Auto",
        "manual" => "Manual",
        "static" => "Static",
        "schedule" => "Schedule",
        _ => "—",
    }
}
