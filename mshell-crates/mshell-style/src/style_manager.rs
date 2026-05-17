use crate::compiled_css;
use crate::style_manager::StyleManagerInput::*;
use crate::style_manager::StyleManagerOutput::QueueFrameRedraw;
use crate::user_css::style::StyleStoreFields;
use crate::user_css::user_style_manager::style_manager;
use mshell_cache::wallpaper::{WallpaperStateStoreFields, source_path, wallpaper_store};
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    ConfigStoreFields, FontStoreFields, Matugen, SizingStoreFields, ThemeAttributes,
    ThemeAttributesStoreFields, ThemeStoreFields,
};
use mshell_config::schema::themes::Themes;
use mshell_matugen::json_struct::{Font, MatugenTheme, MatugenThemeCustomOnly, MShell, Sizing};
use mshell_matugen::matugen::{apply_matugen_from_image_queued, apply_matugen_from_theme_queued};
use mshell_matugen::static_theme_mapping::static_theme;
use reactive_graph::effect::Effect;
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::{CssProvider, STYLE_PROVIDER_PRIORITY_USER, gdk};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::path::PathBuf;
use tracing::{error, warn};

/// Path to the cached matugen CSS — written on every successful
/// matugen run, loaded synchronously at startup to eliminate the
/// "compiled-in baseline → matugen completes ~300ms later" flash.
///
/// Lives in `$XDG_CACHE_HOME/mshell/last_theme.css` (falling back
/// to `~/.cache/mshell/last_theme.css` when XDG_CACHE_HOME is
/// unset). The file is overwritten atomically (write to .tmp, then
/// rename) so a half-written cache can never load.
fn cached_theme_path() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join("mshell").join("last_theme.css")
}

/// Synchronously read the last successful matugen CSS, if any. Used
/// at startup to paint the bar in the correct palette from the very
/// first frame — the async matugen run that follows produces the
/// same result on the steady-state path (wallpaper / theme didn't
/// change) and the CssProvider reload is a no-op visually.
fn read_cached_theme() -> Option<String> {
    let path = cached_theme_path();
    std::fs::read_to_string(&path).ok()
}

/// Atomically overwrite the cache with the latest matugen CSS.
/// Best-effort: a failed write just means the next startup pays
/// the matugen flash one more time; the user-facing CSS is
/// independent of this.
fn write_cached_theme(css: &str) {
    let path = cached_theme_path();
    if let Some(parent) = path.parent()
        && let Err(err) = std::fs::create_dir_all(parent)
    {
        warn!(path = %parent.display(), error = %err, "cached theme: mkdir failed");
        return;
    }
    let tmp = path.with_extension("css.tmp");
    if let Err(err) = std::fs::write(&tmp, css) {
        warn!(path = %tmp.display(), error = %err, "cached theme: tmp write failed");
        return;
    }
    if let Err(err) = std::fs::rename(&tmp, &path) {
        warn!(from = %tmp.display(), to = %path.display(), error = %err, "cached theme: rename failed");
    }
}

pub struct StyleManagerModel {
    user_css_provider: CssProvider,
    theme_css_provider: CssProvider,
    attributes_css_provider: CssProvider,
}

#[derive(Debug)]
pub enum StyleManagerInput {
    ReloadUserCss(String),
    ReloadTheme(Themes),
    WallpaperRevisionChanged,
    SetMatugenCssWithWallpaper(Matugen),
    MatugenUpdate(Matugen),
    SetMatugenCssWithStaticTheme(MatugenTheme),
    MatugenComplete(anyhow::Result<String>),
    AttributesUpdate(ThemeAttributes),
}

#[derive(Debug)]
pub enum StyleManagerOutput {
    QueueFrameRedraw,
}

#[relm4::component(pub)]
impl Component for StyleManagerModel {
    type Input = StyleManagerInput;
    type Init = ();
    type Output = StyleManagerOutput;
    type CommandOutput = ();

    view! {
        #[root]
        gtk::Box {}
    }

