//! Settings → Monitors. A live display editor over margo's outputs:
//!
//! * an **arrangement map** — drag a screen next to another, or use the
//!   Left / Right / Above / Below buttons, instead of typing X/Y;
//! * per-output **resolution / refresh** dropdowns filled from the modes
//!   the output advertises, **rotation** and scale (no VRR switch: margo
//!   has no variable refresh and refuses `adaptive_sync`);
//! * **validate → apply → confirm**: the compositor test-applies the
//!   arrangement first (`wlr-randr --dryrun`), then it goes live and
//!   reverts by itself after [`CONFIRM_SECS`] unless kept. Keep writes it
//!   to `monitorrule` lines in `config.conf` so the next session starts
//!   the same way.
//!
//! The pure parts (parsing, placement, argument + rule generation, the
//! countdown) live in [`crate::monitors_arrange`]. Saved rules are listed
//! below with a Remove button, as before.

use crate::compositor_conf::{read_block, write_block};
use crate::monitors_arrange::{
    CONFIRM_SECS, ConfirmAction, ConfirmFlow, Head, Side, TRANSFORM_LABELS, merge_rules,
    nearest_other, parse_wlr_randr, place, side_for_drop, validate, wlr_randr_args,
};
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::Duration;

/// Editor canvas size (matches Layout → Arrange monitors).
const CANVAS_W: f64 = 470.0;
const CANVAS_H: f64 = 250.0;
const CANVAS_PAD: f64 = 12.0;

#[derive(Debug)]
pub(crate) enum MonitorsInput {
    /// Re-read the live outputs, discarding unapplied edits.
    Reload,
    Select(String),
    /// A tile was dropped with its centre at these layout coordinates.
    Dropped(String, f64, f64),
    Place(Side),
    SetResolution(u32),
    SetRate(u32),
    SetTransform(u32),
    SetScale(f64),
    Apply,
    Keep,
    Revert,
    /// Countdown second for apply generation `.0`.
    Tick(u64),
    Remove(usize),
}

#[derive(Debug)]
pub(crate) enum MonitorsOutput {}

#[derive(Debug)]
pub(crate) enum MonitorsCommandOutput {
    Loaded(Result<Vec<Head>, String>),
    /// The arrangement went live (or why it didn't).
    Applied(Result<(), String>),
    /// A revert finished.
    Reverted(Result<(), String>),
}

pub(crate) struct MonitorsInit {}

pub(crate) struct MonitorsModel {
    /// What the outputs are right now (last read / last applied).
    live: Vec<Head>,
    /// The editor copy.
    heads: Vec<Head>,
    selected: Option<String>,
    flow: ConfirmFlow,
    /// Bumped per apply so a stale countdown's ticks are ignored.
    apply_gen: u64,
    busy: bool,
    status: Option<Result<String, String>>,
    rules: Vec<String>,
    canvas: gtk::Fixed,
    controls: gtk::ListBox,
    list_box: gtk::ListBox,
}

impl MonitorsModel {
    fn selected_head(&self) -> Option<&Head> {
        let name = self.selected.as_deref()?;
        self.heads.iter().find(|h| h.name == name)
    }

    fn selected_head_mut(&mut self) -> Option<&mut Head> {
        let name = self.selected.clone()?;
        self.heads.iter_mut().find(|h| h.name == name)
    }

    fn dirty(&self) -> bool {
        self.heads != self.live
    }

    fn confirm_text(&self) -> String {
        match self.flow.seconds_left() {
            Some(s) => format!("Keep these display settings? Reverting in {s} s."),
            None => String::new(),
        }
    }

    fn status_text(&self) -> String {
        match &self.status {
            Some(Ok(m)) | Some(Err(m)) => m.clone(),
            None => String::new(),
        }
    }
}

