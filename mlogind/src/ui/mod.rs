use log::{error, info, warn};

use std::io;
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use crate::config::{Config, FocusBehaviour, SwitcherVisibility};
use crate::info_caching::{get_cached_information, set_cache};
use crate::post_login::PostLoginEnvironment;
use mlogind_proto::{Conn, Event as ProtoEvent, FdTransport, Request};
use status_message::StatusMessage;
use zeroize::Zeroizing;

use crossterm::cursor::MoveTo;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, Clear, ClearType, LeaveAlternateScreen};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Alignment;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use ratatui::{backend::Backend, Frame, Terminal};

use chrono::{Local, Timelike};

use crate::config::get_color;

mod background;
mod chunks;
mod clock;
mod input_field;
mod key_menu;
mod status_message;
mod switcher;

use chunks::Chunks;
use input_field::{InputFieldDisplayType, InputFieldWidget};
use key_menu::KeyMenuWidget;
use status_message::{ErrorStatusMessage, InfoStatusMessage};
use switcher::{SwitcherItem, SwitcherWidget};

use self::background::BackgroundWidget;

#[derive(Clone)]
struct LoginFormInputMode(Arc<Mutex<InputMode>>);

impl LoginFormInputMode {
    fn new(mode: InputMode) -> Self {
        Self(Arc::new(Mutex::new(mode)))
    }

    fn get_guard(&self) -> MutexGuard<'_, InputMode> {
        let Self(mutex) = self;

        match mutex.lock() {
            Ok(guard) => guard,
            Err(err) => {
                error!("Lock failed. Reason: {}", err);
                std::process::exit(1);
            }
        }
    }

    fn get(&self) -> InputMode {
        *self.get_guard()
    }

    fn prev(&self, skip_switcher: bool) {
        self.get_guard().prev(skip_switcher)
    }
    fn next(&self, skip_switcher: bool) {
        self.get_guard().next(skip_switcher)
    }
    fn set(&self, mode: InputMode) {
        *self.get_guard() = mode;
    }
}

#[derive(Clone)]
struct LoginFormStatusMessage(Arc<Mutex<Option<StatusMessage>>>);

impl LoginFormStatusMessage {
    fn new() -> Self {
        Self(Arc::new(Mutex::new(None)))
    }

    fn get_guard(&self) -> MutexGuard<'_, Option<StatusMessage>> {
        let Self(mutex) = self;

        match mutex.lock() {
            Ok(guard) => guard,
            Err(err) => {
                error!("Lock failed. Reason: {}", err);
                std::process::exit(1);
            }
        }
    }

    fn get(&self) -> Option<StatusMessage> {
        self.get_guard().clone()
    }

    fn clear(&self) {
        *self.get_guard() = None;
    }
    fn set(&self, msg: impl Into<StatusMessage>) {
        *self.get_guard() = Some(msg.into());
    }
}

/// All the different modes for input
#[derive(Clone, Copy)]
enum InputMode {
    /// Using the env switcher widget
    Switcher,

    /// Typing within the Username input field
    Username,

    /// Typing within the Password input field
    Password,

    /// Nothing selected
    Normal,
}

impl InputMode {
    /// Move to the next mode
    fn next(&mut self, skip_switcher: bool) {
        use InputMode::*;

        *self = match self {
            Normal => {
                if skip_switcher {
                    Username
                } else {
                    Switcher
                }
            }
            Switcher => Username,
            Username => Password,
            Password => Password,
        }
    }

    /// Move to the previous mode
    fn prev(&mut self, skip_switcher: bool) {
        use InputMode::*;

        *self = match self {
            Normal => Normal,
            Switcher => Normal,
            Username => {
                if skip_switcher {
                    Normal
                } else {
                    Switcher
                }
            }
            Password => Username,
        }
    }
}

enum UIThreadRequest {
    Redraw,
    /// Leave the alternate screen. The runner is about to take the VT, and the
    /// form never comes back — it exits, and the daemon draws a fresh one after
    /// the session ends. (There used to be an `EnableTui` for the return trip,
    /// back when the form itself forked the session and waited for it.)
    DisableTui,
    StopDrawing(Outcome),
}

