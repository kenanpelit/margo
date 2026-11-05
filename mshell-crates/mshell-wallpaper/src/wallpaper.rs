use std::time::{Duration, Instant};

use gtk4::prelude::Cast;
use gtk4_layer_shell::{Edge, Layer, LayerShell};
use mshell_cache::wallpaper::{
    WallpaperImage, WallpaperStateStoreFields, current_wallpaper_image, wallpaper_store,
};
use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, WallpaperStoreFields};
use mshell_config::schema::content_fit::ContentFit;
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::gdk;
use relm4::gtk::glib;
use relm4::gtk::prelude::{GtkWindowExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

use crate::handoff::{self, TRANSITION_DURATION_MS};

/// One monitor's wallpaper while mshell draws it: a full-screen background
/// layer window, created only when margo doesn't draw the wallpaper (see
/// [`handoff`]) and destroyed when it takes over — a hidden window would
/// keep its swapchain.
///
/// The component's root is a plain window that is never shown, so the
/// shell can hold and `close()` the controller like every other surface.
#[derive(Debug)]
pub struct WallpaperModel {
    monitor: gdk::Monitor,
    content_fit: ContentFit,
    surface: Option<Surface>,
    /// margo draws the wallpaper: no surface wanted.
    margo_draws: bool,
    /// When the surface's crossfade ends; it is not destroyed before.
    transition_end: Option<Instant>,
    /// A queued destroy (cancelled when margo stops drawing).
    destroy_timer: Option<glib::SourceId>,
    _effects: EffectScope,
}

#[derive(Debug)]
struct Surface {
    window: gtk::Window,
    stack: gtk::Stack,
}

#[derive(Debug)]
pub enum WallpaperInput {
    WallpaperChanged(u64),
    ContentFitChanged(ContentFit),
    MargoDrawsChanged(bool),
    DestroySurface,
}

#[derive(Debug)]
pub enum WallpaperOutput {}

pub struct WallpaperInit {
    pub monitor: gdk::Monitor,
}

#[relm4::component(pub)]
impl Component for WallpaperModel {
    type CommandOutput = ();
    type Input = WallpaperInput;
    type Output = WallpaperOutput;
    type Init = WallpaperInit;

    view! {
        #[root]
        gtk::Window {
            set_decorated: false,
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let mut effects = EffectScope::new();

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let revision = wallpaper_store().revision().get();
            sender_clone.input(WallpaperInput::WallpaperChanged(revision));
        });

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let value = config_manager().config().wallpaper().content_fit().get();
            sender_clone.input(WallpaperInput::ContentFitChanged(value));
        });

        let margo_draws = handoff::margo_draws_signal();
        let sender_clone = sender.clone();
        effects.push(move |_| {
            sender_clone.input(WallpaperInput::MargoDrawsChanged(margo_draws.get()));
        });

        let mut model = WallpaperModel {
            monitor: params.monitor,
            content_fit: config_manager()
                .config()
                .wallpaper()
                .content_fit()
                .get_untracked(),
            surface: None,
            margo_draws: handoff::margo_draws(),
            transition_end: None,
            destroy_timer: None,
            _effects: effects,
        };
        if !model.margo_draws {
            model.show_surface();
        }

        let widgets = view_output!();

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            WallpaperInput::WallpaperChanged(revision) => {
                // margo draws (and crossfades) it; a surface still waiting
                // for its own crossfade to end follows along.
                let Some(surface) = &self.surface else {
                    return;
                };
                let Some(image) = current_wallpaper_image() else {
                    clear_stack(&surface.stack);
                    return;
                };
                let name = format!("wallpaper-{revision}");
                let stack = &surface.stack;
                let widget = make_wallpaper_widget(&image, gtk_content_fit(&self.content_fit));
                let old_child = stack.visible_child();
                let animated = old_child.is_some();
                stack.add_named(&widget, Some(&name));
                transition_stack(stack, &name, old_child);
                if animated {
                    self.transition_end = Some(Instant::now() + transition_with_margin());
                }
                if self.margo_draws {
                    self.schedule_destroy(&sender);
                }
            }
            WallpaperInput::ContentFitChanged(content_fit) => {
                self.content_fit = content_fit;
                let Some(surface) = &self.surface else {
                    return;
                };
                let fit = gtk_content_fit(&self.content_fit);
                let mut child = surface.stack.first_child();
                while let Some(widget) = child {
                    child = widget.next_sibling();
                    if let Some(picture) = widget.downcast_ref::<gtk::Picture>() {
                        picture.set_content_fit(fit);
                    }
                }
            }
            WallpaperInput::MargoDrawsChanged(draws) => {
                if draws == self.margo_draws && (draws || self.surface.is_some()) {
                    return;
                }
                self.margo_draws = draws;
                if draws {
                    self.schedule_destroy(&sender);
                } else {
                    if let Some(timer) = self.destroy_timer.take() {
                        timer.remove();
                    }
                    if self.surface.is_none() {
                        self.show_surface();
                    }
                }
            }
            WallpaperInput::DestroySurface => {
                self.destroy_timer = None;
                if self.margo_draws
                    && let Some(surface) = self.surface.take()
                {
                    surface.window.destroy();
                }
            }
        }
    }

    fn shutdown(&mut self, _widgets: &mut Self::Widgets, _output: relm4::Sender<Self::Output>) {
        if let Some(timer) = self.destroy_timer.take() {
            timer.remove();
        }
        if let Some(surface) = self.surface.take() {
            surface.window.destroy();
        }
    }
}

