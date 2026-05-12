use crate::app::{self, App};
use crate::components::{self, ButtonUIRef};
use crate::config::{AppearanceStyle, Position};
use crate::theme::{backdrop_color, use_theme};
use iced::alignment::Vertical;
use iced::widget::container::Style;
use iced::{
    Anchor, Border, Color, Element, KeyboardInteractivity, Layer, LayerShellSettings, Length,
    OutputId, Padding, Pixels, Shadow, SurfaceId, Task, Theme, Vector, destroy_layer_surface,
    new_layer_surface, set_keyboard_interactivity, widget::container,
};
use std::time::{Duration, Instant};

/// Menu open-animation duration in milliseconds.
pub const MENU_OPEN_ANIM_MS: u64 = 180;
/// Pixel distance the menu slides during open animation.
const MENU_SLIDE_PX: f32 = 12.0;
/// How far to backdate `open_at` so the very first frame isn't drawn
/// at opacity = 0. iced's subscription-driven 60 fps tick takes one
/// frame to arm, so naively starting at `Instant::now()` paints the
/// menu surface at α=0 once before the tick begins moving — visible
/// as a brief "blank flash" right before the fade. 30 ms of preroll
/// puts the first render at ~42% opacity (after ease-out-cubic), the
/// animation finishes ~150 ms later, and the perceptual flicker is
/// gone.
const MENU_OPEN_PREROLL_MS: u64 = 30;

fn ease_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

#[derive(Eq, PartialEq, Clone, Debug)]
pub enum MenuType {
    Updates,
    Settings,
    Notifications,
    Tray(String),
    MediaPlayer,
    SystemInfo,
    NetworkSpeed,
    Dns,
    Ufw,
    Power,
    Podman,
    Tempo,
}

#[derive(Clone, Debug)]
pub struct OpenMenu {
    pub id: SurfaceId,
    pub menu_type: MenuType,
    pub button_ui_ref: ButtonUIRef,
    /// Wall-clock instant of the `Menu::open` call. Drives the open
    /// fade+slide animation in `App::menu_wrapper`.
    pub open_at: Instant,
    /// `Some(t)` once `Menu::close` was requested; the layer surface
    /// is kept alive for `MENU_OPEN_ANIM_MS` after `t` so the menu
    /// can fade+slide back out before being destroyed. `None` means
    /// the menu is open (or still opening) and not closing.
    pub closing_at: Option<Instant>,
}

/// Compute the timestamp to seed `open_at` with so the open animation
/// is already a few frames in by the time the first paint hits the
/// screen. When the user has disabled animations (`theme.animations_enabled`),
/// shift the start back by the full animation length so `open_progress`
/// is already saturated at 1.0 — i.e. the menu renders instantly.
fn initial_open_at() -> Instant {
    let now = Instant::now();
    let preroll_ms = if use_theme(|t| t.animations_enabled) {
        MENU_OPEN_PREROLL_MS
    } else {
        MENU_OPEN_ANIM_MS + 16
    };
    now.checked_sub(Duration::from_millis(preroll_ms)).unwrap_or(now)
}

impl OpenMenu {
    fn open_progress(&self) -> f32 {
        let elapsed = self.open_at.elapsed().as_millis() as f32;
        (elapsed / MENU_OPEN_ANIM_MS as f32).clamp(0.0, 1.0)
    }

    fn close_progress(&self) -> f32 {
        match self.closing_at {
            None => 0.0,
            Some(t) => {
                let elapsed = t.elapsed().as_millis() as f32;
                (elapsed / MENU_OPEN_ANIM_MS as f32).clamp(0.0, 1.0)
            }
        }
    }

    /// 0 → 1 during the open fade. While closing, reverses to 0.
    pub fn opacity(&self) -> f32 {
        let open = ease_out_cubic(self.open_progress());
        let close = ease_out_cubic(self.close_progress());
        open * (1.0 - close)
    }

    /// Pixels the menu is offset from its resting position. Positive
    /// = slide away from the anchored edge (caller decides direction
    /// based on bar position). The slide reverses on close.
    pub fn slide_y(&self) -> f32 {
        // Same easing as opacity so the two stay phase-locked.
        MENU_SLIDE_PX * (1.0 - self.opacity())
    }

    /// True if the subscription/redraw loop should keep ticking
    /// because either:
    ///   • the open fade hasn't finished yet, or
    ///   • a close was requested and the surface hasn't been
    ///     destroyed yet (close_progress may already be ≥ 1.0 but
    ///     `finalize_close_if_done` only runs on the next tick).
    /// The latter case is critical — without it the subscription
    /// would die at `close_progress == 1.0` and the destroy tick
    /// would never fire, leaving the (transparent) menu surface
    /// stuck on screen.
    pub fn is_open_animating(&self) -> bool {
        self.open_progress() < 1.0 || self.closing_at.is_some()
    }

