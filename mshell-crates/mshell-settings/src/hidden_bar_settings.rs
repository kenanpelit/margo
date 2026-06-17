//! Hidden Bar widget settings — the drawer behaviour knobs
//! (`bars.widgets.hidden_bar`). The *which widgets* part is handled by the
//! reusable bar-widget section editors (TopHidden / BottomHidden) composed
//! alongside this on the Widgets → Hidden Bar page.

use crate::bar_settings::bar_widget_factory::BarListLocation;
use crate::bar_settings::bar_widget_section::{BarSection, WidgetSectionInit, WidgetSectionModel};
use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::bar_widgets::BarWidget;
use mshell_config::schema::config::{
    BarWidgetsStoreFields, BarsStoreFields, ConfigStoreFields, HiddenBarConfig,
    HiddenBarConfigStoreFields,
};
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::glib;
use relm4::gtk::prelude::{BoxExt, ButtonExt, EditableExt, EntryExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentController, ComponentParts, ComponentSender, Controller, gtk};

pub(crate) struct HiddenBarSettingsModel {
    start_expanded: bool,
    auto_expand: bool,
    hover_delay_ms: u32,
    auto_collapse: bool,
    collapse_delay_ms: u32,
    /// Container the named-drawer cards are (re)built into.
    drawers_container: gtk::Box,
    /// One widget-list editor per named drawer; kept alive here. Rebuilt
    /// (with fresh indices) whenever the `hidden_bars` list changes.
    drawer_sections: Vec<Controller<WidgetSectionModel>>,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum HiddenBarSettingsInput {
    StartExpandedChanged(bool),
    AutoExpandChanged(bool),
    HoverDelayChanged(u32),
    AutoCollapseChanged(bool),
    CollapseDelayChanged(u32),
    StartExpandedEffect(bool),
    AutoExpandEffect(bool),
    HoverDelayEffect(u32),
    AutoCollapseEffect(bool),
    CollapseDelayEffect(u32),
    /// Append a new named drawer.
    AddDrawer,
    /// Rename the drawer at this index.
    RenameDrawer(usize, String),
    /// Delete the drawer at this index (and any pills referencing its name).
    RemoveDrawer(usize),
    /// The `hidden_bars` list changed — rebuild the cards.
    RebuildDrawers,
}

#[derive(Debug)]
pub(crate) enum HiddenBarSettingsOutput {}

pub(crate) struct HiddenBarSettingsInit {}

#[derive(Debug)]
pub(crate) enum HiddenBarSettingsCommandOutput {}

#[relm4::component(pub)]
impl Component for HiddenBarSettingsModel {
    type CommandOutput = HiddenBarSettingsCommandOutput;
    type Input = HiddenBarSettingsInput;
    type Output = HiddenBarSettingsOutput;
    type Init = HiddenBarSettingsInit;