/// Why the login form stopped drawing.
///
/// The form no longer runs a session itself, so it has to say what it did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The user left (Esc in preview), or there was never a runner to talk to.
    Quit,
    /// PAM said yes. The runner is waiting for us to release the screen.
    SessionStarting,
    /// The session runner died mid-conversation. The caller should start a new one.
    RunnerGone,
}

#[derive(Clone)]
struct Widgets {
    background: BackgroundWidget,
    key_menu: KeyMenuWidget,
    environment: Arc<Mutex<SwitcherWidget<PostLoginEnvironment>>>,
    username: Arc<Mutex<InputFieldWidget>>,
    password: Arc<Mutex<InputFieldWidget>>,
}

impl Widgets {
    fn environment_guard(&self) -> MutexGuard<'_, SwitcherWidget<PostLoginEnvironment>> {
        match self.environment.lock() {
            Ok(guard) => guard,
            Err(err) => {
                error!("Lock failed. Reason: {}", err);
                std::process::exit(1);
            }
        }
    }
    fn username_guard(&self) -> MutexGuard<'_, InputFieldWidget> {
        match self.username.lock() {
            Ok(guard) => guard,
            Err(err) => {
                error!("Lock failed. Reason: {}", err);
                std::process::exit(1);
            }
        }
    }
    fn password_guard(&self) -> MutexGuard<'_, InputFieldWidget> {
        match self.password.lock() {
            Ok(guard) => guard,
            Err(err) => {
                error!("Lock failed. Reason: {}", err);
                std::process::exit(1);
            }
        }
    }

    fn get_environment(&self) -> Option<(String, PostLoginEnvironment)> {
        self.environment_guard()
            .selected()
            .map(|s| (s.title.clone(), s.content.clone()))
    }
    fn environment_try_select(&self, title: &str) {
        self.environment_guard().try_select(title);
    }
    fn get_username(&self) -> String {
        self.username_guard().get_content()
    }
    fn set_username(&self, content: &str) {
        self.username_guard().set_content(content)
    }
    fn get_password(&self) -> String {
        self.password_guard().get_content()
    }
    fn clear_password(&self) {
        self.password_guard().clear()
    }
}

/// App holds the state of the application
#[derive(Clone)]
pub struct LoginForm {
    /// No socket to a session runner: submit only animates. Set by `--preview`,
    /// and by a bare `mlogind --greet` with no `MLOGIND_SOCK_FD`.
    preview: bool,

    widgets: Widgets,

    /// The configuration for the app
    config: Config,
}

impl LoginForm {
    fn set_cache(&self) {
        let env_remember = self.config.environment_switcher.remember;
        let username_remember = self.config.username_field.remember;

        if !env_remember && !username_remember {
            info!("Nothing to cache.");
            return;
        }

        let selected_env = if self.config.environment_switcher.remember {
            self.widgets.get_environment().map(|(title, _)| title)
        } else {
            None
        };
        let username = self
            .config
            .username_field
            .remember
            .then_some(self.widgets.get_username());

        info!("Setting cached information");
        set_cache(selected_env.as_deref(), username.as_deref(), &self.config);
    }

    fn load_cache(&self) {
        let env_remember = self.config.environment_switcher.remember;
        let username_remember = self.config.username_field.remember;

        let cached = get_cached_information(&self.config);

        if username_remember {
            if let Some(username) = cached.username() {
                info!("Loading username '{}' from cache", username);
                self.widgets.set_username(username);
            }
        }
        if env_remember {
            if let Some(env) = cached.environment() {
                info!("Loading environment '{}' from cache", env);
                self.widgets.environment_try_select(env);
            }
        }
    }

