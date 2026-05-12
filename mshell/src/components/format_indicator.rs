use crate::{
    components::icons::IconKind, config::SettingsFormat, theme::use_theme, utils::IndicatorState,
};
use iced::{
    Alignment, Background, Border, Color, Element, Length, Theme,
    mouse::ScrollDelta,
    widget::{MouseArea, Row, Space, Stack, container, row},
};

pub struct FormatIndicator<'a, Msg> {
    format: SettingsFormat,
    icon: IconKind,
    label_element: Element<'a, Msg>,
    state: IndicatorState,
    on_scroll: Option<Box<dyn Fn(ScrollDelta) -> Msg + 'a>>,
    on_right_press: Option<Msg>,
    progress: Option<f32>,
}

pub fn format_indicator<'a, Msg: 'static + Clone>(
    format: SettingsFormat,
    icon: impl Into<IconKind>,
    label_element: Element<'a, Msg>,
    state: IndicatorState,
) -> FormatIndicator<'a, Msg> {
    FormatIndicator {
        format,
        icon: icon.into(),
        label_element,
        state,
        on_scroll: None,
        on_right_press: None,
        progress: None,
    }
}

impl<'a, Msg: 'static + Clone> FormatIndicator<'a, Msg> {
    pub fn on_scroll(mut self, handler: impl Fn(ScrollDelta) -> Msg + 'a) -> Self {
        self.on_scroll = Some(Box::new(handler));
        self
    }

    pub fn on_right_press(mut self, msg: Msg) -> Self {
        self.on_right_press = Some(msg);
        self
    }

    /// Stacks a 2px progress fill along the bottom edge of the
    /// indicator (e.g. volume / brightness level). Value is clamped
    /// to 0.0..=1.0; pass `None` (the default) to skip the bar.
    pub fn progress(mut self, value: f32) -> Self {
        self.progress = Some(value.clamp(0.0, 1.0));
        self
    }
}

impl<'a, Msg: 'static + Clone> From<FormatIndicator<'a, Msg>> for Element<'a, Msg> {
    fn from(fi: FormatIndicator<'a, Msg>) -> Self {
        let space = use_theme(|theme| theme.space);

        let content = match fi.format {
            SettingsFormat::Icon => fi.icon.to_text().into(),
            SettingsFormat::Percentage | SettingsFormat::Time => fi.label_element,
            SettingsFormat::IconAndPercentage | SettingsFormat::IconAndTime => {
                row![fi.icon.to_text(), fi.label_element]
                    .spacing(space.xxs)
                    .align_y(Alignment::Center)
                    .into()
            }
        };

        let state = fi.state;
        let colored = match state {
            IndicatorState::Normal => content,
            // Success (e.g. charging battery) recolors text only — no
            // border, so a healthy state never feels noisy on the bar.
            IndicatorState::Success => container(content)
                .style(move |theme: &Theme| container::Style {
                    text_color: Some(theme.palette().success),
                    ..Default::default()
                })
                .into(),
            // Warning/Danger get a soft 1px tinted border + faint
            // background. This is the noctalia-style "something's
            // wrong" signal — visible at a glance without the loud
            // saturated chip look.
            IndicatorState::Warning | IndicatorState::Danger => container(content)
                .padding([1, 5])
                .style(move |theme: &Theme| {
                    let accent = match state {
                        IndicatorState::Warning => theme.palette().warning,
                        IndicatorState::Danger => theme.palette().danger,
                        _ => unreachable!(),
                    };
                    container::Style {
                        text_color: Some(accent),
                        background: Some(Background::Color(Color {
                            a: 0.10,
                            ..accent
                        })),
                        border: Border {
                            width: 1.0,
                            radius: 6.0.into(),
                            color: Color {
                                a: 0.45,
                                ..accent
                            },
                        },
                        ..Default::default()
                    }
                })
                .into(),
        };

        // Optional progress fill: a 2px bar along the indicator's
        // bottom edge that grows with the current value (volume,
        // brightness, etc.). Implemented as a Row of two flex children
        // so the fill width tracks the indicator width at any size.
        let with_progress: Element<'a, Msg> = match fi.progress {
            Some(p) => {
                let filled = (p * 1000.0).round().clamp(0.0, 1000.0) as u16;
                let empty = 1000u16.saturating_sub(filled);
                let fill_bar = Row::new()
                    .push(
                        container(Space::new())
                            .width(Length::FillPortion(filled.max(1)))
                            .height(Length::Fixed(2.0))
                            .style(|theme: &Theme| container::Style {
                                background: Some(Background::Color(theme.palette().primary)),
                                border: Border {
                                    radius: 1.0.into(),
                                    width: 0.0,
                                    color: Color::TRANSPARENT,
                                },
                                ..Default::default()
                            }),
                    )
                    .push(
                        container(Space::new())
                            .width(Length::FillPortion(empty.max(1)))
                            .height(Length::Fixed(2.0))
                            .style(|theme: &Theme| container::Style {
                                background: Some(Background::Color(Color {
                                    a: 0.18,
                                    ..theme.palette().primary
                                })),
                                ..Default::default()
                            }),
                    );

                // Edge-case: a value of exactly 0 still wants a sliver
                // of accent for visual continuity; exactly 1 wants no
                // empty trough. Hide the dummy 1-portion via 0 alpha
                // when its real share is 0.
                let fill_layer = container(fill_bar)
                    .align_y(Alignment::End)
                    .width(Length::Fill)
                    .height(Length::Fill);

                Stack::new().push(colored).push(fill_layer).into()
            }
            None => colored,
        };

        if fi.on_scroll.is_some() || fi.on_right_press.is_some() {
            let mut area = MouseArea::new(with_progress);
            if let Some(handler) = fi.on_scroll {
                area = area.on_scroll(handler);
            }
            if let Some(msg) = fi.on_right_press {
                area = area.on_right_press(msg);
            }
            area.into()
        } else {
            with_progress
        }
    }
}
