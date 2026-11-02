use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, ThemeStoreFields};
use mshell_config::schema::themes::Themes;
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::prelude::{BoxExt, GestureExt, OrientableExt, WidgetExt};
use relm4::prelude::FactoryComponent;
use relm4::{FactorySender, gtk};

#[derive(Debug)]
pub(crate) struct ThemeCardModel {
    pub(crate) theme: Themes,
    pub(crate) is_selected: bool,
    pub(crate) colors: Option<[String; 7]>,
    // Effect scope kept alive for the lifetime of this card so the
    // `config.theme().theme()` watcher we install in `init_model`
    // keeps firing — without holding the scope, the effect drops
    // immediately and `is_selected` never refreshes when the active
    // theme changes (which is the whole point of the check-icon
    // indicator moving as the user clicks around).
    _effects: EffectScope,
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
    type ParentWidget = gtk::FlowBox;

    view! {
        #[root]
        gtk::Overlay {
            add_css_class: "theme-card",

            #[name = "bg"]
            gtk::DrawingArea {
                set_hexpand: true,
                set_height_request: 70,
                set_can_target: false,
            },

            add_overlay = &gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_halign: gtk::Align::Fill,
                set_valign: gtk::Align::Center,
                set_spacing: 4,

                gtk::Label {
                    add_css_class: "label-medium-bold",
                    set_halign: gtk::Align::Center,
                    #[watch]
                    set_markup: &if let Some(colors) = &self.colors {
                        format!("<span foreground=\"{}\">{}</span>", &colors[1], self.theme.label())
                    } else {
                        self.theme.label().to_string()
                    },
                },

                gtk::Label {
                    set_visible: self.theme == Themes::Default,
                    add_css_class: "label-small",
                    set_halign: gtk::Align::Center,
                    set_label: "Default colors with no extra style sheets.",
                    set_wrap: true,
                    set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                    set_margin_start: 40,
                    set_margin_end: 40,
                },

                gtk::Label {
                    set_visible: self.theme == Themes::Wallpaper,
                    add_css_class: "label-small",
                    set_halign: gtk::Align::Center,
                    set_label: "Generate colors based on the selected wallpaper.",
                    set_wrap: true,
                    set_natural_wrap_mode: gtk::NaturalWrapMode::None,
                    set_margin_start: 40,
                    set_margin_end: 40,
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
                    #[name = "swatch4"] gtk::DrawingArea { set_size_request: (16, 16), },
                    #[name = "swatch5"] gtk::DrawingArea { set_size_request: (16, 16), },
                    #[name = "swatch6"] gtk::DrawingArea { set_size_request: (16, 16), },
                },
            },

            add_overlay = &gtk::Box {
                gtk::Image {
                    add_css_class: "image-surface",
                    set_icon_name: Some("check-circle-symbolic"),
                    set_halign: gtk::Align::Start,
                    set_pixel_size: 20,
                    set_margin_start: 12,
                    #[watch]
                    set_visible: self.is_selected,
                },
            },

            add_controller = gtk::GestureClick::builder()
                .button(1)
                .propagation_phase(gtk::PropagationPhase::Capture)
                .build() {
                connect_released[sender] => move |gesture, _, _, _| {
                    gesture.set_state(gtk::EventSequenceState::Claimed);
                    tracing::debug!("theme_card: click released");
                    sender.input(ThemeCardInput::Clicked);
                },
            },
        }
    }

    fn init_model(
        theme: Self::Init,
        _index: &relm4::prelude::DynamicIndex,
        sender: FactorySender<Self>,
    ) -> Self {
        let active_theme = config_manager().config().theme().theme().get_untracked();
        let colors = crate::theme_swatch::theme_swatch_colors(&theme);

        // Each card subscribes directly to the active-theme field so
        // the check-icon indicator on every card refreshes regardless
        // of whether relm4's FactoryVecDeque-forward path is alive
        // (see the comment in `update_with_view`).
        let mut effects = EffectScope::new();
        let sender_clone = sender.clone();
        effects.push(move |_| {
            let active = config_manager().config().theme().theme().get();
            sender_clone.input(ThemeCardInput::SelectionChanged(active));
        });

        Self {
            is_selected: theme == active_theme,
            theme,
            colors,
            _effects: effects,
        }
    }

    fn init_widgets(
        &mut self,
        _index: &relm4::prelude::DynamicIndex,
        root: Self::Root,
        returned_widget: &gtk::FlowBoxChild,
        sender: FactorySender<Self>,
    ) -> Self::Widgets {
        let widgets = view_output!();

        self.set_draw_funcs(&widgets);

        returned_widget.set_focusable(false);
        returned_widget.set_can_focus(false);
        returned_widget.set_focus_on_click(false);

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
                tracing::debug!(theme = ?self.theme, "theme_card: Clicked → update_config");
                // We can't rely on relm4's `FactoryVecDeque::forward`
                // path here — its forward task runs on relm4's
                // single-worker private RUNTIME and (with the number
                // of long-lived `watch!` / `sender.command` tasks the
                // shell has already spawned on that same worker) the
                // factory's output_receiver gets starved indefinitely.
                // The click logs reach `Clicked`, but the parent's
                // `update_with_view(ThemeSelected)` never fires.
                // Bypass by writing the new theme directly into the
                // config store; reactive Effects in StyleManager and
                // sibling cards (see `SelectionChanged`) will catch up.
                let theme = self.theme;
                config_manager().update_config(move |config| {
                    config.theme.theme = theme;
                });
                let _ = sender.output(ThemeCardOutput::Selected(self.theme));
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
            crate::theme_swatch::install_theme_swatches(
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
