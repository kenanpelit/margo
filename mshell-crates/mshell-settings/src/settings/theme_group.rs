//! The Theme group: the Theme container page (scheme, fonts, wallpaper,
//! app themes) with its own sub-sidebar; sub-pages are built on first
//! navigation.

use super::*;

/// Build the Theme container (sub-sidebar + lazily filled sub-stack), seed
/// its routes into the deep-link and search maps, and add it to `stack` as
/// `theme`.
pub(super) fn add_theme_group(
    stack: &gtk::Stack,
    alive: &Alive,
    stale: &Stale,
    subsection_buttons: &Rc<RefCell<HashMap<String, gtk::ToggleButton>>>,
    search_index: &Rc<RefCell<Vec<(String, String)>>>,
    search_titles: &Rc<RefCell<HashMap<String, String>>>,
) {
    // Theme owns related pages (scheme, fonts, wallpaper, app themes)
    // under one top-level sidebar entry. This keeps the main nav compact
    // and makes external-app theming feel like part of the matugen chain.
    let theme_page = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .hexpand(true)
        .vexpand(true)
        .build();

    let theme_sub_sidebar = gtk::ScrolledWindow::builder()
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    let theme_sub_sidebar_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .width_request(180)
        .spacing(2)
        .hexpand(false)
        .css_classes(["settings-subsidebar"])
        .build();
    theme_sub_sidebar.set_child(Some(&theme_sub_sidebar_box));

    theme_sub_sidebar_box.append(&{
        let l = settings_sidebar_title_label("Theme");
        l.set_margin_start(8);
        l.set_margin_top(12);
        l.set_margin_bottom(6);
        l.set_margin_end(8);
        l
    });
    theme_sub_sidebar_box.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

    let theme_sub_stack = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .transition_duration(50)
        .hexpand(true)
        .vexpand(true)
        .hhomogeneous(false)
        .build();

    // Theme sub-pages are built on first navigation into each entry. The
    // buttons + search index below stay eager (search must reach a theme
    // page that was never opened); only the page content is deferred.
    let theme_registry: Rc<RefCell<HashMap<String, PageFactory>>> =
        Rc::new(RefCell::new(HashMap::new()));
    let ensure_theme: Rc<dyn Fn(&str)> = {
        let sub_stack = theme_sub_stack.clone();
        let registry = theme_registry.clone();
        let alive = alive.clone();
        let stale = stale.clone();
        Rc::new(move |name: &str| {
            let route = format!("theme/{name}");
            if !needs_build(sub_stack.child_by_name(name).is_some(), &stale, &route) {
                return;
            }
            let factory = registry.borrow().get(name).cloned();
            if let Some(factory) = factory {
                let build: Rc<dyn Fn() -> Option<BuiltPage>> = Rc::new(move || Some(factory()));
                mount_page(
                    &sub_stack,
                    name,
                    &format!("theme/{name}"),
                    build,
                    &alive,
                    false,
                );
            }
        })
    };

    let make_theme_sub_btn = |label: &str,
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
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.set_valign(gtk::Align::Center);
        row.append(&gtk::Image::from_icon_name(icon));
        row.append(&settings_sidebar_label(label));
        btn.set_child(Some(&row));
        let sub_stack = theme_sub_stack.clone();
        let ensure = ensure_theme.clone();
        btn.connect_toggled(move |b| {
            if b.is_active() {
                ensure(stack_name);
                sub_stack.set_visible_child_name(stack_name);
            }
        });
        let route = format!("theme/{stack_name}");
        subsection_buttons
            .borrow_mut()
            .insert(route.clone(), btn.clone());
        search_index
            .borrow_mut()
            .push((label.to_lowercase(), route.clone()));
        search_titles.borrow_mut().insert(route, label.to_string());
        btn
    };

    theme_registry.borrow_mut().extend([
        (
            "scheme".to_string(),
            page_factory!(ThemeSettingsModel => ThemeSettingsInit {}),
        ),
        (
            "fonts".to_string(),
            page_factory!(FontsSettingsModel => FontsSettingsInit {}),
        ),
        (
            "wallpaper".to_string(),
            page_factory!(WallpaperSettingsModel => WallpaperSettingsInit {}),
        ),
        (
            "apps".to_string(),
            page_factory!(HeliumThemeSettingsModel => HeliumThemeSettingsInit {}),
        ),
    ]);

    let theme_entries: [(&str, &str, &'static str); 4] = [
        ("Scheme", "palette-symbolic", "scheme"),
        ("Fonts", "xsi-font-symbolic", "fonts"),
        ("Wallpaper", "wallpaper-symbolic", "wallpaper"),
        ("Apps", "web-browser-symbolic", "apps"),
    ];
    let mut theme_anchor: Option<gtk::ToggleButton> = None;
    for (label, icon, stack_name) in theme_entries {
        let btn = make_theme_sub_btn(label, icon, stack_name, theme_anchor.as_ref());
        if theme_anchor.is_none() {
            theme_anchor = Some(btn.clone());
        }
        theme_sub_sidebar_box.append(&btn);
    }
    theme_page.append(&theme_sub_sidebar);
    theme_page.append(&theme_sub_stack);

    // Build the default sub-page ("scheme") the first time the Theme
    // container is shown, so its sub-stack isn't blank — but only when
    // empty, so re-entering Theme keeps whichever sub-page the user last
    // left it on (the sub-stack retains its visible child otherwise).
    {
        let ensure_theme = ensure_theme.clone();
        let sub_stack = theme_sub_stack.clone();
        theme_page.connect_map(move |_| {
            if sub_stack.visible_child().is_none() {
                ensure_theme("scheme");
                sub_stack.set_visible_child_name("scheme");
            }
        });
    }

    // The Theme container is a plain Box of sub-sidebar + (initially empty)
    // sub-stack, so it stays eager: its sub-sidebar buttons seed the search
    // index that must resolve before Theme is ever opened.
    stack.add_titled(&theme_page, Some("theme"), "Theme");
}
