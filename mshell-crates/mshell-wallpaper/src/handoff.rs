//! Hand the wallpaper to margo.
//!
//! mshell used to paint the wallpaper in one full-screen background layer
//! window per monitor. Under GTK's Vulkan renderer each window keeps four
//! output-sized swapchain buffers for a static image — about 104 MiB of GPU
//! memory (GTT, i.e. system RAM on an iGPU) on a 1920x1200 + 2560x1440 desk.
//! margo can draw the same image from one texture with no swapchain
//! (`dispatch wallpaper_set`, margo's `wallpaper/runtime.rs`).
//!
//! So, while `wallpaper.compositor_draws` is on, the shell writes the
//! displayed (screen-sized, theme-filtered) image to `$XDG_RUNTIME_DIR`
//! (tmpfs), asks margo to draw it with the shell's fit, window background
//! colour and crossfade, deletes the file once margo holds the pixels, and
//! destroys its windows. When margo refuses (older margo, nested session,
//! another compositor, a failed write) the windows stay or come back: the
//! feature is detected by the reply itself and by `get wallpaper`.
//!
//! What stays the same: the image (the same decoded, filtered pixels), every
//! `content_fit`, the `.wallpaper-window` background around a contained
//! image, the 200 ms crossfade on a change (margo reproduces GtkStack's
//! ease-out cubic; the windows only go away once their own crossfade is
//! over), the first image appearing without a fade, the lock screen
//! (painted by the lock surfaces, untouched) and the greeter image
//! (`wallpaper.raw` in the cache, untouched). Rotation and the daily image
//! go through the same image store, so they are handed over like a pick.
//!
//! [`Machine`] holds the decisions and is unit-tested; the GTK/IPC side
//! only executes its [`Action`]s.

use std::cell::RefCell;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

#[allow(deprecated)] // `StyleContext::to_string`: GTK's only style printer.
use gtk4::prelude::{StyleContextExt, WidgetExt};
use mshell_cache::wallpaper::{
    WallpaperStateStoreFields, current_wallpaper_image, wallpaper_store,
};
use mshell_common::css_vars::CssVars;
use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, WallpaperStoreFields};
use mshell_config::schema::content_fit::ContentFit;
use reactive_graph::prelude::{Get, GetUntracked, Set};
use reactive_graph::signal::ArcRwSignal;
use relm4::gtk;
use relm4::gtk::glib;
use tracing::{info, warn};

/// GtkStack's crossfade in the windows (`wallpaper.rs`), reproduced by
/// margo when it draws.
pub const TRANSITION_DURATION_MS: u32 = 200;

/// How long the windows stay after they come back, before margo's copy is
/// cleared: long enough for them to map and paint first, so the change
/// never shows margo's configured wallpaper in between.
const CLEAR_DELAY: Duration = Duration::from_secs(1);

/// The `owner=` tag of every entry mshell sets, so its clears never touch
/// a wallpaper someone else set.
const OWNER: &str = "mshell";

// ── Decisions ───────────────────────────────────────────────────────────

/// How margo should present the image: the window's look.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Look {
    pub fit: ContentFit,
    /// The `.wallpaper-window` background as `#rrggbbaa`, `None` when
    /// transparent or unknown.
    pub bg: Option<String>,
}

/// Something the executor does for the [`Machine`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Action {
    /// Write the image of `revision` and ask margo to draw it.
    Set {
        revision: u64,
        fade_ms: u32,
        look: Look,
    },
    /// Change how margo presents the image it has.
    Update(Look),
    /// Drop margo's copy, after [`CLEAR_DELAY`] (the windows map first).
    Clear,
    /// Tell the per-monitor windows whether margo draws (they go / come
    /// back).
    Publish(bool),
}

/// The hand-off state machine. Every input returns what to do.
#[derive(Debug)]
pub(crate) struct Machine {
    /// `wallpaper.compositor_draws`.
    enabled: bool,
    /// margo accepted the last request: it draws, the windows are gone.
    margo_draws: bool,
    /// `Some(false)` once `get wallpaper` said margo can't draw it (older
    /// margo, nested backend): no more 17 MB writes for nothing.
    supported: Option<bool>,
    /// The set request awaiting margo's reply.
    in_flight: Option<u64>,
    /// The look margo was last given.
    sent_look: Option<Look>,
    /// The look the windows show now.
    look: Look,
    /// Whether this process has had an image accepted yet: the first one
    /// appears without a fade, like the windows' first stack child.
    shown_once: bool,
}

impl Machine {
    pub(crate) fn new(enabled: bool, look: Look) -> Self {
        Self {
            enabled,
            margo_draws: false,
            supported: None,
            in_flight: None,
            sent_look: None,
            look,
            shown_once: false,
        }
    }

