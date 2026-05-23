//! Display → Layout sub-page — surfaces the `mlayout` CLI in the
//! Settings panel.
//!
//! `mlayout` (a separate binary in this workspace) maintains a
//! catalogue of named monitor arrangements as `layout_<slug>.conf`
//! files under `~/.config/margo/` and flips between them via a
//! `mlayout.conf` symlink that the user's `config.conf` is
//! expected to `source`. This page reads the catalogue with
//! `mlayout list --json`, shows one row per layout (active marked
//! with a primary accent), and lets the user activate / refresh /
//! seed presets via buttons that shell out to `mlayout`.
//!
//! We deliberately drive a child-process boundary instead of
//! depending on `mlayout` as a library — `mlayout` performs side
//! effects (wlr-randr calls, symlink rewrites, `mctl reload`)
//! that we'd rather not duplicate in mshell. The user can also
//! invoke the same commands from the terminal and the panel
//! stays in sync after a Refresh.

use relm4::gtk::prelude::{
    BoxExt, ButtonExt, DrawingAreaExtManual, EditableExt, EntryExt, OrientableExt, WidgetExt,
};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use serde_json::Value;
use std::process::Stdio;
use tracing::warn;

/// One row of `mlayout list --json` output.
#[derive(Debug, Clone)]
pub(crate) struct LayoutEntry {
    pub slug: String,
    pub name: String,
    pub shortcuts: Vec<String>,
    pub active: bool,
    pub outputs: Vec<OutputEntry>,
}

#[derive(Debug, Clone)]
pub(crate) struct OutputEntry {
    pub connector: String,
    pub label: Option<String>,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    /// `mlayout`'s per-output tint (hex `#rrggbb`), used to colour the
    /// rectangle in the mini-map so monitors are told apart at a glance.
    pub color: Option<String>,
}

#[derive(Debug, Default)]
pub(crate) struct LayoutSettingsModel {
    layouts: Vec<LayoutEntry>,
    active_slug: Option<String>,
    config_dir: String,
    /// Last error from a `mlayout` invocation. Shown inline; the
    /// next successful command clears it.
    last_error: Option<String>,
    new_slug_buf: String,
}

#[derive(Debug)]
pub(crate) enum LayoutSettingsInput {
    /// Re-run `mlayout list --json`.
    Refresh,
    /// `mlayout set <slug>` then refresh.
    Activate(String),
    /// `mlayout init` then refresh. Used when the catalogue is
    /// empty — captures the live monitor state as `layout_default.conf`.
    Init,
    /// `mlayout suggest --activate <first>` then refresh.
    /// Non-interactive: picks the first generated preset.
    Suggest,
    /// `mlayout new <slug>` then refresh.
    NewFromCurrent,
    NewSlugChanged(String),
    /// Result of an `mlayout` command — refresh the list and
    /// surface any error.
    CommandResult(Result<String, String>),
    /// Result of a fresh `mlayout list --json` parse.
    ListLoaded(LayoutCatalogue),
}

#[derive(Debug, Default)]
pub(crate) struct LayoutCatalogue {
    pub active: Option<String>,
    pub config_dir: String,
    pub layouts: Vec<LayoutEntry>,
}

#[derive(Debug)]
pub(crate) enum LayoutSettingsOutput {}

pub(crate) struct LayoutSettingsInit {}

#[derive(Debug)]
pub(crate) enum LayoutSettingsCommandOutput {}

#[relm4::component(pub)]
impl Component for LayoutSettingsModel {
    type CommandOutput = LayoutSettingsCommandOutput;
    type Input = LayoutSettingsInput;
    type Output = LayoutSettingsOutput;
    type Init = LayoutSettingsInit;