    /// True once the close animation has fully run out — the caller
    /// should destroy the layer surface and drop this `OpenMenu`.
    pub fn is_closing_complete(&self) -> bool {
        self.closing_at.is_some() && self.close_progress() >= 1.0
    }
}

#[derive(Clone, Debug)]
pub struct Menu {
    pub open: Option<OpenMenu>,
}

impl Menu {
    pub fn new() -> Self {
        Self { open: None }
    }

    pub fn surface_id(&self) -> Option<SurfaceId> {
        self.open.as_ref().map(|o| o.id)
    }

    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    pub fn open<Message: 'static>(
        &mut self,
        menu_type: MenuType,
        button_ui_ref: ButtonUIRef,
        request_keyboard: bool,
        output_id: Option<OutputId>,
    ) -> Task<Message> {
        let keyboard_interactivity = if request_keyboard {
            KeyboardInteractivity::OnDemand
        } else {
            KeyboardInteractivity::None
        };

        let (menu_id, task) = new_layer_surface(LayerShellSettings {
            namespace: "mshell-menu-layer".to_string(),
            size: None,
            layer: Layer::Overlay,
            keyboard_interactivity,
            output: output_id,
            anchor: Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
            ..Default::default()
        });

        self.open = Some(OpenMenu {
            id: menu_id,
            menu_type,
            button_ui_ref,
            open_at: initial_open_at(),
            closing_at: None,
        });
        task
    }

    /// Begin the close animation. The layer surface stays alive for
    /// `MENU_OPEN_ANIM_MS` after this call so the menu can fade+slide
    /// back out; the actual `destroy_layer_surface` happens later via
    /// `finalize_close_if_done`. Idempotent — if already closing, the
    /// `closing_at` timestamp is preserved.
    ///
    /// Keyboard focus is handed back **immediately**, before the
    /// close animation runs — otherwise pressing ESC over a menu
    /// would keep the user's typing trapped inside the dying menu
    /// surface for the full 180ms fade, and `maybe_release_all_keyboards`
    /// would skip the bar release because `menu_is_open()` still
    /// reports true while closing. This is the fix for "ESC closes
    /// the menu visually but the underlying window doesn't get
    /// focus back."
    pub fn close<Message: 'static>(&mut self) -> Task<Message> {
        if let Some(open) = self.open.as_mut()
            && open.closing_at.is_none()
        {
            let now = Instant::now();
            // When animations are disabled, backdate `closing_at` past
            // the close animation window so `is_closing_complete` flips
            // true on the very next tick and `finalize_close_if_done`
            // destroys the surface immediately.
            open.closing_at = Some(if use_theme(|t| t.animations_enabled) {
                now
            } else {
                now.checked_sub(Duration::from_millis(MENU_OPEN_ANIM_MS + 16))
                    .unwrap_or(now)
            });
            return set_keyboard_interactivity(open.id, KeyboardInteractivity::None);
        }
        Task::none()
    }

    /// Tick hook: drop the surface once its close animation has run
    /// out. Returns the `destroy_layer_surface` task and clears the
    /// `open` slot when triggered; otherwise `Task::none()`.
    pub fn finalize_close_if_done<Message: 'static>(&mut self) -> Task<Message> {
        if self
            .open
            .as_ref()
            .is_some_and(|om| om.is_closing_complete())
        {
            let open = self.open.take().expect("checked above");
            destroy_layer_surface(open.id)
        } else {
            Task::none()
        }
    }

    pub fn toggle<Message: 'static>(
        &mut self,
        menu_type: MenuType,
        button_ui_ref: ButtonUIRef,
        request_keyboard: bool,
        output_id: Option<OutputId>,
    ) -> Task<Message> {
        match &mut self.open {
            None => self.open(menu_type, button_ui_ref, request_keyboard, output_id),
            // Same menu, not already closing → close it.
            Some(open) if open.menu_type == menu_type && open.closing_at.is_none() => {
                self.close()
            }
            // Same menu re-clicked mid-close, or different menu swap:
            // cancel any pending close and replay the open animation
            // so the user sees a fresh fade-in.
            Some(open) => {
                open.menu_type = menu_type;
                open.button_ui_ref = button_ui_ref;
                open.open_at = initial_open_at();
                open.closing_at = None;
                Task::none()
            }
        }
    }

    pub fn close_if<Message: 'static>(&mut self, menu_type: MenuType) -> Task<Message> {
        if self.open.as_ref().is_some_and(|o| o.menu_type == menu_type) {
            self.close()
        } else {
            Task::none()
        }
    }

    pub fn request_keyboard<Message: 'static>(&self) -> Task<Message> {
        if let Some(open) = &self.open {
            set_keyboard_interactivity(open.id, KeyboardInteractivity::OnDemand)
        } else {
            Task::none()
        }
    }

    pub fn release_keyboard<Message: 'static>(&self) -> Task<Message> {
        if let Some(open) = &self.open {
            set_keyboard_interactivity(open.id, KeyboardInteractivity::None)
        } else {
            Task::none()
        }
    }
}

