//! The **Controls** tab: the volume / mic / brightness sliders in one row,
//! then every quick setting as a labelled tile — the Control Center's own
//! slider and tile components (same model, probes and pollers, so a toggle
//! here and in the Control Center always agree) in the dashboard layout.

use super::card::GAP;
use crate::frame::MenuId;
use crate::menus::menu_widgets::control_center::sliders::{
    ControlCenterSlidersInit, ControlCenterSlidersModel, ControlCenterSlidersOutput,
};
use crate::menus::menu_widgets::control_center::tiles::{
    ControlCenterTilesInit, ControlCenterTilesInput, ControlCenterTilesModel,
    ControlCenterTilesOutput, TilesLayout,
};
use mshell_config::schema::dashboard::{DashboardCard, DashboardTabConfig};
use relm4::gtk;
use relm4::gtk::prelude::{BoxExt, WidgetExt};
use relm4::{Component, ComponentController, Controller};

pub(crate) struct ControlsTab {
    pub(crate) root: gtk::Box,
    /// Owned so the sliders live as long as the tab (the render fixture
    /// also drives it).
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "kept alive; read by the render fixture")
    )]
    pub(crate) sliders: Controller<ControlCenterSlidersModel>,
    pub(crate) tiles: Controller<ControlCenterTilesModel>,
}

impl ControlsTab {
    pub(crate) fn new(
        cfg: &DashboardTabConfig,
        live: bool,
        open: impl Fn(MenuId) + Clone + Send + 'static,
    ) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, GAP);
        root.add_css_class("mdash-controls");

        let o = open.clone();
        let sliders = ControlCenterSlidersModel::builder()
            .launch(ControlCenterSlidersInit { dashboard: true })
            .connect_receiver(move |_, msg| match msg {
                ControlCenterSlidersOutput::OpenAudioOut | ControlCenterSlidersOutput::OpenMic => {
                    o(MenuId::AudioDashboard)
                }
            });
        let sliders_card = gtk::Box::new(gtk::Orientation::Vertical, 0);
        sliders_card.add_css_class("mdash-card");
        sliders_card.add_css_class("mdash-sliders-card");
        sliders_card.append(sliders.widget());
        sliders_card.set_visible(cfg.shows(DashboardCard::Sliders));
        root.append(&sliders_card);

        let tiles = ControlCenterTilesModel::builder()
            .launch(ControlCenterTilesInit {
                layout: TilesLayout::Dashboard,
                live,
            })
            .connect_receiver(move |_, msg| match msg {
                ControlCenterTilesOutput::OpenMenu(id) => open(id),
                // The dashboard layout never emits detail pages.
                ControlCenterTilesOutput::ExpandPage(_) => {}
            });
        tiles.widget().set_visible(cfg.shows(DashboardCard::Tiles));
        root.append(tiles.widget());

        Self {
            root,
            sliders,
            tiles,
        }
    }

    /// Start / stop the tiles' watchers with the tab's visibility.
    pub(crate) fn set_visible(&self, shown: bool) {
        self.tiles.emit(ControlCenterTilesInput::Reveal(shown));
    }
}