    pub fn new(config: Config, preview: bool) -> LoginForm {
        LoginForm {
            preview,
            widgets: Widgets {
                background: BackgroundWidget::new(config.background.clone()),
                key_menu: KeyMenuWidget::new(
                    config.power_controls.clone(),
                    config.environment_switcher.clone(),
                ),
                environment: Arc::new(Mutex::new(SwitcherWidget::new(
                    crate::post_login::get_envs(&config)
                        .into_iter()
                        .map(|(title, content)| SwitcherItem::new(title, content))
                        .collect(),
                    config.environment_switcher.clone(),
                ))),
                // The fields now live inside the rounded card, so they drop
                // their own border + title (the card draws the chrome; the
                // label is rendered to the left of each row).
                username: Arc::new(Mutex::new(InputFieldWidget::new(
                    InputFieldDisplayType::Echo,
                    {
                        let mut s = config.username_field.style.clone();
                        s.show_border = false;
                        s.show_title = false;
                        s
                    },
                    String::default(),
                ))),
                password: Arc::new(Mutex::new(InputFieldWidget::new(
                    InputFieldDisplayType::Replace(
                        config
                            .password_field
                            .content_replacement_character
                            .to_string(),
                    ),
                    {
                        let mut s = config.password_field.style.clone();
                        s.show_border = false;
                        s.show_title = false;
                        s
                    },
                    String::default(),
                ))),
            },
            config,
        }
    }

