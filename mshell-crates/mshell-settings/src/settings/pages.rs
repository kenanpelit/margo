//! Settings pages: the lazy page registry types, building a top-level page
//! on first navigation, and mounting / refreshing it in the stack.

use super::*;

/// A lazily-built page: `(widget, keep-alive tokens)`. Its relm4 `Controller`s
/// are launched on first navigation, not at panel-build time; the tokens (the
/// controllers) must outlive the panel, so the caller parks them in
/// `SettingsWindowModel::alive`.
pub(super) type BuiltPage = (gtk::Widget, Vec<Box<dyn std::any::Any>>);

/// A one-shot factory for a lazily-built sub-page (theme / widgets sub-stack).
/// Re-runnable so Defaults can rebuild the page in place.
pub(super) type PageFactory = Rc<dyn Fn() -> BuiltPage>;

/// Keep-alive tokens of every built page, by route. Rebuilding a page
/// replaces (and so drops) its old controllers.
pub(super) type Alive = Rc<RefCell<HashMap<String, Vec<Box<dyn std::any::Any>>>>>;

/// Routes of built pages whose values may be stale: the config (or the
/// active profile) changed since they were built. Many pages read the
/// config once at build time, so another monitor's Settings, a profile
/// switch, `mshellctl` or a hand edit left them showing — and, on the next
/// spin-button step, writing back — old values. A stale page is rebuilt from
/// the live config the next time it is navigated to or the panel is shown.
pub(super) type Stale = Rc<RefCell<std::collections::HashSet<String>>>;

/// Whether navigating to `route` must (re)build its page: it was never built,
/// or it went stale. Clears the stale mark it consumes.
pub(super) fn needs_build(exists: bool, stale: &Stale, route: &str) -> bool {
    let was_stale = stale.borrow_mut().remove(route);
    !exists || was_stale
}

/// Build a page into `stack` as child `name` — replacing an existing child
/// of that name — then decorate it (`page_defaults`) and park its controllers
/// under `route`. `show` makes it the visible child (used by rebuilds).
pub(super) fn mount_page(
    stack: &gtk::Stack,
    name: &str,
    route: &str,
    build: Rc<dyn Fn() -> Option<BuiltPage>>,
    alive: &Alive,
    show: bool,
) {
    use gtk::prelude::*;
    let Some((widget, tokens)) = build() else {
        return;
    };
    if !route.contains('/') {
        sync_page_hero(&widget, route);
    }
    let rebuild: Rc<dyn Fn()> = {
        let stack = stack.downgrade();
        let (name, route) = (name.to_string(), route.to_string());
        let (build, alive) = (build.clone(), alive.clone());
        Rc::new(move || {
            if let Some(stack) = stack.upgrade() {
                mount_page(&stack, &name, &route, build.clone(), &alive, true);
            }
        })
    };
    crate::page_defaults::decorate(&widget, route, rebuild);
    if let Some(old) = stack.child_by_name(name) {
        stack.remove(&old);
    }
    // The panel drives visibility through its own sidebar, never a
    // GtkStackSwitcher, so the page title is unused — add_named avoids
    // threading dead title strings through every builder.
    stack.add_named(&widget, Some(name));
    alive.borrow_mut().insert(route.to_string(), tokens);
    if show {
        stack.set_visible_child_name(name);
    }
}

