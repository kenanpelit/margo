//! The Widgets group: the Widgets container page with a sub-sidebar of
//! per-widget pages (catalogue in `widget_pages`); sub-pages are built on
//! first navigation.

use super::*;

/// Build the Widgets container (sub-sidebar + lazily filled sub-stack),
/// seed its routes into the deep-link and search maps, and add it to
/// `stack` as `widgets`.
pub(super) fn add_widgets_group(
    stack: &gtk::Stack,
    alive: &Alive,
    stale: &Stale,
    subsection_buttons: &Rc<RefCell<HashMap<String, gtk::ToggleButton>>>,
    search_index: &Rc<RefCell<Vec<(String, String)>>>,
    search_titles: &Rc<RefCell<HashMap<String, String>>>,
) {
    // Owns per-widget pages. The launcher has its own top-level
    // "Launcher" page, so it intentionally does not appear in this
    // widget catalogue.
    let widgets_page = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .hexpand(true)
        .vexpand(true)
        .build();

    let widgets_sub_sidebar = gtk::ScrolledWindow::builder()
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    let widgets_sub_sidebar_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        // Wide enough for the longest widget names (icon + "Screen
        // Recording" / "System Bluetooth" / "Valent Connect") so the
        // sub-sidebar labels stop ellipsizing to "Notifica…" etc.
        // Paired with the .settings-subsidebar density overrides
        // (smaller label + tighter button padding) in _settings.scss.
        .width_request(216)
        .spacing(2)
        .hexpand(false)
        .css_classes(["settings-subsidebar"])
        .build();
    widgets_sub_sidebar.set_child(Some(&widgets_sub_sidebar_box));

    widgets_sub_sidebar_box.append(&{
        let l = settings_sidebar_title_label("Widgets");
        l.set_margin_start(8);
        l.set_margin_top(12);
        l.set_margin_bottom(6);
        l.set_margin_end(8);
        l
    });
    widgets_sub_sidebar_box.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

    let widgets_sub_stack = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .transition_duration(50)
        .hexpand(true)
        .vexpand(true)
        .build();

    // Widget sub-pages are built on first navigation. The loop below fills
    // this registry with one factory per stack-child name; `ensure_widget`
    // runs the factory the first time that page is shown. The sub-sidebar
    // buttons + search index stay eager (search must reach a widget page
    // that was never opened).
    let widget_registry: Rc<RefCell<HashMap<String, PageFactory>>> =
        Rc::new(RefCell::new(HashMap::new()));
    let ensure_widget: Rc<dyn Fn(&str)> = {
        let sub_stack = widgets_sub_stack.clone();
        let registry = widget_registry.clone();
        let alive = alive.clone();
        let stale = stale.clone();
        Rc::new(move |name: &str| {
            let route = format!("widgets/{name}");
            if !needs_build(sub_stack.child_by_name(name).is_some(), &stale, &route) {
                return;
            }
            let factory = registry.borrow().get(name).cloned();
            if let Some(factory) = factory {
                let build: Rc<dyn Fn() -> Option<BuiltPage>> = Rc::new(move || Some(factory()));
                mount_page(
                    &sub_stack,
                    name,
                    &format!("widgets/{name}"),
                    build,
                    &alive,
                    false,
                );
            }
        })
    };

    // Helper closure: build one sub-sidebar ToggleButton +
    // wire it to flip the sub-stack. All buttons except the
    // first share the same `group` so they radio-toggle.
    let make_sub_btn = |label: &str,
                        icon: &str,
                        stack_name: &'static str,
                        first: Option<&gtk::ToggleButton>|
     -> gtk::ToggleButton {
        let mut builder = gtk::ToggleButton::builder().css_classes(["sidebar-button"]);
        if let Some(g) = first {
            builder = builder.group(g);
        } else {
            builder = builder.active(true);
        }
        let btn = builder.build();
        // 8px icon→label gap (not the top-level sidebar's 12): the
        // sub-sidebar trades air for fitting the long widget names.
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.set_valign(gtk::Align::Center);
        row.append(&gtk::Image::from_icon_name(icon));
        let lbl = settings_sidebar_label(label);
        row.append(&lbl);
        btn.set_child(Some(&row));
        let sub_stack = widgets_sub_stack.clone();
        let ensure = ensure_widget.clone();
        btn.connect_toggled(move |b| {
            if b.is_active() {
                ensure(stack_name);
                sub_stack.set_visible_child_name(stack_name);
            }
        });
        // Index by sub-stack name so `widgets/<name>` deep links can
        // activate this exact page.
        subsection_buttons
            .borrow_mut()
            .insert(stack_name.to_string(), btn.clone());
        // And make the page findable from the sidebar search box.
        search_index
            .borrow_mut()
            .push((label.to_lowercase(), format!("widgets/{stack_name}")));
        search_titles
            .borrow_mut()
            .insert(format!("widgets/{stack_name}"), label.to_string());
        btn
    };

    let entries = widget_entries();

    // The default sub-page shown when the Widgets container is first
    // opened: the alphabetically-first entry, which is also the radio-group
    // anchor built `.active(true)` below. Built lazily on first map.
    let widgets_default: Option<&'static str> = entries.first().map(WidgetEntry::stack_name);

    // Each entry builds its sub-sidebar button eagerly and registers a
    // one-shot page factory (see `widget_pages`).
    register_widget_entries(
        entries,
        &make_sub_btn,
        &widgets_sub_sidebar_box,
        &widget_registry,
    );

    widgets_page.append(&widgets_sub_sidebar);
    widgets_page.append(&widgets_sub_stack);

    // Build the default sub-page the first time the Widgets container is
    // shown, so its sub-stack isn't blank — only when empty, so re-entering
    // Widgets keeps whichever sub-page the user last left it on (the
    // sub-stack retains its visible child otherwise).
    if let Some(default_name) = widgets_default {
        let ensure_widget = ensure_widget.clone();
        let sub_stack = widgets_sub_stack.clone();
        widgets_page.connect_map(move |_| {
            if sub_stack.visible_child().is_none() {
                ensure_widget(default_name);
                sub_stack.set_visible_child_name(default_name);
            }
        });
    }

    // The Widgets container stays eager for the same reason as Theme: its
    // sub-sidebar buttons seed the search index. Only its page content is
    // deferred.
    stack.add_titled(&widgets_page, Some("widgets"), "Widgets");
}
