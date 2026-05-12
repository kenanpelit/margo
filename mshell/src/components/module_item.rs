use crate::{components::position_button, theme::use_theme};
use iced::{
    Alignment, Element, Length,
    widget::{Space, Stack, container},
};

use super::ButtonUIRef;

/// Builder for a bar module item: content wrapped in a position_button
/// with optional press, right-press, scroll-up, and scroll-down handlers.
///
/// When no press handler is set, renders as a plain container.
pub struct ModuleItem<'a, Msg> {
    content: Element<'a, Msg>,
    on_press: Option<Msg>,
    on_press_with_position: Option<Box<dyn Fn(ButtonUIRef) -> Msg + 'a>>,
    on_right_press: Option<Msg>,
    on_scroll_up: Option<Msg>,
    on_scroll_down: Option<Msg>,
    is_active: bool,
}

pub fn module_item<'a, Msg: 'static + Clone>(content: Element<'a, Msg>) -> ModuleItem<'a, Msg> {
    ModuleItem {
        content,
        on_press: None,
        on_press_with_position: None,
        on_right_press: None,
        on_scroll_up: None,
        on_scroll_down: None,
        is_active: false,
    }
}

impl<'a, Msg: 'static + Clone> ModuleItem<'a, Msg> {
    pub fn on_press(mut self, msg: Msg) -> Self {
        self.on_press = Some(msg);
        self
    }

    pub fn on_press_with_position(mut self, handler: impl Fn(ButtonUIRef) -> Msg + 'a) -> Self {
        self.on_press_with_position = Some(Box::new(handler));
        self
    }

    pub fn on_right_press(mut self, msg: Msg) -> Self {
        self.on_right_press = Some(msg);
        self
    }

    pub fn on_scroll_up(mut self, msg: Msg) -> Self {
        self.on_scroll_up = Some(msg);
        self
    }

    pub fn on_scroll_down(mut self, msg: Msg) -> Self {
        self.on_scroll_down = Some(msg);
        self
    }

    /// Mark this module as "active" — its menu (or owned overlay) is
    /// currently on screen. Renders a 2px accent bar along the
    /// capsule's bottom edge to signal the relationship.
    pub fn is_active(mut self, value: bool) -> Self {
        self.is_active = value;
        self
    }
}

impl<'a, Msg: 'static + Clone> From<ModuleItem<'a, Msg>> for Element<'a, Msg> {
    fn from(item: ModuleItem<'a, Msg>) -> Self {
        let (space, module_button_style, active_indicator_style) = use_theme(|theme| {
            (
                theme.space,
                theme.module_button_style(),
                theme.module_active_indicator_style(),
            )
        });

        let is_active = item.is_active;
        let has_action = item.on_press.is_some() || item.on_press_with_position.is_some();

        let core: Element<'a, Msg> = if has_action {
            let mut button = position_button(
                container(item.content)
                    .align_y(Alignment::Center)
                    .height(Length::Fill)
                    .clip(true),
            )
            .padding([2.0, space.xs])
            .height(Length::Fill)
            .style(module_button_style);

            if let Some(handler) = item.on_press_with_position {
                button = button.on_press_with_position(handler);
            } else if let Some(msg) = item.on_press {
                button = button.on_press(msg);
            }

            if let Some(msg) = item.on_right_press {
                button = button.on_right_press(msg);
            }
            if let Some(msg) = item.on_scroll_up {
                button = button.on_scroll_up(msg);
            }
            if let Some(msg) = item.on_scroll_down {
                button = button.on_scroll_down(msg);
            }

            button.into()
        } else {
            container(item.content)
                .padding([2.0, space.xs])
                .height(Length::Fill)
                .align_y(Alignment::Center)
                .clip(true)
                .into()
        };

        if is_active {
            // Centered 60%-of-capsule indicator hugging the bottom
            // edge. Implemented as a 3-cell Row (2/6/2 flex portions)
            // so the accent stripe scales with capsule width without
            // needing to know it explicitly. Stack overlay keeps bar
            // height unchanged when this toggles on/off.
            let stripe = container(
                Space::new()
                    .width(Length::Fill)
                    .height(Length::Fixed(2.0)),
            )
            .style(active_indicator_style);

            let stripe_row = iced::widget::Row::new()
                .push(Space::new().width(Length::FillPortion(2)))
                .push(
                    container(stripe)
                        .width(Length::FillPortion(6))
                        .height(Length::Fixed(2.0)),
                )
                .push(Space::new().width(Length::FillPortion(2)))
                .height(Length::Fixed(2.0));

            let indicator_layer = container(stripe_row)
                .align_y(Alignment::End)
                .width(Length::Fill)
                .height(Length::Fill);

            Stack::new().push(core).push(indicator_layer).into()
        } else {
            core
        }
    }
}