    fn init(
        _init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let base_css_provider = CssProvider::new();
        let user_css_provider = CssProvider::new();
        let theme_css_provider = CssProvider::new();
        let attributes_css_provider = CssProvider::new();

        let display = gdk::Display::default().expect("No GDK display available");
        gtk::style_context_add_provider_for_display(
            &display,
            &base_css_provider,
            STYLE_PROVIDER_PRIORITY_USER,
        );

        gtk::style_context_add_provider_for_display(
            &display,
            &theme_css_provider,
            STYLE_PROVIDER_PRIORITY_USER + 1,
        );

        gtk::style_context_add_provider_for_display(
            &display,
            &attributes_css_provider,
            STYLE_PROVIDER_PRIORITY_USER + 2,
        );

        gtk::style_context_add_provider_for_display(
            &display,
            &user_css_provider,
            STYLE_PROVIDER_PRIORITY_USER + 3,
        );

        base_css_provider.load_from_string(compiled_css());

        // Synchronously load the cached matugen CSS — if a previous
        // session wrote one. This eliminates the ~300 ms "compiled
        // baseline → matugen completes" flash on every login after
        // the first: the cached output produces the same palette as
        // the about-to-run async matugen pass (wallpaper / theme
        // didn't change), so when MatugenComplete fires later the
        // CssProvider reload is visually a no-op. First-ever login
        // pays the flash once because the cache file doesn't exist
        // yet — covered by the margo-aligned compile-time baseline
        // in `_colors.scss` so even that flash is barely visible.
        if let Some(cached) = read_cached_theme() {
            theme_css_provider.load_from_string(&cached);
        }

        style_manager().watch_style();

        let base_style = style_manager().style();
        let style_sender = sender.clone();
        Effect::new(move || {
            let style = base_style.clone();
            let css = style.css().get();
            style_sender.input(ReloadUserCss(css));
        });

        Effect::new(move || {
            let config = config_manager().config();
            let _ = config.theme().css_file().get();
            style_manager().reload_style();
        });

        let sender_clone = sender.clone();
        Effect::new(move || {
            let config = config_manager().config();
            let theme = config.theme().theme().get();
            sender_clone.input(ReloadTheme(theme));
        });

        let sender_clone = sender.clone();
        Effect::new(move || {
            let _revision = wallpaper_store().revision().get();
            sender_clone.input(WallpaperRevisionChanged);
        });

        let sender_clone = sender.clone();
        Effect::new(move || {
            let config = config_manager().config();
            let matugen = config.theme().matugen().get();
            sender_clone.input(MatugenUpdate(matugen));
        });

        let sender_clone = sender.clone();
        Effect::new(move || {
            let config = config_manager().config();
            let attributes = config.theme().attributes().get();
            sender_clone.input(AttributesUpdate(attributes));
        });

        let model = StyleManagerModel {
            user_css_provider,
            theme_css_provider,
            attributes_css_provider,
        };

        let widgets = view_output!();

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            ReloadUserCss(css) => {
                self.user_css_provider.load_from_string(&css);
                let _ = sender.output(QueueFrameRedraw);
            }
            ReloadTheme(theme) => {
                if let Some(static_theme) = static_theme(&theme, Some(build_mshell_matugen())) {
                    sender.input(SetMatugenCssWithStaticTheme(static_theme));
                } else {
                    if theme == Themes::Default {
                        self.theme_css_provider.load_from_string("");
                    } else if theme == Themes::Wallpaper {
                        let source = source_path();
                        if source.exists() {
                            let matugen =
                                config_manager().config().theme().matugen().get_untracked();
                            sender.input(SetMatugenCssWithWallpaper(matugen));
                        } else {
                            self.theme_css_provider.load_from_string("");
                        }
                    }
                }
                let _ = sender.output(QueueFrameRedraw);
            }
            WallpaperRevisionChanged => {
                if config_manager().config().theme().theme().get_untracked() == Themes::Wallpaper {
                    let source = source_path();
                    if source.exists() {
                        let matugen = config_manager().config().theme().matugen().get_untracked();
                        sender.input(SetMatugenCssWithWallpaper(matugen));
                    }
                }
            }
            MatugenUpdate(matugen) => {
                if config_manager().config().theme().theme().get_untracked() == Themes::Wallpaper {
                    let source = source_path();
                    if source.exists() {
                        sender.input(SetMatugenCssWithWallpaper(matugen));
                    }
                }
            }
            SetMatugenCssWithStaticTheme(theme) => {
                let sender = sender.clone();
                apply_matugen_from_theme_queued(theme, move |result| {
                    sender.input(MatugenComplete(result));
                });
            }
            SetMatugenCssWithWallpaper(matugen) => {
                let theme_overrides = MatugenThemeCustomOnly {
                    mshell: build_mshell_matugen(),
                };
                let sender = sender.clone();
                apply_matugen_from_image_queued(
                    source_path(),
                    matugen,
                    theme_overrides,
                    move |result| {
                        sender.input(MatugenComplete(result));
                    },
                );
            }
            MatugenComplete(result) => match result {
                Ok(css) => {
                    self.theme_css_provider.load_from_string(&css);
                    // Persist for next session's synchronous load —
                    // see read_cached_theme in init(). Best-effort:
                    // failures are logged at warn but don't affect
                    // the current session.
                    write_cached_theme(&css);

                    let _ = sender.output(QueueFrameRedraw);
                }
                Err(e) => {
                    error!("Error loading matugen theme: {}", e);
                }
            },
            AttributesUpdate(attributes) => {
                self.attributes_css_provider.load_from_string(&format!(
                    r#":root {{
                        --font-family-primary: {};
                        --font-family-secondary: {};
                        --font-family-tertiary: {};
                        --window-opacity: {};
                        --radius-widget: {}px;
                        --radius-window: {}px;
                        --border-width: {}px;
                        --font-scale-settings: {};
                    }}"#,
                    if attributes.font.primary.is_empty() {
                        "inherit"
                    } else {
                        &attributes.font.primary
                    },
                    if attributes.font.secondary.is_empty() {
                        "inherit"
                    } else {
                        &attributes.font.secondary
                    },
                    if attributes.font.tertiary.is_empty() {
                        "inherit"
                    } else {
                        &attributes.font.tertiary
                    },
                    attributes.window_opacity.get(),
                    attributes.sizing.radius_widget,
                    attributes.sizing.radius_window,
                    attributes.sizing.border_width,
                    // Clamp to a sane range so a stray config
                    // edit can't shrink the panel to 0 or blow
                    // it past the screen. CSS unitless number,
                    // multiplied against px values in
                    // _settings.scss.
                    attributes.sizing.settings_font_scale.clamp(0.5, 2.0),
                ));

                sender.input(ReloadTheme(
                    config_manager().config().theme().theme().get_untracked(),
                ));
            }
        }
    }
}

fn build_mshell_matugen() -> MShell {
    MShell {
        font: Font {
            primary: config_manager()
                .config()
                .theme()
                .attributes()
                .font()
                .primary()
                .get_untracked(),
            secondary: config_manager()
                .config()
                .theme()
                .attributes()
                .font()
                .primary()
                .get_untracked(),
            tertiary: config_manager()
                .config()
                .theme()
                .attributes()
                .font()
                .primary()
                .get_untracked(),
        },
        sizing: Sizing {
            radius_widget: config_manager()
                .config()
                .theme()
                .attributes()
                .sizing()
                .radius_widget()
                .get_untracked(),
            radius_window: config_manager()
                .config()
                .theme()
                .attributes()
                .sizing()
                .radius_window()
                .get_untracked(),
            border_width: config_manager()
                .config()
                .theme()
                .attributes()
                .sizing()
                .border_width()
                .get_untracked(),
        },
        opacity: config_manager()
            .config()
            .theme()
            .attributes()
            .window_opacity()
            .get_untracked()
            .get(),
    }
}
