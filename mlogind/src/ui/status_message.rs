use ratatui::Frame;
use ratatui::backend::Backend;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Style};
use ratatui::widgets::Paragraph;

#[derive(Clone)]
pub enum ErrorStatusMessage {
    /// Verbatim text from the session runner: a `Failure` reason, or a
    /// `PAM_ERROR_MSG` the stack raised mid-conversation ("account expired",
    /// "fingerprint not recognised"). The runner decides what the user is told;
    /// whether the account even exists is not the greeter's business.
    FromRunner(String),
    /// The session runner went away. Nothing the user types can help.
    RunnerGone,
    NoGraphicalEnvironment,
    FailedGraphicalEnvironment,
    FailedDesktop,
    FailedPowerControl(String),
}

impl From<ErrorStatusMessage> for Box<str> {
    fn from(err: ErrorStatusMessage) -> Self {
        use ErrorStatusMessage::*;

        match err {
            FromRunner(text) => text.into(),
            RunnerGone => "Lost the session runner. Check the logs".into(),
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
    /// A `PAM_TEXT_INFO` message, or the text of a prompt the form cannot
    /// answer from the fields the user already filled in — "Touch your security
    /// key", "New password:". Rendered like an info message: it is a question,
    /// not a failure.
    FromRunner(String),
}

impl From<StatusMessage> for Box<str> {
    fn from(msg: StatusMessage) -> Self {
        use StatusMessage::*;

        match msg {
            Error(sm) => sm.into(),
            Info(sm) => sm.into(),
            FromRunner(text) => text.into(),
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