/// Launch one top-level settings page on demand, keyed by its stack-child
/// route. Every page's relm4 controller lives only for as long as its returned
/// token is kept alive. Returns `None` for the two container routes (`theme` /
/// `widgets`, which are built eagerly because their sub-sidebars seed the
/// search index) and for any unknown route — callers guard on `child_by_name`
/// first, so `None` is a no-op.
pub(super) fn build_top_page(route: &str) -> Option<BuiltPage> {
    // Launch a page whose `init` takes a single unit-struct: one line per page,
    // returning the page widget plus the controller as its keep-alive token.
    macro_rules! page {
        ($model:path => $init:expr) => {{
            let ctrl = <$model>::builder().launch($init).detach();
            let widget: gtk::Widget = ctrl.widget().clone().into();
            (widget, vec![Box::new(ctrl) as Box<dyn std::any::Any>])
        }};
    }
    let built = match route {
        "general" => page!(GeneralSettingsModel => GeneralSettingsInit {}),
        "setup" => page!(SetupSettingsModel => SetupSettingsInit {}),
        "calendar" => page!(CalendarSettingsModel => CalendarSettingsInit {}),
        "about" => page!(AboutSettingsModel => AboutSettingsInit {}),
        "guide" => page!(GuideSettingsModel => GuideSettingsInit {}),
        "animations" => page!(AnimationsSettingsModel => AnimationsSettingsInit {}),
        "window_switcher" => page!(WindowSwitcherModel => WindowSwitcherInit {}),
        "appearance" => page!(AppearanceModel => AppearanceInit {}),
        "effects" => page!(EffectsModel => EffectsInit {}),
        "osd" => page!(OsdSettingsModel => OsdSettingsInit {}),
        "behaviour" => page!(BehaviourModel => BehaviourInit {}),
        "window_rules" => page!(WindowRulesModel => WindowRulesInit {}),
        "layer_rules" => page!(LayerRulesModel => LayerRulesInit {}),
        "tag_rules" => page!(TagRulesModel => TagRulesInit {}),
        "startup_env" => page!(StartupEnvModel => StartupEnvInit {}),
        "backup" => page!(BackupSettingsModel => BackupSettingsInit {}),
        "logging" => page!(LoggingModel => LoggingInit {}),
        "overview" => page!(OverviewSettingsModel => OverviewSettingsInit {}),
        "vpn" => page!(VpnSettingsModel => VpnSettingsInit {}),
        "ai" => page!(AiSettingsModel => AiSettingsInit {}),
        "translate" => page!(TranslateSettingsModel => TranslateSettingsInit {}),
        "date_time" => page!(DateTimeSettingsModel => DateTimeSettingsInit {}),
        "region" => page!(RegionSettingsModel => RegionSettingsInit {}),
        "sound" => page!(SoundSettingsModel => SoundSettingsInit {}),
        "users" => page!(UsersSettingsModel => UsersSettingsInit {}),
        "input" => page!(InputSettingsModel => InputSettingsInit {}),
        "keybinds" => page!(KeybindsSettingsModel => KeybindsSettingsInit {}),
        "summon" => {
            page!(crate::summon_settings::SummonSettingsModel => crate::summon_settings::SummonSettingsInit {})
        }
        "display" => page!(DisplaySettingsModel => DisplaySettingsInit {}),
        "bluetooth" => page!(BluetoothSettingsModel => BluetoothSettingsInit {}),
        "default_apps" => page!(DefaultAppsSettingsModel => DefaultAppsSettingsInit {}),
        "network" => page!(NetworkSettingsModel => NetworkSettingsInit {}),
        "power" => page!(PowerSettingsModel => PowerSettingsInit {}),
        "privacy" => page!(PrivacySettingsModel => PrivacySettingsInit {}),
        "idle" => page!(IdleSettingsModel => IdleSettingsInit {}),
        "mtune" => page!(MtuneSettingsModel => MtuneSettingsInit {}),
        "toasts" => page!(ToastSettingsModel => ToastSettingsInit {}),
        "game_mode" => page!(GameModeSettingsModel => GameModeSettingsInit {}),
        "keyboard" => page!(KeyboardSettingsModel => KeyboardSettingsInit {}),
        "lock" => page!(LockSettingsModel => LockSettingsInit {}),
        "login" => page!(LoginSettingsModel => LoginSettingsInit {}),
        "tiling_layout" => page!(TagLayoutSettingsModel => TagLayoutSettingsInit {}),
        "launcher" => page!(LauncherSettingsModel => LauncherSettingsInit {}),
        "bar" => page!(BarSettingsModel => BarSettingsInit {}),
        "plugins" => page!(PluginsSettingsModel => PluginsSettingsInit {}),
        "menus" => page!(MenuSettingsModel => MenuSettingsInit {}),
        _ => return None,
    };
    Some(built)
}

/// Make a top-level page's hero (icon + title) say exactly what its sidebar
/// row says, so a renamed or re-iconed row can't drift from the page it
/// opens. Only touches the standard layout — a `.settings-page` box reached
/// through first children whose first child is the `.settings-hero`; pages
/// with their own inner sidebar (Display) are left alone.
pub(super) fn sync_page_hero(page: &gtk::Widget, route: &str) {
    use gtk::prelude::*;
    let Some((icon, label)) = SIDEBAR.iter().find_map(|e| match e {
        SidebarEntry::Page {
            route: r,
            icon,
            label,
        } if *r == route => Some((*icon, *label)),
        _ => None,
    }) else {
        return;
    };
    let mut node = Some(page.clone());
    for _ in 0..4 {
        match node {
            Some(ref w) if !w.has_css_class("settings-page") => node = w.first_child(),
            _ => break,
        }
    }
    let Some(hero) = node
        .filter(|w| w.has_css_class("settings-page"))
        .and_then(|w| w.first_child())
        .filter(|w| w.has_css_class("settings-hero"))
    else {
        return;
    };
    let mut stack = vec![hero];
    while let Some(w) = stack.pop() {
        if w.has_css_class("settings-hero-icon")
            && let Some(img) = w.downcast_ref::<gtk::Image>()
        {
            img.set_icon_name(Some(icon));
        } else if w.has_css_class("settings-hero-title")
            && let Some(lbl) = w.downcast_ref::<gtk::Label>()
        {
            lbl.set_label(label);
        }
        let mut child = w.first_child();
        while let Some(c) = child {
            child = c.next_sibling();
            stack.push(c);
        }
    }
}

#[cfg(test)]
mod stale_tests {
    use super::*;

    /// A page is built once, then rebuilt only after it went stale.
    #[test]
    fn stale_pages_rebuild_once_on_navigation() {
        let stale: Stale = Rc::new(RefCell::new(std::collections::HashSet::new()));
        assert!(needs_build(false, &stale, "dock"), "first visit builds");
        assert!(!needs_build(true, &stale, "dock"), "built and fresh");
        stale.borrow_mut().insert("dock".into());
        assert!(needs_build(true, &stale, "dock"), "config changed: rebuild");
        assert!(!needs_build(true, &stale, "dock"), "once");
    }
}
