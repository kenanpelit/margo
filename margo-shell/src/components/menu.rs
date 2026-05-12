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
use std::time::Instant;

/// Menu open-animation duration in milliseconds.
pub const MENU_OPEN_ANIM_MS: u64 = 180;
/// Pixel distance the menu slides during open animation.
const MENU_SLIDE_PX: f32 = 12.0;

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
    /// Wall-clock instant of the `Menu::open` call. Used to drive
    /// the open fade+slide animation in `App::menu_wrapper`.
    pub open_at: Instant,
}

impl OpenMenu {
    fn open_progress(&self) -> f32 {
        let elapsed = self.open_at.elapsed().as_millis() as f32;
        (elapsed / MENU_OPEN_ANIM_MS as f32).clamp(0.0, 1.0)
    }

    /// 0.0 right after open → 1.0 once the fade completes.
    pub fn opacity(&self) -> f32 {
        ease_out_cubic(self.open_progress())
    }

    /// Pixels the menu is offset from its resting position. Positive
    /// = slide down from above (the caller flips the sign for a
    /// bottom-anchored bar).
    pub fn slide_y(&self) -> f32 {
        MENU_SLIDE_PX * (1.0 - self.opacity())
    }

    pub fn is_open_animating(&self) -> bool {
        self.open_progress() < 1.0
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
            open_at: Instant::now(),
        });
        task
    }

    pub fn close<Message: 'static>(&mut self) -> Task<Message> {
        if let Some(open) = self.open.take() {
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
            Some(open) if open.menu_type == menu_type => self.close(),
            Some(open) => {
                open.menu_type = menu_type;
                open.button_ui_ref = button_ui_ref;
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
