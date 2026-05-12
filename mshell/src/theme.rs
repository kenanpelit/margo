use std::cell::RefCell;

use crate::{
    components::button::{ButtonHierarchy, ButtonKind},
    config::{
        Appearance, AppearanceColor, AppearanceStyle, BackgroundLevel, MenuAppearance, Position,
    },
};
use iced::{
    Background, Border, Color, Shadow, Theme, Vector,
    theme::{Palette, palette},
    widget::{
        button::{self, Status},
        container,
        text_input::{self},
    },
};

thread_local! {
    pub static THEME: RefCell<MshellTheme> =  RefCell::new(MshellTheme::default());
}

pub fn init_theme(theme: MshellTheme) {
    THEME.replace(theme);
}

pub fn use_theme<R, F: FnOnce(&MshellTheme) -> R>(f: F) -> R {
    THEME.with_borrow(f)
}

#[allow(unused)]
#[derive(Debug, Copy, Clone)]
pub struct Space {
    pub xxs: f32,
    pub xs: f32,
    pub sm: f32,
    pub md: f32,
    pub lg: f32,
    pub xl: f32,
    pub xxl: f32,
}

impl Default for Space {
    // Modüller arasında nefes alabilen ama bar'ı boğmayacak ölçek.
    // Noctalia (1/2/4/6/9/13/18) çok sıkı geliyordu, eski iced default
    // (4/8/12/16/24/32/48) ise çok gevşekti — bu ikisinin ortası.
    fn default() -> Self {
        Self {
            xxs: 3.0,
            xs: 6.0,
            sm: 10.0,
            md: 14.0,
            lg: 20.0,
            xl: 28.0,
            xxl: 40.0,
        }
    }
}

#[allow(unused)]
#[derive(Debug, Clone, Copy)]
pub struct Radius {
    pub sm: f32,
    pub md: f32,
    pub lg: f32,
    pub xl: f32,
}

impl Default for Radius {
    // Eski iced default'lar — Islands capsule'leri için daha yuvarlak
    // his. lg=16 capsule kenarlarını noctalia'dan biraz daha yumuşak
    // gösterir (kullanıcı tercihi).
    fn default() -> Self {
        Self {
            sm: 4.0,
            md: 8.0,
            lg: 16.0,
            xl: 32.0,
        }
    }
}

#[allow(unused)]
#[derive(Debug, Copy, Clone)]
pub struct FontSize {
    pub xxs: f32,
    pub xs: f32,
    pub sm: f32,
    pub md: f32,
    pub lg: f32,
    pub xl: f32,
    pub xxl: f32,
}

