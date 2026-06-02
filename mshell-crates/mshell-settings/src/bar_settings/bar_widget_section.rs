use crate::bar_settings::bar_widget_factory::{
    ActiveWidgetInit, ActiveWidgetModel, BarListLocation,
};
use mshell_config::config_manager::config_manager;
use mshell_config::schema::bar_widgets::BarWidget;
use reactive_graph::prelude::ReadUntracked;
use relm4::factory::FactoryVecDeque;
use relm4::gtk::prelude::{BoxExt, ButtonExt, OrientableExt, PopoverExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BarSection {
    Start,
    Center,
    End,
    Hidden,
}

impl BarSection {
    pub fn display_name(&self) -> &'static str {
        match self {
            BarSection::Start => "Start",
            BarSection::Center => "Center",
            BarSection::End => "End",
            BarSection::Hidden => "Hidden widgets",
        }
    }
}

#[derive(Debug)]
pub struct WidgetSectionModel {
    section: BarSection,
    location: BarListLocation,
    widgets: FactoryVecDeque<ActiveWidgetModel>,
}

#[derive(Debug)]
pub enum WidgetSectionInput {
    /// Replay the section's widget list into the factory. Driven
    /// from `bar_settings.rs`'s reactive effects — the add /
    /// reorder / remove controls all write the config directly,
    /// so this is the only message the section needs.
    SetWidgetsEffect(Vec<BarWidget>),
}

#[derive(Debug)]
pub enum WidgetSectionOutput {}

pub struct WidgetSectionInit {
    pub bar_section: BarSection,
    pub location: BarListLocation,
    pub widgets: Vec<BarWidget>,
}

#[relm4::component(pub)]
impl Component for WidgetSectionModel {
    type CommandOutput = ();
    type Input = WidgetSectionInput;
    type Output = WidgetSectionOutput;
    type Init = WidgetSectionInit;

    view! {
        #[root]
        gtk::Box {
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 8,
            add_css_class: "settings-bar-widget-section",

            gtk::Label {
                add_css_class: "label-medium-bold",
                set_halign: gtk::Align::Start,
                #[watch]
                set_label: model.section.display_name(),
            },

            #[local_ref]
            widget_list -> gtk::ListBox {
                set_selection_mode: gtk::SelectionMode::None,
                add_css_class: "settings-bar-widget-section-list",
            },

            #[name = "add_widget_button"]
            gtk::MenuButton {
                set_label: "Add widget",
                set_halign: gtk::Align::Start,
                set_always_show_arrow: false,
            },
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        _sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let location = params.location;
        // The factory children mutate the config directly, so their
        // (empty) output is just detached.
        let mut widgets = FactoryVecDeque::builder()
            .launch(gtk::ListBox::default())
            .detach();

        params.widgets.iter().for_each(|widget| {
            widgets.guard().push_back(ActiveWidgetInit {
                widget: widget.clone(),
                location,
            });
        });

        let model = WidgetSectionModel {
            section: params.bar_section,
            location,
            widgets,
        };

        let widget_list = model.widgets.widget();

        // Drop target for drag-to-reorder lives on the ListBox (rows only
        // carry the drag source). Drops rewrite this section's config list
        // directly, like the up/down buttons.
        crate::reorder_dnd::attach_listbox_drop_target(widget_list, move |from, to| {
            crate::bar_settings::bar_widget_factory::move_item(location, from, to);
        });

        let widgets_view = view_output!();

        // Build the add-widget menu
        Self::build_add_menu(&widgets_view.add_widget_button, location);

        ComponentParts {
            model,
            widgets: widgets_view,
        }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            WidgetSectionInput::SetWidgetsEffect(new_widgets) => {
                let location = self.location;
                let mut guard = self.widgets.guard();
                guard.clear();
                for widget in new_widgets {
                    guard.push_back(ActiveWidgetInit { widget, location });
                }
            }
        }

        self.update_view(widgets, sender);
    }
}

impl WidgetSectionModel {
    /// Build the "Add widget" popover.
    ///
    /// We used to feed a `gio::Menu` to `MenuButton::set_menu_model`
    /// but GTK's native menu rendering doesn't scroll — with the
    /// catalogue now > 35 entries, the menu ran off the bottom of
    /// the panel and lower items were unreachable. A hand-rolled
    /// `gtk::Popover` with a height-capped `ScrolledWindow` inside
    /// gives us both the lookup-style UX and a scrollbar when
    /// needed.
    fn build_add_menu(button: &gtk::MenuButton, location: BarListLocation) {
        let popover = gtk::Popover::new();
        popover.add_css_class("settings-bar-widget-add-popover");

        let scrolled = gtk::ScrolledWindow::new();
        scrolled.set_vscrollbar_policy(gtk::PolicyType::Automatic);
        scrolled.set_hscrollbar_policy(gtk::PolicyType::Never);
        // 360 px keeps roughly 10 entries visible at once on the
        // panel's default font scale — past that we scroll. The
        // popover sizes to content if the list is shorter.
        scrolled.set_max_content_height(360);
        scrolled.set_propagate_natural_height(true);
        scrolled.set_propagate_natural_width(true);

        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        list.add_css_class("settings-bar-widget-add-list");

        scrolled.set_child(Some(&list));
        popover.set_child(Some(&scrolled));
        button.set_popover(Some(&popover));

        // Repopulate on every show — plugin custom widgets are derived from
        // the live config, so an Install in Settings → Plugins (which mutates
        // bars.widgets.custom_widgets) shows up here on the next click of the
        // "Add widget" button, without a shell restart.
        populate_add_list(&list, &popover, location);
        {
            let list = list.clone();
            let pop = popover.clone();
            popover.connect_show(move |_| {
                populate_add_list(&list, &pop, location);
            });
        }
    }
}

