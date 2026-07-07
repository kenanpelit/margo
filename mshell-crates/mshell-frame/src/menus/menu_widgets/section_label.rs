//! Dashboard "Section Label" tile — a standalone group heading.
//!
//! A tiny static label used to title a run of tiles or actions
//! inside a panel (the dashboard uses one above the Quick Actions
//! grid so the bottom half reads with the same titled rhythm as the
//! cards above). Purely decorative — it carries only its text.

use mshell_config::schema::menu_widgets::SectionLabelConfig;
use relm4::gtk::prelude::WidgetExt;
use relm4::{ComponentParts, ComponentSender, SimpleComponent, gtk};

#[derive(Debug, Clone)]
pub(crate) struct SectionLabelModel {
    text: String,
}

pub(crate) struct SectionLabelInit {
    pub config: SectionLabelConfig,
}

#[relm4::component(pub)]
impl SimpleComponent for SectionLabelModel {
    type Input = ();
    type Output = ();
    type Init = SectionLabelInit;

    view! {
        #[root]
        gtk::Label {
            add_css_class: "section-label-menu-widget",
            set_label: &model.text,
            set_halign: gtk::Align::Start,
            set_xalign: 0.0,
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        _sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = SectionLabelModel {
            text: params.config.text,
        };

        let widgets = view_output!();

        ComponentParts { model, widgets }
    }
}
