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
    BoxExt, ButtonExt, DrawingAreaExtManual, EditableExt, EntryExt, FixedExt, GestureDragExt,
    OrientableExt, WidgetExt,
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

/// One draggable monitor in the arrange editor. Coordinates are the
/// real compositor pixel space (what wlr-randr `--pos` expects); the
/// canvas scales them down for display.
#[derive(Debug, Clone)]
pub(crate) struct EditorOut {
    pub connector: String,
    pub label: String,
    pub color: Option<String>,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
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
    /// Live monitors for the drag-to-arrange editor (from
    /// `mlayout outputs`). Mutated as the user drags tiles.
    editor: Vec<EditorOut>,
    /// True while the dragged arrangement differs from what's applied,
    /// so the Apply button can light up / dim.
    editor_dirty: bool,
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
    /// Live outputs parsed from `mlayout outputs` — seeds the editor.
    OutputsLoaded(Vec<EditorOut>),
    /// A tile was dropped at real coords `(x, y)` — snap + re-layout.
    OutputMoved(String, i32, i32),
    /// Apply the dragged arrangement to the live session via wlr-randr.
    ApplyArrangement,
    /// Re-read live outputs, discarding un-applied drags.
    ResetArrangement,
    /// Apply a semantic quick-profile (Internal/External/Extend/Mirror)
    /// live via `mctl dispatch enable_output/disable_output/mirror_output`.
    ApplyProfile(DisplayProfile),
    /// Flip one output on/off via `mctl dispatch toggle_output <connector>`.
    ToggleOutput(String),
}

/// A semantic quick display profile — the four universal arrangements the
/// niri/Win+P-style switcher offers, applied on top of the live outputs
/// (laptop panel detected by an eDP/LVDS connector).
#[derive(Debug, Clone, Copy)]
pub(crate) enum DisplayProfile {
    /// Laptop panel only — every external output off.
    InternalOnly,
    /// External monitors only — the laptop panel off.
    ExternalOnly,
    /// Every output on, side by side.
    Extend,
    /// Every output on + margo's native mirror showing the laptop panel on
    /// the first external monitor (`mirror_output`).
    Mirror,
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

