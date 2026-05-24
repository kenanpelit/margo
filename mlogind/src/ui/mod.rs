use log::{error, info, warn};

use std::io;
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use crate::config::{Config, FocusBehaviour, SwitcherVisibility};
use crate::info_caching::{get_cached_information, set_cache};
use crate::post_login::PostLoginEnvironment;
use crate::{start_session, Hooks, StartSessionError};
use status_message::StatusMessage;

use crossterm::cursor::MoveTo;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen,
};
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
    DisableTui,
    EnableTui,
    StopDrawing,
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
    /// Whether the application is running in preview mode
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
                    config.system_shell.clone(),
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

    pub fn run(self, terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> io::Result<()> {
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

        std::thread::spawn(move || {
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

            let pre_auth = || {
                self.widgets.clear_password();

                status_message.set(InfoStatusMessage::Authenticating);
                send_ui_request(UIThreadRequest::Redraw);
            };
            let pre_environment = || {
                // Remember username and environment for next time
                self.set_cache();

                status_message.set(InfoStatusMessage::LoggingIn);
                send_ui_request(UIThreadRequest::Redraw);

                // Disable the rendering of the login manager
                send_ui_request(UIThreadRequest::DisableTui);
            };
            let pre_return = || {
                // Enable the rendering of the login manager
                send_ui_request(UIThreadRequest::EnableTui);

                status_message.clear();
                send_ui_request(UIThreadRequest::Redraw);
            };

            let hooks = Hooks {
                pre_validate: None,
                pre_auth: Some(&pre_auth),
                pre_environment: Some(&pre_environment),
                pre_wait: None,
                pre_return: Some(&pre_return),
            };

            loop {
                if let Ok(Event::Key(key)) = event::read() {
                    match (key.code, input_mode.get(), key.modifiers) {
                        (KeyCode::Enter, InputMode::Password, _) => {
                            if self.preview {
                                // This is only for demonstration purposes
                                status_message.set(InfoStatusMessage::Authenticating);
                                send_ui_request(UIThreadRequest::Redraw);
                                std::thread::sleep(Duration::from_secs(2));

                                status_message.set(InfoStatusMessage::LoggingIn);
                                send_ui_request(UIThreadRequest::Redraw);
                                std::thread::sleep(Duration::from_secs(2));

                                status_message.clear();
                                send_ui_request(UIThreadRequest::Redraw);
                            } else {
                                let environment =
                                    self.widgets.get_environment().map(|(_, content)| content);
                                let username = self.widgets.get_username();
                                let password = self.widgets.get_password();
                                let config = self.config.clone();

                                let Some(post_login_env) = environment else {
                                    status_message.set(ErrorStatusMessage::NoGraphicalEnvironment);
                                    send_ui_request(UIThreadRequest::Redraw);
                                    continue;
                                };

                                match start_session(
                                    &username,
                                    &password,
                                    &post_login_env,
                                    &hooks,
                                    &config,
                                ) {
                                    Ok(()) => {}
                                    Err(StartSessionError::AuthenticationError(err)) => {
                                        status_message
                                            .set(ErrorStatusMessage::AuthenticationError(err));
                                        send_ui_request(UIThreadRequest::Redraw);
                                    }
                                    Err(StartSessionError::ForkFailed) => {
                                        error!("Failed to fork session child process");
                                        send_ui_request(UIThreadRequest::EnableTui);

                                        status_message
                                            .set(ErrorStatusMessage::FailedGraphicalEnvironment);
                                        send_ui_request(UIThreadRequest::Redraw);
                                    }
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
                                req_send_channel.send(UIThreadRequest::StopDrawing).unwrap();
                            }
                        }

                        (KeyCode::Esc, _, _) => {
                            input_mode.set(InputMode::Normal);
                        }

                        (KeyCode::F(_), _, _) => {
                            self.widgets.key_menu.key_press(key.code);
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
        // This blocks until we actually call StopDrawing
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
                    disable_raw_mode()?;
                    execute!(
                        terminal.backend_mut(),
                        LeaveAlternateScreen,
                        Clear(ClearType::All),
                        MoveTo(0, 0)
                    )?;
                    terminal.show_cursor()?;
                }
                UIThreadRequest::EnableTui => {
                    enable_raw_mode()?;
                    let mut stdout = io::stdout();
                    execute!(stdout, EnterAlternateScreen)?;
                    terminal.clear()?;
                    tui_enabled.store(true, Ordering::Relaxed);
                }
                _ => break,
            }
        }

        Ok(())
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
    let label_style = |focused: bool| {
        Style::default().fg(if focused { theme.accent } else { theme.muted })
    };

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

    // The rounded accent card around the credentials — always accent, so the
    // theme reads even before the user types (mlock draws its border the same
    // way).
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
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
    StatusMessage::render(status_message, frame, chunks.status_message, theme.danger, theme.muted);
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
            assert!(out.contains("Password"), "Password label missing at {w}x{h}\n{out}");
            assert!(out.contains("Margo"), "session name missing at {w}x{h}\n{out}");
        }
    }
}
