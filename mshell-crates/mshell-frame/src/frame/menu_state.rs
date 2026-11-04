//! Menu state for `Frame`: which menus are revealed, keyboard mode, building
//! a menu on first open and toggling menus.

use super::*;

impl Frame {
    pub(super) fn any_menu_revealed(&self) -> bool {
        self.left_revealed
            || self.right_revealed
            || self.top_revealed
            || self.top_left_revealed
            || self.top_right_revealed
            || self.bottom_revealed
            || self.bottom_left_revealed
            || self.bottom_right_revealed
    }

    /// Switch the frame's layer-shell keyboard interactivity to track
    /// the current menu state. We need `Exclusive` while a menu is
    /// open so the compositor actually delivers keys to mshell — with
    /// margo's `compute_desired_focus`, anything weaker (OnDemand,
    /// None) gets the focus stolen back by the active toplevel
    /// window between the pointer click and the next refresh, and
    /// the ESC shortcut on the frame never fires. When no menus are
    /// open we go back to `None` so the user's toplevels keep the
    /// keyboard.
    ///
    /// **Debounced.** Each call (re)arms a 90 ms glib timer; only the
    /// last menu-state determines the applied mode. Without this,
    /// rapid widget clicks (open → close → open …) submit a flurry
    /// of `set_keyboard_interactivity` changes; each commit forces
    /// margo to `arrange()` the layer map, ack a fresh configure,
    /// recompute focus, and the bar visibly drops out for ~1 frame
    /// every cycle — the user sees the bar entirely disappear when
    /// clicking widgets in quick succession.
    pub(super) fn sync_keyboard_mode(&self, root: &<Self as Component>::Root) {
        let desired = if self.any_menu_revealed() {
            gtk4_layer_shell::KeyboardMode::Exclusive
        } else {
            gtk4_layer_shell::KeyboardMode::None
        };

        // Cancel any pending switch and replace it with the new one.
        if let Some(id) = self.pending_kbd_mode_timeout.borrow_mut().take() {
            id.remove();
        }
        *self.pending_kbd_mode.borrow_mut() = Some(desired);

        let root_weak = root.downgrade();
        let pending_mode = self.pending_kbd_mode.clone();
        let pending_timeout = self.pending_kbd_mode_timeout.clone();
        let id =
            gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(90), move || {
                *pending_timeout.borrow_mut() = None;
                let Some(mode) = pending_mode.borrow_mut().take() else {
                    return;
                };
                let Some(root) = root_weak.upgrade() else {
                    return;
                };
                root.set_keyboard_mode(mode);
                // Force a surface commit so the new keyboard_interactivity
                // actually reaches the compositor. When a menu is closed via
                // IPC (`mshellctl dock toggle`) rather than a keypress, there is
                // no further input/render to flush the deferred layer-shell
                // state, so margo never sees the Exclusive→None flip and keeps
                // the keyboard until the next event (the user had to press Esc).
                root.queue_draw();
                tracing::debug!(?mode, "frame: sync_keyboard_mode (applied)");
            });
        *self.pending_kbd_mode_timeout.borrow_mut() = Some(id);
    }

    /// True if a menu by `name` is the visible child of any stack
    /// AND that stack is currently revealed on this frame. Used by
    /// the idempotent `Close*Menu` paths to skip the toggle when
    /// the menu isn't actually showing here.
    pub(super) fn is_menu_visible_now(&self, name: &str, widgets: &FrameWidgets) -> bool {
        let stacks = [
            (&widgets.left_stack, self.left_revealed),
            (&widgets.right_stack, self.right_revealed),
            (&widgets.top_stack, self.top_revealed),
            (&widgets.top_left_stack, self.top_left_revealed),
            (&widgets.top_right_stack, self.top_right_revealed),
            (&widgets.bottom_stack, self.bottom_revealed),
            (&widgets.bottom_left_stack, self.bottom_left_revealed),
            (&widgets.bottom_right_stack, self.bottom_right_revealed),
        ];
        stacks.iter().any(|(stack, revealed)| {
            *revealed && stack.visible_child_name().map(|n| n.to_string()) == Some(name.to_string())
        })
    }

    /// Build a uniform [`MenuId`] menu + attach it to its configured stack on
    /// first open, mirroring [`ensure_settings_built`]. Deferred from `init`
    /// because each menu shell is a relm4 `Controller` plus its config effects
    /// and one Frame exists per monitor (see the `menus` field doc).
    /// Idempotent: a no-op once the menu is in the map. `toggle_menu` finds the
    /// menu by looking it up as a stack child, so this must run before any
    /// toggle so the widget is present in its stack.
    pub(super) fn ensure_menu_built(
        &mut self,
        sender: &ComponentSender<Self>,
        id: MenuId,
        widgets: &FrameWidgets,
    ) {
        let name = id.menu_name();
        if self.menus.contains_key(name) {
            return;
        }
        let controller = Self::build_menu(sender, id.menu_type());
        let widget: Widget = controller.widget().clone().upcast();
        Self::add_to_stack(widgets, &widget, name, &menu_position(name));
        self.menus.insert(name, controller);
    }

    /// Build the Settings panel + attach it to its stack on first open.
    /// Deferred from `init` because the panel launches ~48 page
    /// controllers and one Frame exists per monitor (see the
    /// `settings_menu` field doc). Idempotent: a no-op once built.
    pub(super) fn ensure_settings_built(&mut self, widgets: &FrameWidgets) {
        if self.settings_menu.is_some() {
            return;
        }
        let controller = mshell_settings::SettingsWindowModel::builder()
            .launch(mshell_settings::SettingsWindowInit {
                monitor: Some(self.monitor.clone()),
            })
            .detach();
        let widget: Widget = controller.widget().clone().upcast();
        let position = mshell_config::config_manager::config_manager()
            .config()
            .menus()
            .settings_menu()
            .position()
            .get();
        Self::add_to_stack(widgets, &widget, SETTINGS_MENU, &position);
        self.settings_menu = Some(controller);
    }

    pub(super) fn toggle_menu(&mut self, name: &str, widgets: &mut FrameWidgets) {
        let mut now_visible = true;
        let in_left = widgets.left_stack.child_by_name(name).is_some();
        let in_right = widgets.right_stack.child_by_name(name).is_some();
        let in_top = widgets.top_stack.child_by_name(name).is_some();
        let in_top_left = widgets.top_left_stack.child_by_name(name).is_some();
        let in_top_right = widgets.top_right_stack.child_by_name(name).is_some();
        let in_bottom = widgets.bottom_stack.child_by_name(name).is_some();
        let in_bottom_left = widgets.bottom_left_stack.child_by_name(name).is_some();
        let in_bottom_right = widgets.bottom_right_stack.child_by_name(name).is_some();

        let left_revealed = self.left_revealed;
        let right_revealed = self.right_revealed;
        let top_revealed = self.top_revealed;
        let top_left_revealed = self.top_left_revealed;
        let top_right_revealed = self.top_right_revealed;
        let bottom_revealed = self.bottom_revealed;
        let bottom_left_revealed = self.bottom_left_revealed;
        let bottom_right_revealed = self.bottom_right_revealed;

        self.left_revealed = false;
        self.right_revealed = false;
        self.top_revealed = false;
        self.top_left_revealed = false;
        self.top_right_revealed = false;
        self.bottom_revealed = false;
        self.bottom_left_revealed = false;
        self.bottom_right_revealed = false;

        if in_left {
            if let Some(visible) = widgets.left_stack.visible_child_name() {
                if visible.as_str() == name {
                    self.left_revealed = !left_revealed;
                    now_visible = self.left_revealed;
                } else {
                    widgets
                        .left_stack
                        .set_visible_child_full(name, gtk::StackTransitionType::None);
                    self.left_revealed = true;
                }
            }
        } else if in_right {
            if let Some(visible) = widgets.right_stack.visible_child_name() {
                if visible.as_str() == name {
                    self.right_revealed = !right_revealed;
                    now_visible = self.right_revealed;
                } else {
                    widgets
                        .right_stack
                        .set_visible_child_full(name, gtk::StackTransitionType::None);
                    self.right_revealed = true;
                }
            }
        } else if in_top {
            if let Some(visible) = widgets.top_stack.visible_child_name() {
                if visible.as_str() == name {
                    self.top_revealed = !top_revealed;
                    now_visible = self.top_revealed;
                } else {
                    widgets
                        .top_stack
                        .set_visible_child_full(name, gtk::StackTransitionType::None);
                    self.top_revealed = true;
                }
            }
        } else if in_top_left {
            if let Some(visible) = widgets.top_left_stack.visible_child_name() {
                if visible.as_str() == name {
                    self.top_left_revealed = !top_left_revealed;
                    now_visible = self.top_left_revealed;
                } else {
                    widgets
                        .top_left_stack
                        .set_visible_child_full(name, gtk::StackTransitionType::None);
                    self.top_left_revealed = true;
                }
            }
        } else if in_top_right {
            if let Some(visible) = widgets.top_right_stack.visible_child_name() {
                if visible.as_str() == name {
                    self.top_right_revealed = !top_right_revealed;
                    now_visible = self.top_right_revealed;
                } else {
                    widgets
                        .top_right_stack
                        .set_visible_child_full(name, gtk::StackTransitionType::None);
                    self.top_right_revealed = true;
                }
            }
        } else if in_bottom {
            if let Some(visible) = widgets.bottom_stack.visible_child_name() {
                if visible.as_str() == name {
                    self.bottom_revealed = !bottom_revealed;
                    now_visible = self.bottom_revealed;
                } else {
                    widgets
                        .bottom_stack
                        .set_visible_child_full(name, gtk::StackTransitionType::None);
                    self.bottom_revealed = true;
                }
            }
        } else if in_bottom_left {
            if let Some(visible) = widgets.bottom_left_stack.visible_child_name() {
                if visible.as_str() == name {
                    self.bottom_left_revealed = !bottom_left_revealed;
                    now_visible = self.bottom_left_revealed;
                } else {
                    widgets
                        .bottom_left_stack
                        .set_visible_child_full(name, gtk::StackTransitionType::None);
                    self.bottom_left_revealed = true;
                }
            }
        } else if in_bottom_right
            && let Some(visible) = widgets.bottom_right_stack.visible_child_name()
        {
            if visible.as_str() == name {
                self.bottom_right_revealed = !bottom_right_revealed;
                now_visible = self.bottom_right_revealed;
            } else {
                widgets
                    .bottom_right_stack
                    .set_visible_child_full(name, gtk::StackTransitionType::None);
                self.bottom_right_revealed = true;
            }
        }

        // Tell every built menu whether it is the one now revealed. The
        // lazy-poll menus (network/IP/DNS/UFW/podman/…) start their refresh
        // I/O only on `RevealChanged(true)`, so the just-opened one must be in
        // this set; the rest get `false` to idle. A never-opened menu isn't
        // built (and wasn't revealed), so skipping it is correct.
        for menu_name in [
            CLOCK_MENU,
            CLIPBOARD_MENU,
            NOTIFICATION_MENU,
            SCREENSHOT_MENU,
            APP_LAUNCHER_MENU,
            WALLPAPER_MENU,
            SESSION_MENU,
            NIP_MENU,
            NVPN_INDICATOR_MENU,
            NDNS_MENU,
            NVPN_MENU,
            NAI_MENU,
            NUFW_MENU,
            NPODMAN_MENU,
            NNETWORK_MENU,
        ] {
            if let Some(menu) = self.menus.get(menu_name) {
                menu.sender()
                    .send(MenuInput::RevealChanged(menu_name == name && now_visible))
                    .unwrap_or_default();
            }
        }

        // Screenshare stays eager (bespoke portal path).
        self.screenshare_menu
            .sender()
            .send(MenuInput::RevealChanged(
                name == SCREENSHARE_MENU && now_visible,
            ))
            .unwrap_or_default();
    }
}