fn spawn_load(sender: &ComponentSender<MonitorsModel>) {
    sender.oneshot_command(async {
        let out = tokio::process::Command::new("wlr-randr")
            .arg("--json")
            .output()
            .await;
        MonitorsCommandOutput::Loaded(match out {
            Ok(o) if o.status.success() => parse_wlr_randr(&String::from_utf8_lossy(&o.stdout)),
            Ok(o) => Err(format!(
                "wlr-randr failed: {}",
                String::from_utf8_lossy(&o.stderr).trim()
            )),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err("wlr-randr is not installed (Arch package: wlr-randr).".into())
            }
            Err(e) => Err(format!("wlr-randr: {e}")),
        })
    });
}

/// Run `wlr-randr` with `args`; `Err` carries its stderr.
async fn run_wlr_randr(args: Vec<String>) -> Result<(), String> {
    let out = tokio::process::Command::new("wlr-randr")
        .args(&args)
        .output()
        .await
        .map_err(|e| format!("wlr-randr: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if err.is_empty() {
            format!("wlr-randr: exit {}", out.status)
        } else {
            err
        })
    }
}

/// Validate with the compositor (`--dryrun`), then apply.
fn spawn_apply(sender: &ComponentSender<MonitorsModel>, heads: Vec<Head>) {
    sender.oneshot_command(async move {
        let args = wlr_randr_args(&heads);
        let mut dry = vec!["--dryrun".to_string()];
        dry.extend(args.iter().cloned());
        let result = match run_wlr_randr(dry).await {
            Err(e) => Err(format!("The compositor rejected this arrangement: {e}")),
            Ok(()) => run_wlr_randr(args).await,
        };
        MonitorsCommandOutput::Applied(result)
    });
}

fn spawn_revert(sender: &ComponentSender<MonitorsModel>, heads: Vec<Head>) {
    sender.oneshot_command(async move {
        MonitorsCommandOutput::Reverted(run_wlr_randr(wlr_randr_args(&heads)).await)
    });
}

/// One tick per second for the confirm countdown of apply `generation`.
fn spawn_countdown(sender: &ComponentSender<MonitorsModel>, generation: u64) {
    let s = sender.clone();
    relm4::spawn(async move {
        for _ in 0..=CONFIRM_SECS {
            tokio::time::sleep(Duration::from_secs(1)).await;
            s.input(MonitorsInput::Tick(generation));
        }
    });
}

/// Scale + offset that fit the enabled outputs into the canvas.
struct MapFit {
    min_x: f64,
    min_y: f64,
    scale: f64,
    off_x: f64,
    off_y: f64,
}

fn map_fit(heads: &[Head]) -> Option<MapFit> {
    let on: Vec<&Head> = heads.iter().filter(|h| h.enabled).collect();
    let min_x = on.iter().map(|h| h.x).min()? as f64;
    let min_y = on.iter().map(|h| h.y).min()? as f64;
    let max_x = on.iter().map(|h| h.x + h.logical_size().0.max(1)).max()? as f64;
    let max_y = on.iter().map(|h| h.y + h.logical_size().1.max(1)).max()? as f64;
    let span_w = (max_x - min_x).max(1.0);
    let span_h = (max_y - min_y).max(1.0);
    let scale =
        ((CANVAS_W - 2.0 * CANVAS_PAD) / span_w).min((CANVAS_H - 2.0 * CANVAS_PAD) / span_h);
    Some(MapFit {
        min_x,
        min_y,
        scale,
        off_x: (CANVAS_W - span_w * scale) / 2.0,
        off_y: (CANVAS_H - span_h * scale) / 2.0,
    })
}

