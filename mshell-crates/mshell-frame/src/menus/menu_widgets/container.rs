use crate::menus::builder::build_widget;
use crate::menus::menu::MenuModel;
use mshell_common::dynamic_box::generic_widget_controller::GenericWidgetController;
use mshell_config::schema::menu_widgets::ContainerConfig;
use mshell_config::schema::position::Orientation;
use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::{ComponentParts, ComponentSender, SimpleComponent, gtk};

pub(crate) struct ContainerModel {
    widget_controllers: Vec<Box<dyn GenericWidgetController>>,
    spacing: i32,
    orientation: gtk::Orientation,
    minimum_width: i32,
    homogeneous: bool,
}

#[derive(Debug)]
pub(crate) enum ContainerInput {}

#[derive(Debug)]
pub(crate) enum ContainerOutput {}

pub(crate) struct ContainerInit {
    pub config: ContainerConfig,
    pub menu_sender: ComponentSender<MenuModel>,
}

#[relm4::component(pub)]
impl SimpleComponent for ContainerModel {
    type Input = ContainerInput;
    type Output = ContainerOutput;
    type Init = ContainerInit;

    view! {
        #[root]
        #[name = "widget_container"]
        gtk::Box {
            add_css_class: "container-menu-widget",
            set_orientation: model.orientation,
            set_hexpand: true,
            // vexpand lets a Container claim parent height — used
            // by the dashboard's 2-col body so left + right columns
            // share the same height regardless of which side has
            // more / taller tiles. Children still pile from the
            // top in vertical containers; the trailing space in
            // the shorter column reads as a balanced visual frame
            // instead of leaving the columns mis-aligned.
            set_vexpand: true,
            set_valign: gtk::Align::Fill,
            set_spacing: model.spacing,
            set_width_request: model.minimum_width,
            // When set, every child gets an identical allocation
            // along the orientation axis — the dashboard's 2-col
            // body uses this so left + right panes are exactly the
            // same width regardless of natural content width.
            set_homogeneous: model.homogeneous,
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        _sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let mut widget_controllers: Vec<Box<dyn GenericWidgetController>> = Vec::new();

        for item in params.config.widgets {
            let controller = build_widget(
                &item,
                if params.config.orientation == Orientation::Horizontal {
                    gtk::Orientation::Horizontal
                } else {
                    gtk::Orientation::Vertical
                },
                &params.menu_sender,
            );
            widget_controllers.push(controller);
        }

        let model = ContainerModel {
            widget_controllers,
            spacing: params.config.spacing,
            orientation: if params.config.orientation == Orientation::Horizontal {
                gtk::Orientation::Horizontal
            } else {
                gtk::Orientation::Vertical
            },
            minimum_width: params.config.minimum_width,
            homogeneous: params.config.homogeneous,
        };

        let widgets = view_output!();

        for controller in &model.widget_controllers {
            widgets.widget_container.append(&controller.root_widget());
        }

        ComponentParts { model, widgets }
    }
}
