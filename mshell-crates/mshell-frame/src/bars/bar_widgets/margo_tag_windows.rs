//! App icons of the active tag's windows, shown after the tag pills when
//! `bars.widgets.margo_tags.window_icons` is on (port of the
//! niri-workspace-stack plugin's window icons onto margo tags).
//!
//! One icon button per visible window on the focused output's active tag,
//! in margo's client order; the keyboard-focused one carries a quiet
//! selection tint (the active tag pill keeps the accent). Click focuses
//! that window (`focuswindowid`). Source:
//! `margo_service().active_tag_windows`, change-only, so a title edit on
//! another tag does not rebuild the icons.

use futures::StreamExt;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    BarWidgetsStoreFields, BarsStoreFields, ConfigStoreFields, IconsStoreFields,
    MargoTagsConfigStoreFields, ThemeStoreFields,
};
use mshell_margo_client::indicators::TagWindow;
use mshell_services::margo_service;
use mshell_utils::app_icon::app_icon::set_icon;
use mshell_utils::app_info::find_app_info;
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::Orientation;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};

pub(crate) struct MargoTagWindowsModel {
    windows: Vec<TagWindow>,
    enabled: bool,
    orientation: Orientation,
    _effects: mshell_common::scoped_effects::EffectScope,
}

#[derive(Debug)]
pub(crate) enum MargoTagWindowsInput {
    /// `bars.widgets.margo_tags.window_icons` changed.
    Enabled(bool),
}

#[derive(Debug)]
pub(crate) enum MargoTagWindowsOutput {}

pub(crate) struct MargoTagWindowsInit {
    pub(crate) orientation: Orientation,
}

#[derive(Debug)]
pub(crate) enum MargoTagWindowsCommandOutput {
    Windows(Vec<TagWindow>),
}

#[relm4::component(pub)]
impl Component for MargoTagWindowsModel {
    type CommandOutput = MargoTagWindowsCommandOutput;
    type Input = MargoTagWindowsInput;
    type Output = MargoTagWindowsOutput;
    type Init = MargoTagWindowsInit;

    view! {
        #[root]
        #[name = "root"]
        gtk::Box {
            add_css_class: "margo-tag-windows",
            set_orientation: model.orientation,
            set_halign: gtk::Align::Center,
            set_valign: gtk::Align::Center,
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        sender.command(|out, shutdown| async move {
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            let mut stream = margo_service().active_tag_windows.watch();
            loop {
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    next = stream.next() => match next {
                        Some(w) => {
                            let _ = out.send(MargoTagWindowsCommandOutput::Windows(w));
                        }
                        None => break,
                    },
                }
            }
        });

        // The toggle is live: flipping it in Settings shows / hides the
        // icons without rebuilding the bar.
        let mut effects = mshell_common::scoped_effects::EffectScope::new();
        {
            let s = sender.clone();
            effects.push(move |_| {
                let on = config_manager()
                    .config()
                    .bars()
                    .widgets()
                    .margo_tags()
                    .window_icons()
                    .get();
                s.input(MargoTagWindowsInput::Enabled(on));
            });
        }

        let model = MargoTagWindowsModel {
            windows: margo_service().active_tag_windows.get(),
            enabled: config_manager()
                .config()
                .bars()
                .widgets()
                .margo_tags()
                .window_icons()
                .get_untracked(),
            orientation: params.orientation,
            _effects: effects,
        };
        let widgets = view_output!();
        render(&widgets.root, &model);
        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::Input,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            MargoTagWindowsInput::Enabled(on) => {
                if on != self.enabled {
                    self.enabled = on;
                    render(&widgets.root, self);
                }
            }
        }
    }

    fn update_cmd_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::CommandOutput,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            MargoTagWindowsCommandOutput::Windows(w) => {
                if w != self.windows {
                    self.windows = w;
                    if self.enabled {
                        render(&widgets.root, self);
                    }
                }
            }
        }
    }
}

/// Rebuild the icon row (a tag holds a handful of windows).
fn render(root: &gtk::Box, model: &MargoTagWindowsModel) {
    while let Some(child) = root.first_child() {
        root.remove(&child);
    }
    let show = model.enabled && !model.windows.is_empty();
    root.set_visible(show);
    if !show {
        return;
    }
    // Icon look follows the dock's theme / recolour settings.
    let icons = || config_manager().config().theme().icons();
    let icon_theme = icons().app_icon_theme().get_untracked();
    let color_theme = config_manager().config().theme().theme().get_untracked();
    let apply_filter = icons().apply_theme_filter().get_untracked();
    let filter_strength = icons().filter_strength().get_untracked().get();
    let monochrome_strength = icons().monochrome_strength().get_untracked().get();
    let contrast_strength = icons().contrast_strength().get_untracked().get();
    for w in &model.windows {
        let image = gtk::Image::new();
        image.add_css_class("margo-tag-window-icon");
        set_icon(
            &find_app_info(&w.app_id),
            &Some(w.app_id.clone()),
            &image,
            icon_theme.clone(),
            &color_theme,
            apply_filter,
            filter_strength,
            monochrome_strength,
            contrast_strength,
        );
        let mut classes = vec!["margo-tag", "margo-tag-window"];
        if w.focused {
            classes.push("focused");
        }
        let button = gtk::Button::new();
        button.set_css_classes(&classes);
        button.set_child(Some(&image));
        let title = if w.title.trim().is_empty() {
            w.app_id.as_str()
        } else {
            w.title.trim()
        };
        button.set_tooltip_text(Some(&format!("{}  ·  {title}\n\nClick: focus", w.app_id)));
        mshell_common::a11y::name(&button, &format!("Focus {title}"));
        let id = w.id.to_string();
        button.connect_clicked(move |_| {
            mshell_margo_client::ctl::dispatch_detached("focuswindowid", &[&id]);
        });
        root.append(&button);
    }
}