#[allow(unused)]
pub enum MenuSize {
    Small,
    Medium,
    Large,
    XLarge,
}

impl MenuSize {
    pub fn size(&self) -> f32 {
        match self {
            MenuSize::Small => 250.,
            MenuSize::Medium => 350.,
            MenuSize::Large => 450.,
            MenuSize::XLarge => 650.,
        }
    }
}

impl From<MenuSize> for Length {
    fn from(value: MenuSize) -> Self {
        Length::Fixed(value.size())
    }
}

impl From<MenuSize> for Pixels {
    fn from(value: MenuSize) -> Self {
        Pixels::from(value.size())
    }
}

impl App {
    /// Animation state for the open menu identified by `id`. Returns
    /// `(opacity, slide_y_px)` — `slide_y_px` is the distance the
    /// menu is offset from its resting position (positive = below
    /// resting, so we *add* to top padding on a top-anchored bar
    /// to slide the menu down into view).
    fn menu_anim_state(&self, id: SurfaceId) -> (f32, f32) {
        let opacity_slide = self
            .outputs
            .find_menu(id)
            .map(|om| (om.opacity(), om.slide_y()));
        opacity_slide.unwrap_or((1.0, 0.0))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn menu_wrapper<'a>(
        &'a self,
        id: SurfaceId,
        content: Element<'a, app::Message>,
        button_ui_ref: ButtonUIRef,
    ) -> Element<'a, app::Message> {
        let (space, menu_opacity, radius, bar_style, bar_position, menu_backdrop) =
            use_theme(|t| {
                (
                    t.space,
                    t.menu.opacity,
                    t.radius,
                    t.bar_style,
                    t.bar_position,
                    t.menu.backdrop,
                )
            });

        let (anim_alpha, slide_y) = self.menu_anim_state(id);
        let combined_alpha = menu_opacity * anim_alpha;
        let backdrop_alpha = anim_alpha;

        components::MenuWrapper::new(
            button_ui_ref.position.x,
            container(content)
                .padding(space.md)
                .style(move |theme: &Theme| Style {
                    background: Some(
                        theme.palette().background.scale_alpha(combined_alpha).into(),
                    ),
                    border: Border {
                        color: theme
                            .extended_palette()
                            .background
                            .weakest
                            .color
                            .scale_alpha(combined_alpha),
                        width: 1.,
                        radius: radius.lg.into(),
                    },
                    // Floating menu drop shadow — sits over the bar; the
                    // y-offset follows the bar position so the shadow
                    // always casts away from the anchored edge.
                    shadow: Shadow {
                        color: Color {
                            r: 0.,
                            g: 0.,
                            b: 0.,
                            a: 0.45 * anim_alpha,
                        },
                        offset: Vector::new(
                            0.,
                            match bar_position {
                                Position::Top => 8.,
                                Position::Bottom => -8.,
                            },
                        ),
                        blur_radius: 24.,
                    },
                    ..Default::default()
                })
                .width(Length::Shrink)
                .into(),
        )
        .padding({
            let v_padding = match bar_style {
                AppearanceStyle::Solid | AppearanceStyle::Gradient => 2,
                AppearanceStyle::Islands => 0,
            } as f32;

            // Slide-in: while opening, push the menu away from its
            // anchor by `slide_y` px. Animation eases this back to 0.
            Padding::new(0.)
                .top(if bar_position == Position::Top {
                    v_padding + slide_y
                } else {
                    0.0
                })
                .bottom(if bar_position == Position::Bottom {
                    v_padding + slide_y
                } else {
                    0.0
                })
        })
        .align_y(match bar_position {
            Position::Top => Vertical::Top,
            Position::Bottom => Vertical::Bottom,
        })
        .backdrop(backdrop_color(menu_backdrop).scale_alpha(backdrop_alpha))
        .on_click_outside(app::Message::CloseMenu(id))
        .into()
    }
}