    /// Draw the form and, if we have a socket to a session runner, drive one
    /// login conversation over it.
    ///
    /// `conn` is taken by value: the event loop lives on its own thread, and a
    /// borrow could not cross `thread::spawn`. The caller keeps the `OwnedFd`;
    /// `Conn` only borrows the number. We join the thread before returning, so
    /// the fd cannot be closed underneath it.
    pub fn run(
        self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
        conn: Option<Conn<FdTransport>>,
    ) -> io::Result<Outcome> {
        self.load_cache();
        let input_mode = LoginFormInputMode::new(match self.config.focus_behaviour {
            FocusBehaviour::FirstNonCached => match (
                self.config.username_field.remember && !self.widgets.get_username().is_empty(),
                self.config.environment_switcher.remember
                    && self
                        .widgets
                        .get_environment()
                        .map(|(title, _)| !title.is_empty())
                        .unwrap_or(false),
            ) {
                (true, true) => InputMode::Password,
                (true, _) => InputMode::Username,
                _ => {
                    if self.config.environment_switcher.switcher_visibility
                        == SwitcherVisibility::Visible
                    {
                        InputMode::Switcher
                    } else {
                        InputMode::Username
                    }
                }
            },
            FocusBehaviour::NoFocus => InputMode::Normal,
            FocusBehaviour::Environment => InputMode::Switcher,
            FocusBehaviour::Username => InputMode::Username,
            FocusBehaviour::Password => InputMode::Password,
        });
        let status_message = LoginFormStatusMessage::new();
        let background = self.widgets.background.clone();
        let key_menu = self.widgets.key_menu.clone();
        let environment = self.widgets.environment.clone();
        let username = self.widgets.username.clone();
        let password = self.widgets.password.clone();
        let theme = Theme::from_config(&self.config);

        let draw_action = terminal.draw(|f| {
            let layout = Chunks::new(f);
            login_form_render(
                f,
                layout,
                theme,
                background.clone(),
                key_menu.clone(),
                environment.clone(),
                username.clone(),
                password.clone(),
                input_mode.get(),
                status_message.get(),
            );
        });

        if let Err(err) = draw_action {
            error!("Failed to draw. Reason: {}", err);
            std::process::exit(1);
        }

        let event_input_mode = input_mode.clone();
        let event_status_message = status_message.clone();

        let (req_send_channel, req_recv_channel) = channel();

        // Keep the clock live: tick a redraw every second. Gated by
        // `tui_enabled` in the draw loop so we never paint over a session
        // during the login hand-off. The send fails (and the thread exits)
        // once the receiver is dropped at shutdown.
        let tui_enabled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        {
            let tick_sender = req_send_channel.clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(Duration::from_secs(1));
                if tick_sender.send(UIThreadRequest::Redraw).is_err() {
                    break;
                }
            });
        }

        let event_thread = std::thread::spawn(move || {
            let mut switcher_hidden = self
                .widgets
                .environment
                .lock()
                .expect("Failed to grab environment lock")
                .hidden();
            let input_mode = event_input_mode;
            let status_message = event_status_message;

            let send_ui_request = |request: UIThreadRequest| match req_send_channel.send(request) {
                Ok(_) => {}
                Err(err) => warn!("Failed to send UI request. Reason: {}", err),
            };

            let redraw = || send_ui_request(UIThreadRequest::Redraw);

            let mut conn = conn;
            // True while PAM is mid-conversation and has asked something the
            // filled-in form could not answer. The next Enter sends the reply
            // rather than starting a fresh login.
            let mut awaiting_prompt = false;

            loop {
                if let Ok(Event::Key(key)) = event::read() {
                    match (key.code, input_mode.get(), key.modifiers) {
                        (KeyCode::Enter, InputMode::Password, _) => {
                            let Some(conn) = conn.as_mut() else {
                                // No runner to talk to (`--preview`, or a bare
                                // `mlogind --greet`). Animate and stay put.
                                status_message.set(InfoStatusMessage::Authenticating);
                                send_ui_request(UIThreadRequest::Redraw);
                                std::thread::sleep(Duration::from_secs(2));

                                status_message.set(InfoStatusMessage::LoggingIn);
                                send_ui_request(UIThreadRequest::Redraw);
                                std::thread::sleep(Duration::from_secs(2));

                                status_message.clear();
                                send_ui_request(UIThreadRequest::Redraw);
                                continue;
                            };

                            let pumped = if awaiting_prompt {
                                awaiting_prompt = false;
                                // Answering an extra PAM question — an OTP, a
                                // new password. Zeroizing: this is a root
                                // process, and freed heap survives in core
                                // dumps and swap.
                                let answer = Zeroizing::new(self.widgets.get_password());
                                self.widgets.clear_password();
                                let sent = conn.send_request(&Request::Response {
                                    secret: Zeroizing::new(answer.as_bytes().to_vec()),
                                });
                                if sent.is_err() {
                                    Pumped::Disconnected
                                } else {
                                    let mut nothing = None;
                                    pump(
                                        conn,
                                        &mut nothing,
                                        &self.widgets,
                                        &status_message,
                                        &redraw,
                                    )
                                }
                            } else {
                                let Some((env_name, _)) = self.widgets.get_environment() else {
                                    status_message.set(ErrorStatusMessage::NoGraphicalEnvironment);
                                    send_ui_request(UIThreadRequest::Redraw);
                                    continue;
                                };
                                let username = self.widgets.get_username();
                                let mut password =
                                    Some(Zeroizing::new(self.widgets.get_password()));
                                self.widgets.clear_password();

                                status_message.set(InfoStatusMessage::Authenticating);
                                send_ui_request(UIThreadRequest::Redraw);

                                let sent = conn.send_request(&Request::Begin {
                                    user: username,
                                    session: env_name,
                                });
                                if sent.is_err() {
                                    Pumped::Disconnected
                                } else {
                                    pump(
                                        conn,
                                        &mut password,
                                        &self.widgets,
                                        &status_message,
                                        &redraw,
                                    )
                                }
                            };

                            match pumped {
                                // The runner is holding a prompt open for us.
                                Pumped::NeedInput => {
                                    awaiting_prompt = true;
                                    input_mode.set(InputMode::Password);
                                }
                                // Wrong password. The socket is still good and
                                // the runner is already waiting for a new Begin.
                                Pumped::Failed => input_mode.set(InputMode::Password),
                                Pumped::Success => {
                                    status_message.set(InfoStatusMessage::LoggingIn);
                                    send_ui_request(UIThreadRequest::Redraw);
                                    // Hand the screen back before the runner
                                    // opens DRM on this very VT.
                                    send_ui_request(UIThreadRequest::DisableTui);
                                    req_send_channel
                                        .send(UIThreadRequest::StopDrawing(
                                            Outcome::SessionStarting,
                                        ))
                                        .ok();
                                    return;
                                }
                                Pumped::Disconnected => {
                                    error!("greeter: lost the session runner");
                                    status_message.set(ErrorStatusMessage::RunnerGone);
                                    send_ui_request(UIThreadRequest::Redraw);
                                    req_send_channel
                                        .send(UIThreadRequest::StopDrawing(Outcome::RunnerGone))
                                        .ok();
                                    return;
                                }
                            }
                        }
                        (KeyCode::Char('s'), InputMode::Normal, _) => self.set_cache(),

                        // On the TTY, it triggers the ALT key for some reason.
                        (KeyCode::Up | KeyCode::BackTab, _, _)
                        | (KeyCode::Tab, _, KeyModifiers::ALT | KeyModifiers::SHIFT)
                        | (KeyCode::Char('p'), _, KeyModifiers::CONTROL) => {
                            input_mode.prev(switcher_hidden);
                        }

                        (KeyCode::Enter | KeyCode::Down | KeyCode::Tab, _, _)
                        | (KeyCode::Char('n'), _, KeyModifiers::CONTROL) => {
                            input_mode.next(switcher_hidden);
                        }

                        // Esc is the overall key to get out of your input mode
                        (KeyCode::Esc, InputMode::Normal, _) => {
                            if self.preview {
                                info!("Pressed escape in preview mode to exit the application");
                                req_send_channel
                                    .send(UIThreadRequest::StopDrawing(Outcome::Quit))
                                    .ok();
                                return;
                            }
                        }

                        (KeyCode::Esc, _, _) => {
                            input_mode.set(InputMode::Normal);
                        }

                        (KeyCode::F(_), _, _) => {
                            // Power actions are the root runner's to perform: under
                            // `cage` this same form is the unprivileged greeter, and
                            // in `--preview` it must not shut the machine down at
                            // all (it used to — `mlogind --preview` plus F1).
                            if let Some(index) = self.widgets.key_menu.power_index(key.code) {
                                match conn.as_mut() {
                                    Some(conn) => {
                                        if let Some(msg) = request_power(conn, index, &redraw) {
                                            status_message.set(msg);
                                        }
                                    }
                                    None => info!(
                                        "greeter: power action {index} ignored; no session runner"
                                    ),
                                }
                            }
                            self.widgets.environment_guard().key_press(key.code);

                            switcher_hidden = self
                                .widgets
                                .environment
                                .lock()
                                .expect("Failed to grab lock")
                                .hidden();

                            if matches!(input_mode.get(), InputMode::Switcher) && switcher_hidden {
                                input_mode.next(true);
                            }
                        }

                        // For the different input modes the key should be passed to the corresponding
                        // widget.
                        (k, mode, modifiers) => {
                            let status_message_opt = match mode {
                                InputMode::Switcher => {
                                    self.widgets.environment_guard().key_press(k)
                                }
                                InputMode::Username => {
                                    self.widgets.username_guard().key_press(k, modifiers)
                                }
                                InputMode::Password => {
                                    self.widgets.password_guard().key_press(k, modifiers)
                                }
                                _ => None,
                            };

                            // We don't wanna clear any existing error messages
                            if let Some(status_msg) = status_message_opt {
                                status_message.set(status_msg);
                            }
                        }
                    };
                }

                send_ui_request(UIThreadRequest::Redraw);
            }
        });

        // Start the UI thread. This actually draws to the screen.
        //
        // This blocks until the event thread calls StopDrawing.
        let mut outcome = Outcome::Quit;
        while let Ok(request) = req_recv_channel.recv() {
            use std::sync::atomic::Ordering;
            match request {
                // Skip the 1 s clock ticks (and any stray redraws) while the
                // TUI is handed off to a starting session.
                UIThreadRequest::Redraw if !tui_enabled.load(Ordering::Relaxed) => {}
                UIThreadRequest::Redraw => {
                    let draw_action = terminal.draw(|f| {
                        let layout = Chunks::new(f);
                        login_form_render(
                            f,
                            layout,
                            theme,
                            background.clone(),
                            key_menu.clone(),
                            environment.clone(),
                            username.clone(),
                            password.clone(),
                            input_mode.get(),
                            status_message.get(),
                        );
                    });

                    if let Err(err) = draw_action {
                        warn!("Failed to draw to screen. Reason: {err}");
                    }
                }
                UIThreadRequest::DisableTui => {
                    tui_enabled.store(false, Ordering::Relaxed);
                    // Restore the console's default (dark) palette before we
                    // clear the screen for the session hand-off. We reprogram
                    // VT palette slots to the matugen theme (a purple surface +
                    // accent); leaving them set meant `Clear` painted the bare
                    // console — every monitor — in that purple for the 1-2 s
                    // until the compositor took over. Resetting first keeps the
                    // hand-off dark; the next greeter draw reprograms it.
                    crate::console_palette::reset();
                    disable_raw_mode()?;
                    execute!(
                        terminal.backend_mut(),
                        LeaveAlternateScreen,
                        Clear(ClearType::All),
                        MoveTo(0, 0)
                    )?;
                    terminal.show_cursor()?;
                }
                UIThreadRequest::StopDrawing(reason) => {
                    outcome = reason;
                    break;
                }
            }
        }

        // The event thread has already returned by the time it sends
        // StopDrawing, so this is prompt — and it guarantees nothing still
        // holds the socket when the caller drops the fd.
        if event_thread.join().is_err() {
            error!("greeter: the event thread panicked");
        }

        Ok(outcome)
    }
}