impl Default for FontSize {
    // Eski iced ölçeği — bar metinleri yeterince büyük, başlıklar
    // okunaklı. Noctalia 10/11/13 fazla küçük geliyordu.
    fn default() -> Self {
        Self {
            xxs: 8.0,
            xs: 10.0,
            sm: 12.0,
            md: 16.0,
            lg: 20.0,
            xl: 22.0,
            xxl: 32.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct MshellTheme {
    pub iced_theme: Theme,
    pub space: Space,
    pub radius: Radius,
    pub font_size: FontSize,
    pub bar_position: Position,
    pub bar_style: AppearanceStyle,
    pub opacity: f32,
    pub menu: MenuAppearance,
    pub workspace_colors: Vec<AppearanceColor>,
    pub special_workspace_colors: Option<Vec<AppearanceColor>>,
    pub scale_factor: f64,
    /// Bar yüksekliği (ham piksel; scale uygulamadan önce). Eski sabit
    /// HEIGHT 34 yerine artık config'ten gelir.
    pub bar_height: f64,
    /// Islands modunda her capsule'a ince outline çiz.
    pub show_outline: bool,
    /// Bar üzerindeki tüm modüllerin metin boyutu — font birliği için.
    /// Tek noktadan ayarlanır, her modülün view() fonksiyonu bunu kullanır.
    /// Varsayılan `font_size.sm` (12).
    pub bar_font_size: f32,
    // Read by animation call sites added in subsequent PRs.
    #[allow(dead_code)]
    pub animations_enabled: bool,
}

impl Default for MshellTheme {
    fn default() -> Self {
        let appearance = Appearance::default();

        MshellTheme {
            space: Space::default(),
            radius: Radius::default(),
            font_size: FontSize::default(),
            bar_position: Position::default(),
            bar_style: appearance.style,
            opacity: appearance.opacity,
            menu: appearance.menu,
            workspace_colors: appearance.workspace_colors.clone(),
            special_workspace_colors: appearance.special_workspace_colors.clone(),
            scale_factor: appearance.scale_factor,
            bar_height: appearance.bar_density.height(),
            show_outline: appearance.show_outline,
            bar_font_size: FontSize::default().sm,
            animations_enabled: false,
            iced_theme: Theme::custom_with_fn(
                "local".to_string(),
                Palette {
                    background: appearance.background_color.get_base(),
                    text: appearance.text_color.get_base(),
                    primary: appearance.primary_color.get_base(),
                    success: appearance.success_color.get_base(),
                    warning: appearance.warning_color.get_base(),
                    danger: appearance.danger_color.get_base(),
                },
                |palette| {
                    let text = palette.text;
                    let bg_text = appearance.background_color.get_text().unwrap_or(text);

                    let default_bg = palette::Background::new(palette.background, bg_text);
                    let bg = |level, fallback| {
                        appearance
                            .background_color
                            .get_pair(level, text)
                            .unwrap_or(fallback)
                    };

                    let default_primary = palette::Primary::generate(
                        palette.primary,
                        palette.background,
                        appearance.primary_color.get_text().unwrap_or(text),
                    );
                    let default_success = palette::Success::generate(
                        palette.success,
                        palette.background,
                        appearance.success_color.get_text().unwrap_or(text),
                    );
                    let default_warning = palette::Warning::generate(
                        palette.warning,
                        palette.background,
                        appearance.warning_color.get_text().unwrap_or(text),
                    );
                    let default_danger = palette::Danger::generate(
                        palette.danger,
                        palette.background,
                        appearance.danger_color.get_text().unwrap_or(text),
                    );

                    palette::Extended {
                        background: palette::Background {
                            base: default_bg.base,
                            weakest: bg(BackgroundLevel::Weakest, default_bg.weakest),
                            weaker: bg(BackgroundLevel::Weaker, default_bg.weaker),
                            weak: bg(BackgroundLevel::Weak, default_bg.weak),
                            neutral: bg(BackgroundLevel::Neutral, default_bg.neutral),
                            strong: bg(BackgroundLevel::Strong, default_bg.strong),
                            stronger: bg(BackgroundLevel::Stronger, default_bg.stronger),
                            strongest: bg(BackgroundLevel::Strongest, default_bg.strongest),
                        },
                        primary: palette::Primary {
                            base: default_primary.base,
                            weak: appearance
                                .primary_color
                                .get_weak_pair(text)
                                .unwrap_or(default_primary.weak),
                            strong: appearance
                                .primary_color
                                .get_strong_pair(text)
                                .unwrap_or(default_primary.strong),
                        },
                        secondary: palette::Secondary::generate(palette.background, text),
                        success: palette::Success {
                            base: default_success.base,
                            weak: appearance
                                .success_color
                                .get_weak_pair(text)
                                .unwrap_or(default_success.weak),
                            strong: appearance
                                .success_color
                                .get_strong_pair(text)
                                .unwrap_or(default_success.strong),
                        },
                        warning: palette::Warning {
                            base: default_warning.base,
                            weak: appearance
                                .warning_color
                                .get_weak_pair(text)
                                .unwrap_or(default_warning.weak),
                            strong: appearance
                                .warning_color
                                .get_strong_pair(text)
                                .unwrap_or(default_warning.strong),
                        },
                        danger: palette::Danger {
                            base: default_danger.base,
                            weak: appearance
                                .danger_color
                                .get_weak_pair(text)
                                .unwrap_or(default_danger.weak),
                            strong: appearance
                                .danger_color
                                .get_strong_pair(text)
                                .unwrap_or(default_danger.strong),
                        },
                        is_dark: true,
                    }
                },
            ),
        }
    }
}

impl MshellTheme {
    pub fn new(
        position: Position,
        appearance: &Appearance,
        animations: &crate::config::AnimationsConfig,
    ) -> Self {
        MshellTheme {
            animations_enabled: animations.enabled,
            space: Space::default(),
            radius: Radius::default(),
            font_size: FontSize::default(),
            bar_position: position,
            bar_style: appearance.style,
            opacity: appearance.opacity,
            menu: appearance.menu,
            workspace_colors: appearance.workspace_colors.clone(),
            special_workspace_colors: appearance.special_workspace_colors.clone(),
            scale_factor: appearance.scale_factor,
            bar_height: appearance.bar_density.height(),
            show_outline: appearance.show_outline,
            bar_font_size: FontSize::default().sm,
            iced_theme: Theme::custom_with_fn(
                "local".to_string(),
                Palette {
                    background: appearance.background_color.get_base(),
                    text: appearance.text_color.get_base(),
                    primary: appearance.primary_color.get_base(),
                    success: appearance.success_color.get_base(),
                    warning: appearance.warning_color.get_base(),
                    danger: appearance.danger_color.get_base(),
                },
                |palette| {
                    let text = palette.text;
                    let bg_text = appearance.background_color.get_text().unwrap_or(text);

                    let default_bg = palette::Background::new(palette.background, bg_text);
                    let bg = |level, fallback| {
                        appearance
                            .background_color
                            .get_pair(level, text)
                            .unwrap_or(fallback)
                    };

                    let default_primary = palette::Primary::generate(
                        palette.primary,
                        palette.background,
                        appearance.primary_color.get_text().unwrap_or(text),
                    );
                    let default_success = palette::Success::generate(
                        palette.success,
                        palette.background,
                        appearance.success_color.get_text().unwrap_or(text),
                    );
                    let default_warning = palette::Warning::generate(
                        palette.warning,
                        palette.background,
                        appearance.warning_color.get_text().unwrap_or(text),
                    );
                    let default_danger = palette::Danger::generate(
                        palette.danger,
                        palette.background,
                        appearance.danger_color.get_text().unwrap_or(text),
                    );

                    palette::Extended {
                        background: palette::Background {
                            base: default_bg.base,
                            weakest: bg(BackgroundLevel::Weakest, default_bg.weakest),
                            weaker: bg(BackgroundLevel::Weaker, default_bg.weaker),
                            weak: bg(BackgroundLevel::Weak, default_bg.weak),
                            neutral: bg(BackgroundLevel::Neutral, default_bg.neutral),
                            strong: bg(BackgroundLevel::Strong, default_bg.strong),
                            stronger: bg(BackgroundLevel::Stronger, default_bg.stronger),
                            strongest: bg(BackgroundLevel::Strongest, default_bg.strongest),
                        },
                        primary: palette::Primary {
                            base: default_primary.base,
                            weak: appearance
                                .primary_color
                                .get_weak_pair(text)
                                .unwrap_or(default_primary.weak),
                            strong: appearance
                                .primary_color
                                .get_strong_pair(text)
                                .unwrap_or(default_primary.strong),
                        },
                        secondary: palette::Secondary::generate(palette.background, text),
                        success: palette::Success {
                            base: default_success.base,
                            weak: appearance
                                .success_color
                                .get_weak_pair(text)
                                .unwrap_or(default_success.weak),
                            strong: appearance
                                .success_color
                                .get_strong_pair(text)
                                .unwrap_or(default_success.strong),
                        },
                        warning: palette::Warning {
                            base: default_warning.base,
                            weak: appearance
                                .warning_color
                                .get_weak_pair(text)
                                .unwrap_or(default_warning.weak),
                            strong: appearance
                                .warning_color
                                .get_strong_pair(text)
                                .unwrap_or(default_warning.strong),
                        },
                        danger: palette::Danger {
                            base: default_danger.base,
                            weak: appearance
                                .danger_color
                                .get_weak_pair(text)
                                .unwrap_or(default_danger.weak),
                            strong: appearance
                                .danger_color
                                .get_strong_pair(text)
                                .unwrap_or(default_danger.strong),
                        },
                        is_dark: true,
                    }
                },
            ),
        }
    }

    pub fn button_style(
        &self,
        kind: ButtonKind,
        hierarchy: ButtonHierarchy,
    ) -> impl Fn(&Theme, Status) -> button::Style + use<> {
        let radius = match kind {
            ButtonKind::Transparent => self.radius.sm,
            ButtonKind::Solid | ButtonKind::Outline => self.radius.xl,
        };
        let opacity = self.opacity;

        move |theme: &Theme, status: Status| {
            let palette = theme.palette();
            let ext = theme.extended_palette();

            let (base_bg, hover_bg, base_text, hover_text, border_color) = match hierarchy {
                ButtonHierarchy::Primary => (
                    palette.primary,
                    ext.primary.weak.color,
                    ext.primary.base.text,
                    ext.primary.base.text,
                    palette.primary,
                ),
                ButtonHierarchy::Secondary => (
                    ext.background.weak.color,
                    ext.background.strong.color,
                    palette.text,
                    palette.text,
                    ext.background.weak.color,
                ),
                ButtonHierarchy::Danger => (
                    palette.danger,
                    ext.danger.weak.color,
                    ext.danger.base.text,
                    ext.danger.base.text,
                    palette.danger,
                ),
            };

            match (kind, status) {
                (ButtonKind::Solid, Status::Active) => button::Style {
                    background: Some(base_bg.scale_alpha(opacity).into()),
                    border: Border {
                        width: 0.0,
                        radius: radius.into(),
                        color: Color::TRANSPARENT,
                    },
                    text_color: base_text,
                    ..button::Style::default()
                },
                (ButtonKind::Solid, Status::Hovered) => button::Style {
                    background: Some(hover_bg.scale_alpha(opacity).into()),
                    border: Border {
                        width: 0.0,
                        radius: radius.into(),
                        color: Color::TRANSPARENT,
                    },
                    text_color: hover_text,
                    ..button::Style::default()
                },

                (ButtonKind::Transparent, Status::Active) => button::Style {
                    background: None,
                    border: Border {
                        width: 0.0,
                        radius: radius.into(),
                        color: Color::TRANSPARENT,
                    },
                    text_color: palette.text,
                    ..button::Style::default()
                },
                (ButtonKind::Transparent, Status::Hovered) => button::Style {
                    background: Some(
                        theme
                            .extended_palette()
                            .background
                            .base
                            .text
                            .scale_alpha(0.04)
                            .into(),
                    ),
                    border: Border {
                        width: 0.0,
                        radius: radius.into(),
                        color: Color::TRANSPARENT,
                    },
                    text_color: match hierarchy {
                        ButtonHierarchy::Danger => palette.danger,
                        ButtonHierarchy::Primary => palette.primary,
                        ButtonHierarchy::Secondary => palette.text,
                    },
                    ..button::Style::default()
                },

                (ButtonKind::Outline, Status::Active) => button::Style {
                    background: None,
                    border: Border {
                        width: 2.0,
                        radius: radius.into(),
                        color: border_color,
                    },
                    text_color: palette.text,
                    ..button::Style::default()
                },
                (ButtonKind::Outline, Status::Hovered) => button::Style {
                    background: Some(base_bg.scale_alpha(opacity).into()),
                    border: Border {
                        width: 2.0,
                        radius: radius.into(),
                        color: border_color,
                    },
                    text_color: palette.text,
                    ..button::Style::default()
                },

                (kind, Status::Disabled) => {
                    let disabled_opacity = 0.3;
                    match kind {
                        ButtonKind::Solid => button::Style {
                            background: Some(
                                base_bg.scale_alpha(opacity * disabled_opacity).into(),
                            ),
                            border: Border {
                                width: 0.0,
                                radius: radius.into(),
                                color: Color::TRANSPARENT,
                            },
                            text_color: base_text.scale_alpha(0.5),
                            ..button::Style::default()
                        },
                        ButtonKind::Transparent => button::Style {
                            background: None,
                            border: Border {
                                width: 0.0,
                                radius: radius.into(),
                                color: Color::TRANSPARENT,
                            },
                            text_color: palette.text.scale_alpha(disabled_opacity),
                            ..button::Style::default()
                        },
                        ButtonKind::Outline => button::Style {
                            background: None,
                            border: Border {
                                width: 2.0,
                                radius: radius.into(),
                                color: border_color.scale_alpha(disabled_opacity),
                            },
                            text_color: palette.text.scale_alpha(disabled_opacity),
                            ..button::Style::default()
                        },
                    }
                }

                _ => button::Style {
                    background: None,
                    border: Border {
                        width: 0.0,
                        radius: radius.into(),
                        color: Color::TRANSPARENT,
                    },
                    text_color: palette.text,
                    ..button::Style::default()
                },
            }
        }
    }

    pub fn quick_settings_submenu_button_style(
        &self,
        is_active: bool,
    ) -> impl Fn(&Theme, Status) -> button::Style + use<> {
        let radius_lg = self.radius.lg;
        let opacity = self.opacity;
        move |theme: &Theme, status: Status| {
            let mut base = button::Style {
                background: None,
                border: Border {
                    width: 0.0,
                    radius: radius_lg.into(),
                    color: Color::TRANSPARENT,
                },
                text_color: if is_active {
                    theme.extended_palette().primary.base.text
                } else {
                    theme.palette().text
                },
                ..button::Style::default()
            };
            match status {
                Status::Active => base,
                Status::Hovered => {
                    base.background = Some(
                        theme
                            .extended_palette()
                            .background
                            .weak
                            .color
                            .scale_alpha(opacity)
                            .into(),
                    );
                    base.text_color = theme.palette().text;
                    base
                }
                _ => base,
            }
        }
    }

    pub fn quick_settings_button_style(
        &self,
        is_active: bool,
    ) -> impl Fn(&Theme, Status) -> button::Style + use<> {
        let radius_xl = self.radius.xl;
        let opacity = self.opacity;
        move |theme: &Theme, status: Status| {
            let primary = theme.palette().primary;
            let weak_bg = theme.extended_palette().background.weak.color;
            let strong_bg = theme.extended_palette().background.strong.color;
            let peach = theme.extended_palette().primary.weak.color;
            let on_primary = theme.extended_palette().primary.base.text;

            // Per state: (background, border_width, border_color, text)
            let (bg, border_w, border_c, text_c) = match (is_active, status) {
                // Active toggle — accent fill + ring.
                (true, Status::Hovered) => (
                    peach.scale_alpha(opacity),
                    1.5,
                    primary.scale_alpha(0.85),
                    on_primary,
                ),
                (true, Status::Pressed) => (
                    primary.scale_alpha(0.95),
                    2.0,
                    primary,
                    on_primary,
                ),
                (true, _) => (
                    primary.scale_alpha(opacity),
                    1.0,
                    primary.scale_alpha(0.65),
                    on_primary,
                ),
                // Inactive toggle — neutral with accent hover hint.
                (false, Status::Hovered) => (
                    strong_bg.scale_alpha(opacity),
                    1.0,
                    primary.scale_alpha(0.45),
                    theme.palette().text,
                ),
                (false, Status::Pressed) => (
                    primary.scale_alpha(0.22),
                    1.0,
                    primary.scale_alpha(0.6),
                    theme.palette().text,
                ),
                (false, _) => (
                    weak_bg.scale_alpha(opacity),
                    0.0,
                    Color::TRANSPARENT,
                    theme.palette().text,
                ),
            };

            button::Style {
                background: Some(bg.into()),
                border: Border {
                    width: border_w,
                    radius: radius_xl.into(),
                    color: border_c,
                },
                text_color: text_c,
                // Soft drop shadow — active buttons lift, inactive
                // ones get a hairline shadow for separation.
                shadow: Shadow {
                    color: Color { r: 0., g: 0., b: 0., a: if is_active { 0.28 } else { 0.14 } },
                    offset: Vector::new(0.0, if is_active { 3.0 } else { 1.5 }),
                    blur_radius: if is_active { 10.0 } else { 4.0 },
                },
                ..button::Style::default()
            }
        }
    }

    /// Volume / brightness / source slider — thicker rail with an
    /// accent fill on the value side, a 16 px round handle with a
    /// white inner ring and drop shadow so it reads as a pickable
    /// affordance against any wallpaper.
    pub fn slider_style(
        &self,
    ) -> impl Fn(&Theme, iced::widget::slider::Status) -> iced::widget::slider::Style + use<> {
        move |theme: &Theme, status: iced::widget::slider::Status| {
            use iced::widget::slider::{Handle, HandleShape, Rail, Status as SS, Style};

            let primary = theme.palette().primary;
            let strong_bg = theme.extended_palette().background.strong.color;

            let rail_width = 5.0;
            let handle_radius = match status {
                SS::Dragged => 9.5,
                SS::Hovered => 8.5,
                SS::Active => 7.5,
            };
            let handle_border_color = match status {
                SS::Dragged => primary,
                _ => theme.palette().background,
            };

            Style {
                rail: Rail {
                    backgrounds: (
                        // Filled side (left of handle).
                        primary.into(),
                        // Empty side.
                        strong_bg.into(),
                    ),
                    width: rail_width,
                    border: Border {
                        width: 0.0,
                        radius: (rail_width / 2.0).into(),
                        color: Color::TRANSPARENT,
                    },
                },
                handle: Handle {
                    shape: HandleShape::Circle { radius: handle_radius },
                    background: primary.into(),
                    border_width: 2.0,
                    border_color: handle_border_color,
                },
            }
        }
    }

    pub fn workspace_button_style(
        &self,
        is_empty: bool,
        colors: Option<Option<AppearanceColor>>,
    ) -> impl Fn(&Theme, Status) -> button::Style + use<> {
        let radius_lg = self.radius.lg;
        move |theme: &Theme, status: Status| {
            let (bg_color, fg_color) = colors.map_or_else(
                || {
                    (
                        theme.extended_palette().background.weak.color,
                        theme.palette().text,
                    )
                },
                |c| {
                    c.map_or_else(
                        || {
                            (
                                theme.extended_palette().primary.base.color,
                                theme.extended_palette().primary.base.text,
                            )
                        },
                        |c| {
                            let color = palette::Primary::generate(
                                c.get_base(),
                                theme.palette().background,
                                c.get_text().unwrap_or_else(|| theme.palette().text),
                            );
                            (color.base.color, color.base.text)
                        },
                    )
                },
            );
            let mut base = button::Style {
                background: Some(Background::Color(if is_empty {
                    theme.extended_palette().background.weak.color
                } else {
                    bg_color
                })),
                border: Border {
                    width: if is_empty { 1.0 } else { 0.0 },
                    color: bg_color,
                    radius: radius_lg.into(),
                },
                text_color: if is_empty {
                    theme.extended_palette().background.weak.text
                } else {
                    fg_color
                },
                ..button::Style::default()
            };
            match status {
                Status::Active => base,
                Status::Hovered => {
                    let (bg_color, fg_color) = colors.map_or_else(
                        || {
                            (
                                theme.extended_palette().background.strong.color,
                                theme.palette().text,
                            )
                        },
                        |c| {
                            c.map_or_else(
                                || {
                                    (
                                        theme.extended_palette().primary.strong.color,
                                        theme.extended_palette().primary.strong.text,
                                    )
                                },
                                |c| {
                                    let color = palette::Primary::generate(
                                        c.get_base(),
                                        theme.palette().background,
                                        c.get_text().unwrap_or_else(|| theme.palette().text),
                                    );
                                    (color.strong.color, color.strong.text)
                                },
                            )
                        },
                    );

                    base.background = Some(Background::Color(if is_empty {
                        theme.extended_palette().background.strong.color
                    } else {
                        bg_color
                    }));
                    base.text_color = if is_empty {
                        theme.extended_palette().background.weak.text
                    } else {
                        fg_color
                    };
                    base
                }
                _ => base,
            }
        }
    }

    pub fn text_input_style(
        &self,
    ) -> impl Fn(&Theme, text_input::Status) -> text_input::Style + use<> {
        let radius_xl = self.radius.xl;
        move |theme: &Theme, status: text_input::Status| {
            let mut base = text_input::Style {
                background: theme.palette().background.into(),
                border: Border {
                    width: 2.0,
                    radius: radius_xl.into(),
                    color: theme.extended_palette().background.weak.color,
                },
                icon: theme.palette().text,
                placeholder: theme.palette().text,
                value: theme.palette().text,
                selection: theme.palette().primary,
            };
            match status {
                text_input::Status::Active => base,
                text_input::Status::Focused { .. } | text_input::Status::Hovered => {
                    base.border.color = theme.extended_palette().background.strong.color;
                    base
                }
                text_input::Status::Disabled => {
                    base.background = theme.extended_palette().background.weak.color.into();
                    base.border.color = Color::TRANSPARENT;
                    base
                }
            }
        }
    }

    /// Bottom accent strip for the currently active workspace pill.
    /// 2.5px tall, sits over the pill's bottom edge inside a Stack —
    /// gives a sharp "this one is selected" signal without bloating
    /// the bar height. Color follows the pill's own accent (custom
    /// `[workspaces.colors]` override → workspace_colors palette →
    /// primary).
    pub fn workspace_active_indicator_style(
        &self,
        colors: Option<Option<AppearanceColor>>,
    ) -> impl Fn(&Theme) -> container::Style + use<> {
        let radius = self.radius.sm;
        move |theme: &Theme| {
            let accent = colors.map_or_else(
                || theme.palette().primary,
                |c| {
                    c.map_or_else(
                        || theme.palette().primary,
                        |c| c.get_base(),
                    )
                },
            );
            container::Style {
                background: Some(Background::Color(accent)),
                border: Border {
                    radius: radius.into(),
                    width: 0.0,
                    color: Color::TRANSPARENT,
                },
                ..container::Style::default()
            }
        }
    }

    /// Small muted dot used to signal "this inactive workspace has
    /// open windows." Same accent as the active indicator but at
    /// 55% alpha so it never competes with the live workspace.
    pub fn workspace_window_dot_style(
        &self,
        colors: Option<Option<AppearanceColor>>,
    ) -> impl Fn(&Theme) -> container::Style + use<> {
        move |theme: &Theme| {
            let accent = colors.map_or_else(
                || theme.palette().primary,
                |c| {
                    c.map_or_else(
                        || theme.palette().primary,
                        |c| c.get_base(),
                    )
                },
            );
            container::Style {
                background: Some(Background::Color(accent.scale_alpha(0.55))),
                border: Border {
                    radius: 1.5.into(),
                    width: 0.0,
                    color: Color::TRANSPARENT,
                },
                ..container::Style::default()
            }
        }
    }

    /// Module button style. Transparent base, accent-tinted hover,
    /// brighter accent pressed — gives the bar a "live" feel without
    /// the harsh swatches you get from naked primary backgrounds.
    /// The Islands background is handled by `module_group`, not here.
    pub fn module_button_style(&self) -> impl Fn(&Theme, Status) -> button::Style + use<> {
        let radius_lg = self.radius.lg;
        move |theme, status| {
            let primary = theme.palette().primary;
            let mut base = button::Style {
                background: None,
                border: Border {
                    width: 0.0,
                    radius: radius_lg.into(),
                    color: Color::TRANSPARENT,
                },
                text_color: theme.palette().text,
                ..button::Style::default()
            };
            match status {
                Status::Active | Status::Disabled => base,
                Status::Hovered => {
                    base.background = Some(primary.scale_alpha(0.14).into());
                    base.border = Border {
                        width: 1.0,
                        radius: radius_lg.into(),
                        color: primary.scale_alpha(0.45),
                    };
                    base
                }
                Status::Pressed => {
                    base.background = Some(primary.scale_alpha(0.28).into());
                    base.border = Border {
                        width: 1.0,
                        radius: radius_lg.into(),
                        color: primary.scale_alpha(0.65),
                    };
                    base
                }
            }
        }
    }
}

pub fn backdrop_color(backdrop: f32) -> Color {
    Color::from_rgba(0.0, 0.0, 0.0, backdrop)
}

pub fn darken_color(color: Color, darkening_alpha: f32) -> Color {
    let new_r = color.r * (1.0 - darkening_alpha);
    let new_g = color.g * (1.0 - darkening_alpha);
    let new_b = color.b * (1.0 - darkening_alpha);
    let new_a = color.a + (1.0 - color.a) * darkening_alpha;

    Color::from([new_r, new_g, new_b, new_a])
}
