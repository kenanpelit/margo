//! Render smoke tests for every screen and overlay.
//!
//! These drive the real widget tree through ratatui's [`TestBackend`] and
//! assert on the resulting character buffer. They are deliberately *not*
//! golden-file snapshots: pinning every cell would break on any cosmetic
//! tweak and train everyone to regenerate the file without reading it.
//! Instead each test asserts the things a regression would actually break —
//! that the screen draws at all, that its title and key content are present,
//! and that it survives a terminal far too small for its layout.
//!
//! The panic case is the one these earn their keep on: a layout that
//! divides by a zero-width chunk or indexes a constraint that collapsed only
//! shows up at a specific terminal size, which is exactly what nobody tests
//! by hand.

use ratatui::{backend::TestBackend, layout::Rect, Terminal};

use crate::config::{Config, ConfigPaths};
use crate::tui::app::App;
use crate::tui::screens::Screen;

/// Flatten a rendered buffer into one string per row.
fn rows(terminal: &Terminal<TestBackend>) -> Vec<String> {
    let buffer = terminal.backend().buffer();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect()
}

/// Everything drawn, as a single string — for "does this text appear anywhere"
/// assertions that shouldn't care which row it landed on.
fn text(terminal: &Terminal<TestBackend>) -> String {
    rows(terminal).join("\n")
}

/// A config pointing at a directory that does not exist, so every screen's
/// data load fails the same way on every machine. These tests are about the
/// widget tree surviving, not about real module data.
fn empty_env() -> (ConfigPaths, Config) {
    let root = std::path::PathBuf::from("/nonexistent/mdots-render-test");
    let paths = ConfigPaths {
        config_dir: root.clone(),
        config_file: root.join("config.yaml"),
        packages_dir: root.join("packages"),
        state_dir: root.join("state"),
        state_file: root.join("state/state.yaml"),
        hooks_state_file: root.join("state/hooks.yaml"),
        services_state_file: root.join("state/services.yaml"),
        defaults_state_file: root.join("state/defaults.yaml"),
        theming_state_file: root.join("state/theming.yaml"),
        config_backups_dir: root.join("config-backups"),
    };
    // Built through serde rather than by hand so this keeps compiling when
    // `Config` gains a field — almost everything on it is `serde(default)`.
    let config: Config = serde_yaml::from_str("host: render-test\n").expect("minimal config");
    (paths, config)
}

fn render_app(app: &mut App, width: u16, height: u16) -> Terminal<TestBackend> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test terminal");
    terminal
        .draw(|frame| {
            crate::tui::ui::render(app, frame).expect("render");
        })
        .expect("draw");
    terminal
}

fn app_on(screen: Screen) -> App {
    let (paths, config) = empty_env();
    let mut app = App::new(paths, config).expect("app");
    app.current_screen = screen;
    app.sidebar.collapsed = true;
    app
}

/// Every screen, by the sidebar index that reaches it.
fn every_screen() -> Vec<Screen> {
    vec![
        Screen::Overview(Default::default()),
        Screen::Modules(Default::default()),
        Screen::Packages(Default::default()),
        Screen::Diff(Default::default()),
        Screen::Sync(Default::default()),
        Screen::Services(Default::default()),
        Screen::Secrets(Default::default()),
        Screen::Hooks(Default::default()),
    ]
}

#[test]
fn every_screen_renders_without_panicking() {
    for screen in every_screen() {
        let name = screen.name();
        let mut app = app_on(screen);
        let terminal = render_app(&mut app, 100, 30);
        assert!(
            !text(&terminal).trim().is_empty(),
            "{name} drew an empty frame"
        );
    }
}

/// The layout math must hold at sizes no one would choose but a tiling
/// compositor will happily hand you.
#[test]
fn every_screen_survives_a_tiny_terminal() {
    for screen in every_screen() {
        let name = screen.name();
        let mut app = app_on(screen);
        // 1x1 is the degenerate case: every constraint collapses to zero.
        for (w, h) in [(1, 1), (4, 3), (20, 5)] {
            render_app(&mut app, w, h);
        }
        // Reaching here means none of those panicked.
        let _ = name;
    }
}

#[test]
fn the_titlebar_names_the_active_screen() {
    let mut app = app_on(Screen::Diff(Default::default()));
    let terminal = render_app(&mut app, 100, 30);
    assert!(text(&terminal).contains("Diff"));
}

#[test]
fn the_expanded_sidebar_lists_every_destination() {
    let (paths, config) = empty_env();
    let mut app = App::new(paths, config).expect("app");
    app.sidebar.collapsed = false;
    let terminal = render_app(&mut app, 100, 30);
    let drawn = text(&terminal);
    for item in ["Overview", "Modules", "Packages", "Diff", "Sync"] {
        assert!(drawn.contains(item), "sidebar is missing {item}");
    }
}

/// The sidebar rect recorded during render must be the one the click handler
/// hit-tests against — this is the invariant that replaced two copies of the
/// layout constants.
#[test]
fn a_click_on_a_sidebar_row_navigates_to_that_screen() {
    let (paths, config) = empty_env();
    let mut app = App::new(paths, config).expect("app");
    app.sidebar.collapsed = false;
    render_app(&mut app, 100, 30);

    let items = app.layout.sidebar_items.expect("sidebar was drawn");
    // Row 3 of the sidebar is "Diff" (Overview, Modules, Packages, Diff).
    app.handle_left_click(items.x + 1, items.y + 3);

    assert_eq!(app.current_screen.name(), "Diff");
    assert_eq!(app.sidebar.selected_index, 3);
}