/// Ask the session runner to run power action `index`, and wait for its verdict.
///
/// The runner always answers — `Info` on success, `Error` on failure — so this
/// cannot hang when an action returns instead of taking the machine down
/// (`suspend`). Returns a status message when there is something to say.
fn request_power(
    conn: &mut Conn<FdTransport>,
    index: usize,
    redraw: &dyn Fn(),
) -> Option<StatusMessage> {
    let index = u32::try_from(index).ok()?;
    if conn.send_request(&Request::Power { index }).is_err() {
        return Some(ErrorStatusMessage::RunnerGone.into());
    }
    let message = match conn.recv_event() {
        Ok(Some(ProtoEvent::Info { text })) => StatusMessage::FromRunner(text),
        Ok(Some(ProtoEvent::Error { text })) => ErrorStatusMessage::FromRunner(text).into(),
        // Anything else here is the runner losing the plot; do not act on it.
        Ok(Some(_)) => ErrorStatusMessage::FromRunner("Unexpected reply".into()).into(),
        Ok(None) | Err(_) => ErrorStatusMessage::RunnerGone.into(),
    };
    redraw();
    Some(message)
}

/// Where a pumped conversation left off.
enum Pumped {
    /// PAM asked something the filled-in form cannot answer.
    NeedInput,
    Success,
    /// This attempt failed. The socket is still good; retry from the form.
    Failed,
    /// The runner is gone. Nothing the user types can help.
    Disconnected,
}