/// (Re)build the draggable output tiles: the Layout page's editor tiles,
/// plus a click to select and a drop that snaps to the nearest output.
fn rebuild_canvas(model: &MonitorsModel, sender: &ComponentSender<MonitorsModel>) {
    let fixed = &model.canvas;
    while let Some(child) = fixed.first_child() {
        fixed.remove(&child);
    }
    let Some(fit) = map_fit(&model.heads) else {
        return;
    };
    for h in model.heads.iter().filter(|h| h.enabled) {
        let (lw, lh) = h.logical_size();
        let cx = fit.off_x + (f64::from(h.x) - fit.min_x) * fit.scale;
        let cy = fit.off_y + (f64::from(h.y) - fit.min_y) * fit.scale;
        let tw = (f64::from(lw) * fit.scale).max(28.0);
        let th = (f64::from(lh) * fit.scale).max(20.0);

        let tile = gtk::Box::new(gtk::Orientation::Vertical, 0);
        tile.add_css_class("layout-tile");
        if model.selected.as_deref() == Some(h.name.as_str()) {
            tile.add_css_class("selected");
        }
        tile.set_size_request(tw as i32, th as i32);
        tile.set_cursor_from_name(Some("grab"));
        let label = gtk::Label::new(None);
        label.set_justify(gtk::Justification::Center);
        label.set_halign(gtk::Align::Center);
        label.set_valign(gtk::Align::Center);
        label.set_hexpand(true);
        label.set_vexpand(true);
        label.set_markup(&format!(
            "{}\n<span size=\"x-small\">{}×{}</span>",
            gtk::glib::markup_escape_text(&h.name),
            lw,
            lh,
        ));
        tile.append(&label);
        mshell_common::a11y::name(&tile, &format!("{} — drag to place it", h.name));
        fixed.put(&tile, cx, cy);

        let drag = gtk::GestureDrag::new();
        {
            let f_up = fixed.clone();
            let t_up = tile.clone();
            drag.connect_drag_update(move |_, ox, oy| {
                f_up.move_(&t_up, cx + ox, cy + oy);
            });
        }
        {
            let name = h.name.clone();
            let s = sender.clone();
            let (min_x, min_y, scale, off_x, off_y) =
                (fit.min_x, fit.min_y, fit.scale, fit.off_x, fit.off_y);
            drag.connect_drag_end(move |_, ox, oy| {
                if ox.abs() < 3.0 && oy.abs() < 3.0 {
                    // A click, not a drag: select it.
                    s.input(MonitorsInput::Select(name.clone()));
                    return;
                }
                let lx = min_x + (cx + ox + tw / 2.0 - off_x) / scale;
                let ly = min_y + (cy + oy + th / 2.0 - off_y) / scale;
                s.input(MonitorsInput::Dropped(name.clone(), lx, ly));
            });
        }
        tile.add_controller(drag);
    }
}

/// A Settings row (the `Row` template's structure) built at runtime.
fn control_row(title: &str, desc: &str, control: &impl IsA<gtk::Widget>) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.set_selectable(false);
    row.set_activatable(false);
    let hbox = gtk::Box::new(gtk::Orientation::Horizontal, 20);
    hbox.add_css_class("action-row");
    let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
    text.set_hexpand(true);
    let t = gtk::Label::new(Some(title));
    t.add_css_class("label-medium-bold");
    t.set_halign(gtk::Align::Start);
    text.append(&t);
    if !desc.is_empty() {
        let d = gtk::Label::new(Some(desc));
        d.add_css_class("label-small");
        d.set_halign(gtk::Align::Start);
        d.set_xalign(0.0);
        d.set_wrap(true);
        d.set_natural_wrap_mode(gtk::NaturalWrapMode::None);
        text.append(&d);
    }
    hbox.append(&text);
    control.set_valign(gtk::Align::Center);
    hbox.append(control);
    row.set_child(Some(&hbox));
    row
}

fn dropdown(items: &[String], selected: usize) -> gtk::DropDown {
    let strs: Vec<&str> = items.iter().map(String::as_str).collect();
    let dd = gtk::DropDown::from_strings(&strs);
    dd.set_width_request(220);
    dd.set_selected(selected.min(items.len().saturating_sub(1)) as u32);
    dd
}

