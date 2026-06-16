use crate::compiled_css;
use crate::style_manager::StyleManagerInput::*;
use crate::style_manager::StyleManagerOutput::QueueFrameRedraw;
use crate::user_css::style::StyleStoreFields;
use crate::user_css::user_style_manager::style_manager;
use mshell_cache::wallpaper::{WallpaperStateStoreFields, source_path, wallpaper_store};
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    ConfigStoreFields, FontStoreFields, Matugen, OsdStoreFields, SizingStoreFields,
    ThemeAttributes, ThemeAttributesStoreFields, ThemeStoreFields,
};
use mshell_config::schema::themes::{MatugenMode, Themes};
use mshell_matugen::json_struct::{Font, MShell, MatugenTheme, MatugenThemeCustomOnly, Sizing};
use mshell_matugen::matugen::{apply_matugen_from_image_queued, apply_matugen_from_theme_queued};
use mshell_matugen::static_theme_mapping::static_theme;
use reactive_graph::effect::Effect;
use reactive_graph::prelude::{Get, GetUntracked};
use reactive_graph::traits::ReadUntracked;
use relm4::gtk::{CssProvider, STYLE_PROVIDER_PRIORITY_USER, gdk};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::path::PathBuf;
use tracing::{error, warn};

/// Wallpaper average-luminance cutoff for auto light/dark polarity
/// (`matugen.auto_polarity`): at or above → Light scheme, below → Dark.
const AUTO_POLARITY_THRESHOLD: f64 = 0.5;

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

/// Propagate margo's light/dark choice to GTK apps so they follow the
/// shell: the GNOME `color-scheme` (libadwaita + the xdg-desktop-portal
/// `org.freedesktop.appearance` slot read this) plus the GTK3/GTK4
/// `gtk-application-prefer-dark-theme` flag. Idempotent — a process-global
/// guard skips the work when the value is unchanged, so wallpaper rotation
/// (auto-polarity) doesn't respawn `gsettings` every cycle.
fn sync_gtk_appearance(dark: bool) {
    use std::sync::atomic::{AtomicU8, Ordering};
    static LAST: AtomicU8 = AtomicU8::new(0); // 0 = unknown, 1 = light, 2 = dark
    let want = if dark { 2 } else { 1 };
    if LAST.swap(want, Ordering::Relaxed) == want {
        return;
    }

    // Off the GTK main thread — `gsettings` spawns a subprocess; we never
    // want that (or the file writes) to hitch the UI on a theme toggle.
    std::thread::spawn(move || {
        // libadwaita / GNOME apps / portal appearance.
        let scheme = if dark { "prefer-dark" } else { "prefer-light" };
        if let Err(err) = std::process::Command::new("gsettings")
            .args(["set", "org.gnome.desktop.interface", "color-scheme", scheme])
            .status()
        {
            warn!(error = %err, "gtk dark sync: gsettings spawn failed");
        }

        // GTK3 + GTK4 settings.ini `gtk-application-prefer-dark-theme`.
        let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
            return;
        };
        for sub in ["gtk-3.0", "gtk-4.0"] {
            let dir = home.join(".config").join(sub);
            if std::fs::create_dir_all(&dir).is_err() {
                continue;
            }
            write_prefer_dark_ini(&dir.join("settings.ini"), dark);
        }
    });
}

