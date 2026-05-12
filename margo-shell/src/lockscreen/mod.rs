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
    widget::{Column, Space, container, image, stack, text, text_input},
};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const FONT_CLOCK: f32 = 96.0;
const FONT_DATE: f32 = 22.0;
const FONT_USER: f32 = 18.0;
const FONT_STATUS: f32 = 14.0;
const FONT_INPUT: f32 = 16.0;
const PWD_ID: &str = "lockscreen-password";
/// Sigma for the wallpaper blur. Higher = more blur. ~15 px sigma at
/// 1080p gives a clean frosted-glass look; we resize to 720p first to
/// keep blur compute under ~100 ms.
const BLUR_SIGMA: f32 = 18.0;
/// Internal resize target (long edge) before blur. Smaller = faster.
const BLUR_RESIZE_LONG_EDGE: u32 = 1280;

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
    /// Blurred wallpaper as an iced image handle. `None` when no
    /// wallpaper path was resolved or the decode failed.
    backdrop: Option<image::Handle>,
    /// `true` until we've sent the first focus task to the password
    /// input. boot()'s task fires before the widget tree exists, so
    /// we defer to the first Tick (when iced has rendered at least
    /// once and the widget Id is known).
    needs_focus: bool,
}

fn boot() -> (LockState, Task<Message>) {
    let user = pam::current_user().unwrap_or_else(|| "user".to_string());
    let backdrop = build_backdrop();
    let state = LockState {
        user,
        password: String::new(),
        busy: Arc::new(AtomicBool::new(false)),
        fail_message: None,
        fail_clear_at: None,
        now: Local::now(),
        backdrop,
        needs_focus: true,
    };
    // Don't focus from boot() — the widget tree isn't built yet,
    // so the focus operation no-ops. The first Tick handler picks
    // this up once iced has rendered at least one frame.
    (state, Task::none())
}

fn update(state: &mut LockState, message: Message) -> Task<Message> {
    match message {
        Message::PasswordChanged(s) => {
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
            std::process::exit(0);
        }
        Message::AuthResult(false) => {
            state.fail_message = Some("Yanlış parola".to_string());
            state.fail_clear_at = Some(std::time::Instant::now() + Duration::from_millis(1500));
            // Re-focus the input so the user can immediately retype.
            Task::Iced(iced_runtime::widget::operation::focus(iced::widget::Id::new(PWD_ID)))
        }
        Message::Tick => {
            state.now = Local::now();
            if let Some(t) = state.fail_clear_at
                && std::time::Instant::now() >= t
            {
                state.fail_message = None;
                state.fail_clear_at = None;
            }
            if state.needs_focus {
                state.needs_focus = false;
                // First Tick — widget tree has been built at least once,
                // so the focus operation can now find the password input.
                return Task::Iced(iced_runtime::widget::operation::focus(
                    iced::widget::Id::new(PWD_ID),
                ));
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
        .id(iced::widget::Id::new(PWD_ID))
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

    let foreground = container(center)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .style(|_theme: &Theme| container::Style {
            // Dim the wallpaper backdrop so foreground stays readable.
            background: Some(Color::from_rgba(0.05, 0.05, 0.10, 0.55).into()),
            ..Default::default()
        });

    if let Some(handle) = state.backdrop.as_ref() {
        // Stack: blurred wallpaper at the bottom, dim+content on top.
        stack![
            image(handle.clone())
                .width(Length::Fill)
                .height(Length::Fill)
                .content_fit(iced::ContentFit::Cover),
            foreground,
        ]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    } else {
        // No wallpaper available — fall back to opaque dark
        // background (95% alpha, same as the original MVP).
        container(center_no_backdrop(state))
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .style(|_theme: &Theme| container::Style {
                background: Some(Color::from_rgba(0.05, 0.05, 0.10, 0.95).into()),
                ..Default::default()
            })
            .into()
    }
}

/// `view()` builds the centered content twice (once for the stacked
/// + dimmed path, once for the wallpaper-less path). This helper
/// returns just the central Column so both code paths share it.
/// Kept close to `view` so any layout edits stay in sync.
fn center_no_backdrop(state: &LockState) -> Element<'_, Message> {
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
        .id(iced::widget::Id::new(PWD_ID))
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

    Column::new()
        .push(clock)
        .push(date)
        .push(Space::new().height(Length::Fixed(48.0)))
        .push(user_label)
        .push(Space::new().height(Length::Fixed(12.0)))
        .push(input_container)
        .push(Space::new().height(Length::Fixed(8.0)))
        .push(status)
        .align_x(Alignment::Center)
        .spacing(8)
        .into()
}

fn subscription(_state: &LockState) -> Subscription<Message> {
    // 50 ms tick — drives the first-frame focus retry plus the
    // visible clock. Once focus settles the cost is negligible
    // (single now() + no-op update).
    iced::time::every(Duration::from_millis(50)).map(|_| Message::Tick)
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
            exclusive_zone: -1,
            size: None,
            keyboard_interactivity: KeyboardInteractivity::Exclusive,
            namespace: "mshell-lockscreen".into(),
            ..Default::default()
        })
        .subscription(subscription)
        .theme(theme)
        .run()
}