/// The selected output's controls: placement, resolution, refresh,
/// rotation, scale, VRR.
fn rebuild_controls(model: &MonitorsModel, sender: &ComponentSender<MonitorsModel>) {
    let list = &model.controls;
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    let Some(h) = model.selected_head() else {
        return;
    };
    let mode = h.mode;

    // Placement relative to the nearest other output.
    let (lw, lh) = h.logical_size();
    let anchor = nearest_other(
        &model.heads,
        &h.name,
        f64::from(h.x) + f64::from(lw) / 2.0,
        f64::from(h.y) + f64::from(lh) / 2.0,
    )
    .map(|a| a.name.clone());
    if let Some(anchor) = anchor {
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        for (icon, side, label) in [
            ("go-previous-symbolic", Side::Left, "Left of"),
            ("go-next-symbolic", Side::Right, "Right of"),
            ("go-up-symbolic", Side::Above, "Above"),
            ("go-down-symbolic", Side::Below, "Below"),
        ] {
            let b = gtk::Button::from_icon_name(icon);
            b.add_css_class("ok-button-surface");
            mshell_common::a11y::name(&b, &format!("Place {} {label} {anchor}", h.name));
            let s = sender.clone();
            b.connect_clicked(move |_| s.input(MonitorsInput::Place(side)));
            buttons.append(&b);
        }
        list.append(&control_row(
            "Position",
            &format!("Edge to edge with {anchor}, centred. Or drag it on the map."),
            &buttons,
        ));
    }

    // Resolution, from the advertised modes.
    let resolutions = h.resolutions();
    if !resolutions.is_empty() {
        let items: Vec<String> = resolutions
            .iter()
            .map(|(w, h)| format!("{w} × {h}"))
            .collect();
        let cur = mode
            .and_then(|m| resolutions.iter().position(|r| *r == (m.width, m.height)))
            .unwrap_or(0);
        let dd = dropdown(&items, cur);
        let s = sender.clone();
        dd.connect_selected_notify(move |d| s.input(MonitorsInput::SetResolution(d.selected())));
        list.append(&control_row(
            "Resolution",
            "Modes this display advertises.",
            &dd,
        ));

        if let Some(m) = mode {
            let rates = h.rates(m.width, m.height);
            let items: Vec<String> = rates.iter().map(|r| r.refresh_label()).collect();
            let cur = rates
                .iter()
                .position(|r| (r.refresh - m.refresh).abs() < 0.0005)
                .unwrap_or(0);
            let dd = dropdown(&items, cur);
            let s = sender.clone();
            dd.connect_selected_notify(move |d| s.input(MonitorsInput::SetRate(d.selected())));
            list.append(&control_row("Refresh rate", "", &dd));
        }
    }

    let items: Vec<String> = TRANSFORM_LABELS.iter().map(|s| s.to_string()).collect();
    let dd = dropdown(&items, h.transform);
    let s = sender.clone();
    dd.connect_selected_notify(move |d| s.input(MonitorsInput::SetTransform(d.selected())));
    list.append(&control_row(
        "Rotation",
        "90° and 270° turn the display portrait.",
        &dd,
    ));

    let spin = gtk::SpinButton::new(
        Some(&gtk::Adjustment::new(h.scale, 0.5, 4.0, 0.05, 0.25, 0.0)),
        0.05,
        2,
    );
    let s = sender.clone();
    spin.connect_value_changed(move |sp| s.input(MonitorsInput::SetScale(sp.value())));
    list.append(&control_row("Scale", "", &spin));
}

