//! Settings → Theme → Apps → Helium.
//!
//! Helium stores each isolated browser root as a Chromium user-data-dir under
//! `~/.helium/isolated`. Each root can still contain several Chromium
//! profiles (`Default`, `Profile 1`, …), so the settings page discovers
//! targets from `Local State` plus real `Preferences` files instead of
//! hard-coding `Kenp/Default`.

use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{HeliumProfileTarget, HeliumTargetMode, HeliumTheme};
use mshell_style::app_theme::{
    HeliumProfile, apply_helium_from_cache, discover_helium_profiles, expand_home,
};
use reactive_graph::traits::ReadUntracked;
use relm4::gtk::prelude::{BoxExt, ButtonExt, EditableExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

#[derive(Debug)]
pub(crate) struct HeliumThemeSettingsModel {
    config: HeliumTheme,
    profiles: Vec<HeliumProfile>,
    target_modes: gtk::StringList,
    status: String,
}

#[derive(Debug)]
pub(crate) enum HeliumThemeSettingsInput {
    EnabledChanged(bool),
    ApplyOnThemeChanged(bool),
    RootChanged(String),
    TargetModeChanged(HeliumTargetMode),
    RefreshProfiles,
    TargetToggled {
        instance: String,
        profile: String,
        enabled: bool,
    },
    ApplyNow,
}

#[derive(Debug)]
pub(crate) enum HeliumThemeSettingsOutput {}

pub(crate) struct HeliumThemeSettingsInit {}

#[derive(Debug)]
pub(crate) enum HeliumThemeSettingsCommandOutput {}

#[relm4::component(pub)]
impl Component for HeliumThemeSettingsModel {
    type CommandOutput = HeliumThemeSettingsCommandOutput;
    type Input = HeliumThemeSettingsInput;
    type Output = HeliumThemeSettingsOutput;
    type Init = HeliumThemeSettingsInit;

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
                        set_icon_name: Some("web-browser-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "App Themes",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "Let external apps follow the current matugen palette. Helium uses Chromium's native user-colour theme and keeps isolated profiles separate.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Helium",
                    set_halign: gtk::Align::Start,
                },

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
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_halign: gtk::Align::Start,
                                set_label: "Follow matugen",
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_label: "Write Helium's Chromium theme seed when the shell palette changes.",
                            },
                        },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(enabled_handler)]
                            set_active: model.config.enabled,
                            connect_active_notify[sender] => move |s| {
                                sender.input(HeliumThemeSettingsInput::EnabledChanged(s.is_active()));
                            } @enabled_handler,
                        },
                    },

                    gtk::Box {
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_halign: gtk::Align::Start,
                                set_label: "Apply after theme changes",
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_label: "Runs automatically after matugen finishes. Helium may need a restart to show the new colour.",
                            },
                        },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(auto_handler)]
                            set_active: model.config.apply_on_theme_change,
                            connect_active_notify[sender] => move |s| {
                                sender.input(HeliumThemeSettingsInput::ApplyOnThemeChanged(s.is_active()));
                            } @auto_handler,
                        },
                    },

                    gtk::Box {
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_halign: gtk::Align::Start,
                                set_label: "Isolated root",
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_label: "Directory that contains Helium isolated browser roots.",
                            },
                        },
                        #[name = "root_entry"]
                        gtk::Entry {
                            set_valign: gtk::Align::Center,
                            set_width_request: 260,
                            set_text: &model.config.isolated_root,
                            connect_changed[sender] => move |e| {
                                sender.input(HeliumThemeSettingsInput::RootChanged(e.text().to_string()));
                            },
                        },
                    },

                    gtk::Box {
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        gtk::Box {
                            set_orientation: gtk::Orientation::Vertical,
                            set_valign: gtk::Align::Center,
                            gtk::Label {
                                add_css_class: "label-medium-bold",
                                set_halign: gtk::Align::Start,
                                set_label: "Targets",
                            },
                            gtk::Label {
                                add_css_class: "label-small",
                                set_halign: gtk::Align::Start,
                                set_xalign: 0.0,
                                set_wrap: true,
                                set_label: "Last-used is safest with multiple isolated roots; selected lets you pin exact profiles like Kenp/Default.",
                            },
                        },
                        gtk::DropDown {
                            set_width_request: 210,
                            set_valign: gtk::Align::Center,
                            set_model: Some(&model.target_modes),
                            #[watch]
                            #[block_signal(mode_handler)]
                            set_selected: mode_index(model.config.target_mode),
                            connect_selected_notify[sender] => move |dd| {
                                if let Some(mode) = HeliumTargetMode::all().get(dd.selected() as usize) {
                                    sender.input(HeliumThemeSettingsInput::TargetModeChanged(*mode));
                                }
                            } @mode_handler,
                        },
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 8,
                    gtk::Button {
                        add_css_class: "pill-button",
                        set_label: "Refresh profiles",
                        connect_clicked[sender] => move |_| {
                            sender.input(HeliumThemeSettingsInput::RefreshProfiles);
                        },
                    },
                    gtk::Button {
                        add_css_class: "suggested-action",
                        set_label: "Apply now",
                        connect_clicked[sender] => move |_| {
                            sender.input(HeliumThemeSettingsInput::ApplyNow);
                        },
                    },
                },

                gtk::Label {
                    add_css_class: "label-small",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    #[watch]
                    set_label: &model.status,
                },

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Discovered profiles",
                    set_halign: gtk::Align::Start,
                },

                #[name = "profiles_box"]
                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,
                },
            },
        }
    }

    fn init(
        _init: Self::Init,
        _root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let config = config_manager()
            .config()
            .read_untracked()
            .theme
            .apps
            .helium
            .clone();
        let profiles = discover_helium_profiles(&expand_home(&config.isolated_root));
        let target_modes = gtk::StringList::new(
            &HeliumTargetMode::all()
                .iter()
                .map(|m| m.display_name())
                .collect::<Vec<_>>(),
        );
        let status = profile_status(&profiles);
        let model = HeliumThemeSettingsModel {
            config,
            profiles,
            target_modes,
            status,
        };
        let widgets = view_output!();
        populate_profiles(
            &widgets.profiles_box,
            &model.profiles,
            &model.config,
            &sender,
        );
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
            HeliumThemeSettingsInput::EnabledChanged(v) => {
                self.config.enabled = v;
                config_manager().update_config(|c| c.theme.apps.helium.enabled = v);
            }
            HeliumThemeSettingsInput::ApplyOnThemeChanged(v) => {
                self.config.apply_on_theme_change = v;
                config_manager().update_config(|c| c.theme.apps.helium.apply_on_theme_change = v);
            }
            HeliumThemeSettingsInput::RootChanged(root) => {
                if self.config.isolated_root != root {
                    self.config.isolated_root = root;
                    self.status = "Root edited. Refresh profiles to rescan.".to_string();
                }
            }
            HeliumThemeSettingsInput::TargetModeChanged(mode) => {
                self.config.target_mode = mode;
                config_manager().update_config(|c| c.theme.apps.helium.target_mode = mode);
            }
            HeliumThemeSettingsInput::RefreshProfiles => {
                let root = self.config.isolated_root.clone();
                config_manager().update_config(|c| c.theme.apps.helium.isolated_root = root);
                self.profiles = discover_helium_profiles(&expand_home(&self.config.isolated_root));
                self.status = profile_status(&self.profiles);
                populate_profiles(&widgets.profiles_box, &self.profiles, &self.config, &sender);
            }
            HeliumThemeSettingsInput::TargetToggled {
                instance,
                profile,
                enabled,
            } => {
                set_target(&mut self.config.targets, &instance, &profile, enabled);
                let targets = self.config.targets.clone();
                config_manager().update_config(|c| c.theme.apps.helium.targets = targets);
                populate_profiles(&widgets.profiles_box, &self.profiles, &self.config, &sender);
            }
            HeliumThemeSettingsInput::ApplyNow => {
                let root = self.config.isolated_root.clone();
                config_manager().update_config(|c| c.theme.apps.helium.isolated_root = root);
                let report = apply_helium_from_cache(&self.config);
                self.status = report.summary();
                if report.is_ok() {
                    mshell_launcher::notify::toast("Helium theme", self.status.clone());
                } else {
                    mshell_launcher::notify::toast("Helium theme failed", report.errors.join("\n"));
                }
            }
        }
        self.update_view(widgets, sender);
    }
}

