//! Display → Tiling Layout sub-page — pick a global default tiling layout and
//! add per-tag overrides.
//!
//! Unlike the sibling `layout_settings` page (which drives `mlayout`, the
//! *monitor* arrangement tool), this edits the compositor's per-tag *tiling*
//! layout. mshell owns a managed `~/.config/margo/taglayouts.conf` holding a
//! `default_layout = <name>` line (applied to every tag without an override —
//! it `source`s after `config.conf` so it wins), zero or more
//! `taglayout = <tag>, <name>` override lines, and a `taglayout_force` flag.
//! mshell ensures `config.conf` `source`s the file and asks margo to reload.

use relm4::gtk::glib;
use relm4::gtk::prelude::{BoxExt, ButtonExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::path::PathBuf;

/// The 14 tiling layouts, in `LayoutId` order.
const LAYOUTS: &[&str] = &[
    "tile",
    "scroller",
    "grid",
    "monocle",
    "deck",
    "center_tile",
    "right_tile",
    "vertical_tile",
    "vertical_scroller",
    "vertical_grid",
    "vertical_deck",
    "tgmix",
    "canvas",
    "dwindle",
];

/// margo's tag count.
const MAX_TAGS: usize = 9;

fn layout_index(name: &str) -> Option<usize> {
    LAYOUTS.iter().position(|l| *l == name)
}

pub(crate) struct TagLayoutSettingsModel {
    /// Index into `LAYOUTS` for the global `default_layout`.
    default_sel: usize,
    /// Per-tag overrides: `(tag 1..=MAX_TAGS, layout index into LAYOUTS)`.
    rows: Vec<(usize, usize)>,
    force: bool,
    status: String,
}

#[derive(Debug)]
pub(crate) enum TagLayoutSettingsInput {
    DefaultChanged(usize),
    RowTagChanged(usize, usize),    // (row index, tag 1..=MAX_TAGS)
    RowLayoutChanged(usize, usize), // (row index, layout index)
    AddRow,
    RemoveRow(usize),
    ForceChanged(bool),
    Apply,
}

#[derive(Debug)]
pub(crate) enum TagLayoutSettingsOutput {}

pub(crate) struct TagLayoutSettingsInit {}

#[derive(Debug)]
pub(crate) enum TagLayoutSettingsCommandOutput {}

// ── config paths + I/O ──────────────────────────────────────────────────────

fn margo_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config"))
        .join("margo")
}

fn taglayouts_path() -> PathBuf {
    // Margo config fragments live under `conf.d/`.
    margo_dir().join("conf.d").join("taglayouts.conf")
}

/// Pull a `default_layout = <name>` value out of a conf file's text.
fn default_layout_in(text: &str) -> Option<usize> {
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("default_layout")
            && let Some(v) = rest.trim_start().strip_prefix('=')
            && let Some(idx) = layout_index(v.trim())
        {
            return Some(idx);
        }
    }
    None
}

/// Read the managed file (falling back to `config.conf` for the initial
/// default) into (default_sel, rows, force).
fn read_state() -> (usize, Vec<(usize, usize)>, bool) {
    let mut rows: Vec<(usize, usize)> = Vec::new();
    let mut force = false;
    let text = std::fs::read_to_string(taglayouts_path()).unwrap_or_default();

    // Default: our managed file wins; else the user's config.conf; else `tile`.
    let default_sel = default_layout_in(&text)
        .or_else(|| {
            std::fs::read_to_string(margo_dir().join("config.conf"))
                .ok()
                .and_then(|c| default_layout_in(&c))
        })
        .unwrap_or(0);

    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("taglayout_force") {
            if let Some(v) = rest.trim_start().strip_prefix('=') {
                force = matches!(v.trim(), "true" | "1" | "yes" | "on");
            }
        } else if let Some(rest) = line.strip_prefix("taglayout") {
            // `taglayout = <tag>, <name>` (not the `taglayout_force` key above)
            if let Some(v) = rest.trim_start().strip_prefix('=')
                && let Some((t, name)) = v.split_once(',')
                && let (Ok(tag), Some(idx)) = (t.trim().parse::<usize>(), layout_index(name.trim()))
                && (1..=MAX_TAGS).contains(&tag)
                && !rows.iter().any(|(rt, _)| *rt == tag)
            {
                rows.push((tag, idx));
            }
        }
    }
    rows.sort_by_key(|(t, _)| *t);
    (default_sel, rows, force)
}

/// `true` if `config.conf` already `source`s our file (whitespace-tolerant).
fn config_sources_us(text: &str) -> bool {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#'))
        .any(|l| {
            l.strip_prefix("source")
                .map(str::trim_start)
                .and_then(|r| r.strip_prefix('='))
                .map(|v| v.contains("taglayouts.conf"))
                .unwrap_or(false)
        })
}