/// Drive the conversation until it needs the user again, or ends.
///
/// `password` is the one answer we already hold. PAM's first *blind* prompt is
/// the password prompt — the runner answers the username prompt itself, from
/// `Begin`, so it never reaches us. Everything after that is a real question: a
/// second factor, an expired-password change. Those go back to the form.
fn pump(
    conn: &mut Conn<FdTransport>,
    password: &mut Option<Zeroizing<String>>,
    widgets: &Widgets,
    status_message: &LoginFormStatusMessage,
    redraw: &dyn Fn(),
) -> Pumped {
    loop {
        let event = match conn.recv_event() {
            Ok(Some(event)) => event,
            Ok(None) => return Pumped::Disconnected,
            Err(err) => {
                error!("greeter: protocol error: {err}");
                return Pumped::Disconnected;
            }
        };

        match event {
            ProtoEvent::Prompt { echo, text } => {
                if !echo {
                    if let Some(secret) = password.take() {
                        let sent = conn.send_request(&Request::Response {
                            secret: Zeroizing::new(secret.as_bytes().to_vec()),
                        });
                        if sent.is_err() {
                            return Pumped::Disconnected;
                        }
                        continue;
                    }
                }
                // A question the form has no answer for. The reply is always
                // masked, whatever `echo` asked for: A1's TUI has exactly one
                // spare field and it is the password widget. Echoing an
                // echo-on prompt is phase D.
                let _ = echo;
                widgets.clear_password();
                status_message.set(StatusMessage::FromRunner(text));
                redraw();
                return Pumped::NeedInput;
            }
            ProtoEvent::Info { text } => {
                status_message.set(StatusMessage::FromRunner(text));
                redraw();
            }
            ProtoEvent::Error { text } => {
                status_message.set(ErrorStatusMessage::FromRunner(text));
                redraw();
            }
            ProtoEvent::Success => return Pumped::Success,
            ProtoEvent::Failure { reason } => {
                widgets.clear_password();
                status_message.set(ErrorStatusMessage::FromRunner(reason));
                redraw();
                return Pumped::Failed;
            }
        }
    }
}

