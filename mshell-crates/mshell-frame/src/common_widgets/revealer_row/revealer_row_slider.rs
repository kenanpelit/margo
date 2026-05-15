use relm4::gtk::glib;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};

pub(crate) struct RevealerRowSliderModel {
    value_changed_signal: glib::SignalHandlerId,
}

#[derive(Debug)]
pub(crate) enum RevealerRowSliderInput {
    SetValue(f64),
}

#[derive(Debug)]
pub(crate) enum RevealerRowSliderOutput {
    ValueChanged(f64),
}

pub(crate) struct RevealerRowSliderInit {}

#[derive(Debug)]
pub(crate) enum RevealerRowSliderCommandOutput {}

#[relm4::component(pub)]
impl Component for RevealerRowSliderModel {
    type CommandOutput = RevealerRowSliderCommandOutput;
    type Input = RevealerRowSliderInput;
    type Output = RevealerRowSliderOutput;
    type Init = RevealerRowSliderInit;

    // Scale + numeric percent label side-by-side. The slider
    // shows position; the label spells out the value so the user
    // can read "62%" at a glance instead of eyeballing the
    // thumb position. The label width is fixed (4 chars) so a
    // value change doesn't shift the slider's right edge.
    view! {
        #[root]
        gtk::Box {
            set_orientation: gtk::Orientation::Horizontal,
            set_hexpand: true,
            set_spacing: 8,
            set_valign: gtk::Align::Center,
            set_margin_end: 12,

            #[name = "scale"]
            gtk::Scale {
                add_css_class: "ok-progress-bar",
                set_hexpand: true,
                set_can_focus: false,
                set_focus_on_click: false,
                set_range: (0.0, 1.0),
            },

            #[name = "value_label"]
            gtk::Label {
                add_css_class: "label-small",
                add_css_class: "qs-slider-value",
                set_label: "0%",
                set_width_chars: 4,
                set_xalign: 1.0,
                set_valign: gtk::Align::Center,
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let widgets = view_output!();

        // Live label update as the user drags the slider. Wrap in
        // the captured `value_label` so we don't allocate per
        // change. Output is fired separately by the value-changed
        // signal below.
        let label = widgets.value_label.clone();
        widgets.scale.connect_value_changed(move |scale| {
            label.set_label(&format!("{}%", (scale.value() * 100.0).round() as i32));
        });

        let signal_id = widgets.scale.connect_value_changed(move |scale| {
            let _ = sender.output(RevealerRowSliderOutput::ValueChanged(scale.value()));
        });

        let model = RevealerRowSliderModel {
            value_changed_signal: signal_id,
        };

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
            RevealerRowSliderInput::SetValue(value) => {
                // Block the output-emitting handler so an external
                // update doesn't echo back as a user drag. The
                // separate label-update handler runs in any case.
                widgets.scale.block_signal(&self.value_changed_signal);
                widgets.scale.set_value(value);
                widgets.scale.unblock_signal(&self.value_changed_signal);
                widgets
                    .value_label
                    .set_label(&format!("{}%", (value * 100.0).round() as i32));
            }
        }
    }
}