fn write_and_reload(
    default_sel: usize,
    rows: &[(usize, usize)],
    force: bool,
) -> Result<(), String> {
    let dir = margo_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    if let Some(parent) = taglayouts_path().parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let mut body = String::from(
        "# Generated by mshell — Settings → Display → Tiling Layout.\n\
         # `default_layout` applies to every tag without an override below\n\
         # (it sources after config.conf, so it wins). `taglayout = <tag>,\n\
         # <name>` pins one tag. `taglayout_force = true` re-applies these on\n\
         # every margo start; false only seeds them (live changes are kept).\n\n",
    );
    body.push_str(&format!(
        "default_layout = {}\n\n",
        LAYOUTS.get(default_sel).copied().unwrap_or("tile")
    ));

    // Dedup by tag (last write wins), then emit sorted.
    let mut by_tag: Vec<(usize, usize)> = Vec::new();
    for (tag, lay) in rows {
        if (1..=MAX_TAGS).contains(tag) {
            by_tag.retain(|(t, _)| t != tag);
            by_tag.push((*tag, *lay));
        }
    }
    by_tag.sort_by_key(|(t, _)| *t);
    for (tag, lay) in &by_tag {
        body.push_str(&format!(
            "taglayout = {}, {}\n",
            tag,
            LAYOUTS.get(*lay).copied().unwrap_or("tile")
        ));
    }
    body.push_str(&format!("\ntaglayout_force = {force}\n"));
    std::fs::write(taglayouts_path(), body).map_err(|e| e.to_string())?;

    // Make sure config.conf pulls it in (append once).
    let config_conf = dir.join("config.conf");
    let current = std::fs::read_to_string(&config_conf).unwrap_or_default();
    if !config_sources_us(&current) {
        let mut updated = current;
        if !updated.ends_with('\n') && !updated.is_empty() {
            updated.push('\n');
        }
        updated.push_str(
            "\n# Per-tag tiling layouts (managed by mshell).\nsource = conf.d/taglayouts.conf\n",
        );
        std::fs::write(&config_conf, updated).map_err(|e| e.to_string())?;
    }

    // Ask margo to reload.
    let out = std::process::Command::new("mctl")
        .arg("reload")
        .output()
        .map_err(|e| format!("mctl reload: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "mctl reload failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

/// Lowest tag (1..=MAX_TAGS) not already overridden, if any.
fn next_free_tag(rows: &[(usize, usize)]) -> Option<usize> {
    (1..=MAX_TAGS).find(|t| !rows.iter().any(|(rt, _)| rt == t))
}

// ── row rendering (rebuilt on add/remove) ────────────────────────────────────

/// Tear down + rebuild the per-tag override rows from `rows`. Called on
/// add/remove only (not on a dropdown value change), so a row's controls
/// aren't destroyed mid-interaction.
fn rebuild_rows(
    tags_box: &gtk::Box,
    rows: &[(usize, usize)],
    sender: &ComponentSender<TagLayoutSettingsModel>,
) {
    while let Some(child) = tags_box.first_child() {
        tags_box.remove(&child);
    }

    if rows.is_empty() {
        let empty = gtk::Label::new(Some(
            "No per-tag overrides — every tag uses the default layout above. Add one below.",
        ));
        empty.add_css_class("label-small");
        empty.set_halign(gtk::Align::Start);
        empty.set_xalign(0.0);
        empty.set_wrap(true);
        tags_box.append(&empty);
        return;
    }

    let tag_options: Vec<String> = (1..=MAX_TAGS).map(|t| format!("Tag {t}")).collect();
    let tag_refs: Vec<&str> = tag_options.iter().map(String::as_str).collect();

    for (idx, (tag, lay)) in rows.iter().enumerate() {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);

        // Tag picker (1..=MAX_TAGS).
        let tag_dd = gtk::DropDown::from_strings(&tag_refs);
        tag_dd.set_valign(gtk::Align::Center);
        tag_dd.set_selected((tag.saturating_sub(1)) as u32);
        let s = sender.clone();
        tag_dd.connect_selected_notify(move |d| {
            s.input(TagLayoutSettingsInput::RowTagChanged(
                idx,
                d.selected() as usize + 1,
            ));
        });
        row.append(&tag_dd);

        // Layout picker (14 layouts).
        let lay_dd = gtk::DropDown::from_strings(LAYOUTS);
        lay_dd.set_valign(gtk::Align::Center);
        lay_dd.set_hexpand(true);
        lay_dd.set_selected(*lay as u32);
        let s = sender.clone();
        lay_dd.connect_selected_notify(move |d| {
            s.input(TagLayoutSettingsInput::RowLayoutChanged(
                idx,
                d.selected() as usize,
            ));
        });
        row.append(&lay_dd);

        // Remove.
        let remove = gtk::Button::from_icon_name("user-trash-symbolic");
        remove.add_css_class("ok-button-flat");
        remove.set_valign(gtk::Align::Center);
        remove.set_tooltip_text(Some(
            "Remove this override (the tag falls back to the default)",
        ));
        let s = sender.clone();
        remove.connect_clicked(move |_| {
            s.input(TagLayoutSettingsInput::RemoveRow(idx));
        });
        row.append(&remove);

        tags_box.append(&row);
    }
}

// ── Component ────────────────────────────────────────────────────────────────

#[relm4::component(pub)]
impl Component for TagLayoutSettingsModel {
    type CommandOutput = TagLayoutSettingsCommandOutput;
    type Input = TagLayoutSettingsInput;
    type Output = TagLayoutSettingsOutput;
    type Init = TagLayoutSettingsInit;

    view! {
        #[root]
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
                    set_orientation: gtk::Orientation::Horizontal,
                    set_halign: gtk::Align::Start,
                    set_spacing: 16,
                    gtk::Image {
                        add_css_class: "settings-hero-icon",
                        set_icon_name: Some("view-grid-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "Tiling Layout",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "Pick a default tiling layout, then override individual tags.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                // ── Default layout ──
                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    gtk::Box {
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_halign: gtk::Align::Start,
                                set_label: "Default layout",
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "Used by every tag without an override below.",
                                set_xalign: 0.0,
                                set_wrap: true,
                            },
                        },
                        #[name = "default_dd_slot"]
                        gtk::Box {
                            set_valign: gtk::Align::Center,
                        },
                    },
                },

                gtk::Separator {},

                gtk::Label {
                    add_css_class: "label-medium-bold",
                    set_halign: gtk::Align::Start,
                    set_label: "Per-tag overrides",
                },

                // Per-tag override rows (rebuilt in init / on add+remove).
                #[name = "tags_box"]
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 8,
                },

                gtk::Button {
                    add_css_class: "ok-button-surface",
                    set_halign: gtk::Align::Start,
                    set_label: "＋ Add tag",
                    connect_clicked => TagLayoutSettingsInput::AddRow,
                },

                gtk::Separator {},

                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    gtk::Box {
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            set_hexpand: true,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_halign: gtk::Align::Start,
                                set_label: "Force on startup",
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_label: "Re-apply these layouts on every start, overriding live changes.",
                                set_xalign: 0.0,
                                set_wrap: true,
                            },
                        },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(force_handler)]
                            set_active: model.force,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(TagLayoutSettingsInput::ForceChanged(v));
                                glib::Propagation::Proceed
                            } @force_handler,
                        },
                    },
                },

                gtk::Button {
                    add_css_class: "ok-button-primary",
                    set_halign: gtk::Align::Start,
                    set_label: "Apply",
                    connect_clicked => TagLayoutSettingsInput::Apply,
                },

                gtk::Label {
                    add_css_class: "label-small",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    #[watch]
                    set_label: &model.status,
                    #[watch]
                    set_visible: !model.status.is_empty(),
                },
            }
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let (default_sel, rows, force) = read_state();
        let model = TagLayoutSettingsModel {
            default_sel,
            rows,
            force,
            status: String::new(),
        };

        let widgets = view_output!();

        // Default-layout dropdown — built here so we can set-then-connect
        // (no spurious DefaultChanged on first paint).
        let default_dd = gtk::DropDown::from_strings(LAYOUTS);
        default_dd.set_selected(model.default_sel as u32);
        let s = sender.clone();
        default_dd.connect_selected_notify(move |d| {
            s.input(TagLayoutSettingsInput::DefaultChanged(d.selected() as usize));
        });
        widgets.default_dd_slot.append(&default_dd);

        rebuild_rows(&widgets.tags_box, &model.rows, &sender);

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
            TagLayoutSettingsInput::DefaultChanged(sel) => self.default_sel = sel,
            TagLayoutSettingsInput::RowTagChanged(idx, tag) => {
                if let Some(r) = self.rows.get_mut(idx) {
                    r.0 = tag;
                }
            }
            TagLayoutSettingsInput::RowLayoutChanged(idx, lay) => {
                if let Some(r) = self.rows.get_mut(idx) {
                    r.1 = lay;
                }
            }
            TagLayoutSettingsInput::AddRow => {
                if let Some(tag) = next_free_tag(&self.rows) {
                    self.rows.push((tag, self.default_sel));
                    rebuild_rows(&widgets.tags_box, &self.rows, &sender);
                } else {
                    self.status = "All tags already have an override.".to_string();
                }
            }
            TagLayoutSettingsInput::RemoveRow(idx) => {
                if idx < self.rows.len() {
                    self.rows.remove(idx);
                    rebuild_rows(&widgets.tags_box, &self.rows, &sender);
                }
            }
            TagLayoutSettingsInput::ForceChanged(v) => self.force = v,
            TagLayoutSettingsInput::Apply => {
                self.status = match write_and_reload(self.default_sel, &self.rows, self.force) {
                    Ok(()) => "Applied — the default and any overrides are live now. With Force on startup off, a layout you later change live is kept by the session.".to_string(),
                    Err(e) => format!("Couldn't apply: {e}"),
                };
            }
        }
        self.update_view(widgets, sender);
    }
}
