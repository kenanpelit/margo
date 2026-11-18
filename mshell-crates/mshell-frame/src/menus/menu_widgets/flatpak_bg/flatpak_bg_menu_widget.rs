//! Flatpak Background menu — the panel for `MenuType::FlatpakBackground`:
//! Flatpak apps that run without a window (see [`crate::flatpak_bg`]),
//! one boxed-list row each with its icon, name and id, *Stop* (`flatpak
//! kill`) and *Ignore* (adds it to `flatpak_bg.ignored_apps`). The header
//! rescans and stops all. Without `flatpak` the panel says so.

use crate::flatpak_bg::{self, BgApp, FlatpakBgState};
use crate::menus::menu_widgets::panel_header::PanelHeader;
use crate::menus::reveal_poll::{RevealGate, watch_stream};
use mshell_config::config_manager::config_manager;
use mshell_config::schema::menu_widgets::MenuWidget;
use relm4::gtk::prelude::{AppInfoExt, BoxExt, ButtonExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

pub(crate) struct FlatpakBackgroundMenuWidgetModel {
    state: FlatpakBgState,
    list: gtk::Box,
    subtitle: gtk::Label,
}

#[derive(Debug)]
pub(crate) enum FlatpakBackgroundMenuWidgetInput {
    Stop(String),
    StopAll,
    Ignore(String),
    Refresh,
}

#[derive(Debug)]
pub(crate) enum FlatpakBackgroundMenuWidgetOutput {}

pub(crate) struct FlatpakBackgroundMenuWidgetInit {}

#[derive(Debug)]
pub(crate) enum FlatpakBackgroundMenuWidgetCommandOutput {
    State(FlatpakBgState),
}

#[relm4::component(pub(crate))]
impl Component for FlatpakBackgroundMenuWidgetModel {
    type CommandOutput = FlatpakBackgroundMenuWidgetCommandOutput;
    type Input = FlatpakBackgroundMenuWidgetInput;
    type Output = FlatpakBackgroundMenuWidgetOutput;
    type Init = FlatpakBackgroundMenuWidgetInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "flatpak-bg-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 10,

            #[local_ref]
            header_root -> gtk::Box {},

            #[name = "note"]
            gtk::Label {
                set_css_classes: &["label-small", "dim-label"],
                set_xalign: 0.0,
                set_wrap: true,
            },

            #[name = "fill_list"]
            gtk::ScrolledWindow {
                set_hscrollbar_policy: gtk::PolicyType::Never,
                set_min_content_height: 0,
                set_propagate_natural_height: true,

                #[local_ref]
                list -> gtk::Box {
                    set_css_classes: &["ok-boxed-list", "compact", "flatpak-bg-list"],
                    set_orientation: gtk::Orientation::Vertical,
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let (s_refresh, s_all) = (sender.clone(), sender.clone());
        let header = PanelHeader::new(
            "package-x-generic-symbolic",
            MenuWidget::FlatpakBackground.display_name(),
        )
        .with_subtitle()
        .action("view-refresh-symbolic", "Scan again", move || {
            s_refresh.input(FlatpakBackgroundMenuWidgetInput::Refresh);
        })
        .action("process-stop-symbolic", "Stop all of them", move || {
            s_all.input(FlatpakBackgroundMenuWidgetInput::StopAll);
        });
        let header_root = header.root().clone();
        let subtitle = header.subtitle().cloned().unwrap_or_default();
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);

        let open = RevealGate::follow_mapped(&root);
        sender.command(|out, shutdown| async move {
            let watch = open.stream_while_revealed(
                || watch_stream(flatpak_bg::subscribe()),
                |state| {
                    let _ = out.send(FlatpakBackgroundMenuWidgetCommandOutput::State(state));
                },
            );
            tokio::select! {
                () = shutdown.wait() => {}
                () = watch => {}
            }
        });

        let model = FlatpakBackgroundMenuWidgetModel {
            state: FlatpakBgState::default(),
            list: list.clone(),
            subtitle,
        };
        let widgets = view_output!();
        crate::menus::list_fit::mark_menu_list(&widgets.fill_list, 420);
        model.render(&widgets, &sender);
        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {
        match msg {
            FlatpakBackgroundMenuWidgetInput::Stop(id) => flatpak_bg::stop(id),
            FlatpakBackgroundMenuWidgetInput::StopAll => {
                for a in &self.state.apps {
                    flatpak_bg::stop(a.id.clone());
                }
            }
            FlatpakBackgroundMenuWidgetInput::Ignore(id) => {
                config_manager().update_config(move |c| {
                    if !c.flatpak_bg.ignored_apps.contains(&id) {
                        c.flatpak_bg.ignored_apps.push(id.clone());
                    }
                });
                flatpak_bg::refresh();
            }
            FlatpakBackgroundMenuWidgetInput::Refresh => flatpak_bg::refresh(),
        }
    }

    fn update_cmd_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        msg: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match msg {
            FlatpakBackgroundMenuWidgetCommandOutput::State(s) => {
                self.state = s;
                self.render(widgets, &sender);
            }
        }
    }
}

impl FlatpakBackgroundMenuWidgetModel {
    fn render(
        &self,
        widgets: &FlatpakBackgroundMenuWidgetModelWidgets,
        sender: &ComponentSender<Self>,
    ) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let s = &self.state;
        for a in &s.apps {
            self.list.append(&row(a, sender));
        }
        self.list.set_visible(!s.apps.is_empty());
        self.subtitle.set_label(&match s.apps.len() {
            0 => "None in the background".to_string(),
            1 => "1 app without a window".to_string(),
            n => format!("{n} apps without a window"),
        });
        widgets.note.set_label(&if !s.available {
            "flatpak is not installed (or not on PATH).".to_string()
        } else if let Some(e) = &s.error {
            format!("flatpak ps failed: {e}")
        } else if s.apps.is_empty() {
            "Every running Flatpak app has a window.".to_string()
        } else {
            "Running, but with no window on any tag. Stop ends the app's sandbox (flatpak kill)."
                .to_string()
        });
    }
}