fn mode_index(mode: HeliumTargetMode) -> u32 {
    HeliumTargetMode::all()
        .iter()
        .position(|m| *m == mode)
        .unwrap_or(0) as u32
}

fn profile_status(profiles: &[HeliumProfile]) -> String {
    if profiles.is_empty() {
        "No Helium profiles found under the configured isolated root.".to_string()
    } else {
        format!("{} profiles discovered.", profiles.len())
    }
}

fn target_enabled(config: &HeliumTheme, profile: &HeliumProfile) -> bool {
    config
        .targets
        .iter()
        .find(|t| t.instance == profile.instance && t.profile == profile.profile)
        .map(|t| t.enabled)
        .unwrap_or(profile.last_used)
}

fn set_target(
    targets: &mut Vec<HeliumProfileTarget>,
    instance: &str,
    profile: &str,
    enabled: bool,
) {
    if let Some(target) = targets
        .iter_mut()
        .find(|t| t.instance == instance && t.profile == profile)
    {
        target.enabled = enabled;
        return;
    }
    targets.push(HeliumProfileTarget {
        instance: instance.to_string(),
        profile: profile.to_string(),
        enabled,
    });
}

fn populate_profiles(
    container: &gtk::Box,
    profiles: &[HeliumProfile],
    config: &HeliumTheme,
    sender: &ComponentSender<HeliumThemeSettingsModel>,
) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }

    if profiles.is_empty() {
        let label = gtk::Label::builder()
            .css_classes(["label-small"])
            .label("No Local State / Preferences pairs were found.")
            .halign(gtk::Align::Start)
            .xalign(0.0)
            .wrap(true)
            .build();
        container.append(&label);
        return;
    }

    for profile in profiles {
        let row = gtk::Box::builder()
            .css_classes(["action-row"])
            .orientation(gtk::Orientation::Horizontal)
            .spacing(20)
            .build();

        let text = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .valign(gtk::Align::Center)
            .build();
        let title = format!(
            "{} / {} ({})",
            profile.instance, profile.display_name, profile.profile
        );
        let title_label = gtk::Label::builder()
            .css_classes(["label-medium-bold"])
            .label(title)
            .halign(gtk::Align::Start)
            .xalign(0.0)
            .build();
        let mut detail = profile.preferences_path.display().to_string();
        if profile.last_used {
            detail.push_str(" · last used");
        }
        let detail_label = gtk::Label::builder()
            .css_classes(["label-small"])
            .label(detail)
            .halign(gtk::Align::Start)
            .xalign(0.0)
            .wrap(true)
            .build();
        text.append(&title_label);
        text.append(&detail_label);
        row.append(&text);

        let sw = gtk::Switch::builder()
            .valign(gtk::Align::Center)
            .active(target_enabled(config, profile))
            .build();
        let instance = profile.instance.clone();
        let profile_dir = profile.profile.clone();
        let sender = sender.clone();
        sw.connect_active_notify(move |s| {
            sender.input(HeliumThemeSettingsInput::TargetToggled {
                instance: instance.clone(),
                profile: profile_dir.clone(),
                enabled: s.is_active(),
            });
        });
        row.append(&sw);
        container.append(&row);
    }
}
