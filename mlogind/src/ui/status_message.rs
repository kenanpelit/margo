use ratatui::backend::Backend;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Style};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::auth::AuthenticationError;

#[derive(Clone)]
pub enum ErrorStatusMessage {
    AuthenticationError(AuthenticationError),
    NoGraphicalEnvironment,
    FailedGraphicalEnvironment,
    FailedDesktop,
    FailedPowerControl(String),
}

impl From<ErrorStatusMessage> for Box<str> {
    fn from(err: ErrorStatusMessage) -> Self {
        use ErrorStatusMessage::*;

        match err {
            AuthenticationError(_) => "Authentication failed".into(),
            NoGraphicalEnvironment => "No graphical environment specified".into(),
            FailedGraphicalEnvironment => "Failed booting into the graphical environment".into(),
            FailedDesktop => "Failed booting into desktop environment".into(),
            FailedPowerControl(name) => {
                format!("Failed to {name}... Check the logs for more information").into()
            }
        }
    }
}

impl From<ErrorStatusMessage> for StatusMessage {
    fn from(err: ErrorStatusMessage) -> Self {
        Self::Error(err)
    }
}

#[derive(Clone, Copy)]
pub enum InfoStatusMessage {
    LoggingIn,
    Authenticating,
}

impl From<InfoStatusMessage> for Box<str> {
    fn from(info: InfoStatusMessage) -> Self {
        use InfoStatusMessage::*;

        match info {
            LoggingIn => "Authentication successful. Logging in...".into(),
            Authenticating => "Verifying credentials".into(),
        }
    }
}

impl From<InfoStatusMessage> for StatusMessage {
    fn from(info: InfoStatusMessage) -> Self {
        Self::Info(info)
    }
}

#[derive(Clone)]
pub enum StatusMessage {
    Error(ErrorStatusMessage),
    Info(InfoStatusMessage),
}

impl From<StatusMessage> for Box<str> {
    fn from(msg: StatusMessage) -> Self {
        use StatusMessage::*;

        match msg {
            Error(sm) => sm.into(),
            Info(sm) => sm.into(),
        }
    }
}

impl StatusMessage {
    /// Fetch whether status is an error
    pub fn is_error(&self) -> bool {
        matches!(self, Self::Error(_))
    }

    /// Centred, themed status line. Errors use the palette's danger colour,
    /// info messages the muted colour — matching the rest of the greeter
    /// instead of fixed red/yellow.
    pub fn render<B: Backend>(
        status: Option<Self>,
        frame: &mut Frame<B>,
        area: Rect,
        danger: Color,
        info: Color,
    ) {
        if let Some(status_message) = status {
            let text: Box<str> = status_message.clone().into();
            let color = if status_message.is_error() {
                danger
            } else {
                info
            };
            let widget = Paragraph::new(text.as_ref())
                .alignment(Alignment::Center)
                .style(Style::default().fg(color));

            frame.render_widget(widget, area);
        } else {
            // Clear the area
            frame.render_widget(Paragraph::new(""), area);
        }
    }
}
