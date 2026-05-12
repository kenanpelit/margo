//! `mshell lock` — wlr-layer-shell Overlay + Exclusive keyboard ile
//! ekranı kilitler. Hyprlock-tarzı tasarım: ortada büyük saat + tarih
//! + password input. Enter ile `pam` üzerinden auth, başarılı olursa
//! process exit eder; başarısız olursa input temizlenir + "Yanlış
//! parola" mesajı 1.5 sn görünür.
//!
//! Bu ext-session-lock-v1 değil — sadece overlay layer. Kötü niyetli
//! bir başka client Layer::Overlay'e başka surface yapıştıramaz ama
//! TTY (Ctrl-Alt-F2) hâlâ açık. Tam izolasyon için sonra
//! ext-session-lock'a port edilecek.

pub mod pam;

use chrono::Local;
use iced::{
    Alignment, Anchor, Color, Element, Font, KeyboardInteractivity, Layer, LayerShellSettings,
    Length, Subscription, SurfaceId, Task, Theme,
    widget::{Column, Space, container, text, text_input},
};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const FONT_CLOCK: f32 = 96.0;
const FONT_DATE: f32 = 22.0;
const FONT_USER: f32 = 18.0;
const FONT_STATUS: f32 = 14.0;
const FONT_INPUT: f32 = 16.0;

#[derive(Debug, Clone)]
pub enum Message {
    PasswordChanged(String),
    Submit,
    AuthResult(bool),
    Tick,
}

pub struct LockState {
    user: String,
    password: String,
    busy: Arc<AtomicBool>,
    fail_message: Option<String>,
    fail_clear_at: Option<std::time::Instant>,
    now: chrono::DateTime<Local>,
}

fn boot() -> (LockState, Task<Message>) {
    let user = pam::current_user().unwrap_or_else(|| "user".to_string());
    let state = LockState {
        user,
        password: String::new(),
        busy: Arc::new(AtomicBool::new(false)),
        fail_message: None,
        fail_clear_at: None,
        now: Local::now(),
    };
    (state, Task::none())
}

fn update(state: &mut LockState, message: Message) -> Task<Message> {
    match message {
        Message::PasswordChanged(s) => {
            // Yeni karakter yazılınca eski fail mesajını sil.
            if state.fail_message.is_some() {
                state.fail_message = None;
                state.fail_clear_at = None;
            }
            state.password = s;
            Task::none()
        }
        Message::Submit => {
            if state.busy.load(Ordering::Relaxed) || state.password.is_empty() {
                return Task::none();
            }
            state.busy.store(true, Ordering::Relaxed);
            let user = state.user.clone();
            let password = std::mem::take(&mut state.password);
            let busy = state.busy.clone();
            Task::perform(
                async move {
                    // PAM blocks (shadow read, hash compute) — push to
                    // a blocking task so iced's event loop stays
                    // responsive (clock keeps ticking, etc.).
                    let res = tokio::task::spawn_blocking(move || {
                        pam::authenticate(pam::SERVICE_LOGIN, &user, &password).is_ok()
                    })
                    .await
                    .unwrap_or(false);
                    busy.store(false, Ordering::Relaxed);
                    res
                },
                Message::AuthResult,
            )
        }
        Message::AuthResult(true) => {
            // Auth succeeded — exit the process so the layer surface
            // disappears and the session continues. iced doesn't have
            // a clean "quit" API for layer-shell apps in 0.13.
            std::process::exit(0);
        }
        Message::AuthResult(false) => {
            state.fail_message = Some("Yanlış parola".to_string());
            state.fail_clear_at = Some(std::time::Instant::now() + Duration::from_millis(1500));
            Task::none()
        }
        Message::Tick => {
            state.now = Local::now();
            // Clear stale fail message once its TTL expires.
            if let Some(t) = state.fail_clear_at
                && std::time::Instant::now() >= t
            {
                state.fail_message = None;
                state.fail_clear_at = None;
            }
            Task::none()
        }
    }
}

fn view(state: &LockState, _id: SurfaceId) -> Element<'_, Message> {
    let clock_str = state.now.format("%H:%M").to_string();
    let date_str = state.now.format("%A, %-d %B %Y").to_string();

    let clock =
        container(text(clock_str).size(FONT_CLOCK).font(Font::default())).center_x(Length::Fill);
    let date = container(
        text(date_str)
            .size(FONT_DATE)
            .style(|theme: &Theme| iced::widget::text::Style {
                color: Some(theme.extended_palette().background.weak.text),
            }),
    )
    .center_x(Length::Fill);

    let user_label = container(
        text(format!("@ {}", state.user))
            .size(FONT_USER)
            .style(|theme: &Theme| iced::widget::text::Style {
                color: Some(theme.extended_palette().background.weak.text),
            }),
    )
    .center_x(Length::Fill);

    let input = text_input("…", &state.password)
        .on_input(Message::PasswordChanged)
        .on_submit(Message::Submit)
        .secure(true)
        .padding(12)
        .size(FONT_INPUT)
        .width(Length::Fixed(320.0));

    let input_container = container(input).center_x(Length::Fill);

    let status: Element<'_, Message> = match (
        &state.fail_message,
        state.busy.load(Ordering::Relaxed),
    ) {
        (Some(msg), _) => container(
            text(msg.clone())
                .size(FONT_STATUS)
                .style(|theme: &Theme| iced::widget::text::Style {
                    color: Some(theme.palette().danger),
                }),
        )
        .center_x(Length::Fill)
        .into(),
        (_, true) => container(
            text("Doğrulanıyor…")
                .size(FONT_STATUS)
                .style(|theme: &Theme| iced::widget::text::Style {
                    color: Some(theme.extended_palette().background.weak.text),
                }),
        )
        .center_x(Length::Fill)
        .into(),
        _ => Space::new()
            .height(Length::Fixed(FONT_STATUS + 4.0))
            .into(),
    };

    let center: Column<'_, Message> = Column::new()
        .push(clock)
        .push(date)
        .push(Space::new().height(Length::Fixed(48.0)))
        .push(user_label)
        .push(Space::new().height(Length::Fixed(12.0)))
        .push(input_container)
        .push(Space::new().height(Length::Fixed(8.0)))
        .push(status)
        .align_x(Alignment::Center)
        .spacing(8);

    container(center)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .style(|_theme: &Theme| container::Style {
            background: Some(Color::from_rgba(0.05, 0.05, 0.10, 0.95).into()),
            ..Default::default()
        })
        .into()
}

fn subscription(_state: &LockState) -> Subscription<Message> {
    iced::time::every(Duration::from_millis(500)).map(|_| Message::Tick)
}

fn theme(_state: &LockState) -> Theme {
    Theme::CatppuccinMocha
}

/// Run the lock screen. Blocks until auth succeeds (process::exit(0))
/// or the user kills the process.
pub fn run() -> iced::Result {
    iced::application(boot, update, view)
        .layer_shell(LayerShellSettings {
            anchor: Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
            layer: Layer::Overlay,
            exclusive_zone: -1, // ignore exclusive zones — cover the bar too.
            size: None,         // 0 size = fill the output (with all anchors set).
            keyboard_interactivity: KeyboardInteractivity::Exclusive,
            namespace: "mshell-lockscreen".into(),
            ..Default::default()
        })
        .subscription(subscription)
        .theme(theme)
        .run()
}