    view! {
        #[root]
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
                    set_icon_name: Some("view-more-horizontal-symbolic"),
                    set_valign: gtk::Align::Center,
                },
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_valign: gtk::Align::Center,
                    gtk::Label {
                        add_css_class: "settings-hero-title",
                        set_halign: gtk::Align::Start,
                        set_label: "Hidden Bar",
                    },
                    gtk::Label {
                        add_css_class: "settings-hero-subtitle",
                        set_halign: gtk::Align::Start,
                        set_label: "Collapse bar widgets behind a drawer. Pick which widgets to hide below; left-click the trigger to toggle, right-click to pin.",
                        set_wrap: true,
                        set_xalign: 0.0,
                    },
                },
            },

            gtk::Box {
                add_css_class: "boxed-list",
                set_orientation: gtk::Orientation::Vertical,

                // Reveal on hover
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
                            set_label: "Reveal on hover",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Expand the drawer when the pointer hovers the trigger (in addition to clicking).",
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        #[watch]
                        #[block_signal(auto_expand_handler)]
                        set_active: model.auto_expand,
                        connect_state_set[sender] => move |_, v| {
                            sender.input(HiddenBarSettingsInput::AutoExpandChanged(v));
                            glib::Propagation::Proceed
                        } @auto_expand_handler,
                    },
                },

                // Hover delay
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
                            set_label: "Hover delay (ms)",
                            set_hexpand: true,
                        },
                    },
                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_range: (0.0, 5000.0),
                        set_increments: (50.0, 250.0),
                        set_digits: 0,
                        #[watch]
                        #[block_signal(hover_delay_handler)]
                        set_value: model.hover_delay_ms as f64,
                        connect_value_changed[sender] => move |s| {
                            sender.input(HiddenBarSettingsInput::HoverDelayChanged(s.value() as u32));
                        } @hover_delay_handler,
                    },
                },

                // Auto-collapse
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
                            set_label: "Auto-collapse",
                            set_hexpand: true,
                        },
                        gtk::Label {
                            add_css_class: "label-small",
                            set_halign: gtk::Align::Start,
                            set_label: "Collapse again after the pointer leaves (unless pinned with right-click).",
                            set_xalign: 0.0,
                            set_wrap: true,
                            set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                        },
                    },
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        #[watch]
                        #[block_signal(auto_collapse_handler)]
                        set_active: model.auto_collapse,
                        connect_state_set[sender] => move |_, v| {
                            sender.input(HiddenBarSettingsInput::AutoCollapseChanged(v));
                            glib::Propagation::Proceed
                        } @auto_collapse_handler,
                    },
                },

                // Collapse delay
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
                            set_label: "Collapse delay (ms)",
                            set_hexpand: true,
                        },
                    },
                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_range: (0.0, 10000.0),
                        set_increments: (100.0, 500.0),
                        set_digits: 0,
                        #[watch]
                        #[block_signal(collapse_delay_handler)]
                        set_value: model.collapse_delay_ms as f64,
                        connect_value_changed[sender] => move |s| {
                            sender.input(HiddenBarSettingsInput::CollapseDelayChanged(s.value() as u32));
                        } @collapse_delay_handler,
                    },
                },

                // Start expanded
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
                            set_label: "Start expanded",
                            set_hexpand: true,
                        },
                    },
                    gtk::Switch {
                        set_valign: gtk::Align::Center,
                        #[watch]
                        #[block_signal(start_expanded_handler)]
                        set_active: model.start_expanded,
                        connect_state_set[sender] => move |_, v| {
                            sender.input(HiddenBarSettingsInput::StartExpandedChanged(v));
                            glib::Propagation::Proceed
                        } @start_expanded_handler,
                    },
                },
            },

            // ── Named drawers ─────────────────────────────────────────────
            // Define extra, independently-addressable drawers. Each has its
            // own widget list; place one in a bar via the "Add widget" menu
            // (it appears as "Hidden Bar · <name>"), and toggle it from the
            // CLI with `mshellctl hidden-bar toggle <name>`.
            gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 4,
                gtk::Label {
                    add_css_class: "label-large",
                    set_halign: gtk::Align::Start,
                    set_label: "Named drawers",
                },
                gtk::Label {
                    add_css_class: "label-small",
                    set_halign: gtk::Align::Start,
                    set_label: "Extra drawers, each with its own widgets. Place one in a bar from the \"Add widget\" menu (\"Hidden Bar · <name>\"); toggle it with `mshellctl hidden-bar toggle <name>`.",
                    set_wrap: true,
                    set_xalign: 0.0,
                },
            },

            #[local_ref]
            drawers_container -> gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 12,
            },

            gtk::Button {
                add_css_class: "settings-bar-widget-add-item",
                set_halign: gtk::Align::Start,
                set_label: "Add drawer",
                connect_clicked[sender] => move |_| {
                    sender.input(HiddenBarSettingsInput::AddDrawer);
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let mut effects = EffectScope::new();

        macro_rules! push_effect {
            ($field:ident, $variant:ident) => {{
                let sc = sender.clone();
                effects.push(move |_| {
                    let v = config_manager()
                        .config()
                        .bars()
                        .widgets()
                        .hidden_bar()
                        .$field()
                        .get();
                    sc.input(HiddenBarSettingsInput::$variant(v));
                });
            }};
        }
        push_effect!(start_expanded, StartExpandedEffect);
        push_effect!(auto_expand, AutoExpandEffect);
        push_effect!(hover_delay_ms, HoverDelayEffect);
        push_effect!(auto_collapse, AutoCollapseEffect);
        push_effect!(collapse_delay_ms, CollapseDelayEffect);

        // Rebuild the named-drawer cards whenever the list changes (add /
        // remove / rename / widget edit). Fires once on init to populate.
        {
            let sc = sender.clone();
            effects.push(move |_| {
                let _ = config_manager()
                    .config()
                    .bars()
                    .widgets()
                    .hidden_bars()
                    .get();
                sc.input(HiddenBarSettingsInput::RebuildDrawers);
            });
        }

        let drawers_container = gtk::Box::default();

        macro_rules! read {
            ($field:ident) => {
                config_manager()
                    .config()
                    .bars()
                    .widgets()
                    .hidden_bar()
                    .$field()
                    .get_untracked()
            };
        }
        let model = HiddenBarSettingsModel {
            start_expanded: read!(start_expanded),
            auto_expand: read!(auto_expand),
            hover_delay_ms: read!(hover_delay_ms),
            auto_collapse: read!(auto_collapse),
            collapse_delay_ms: read!(collapse_delay_ms),
            drawers_container: drawers_container.clone(),
            drawer_sections: Vec::new(),
            _effects: effects,
        };

        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            HiddenBarSettingsInput::StartExpandedChanged(v) => {
                config_manager()
                    .update_config(move |c| c.bars.widgets.hidden_bar.start_expanded = v);
            }
            HiddenBarSettingsInput::AutoExpandChanged(v) => {
                config_manager().update_config(move |c| c.bars.widgets.hidden_bar.auto_expand = v);
            }
            HiddenBarSettingsInput::HoverDelayChanged(v) => {
                config_manager()
                    .update_config(move |c| c.bars.widgets.hidden_bar.hover_delay_ms = v);
            }
            HiddenBarSettingsInput::AutoCollapseChanged(v) => {
                config_manager()
                    .update_config(move |c| c.bars.widgets.hidden_bar.auto_collapse = v);
            }
            HiddenBarSettingsInput::CollapseDelayChanged(v) => {
                config_manager()
                    .update_config(move |c| c.bars.widgets.hidden_bar.collapse_delay_ms = v);
            }
            HiddenBarSettingsInput::StartExpandedEffect(v) => self.start_expanded = v,
            HiddenBarSettingsInput::AutoExpandEffect(v) => self.auto_expand = v,
            HiddenBarSettingsInput::HoverDelayEffect(v) => self.hover_delay_ms = v,
            HiddenBarSettingsInput::AutoCollapseEffect(v) => self.auto_collapse = v,
            HiddenBarSettingsInput::CollapseDelayEffect(v) => self.collapse_delay_ms = v,
            HiddenBarSettingsInput::AddDrawer => {
                config_manager().update_config(|c| {
                    let name = unique_drawer_name(&c.bars.widgets.hidden_bars);
                    c.bars.widgets.hidden_bars.push(HiddenBarConfig {
                        name,
                        ..HiddenBarConfig::default()
                    });
                });
            }
            HiddenBarSettingsInput::RenameDrawer(idx, new_name) => {
                let new_name = new_name.trim().to_string();
                if new_name.is_empty() {
                    return;
                }
                config_manager().update_config(move |c| {
                    // Ignore a clash with another drawer's name.
                    if c.bars
                        .widgets
                        .hidden_bars
                        .iter()
                        .enumerate()
                        .any(|(j, d)| j != idx && d.name == new_name)
                    {
                        return;
                    }
                    let Some(old) = c.bars.widgets.hidden_bars.get(idx).map(|d| d.name.clone())
                    else {
                        return;
                    };
                    if old == new_name {
                        return;
                    }
                    c.bars.widgets.hidden_bars[idx].name = new_name.clone();
                    // Repoint every pill that referenced the old name.
                    rename_drawer_refs(c, &old, &new_name);
                });
            }
            HiddenBarSettingsInput::RemoveDrawer(idx) => {
                config_manager().update_config(move |c| {
                    if idx >= c.bars.widgets.hidden_bars.len() {
                        return;
                    }
                    let name = c.bars.widgets.hidden_bars.remove(idx).name;
                    // Drop any pills that referenced the removed drawer.
                    remove_drawer_refs(c, &name);
                });
            }
            HiddenBarSettingsInput::RebuildDrawers => {
                self.rebuild_drawers(&sender);
            }
        }
    }
}