impl WallpaperModel {
    /// Map the layer window showing the current image at once (a first
    /// stack child appears without a transition).
    fn show_surface(&mut self) {
        let surface = build_surface(&self.monitor);
        if let Some(image) = current_wallpaper_image() {
            let widget = make_wallpaper_widget(&image, gtk_content_fit(&self.content_fit));
            let revision = wallpaper_store().revision().get_untracked();
            surface
                .stack
                .add_named(&widget, Some(&format!("wallpaper-{revision}")));
        }
        surface.window.set_visible(true);
        self.transition_end = None;
        self.surface = Some(surface);
    }

    /// Destroy the surface once its crossfade (if any) is over.
    fn schedule_destroy(&mut self, sender: &ComponentSender<Self>) {
        if self.surface.is_none() {
            return;
        }
        if let Some(timer) = self.destroy_timer.take() {
            timer.remove();
        }
        let wait = destroy_delay(self.transition_end, Instant::now());
        let sender = sender.clone();
        self.destroy_timer = Some(glib::timeout_add_local_once(wait, move || {
            sender.input(WallpaperInput::DestroySurface);
        }));
    }
}

/// The crossfade plus the margin `transition_stack` waits before dropping
/// the old child.
fn transition_with_margin() -> Duration {
    Duration::from_millis(u64::from(TRANSITION_DURATION_MS) + 50)
}

/// How long to keep a surface whose crossfade ends at `end`.
pub(crate) fn destroy_delay(end: Option<Instant>, now: Instant) -> Duration {
    end.map_or(Duration::ZERO, |end| end.saturating_duration_since(now))
}

fn build_surface(monitor: &gdk::Monitor) -> Surface {
    let window = gtk::Window::new();
    window.add_css_class("wallpaper-window");
    window.set_decorated(false);
    window.init_layer_shell();
    window.set_monitor(Some(monitor));
    window.set_namespace(Some("mshell-wallpaper"));
    window.set_layer(Layer::Background);
    window.set_exclusive_zone(-1);
    for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
        window.set_anchor(edge, true);
    }
    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    stack.set_transition_duration(TRANSITION_DURATION_MS);
    stack.set_hexpand(true);
    stack.set_vexpand(true);
    window.set_child(Some(&stack));
    Surface { window, stack }
}

fn clear_stack(stack: &gtk::Stack) {
    while let Some(child) = stack.first_child() {
        stack.remove(&child);
    }
}

fn transition_stack(stack: &gtk::Stack, new_name: &str, old_child: Option<gtk::Widget>) {
    let stack_clone = stack.clone();
    let new_name = new_name.to_string();
    glib::idle_add_local_once(move || {
        stack_clone.set_visible_child_name(&new_name);

        if let Some(old) = old_child {
            let stack_clone2 = stack_clone.clone();
            glib::timeout_add_local_once(transition_with_margin(), move || {
                if old.parent().as_ref() == Some(stack_clone2.upcast_ref()) {
                    stack_clone2.remove(&old);
                }
            });
        }
    });
}

fn make_wallpaper_widget(image: &WallpaperImage, content_fit: gtk::ContentFit) -> gtk::Widget {
    // The texture is shared by every monitor's wallpaper window and the lock
    // screen — no per-surface pixel copy.
    let picture = gtk::Picture::for_paintable(&image.texture);
    picture.set_hexpand(true);
    picture.set_vexpand(true);
    picture.set_content_fit(content_fit);
    picture.set_can_shrink(true);
    picture.upcast()
}

fn gtk_content_fit(content_fit: &ContentFit) -> gtk::ContentFit {
    match content_fit {
        ContentFit::Contain => gtk::ContentFit::Contain,
        ContentFit::Cover => gtk::ContentFit::Cover,
        ContentFit::Fill => gtk::ContentFit::Fill,
        ContentFit::ScaleDown => gtk::ContentFit::ScaleDown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_surface_outlives_its_crossfade_only() {
        let now = Instant::now();
        assert_eq!(destroy_delay(None, now), Duration::ZERO);
        assert_eq!(
            destroy_delay(Some(now - Duration::from_millis(5)), now),
            Duration::ZERO
        );
        assert_eq!(
            destroy_delay(Some(now + Duration::from_millis(120)), now),
            Duration::from_millis(120)
        );
    }
}