    pub(crate) fn margo_draws(&self) -> bool {
        self.margo_draws
    }

    /// Stop drawing in margo: windows back, margo's copy cleared.
    fn fall_back(&mut self) -> Vec<Action> {
        self.in_flight = None;
        self.sent_look = None;
        if !self.margo_draws {
            return Vec::new();
        }
        self.margo_draws = false;
        vec![Action::Publish(false), Action::Clear]
    }

    /// The displayed image changed: `revision` of the store, `None` when
    /// there is no image (the windows then show their bare background).
    pub(crate) fn image_changed(&mut self, revision: Option<u64>, animate: bool) -> Vec<Action> {
        if !self.enabled || self.supported == Some(false) {
            return Vec::new();
        }
        let Some(revision) = revision else {
            return self.fall_back();
        };
        let fade_ms = if self.margo_draws && self.shown_once && animate {
            TRANSITION_DURATION_MS
        } else {
            0
        };
        self.in_flight = Some(revision);
        self.sent_look = Some(self.look.clone());
        vec![Action::Set {
            revision,
            fade_ms,
            look: self.look.clone(),
        }]
    }

    /// margo answered the set request for `revision`.
    pub(crate) fn set_done(&mut self, revision: u64, ok: bool) -> Vec<Action> {
        if self.in_flight != Some(revision) {
            // Superseded (a newer image or a disable came in meanwhile). If
            // margo took it while the feature is off, drop it again.
            return if ok && !self.enabled {
                vec![Action::Clear]
            } else {
                Vec::new()
            };
        }
        self.in_flight = None;
        if !ok {
            return self.fall_back();
        }
        self.shown_once = true;
        let mut actions = Vec::new();
        if !self.margo_draws {
            self.margo_draws = true;
            actions.push(Action::Publish(true));
        }
        if self.sent_look.as_ref() != Some(&self.look) {
            self.sent_look = Some(self.look.clone());
            actions.push(Action::Update(self.look.clone()));
        }
        actions
    }

    /// Fit or window background changed.
    pub(crate) fn look_changed(&mut self, look: Look) -> Vec<Action> {
        if look == self.look {
            return Vec::new();
        }
        self.look = look;
        // With a set in flight, `set_done` sends the difference.
        if !self.margo_draws || self.in_flight.is_some() {
            return Vec::new();
        }
        self.sent_look = Some(self.look.clone());
        vec![Action::Update(self.look.clone())]
    }

    /// margo answered an update.
    pub(crate) fn update_done(&mut self, ok: bool) -> Vec<Action> {
        if ok || !self.margo_draws {
            return Vec::new();
        }
        self.fall_back()
    }

    /// `wallpaper.compositor_draws` toggled; `revision` is the current image.
    pub(crate) fn enabled_changed(&mut self, enabled: bool, revision: Option<u64>) -> Vec<Action> {
        if enabled == self.enabled {
            return Vec::new();
        }
        self.enabled = enabled;
        if enabled {
            self.image_changed(revision, false)
        } else {
            self.fall_back()
        }
    }

    /// The start-up `get wallpaper` probe: whether margo can draw it at all,
    /// and whether it still draws an image an earlier mshell gave it (a
    /// crash or kill skips the clear on quit). The windows map at start-up
    /// as before and cover such a leftover until the first hand-over
    /// replaces it; with the feature off nothing would, so it is cleared.
    pub(crate) fn probed(&mut self, supported: bool, ours_shown: bool) -> Vec<Action> {
        self.supported = Some(supported);
        if !supported {
            return self.fall_back();
        }
        if ours_shown && !self.enabled {
            return vec![Action::Clear];
        }
        Vec::new()
    }

    /// Whether margo's copy may be cleared now: the windows draw and no set
    /// request is on its way (clearing then would drop the new image).
    pub(crate) fn may_clear(&self) -> bool {
        !self.margo_draws && self.in_flight.is_none()
    }
}

// ── Pure helpers ────────────────────────────────────────────────────────

/// margo's name for a `content_fit`.
pub(crate) fn fit_name(fit: &ContentFit) -> &'static str {
    match fit {
        ContentFit::Contain => "contain",
        ContentFit::Cover => "cover",
        ContentFit::Fill => "fill",
        ContentFit::ScaleDown => "scale-down",
    }
}

/// The `fit=… bg=…` options for `look`.
fn look_options(look: &Look) -> String {
    format!(
        "fit={} bg={}",
        fit_name(&look.fit),
        look.bg.as_deref().unwrap_or("none")
    )
}