// ── wallpaper backdrop ───────────────────────────────────────────────

/// Resolve the active output's wallpaper path the same way
/// `matugen::resolve_active_wallpaper` does, then load + blur it.
/// On any failure returns None — the caller falls back to a solid
/// dark backdrop.
fn build_backdrop() -> Option<image::Handle> {
    let path = active_wallpaper_path()?;
    let bytes = std::fs::read(&path).ok()?;
    let img = ::image::load_from_memory(&bytes).ok()?;

    // Resize to a sane working size before blur. blur() is roughly
    // O(w·h·sigma); 1280-px-long edge keeps it under ~100 ms.
    let (w, h) = (img.width(), img.height());
    let scale = (BLUR_RESIZE_LONG_EDGE as f32 / w.max(h) as f32).min(1.0);
    let work = if scale < 1.0 {
        img.resize(
            (w as f32 * scale) as u32,
            (h as f32 * scale) as u32,
            ::image::imageops::FilterType::Lanczos3,
        )
    } else {
        img
    };

    let blurred = work.blur(BLUR_SIGMA);
    let rgba = blurred.to_rgba8();
    let (bw, bh) = (rgba.width(), rgba.height());
    Some(image::Handle::from_rgba(bw, bh, rgba.into_raw()))
}

fn active_wallpaper_path() -> Option<std::path::PathBuf> {
    // Mirror matugen's lookup: state.json active output → tag mask →
    // mshell.toml [wallpaper.tags] key. Kept as a separate helper
    // because matugen's version returns Result + we just want Option.
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            let uid = unsafe { libc::getuid() };
            std::path::PathBuf::from(format!("/run/user/{uid}"))
        });
    let state_path = runtime.join("margo").join("state.json");
    let raw = std::fs::read(&state_path).ok()?;
    let state: serde_json::Value = serde_json::from_slice(&raw).ok()?;
    let outputs = state.get("outputs")?.as_array()?;
    let active = outputs
        .iter()
        .find(|o| o.get("active").and_then(|v| v.as_bool()).unwrap_or(false))?;

    // Prefer state.json's own wallpaper (margo tagrule) if non-empty.
    if let Some(p) = active
        .get("wallpaper")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        return Some(expand_home(p));
    }

    let mask = active.get("active_tag_mask")?.as_u64()?;
    if mask == 0 {
        return None;
    }
    let tag = (mask as u32).trailing_zeros() + 1;

    let (cfg, _) = crate::config::get_config(None).ok()?;
    let raw = cfg.wallpaper.tags.get(&tag.to_string())?;
    Some(expand_home(raw))
}

fn expand_home(p: &str) -> std::path::PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        let home = std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from("/"));
        home.join(rest)
    } else {
        std::path::PathBuf::from(p)
    }
}