                // ── Quick profiles + per-output power ──────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Quick profiles",
                    set_halign: gtk::Align::Start,
                },
                gtk::Label {
                    add_css_class: "label-small",
                    set_label: "One-click display arrangements, applied live (mctl). The laptop panel is detected by its eDP/LVDS connector; Mirror shows it on an external monitor, scaled to fit. Use Extend to bring every screen back.",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                },
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 8,
                    set_homogeneous: true,
                    gtk::Button {
                        add_css_class: "ok-button-surface",
                        set_label: "Internal only",
                        connect_clicked[sender] => move |_| sender.input(LayoutSettingsInput::ApplyProfile(DisplayProfile::InternalOnly)),
                    },
                    gtk::Button {
                        add_css_class: "ok-button-surface",
                        set_label: "External only",
                        connect_clicked[sender] => move |_| sender.input(LayoutSettingsInput::ApplyProfile(DisplayProfile::ExternalOnly)),
                    },
                    gtk::Button {
                        add_css_class: "ok-button-surface",
                        set_label: "Extend",
                        connect_clicked[sender] => move |_| sender.input(LayoutSettingsInput::ApplyProfile(DisplayProfile::Extend)),
                    },
                    gtk::Button {
                        add_css_class: "ok-button-surface",
                        set_label: "Mirror",
                        set_tooltip_text: Some("Show the laptop screen on an external monitor, scaled to fit"),
                        connect_clicked[sender] => move |_| sender.input(LayoutSettingsInput::ApplyProfile(DisplayProfile::Mirror)),
                    },
                },
                gtk::Label {
                    add_css_class: "label-medium-bold",
                    set_label: "Outputs",
                    set_halign: gtk::Align::Start,
                    set_margin_top: 4,
                },
                #[name = "power_box"]
                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,
                },

                // ── Drag-to-arrange editor ─────────────────────
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Arrange monitors",
                    set_halign: gtk::Align::Start,
                },
                gtk::Label {
                    add_css_class: "label-small",
                    set_label: "Drag a screen to reposition it — edges snap to neighbours. Apply sets the live arrangement (via wlr-randr); save it as a named layout below to keep it.",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                },

                #[name = "editor_canvas"]
                gtk::Fixed {
                    add_css_class: "layout-editor",
                    set_size_request: (470, 250),
                    set_halign: gtk::Align::Center,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 8,
                    set_halign: gtk::Align::Center,

                    gtk::Button {
                        set_css_classes: &["ok-button-primary"],
                        set_label: "Apply arrangement",
                        #[watch]
                        set_sensitive: model.editor_dirty,
                        set_tooltip_text: Some("Set the live monitor positions to match the editor."),
                        connect_clicked[sender] => move |_| {
                            sender.input(LayoutSettingsInput::ApplyArrangement);
                        },
                    },
                    gtk::Button {
                        add_css_class: "ok-button-surface",
                        set_label: "Reset",
                        set_tooltip_text: Some("Discard un-applied drags and re-read the live arrangement."),
                        connect_clicked[sender] => move |_| {
                            sender.input(LayoutSettingsInput::ResetArrangement);
                        },
                    },
                },

                gtk::Separator {},

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
                        add_css_class: "ok-button-surface",
                        set_label: "Refresh",
                        connect_clicked[sender] => move |_| {
                            sender.input(LayoutSettingsInput::Refresh);
                        },
                    },
                    gtk::Button {
                        add_css_class: "ok-button-surface",
                        set_label: "Init",
                        set_tooltip_text: Some(
                            "Capture the live monitor configuration as layout_default.conf."
                        ),
                        connect_clicked[sender] => move |_| {
                            sender.input(LayoutSettingsInput::Init);
                        },
                    },
                    gtk::Button {
                        add_css_class: "ok-button-surface",
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
                        add_css_class: "ok-button-surface",
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
        spawn_outputs(sender.clone());
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
                spawn_cmd(sender.clone(), vec!["set".to_string(), slug]);
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
                    vec!["new".to_string(), slug, "--activate".to_string()],
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
                // Re-fetch both the catalogue (active marker + new files)
                // and the live outputs (so the editor reflects reality).
                spawn_list(sender.clone());
                spawn_outputs(sender.clone());
            }
            LayoutSettingsInput::ListLoaded(cat) => {
                self.active_slug = cat.active;
                self.config_dir = cat.config_dir;
                self.layouts = cat.layouts;
                rebuild_layout_rows(&widgets.layouts_list, &self.layouts, sender.clone());
            }
            LayoutSettingsInput::OutputsLoaded(outs) => {
                self.editor = outs;
                self.editor_dirty = false;
                rebuild_editor_canvas(&widgets.editor_canvas, &self.editor, sender.clone());
                rebuild_power(&widgets.power_box, &self.editor, sender.clone());
            }
            LayoutSettingsInput::OutputMoved(connector, x, y) => {
                if let Some(o) = self.editor.iter_mut().find(|o| o.connector == connector) {
                    o.x = x;
                    o.y = y;
                }
                snap_arrangement(&mut self.editor, &connector);
                self.editor_dirty = true;
                rebuild_editor_canvas(&widgets.editor_canvas, &self.editor, sender.clone());
            }
            LayoutSettingsInput::ApplyArrangement => {
                spawn_apply_geometry(self.editor.clone(), sender.clone());
            }
            LayoutSettingsInput::ResetArrangement => {
                spawn_outputs(sender.clone());
            }
            LayoutSettingsInput::ApplyProfile(profile) => {
                spawn_profile(sender.clone(), profile, self.editor.clone());
            }
            LayoutSettingsInput::ToggleOutput(connector) => {
                run_dispatches(
                    sender.clone(),
                    vec![vec!["toggle_output".to_string(), connector.clone()]],
                    format!("toggle {connector}"),
                );
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
        action.add_css_class("ok-button-surface");
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

/// Read the live monitor configuration via `mlayout outputs` and seed
/// the drag editor.
fn spawn_outputs(sender: ComponentSender<LayoutSettingsModel>) {
    relm4::spawn(async move {
        let out = tokio::process::Command::new("mlayout")
            .arg("outputs")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await;
        let outs = match out {
            Ok(o) if o.status.success() => {
                parse_outputs(&String::from_utf8_lossy(&o.stdout)).unwrap_or_default()
            }
            Ok(o) => {
                warn!(status = ?o.status, "mlayout outputs: non-zero exit");
                Vec::new()
            }
            Err(e) => {
                warn!(error = %e, "mlayout outputs: spawn failed");
                Vec::new()
            }
        };
        sender.input(LayoutSettingsInput::OutputsLoaded(outs));
    });
}

/// The laptop panel is the eDP / LVDS connector.
fn is_internal_connector(connector: &str) -> bool {
    let l = connector.to_ascii_lowercase();
    l.starts_with("edp") || l.starts_with("lvds")
}

/// Run a sequence of `dispatch <action> <args…>` requests on margo's IPC
/// socket. A `CommandResult` re-reads the catalogue + live outputs
/// afterwards.
fn run_dispatches(
    sender: ComponentSender<LayoutSettingsModel>,
    dispatches: Vec<Vec<String>>,
    label: String,
) {
    relm4::spawn(async move {
        let mut err: Option<String> = None;
        for d in &dispatches {
            let Some((action, args)) = d.split_first() else {
                continue;
            };
            if let Err(e) =
                mshell_margo_client::ctl::dispatch_async(action.clone(), args.to_vec()).await
            {
                err = Some(format!("dispatch {}: {e}", d.join(" ")));
                break;
            }
        }
        sender.input(LayoutSettingsInput::CommandResult(match err {
            Some(e) => Err(e),
            None => Ok(label),
        }));
    });
}

/// Build the `mctl dispatch` plan for a semantic profile from the live
/// outputs and run it. Laptop panel detected by an eDP/LVDS connector.
fn spawn_profile(
    sender: ComponentSender<LayoutSettingsModel>,
    profile: DisplayProfile,
    outputs: Vec<EditorOut>,
) {
    match profile_plan(profile, &outputs) {
        Ok((plan, label)) => run_dispatches(sender, plan, label.to_string()),
        Err(e) => sender.input(LayoutSettingsInput::CommandResult(Err(e.to_string()))),
    }
}

/// A dispatch plan: `mctl dispatch` argument lists, in order.
type DispatchPlan = Vec<Vec<String>>;

/// The `mctl dispatch` plan for a display profile over the live outputs,
/// and the label reported when it ran. Every plan first ends a running
/// mirror (its target is out of the layout until then). `Err` (the message
/// to show) when the profile can't apply to these outputs — in particular,
/// never a plan that would leave every output switched off.
fn profile_plan(
    profile: DisplayProfile,
    outputs: &[EditorOut],
) -> Result<(DispatchPlan, &'static str), &'static str> {
    if outputs.is_empty() {
        return Err("No outputs detected");
    }
    let internal: Vec<String> = outputs
        .iter()
        .filter(|o| is_internal_connector(&o.connector))
        .map(|o| o.connector.clone())
        .collect();
    let external: Vec<String> = outputs
        .iter()
        .filter(|o| !is_internal_connector(&o.connector))
        .map(|o| o.connector.clone())
        .collect();

    let enable = |c: &String| vec!["enable_output".to_string(), c.clone()];
    let disable = |c: &String| vec!["disable_output".to_string(), c.clone()];
    let stop = || std::iter::once(vec!["stop_mirror".to_string()]);

    Ok(match profile {
        DisplayProfile::InternalOnly if internal.is_empty() => {
            return Err("No laptop screen to keep on");
        }
        DisplayProfile::ExternalOnly if external.is_empty() => {
            return Err("No external monitor to switch to");
        }
        DisplayProfile::InternalOnly => (
            stop()
                .chain(internal.iter().map(&enable))
                .chain(external.iter().map(&disable))
                .collect(),
            "profile: internal only",
        ),
        DisplayProfile::ExternalOnly => (
            stop()
                .chain(external.iter().map(&enable))
                .chain(internal.iter().map(&disable))
                .collect(),
            "profile: external only",
        ),
        DisplayProfile::Extend => (
            stop()
                .chain(outputs.iter().map(|o| enable(&o.connector)))
                .collect(),
            "profile: extend",
        ),
        DisplayProfile::Mirror => {
            let (Some(ext), Some(int)) = (external.first(), internal.first()) else {
                return Err("Mirror needs both a laptop screen and an external monitor");
            };
            (
                stop()
                    .chain(outputs.iter().map(|o| enable(&o.connector)))
                    .chain(std::iter::once(vec![
                        "mirror_output".to_string(),
                        int.clone(),
                        ext.clone(),
                    ]))
                    .collect(),
                "profile: mirror",
            )
        }
    })
}

/// Rebuild the per-output power list: one row per live output with a
/// toggle that flips it on/off (`mctl dispatch toggle_output <connector>`).
fn rebuild_power(
    container: &gtk::Box,
    outputs: &[EditorOut],
    sender: ComponentSender<LayoutSettingsModel>,
) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
    if outputs.is_empty() {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.set_margin_top(8);
        row.set_margin_bottom(8);
        row.set_margin_start(8);
        let lbl = gtk::Label::builder()
            .label("No live outputs detected.")
            .css_classes(["label-small"])
            .halign(gtk::Align::Start)
            .build();
        row.append(&lbl);
        container.append(&row);
        return;
    }
    for o in outputs {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.set_margin_top(6);
        row.set_margin_bottom(6);
        row.set_margin_start(8);
        row.set_margin_end(8);

        let title = if is_internal_connector(&o.connector) {
            format!("{} · Laptop screen", o.connector)
        } else if o.label.is_empty() {
            o.connector.clone()
        } else {
            format!("{} · {}", o.connector, o.label)
        };
        let name = gtk::Label::builder()
            .label(title)
            .css_classes(["label-medium"])
            .halign(gtk::Align::Start)
            .hexpand(true)
            .xalign(0.0)
            .build();
        row.append(&name);

        let btn = gtk::Button::builder()
            .label("Toggle")
            .css_classes(["ok-button-surface"])
            .valign(gtk::Align::Center)
            .tooltip_text("Turn this output off / on (use Extend to bring all back)")
            .build();
        let s = sender.clone();
        let conn = o.connector.clone();
        btn.connect_clicked(move |_| s.input(LayoutSettingsInput::ToggleOutput(conn.clone())));
        row.append(&btn);

        container.append(&row);
    }
}

fn parse_outputs(body: &str) -> Option<Vec<EditorOut>> {
    let v: Value = serde_json::from_str(body).ok()?;
    let arr = v.get("outputs")?.as_array()?;
    Some(
        arr.iter()
            .filter_map(|o| {
                Some(EditorOut {
                    connector: o.get("connector")?.as_str()?.to_string(),
                    label: o
                        .get("label")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    color: o.get("color").and_then(Value::as_str).map(str::to_string),
                    x: o.get("x").and_then(Value::as_i64).unwrap_or(0) as i32,
                    y: o.get("y").and_then(Value::as_i64).unwrap_or(0) as i32,
                    width: o.get("width").and_then(Value::as_i64).unwrap_or(0) as i32,
                    height: o.get("height").and_then(Value::as_i64).unwrap_or(0) as i32,
                })
            })
            .collect(),
    )
}

/// Snap the just-moved output's edges to its neighbours so monitors butt
/// up cleanly instead of leaving gaps/overlaps. Considers, for each other
/// output, left/right alignment, top/bottom alignment, and side-by-side
/// adjacency; the nearest candidate within `SNAP` (real px) wins per axis.
fn snap_arrangement(outs: &mut [EditorOut], moved: &str) {
    const SNAP: i32 = 150;
    let Some(mi) = outs.iter().position(|o| o.connector == moved) else {
        return;
    };
    let m = outs[mi].clone();
    let mut best_x = m.x;
    let mut best_dx = SNAP + 1;
    let mut best_y = m.y;
    let mut best_dy = SNAP + 1;
    for (i, o) in outs.iter().enumerate() {
        if i == mi {
            continue;
        }
        // X: align-left, align-right-edge, place-right-of, place-left-of.
        for cand in [o.x, o.x + o.width - m.width, o.x + o.width, o.x - m.width] {
            let d = (m.x - cand).abs();
            if d < best_dx {
                best_dx = d;
                best_x = cand;
            }
        }
        for cand in [
            o.y,
            o.y + o.height - m.height,
            o.y + o.height,
            o.y - m.height,
        ] {
            let d = (m.y - cand).abs();
            if d < best_dy {
                best_dy = d;
                best_y = cand;
            }
        }
    }
    if best_dx <= SNAP {
        outs[mi].x = best_x;
    }
    if best_dy <= SNAP {
        outs[mi].y = best_y;
    }
}

/// Apply the editor arrangement to the live session via `wlr-randr`
/// (positions only). Normalises so the top-left output sits at (0,0)
/// before applying — margo expects a non-negative origin.
fn spawn_apply_geometry(mut outs: Vec<EditorOut>, sender: ComponentSender<LayoutSettingsModel>) {
    if outs.is_empty() {
        return;
    }
    normalise_origin(&mut outs);
    relm4::spawn(async move {
        let mut cmd = tokio::process::Command::new("wlr-randr");
        for o in &outs {
            cmd.arg("--output")
                .arg(&o.connector)
                .arg("--pos")
                .arg(format!("{},{}", o.x, o.y));
        }
        let res = cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await;
        let outcome = match res {
            Ok(o) if o.status.success() => Ok("apply arrangement".to_string()),
            Ok(o) => {
                let err = String::from_utf8_lossy(&o.stderr).trim().to_string();
                Err(if err.is_empty() {
                    format!("wlr-randr: exit {}", o.status)
                } else {
                    format!("wlr-randr: {err}")
                })
            }
            Err(e) => Err(format!("wlr-randr spawn failed: {e}")),
        };
        sender.input(LayoutSettingsInput::CommandResult(outcome));
    });
}

/// Shift the arrangement so its top-left output sits at (0,0).
fn normalise_origin(outs: &mut [EditorOut]) {
    let min_x = outs.iter().map(|o| o.x).min().unwrap_or(0);
    let min_y = outs.iter().map(|o| o.y).min().unwrap_or(0);
    for o in outs {
        o.x -= min_x;
        o.y -= min_y;
    }
}

/// (Re)build the draggable monitor tiles in the editor `Fixed`, scaled to
/// fit the canvas. Each tile carries a `GestureDrag` that moves it live
/// and, on release, reports the new real coordinates for snapping.
fn rebuild_editor_canvas(
    fixed: &gtk::Fixed,
    outs: &[EditorOut],
    sender: ComponentSender<LayoutSettingsModel>,
) {
    while let Some(child) = fixed.first_child() {
        fixed.remove(&child);
    }
    if outs.is_empty() {
        return;
    }
    let (cw, ch, pad) = (470.0_f64, 250.0_f64, 12.0_f64);
    let min_x = outs.iter().map(|o| o.x).min().unwrap_or(0) as f64;
    let min_y = outs.iter().map(|o| o.y).min().unwrap_or(0) as f64;
    let max_x = outs.iter().map(|o| o.x + o.width.max(1)).max().unwrap_or(1) as f64;
    let max_y = outs
        .iter()
        .map(|o| o.y + o.height.max(1))
        .max()
        .unwrap_or(1) as f64;
    let span_w = (max_x - min_x).max(1.0);
    let span_h = (max_y - min_y).max(1.0);
    let s = ((cw - 2.0 * pad) / span_w).min((ch - 2.0 * pad) / span_h);
    let off_x = (cw - span_w * s) / 2.0;
    let off_y = (ch - span_h * s) / 2.0;

    for o in outs {
        let cx = off_x + (o.x as f64 - min_x) * s;
        let cy = off_y + (o.y as f64 - min_y) * s;
        let tw = (o.width as f64 * s).max(28.0);
        let th = (o.height as f64 * s).max(20.0);
        let tile = build_tile(o, tw, th);
        fixed.put(&tile, cx, cy);

        let drag = gtk::GestureDrag::new();
        let f_up = fixed.clone();
        let t_up = tile.clone();
        drag.connect_drag_update(move |_, ox, oy| {
            f_up.move_(&t_up, cx + ox, cy + oy);
        });
        let conn = o.connector.clone();
        let s_end = sender.clone();
        drag.connect_drag_end(move |_, ox, oy| {
            let rx = (min_x + (cx + ox - off_x) / s).round() as i32;
            let ry = (min_y + (cy + oy - off_y) / s).round() as i32;
            s_end.input(LayoutSettingsInput::OutputMoved(conn.clone(), rx, ry));
        });
        tile.add_controller(drag);
    }
}

fn build_tile(o: &EditorOut, tw: f64, th: f64) -> gtk::Box {
    let tile = gtk::Box::new(gtk::Orientation::Vertical, 0);
    tile.add_css_class("layout-tile");
    tile.set_size_request(tw as i32, th as i32);
    tile.set_cursor_from_name(Some("grab"));

    let dot = dot_markup(o.color.as_deref());
    let name = if o.label.is_empty() {
        o.connector.clone()
    } else {
        o.label.clone()
    };
    let label = gtk::Label::new(None);
    label.set_use_markup(true);
    label.set_justify(gtk::Justification::Center);
    label.set_halign(gtk::Align::Center);
    label.set_valign(gtk::Align::Center);
    label.set_hexpand(true);
    label.set_vexpand(true);
    label.set_markup(&format!(
        "{} {}\n<span size=\"x-small\">{}×{}</span>",
        dot,
        gtk::glib::markup_escape_text(&name),
        o.width,
        o.height,
    ));
    tile.append(&label);
    tile
}

/// The tile's output-colour dot. An output without (or with an unreadable)
/// mlayout colour gets a plain dot in the label's own theme colour rather
/// than a hardcoded grey; a readable one is re-emitted as `#rrggbb`, so the
/// raw config string never lands inside the markup attribute.
fn dot_markup(color: Option<&str>) -> String {
    match color.and_then(mshell_common::color::parse_hex_bytes) {
        Some((r, g, b, _)) => format!("<span foreground=\"#{r:02x}{g:02x}{b:02x}\">●</span>"),
        None => "●".to_string(),
    }
}

fn parse_catalogue(body: &str) -> Result<LayoutCatalogue, String> {
    let v: Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    let active = v.get("active").and_then(Value::as_str).map(str::to_string);
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
    let active = v.get("active").and_then(Value::as_bool).unwrap_or(false);
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
    let label = v.get("label").and_then(Value::as_str).map(str::to_string);
    let x = v.get("x").and_then(Value::as_i64).unwrap_or(0) as i32;
    let y = v.get("y").and_then(Value::as_i64).unwrap_or(0) as i32;
    let width = v.get("width").and_then(Value::as_i64).unwrap_or(0) as i32;
    let height = v.get("height").and_then(Value::as_i64).unwrap_or(0) as i32;
    let color = v.get("color").and_then(Value::as_str).map(str::to_string);
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
    let max_x = outputs
        .iter()
        .map(|o| o.x + o.width.max(1))
        .max()
        .unwrap_or(1) as f64;
    let max_y = outputs
        .iter()
        .map(|o| o.y + o.height.max(1))
        .max()
        .unwrap_or(1) as f64;
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

        rounded_rect(
            cr,
            rx + 1.0,
            ry + 1.0,
            (rw - 2.0).max(1.0),
            (rh - 2.0).max(1.0),
            3.0,
        );
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

/// An output's mlayout colour as RGB (alpha ignored — the preview paints
/// its own translucency).
fn parse_hex_rgb(s: Option<&str>) -> Option<(f64, f64, f64)> {
    let (r, g, b, _) = mshell_common::color::parse_hex_rgba(s?)?;
    Some((r, g, b))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dot_markup_uses_theme_colour_without_a_valid_output_colour() {
        assert_eq!(dot_markup(None), "●");
        assert_eq!(dot_markup(Some("\"><b>x")), "●");
        assert_eq!(
            dot_markup(Some("0x3366ccff")),
            "<span foreground=\"#3366cc\">●</span>"
        );
    }

    #[test]
    fn output_colour_parses_to_rgb() {
        assert_eq!(parse_hex_rgb(Some("#ff0000")), Some((1.0, 0.0, 0.0)));
        // The old byte-slicing parser panicked here (non-char-boundary slice).
        assert_eq!(parse_hex_rgb(Some("1ä23456")), None);
        assert_eq!(parse_hex_rgb(None), None);
    }

    fn out(connector: &str, x: i32, y: i32, width: i32, height: i32) -> EditorOut {
        EditorOut {
            connector: connector.to_string(),
            label: String::new(),
            color: None,
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn catalogue_parses_mlayout_list_json() {
        let body = r##"{
            "active": "desk",
            "config_dir": "/home/u/.config/margo",
            "layouts": [
                {"slug": "desk", "name": "Desk", "active": true, "shortcuts": ["super+1", 7],
                 "outputs": [
                    {"connector": "eDP-1", "label": "Laptop", "x": 0, "y": 1080,
                     "width": 1920, "height": 1200, "color": "#ff0000"},
                    {"connector": "DP-3", "x": 0, "y": 0, "width": 2560, "height": 1440},
                    {"label": "no connector"}
                 ]},
                {"slug": "solo"},
                {"name": "slugless"},
                "garbage"
            ]
        }"##;
        let cat = parse_catalogue(body).unwrap();
        assert_eq!(cat.active.as_deref(), Some("desk"));
        assert_eq!(cat.config_dir, "/home/u/.config/margo");
        assert_eq!(cat.layouts.len(), 2, "{:#?}", cat.layouts);

        let desk = &cat.layouts[0];
        assert_eq!(
            (desk.slug.as_str(), desk.name.as_str(), desk.active),
            ("desk", "Desk", true)
        );
        // Non-string shortcuts are skipped, not fatal.
        assert_eq!(desk.shortcuts, ["super+1"]);
        assert_eq!(desk.outputs.len(), 2);
        let edp = &desk.outputs[0];
        assert_eq!(
            (
                edp.connector.as_str(),
                edp.label.as_deref(),
                edp.x,
                edp.y,
                edp.width,
                edp.height
            ),
            ("eDP-1", Some("Laptop"), 0, 1080, 1920, 1200)
        );
        assert_eq!(edp.color.as_deref(), Some("#ff0000"));
        assert_eq!(desk.outputs[1].label, None);

        // Missing optional fields default; the name falls back to the slug.
        let solo = &cat.layouts[1];
        assert_eq!((solo.name.as_str(), solo.active), ("solo", false));
        assert!(solo.shortcuts.is_empty() && solo.outputs.is_empty());
    }

    #[test]
    fn catalogue_tolerates_missing_sections_and_rejects_bad_json() {
        let cat = parse_catalogue("{}").unwrap();
        assert!(cat.active.is_none() && cat.config_dir.is_empty() && cat.layouts.is_empty());
        let cat = parse_catalogue(r#"{"active": null, "layouts": {}}"#).unwrap();
        assert!(cat.active.is_none() && cat.layouts.is_empty());
        assert!(parse_catalogue("").is_err());
        assert!(parse_catalogue("mlayout: command not found").is_err());
    }

    #[test]
    fn outputs_parse_and_default_missing_geometry() {
        let body = r##"{"outputs": [
            {"connector": "eDP-1", "label": "Laptop", "x": -1920, "y": 0, "width": 1920, "height": 1080},
            {"connector": "HDMI-A-1", "x": 1.5, "color": "#00ff00"},
            {"label": "nameless"}
        ]}"##;
        let outs = parse_outputs(body).unwrap();
        assert_eq!(outs.len(), 2);
        assert_eq!(
            (outs[0].x, outs[0].width, outs[0].label.as_str()),
            (-1920, 1920, "Laptop")
        );
        // Non-integer / missing numbers read as 0; a missing label as "".
        assert_eq!((outs[1].x, outs[1].y, outs[1].width), (0, 0, 0));
        assert_eq!(outs[1].label, "");
        assert_eq!(outs[1].color.as_deref(), Some("#00ff00"));

        assert!(parse_outputs(r#"{"outputs": []}"#).unwrap().is_empty());
        assert!(parse_outputs("{}").is_none());
        assert!(parse_outputs(r#"{"outputs": 3}"#).is_none());
        assert!(parse_outputs("not json").is_none());
    }

    #[test]
    fn internal_connectors_are_edp_and_lvds() {
        for c in ["eDP-1", "edp-2", "LVDS-1"] {
            assert!(is_internal_connector(c), "{c}");
        }
        for c in ["DP-1", "HDMI-A-1", "DVI-D-1", "Virtual-1", ""] {
            assert!(!is_internal_connector(c), "{c}");
        }
    }

    #[test]
    fn slugs_follow_mlayout_rules() {
        for ok in ["desk", "work-2", "home_office", "  padded  "] {
            assert!(is_valid_slug(ok), "{ok:?}");
        }
        for bad in [
            "",
            "   ",
            "two words",
            "../etc",
            "slash/y",
            "ümlaut",
            "dot.conf",
        ] {
            assert!(!is_valid_slug(bad), "{bad:?}");
        }
    }

    #[test]
    fn snapping_butts_a_dragged_monitor_against_its_neighbour() {
        let mut outs = vec![
            out("eDP-1", 0, 0, 1920, 1080),
            out("DP-1", 1990, 60, 2560, 1440),
        ];
        snap_arrangement(&mut outs, "DP-1");
        // 70 px gap closes to adjacency; 60 px offset snaps to top-aligned.
        assert_eq!((outs[1].x, outs[1].y), (1920, 0));
        // Other outputs never move.
        assert_eq!((outs[0].x, outs[0].y), (0, 0));
    }

    #[test]
    fn snapping_leaves_far_moves_and_unknown_outputs_alone() {
        let mut outs = vec![
            out("eDP-1", 0, 0, 1920, 1080),
            out("DP-1", 2500, 600, 2560, 1440),
        ];
        snap_arrangement(&mut outs, "DP-1");
        assert_eq!((outs[1].x, outs[1].y), (2500, 600));
        snap_arrangement(&mut outs, "HDMI-A-9");
        assert_eq!((outs[1].x, outs[1].y), (2500, 600));
        let mut single = vec![out("eDP-1", 37, 12, 1920, 1080)];
        snap_arrangement(&mut single, "eDP-1");
        assert_eq!((single[0].x, single[0].y), (37, 12));
    }

    #[test]
    fn origin_is_normalised_to_the_top_left_output() {
        let mut outs = vec![
            out("eDP-1", -1920, 300, 1920, 1080),
            out("DP-1", 0, -200, 2560, 1440),
        ];
        normalise_origin(&mut outs);
        assert_eq!((outs[0].x, outs[0].y), (0, 500));
        assert_eq!((outs[1].x, outs[1].y), (1920, 0));
        normalise_origin(&mut []);
    }

    fn cmds(plan: &DispatchPlan) -> Vec<String> {
        plan.iter().map(|c| c.join(" ")).collect()
    }

    #[test]
    fn profiles_enable_before_disabling() {
        let outs = [
            out("eDP-1", 0, 0, 1920, 1080),
            out("DP-1", 1920, 0, 2560, 1440),
        ];
        // Every profile first ends a running mirror (its target is out of
        // the layout until then).
        let (plan, _) = profile_plan(DisplayProfile::InternalOnly, &outs).unwrap();
        assert_eq!(
            cmds(&plan),
            ["stop_mirror", "enable_output eDP-1", "disable_output DP-1"]
        );
        let (plan, _) = profile_plan(DisplayProfile::ExternalOnly, &outs).unwrap();
        assert_eq!(
            cmds(&plan),
            ["stop_mirror", "enable_output DP-1", "disable_output eDP-1"]
        );
        let (plan, label) = profile_plan(DisplayProfile::Extend, &outs).unwrap();
        assert_eq!(
            cmds(&plan),
            ["stop_mirror", "enable_output eDP-1", "enable_output DP-1"]
        );
        assert_eq!(label, "profile: extend");
        // Mirror is margo's own: the laptop panel shown on the external one.
        let (plan, _) = profile_plan(DisplayProfile::Mirror, &outs).unwrap();
        assert_eq!(
            cmds(&plan),
            [
                "stop_mirror",
                "enable_output eDP-1",
                "enable_output DP-1",
                "mirror_output eDP-1 DP-1"
            ]
        );
    }

    #[test]
    fn mirror_needs_both_kinds_of_output() {
        let laptop = [out("eDP-1", 0, 0, 1920, 1080)];
        assert!(profile_plan(DisplayProfile::Mirror, &laptop).is_err());
        let desktop = [
            out("DP-1", 0, 0, 2560, 1440),
            out("DP-2", 2560, 0, 2560, 1440),
        ];
        assert!(profile_plan(DisplayProfile::Mirror, &desktop).is_err());
    }

    #[test]
    fn single_kind_profiles_refuse_to_turn_every_output_off() {
        // A desktop has no laptop panel; a closed-lid laptop on the road has
        // no external monitor. "Only" profiles there would disable every
        // output and leave a black screen.
        let desktop = [out("DP-1", 0, 0, 2560, 1440)];
        assert!(profile_plan(DisplayProfile::InternalOnly, &desktop).is_err());
        let laptop = [out("eDP-1", 0, 0, 1920, 1080)];
        assert!(profile_plan(DisplayProfile::ExternalOnly, &laptop).is_err());
        assert!(profile_plan(DisplayProfile::Extend, &[]).is_err());
        // The valid cases still plan.
        assert!(profile_plan(DisplayProfile::InternalOnly, &laptop).is_ok());
        assert!(profile_plan(DisplayProfile::ExternalOnly, &desktop).is_ok());
    }
}