/// The `wallpaper_set` argument text.
pub(crate) fn set_args(look: &Look, fade_ms: u32, path: &Path) -> String {
    format!(
        "* {} fade={fade_ms} owner={OWNER} {}",
        look_options(look),
        path.display()
    )
}

/// `#rrggbbaa` of unit-float RGBA; `None` when fully transparent.
fn hex_rgba((r, g, b, a): (f64, f64, f64, f64)) -> Option<String> {
    if a <= 0.0 {
        return None;
    }
    let byte = |c: f64| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
    Some(format!(
        "#{:02x}{:02x}{:02x}{:02x}",
        byte(r),
        byte(g),
        byte(b),
        byte(a)
    ))
}

/// The window background in a printed GTK style (`SHOW_STYLE`). The
/// stylesheet paints `.wallpaper-window` with `var(--surface)`
/// (`_wallpaper_window.scss`, pinned by a test), and GTK prints a node's
/// custom properties but not its resolved `background-color`, so this is
/// `--surface`. `None` when transparent or unset (margo then shows its own
/// clear colour around a contained image).
pub(crate) fn background_from_style(css: &str) -> Option<String> {
    hex_rgba(CssVars::parse(css).color("--surface")?)
}

/// Whether a `get wallpaper` reply says margo can draw it, and whether it
/// shows an image mshell set for every output.
pub(crate) fn parse_probe(reply: &serde_json::Value) -> (bool, bool) {
    let supported = reply["supported"].as_bool() == Some(true);
    let ours = reply["entries"].as_array().is_some_and(|entries| {
        entries
            .iter()
            .any(|e| e["outputs"] == "*" && e["owner"] == OWNER)
    });
    (supported, supported && ours)
}

// ── Worker ──────────────────────────────────────────────────────────────

/// Where the worker talks to margo and puts its files. Resolved on the
/// main thread: reading the environment on a worker races any `setenv`.
#[derive(Debug, Clone)]
pub(crate) struct Endpoint {
    pub socket: PathBuf,
    pub dir: Option<PathBuf>,
}

enum Job {
    Probe,
    Set {
        revision: u64,
        width: u32,
        height: u32,
        rgba: glib::Bytes,
        fade_ms: u32,
        look: Look,
    },
    Update(Look),
    Clear,
}

/// What the worker reports back.
enum Done {
    Probe { supported: bool, ours_shown: bool },
    Set { revision: u64, ok: bool },
    Update { ok: bool },
}

/// Write `[u32 LE w][u32 LE h][RGBA]` to `path` (private to the user).
pub(crate) fn write_raw(path: &Path, width: u32, height: u32, rgba: &[u8]) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    if let Some(dir) = path.parent() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(&width.to_le_bytes())?;
    file.write_all(&height.to_le_bytes())?;
    file.write_all(rgba)
}

/// Hand one image to margo: write it, `wallpaper_set` it, delete the file
/// (margo read it before replying). `Err` says why it isn't drawn.
pub(crate) fn hand_over(
    endpoint: &Endpoint,
    revision: u64,
    (width, height, rgba): (u32, u32, &[u8]),
    fade_ms: u32,
    look: &Look,
) -> Result<(), String> {
    let dir = endpoint.dir.as_ref().ok_or("XDG_RUNTIME_DIR is not set")?;
    let path = dir.join(format!("wallpaper-{}-{revision}.raw", std::process::id()));
    if let Err(e) = write_raw(&path, width, height, rgba) {
        let _ = std::fs::remove_file(&path);
        return Err(format!("{}: {e}", path.display()));
    }
    let args = set_args(look, fade_ms, &path);
    let result = mshell_margo_client::ctl::dispatch_at(&endpoint.socket, "wallpaper_set", &[&args]);
    let _ = std::fs::remove_file(&path);
    result.map_err(|e| e.to_string())
}

