use ratatui::{
    style::Style,
    widgets::{Block, BorderType, Borders},
    Frame,
};

use crate::config::{get_color, BackgroundConfig};

#[derive(Clone)]
pub struct BackgroundWidget {
    config: BackgroundConfig,
}

impl BackgroundWidget {
    pub fn new(config: BackgroundConfig) -> Self {
        Self { config }
    }

    pub fn render(&self, frame: &mut Frame<impl ratatui::backend::Backend>) {
        if !self.config.show_background {
            return;
        }
        let block = Block::default().style(self.background_style());

        let bounding_box = frame.size();

        let block = if self.config.style.show_border {
            block
                .borders(Borders::ALL)
                // Rounded corners to match the credential card (the square
                // ┌┐└┘ default clashed with the card's ╭╮╰╯).
                .border_type(BorderType::Rounded)
                .border_style(self.border_style())
        } else {
            block
        };

        frame.render_widget(block, bounding_box);
    }

    fn background_style(&self) -> Style {
        Style::default().bg(get_color(&self.config.style.color))
    }
    fn border_style(&self) -> Style {
        Style::default().fg(get_color(&self.config.style.border_color))
    }
}