fn rebuild_list(model: &MonitorsModel, sender: &ComponentSender<MonitorsModel>) {
    while let Some(child) = model.list_box.first_child() {
        model.list_box.remove(&child);
    }
    if model.rules.is_empty() {
        let row = gtk::ListBoxRow::new();
        row.set_selectable(false);
        let lbl = gtk::Label::new(Some(
            "No saved monitor rules — outputs start with auto-detected modes.",
        ));
        lbl.add_css_class("label-small");
        lbl.set_halign(gtk::Align::Start);
        lbl.set_margin_top(8);
        lbl.set_margin_bottom(8);
        lbl.set_margin_start(8);
        row.set_child(Some(&lbl));
        model.list_box.append(&row);
        return;
    }
    for (i, payload) in model.rules.iter().enumerate() {
        let row = gtk::ListBoxRow::new();
        row.set_selectable(false);
        let hbox = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        hbox.set_margin_top(6);
        hbox.set_margin_bottom(6);
        hbox.set_margin_start(8);
        hbox.set_margin_end(8);
        let lbl = gtk::Label::new(Some(payload));
        lbl.set_halign(gtk::Align::Start);
        lbl.set_hexpand(true);
        lbl.set_xalign(0.0);
        lbl.set_wrap(true);
        lbl.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        lbl.set_natural_wrap_mode(gtk::NaturalWrapMode::None);
        lbl.set_selectable(true);
        lbl.add_css_class("label-medium");
        hbox.append(&lbl);
        let btn = gtk::Button::from_icon_name("user-trash-symbolic");
        btn.add_css_class("flat");
        btn.set_valign(gtk::Align::Center);
        mshell_common::a11y::name(&btn, "Remove this monitor rule");
        let s = sender.clone();
        btn.connect_clicked(move |_| s.input(MonitorsInput::Remove(i)));
        hbox.append(&btn);
        row.set_child(Some(&hbox));
        model.list_box.append(&row);
    }
}

#[relm4::component(pub)]
impl Component for MonitorsModel {
    type CommandOutput = MonitorsCommandOutput;
    type Input = MonitorsInput;
    type Output = MonitorsOutput;
    type Init = MonitorsInit;

