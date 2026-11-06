//! Isolated native GTK render fixture. Never starts the service watcher or player.
//! Run only under a private D-Bus session and Xvfb/Broadway (see README.md).
use super::*;
use relm4::ComponentController;

fn pump() {
    let context = gtk::glib::MainContext::default();
    for _ in 0..40 {
        while context.pending() {
            context.iteration(false);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn screenshot(window: &gtk::Window, path: &std::path::Path) {
    fn verify_lists(widget: &gtk::Widget) {
        if widget.is_mapped() && widget.is::<gtk::ScrolledWindow>() {
            assert!(
                widget.height() >= 32,
                "main list must retain visible content height"
            );
        }
        let mut child = widget.first_child();
        while let Some(w) = child {
            verify_lists(&w);
            child = w.next_sibling();
        }
    }
    verify_lists(window.upcast_ref());
    let paintable = gtk::WidgetPaintable::new(Some(window));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, window.width() as f64, window.height() as f64);
    let node = snapshot.to_node().expect("rendered native widget tree");
    let surface = window.surface().expect("fixture surface");
    let renderer = gtk::gsk::Renderer::for_surface(&surface).expect("fixture renderer");
    let texture = renderer.render_texture(&node, None);
    texture
        .save_to_png(path)
        .expect("save actual GTK screenshot");
    renderer.unrealize();
}

#[test]
#[ignore = "requires isolated display; writes actual GTK screenshots to TUNE_RENDER_DIR"]
fn native_tune_menu_render_fixture() {
    gtk::init().expect("isolated GTK display");
    let output = std::path::PathBuf::from(
        std::env::var("TUNE_RENDER_DIR").expect("fixture output directory"),
    );
    std::fs::create_dir_all(&output).expect("fixture output directory");
    let css = gtk::CssProvider::new();
    css.load_from_path(std::env::var("TUNE_RENDER_CSS").expect("compiled mshell stylesheet"));
    gtk::style_context_add_provider_for_display(
        &gtk::gdk::Display::default().expect("display"),
        &css,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    let p = mtune_service().player.clone();
    p.running.set(true);
    p.playing.set(true);
    p.has_song.set(true);
    p.title.set("A quiet place to begin".into());
    p.artist
        .set("Native GTK fixture · not a live player".into());
    p.queue_entries.set(
        (0..18)
            .map(|i| {
                (
                    format!("Track {} — evening sessions", i + 1),
                    "Fixture artist".into(),
                    240,
                )
            })
            .collect(),
    );
    p.queue_len.set(18);
    p.current_index.set(2);
    p.duration.set(Duration::from_secs(240));
    p.position.set(Duration::from_secs(87));
    p.playlists
        .set(vec!["Evening sessions".into(), "Focus".into()]);
    p.library_roots.set(vec!["/fixture/Music".into()]);
    p.radio_stations.set(vec![
        (
            "fixture-one".into(),
            "Radio One".into(),
            "https://example.invalid/live".into(),
            "Jazz · Türkiye".into(),
        ),
        (
            "fixture-two".into(),
            "Night Signals".into(),
            "https://example.invalid/night".into(),
            "Ambient · Germany".into(),
        ),
    ]);
    let controller = MtuneMenuWidgetModel::builder()
        .launch(MtuneMenuWidgetInit {})
        .detach();
    let window = gtk::Window::builder()
        .title("Tune native fixture")
        .default_width(440)
        .default_height(700)
        .child(controller.widget())
        .build();
    window.set_size_request(440, 700);
    window.present();
    pump();
    screenshot(&window, &output.join("tune-queue.png"));
    controller.emit(MtuneMenuInput::Tab(1));
    pump();
    screenshot(&window, &output.join("tune-library.png"));
    p.is_live.set(true);
    p.station_name.set("Radio One".into());
    p.title.set("Late night jazz".into());
    controller.emit(MtuneMenuInput::Tab(2));
    pump();
    fn assert_station_navigation(widget: &gtk::Widget) {
        if let Some(button) = widget.downcast_ref::<gtk::Button>()
            && matches!(button.tooltip_text().as_deref(), Some("Previous" | "Next"))
        {
            assert!(
                button.is_sensitive(),
                "station navigation must remain enabled"
            );
        }
        let mut child = widget.first_child();
        while let Some(w) = child {
            assert_station_navigation(&w);
            child = w.next_sibling();
        }
    }
    assert_station_navigation(controller.widget().upcast_ref());
    screenshot(&window, &output.join("tune-radio.png"));
    window.set_size_request(360, 640);
    window.set_default_size(360, 640);
    pump();
    screenshot(&window, &output.join("tune-radio-small.png"));
    assert!(
        window.width() <= 360,
        "small menu must not widen its host: {}",
        window.width()
    );
    fn expand_custom(widget: &gtk::Widget) {
        if let Some(expander) = widget.downcast_ref::<gtk::Expander>() {
            expander.set_expanded(true);
        }
        let mut child = widget.first_child();
        while let Some(w) = child {
            expand_custom(&w);
            child = w.next_sibling();
        }
    }
    expand_custom(controller.widget().upcast_ref());
    pump();
    fn scroll_to_editor(widget: &gtk::Widget) {
        if widget.is_mapped()
            && let Some(scroller) = widget.downcast_ref::<gtk::ScrolledWindow>()
        {
            let adjustment = scroller.vadjustment();
            adjustment.set_value((adjustment.upper() - adjustment.page_size()).max(0.0));
        }
        let mut child = widget.first_child();
        while let Some(w) = child {
            scroll_to_editor(&w);
            child = w.next_sibling();
        }
    }
    scroll_to_editor(controller.widget().upcast_ref());
    pump();
    screenshot(&window, &output.join("tune-radio-custom-small.png"));
    assert!(window.width() <= 360, "inline editor must fit small host");
    assert!(
        window.width() <= 440,
        "menu content must not widen its host: {}",
        window.width()
    );
    window.close();
    pump();
}