/// The greeter's semantic colours, pulled from the (matugen-driven) config
/// so the new decorative elements track the wallpaper theme exactly like
/// the input widgets do. The field colours map straight to margo's palette
/// variables: `border_color_focused → $accent`, `content_color → $text`,
/// `title_color → $subtext/muted`, `no_envs_color_focused → $danger`.
#[derive(Clone, Copy)]
struct Theme {
    accent: Color,
    text: Color,
    muted: Color,
    danger: Color,
}

impl Theme {
    fn from_config(config: &Config) -> Self {
        Self {
            accent: get_color(&config.username_field.style.border_color_focused),
            text: get_color(&config.username_field.style.content_color),
            muted: get_color(&config.username_field.style.title_color),
            danger: get_color(&config.environment_switcher.no_envs_color_focused),
        }
    }
}

fn greeting_for(hour: u32) -> &'static str {
    match hour {
        5..=11 => "Good morning",
        12..=16 => "Good afternoon",
        17..=20 => "Good evening",
        _ => "Good night",
    }
}

/// Truncate `s` to `max` columns, adding an ellipsis when it has to cut.
fn fit(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[allow(clippy::too_many_arguments)]
fn login_form_render<B: Backend>(
    frame: &mut Frame<B>,
    chunks: Chunks,
    theme: Theme,
    background: BackgroundWidget,
    key_menu: KeyMenuWidget,
    environment: Arc<Mutex<SwitcherWidget<PostLoginEnvironment>>>,
    username: Arc<Mutex<InputFieldWidget>>,
    password: Arc<Mutex<InputFieldWidget>>,
    input_mode: InputMode,
    status_message: Option<StatusMessage>,
) {
    background.render(frame);

    let now = Local::now();
    let muted = Style::default().fg(theme.muted);
    let label_style =
        |focused: bool| Style::default().fg(if focused { theme.accent } else { theme.muted });

    // Greeting.
    frame.render_widget(
        Paragraph::new(greeting_for(now.hour()))
            .alignment(Alignment::Center)
            .style(muted),
        chunks.greeting,
    );

    // Big block clock (mlock's centrepiece). The 5 equal-width rows centre
    // as a block; `text` colour keeps accent reserved for focus.
    frame.render_widget(
        Paragraph::new(clock::big_time(&now.format("%H:%M").to_string()).join("\n"))
            .alignment(Alignment::Center)
            .style(Style::default().fg(theme.text).add_modifier(Modifier::BOLD)),
        chunks.clock,
    );

    // Date.
    frame.render_widget(
        Paragraph::new(now.format("%A, %-d %B %Y").to_string())
            .alignment(Alignment::Center)
            .style(muted),
        chunks.date,
    );

    // The accent card around the credentials — always accent, so the theme
    // reads even before the user types. Square corners (Plain): the bare VT's
    // console font has ┌┐└┘ but not the rounded ╭╮╰╯ glyphs, which would show
    // as broken / unjoined corners on a real TTY (mlock can do rounded because
    // it's a graphical Wayland surface; a TUI greeter can't).
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Plain)
            .border_style(Style::default().fg(theme.accent)),
        chunks.card,
    );

    // Session row — only when the switcher is shown. The focused row's label
    // lights up in accent (the field is borderless now, so the label is the
    // focus cue alongside the cursor).
    {
        let env = environment.lock().unwrap_or_else(|err| {
            error!("Failed to lock post-login environment. Reason: {}", err);
            std::process::exit(1);
        });
        if !env.hidden() {
            let focused = matches!(input_mode, InputMode::Switcher);
            frame.render_widget(
                Paragraph::new("Session").style(label_style(focused)),
                chunks.label_session,
            );
            // Render the session inline (no fixed-width carousel that clips at
            // narrow widths): the name sits next to the label with ‹ › arrows
            // when there's more than one, truncated to the available width.
            let avail = chunks.switcher.width as usize;
            let (text, color) = match env.current_title() {
                Some(title) => {
                    let left = if env.has_prev() { "‹ " } else { "" };
                    let right = if env.has_next() { " ›" } else { "" };
                    let fg = if focused { theme.accent } else { theme.text };
                    (fit(&format!("{left}{title}{right}"), avail), fg)
                }
                None => (fit(env.no_envs_text(), avail), theme.danger),
            };
            frame.render_widget(
                Paragraph::new(text).style(Style::default().fg(color)),
                chunks.switcher,
            );
        }
    }

    frame.render_widget(
        Paragraph::new("User").style(label_style(matches!(input_mode, InputMode::Username))),
        chunks.label_username,
    );
    username
        .lock()
        .unwrap_or_else(|err| {
            error!("Failed to lock username. Reason: {}", err);
            std::process::exit(1);
        })
        .render(
            frame,
            chunks.username_field,
            matches!(input_mode, InputMode::Username),
        );

    frame.render_widget(
        Paragraph::new("Password").style(label_style(matches!(input_mode, InputMode::Password))),
        chunks.label_password,
    );
    password
        .lock()
        .unwrap_or_else(|err| {
            error!("Failed to lock password. Reason: {}", err);
            std::process::exit(1);
        })
        .render(
            frame,
            chunks.password_field,
            matches!(input_mode, InputMode::Password),
        );

    // Status line (centred, themed) + the power-control chip row.
    StatusMessage::render(
        status_message,
        frame,
        chunks.status_message,
        theme.danger,
        theme.muted,
    );
    key_menu.render(frame, chunks.key_menu, theme.accent);
}