    view! {
        #[root]
        gtk::ScrolledWindow {
            set_vscrollbar_policy: gtk::PolicyType::Automatic,
            set_hscrollbar_policy: gtk::PolicyType::Never,
            set_propagate_natural_height: false,
            set_propagate_natural_width: false,
            set_hexpand: true,
            set_vexpand: true,

            gtk::Box {
                add_css_class: "settings-page",
                set_orientation: gtk::Orientation::Vertical,
                set_hexpand: true,
                set_spacing: 16,

                gtk::Box {
                    add_css_class: "settings-hero",
                    set_orientation: gtk::Orientation::Horizontal,
                    set_halign: gtk::Align::Start,
                    set_spacing: 16,
                    gtk::Image {
                        add_css_class: "settings-hero-icon",
                        set_icon_name: Some("preferences-desktop-display-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "Layout",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "Saved mlayout profiles for this hardware setup. Click a row to activate.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Monitor layouts",
                    set_halign: gtk::Align::Start,
                },

                gtk::Label {
                    add_css_class: "label-small",
                    set_label: "Named monitor arrangements live as `layout_<slug>.conf` files in margo's config directory; the active one is symlinked into `mlayout.conf`. Activate switches the symlink, re-applies geometry via wlr-randr, and pokes `mctl reload`.",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                },

                gtk::Label {
                    add_css_class: "label-small",
                    #[watch]
                    set_label: &format!("Config dir: {}", model.config_dir),
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                },

                gtk::Label {
                    add_css_class: "label-small",
                    #[watch]
                    set_visible: model.last_error.is_some(),
                    #[watch]
                    set_label: model.last_error.as_deref().unwrap_or(""),
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                },

                gtk::Separator {},

                gtk::Label {
                    add_css_class: "label-medium-bold",
                    set_label: "Available layouts",
                    set_halign: gtk::Align::Start,
                },

                gtk::Label {
                    add_css_class: "label-small",
                    #[watch]
                    set_visible: model.layouts.is_empty(),
                    set_label: "No layouts yet. Click `Init` to capture the current monitor configuration as `layout_default.conf`, or `Suggest` to generate the common arrangements for the detected outputs.",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                },

                #[name = "layouts_list"]
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    add_css_class: "layout-list",
                    set_spacing: 6,
                    #[watch]
                    set_visible: !model.layouts.is_empty(),
                },

                gtk::Separator {},

                gtk::Label {
                    add_css_class: "label-medium-bold",
                    set_label: "Actions",
                    set_halign: gtk::Align::Start,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 8,
                    set_halign: gtk::Align::Start,

                    gtk::Button {
                        set_label: "Refresh",
                        connect_clicked[sender] => move |_| {
                            sender.input(LayoutSettingsInput::Refresh);
                        },
                    },
                    gtk::Button {
                        set_label: "Init",
                        set_tooltip_text: Some(
                            "Capture the live monitor configuration as layout_default.conf."
                        ),
                        connect_clicked[sender] => move |_| {
                            sender.input(LayoutSettingsInput::Init);
                        },
                    },
                    gtk::Button {
                        set_label: "Suggest",
                        set_tooltip_text: Some(
                            "Generate preset layouts for the detected monitor arrangement and activate the first one."
                        ),
                        connect_clicked[sender] => move |_| {
                            sender.input(LayoutSettingsInput::Suggest);
                        },
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 8,
                    set_halign: gtk::Align::Start,

                    gtk::Label {
                        add_css_class: "label-medium",
                        set_label: "Capture current as:",
                        set_valign: gtk::Align::Center,
                    },
                    #[name = "new_slug_entry"]
                    gtk::Entry {
                        set_valign: gtk::Align::Center,
                        set_width_request: 180,
                        set_placeholder_text: Some("e.g. meeting"),
                        #[watch]
                        #[block_signal(new_slug_handler)]
                        set_text: &model.new_slug_buf,
                        connect_changed[sender] => move |e| {
                            sender.input(LayoutSettingsInput::NewSlugChanged(e.text().to_string()));
                        } @new_slug_handler,
                    },
                    gtk::Button {
                        set_label: "Capture",
                        #[watch]
                        set_sensitive: is_valid_slug(&model.new_slug_buf),
                        set_tooltip_text: Some(
                            "Write the current monitor configuration to layout_<slug>.conf."
                        ),
                        connect_clicked[sender] => move |_| {
                            sender.input(LayoutSettingsInput::NewFromCurrent);
                        },
                    },
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = LayoutSettingsModel::default();
        let widgets = view_output!();
        sender.input(LayoutSettingsInput::Refresh);
        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            LayoutSettingsInput::Refresh => {
                spawn_list(sender.clone());
            }
            LayoutSettingsInput::Activate(slug) => {
                spawn_cmd(
                    sender.clone(),
                    vec!["set".to_string(), slug],
                );
            }
            LayoutSettingsInput::Init => {
                spawn_cmd(
                    sender.clone(),
                    vec!["init".to_string(), "--yes".to_string()],
                );
            }
            LayoutSettingsInput::Suggest => {
                // Non-interactive: `--activate` requires a slug, so
                // we use a two-phase: first generate the catalogue
                // without picking (suggest reads stdin); to avoid
                // hanging on stdin we feed an empty line which the
                // CLI treats as "abort, nothing written" — that's
                // useless. Instead pass `--activate vertical-ext-top`
                // as the most common pick. If that slug doesn't
                // exist for the detected outputs, the CLI bails.
                // The user can also run `mlayout suggest`
                // interactively from a terminal — this button is
                // a convenience, not the only way in.
                spawn_cmd(
                    sender.clone(),
                    vec![
                        "suggest".to_string(),
                        "--yes".to_string(),
                        "--activate".to_string(),
                        "vertical-ext-top".to_string(),
                    ],
                );
            }
            LayoutSettingsInput::NewFromCurrent => {
                let slug = self.new_slug_buf.trim().to_string();
                if !is_valid_slug(&slug) {
                    return;
                }
                spawn_cmd(
                    sender.clone(),
                    vec![
                        "new".to_string(),
                        slug,
                        "--activate".to_string(),
                    ],
                );
                self.new_slug_buf.clear();
            }
            LayoutSettingsInput::NewSlugChanged(s) => {
                self.new_slug_buf = s;
            }
            LayoutSettingsInput::CommandResult(res) => {
                match res {
                    Ok(_) => self.last_error = None,
                    Err(e) => self.last_error = Some(e),
                }
                // Either way: re-fetch the list so the active
                // marker + any new files are reflected.
                spawn_list(sender.clone());
            }
            LayoutSettingsInput::ListLoaded(cat) => {
                self.active_slug = cat.active;
                self.config_dir = cat.config_dir;
                self.layouts = cat.layouts;
                rebuild_layout_rows(&widgets.layouts_list, &self.layouts, sender.clone());
            }
        }

        self.update_view(widgets, sender);
    }
}

fn rebuild_layout_rows(
    container: &gtk::Box,
    layouts: &[LayoutEntry],
    sender: ComponentSender<LayoutSettingsModel>,
) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
    for layout in layouts {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.add_css_class("layout-row");
        if layout.active {
            row.add_css_class("layout-row-active");
        }
        let active_marker = gtk::Label::new(Some(if layout.active { "●" } else { " " }));
        active_marker.add_css_class("label-medium-bold");
        active_marker.set_width_chars(2);
        row.append(&active_marker);

        // Scaled mini-map of the arrangement — see the monitor layout at
        // a glance instead of reading a list of rectangles.
        let map = gtk::DrawingArea::new();
        map.add_css_class("layout-map");
        map.set_size_request(208, 116);
        map.set_valign(gtk::Align::Center);
        let outs = layout.outputs.clone();
        map.set_draw_func(move |area, cr, w, h| {
            draw_layout_map(cr, w as f64, h as f64, &outs, area.color());
        });
        row.append(&map);

        let info_col = gtk::Box::new(gtk::Orientation::Vertical, 2);
        info_col.set_hexpand(true);
        let name = gtk::Label::new(Some(&layout.name));
        name.add_css_class("label-medium-bold");
        name.set_halign(gtk::Align::Start);
        info_col.append(&name);

        let mut subtitle = format!("slug `{}`", layout.slug);
        if !layout.shortcuts.is_empty() {
            subtitle.push_str(&format!(" — keys: {}", layout.shortcuts.join(", ")));
        }
        let sub = gtk::Label::new(Some(&subtitle));
        sub.add_css_class("label-small");
        sub.set_halign(gtk::Align::Start);
        info_col.append(&sub);

        if !layout.outputs.is_empty() {
            let n = layout.outputs.len();
            let summary = format!("{n} monitor{}", if n == 1 { "" } else { "s" });
            let out_lbl = gtk::Label::new(Some(&summary));
            out_lbl.add_css_class("label-small");
            out_lbl.set_halign(gtk::Align::Start);
            info_col.append(&out_lbl);
        }
        row.append(&info_col);

        let action = gtk::Button::with_label(if layout.active { "Active" } else { "Activate" });
        action.set_valign(gtk::Align::Center);
        action.set_sensitive(!layout.active);
        let slug = layout.slug.clone();
        let s = sender.clone();
        action.connect_clicked(move |_| {
            s.input(LayoutSettingsInput::Activate(slug.clone()));
        });
        row.append(&action);

        container.append(&row);
    }
}

fn spawn_list(sender: ComponentSender<LayoutSettingsModel>) {
    relm4::spawn(async move {
        let out = tokio::process::Command::new("mlayout")
            .args(["list", "--json"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await;
        match out {
            Ok(o) if o.status.success() => {
                let body = String::from_utf8_lossy(&o.stdout).into_owned();
                let cat = parse_catalogue(&body).unwrap_or_else(|e| {
                    warn!(error = %e, "mlayout list --json: parse failed");
                    LayoutCatalogue::default()
                });
                sender.input(LayoutSettingsInput::ListLoaded(cat));
            }
            Ok(o) => {
                let err = String::from_utf8_lossy(&o.stderr).trim().to_string();
                warn!(?o.status, %err, "mlayout list --json: non-zero exit");
                sender.input(LayoutSettingsInput::ListLoaded(LayoutCatalogue::default()));
            }
            Err(e) => {
                warn!(error = %e, "mlayout list --json: spawn failed");
                sender.input(LayoutSettingsInput::ListLoaded(LayoutCatalogue::default()));
            }
        }
    });
}

fn spawn_cmd(sender: ComponentSender<LayoutSettingsModel>, args: Vec<String>) {
    relm4::spawn(async move {
        let label = args.join(" ");
        let res = tokio::process::Command::new("mlayout")
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await;
        let outcome = match res {
            Ok(o) if o.status.success() => Ok(label),
            Ok(o) => {
                let stderr = String::from_utf8_lossy(&o.stderr).trim().to_string();
                if stderr.is_empty() {
                    Err(format!("mlayout {label}: exit {}", o.status))
                } else {
                    Err(format!("mlayout {label}: {stderr}"))
                }
            }
            Err(e) => Err(format!("mlayout {label}: spawn failed: {e}")),
        };
        sender.input(LayoutSettingsInput::CommandResult(outcome));
    });
}

fn parse_catalogue(body: &str) -> Result<LayoutCatalogue, String> {
    let v: Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    let active = v
        .get("active")
        .and_then(Value::as_str)
        .map(str::to_string);
    let config_dir = v
        .get("config_dir")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let layouts = v
        .get("layouts")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(parse_layout_entry)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Ok(LayoutCatalogue {
        active,
        config_dir,
        layouts,
    })
}

fn parse_layout_entry(v: &Value) -> Option<LayoutEntry> {
    let slug = v.get("slug")?.as_str()?.to_string();
    let name = v
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or(&slug)
        .to_string();
    let active = v
        .get("active")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let shortcuts = v
        .get("shortcuts")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|s| s.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let outputs = v
        .get("outputs")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(parse_output_entry)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Some(LayoutEntry {
        slug,
        name,
        shortcuts,
        active,
        outputs,
    })
}

fn parse_output_entry(v: &Value) -> Option<OutputEntry> {
    let connector = v.get("connector")?.as_str()?.to_string();
    let label = v
        .get("label")
        .and_then(Value::as_str)
        .map(str::to_string);
    let x = v.get("x").and_then(Value::as_i64).unwrap_or(0) as i32;
    let y = v.get("y").and_then(Value::as_i64).unwrap_or(0) as i32;
    let width = v.get("width").and_then(Value::as_i64).unwrap_or(0) as i32;
    let height = v.get("height").and_then(Value::as_i64).unwrap_or(0) as i32;
    let color = v
        .get("color")
        .and_then(Value::as_str)
        .map(str::to_string);
    Some(OutputEntry {
        connector,
        label,
        x,
        y,
        width,
        height,
        color,
    })
}

/// Draw a scaled top-down map of `outputs` into `cr` (size `area_w ×
/// area_h`). Rectangles keep the real aspect + relative positions; each
/// is tinted with its mlayout colour so monitors are distinguishable,
/// labelled with the connector + resolution in the widget's foreground.
fn draw_layout_map(
    cr: &gtk::cairo::Context,
    area_w: f64,
    area_h: f64,
    outputs: &[OutputEntry],
    fg: gtk::gdk::RGBA,
) {
    if outputs.is_empty() {
        return;
    }
    let min_x = outputs.iter().map(|o| o.x).min().unwrap_or(0) as f64;
    let min_y = outputs.iter().map(|o| o.y).min().unwrap_or(0) as f64;
    let max_x = outputs.iter().map(|o| o.x + o.width.max(1)).max().unwrap_or(1) as f64;
    let max_y = outputs.iter().map(|o| o.y + o.height.max(1)).max().unwrap_or(1) as f64;
    let span_w = (max_x - min_x).max(1.0);
    let span_h = (max_y - min_y).max(1.0);
    let pad = 6.0;
    let scale = ((area_w - pad * 2.0) / span_w).min((area_h - pad * 2.0) / span_h);
    let off_x = (area_w - span_w * scale) / 2.0;
    let off_y = (area_h - span_h * scale) / 2.0;

    for (i, o) in outputs.iter().enumerate() {
        let rx = off_x + (o.x as f64 - min_x) * scale;
        let ry = off_y + (o.y as f64 - min_y) * scale;
        let rw = (o.width.max(1) as f64) * scale;
        let rh = (o.height.max(1) as f64) * scale;
        let (r, g, b) = parse_hex_rgb(o.color.as_deref()).unwrap_or_else(|| default_swatch(i));

        rounded_rect(cr, rx + 1.0, ry + 1.0, (rw - 2.0).max(1.0), (rh - 2.0).max(1.0), 3.0);
        cr.set_source_rgba(r, g, b, 0.28);
        let _ = cr.fill_preserve();
        cr.set_source_rgba(r, g, b, 0.95);
        cr.set_line_width(1.5);
        let _ = cr.stroke();

        cr.set_source_rgba(
            fg.red() as f64,
            fg.green() as f64,
            fg.blue() as f64,
            fg.alpha() as f64,
        );
        if rh > 22.0 {
            let name = o.label.clone().unwrap_or_else(|| o.connector.clone());
            cr.select_font_face(
                "sans-serif",
                gtk::cairo::FontSlant::Normal,
                gtk::cairo::FontWeight::Bold,
            );
            cr.set_font_size(10.0);
            if let Ok(ext) = cr.text_extents(&name)
                && ext.width() < rw - 4.0
            {
                cr.move_to(rx + (rw - ext.width()) / 2.0, ry + rh / 2.0 - 1.0);
                let _ = cr.show_text(&name);
            }
            let res = format!("{}×{}", o.width, o.height);
            cr.select_font_face(
                "sans-serif",
                gtk::cairo::FontSlant::Normal,
                gtk::cairo::FontWeight::Normal,
            );
            cr.set_font_size(8.5);
            if let Ok(ext) = cr.text_extents(&res)
                && ext.width() < rw - 4.0
            {
                cr.move_to(rx + (rw - ext.width()) / 2.0, ry + rh / 2.0 + 10.0);
                let _ = cr.show_text(&res);
            }
        }
    }
}

fn rounded_rect(cr: &gtk::cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    let pi = std::f64::consts::PI;
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -pi / 2.0, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, pi / 2.0);
    cr.arc(x + r, y + h - r, r, pi / 2.0, pi);
    cr.arc(x + r, y + r, r, pi, 1.5 * pi);
    cr.close_path();
}

fn parse_hex_rgb(s: Option<&str>) -> Option<(f64, f64, f64)> {
    let s = s?.trim().trim_start_matches('#');
    if s.len() < 6 {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some((r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0))
}

/// Distinct fallback swatches when an output carries no mlayout colour.
fn default_swatch(i: usize) -> (f64, f64, f64) {
    const SWATCHES: [(f64, f64, f64); 6] = [
        (0.45, 0.62, 0.95),
        (0.52, 0.80, 0.55),
        (0.93, 0.69, 0.36),
        (0.86, 0.51, 0.78),
        (0.40, 0.78, 0.82),
        (0.90, 0.55, 0.50),
    ];
    SWATCHES[i % SWATCHES.len()]
}

/// Slug validation mirrors what `mlayout new` accepts: ascii
/// alphanumerics, `-`, `_`. Empty is rejected.
fn is_valid_slug(s: &str) -> bool {
    let s = s.trim();
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}