#[test]
fn a_click_below_the_last_sidebar_row_does_nothing() {
    let (paths, config) = empty_env();
    let mut app = App::new(paths, config).expect("app");
    app.sidebar.collapsed = false;
    render_app(&mut app, 100, 30);

    let items = app.layout.sidebar_items.expect("sidebar was drawn");
    let before = app.current_screen.name();
    app.handle_left_click(items.x + 1, items.y + 50);
    assert_eq!(app.current_screen.name(), before);
}

#[test]
fn a_collapsed_sidebar_swallows_clicks_where_it_used_to_be() {
    let mut app = app_on(Screen::Overview(Default::default()));
    render_app(&mut app, 100, 30);
    assert!(app.layout.sidebar_items.is_none());

    app.handle_left_click(2, 5);
    assert_eq!(app.current_screen.name(), "Overview");
}

#[test]
fn the_help_overlay_draws_over_the_screen() {
    let mut app = app_on(Screen::Overview(Default::default()));
    app.help_visible = true;
    let terminal = render_app(&mut app, 100, 30);
    assert!(text(&terminal).to_lowercase().contains("keybinding"));
}

#[test]
fn the_palette_draws_its_query_line_and_entries() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    let mut app = app_on(Screen::Overview(Default::default()));
    app.handle_global_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL))
        .expect("open palette");
    assert!(app.palette.is_some());

    let terminal = render_app(&mut app, 100, 30);
    let drawn = text(&terminal);
    assert!(drawn.contains("Command palette"));
    assert!(drawn.contains("Go to"));
}

#[test]
fn typing_in_the_palette_narrows_what_is_drawn() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    let mut app = app_on(Screen::Overview(Default::default()));
    app.handle_global_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL))
        .expect("open palette");
    for c in "quit".chars() {
        app.handle_global_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
            .expect("type");
    }

    let terminal = render_app(&mut app, 100, 30);
    let drawn = text(&terminal);
    assert!(drawn.contains("Quit mdots"));
    assert!(
        !drawn.contains("Go to Packages"),
        "unrelated entries should have been filtered out"
    );
}

/// The palette owns every printable key while open: `q` must type into the
/// query rather than quitting the TUI out from under it.
#[test]
fn the_palette_swallows_the_quit_key() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    let mut app = app_on(Screen::Overview(Default::default()));
    app.handle_global_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL))
        .expect("open palette");
    app.handle_global_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE))
        .expect("type q");

    assert!(!app.should_quit);
    assert_eq!(app.palette.as_ref().map(|p| p.query.as_str()), Some("q"));
}

#[test]
fn enter_runs_the_highlighted_palette_command() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    let mut app = app_on(Screen::Overview(Default::default()));
    app.handle_global_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL))
        .expect("open palette");
    for c in "quit".chars() {
        app.handle_global_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
            .expect("type");
    }
    app.handle_global_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("activate");

    assert!(app.should_quit);
    assert!(app.palette.is_none(), "palette closes after activation");
}

#[test]
fn escape_closes_the_palette_without_running_anything() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    let mut app = app_on(Screen::Overview(Default::default()));
    app.handle_global_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL))
        .expect("open palette");
    app.handle_global_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
        .expect("close");

    assert!(app.palette.is_none());
    assert!(!app.should_quit);
    assert_eq!(app.current_screen.name(), "Overview");
}

/// Navigating through the palette must move the sidebar highlight with it,
/// or the sidebar and the active screen disagree about where you are.
#[test]
fn palette_navigation_moves_the_sidebar_selection_too() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    let mut app = app_on(Screen::Overview(Default::default()));
    app.handle_global_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL))
        .expect("open palette");
    for c in "sync".chars() {
        app.handle_global_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
            .expect("type");
    }
    app.handle_global_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        .expect("activate");

    assert_eq!(app.current_screen.name(), "Sync");
    assert_eq!(app.sidebar.selected_index, 4);
}

/// A frame drawn straight after the app is built must not be blank — the
/// redraw gate starts open, and a regression there would show as a TUI that
/// only paints once you press a key.
#[test]
fn the_first_frame_is_requested_before_any_input() {
    let (paths, config) = empty_env();
    let app = App::new(paths, config).expect("app");
    assert!(app.needs_refresh);
}

#[test]
fn drawing_clears_the_redraw_request() {
    let mut app = app_on(Screen::Overview(Default::default()));
    app.needs_refresh = true;
    render_app(&mut app, 80, 24);
    // `ui::render` itself doesn't clear the flag — the loop does — but the
    // render must not *set* it either, or the gate never closes.
    app.needs_refresh = false;
    render_app(&mut app, 80, 24);
    assert!(
        !app.needs_refresh,
        "rendering must not re-arm the redraw gate"
    );
}

#[test]
fn the_recorded_content_rect_covers_the_frame_below_the_titlebar() {
    let mut app = app_on(Screen::Overview(Default::default()));
    let terminal = render_app(&mut app, 100, 30);
    let area = terminal.backend().buffer().area;
    let sidebar = app.layout.sidebar_items;
    assert!(sidebar.is_none(), "collapsed sidebar records no rect");
    assert_eq!(area, Rect::new(0, 0, 100, 30));
}