#[cfg(test)]
mod render_tests {
    use super::*;
    use crate::config::Config;
    use ratatui::backend::TestBackend;

    /// Render one greeter frame to an in-memory buffer and return it as text,
    /// so the layout can be inspected/asserted without a real TTY. Two known
    /// sessions are injected so the session row is exercised independently of
    /// whatever the build host happens to have installed.
    fn render(w: u16, h: u16) -> String {
        crate::console_palette::init(true); // pass-through colours, no VT escapes
        let form = LoginForm::new(Config::default(), true);
        {
            let mut env = form.widgets.environment.lock().unwrap();
            *env = SwitcherWidget::new(
                vec![
                    SwitcherItem::new("Margo (UWSM)", PostLoginEnvironment::Shell),
                    SwitcherItem::new("Hyprland", PostLoginEnvironment::Shell),
                ],
                form.config.environment_switcher.clone(),
            );
        }
        let theme = Theme::from_config(&form.config);
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| {
            let chunks = Chunks::new(f);
            login_form_render(
                f,
                chunks,
                theme,
                form.widgets.background.clone(),
                form.widgets.key_menu.clone(),
                form.widgets.environment.clone(),
                form.widgets.username.clone(),
                form.widgets.password.clone(),
                InputMode::Password,
                None,
            );
        })
        .unwrap();
        let buf = term.backend().buffer();
        let mut s = String::new();
        for y in 0..h {
            for x in 0..w {
                s.push_str(&buf.get(x, y).symbol);
            }
            s.push('\n');
        }
        s
    }

    /// Across a wide range of terminal sizes (the bare VT can be much shorter
    /// *or* narrower than a terminal-emulator preview) the essentials must
    /// always be on screen: both F-keys, the credential labels, and the
    /// selected session name. This reproduced the "F-keys clipped / session
    /// vanishes at some resolutions" bug.
    #[test]
    fn essentials_survive_every_size() {
        for (w, h) in [
            (80, 24),
            (100, 30),
            (120, 40),
            (80, 20),
            (80, 18),
            (60, 16),
            (45, 22),
            (40, 12),
            (30, 24),
            (24, 20),
        ] {
            let out = render(w, h);
            eprintln!("\n===== {w}x{h} =====\n{out}");
            assert!(out.contains("F1"), "F1 missing at {w}x{h}\n{out}");
            assert!(out.contains("F2"), "F2 missing at {w}x{h}\n{out}");
            assert!(out.contains("F3"), "F3 missing at {w}x{h}\n{out}");
            assert!(
                out.contains("Password"),
                "Password label missing at {w}x{h}\n{out}"
            );
            assert!(
                out.contains("Margo"),
                "session name missing at {w}x{h}\n{out}"
            );
        }
    }
}