fn populate_add_list(list: &gtk::Box, popover: &gtk::Popover, location: BarListLocation) {
    // Clear whatever the previous show built.
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }

    // Static catalogue, alphabetised.
    let mut widgets: Vec<BarWidget> = BarWidget::all().to_vec();
    widgets.sort_by_key(|w| w.display_name().to_ascii_lowercase());

    for widget in &widgets {
        let btn = gtk::Button::with_label(widget.display_name());
        btn.set_css_classes(&["settings-bar-widget-add-item"]);
        btn.set_halign(gtk::Align::Fill);
        btn.set_has_frame(false);

        let widget_clone = widget.clone();
        let popover_clone = popover.clone();
        btn.connect_clicked(move |_| {
            let widget_clone = widget_clone.clone();
            config_manager().update_config(move |config| {
                let list = match location {
                    BarListLocation::TopStart => &mut config.bars.top_bar.left_widgets,
                    BarListLocation::TopCenter => &mut config.bars.top_bar.center_widgets,
                    BarListLocation::TopEnd => &mut config.bars.top_bar.right_widgets,
                    BarListLocation::BottomStart => &mut config.bars.bottom_bar.left_widgets,
                    BarListLocation::BottomCenter => &mut config.bars.bottom_bar.center_widgets,
                    BarListLocation::BottomEnd => &mut config.bars.bottom_bar.right_widgets,
                    BarListLocation::TopHidden => &mut config.bars.top_bar.hidden_widgets,
                    BarListLocation::BottomHidden => &mut config.bars.bottom_bar.hidden_widgets,
                };
                list.push(widget_clone);
            });
            popover_clone.popdown();
        });

        list.append(&btn);
    }

    // User-defined + plugin custom widgets — read fresh on every show.
    let cfg = config_manager().config().read_untracked().clone();
    for cw in &cfg.bars.widgets.custom_widgets {
        if cw.name.trim().is_empty() {
            continue;
        }
        let btn = gtk::Button::with_label(&custom_widget_label(&cw.name));
        btn.set_css_classes(&["settings-bar-widget-add-item"]);
        btn.set_halign(gtk::Align::Fill);
        btn.set_has_frame(false);

        let name = cw.name.clone();
        let popover_clone = popover.clone();
        btn.connect_clicked(move |_| {
            let name = name.clone();
            config_manager().update_config(move |config| {
                let list = match location {
                    BarListLocation::TopStart => &mut config.bars.top_bar.left_widgets,
                    BarListLocation::TopCenter => &mut config.bars.top_bar.center_widgets,
                    BarListLocation::TopEnd => &mut config.bars.top_bar.right_widgets,
                    BarListLocation::BottomStart => &mut config.bars.bottom_bar.left_widgets,
                    BarListLocation::BottomCenter => &mut config.bars.bottom_bar.center_widgets,
                    BarListLocation::BottomEnd => &mut config.bars.bottom_bar.right_widgets,
                    BarListLocation::TopHidden => &mut config.bars.top_bar.hidden_widgets,
                    BarListLocation::BottomHidden => &mut config.bars.bottom_bar.hidden_widgets,
                };
                list.push(BarWidget::Custom(name.clone()));
            });
            popover_clone.popdown();
        });

        list.append(&btn);
    }
}

/// Display label for a custom widget in the add menu. Plugin widgets are
/// named `plugin:<key>:<widget>` (the key may itself contain `:` for
/// custom-source plugins); show them as "<plugin name> · <widget>" using the
/// installed manifest's name, falling back to the key. Non-plugin custom
/// widgets keep their raw name.
pub(crate) fn custom_widget_label(name: &str) -> String {
    if let Some(rest) = name.strip_prefix("plugin:")
        && let Some((key, widget)) = rest.rsplit_once(':')
    {
        let pretty = mshell_plugins::PluginStore::new()
            .installed()
            .into_iter()
            .find(|p| p.key == key)
            .map(|p| p.manifest.name)
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| key.to_string());
        return format!("{pretty} · {widget}");
    }
    name.to_string()
}