fn run_job(endpoint: &Endpoint, job: Job) -> Option<Done> {
    use mshell_margo_client::ctl::{dispatch_at, request_at};
    match job {
        Job::Probe => {
            let (supported, ours_shown) = match request_at(&endpoint.socket, "get wallpaper") {
                Ok(reply) => parse_probe(&reply),
                Err(e) => {
                    warn!(error = %e, "wallpaper hand-off: margo not reachable");
                    (false, false)
                }
            };
            Some(Done::Probe {
                supported,
                ours_shown,
            })
        }
        Job::Set {
            revision,
            width,
            height,
            rgba,
            fade_ms,
            look,
        } => {
            let result = hand_over(endpoint, revision, (width, height, &rgba), fade_ms, &look);
            match &result {
                Ok(()) => info!(revision, "wallpaper: margo draws it"),
                Err(e) => warn!(error = %e, "wallpaper: margo can't draw it; mshell does"),
            }
            Some(Done::Set {
                revision,
                ok: result.is_ok(),
            })
        }
        Job::Update(look) => {
            let args = format!("* {}", look_options(&look));
            let result = dispatch_at(&endpoint.socket, "wallpaper_update", &[&args]);
            if let Err(e) = &result {
                warn!(error = %e, "wallpaper: margo refused the update; mshell draws it");
            }
            Some(Done::Update { ok: result.is_ok() })
        }
        Job::Clear => {
            let args = format!("* owner={OWNER}");
            if let Err(e) = dispatch_at(&endpoint.socket, "wallpaper_clear", &[&args]) {
                warn!(error = %e, "wallpaper: clearing margo's copy failed");
            }
            None
        }
    }
}

fn spawn_worker(endpoint: Endpoint) -> Option<mpsc::Sender<Job>> {
    let (tx, rx) = mpsc::channel::<Job>();
    let spawned = std::thread::Builder::new()
        .name("wallpaper-handoff".into())
        .spawn(move || {
            while let Ok(mut job) = rx.recv() {
                // Only the newest image matters; earlier ones are skipped
                // (their replies would be ignored as superseded anyway).
                while let Job::Set { .. } = job {
                    match rx.try_recv() {
                        Ok(next @ Job::Set { .. }) => job = next,
                        Ok(other) => {
                            report(run_job(&endpoint, job));
                            job = other;
                            break;
                        }
                        Err(_) => break,
                    }
                }
                report(run_job(&endpoint, job));
            }
        });
    match spawned {
        Ok(_) => Some(tx),
        Err(e) => {
            warn!("wallpaper hand-off: cannot spawn worker: {e}");
            None
        }
    }
}

/// Deliver a result to the main thread.
fn report(done: Option<Done>) {
    if let Some(done) = done {
        glib::idle_add_once(move || with_controller(|c| c.on_done(done)));
    }
}

// ── Controller (main thread) ────────────────────────────────────────────

struct Controller {
    machine: Machine,
    /// Never shown: resolves the `.wallpaper-window` background with no
    /// window mapped.
    probe: gtk::Window,
    margo_draws: ArcRwSignal<bool>,
    worker: Option<mpsc::Sender<Job>>,
    _effects: EffectScope,
}

thread_local! {
    static CONTROLLER: RefCell<Option<Controller>> = const { RefCell::new(None) };
}

/// Run `f` on the controller (main thread), creating it on first use.
/// Afterwards the published signal follows the machine; it is set outside
/// the borrow, as setting it schedules the windows' effects.
fn with_controller<R>(f: impl FnOnce(&mut Controller) -> R) -> R {
    let (result, publish) = CONTROLLER.with(|cell| {
        let mut slot = cell.borrow_mut();
        let controller = slot.get_or_insert_with(Controller::new);
        let result = f(controller);
        let draws = controller.machine.margo_draws();
        let signal = &controller.margo_draws;
        let publish = (signal.get_untracked() != draws).then(|| (signal.clone(), draws));
        (result, publish)
    });
    if let Some((signal, draws)) = publish {
        signal.set(draws);
    }
    result
}

impl Controller {
    fn new() -> Self {
        let probe = gtk::Window::new();
        probe.add_css_class("wallpaper-window");
        let look = Look {
            fit: config_manager()
                .config()
                .wallpaper()
                .content_fit()
                .get_untracked(),
            bg: background_of(&probe),
        };
        let enabled = config_manager()
            .config()
            .wallpaper()
            .compositor_draws()
            .get_untracked();
        let endpoint = Endpoint {
            socket: mshell_margo_client::state_json::socket_path(),
            dir: margo_fs::xdg::runtime_dir().map(|d| d.join("mshell")),
        };
        let worker = spawn_worker(endpoint);
        if let Some(tx) = &worker {
            let _ = tx.send(Job::Probe);
        }
        Self {
            machine: Machine::new(enabled, look),
            probe,
            margo_draws: ArcRwSignal::new(false),
            worker,
            // Their bodies only queue idle callbacks, so creating them while
            // the controller is being stored is fine.
            _effects: Self::effects(),
        }
    }

