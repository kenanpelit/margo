use crate::bars::bar::BarType;
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use relm4::gtk::Orientation;
use relm4::gtk::gdk::Monitor;
use relm4::gtk::prelude::{GtkWindowExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

#[derive(Debug, Clone)]
pub(crate) struct FrameSpacerModel {
    orientation: Orientation,
    height: i32,
    border_height: i32,
}

#[derive(Debug)]
pub(crate) enum FrameSpacerInput {
    HeightUpdated(i32),
    BorderHeightUpdated(i32),
}

#[derive(Debug)]
pub(crate) enum FrameSpacerOutput {}

pub(crate) struct FrameSpacerInit {
    pub bar_type: BarType,
    pub monitor: Monitor,
}

#[relm4::component(pub)]
impl Component for FrameSpacerModel {
    type CommandOutput = ();
    type Input = FrameSpacerInput;
    type Output = FrameSpacerOutput;
    type Init = FrameSpacerInit;

    view! {
        #[root]
        gtk::Window {
            add_css_class: "frame-spacer-window",
            set_default_height: if model.orientation == Orientation::Horizontal {
                1
            } else {
                0
            },
            set_default_width: if model.orientation == Orientation::Vertical {
                1
            } else {
                0
            },
            set_can_target: false,
            set_can_focus: false,

            #[name = "spacer"]
            gtk::Box {
                #[watch]
                set_height_request: model.height + model.border_height,
            }
        },
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        _sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        root.init_layer_shell();
        root.set_monitor(Some(&params.monitor));
        root.set_layer(Layer::Background);
        root.auto_exclusive_zone_enable();
        root.set_decorated(false);
        root.set_visible(true);
        root.set_namespace(Some("mshell-spacer"));

        match params.bar_type {
            BarType::Top => {
                root.set_anchor(Edge::Top, true);
                root.set_anchor(Edge::Left, true);
                root.set_anchor(Edge::Right, true);
            }
            BarType::Bottom => {
                root.set_anchor(Edge::Bottom, true);
                root.set_anchor(Edge::Left, true);
                root.set_anchor(Edge::Right, true);
            }
        }

        let orientation = match params.bar_type {
            // Only horizontal bars exist (vertical Left / Right were
            // removed).
            BarType::Top | BarType::Bottom => Orientation::Horizontal,
        };
        let model = FrameSpacerModel {
            orientation,
            height: 0,
            border_height: 0,
        };

        let widgets = view_output!();

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
            FrameSpacerInput::HeightUpdated(val) => {
                self.height = val;
            }
            FrameSpacerInput::BorderHeightUpdated(height) => {
                self.border_height = height;
            }
        }
        self.update_view(widgets, sender);
    }
}
