use crate::{
    config::{WindowTitleConfig, WindowTitleMode},
    services::{ReadOnlyService, ServiceEvent, compositor::CompositorService},
    theme::use_theme,
    utils::truncate_text,
};
use iced::{
    Element, Subscription,
    widget::{container, text},
};

#[derive(Debug, Clone)]
pub enum Message {
    ServiceEvent(Box<ServiceEvent<CompositorService>>),
    ConfigReloaded(WindowTitleConfig),
}

pub struct WindowTitle {
    config: WindowTitleConfig,
    service: Option<CompositorService>,
    value: Option<String>,
}

impl WindowTitle {
    pub fn new(config: WindowTitleConfig) -> Self {
        Self {
            config,
            service: None,
            value: None,
        }
    }

    pub fn update(&mut self, message: Message) {
        match message {
            Message::ServiceEvent(event) => match *event {
                ServiceEvent::Init(service) => {
                    self.service = Some(service);
                    self.recalculate_value();
                }
                ServiceEvent::Update(event) => {
                    if let Some(service) = &mut self.service {
                        service.update(event);
                        self.recalculate_value();
                    }
                }
                _ => {}
            },
            Message::ConfigReloaded(cfg) => {
                self.config = cfg;
                self.recalculate_value();
            }
        }
    }

    fn recalculate_value(&mut self) {
        let Some(service) = &self.service else {
            return;
        };

        // Hold the last-known toplevel title when focus shifts to a
        // non-toplevel surface (one of mshell's own menus opens with
        // KeyboardInteractivity::Exclusive — margo then reports the
        // layer surface as focused, the client list has no matching
        // entry, and `active_window` collapses to `None`). Letting
        // that propagate would blank the bar item and shift the
        // whole bar layout every time a menu opens. Same logic for
        // an `active_window` whose title is empty (e.g. a window
        // that hasn't set xdg_toplevel.set_title yet) — keep the
        // last value rather than churn.
        let Some(w) = service.active_window.as_ref() else {
            return;
        };

        let raw_title: &str = match self.config.mode {
            WindowTitleMode::Title => &w.title,
            WindowTitleMode::Class => &w.class,
            // margo doesn't track the toplevel's first-commit
            // app_id/title separately from the current one;
            // fall back to the live value so the module still
            // renders something useful instead of going blank.
            WindowTitleMode::InitialTitle => &w.title,
            WindowTitleMode::InitialClass => &w.class,
        };

        if raw_title.is_empty() {
            return;
        }

        // Apply hard limit of 2048 characters to prevent Wayland E2BIG errors
        let max_length = if self.config.truncate_title_after_length > 0 {
            std::cmp::min(self.config.truncate_title_after_length, 2048)
        } else {
            2048
        };

        self.value = Some(truncate_text(raw_title, max_length));
    }

    pub fn get_value(&self) -> Option<String> {
        self.value.clone()
    }

    pub fn view(&'_ self, title: String) -> Element<'_, Message> {
        use_theme(|theme| {
            container(
                text(title)
                    .size(theme.bar_font_size)
                    .wrapping(text::Wrapping::None),
            )
            .clip(true)
            .into()
        })
    }

    pub fn subscription(&self) -> Subscription<Message> {
        CompositorService::subscribe().map(|event| Message::ServiceEvent(Box::new(event)))
    }
}