    fn effects() -> EffectScope {
        let mut effects = EffectScope::new();
        effects.push(|_| {
            let _revision = wallpaper_store().revision().get();
            glib::idle_add_local_once(|| {
                with_controller(|c| {
                    let revision = current_revision();
                    let actions = c.machine.image_changed(revision, animations_enabled());
                    c.execute(actions);
                });
            });
        });
        effects.push(|_| {
            let fit = config_manager().config().wallpaper().content_fit().get();
            glib::idle_add_local_once(move || {
                with_controller(|c| {
                    let look = Look {
                        fit,
                        bg: c.machine.look.bg.clone(),
                    };
                    let actions = c.machine.look_changed(look);
                    c.execute(actions);
                });
            });
        });
        effects.push(|_| {
            let enabled = config_manager()
                .config()
                .wallpaper()
                .compositor_draws()
                .get();
            glib::idle_add_local_once(move || {
                with_controller(|c| {
                    let actions = c.machine.enabled_changed(enabled, current_revision());
                    c.execute(actions);
                });
            });
        });
        effects
    }

    fn send(&self, job: Job) {
        if let Some(tx) = &self.worker
            && tx.send(job).is_err()
        {
            warn!("wallpaper hand-off: worker is gone");
        }
    }

    fn on_done(&mut self, done: Done) {
        let actions = match done {
            Done::Probe {
                supported,
                ours_shown,
            } => self.machine.probed(supported, ours_shown),
            Done::Set { revision, ok } => self.machine.set_done(revision, ok),
            Done::Update { ok } => self.machine.update_done(ok),
        };
        self.execute(actions);
    }

    fn execute(&mut self, actions: Vec<Action>) {
        for action in actions {
            match action {
                Action::Set {
                    revision,
                    fade_ms,
                    look,
                } => match current_wallpaper_image() {
                    Some(image) => self.send(Job::Set {
                        revision,
                        width: image.width,
                        height: image.height,
                        rgba: image.rgba,
                        fade_ms,
                        look,
                    }),
                    None => {
                        let more = self.machine.set_done(revision, false);
                        self.execute(more);
                    }
                },
                Action::Update(look) => self.send(Job::Update(look)),
                Action::Clear => {
                    glib::timeout_add_local_once(CLEAR_DELAY, || {
                        with_controller(|c| {
                            if c.machine.may_clear() {
                                c.send(Job::Clear);
                            }
                        });
                    });
                }
                // Published by `with_controller` once the borrow ends.
                Action::Publish(_) => {}
            }
        }
    }
}

/// The store's revision when an image is shown, else `None`.
fn current_revision() -> Option<u64> {
    current_wallpaper_image()?;
    Some(wallpaper_store().revision().get_untracked())
}

fn animations_enabled() -> bool {
    gtk::Settings::default().is_none_or(|s| s.is_gtk_enable_animations())
}

fn background_of(widget: &gtk::Window) -> Option<String> {
    #[allow(deprecated)]
    let css = widget
        .style_context()
        .to_string(gtk::StyleContextPrintFlags::SHOW_STYLE);
    background_from_style(css.as_str())
}

// ── Public API ──────────────────────────────────────────────────────────

/// Whether margo draws the wallpaper now (the windows stay away). Tracked:
/// read it in an effect to follow changes. Main thread.
pub fn margo_draws_signal() -> ArcRwSignal<bool> {
    with_controller(|c| c.margo_draws.clone())
}

/// Whether margo draws the wallpaper now, untracked. Main thread.
pub fn margo_draws() -> bool {
    with_controller(|c| c.machine.margo_draws())
}

/// The shell's stylesheet changed: re-read the window background and
/// pass it on. Main thread.
pub fn style_changed() {
    with_controller(|c| {
        let look = Look {
            fit: c.machine.look.fit.clone(),
            bg: background_of(&c.probe),
        };
        let actions = c.machine.look_changed(look);
        c.execute(actions);
    });
}