    view! {
        gtk::ScrolledWindow {
            set_vscrollbar_policy: gtk::PolicyType::Automatic,
            set_hscrollbar_policy: gtk::PolicyType::Never,
            set_hexpand: true,
            set_vexpand: true,

            gtk::Box {
                add_css_class: "settings-page",
                set_orientation: gtk::Orientation::Vertical,
                set_hexpand: true,
                set_spacing: 16,

                gtk::Box {
                    add_css_class: "settings-hero",
                    set_spacing: 16,
                    gtk::Image {
                        add_css_class: "settings-hero-icon",
                        set_icon_name: Some("video-display-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label { add_css_class: "settings-hero-title", set_label: "Monitors", set_halign: gtk::Align::Start },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "Arrange displays and pick their mode, rotation and scale. Changes are checked by the compositor, applied live, and undone after 15 seconds unless you keep them; kept settings are saved as monitor rules.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                gtk::Label { add_css_class: "label-large-bold", set_label: "Arrangement", set_halign: gtk::Align::Start },
                gtk::Label {
                    add_css_class: "label-small",
                    set_label: "Click a display to edit it. Drag it next to another and it snaps edge to edge, centred.",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                },

                #[local_ref]
                canvas -> gtk::Fixed {
                    add_css_class: "layout-editor",
                    set_size_request: (CANVAS_W as i32, CANVAS_H as i32),
                    set_halign: gtk::Align::Center,
                },

                #[local_ref]
                controls -> gtk::ListBox {
                    add_css_class: "boxed-list",
                    set_selection_mode: gtk::SelectionMode::None,
                    #[watch]
                    set_visible: model.selected.is_some(),
                },

                gtk::Label {
                    add_css_class: "label-small",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    #[watch]
                    set_label: &model.status_text(),
                    set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                    #[watch]
                    set_visible: model.status.is_some(),
                },

                // Confirm bar: shown while an applied arrangement counts down.
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 8,
                    #[watch]
                    set_visible: model.flow.seconds_left().is_some(),
                    gtk::Label {
                        add_css_class: "label-medium-bold",
                        set_hexpand: true,
                        set_halign: gtk::Align::Start,
                        set_xalign: 0.0,
                        set_wrap: true,
                        #[watch]
                        set_label: &model.confirm_text(),
                    },
                    gtk::Button {
                        add_css_class: "ok-button-primary",
                        set_label: "Keep",
                        connect_clicked[sender] => move |_| sender.input(MonitorsInput::Keep),
                    },
                    gtk::Button {
                        add_css_class: "ok-button-surface",
                        set_label: "Revert",
                        connect_clicked[sender] => move |_| sender.input(MonitorsInput::Revert),
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 8,
                    set_halign: gtk::Align::Start,
                    #[watch]
                    set_visible: model.flow.seconds_left().is_none(),
                    gtk::Button {
                        add_css_class: "ok-button-primary",
                        set_label: "Apply",
                        set_tooltip_text: Some("Check with the compositor, then apply live."),
                        #[watch]
                        set_sensitive: model.dirty() && !model.busy,
                        connect_clicked[sender] => move |_| sender.input(MonitorsInput::Apply),
                    },
                    gtk::Button {
                        add_css_class: "ok-button-surface",
                        set_label: "Reset",
                        set_tooltip_text: Some("Discard edits and re-read the live displays."),
                        #[watch]
                        set_sensitive: !model.busy,
                        connect_clicked[sender] => move |_| sender.input(MonitorsInput::Reload),
                    },
                },

                gtk::Label { add_css_class: "label-large-bold", set_label: "Saved monitor rules", set_halign: gtk::Align::Start, set_margin_top: 8 },

                #[local_ref]
                list_box -> gtk::ListBox {
                    add_css_class: "boxed-list",
                    set_selection_mode: gtk::SelectionMode::None,
                },
            }
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = MonitorsModel {
            live: Vec::new(),
            heads: Vec::new(),
            selected: None,
            flow: ConfirmFlow::default(),
            apply_gen: 0,
            busy: true,
            status: None,
            rules: read_block("monitorrule"),
            canvas: gtk::Fixed::new(),
            controls: gtk::ListBox::new(),
            list_box: gtk::ListBox::new(),
        };
        let canvas = model.canvas.clone();
        let controls = model.controls.clone();
        let list_box = model.list_box.clone();
        let widgets = view_output!();
        rebuild_list(&model, &sender);
        spawn_load(&sender);
        let _ = root;
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        // Edits rebuild the map + controls; countdown ticks don't.
        let mut redraw = true;
        match message {
            MonitorsInput::Reload => {
                self.busy = true;
                spawn_load(&sender);
                redraw = false;
            }
            MonitorsInput::Select(name) => self.selected = Some(name),
            MonitorsInput::Dropped(name, cx, cy) => {
                self.selected = Some(name.clone());
                if let Some(anchor) = nearest_other(&self.heads, &name, cx, cy) {
                    let side = side_for_drop(anchor, cx, cy);
                    let anchor = anchor.name.clone();
                    place(&mut self.heads, &name, &anchor, side);
                }
            }
            MonitorsInput::Place(side) => {
                if let Some(h) = self.selected_head() {
                    let (w, hh) = h.logical_size();
                    let (cx, cy) = (
                        f64::from(h.x) + f64::from(w) / 2.0,
                        f64::from(h.y) + f64::from(hh) / 2.0,
                    );
                    let name = h.name.clone();
                    if let Some(anchor) = nearest_other(&self.heads, &name, cx, cy) {
                        let anchor = anchor.name.clone();
                        place(&mut self.heads, &name, &anchor, side);
                    }
                }
            }
            MonitorsInput::SetResolution(i) => {
                if let Some(h) = self.selected_head_mut()
                    && let Some(&(w, hh)) = h.resolutions().get(i as usize)
                {
                    if h.mode.map(|m| (m.width, m.height)) == Some((w, hh)) {
                        return;
                    }
                    h.set_resolution(w, hh);
                }
            }
            MonitorsInput::SetRate(i) => {
                if let Some(h) = self.selected_head_mut()
                    && let Some(m) = h.mode
                    && let Some(&r) = h.rates(m.width, m.height).get(i as usize)
                {
                    if r == m {
                        return;
                    }
                    h.mode = Some(r);
                }
                redraw = false;
            }
            MonitorsInput::SetTransform(i) => {
                if let Some(h) = self.selected_head_mut() {
                    if h.transform == i as usize {
                        return;
                    }
                    h.transform = (i as usize).min(7);
                }
            }
            MonitorsInput::SetScale(v) => {
                if let Some(h) = self.selected_head_mut() {
                    h.scale = (v * 100.0).round() / 100.0;
                }
                // The spin button keeps focus: only the map changes.
                rebuild_canvas(self, &sender);
                redraw = false;
            }
            MonitorsInput::Apply => {
                match validate(&self.heads) {
                    Some(e) => self.status = Some(Err(e)),
                    None => {
                        self.busy = true;
                        self.status = Some(Ok("Checking with the compositor…".into()));
                        spawn_apply(&sender, self.heads.clone());
                    }
                }
                redraw = false;
            }
            MonitorsInput::Keep => {
                let action = self.flow.keep();
                self.carry_out(action, &sender);
                redraw = false;
            }
            MonitorsInput::Revert => {
                let action = self.flow.revert();
                self.carry_out(action, &sender);
                redraw = false;
            }
            MonitorsInput::Tick(generation) => {
                if generation == self.apply_gen {
                    let action = self.flow.tick();
                    self.carry_out(action, &sender);
                }
                redraw = false;
            }
            MonitorsInput::Remove(i) => {
                if i < self.rules.len() {
                    self.rules.remove(i);
                    write_block("monitorrule", &self.rules);
                    rebuild_list(self, &sender);
                }
                redraw = false;
            }
        }
        if redraw {
            rebuild_canvas(self, &sender);
            rebuild_controls(self, &sender);
        }
    }

    fn update_cmd(
        &mut self,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            MonitorsCommandOutput::Loaded(Ok(heads)) => {
                self.busy = false;
                if self
                    .selected
                    .as_ref()
                    .is_none_or(|s| !heads.iter().any(|h| h.enabled && &h.name == s))
                {
                    self.selected = heads.iter().find(|h| h.enabled).map(|h| h.name.clone());
                }
                self.live = heads.clone();
                self.heads = heads;
                if matches!(self.status, Some(Err(_))) {
                    self.status = None;
                }
            }
            MonitorsCommandOutput::Loaded(Err(e)) => {
                self.busy = false;
                self.status = Some(Err(e));
            }
            MonitorsCommandOutput::Applied(Ok(())) => {
                self.busy = false;
                self.apply_gen += 1;
                self.flow.applied(self.live.clone(), self.heads.clone());
                self.live = self.heads.clone();
                self.status = None;
                spawn_countdown(&sender, self.apply_gen);
            }
            MonitorsCommandOutput::Applied(Err(e)) => {
                self.busy = false;
                self.status = Some(Err(e));
            }
            MonitorsCommandOutput::Reverted(result) => {
                self.status = Some(match result {
                    Ok(()) => Ok("Reverted to the previous display settings.".into()),
                    Err(e) => Err(format!("Revert failed: {e}")),
                });
                spawn_load(&sender);
            }
        }
        rebuild_canvas(self, &sender);
        rebuild_controls(self, &sender);
    }
}

impl MonitorsModel {
    /// Carry out what the confirm flow decided.
    fn carry_out(&mut self, action: ConfirmAction, sender: &ComponentSender<Self>) {
        match action {
            ConfirmAction::None => {}
            ConfirmAction::Persist(heads) => {
                self.rules = merge_rules(&read_block("monitorrule"), &heads);
                write_block("monitorrule", &self.rules);
                rebuild_list(self, sender);
                self.status = Some(Ok("Kept and saved as monitor rules.".into()));
            }
            ConfirmAction::Revert(heads) => {
                self.status = Some(Ok("Reverting…".into()));
                self.busy = true;
                spawn_revert(sender, heads);
            }
        }
    }
}