/// Pick a `drawerN` name not already taken.
fn unique_drawer_name(existing: &[HiddenBarConfig]) -> String {
    (1..)
        .map(|n| format!("drawer{n}"))
        .find(|cand| !existing.iter().any(|d| &d.name == cand))
        .unwrap_or_else(|| "drawer".to_string())
}

/// Rewrite `!HiddenBarNamed old` pills to `new` across every bar slot.
fn rename_drawer_refs(c: &mut mshell_config::schema::config::Config, old: &str, new: &str) {
    for list in all_slot_lists(c) {
        for w in list.iter_mut() {
            if let BarWidget::HiddenBarNamed(n) = w
                && n == old
            {
                *n = new.to_string();
            }
        }
    }
}

/// Drop `!HiddenBarNamed name` pills from every bar slot.
fn remove_drawer_refs(c: &mut mshell_config::schema::config::Config, name: &str) {
    for list in all_slot_lists(c) {
        list.retain(|w| !matches!(w, BarWidget::HiddenBarNamed(n) if n == name));
    }
}

/// Every bar-slot widget list, for cross-slot reference fixups.
fn all_slot_lists(c: &mut mshell_config::schema::config::Config) -> Vec<&mut Vec<BarWidget>> {
    vec![
        &mut c.bars.top_bar.left_widgets,
        &mut c.bars.top_bar.center_widgets,
        &mut c.bars.top_bar.right_widgets,
        &mut c.bars.top_bar.hidden_widgets,
        &mut c.bars.bottom_bar.left_widgets,
        &mut c.bars.bottom_bar.center_widgets,
        &mut c.bars.bottom_bar.right_widgets,
        &mut c.bars.bottom_bar.hidden_widgets,
    ]
}