/// Before the shell exits: margo stops drawing mshell's wallpaper, as the
/// windows vanished with the process before. Blocking, bounded by the IPC
/// timeout. Main thread.
pub fn release() {
    let draws = CONTROLLER.with(|cell| cell.borrow().as_ref().map(|c| c.machine.margo_draws()));
    if draws != Some(true) {
        return;
    }
    let args = format!("* owner={OWNER}");
    let socket = mshell_margo_client::state_json::socket_path();
    if let Err(e) = mshell_margo_client::ctl::dispatch_at(&socket, "wallpaper_clear", &[&args]) {
        warn!(error = %e, "wallpaper: clearing margo's copy on exit failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::os::unix::net::UnixListener;

    fn look(fit: ContentFit) -> Look {
        Look {
            fit,
            bg: Some("#282a36ff".into()),
        }
    }

    fn set(revision: u64, fade_ms: u32, look: Look) -> Action {
        Action::Set {
            revision,
            fade_ms,
            look,
        }
    }

    #[test]
    fn the_first_image_is_handed_over_without_a_fade_then_the_windows_go() {
        let mut m = Machine::new(true, look(ContentFit::Cover));
        // The windows show it first; margo gets the same image under them.
        assert_eq!(
            m.image_changed(Some(1), true),
            [set(1, 0, look(ContentFit::Cover))]
        );
        assert!(!m.margo_draws());
        assert_eq!(m.set_done(1, true), [Action::Publish(true)]);
        assert!(m.margo_draws());
    }

    #[test]
    fn later_changes_crossfade_in_margo_like_the_stack() {
        let mut m = Machine::new(true, look(ContentFit::Cover));
        m.image_changed(Some(1), true);
        m.set_done(1, true);
        assert_eq!(
            m.image_changed(Some(2), true),
            [set(2, TRANSITION_DURATION_MS, look(ContentFit::Cover))]
        );
        assert_eq!(
            m.set_done(2, true),
            [],
            "already drawing: nothing to publish"
        );
        // gtk-enable-animations off: GTK switches at once, so does margo.
        assert_eq!(
            m.image_changed(Some(3), false),
            [set(3, 0, look(ContentFit::Cover))]
        );
    }

    #[test]
    fn a_refusal_brings_the_windows_back_and_clears_margo() {
        let mut m = Machine::new(true, look(ContentFit::Cover));
        // Older margo: it never drew, so the windows just stay.
        m.image_changed(Some(1), true);
        assert_eq!(m.set_done(1, false), []);
        assert!(!m.margo_draws());
        // margo drew, then refused a change: windows back, its copy dropped.
        m.image_changed(Some(2), true);
        m.set_done(2, true);
        m.image_changed(Some(3), true);
        assert_eq!(
            m.set_done(3, false),
            [Action::Publish(false), Action::Clear]
        );
        assert!(!m.margo_draws());
        // The next change is tried again without a fade (the windows show it).
        assert_eq!(
            m.image_changed(Some(4), true),
            [set(4, 0, look(ContentFit::Cover))]
        );
    }

    #[test]
    fn a_superseded_reply_is_ignored() {
        let mut m = Machine::new(true, look(ContentFit::Cover));
        m.image_changed(Some(1), true);
        m.image_changed(Some(2), true);
        assert_eq!(m.set_done(1, true), [], "revision 2 is on its way");
        assert!(!m.margo_draws());
        assert_eq!(m.set_done(1, false), []);
        assert_eq!(m.set_done(2, true), [Action::Publish(true)]);
    }

    #[test]
    fn fit_and_background_follow_by_update() {
        let mut m = Machine::new(true, look(ContentFit::Cover));
        // Not drawing: the windows apply it themselves.
        assert_eq!(m.look_changed(look(ContentFit::Fill)), []);
        m.image_changed(Some(1), true);
        // A change while the set is in flight is sent once margo took the set.
        assert_eq!(m.look_changed(look(ContentFit::Contain)), []);
        assert_eq!(
            m.set_done(1, true),
            [
                Action::Publish(true),
                Action::Update(look(ContentFit::Contain))
            ]
        );
        let dark = Look {
            fit: ContentFit::Contain,
            bg: Some("#000000ff".into()),
        };
        assert_eq!(m.look_changed(dark.clone()), [Action::Update(dark.clone())]);
        assert_eq!(m.look_changed(dark), [], "unchanged: nothing sent");
        assert_eq!(m.update_done(true), []);
        assert_eq!(
            m.update_done(false),
            [Action::Publish(false), Action::Clear]
        );
    }

    #[test]
    fn toggling_the_setting_switches_who_draws() {
        let mut m = Machine::new(false, look(ContentFit::Cover));
        assert_eq!(m.image_changed(Some(1), true), [], "off: mshell draws");
        assert_eq!(
            m.enabled_changed(true, Some(1)),
            [set(1, 0, look(ContentFit::Cover))]
        );
        m.set_done(1, true);
        assert_eq!(
            m.enabled_changed(false, Some(1)),
            [Action::Publish(false), Action::Clear]
        );
        assert_eq!(m.enabled_changed(false, Some(1)), []);
        // A set margo accepts after the switch-off is cleared again.
        m.enabled_changed(true, Some(2));
        m.enabled_changed(false, Some(2));
        assert_eq!(m.set_done(2, true), [Action::Clear]);
        assert!(!m.margo_draws());
    }

    #[test]
    fn no_image_means_the_windows_bare_background() {
        let mut m = Machine::new(true, look(ContentFit::Cover));
        m.image_changed(Some(1), true);
        m.set_done(1, true);
        assert_eq!(
            m.image_changed(None, true),
            [Action::Publish(false), Action::Clear]
        );
    }

    #[test]
    fn the_probe_stops_useless_writes_and_skips_the_startup_flash() {
        let mut old = Machine::new(true, look(ContentFit::Cover));
        assert_eq!(old.probed(false, false), []);
        assert_eq!(old.image_changed(Some(1), true), [], "margo can't draw it");

        // An earlier mshell's image is still up: the windows cover it until
        // the first hand-over replaces it (no fade: this process's first).
        let mut m = Machine::new(true, look(ContentFit::Cover));
        assert_eq!(m.probed(true, true), []);
        assert!(!m.margo_draws());
        assert_eq!(
            m.image_changed(Some(1), true),
            [set(1, 0, look(ContentFit::Cover))]
        );
        // With the feature off nothing would replace it: cleared.
        let mut off = Machine::new(false, look(ContentFit::Cover));
        assert_eq!(off.probed(true, true), [Action::Clear]);
        let mut empty = Machine::new(false, look(ContentFit::Cover));
        assert_eq!(empty.probed(true, false), []);
    }

    #[test]
    fn margo_is_never_cleared_under_a_set_on_its_way() {
        let mut m = Machine::new(true, look(ContentFit::Cover));
        m.image_changed(Some(1), true);
        m.set_done(1, true);
        m.image_changed(Some(2), true);
        m.set_done(2, false);
        assert!(m.may_clear(), "windows draw, nothing in flight");
        // A new image within the clear delay: the queued clear must not run.
        m.image_changed(Some(3), true);
        assert!(!m.may_clear());
        m.set_done(3, true);
        assert!(!m.may_clear(), "margo draws again");
    }

    #[test]
    fn every_content_fit_has_a_margo_name() {
        let names: Vec<&str> = ContentFit::all().iter().map(fit_name).collect();
        assert_eq!(names, ["contain", "cover", "fill", "scale-down"]);
    }

    #[test]
    fn set_args_carry_fit_background_fade_and_owner() {
        let path = Path::new("/run/user/1000/mshell/wallpaper-7-3.raw");
        assert_eq!(
            set_args(&look(ContentFit::ScaleDown), 200, path),
            "* fit=scale-down bg=#282a36ff fade=200 owner=mshell /run/user/1000/mshell/wallpaper-7-3.raw"
        );
        let bare = Look {
            fit: ContentFit::Cover,
            bg: None,
        };
        assert!(set_args(&bare, 0, path).starts_with("* fit=cover bg=none fade=0 "));
    }

    #[test]
    fn the_window_background_is_read_from_the_printed_style() {
        // What GTK 4 prints for the probe window (seen under Xvfb): the
        // custom properties, not the resolved `background-color`.
        let printed = "[window.background.wallpaper-window:dir(ltr)]\n  --surface: #282A36;\n";
        assert_eq!(background_from_style(printed).as_deref(), Some("#282a36ff"));
        let translucent = "  --surface: rgba(0, 0, 0, 0.5);\n";
        assert_eq!(
            background_from_style(translucent).as_deref(),
            Some("#00000080")
        );
        let clear = "  --surface: rgba(0, 0, 0, 0);\n";
        assert_eq!(
            background_from_style(clear),
            None,
            "transparent: margo's clear shows"
        );
        let indirect = "  --surface: var(--base);\n  --base: #102030;\n";
        assert_eq!(
            background_from_style(indirect).as_deref(),
            Some("#102030ff")
        );
        assert_eq!(background_from_style(""), None);
    }

    #[test]
    fn the_stylesheet_paints_the_window_with_the_variable_margo_is_given() {
        // `background_from_style` reads `--surface`; if the window's colour
        // comes from anywhere else, margo would paint the wrong one.
        let scss = include_str!("../../mshell-style/scss/06-windows/_wallpaper_window.scss");
        let rule: String = scss.split_whitespace().collect();
        assert_eq!(rule, ".wallpaper-window{background-color:var(--surface);}");
    }

    #[test]
    fn the_probe_reply_tells_support_and_ownership() {
        use serde_json::json;
        let ours = json!({ "supported": true, "entries": [{ "outputs": "*", "owner": "mshell" }] });
        assert_eq!(parse_probe(&ours), (true, true));
        let theirs = json!({ "supported": true, "entries": [{ "outputs": "*", "owner": "" }] });
        assert_eq!(parse_probe(&theirs), (true, false));
        let one_output =
            json!({ "supported": true, "entries": [{ "outputs": "DP-3", "owner": "mshell" }] });
        assert_eq!(parse_probe(&one_output), (true, false));
        let nested = json!({ "supported": false, "entries": [] });
        assert_eq!(parse_probe(&nested), (false, false));
        let old = json!({ "error": "unknown topic: wallpaper" });
        assert_eq!(parse_probe(&old), (false, false));
    }

    /// A one-shot fake margo: records the request line, checks the file it
    /// names, answers `reply`.
    fn fake_margo(
        dir: &Path,
        reply: &'static str,
    ) -> std::thread::JoinHandle<(String, Option<Vec<u8>>)> {
        let listener = UnixListener::bind(dir.join("margo-ipc.sock")).expect("bind");
        std::thread::spawn(move || {
            let (sock, _) = listener.accept().expect("accept");
            let mut reader = BufReader::new(sock.try_clone().expect("clone"));
            let mut line = String::new();
            reader.read_line(&mut line).expect("read");
            let file = line
                .split_whitespace()
                .last()
                .and_then(|p| std::fs::read(p).ok());
            (&sock).write_all(reply.as_bytes()).expect("reply");
            (line, file)
        })
    }

    #[test]
    fn hand_over_writes_the_raw_image_and_deletes_it_after_the_reply() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let endpoint = Endpoint {
            socket: tmp.path().join("margo-ipc.sock"),
            dir: Some(tmp.path().join("mshell")),
        };
        let server = fake_margo(tmp.path(), "{\"ok\":true}\n");
        let pixels = [1u8, 2, 3, 4, 5, 6, 7, 8];
        hand_over(
            &endpoint,
            9,
            (2, 1, &pixels),
            200,
            &look(ContentFit::Contain),
        )
        .expect("accepted");
        let (line, file) = server.join().expect("server");
        assert!(
            line.starts_with(
                "dispatch wallpaper_set * fit=contain bg=#282a36ff fade=200 owner=mshell "
            ),
            "{line}"
        );
        let file = file.expect("the file existed while margo read it");
        assert_eq!(&file[..8], &[2, 0, 0, 0, 1, 0, 0, 0]);
        assert_eq!(&file[8..], &pixels);
        let left = std::fs::read_dir(tmp.path().join("mshell"))
            .expect("dir")
            .count();
        assert_eq!(left, 0, "deleted once margo holds the pixels");
    }

    #[test]
    fn a_refusal_or_no_runtime_dir_is_an_error_and_leaves_no_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let endpoint = Endpoint {
            socket: tmp.path().join("margo-ipc.sock"),
            dir: Some(tmp.path().join("mshell")),
        };
        let server = fake_margo(
            tmp.path(),
            "{\"ok\":false,\"error\":\"unknown action: wallpaper_set\"}\n",
        );
        let err =
            hand_over(&endpoint, 1, (1, 1, &[0; 4]), 0, &look(ContentFit::Cover)).unwrap_err();
        assert!(err.contains("unknown action"), "{err}");
        server.join().expect("server");
        assert_eq!(
            std::fs::read_dir(tmp.path().join("mshell"))
                .expect("dir")
                .count(),
            0
        );

        let no_margo = Endpoint {
            socket: tmp.path().join("none.sock"),
            dir: Some(tmp.path().join("mshell")),
        };
        assert!(hand_over(&no_margo, 2, (1, 1, &[0; 4]), 0, &look(ContentFit::Cover)).is_err());
        let no_dir = Endpoint {
            socket: tmp.path().join("margo-ipc.sock"),
            dir: None,
        };
        assert!(hand_over(&no_dir, 3, (1, 1, &[0; 4]), 0, &look(ContentFit::Cover)).is_err());
    }

    #[test]
    fn a_never_shown_window_resolves_the_stylesheet_background() {
        // The probe window is never mapped; its style must still come from the
        // display's providers. Needs a display: skipped without one.
        if gtk::init().is_err() {
            eprintln!("skipped: no display");
            return;
        }
        let provider = gtk::CssProvider::new();
        provider.load_from_string(
            ":root { --surface: #282A36; } .wallpaper-window { background-color: var(--surface); }",
        );
        let Some(display) = gtk::gdk::Display::default() else {
            eprintln!("skipped: no display");
            return;
        };
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_USER,
        );
        let probe = gtk::Window::new();
        probe.add_css_class("wallpaper-window");
        assert_eq!(background_of(&probe).as_deref(), Some("#282a36ff"));
        provider.load_from_string(
            ":root { --surface: #ffffff; } .wallpaper-window { background-color: var(--surface); }",
        );
        assert_eq!(
            background_of(&probe).as_deref(),
            Some("#ffffffff"),
            "a reloaded stylesheet is seen"
        );
    }
}