fn row(a: &BgApp, sender: &ComponentSender<FlatpakBackgroundMenuWidgetModel>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.add_css_class("flatpak-bg-row");
    let icon = gio_unix::DesktopAppInfo::new(&format!("{}.desktop", a.id))
        .and_then(|i| i.icon())
        .map_or_else(
            || gtk::Image::from_icon_name("package-x-generic-symbolic"),
            |g| gtk::Image::from_gicon(&g),
        );
    icon.set_pixel_size(24);
    row.append(&icon);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_hexpand(true);
    let name = gtk::Label::new(Some(&a.name));
    name.add_css_class("label-medium");
    name.set_xalign(0.0);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text.append(&name);
    let id = gtk::Label::new(Some(&a.id));
    id.set_css_classes(&["label-small", "dim-label"]);
    id.set_xalign(0.0);
    id.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    text.append(&id);
    row.append(&text);
    for (icon, tip, stop) in [
        ("view-conceal-symbolic", "Ignore (never list it)", false),
        ("process-stop-symbolic", "Stop (flatpak kill)", true),
    ] {
        let b = gtk::Button::from_icon_name(icon);
        mshell_common::a11y::name(&b, tip);
        b.add_css_class("ok-button-flat");
        b.set_valign(gtk::Align::Center);
        b.set_tooltip_text(Some(tip));
        let (s, app) = (sender.clone(), a.id.clone());
        b.connect_clicked(move |_| {
            s.input(if stop {
                FlatpakBackgroundMenuWidgetInput::Stop(app.clone())
            } else {
                FlatpakBackgroundMenuWidgetInput::Ignore(app.clone())
            });
        });
        row.append(&b);
    }
    row
}