/// Set `gtk-application-prefer-dark-theme` under `[Settings]` in a GTK
/// `settings.ini`, preserving any other keys the user has.
fn write_prefer_dark_ini(path: &std::path::Path, dark: bool) {
    const KEY: &str = "gtk-application-prefer-dark-theme";
    let val = if dark { "1" } else { "0" };

    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let mut lines: Vec<String> = existing.lines().map(String::from).collect();

    let mut has_section = false;
    let mut key_set = false;
    for line in lines.iter_mut() {
        let t = line.trim();
        if t.eq_ignore_ascii_case("[settings]") {
            has_section = true;
        }
        if t.split('=').next().map(str::trim) == Some(KEY) {
            *line = format!("{KEY}={val}");
            key_set = true;
        }
    }
    if !has_section {
        lines.insert(0, "[Settings]".to_string());
    }
    if !key_set {
        let idx = lines
            .iter()
            .position(|l| l.trim().eq_ignore_ascii_case("[settings]"))
            .map(|i| i + 1)
            .unwrap_or(lines.len());
        lines.insert(idx, format!("{KEY}={val}"));
    }

    let mut out = lines.join("\n");
    out.push('\n');
    if let Err(err) = std::fs::write(path, out) {
        warn!(path = %path.display(), error = %err, "gtk dark sync: settings.ini write failed");
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
            let attributes = config_manager().config().theme().attributes().get();
            // Subscribe to the OSD chrome knobs too — they ride the same
            // attributes CSS provider (--osd-* injected in the handler), so
            // changing them in Settings → OSD re-injects live. (Each store
            // accessor consumes the handle, so re-fetch per read.)
            let _ = config_manager().config().osd().width().get();
            let _ = config_manager().config().osd().radius().get();
            let _ = config_manager().config().osd().border_width().get();
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
                let theme = config_manager().config().theme().theme().get_untracked();
                if theme == Themes::Wallpaper && source_path().exists() {
                    // The wallpaper apply path resolves the effective mode
                    // (auto-polarity) and syncs GTK there.
                    sender.input(SetMatugenCssWithWallpaper(matugen));
                } else {
                    // Static / Default / no-wallpaper: GTK follows the
                    // explicit Mode (the Dark Mode toggle).
                    sync_gtk_appearance(matugen.mode == MatugenMode::Dark);
                }
            }
            SetMatugenCssWithStaticTheme(theme) => {
                let sender = sender.clone();
                apply_matugen_from_theme_queued(theme, move |result| {
                    sender.input(MatugenComplete(result));
                });
            }
            SetMatugenCssWithWallpaper(mut matugen) => {
                // Auto polarity: derive Light/Dark from the wallpaper's
                // average luminance (bright → Light, dark → Dark), overriding
                // the configured `mode`. Decoded small, so cheap enough to do
                // inline on a wallpaper change.
                if matugen.auto_polarity
                    && let Some(lum) = mshell_image::lut::average_luminance(&source_path())
                {
                    matugen.mode = if lum >= AUTO_POLARITY_THRESHOLD {
                        MatugenMode::Light
                    } else {
                        MatugenMode::Dark
                    };
                }
                // Propagate the *effective* light/dark to GTK apps (covers
                // both the manual Mode and the auto-polarity resolution).
                sync_gtk_appearance(matugen.mode == MatugenMode::Dark);
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
                    let helium = config_manager()
                        .config()
                        .read_untracked()
                        .theme
                        .apps
                        .helium
                        .clone();
                    crate::app_theme::apply_helium_from_cache_async(helium);

                    let _ = sender.output(QueueFrameRedraw);
                }
                Err(e) => {
                    error!("Error loading matugen theme: {}", e);
                }
            },
            AttributesUpdate(attributes) => {
                // Manual frame colour overrides (Settings → Bar → Frame).
                // Empty = leave the SCSS default (matugen --surface /
                // --outline) in place. Non-empty CSS hex overrides --frame-bg
                // / --frame-border, which the painted bar frame reads.
                let mut frame_overrides = String::new();
                let fc = attributes.sizing.frame_color.trim();
                if !fc.is_empty() {
                    frame_overrides.push_str(&format!("--frame-bg: {fc};"));
                }
                let fbc = attributes.sizing.frame_border_color.trim();
                if !fbc.is_empty() {
                    frame_overrides.push_str(&format!("--frame-border: {fbc};"));
                }
                let sc = attributes.sizing.separator_color.trim();
                if !sc.is_empty() {
                    frame_overrides.push_str(&format!("--bar-separator-color: {sc};"));
                }
                // OSD capsule chrome (Settings → OSD). Read here (untracked —
                // the effect above subscribes for live re-injection) and
                // clamped so a stray edit can't break the layout. Drives the
                // `--osd-*` fallbacks in `_osd_window.scss`.
                let osd_width = config_manager()
                    .config()
                    .osd()
                    .width()
                    .get()
                    .clamp(80, 1200);
                let osd_radius = config_manager().config().osd().radius().get().clamp(0, 200);
                let osd_border = config_manager()
                    .config()
                    .osd()
                    .border_width()
                    .get()
                    .clamp(0, 20);
                self.attributes_css_provider.load_from_string(&format!(
                    r#":root {{
                        --font-family-primary: {};
                        --font-family-secondary: {};
                        --font-family-tertiary: {};
                        --font-family-monospace: {};
                        --window-opacity: {};
                        --radius-widget: {}px;
                        --radius-window: {}px;
                        --border-width: {}px;
                        --bar-hover-strength: {}%;
                        --font-scale-settings: {:.4};
                        --font-scale: {:.4};
                        --font-bar-scale: {:.4};
                        --surface-opacity: {}%;
                        --osd-width: {}px;
                        --osd-radius: {}px;
                        --osd-border-width: {}px;
                        {}
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
                    if attributes.font.monospace.is_empty() {
                        "monospace"
                    } else {
                        &attributes.font.monospace
                    },
                    attributes.window_opacity.get(),
                    attributes.sizing.radius_widget,
                    attributes.sizing.radius_window,
                    attributes.sizing.border_width,
                    attributes.sizing.bar_hover_strength.clamp(0, 60),
                    // Clamp to a sane range so a stray config
                    // edit can't shrink the panel to 0 or blow
                    // it past the screen. CSS unitless number,
                    // multiplied against px values in
                    // _settings.scss.
                    attributes.sizing.settings_font_scale.clamp(0.5, 2.0),
                    // Global UI font scale + bar-pill font scale — same
                    // sane clamp, multiplied against the px font tokens
                    // / --font-bar in _font.scss.
                    attributes.sizing.font_scale.clamp(0.5, 2.0),
                    attributes.sizing.bar_font_scale.clamp(0.5, 2.0),
                    // Painted shell-surface opacity as a CSS percentage;
                    // clamped so a frosted surface never fully vanishes. Read
                    // by the frame-draw widget (fill alpha) and color-mix on
                    // the frameless panel backgrounds.
                    attributes.sizing.surface_opacity.clamp(60, 100),
                    osd_width,
                    osd_radius,
                    osd_border,
                    frame_overrides,
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