impl HiddenBarSettingsModel {
    /// Rebuild the named-drawer cards from the current `hidden_bars` list,
    /// recreating each drawer's widget-list editor with a fresh index.
    fn rebuild_drawers(&mut self, sender: &ComponentSender<Self>) {
        while let Some(child) = self.drawers_container.first_child() {
            self.drawers_container.remove(&child);
        }
        self.drawer_sections.clear();

        let drawers = config_manager()
            .config()
            .bars()
            .widgets()
            .hidden_bars()
            .get_untracked();

        for (i, hb) in drawers.iter().enumerate() {
            let card = gtk::Box::new(gtk::Orientation::Vertical, 8);
            card.add_css_class("boxed-list");

            // Header: name entry + remove.
            let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            header.add_css_class("action-row");

            let entry = gtk::Entry::new();
            entry.set_hexpand(true);
            entry.set_text(&hb.name);
            entry.set_placeholder_text(Some("drawer name"));
            {
                let sc = sender.clone();
                entry.connect_activate(move |e| {
                    sc.input(HiddenBarSettingsInput::RenameDrawer(
                        i,
                        e.text().to_string(),
                    ));
                });
            }
            {
                let sc = sender.clone();
                let entry_for_focus = entry.clone();
                let focus = gtk::EventControllerFocus::new();
                focus.connect_leave(move |_| {
                    sc.input(HiddenBarSettingsInput::RenameDrawer(
                        i,
                        entry_for_focus.text().to_string(),
                    ));
                });
                entry.add_controller(focus);
            }
            header.append(&entry);

            let remove = gtk::Button::from_icon_name("user-trash-symbolic");
            remove.set_valign(gtk::Align::Center);
            remove.set_tooltip_text(Some("Delete this drawer"));
            {
                let sc = sender.clone();
                remove.connect_clicked(move |_| {
                    sc.input(HiddenBarSettingsInput::RemoveDrawer(i));
                });
            }
            header.append(&remove);
            card.append(&header);

            // The drawer's own widget list (add / remove / reorder), bound to
            // its `hidden_bars[i].widgets` via the NamedDrawer location.
            let section = WidgetSectionModel::builder()
                .launch(WidgetSectionInit {
                    bar_section: BarSection::Drawer,
                    location: BarListLocation::NamedDrawer(i),
                    widgets: hb.widgets.clone(),
                })
                .detach();
            card.append(section.widget());
            self.drawer_sections.push(section);

            self.drawers_container.append(&card);
        }
    }
}
