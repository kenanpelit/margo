use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, ThemeStoreFields};
use mshell_config::schema::themes::Themes;
use reactive_graph::prelude::GetUntracked;
use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::prelude::FactoryComponent;
use relm4::{FactorySender, gtk};

#[derive(Debug)]
pub(crate) struct ThemeCardModel {
    pub(crate) theme: Themes,
    pub(crate) is_selected: bool,
    pub(crate) colors: Option<[String; 7]>,
}

#[derive(Debug)]
pub(crate) enum ThemeCardInput {
    Clicked,
    SelectionChanged(Themes),
}

#[derive(Debug)]
pub(crate) enum ThemeCardOutput {
    Selected(Themes),
}

#[relm4::factory(pub(crate))]
impl FactoryComponent for ThemeCardModel {
    type Init = Themes;
    type Input = ThemeCardInput;
    type Output = ThemeCardOutput;
    type CommandOutput = ();
    type ParentWidget = gtk::Box;

    view! {
        #[root]
        gtk::Overlay {
            set_width_request: 100,
            add_css_class: "theme-card",

            #[name = "bg"]
            gtk::DrawingArea {
                set_hexpand: true,
                set_height_request: 120,
                set_can_target: false,
            },

            add_overlay = &gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_vexpand: false,
                set_halign: gtk::Align::Fill,
                set_valign: gtk::Align::Fill,
                set_spacing: 4,

                gtk::Label {
                    add_css_class: "label-small",
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Start,
                    set_vexpand: true,
                    set_margin_top: 4,
                    set_margin_start: 4,
                    set_margin_end: 4,
                    set_wrap: true,
                    set_justify: gtk::Justification::Center,
                    set_xalign: 0.5,
                    #[watch]
                    set_markup: &if let Some(colors) = &self.colors {
                        format!("<span foreground=\"{}\">{}</span>", &colors[1], self.theme.label())
                    } else {
                        self.theme.label().to_string()
                    },
                },

                gtk::Image {
                    add_css_class: "image-surface",
                    set_visible: self.theme == Themes::Default,
                    set_halign: gtk::Align::Center,
                    set_icon_name: Some("palette-symbolic"),
                    set_pixel_size: 60,
                },

                gtk::Image {
                    add_css_class: "image-surface",
                    set_visible: self.theme == Themes::Wallpaper,
                    set_halign: gtk::Align::Center,
                    set_icon_name: Some("wallpaper-symbolic"),
                    set_pixel_size: 60,
                },

                gtk::Box {
                    set_visible: self.theme != Themes::Wallpaper && self.theme != Themes::Default,
                    set_orientation: gtk::Orientation::Horizontal,
                    set_halign: gtk::Align::Center,
                    set_spacing: 8,
                    add_css_class: "theme-swatches",

                    #[name = "swatch1"] gtk::DrawingArea { set_size_request: (16, 16), },
                    #[name = "swatch2"] gtk::DrawingArea { set_size_request: (16, 16), },
                    #[name = "swatch3"] gtk::DrawingArea { set_size_request: (16, 16), },
                },

                gtk::Box {
                    set_visible: self.theme != Themes::Wallpaper && self.theme != Themes::Default,
                    set_orientation: gtk::Orientation::Horizontal,
                    set_halign: gtk::Align::Center,
                    set_spacing: 8,
                    add_css_class: "theme-swatches",

                    #[name = "swatch4"] gtk::DrawingArea { set_size_request: (16, 16), },
                    #[name = "swatch5"] gtk::DrawingArea { set_size_request: (16, 16), },
                    #[name = "swatch6"] gtk::DrawingArea { set_size_request: (16, 16), },
                },

                gtk::Box {
                    set_height_request: 26,
                }
            },

            add_overlay = &gtk::Box {
                gtk::Image {
                    add_css_class: "image-surface",
                    set_icon_name: Some("check-circle-symbolic"),
                    set_halign: gtk::Align::Start,
                    set_valign: gtk::Align::End,
                    set_pixel_size: 20,
                    set_margin_start: 6,
                    set_margin_bottom: 6,
                    #[watch]
                    set_visible: self.is_selected,
                },
            },

            add_controller = gtk::GestureClick::builder().button(1).build() {
                connect_released[sender] => move |_, _, _, _| {
                    sender.input(ThemeCardInput::Clicked);
                },
            },
        }
    }

    fn init_model(
        theme: Self::Init,
        _index: &relm4::prelude::DynamicIndex,
        _sender: FactorySender<Self>,
    ) -> Self {
        let active_theme = config_manager().config().theme().theme().get_untracked();
        let colors = mshell_settings::theme_swatch::theme_swatch_colors(&theme);
        Self {
            is_selected: theme == active_theme,
            theme,
            colors,
        }
    }

    fn init_widgets(
        &mut self,
        _index: &relm4::prelude::DynamicIndex,
        root: Self::Root,
        _returned_widget: &gtk::Widget,
        sender: FactorySender<Self>,
    ) -> Self::Widgets {
        let widgets = view_output!();

        self.set_draw_funcs(&widgets);

        widgets
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::Input,
        sender: FactorySender<Self>,
    ) {
        match message {
            ThemeCardInput::Clicked => {
                sender.output(ThemeCardOutput::Selected(self.theme)).ok();
            }
            ThemeCardInput::SelectionChanged(active_theme) => {
                self.is_selected = self.theme == active_theme;
            }
        }
        self.update_view(widgets, sender);
    }
}

impl ThemeCardModel {
    fn set_draw_funcs(&self, widgets: &ThemeCardModelWidgets) {
        if let Some(colors) = &self.colors {
            mshell_settings::theme_swatch::install_theme_swatches(
                colors,
                &widgets.bg,
                [
                    &widgets.swatch1,
                    &widgets.swatch2,
                    &widgets.swatch3,
                    &widgets.swatch4,
                    &widgets.swatch5,
                    &widgets.swatch6,
                ],
            );
        }
    }
}
