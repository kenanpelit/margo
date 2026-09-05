// SPDX-FileCopyrightText: 2022  Emmanuele Bassi
// SPDX-License-Identifier: GPL-3.0-or-later

use std::{cell::RefCell, rc::Rc};

use adw::prelude::AdwDialogExt;
use adw::subclass::prelude::*;
use async_channel::Receiver;
use glib::clone;
use gtk::{gio, glib, prelude::*};
use log::debug;

use crate::{
    audio::AudioPlayer,
    bridge::{AppCommand, CommandReceiver, CommandSender, SharedSnapshot, new_shared},
    config::{APPLICATION_ID, VERSION},
    dbus,
    i18n::i18n,
    library::LibraryEvent,
    library::config::{MtuneConfig, OnStart},
    library::index::{IndexEntry, LibraryIndex, mtime_of},
    library::scanner,
    library::watcher::LibraryWatcher,
    tray::{self, TuneTray},
    utils,
    window::{StartIntent, Window},
};

pub enum ApplicationAction {
    Present,
    /// MPRIS `Quit` — shut the app down.
    Quit,
    /// Hold the GApplication alive with no window (background playback, and —
    /// a later phase — while the tray item is registered), or release it.
    /// Sent by the player on every playback-state transition.
    BackgroundHold(bool),
}

mod imp {
    use super::*;

    pub struct Application {
        pub player: Rc<AudioPlayer>,
        pub receiver: RefCell<Option<Receiver<ApplicationAction>>>,
        pub background_hold: RefCell<Option<gio::ApplicationHoldGuard>>,
        pub settings: gio::Settings,
        pub config: RefCell<MtuneConfig>,
        pub watcher: RefCell<Option<LibraryWatcher>>,
        /// Guard so `load_library` only ever runs once per process.
        pub library_loaded: std::cell::Cell<bool>,
        /// `Send` mirror of playback/library state for the tray + D-Bus.
        pub snap: SharedSnapshot,
        pub cmd_tx: CommandSender,
        pub cmd_rx: RefCell<Option<CommandReceiver>>,
        pub dbus_conn: RefCell<Option<zbus::Connection>>,
        pub tray: RefCell<Option<ksni::Handle<TuneTray>>>,
        /// Overlays the margo matugen palette (`~/.cache/mshell/last_theme.css`)
        /// on top of the baked stylesheet; reloaded when matugen rewrites it.
        /// Built lazily in `setup_matugen_palette` — `CssProvider::new()`
        /// aborts if called before `gtk::init` (i.e. at object construction).
        pub matugen_provider: RefCell<Option<gtk::CssProvider>>,
        pub matugen_monitor: RefCell<Option<gio::FileMonitor>>,
        /// Last `org.freedesktop.Notifications` ids so a new toast
        /// replaces the previous one in its slot instead of stacking:
        /// `np` = now-playing, `setting` = the transient setting blips.
        pub np_notify_id: std::cell::Cell<u32>,
        pub setting_notify_id: std::cell::Cell<u32>,
        /// `mtune --hidden` was passed on this launch.
        pub hidden_flag: std::cell::Cell<bool>,
        /// `activate()` has run once — later re-launches always show the
        /// window even when the first start was hidden.
        pub activated: std::cell::Cell<bool>,
        /// Keeps the GApplication alive while it runs windowless after a
        /// hidden start (it would otherwise quit with zero windows).
        pub startup_hold: RefCell<Option<gio::ApplicationHoldGuard>>,
    }

    impl std::fmt::Debug for Application {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("Application").finish_non_exhaustive()
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Application {
        const NAME: &'static str = "TuneApplication";
        type Type = super::Application;
        type ParentType = adw::Application;

        fn new() -> Self {
            let (sender, r) = async_channel::unbounded();
            let receiver = RefCell::new(Some(r));
            let (cmd_tx, cmd_rx) = async_channel::unbounded();

            Self {
                player: AudioPlayer::new(sender),
                receiver,
                background_hold: RefCell::default(),
                settings: utils::settings_manager(),
                config: RefCell::new(MtuneConfig::load()),
                watcher: RefCell::default(),
                library_loaded: std::cell::Cell::new(false),
                snap: new_shared(),
                cmd_tx,
                cmd_rx: RefCell::new(Some(cmd_rx)),
                dbus_conn: RefCell::default(),
                tray: RefCell::default(),
                matugen_provider: RefCell::default(),
                matugen_monitor: RefCell::default(),
                np_notify_id: std::cell::Cell::new(0),
                setting_notify_id: std::cell::Cell::new(0),
                hidden_flag: std::cell::Cell::new(false),
                activated: std::cell::Cell::new(false),
                startup_hold: RefCell::default(),
            }
        }
    }

    impl ObjectImpl for Application {
        fn constructed(&self) {
            self.parent_constructed();

            let obj = self.obj();

            // Register `--hidden` so GApplication documents it in `--help`
            // and doesn't abort on it; the value itself is set from argv
            // in `main` before `run()` (a second `mtune --hidden` while
            // one is already running just raises the window).
            obj.add_main_option(
                "hidden",
                glib::Char(0),
                glib::OptionFlags::NONE,
                glib::OptionArg::None,
                "Start with no window — just the tray icon",
                None,
            );

            obj.setup_channel();
            obj.setup_gactions();
            obj.setup_settings();
            obj.setup_bridge();
            obj.setup_notifications();

            obj.set_accels_for_action("app.quit", &["<primary>q"]);

            obj.set_accels_for_action("queue.add-song", &["<primary>s"]);
            obj.set_accels_for_action("queue.add-folder", &["<primary>a"]);
            obj.set_accels_for_action("queue.clear", &["<primary>L"]);
            obj.set_accels_for_action("queue.toggle", &["F9"]);
            obj.set_accels_for_action("queue.search", &["<primary>F"]);
            obj.set_accels_for_action("queue.shuffle", &["<primary>r"]);

            obj.set_accels_for_action("win.seek-backwards", &["<primary>Left"]);
            obj.set_accels_for_action("win.seek-forward", &["<primary>Right"]);
            obj.set_accels_for_action("win.previous", &["<primary>b"]);
            obj.set_accels_for_action("win.next", &["<primary>n"]);
            obj.set_accels_for_action("win.play", &["<primary>p"]);
            obj.set_accels_for_action("win.copy", &["<primary>c"]);
            obj.set_accels_for_action("win.cycle-view", &["<primary>m"]);
            obj.set_accels_for_action("queue.open-playlist", &["<primary>o"]);
            obj.set_accels_for_action("queue.save-playlist", &["<primary><shift>s"]);
            obj.set_accels_for_action("app.shortcuts", &["<primary>question", "<primary>slash"]);
        }
    }

    impl ApplicationImpl for Application {
        fn startup(&self) {
            self.parent_startup();

            gtk::Window::set_default_icon_name(APPLICATION_ID);
            self.obj().setup_matugen_palette();

            // Checkpoint the resume position while running, so a crash /
            // SIGKILL still leaves a recent spot to come back to.
            glib::timeout_add_seconds_local(
                10,
                clone!(
                    #[weak(rename_to = this)]
                    self.obj(),
                    #[upgrade_or]
                    glib::ControlFlow::Break,
                    move || {
                        this.persist_resume();
                        glib::ControlFlow::Continue
                    }
                ),
            );
        }

        fn shutdown(&self) {
            self.obj().persist_resume();
            self.parent_shutdown();
        }

        fn activate(&self) {
            debug!("Application::activate");
            let obj = self.obj();

            // Start-hidden: only on the very first activation, only when
            // nothing forced a window open. `mtune --hidden` or the
            // `[behaviour] start_hidden` config both take this path.
            let first = !self.activated.replace(true);
            let start_hidden = first
                && obj.active_window().is_none()
                && (self.hidden_flag.get() || self.config.borrow().behaviour.start_hidden);

            if start_hidden {
                debug!("Application::activate — hidden start, tray only");
                // No window → hold the app so GApplication doesn't quit.
                // The `tray::spawn` failure path (setup_bridge) shows the
                // window if there's nowhere for the icon to live.
                self.startup_hold.replace(Some(obj.hold()));
            } else {
                obj.present_main_window();
            }
            obj.load_library();
        }

        fn open(&self, files: &[gio::File], _hint: &str) {
            debug!("Application::open");

            let application = self.obj();
            application.present_main_window();
            if let Some(window) = application.active_window() {
                window.downcast_ref::<Window>().unwrap().open_files(files);
            }
            // Files passed on the command line take precedence over the
            // configured library for this run.
            self.library_loaded.set(true);
        }
    }

    impl GtkApplicationImpl for Application {}
    impl AdwApplicationImpl for Application {}
}

glib::wrapper! {
    pub struct Application(ObjectSubclass<imp::Application>)
        @extends gio::Application, gtk::Application, adw::Application,
        @implements gio::ActionGroup, gio::ActionMap;
}

impl Default for Application {
    fn default() -> Self {
        glib::Object::builder::<Application>()
            .property("application-id", APPLICATION_ID)
            .property("flags", gio::ApplicationFlags::HANDLES_OPEN)
            .property("resource-base-path", "/org/margo/Tune")
            .build()
    }
}

impl Application {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set from `main` when `--hidden` is on this launch's argv — start
    /// with no window, just the tray icon.
    pub fn set_start_hidden(&self, hidden: bool) {
        self.imp().hidden_flag.set(hidden);
    }

    pub fn player(&self) -> Rc<AudioPlayer> {
        self.imp().player.clone()
    }

    /// Load the configured folder library into the queue, then start the
    /// inotify watcher. Runs once per process; a no-op when no roots are
    /// configured (the plain "open a folder" flow still works) or when the
    /// app was launched with files on the command line.
    pub fn load_library(&self) {
        if self.imp().library_loaded.replace(true) {
            return;
        }
        let cfg = self.imp().config.borrow().clone();
        self.imp()
            .player
            .queue()
            .set_repeat_count(cfg.playback.repeat_count);
        let roots = cfg.library.resolved_roots();
        if roots.is_empty() {
            debug!("mtune: no library roots configured");
            return;
        }

        {
            let mut s = self
                .imp()
                .snap
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            s.scanning = true;
            s.scan_done = 0;
            s.scan_total = 0;
        }
        self.refresh_bridge();

        let this = self.clone();
        glib::spawn_future_local(async move {
            let lib = cfg.library.clone();

            // Prefer the cached index when the user opted out of a full
            // rescan and it still reflects the filesystem; otherwise scan.
            let paths: Vec<std::path::PathBuf> = if lib.scan_on_start {
                let (r, l) = (roots.clone(), lib.clone());
                gio::spawn_blocking(move || scanner::scan_blocking(&r, &l))
                    .await
                    .unwrap_or_default()
            } else {
                let fresh = LibraryIndex::load().fresh_paths();
                if fresh.is_empty() {
                    let (r, l) = (roots.clone(), lib.clone());
                    gio::spawn_blocking(move || scanner::scan_blocking(&r, &l))
                        .await
                        .unwrap_or_default()
                } else {
                    fresh
                }
            };

            debug!("mtune: library has {} tracks", paths.len());
            {
                let mut s = this
                    .imp()
                    .snap
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                s.scanning = false;
                s.scan_done = paths.len() as u32;
                s.scan_total = paths.len() as u32;
            }
            this.refresh_bridge();

            // Refresh the on-disk index (path + mtime is enough to drive
            // the fast path next launch; tags are re-read from the songs).
            let index = LibraryIndex {
                entries: paths
                    .iter()
                    .filter_map(|p| {
                        Some(IndexEntry {
                            path: p.clone(),
                            mtime: mtime_of(p)?,
                            title: String::new(),
                            artist: String::new(),
                            album: String::new(),
                            duration_secs: 0,
                        })
                    })
                    .collect(),
            };
            if let Err(e) = index.save() {
                debug!("mtune: could not save the library index: {e}");
            }

            let intent = match cfg.playback.on_start {
                OnStart::Nothing => StartIntent::Nothing,
                OnStart::Library => StartIntent::Top,
                OnStart::Resume => {
                    let uri = this.imp().settings.string("resume-uri");
                    let pos = this.imp().settings.uint64("resume-position");
                    if uri.is_empty() {
                        StartIntent::Top
                    } else {
                        StartIntent::Resume(uri.to_string(), pos)
                    }
                }
            };

            if let Some(win) = this.active_window().and_downcast::<Window>() {
                let files: Vec<gio::File> = paths.iter().map(gio::File::for_path).collect();
                win.load_library_files(files, intent);
            }

            if lib.watch {
                this.start_watch(lib);
            }
        });
    }

    /// Watch the library roots and reflect adds / removes into the queue live.
    fn start_watch(&self, lib: crate::library::config::LibrarySection) {
        let (tx, rx) = async_channel::unbounded();
        match LibraryWatcher::start(lib, tx) {
            Ok(w) => {
                self.imp().watcher.replace(Some(w));
            }
            Err(e) => {
                debug!("mtune: library watch unavailable: {e}");
                return;
            }
        }
        let this = self.clone();
        glib::spawn_future_local(async move {
            while let Ok(ev) = rx.recv().await {
                let Some(win) = this.active_window().and_downcast::<Window>() else {
                    continue;
                };
                let player = this.imp().player.clone();
                match ev {
                    LibraryEvent::Added(p) => {
                        let uri = gio::File::for_path(&p).uri().to_string();
                        if player.queue().position_of_uri(&uri).is_none() {
                            win.open_files(&[gio::File::for_path(&p)]);
                        }
                    }
                    LibraryEvent::Removed(p) => {
                        let uri = gio::File::for_path(&p).uri().to_string();
                        if let Some(ix) = player.queue().position_of_uri(&uri)
                            && let Some(song) = player.queue().song_at(ix)
                        {
                            player.remove_song(&song);
                        }
                    }
                }
            }
        });
    }

    /// The margo matugen palette file (`:root { --surface: …; --primary: …; }`),
    /// shared with the shell + `mlock` / `mgreet`.
    fn matugen_palette_path() -> std::path::PathBuf {
        let base = std::env::var_os("XDG_CACHE_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".cache"))
            })
            .unwrap_or_default();
        base.join("mshell").join("last_theme.css")
    }

    /// Load the matugen palette as a CSS provider above the baked stylesheet,
    /// and reload it whenever matugen rewrites the file (wallpaper change).
    fn setup_matugen_palette(&self) {
        let Some(display) = gtk::gdk::Display::default() else {
            return;
        };
        let provider = gtk::CssProvider::new();
        self.imp().matugen_provider.replace(Some(provider.clone()));
        // 700 > GTK_STYLE_PROVIDER_PRIORITY_APPLICATION (600, the auto-loaded
        // style.css), < USER (800) — the palette's `:root` tokens win over the
        // stylesheet's fallbacks without overriding a user's own gtk.css.
        gtk::style_context_add_provider_for_display(&display, &provider, 700);

        let path = Self::matugen_palette_path();
        Self::load_matugen(&provider, &path);

        let file = gio::File::for_path(&path);
        if let Ok(monitor) = file.monitor_file(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE)
        {
            monitor.connect_changed(clone!(
                #[weak]
                provider,
                move |_, _, _, _| {
                    Self::load_matugen(&provider, &Self::matugen_palette_path());
                }
            ));
            self.imp().matugen_monitor.replace(Some(monitor));
        }
    }

    fn load_matugen(provider: &gtk::CssProvider, path: &std::path::Path) {
        match std::fs::read_to_string(path) {
            Ok(css) => provider.load_from_string(&css),
            Err(_) => provider.load_from_string(""), // no palette yet — fall back
        }
    }

    // ── tray + `org.margo.Tune` bridge ──────────────────────────────

    /// Spawn the command-receiver loop, the `org.margo.Tune` D-Bus service,
    /// and the tray; wire player/queue change signals to refresh both.
    fn setup_bridge(&self) {
        let imp = self.imp();

        // 1. Apply tray / D-Bus commands on the main context.
        if let Some(rx) = imp.cmd_rx.borrow_mut().take() {
            let this = self.clone();
            glib::spawn_future_local(async move {
                while let Ok(cmd) = rx.recv().await {
                    this.apply_command(cmd);
                }
            });
        }

        // 2. Serve the supplementary org.margo.Tune interface. It rides
        //    on the MPRIS server's connection (the GApplication owns the
        //    bare `org.margo.Tune` name, so a fresh zbus connection can't
        //    claim it) — wait for that connection to come up first.
        {
            let this = self.clone();
            let snap = imp.snap.clone();
            let tx = imp.cmd_tx.clone();
            let player = imp.player.clone();
            glib::spawn_future_local(async move {
                let mut conn = None;
                for _ in 0..60 {
                    if let Some(c) = player.mpris_connection() {
                        conn = Some(c);
                        break;
                    }
                    glib::timeout_future(std::time::Duration::from_millis(50)).await;
                }
                let Some(conn) = conn else {
                    log::warn!(
                        "mtune: MPRIS connection never came up — org.margo.Tune interface unavailable"
                    );
                    return;
                };
                if let Some(conn) = dbus::serve_on(&conn, snap, tx).await {
                    this.imp().dbus_conn.replace(Some(conn));
                    this.refresh_bridge();
                }
            });
        }

        // 3. Register the tray item.
        {
            let this = self.clone();
            let snap = imp
                .snap
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone();
            let tx = imp.cmd_tx.clone();
            glib::spawn_future_local(async move {
                match tray::spawn(snap, tx).await {
                    Some(handle) => {
                        this.imp().tray.replace(Some(handle));
                        this.refresh_bridge();
                    }
                    None => {
                        // No StatusNotifierWatcher — a hidden start would
                        // leave an invisible process. Fall back to the
                        // window and drop the windowless hold.
                        if this.imp().startup_hold.borrow().is_some() {
                            log::warn!("mtune: no system tray available — showing the window");
                            this.present_main_window();
                            this.imp().startup_hold.replace(None);
                        }
                    }
                }
            });
        }

        // 4. Refresh on every playback / queue change.
        let state = imp.player.state();
        let queue = imp.player.queue();
        for (obj, sig) in [
            (state.upcast_ref::<glib::Object>(), "playing"),
            (state.upcast_ref::<glib::Object>(), "song"),
            (state.upcast_ref::<glib::Object>(), "volume"),
            (queue.upcast_ref::<glib::Object>(), "current"),
            (queue.upcast_ref::<glib::Object>(), "n-songs"),
            (queue.upcast_ref::<glib::Object>(), "repeat-mode"),
        ] {
            obj.connect_notify_local(
                Some(sig),
                clone!(
                    #[weak(rename_to = this)]
                    self,
                    move |_, _| this.refresh_bridge()
                ),
            );
        }

        self.refresh_bridge();
    }

    /// Snapshot the player + library into the `Send` mirror, then poke the
    /// tray and emit the D-Bus `Changed` signal.
    fn refresh_bridge(&self) {
        let imp = self.imp();
        let state = imp.player.state();
        let queue = imp.player.queue();
        let queue_entries: Vec<(String, String, u64)> = (0..queue.n_songs())
            .filter_map(|i| queue.song_at(i))
            .map(|s| (s.title(), s.artist(), s.duration()))
            .collect();

        // Read the scan fields in one lock — a plain `std::sync::Mutex`
        // is not reentrant, and temporaries in a `let x = { StructLit {
        // .lock()…, .lock()… } };` all live to the end of the `let`, so
        // three inline `.lock()` calls self-deadlock the first launch.
        let (scanning, scan_done, scan_total) = {
            let s = imp
                .snap
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            (s.scanning, s.scan_done, s.scan_total)
        };

        let next = {
            let song = state.current_song();
            crate::bridge::Snapshot {
                has_song: song.is_some(),
                playing: state.playing(),
                title: state.title().unwrap_or_default(),
                artist: state.artist().unwrap_or_default(),
                album: state.album().unwrap_or_default(),
                lyrics: state.lyrics().unwrap_or_default(),
                cover_art: song
                    .as_ref()
                    .and_then(|s| s.cover_cache())
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                position_secs: state.position(),
                duration_secs: state.duration(),
                volume: state.volume(),
                rate: state.playback_rate(),
                shuffle: queue.is_shuffled(),
                repeat: queue.repeat_mode(),
                queue_len: queue.n_songs(),
                current_index: queue.current_song_index().map(|i| i as i64).unwrap_or(-1),
                queue_entries,
                library_roots: imp
                    .config
                    .borrow()
                    .library
                    .roots
                    .iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect(),
                playlists: crate::playlist::saved_names(),
                scanning,
                scan_done,
                scan_total,
            }
        };

        *imp.snap
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = next.clone();

        if let Some(handle) = imp.tray.borrow().clone() {
            glib::spawn_future_local(async move {
                handle.update(move |t| t.set_snapshot(next)).await;
            });
        }
        if let Some(conn) = imp.dbus_conn.borrow().clone() {
            glib::spawn_future_local(async move {
                dbus::emit_changed(&conn).await;
            });
        }
    }

    fn apply_command(&self, cmd: AppCommand) {
        let imp = self.imp();
        let player = imp.player.clone();
        match cmd {
            AppCommand::PlayPause => player.toggle_play(),
            AppCommand::Next => player.skip_next(),
            AppCommand::Previous => player.skip_previous(),
            AppCommand::Stop => player.stop(),
            AppCommand::SetShuffle(b) => player.queue().set_shuffled(b),
            AppCommand::SetRepeat(m) => player.update_repeat_mode(m),
            AppCommand::SeekAbs(s) => player.seek_position_abs(s),
            AppCommand::SetVolume(v) => player.set_volume(v),
            AppCommand::PlayIndex(i) => player.skip_to(i),
            AppCommand::RemoveIndex(i) => {
                if let Some(song) = player.queue().song_at(i) {
                    player.remove_song(&song);
                }
            }
            AppCommand::ToggleWindow => {
                if let Some(win) = self.active_window() {
                    if win.is_visible() {
                        win.set_visible(false);
                    } else {
                        win.present();
                    }
                } else {
                    self.activate();
                }
            }
            AppCommand::Raise => {
                if let Some(win) = self.active_window() {
                    win.present();
                } else {
                    self.activate();
                }
            }
            AppCommand::Quit => self.quit(),
            AppCommand::PlayFolder(path) => {
                player.clear_queue();
                if let Some(win) = self.active_window().and_downcast::<Window>() {
                    win.load_library_files(vec![gio::File::for_path(&path)], StartIntent::Top);
                }
            }
            AppCommand::SetLibraryRoots(roots) => {
                {
                    let mut cfg = imp.config.borrow_mut();
                    cfg.library.roots = roots;
                    if let Err(e) = cfg.save() {
                        debug!("mtune: could not save mtune.toml: {e}");
                    }
                }
                imp.library_loaded.set(false);
                imp.watcher.replace(None);
                player.clear_queue();
                self.load_library();
            }
            AppCommand::RescanLibrary => {
                imp.library_loaded.set(false);
                imp.watcher.replace(None);
                player.clear_queue();
                self.load_library();
            }
            AppCommand::SetRate(rate) => player.set_playback_rate(rate),
            AppCommand::LoadPlaylist(name) => {
                if let Some(win) = self.active_window().and_downcast::<Window>() {
                    win.open_playlist_file(&crate::playlist::saved_path(&name));
                }
            }
            AppCommand::OpenPlaylist(path) => {
                if let Some(win) = self.active_window().and_downcast::<Window>() {
                    win.open_playlist_file(&path);
                }
            }
            AppCommand::SavePlaylist(name) => {
                if let Err(e) = crate::playlist::save(&name, player.queue()) {
                    debug!("mtune: save playlist: {e}");
                }
            }
        }
        self.refresh_bridge();
    }

    /// Hold the GApplication alive with no visible window while playback is
    /// active (and `background-play` is enabled), release it otherwise. This
    /// replaces the upstream `ashpd` Background-portal request — margo does
    /// not service that portal; the `gio` hold guard is the real mechanism.
    pub fn set_background_hold(&self, active: bool) {
        let imp = self.imp();
        let want = active && imp.settings.boolean("background-play");
        let held = imp.background_hold.borrow().is_some();
        if want && !held {
            imp.background_hold.replace(Some(self.hold()));
        } else if !want && held {
            imp.background_hold.replace(None);
        }
    }

    fn setup_settings(&self) {
        self.imp().settings.connect_changed(
            Some("background-play"),
            clone!(
                #[weak(rename_to = this)]
                self,
                move |settings, _| {
                    let background_play = settings.boolean("background-play");
                    debug!("GSettings:background-play: {background_play}");
                    if !background_play {
                        debug!("Dropping background hold");
                        this.imp().background_hold.replace(None);
                    }
                }
            ),
        );

        let _dummy = self.imp().settings.boolean("background-play");

        // Playback rate is sticky: restore the last value, and persist it
        // whenever it changes (the pill / MPRIS / window all route through
        // `PlayerState`).
        let imp = self.imp();
        let rate = imp.settings.double("playback-rate");
        if (rate - 1.0).abs() > f64::EPSILON {
            imp.player.set_playback_rate(rate);
        }
        imp.player.state().connect_notify_local(
            Some("playback-rate"),
            clone!(
                #[weak(rename_to = this)]
                self,
                move |state, _| {
                    let _ = this
                        .imp()
                        .settings
                        .set_double("playback-rate", state.playback_rate());
                }
            ),
        );
    }

    /// Wire desktop-notification toasts: the now-playing track (only
    /// while Tune isn't the focused window) and a transient blip on each
    /// playback-setting toggle. All gated on the `notifications`
    /// GSetting; every toast rides the MPRIS session-bus connection.
    fn setup_notifications(&self) {
        let imp = self.imp();
        let state = imp.player.state();
        let queue = imp.player.queue();

        state.connect_notify_local(
            Some("song"),
            clone!(
                #[weak(rename_to = this)]
                self,
                move |_, _| this.toast_track_change()
            ),
        );
        state.connect_notify_local(
            Some("playback-rate"),
            clone!(
                #[weak(rename_to = this)]
                self,
                move |s, _| {
                    let rate = (s.playback_rate() * 100.0).round() / 100.0;
                    this.toast_setting(&format!("Speed: {rate}×"));
                }
            ),
        );
        queue.connect_notify_local(
            Some("repeat-mode"),
            clone!(
                #[weak(rename_to = this)]
                self,
                move |q, _| {
                    let label = match q.repeat_mode() {
                        crate::audio::RepeatMode::RepeatAll => "all",
                        crate::audio::RepeatMode::RepeatOne => "one",
                        crate::audio::RepeatMode::RepeatEach => "each",
                        crate::audio::RepeatMode::Consecutive => "off",
                    };
                    this.toast_setting(&format!("Repeat: {label}"));
                }
            ),
        );
        queue.connect_notify_local(
            Some("shuffled"),
            clone!(
                #[weak(rename_to = this)]
                self,
                move |q, _| {
                    let on = if q.is_shuffled() { "on" } else { "off" };
                    this.toast_setting(&format!("Shuffle: {on}"));
                }
            ),
        );
        imp.settings.connect_changed(
            Some("replay-gain"),
            clone!(
                #[weak(rename_to = this)]
                self,
                move |settings, _| {
                    let label = match settings.enum_("replay-gain") {
                        0 => "album",
                        1 => "track",
                        _ => "off",
                    };
                    this.toast_setting(&format!("ReplayGain: {label}"));
                }
            ),
        );
    }

    /// Toast the current track — unless Tune's own window is focused
    /// (then it's just noise) or notifications are off.
    fn toast_track_change(&self) {
        let imp = self.imp();
        if !imp.settings.boolean("notifications") {
            return;
        }
        if self.active_window().map(|w| w.is_active()).unwrap_or(false) {
            return;
        }
        let Some(conn) = imp.dbus_conn.borrow().clone() else {
            return;
        };
        let state = imp.player.state();
        let title = state.title().unwrap_or_default();
        if title.is_empty() {
            return;
        }
        let artist = state.artist().unwrap_or_default();
        let album = state.album().unwrap_or_default();
        let body = match (artist.is_empty(), album.is_empty()) {
            (false, false) => format!("{artist} · {album}"),
            (false, true) => artist,
            (true, false) => album,
            (true, true) => String::new(),
        };
        let prev = imp.np_notify_id.get();
        let this = self.clone();
        glib::spawn_future_local(async move {
            let id = crate::notify::notify(&conn, prev, &title, &body, false, -1).await;
            this.imp().np_notify_id.set(id);
        });
    }

    /// A short transient toast confirming a playback-setting change.
    fn toast_setting(&self, summary: &str) {
        let imp = self.imp();
        if !imp.settings.boolean("notifications") {
            return;
        }
        let Some(conn) = imp.dbus_conn.borrow().clone() else {
            return;
        };
        let prev = imp.setting_notify_id.get();
        let summary = summary.to_owned();
        let this = self.clone();
        glib::spawn_future_local(async move {
            let id = crate::notify::notify(&conn, prev, &summary, "", true, 1500).await;
            this.imp().setting_notify_id.set(id);
        });
    }

    /// Write the current track + position to GSettings so
    /// `[playback] on_start = "resume"` can pick it up next launch.
    /// Called on a timer, on MPRIS/tray `Quit`, on `shutdown`, and on
    /// window close.
    pub fn persist_resume(&self) {
        let imp = self.imp();
        let Some(song) = imp.player.state().current_song() else {
            return;
        };
        let _ = imp.settings.set_string("resume-uri", &song.uri());
        let _ = imp
            .settings
            .set_uint64("resume-position", imp.player.state().position());
    }

    fn setup_channel(&self) {
        let receiver = self.imp().receiver.borrow_mut().take().unwrap();
        glib::MainContext::default().spawn_local(clone!(
            #[strong(rename_to = this)]
            self,
            async move {
                use futures::prelude::*;

                let mut receiver = std::pin::pin!(receiver);

                while let Some(action) = receiver.next().await {
                    this.process_action(action);
                }
            }
        ));
    }

    fn process_action(&self, action: ApplicationAction) -> glib::ControlFlow {
        match action {
            ApplicationAction::Present => self.present_main_window(),
            ApplicationAction::Quit => {
                self.persist_resume();
                self.quit();
            }
            ApplicationAction::BackgroundHold(active) => self.set_background_hold(active),
        }

        glib::ControlFlow::Continue
    }

    fn present_main_window(&self) {
        let window = if let Some(window) = self.active_window() {
            window
        } else {
            let window = Window::new(self);
            window.upcast()
        };

        window.present();
    }

    fn setup_gactions(&self) {
        self.add_action_entries([
            gio::ActionEntry::builder("quit")
                .activate(|app: &Application, _, _| {
                    app.quit();
                })
                .build(),
            gio::ActionEntry::builder("about")
                .activate(|app: &Application, _, _| {
                    app.show_about();
                })
                .build(),
            gio::ActionEntry::builder("shortcuts")
                .activate(|app: &Application, _, _| {
                    app.show_shortcuts();
                })
                .build(),
        ]);

        let background_play = self.imp().settings.boolean("background-play");
        self.add_action_entries([gio::ActionEntry::builder("background-play")
            .state(background_play.to_variant())
            .activate(|this: &Application, action, _| {
                let state = action.state().unwrap();
                let action_state: bool = state.get().unwrap();
                let background_play = !action_state;
                action.set_state(&background_play.to_variant());

                this.imp()
                    .settings
                    .set_boolean("background-play", background_play)
                    .expect("Unable to store background-play setting");
            })
            .build()]);

        let notifications = self.imp().settings.boolean("notifications");
        self.add_action_entries([gio::ActionEntry::builder("notifications")
            .state(notifications.to_variant())
            .activate(|this: &Application, action, _| {
                let on = action.state().and_then(|s| s.get::<bool>()).unwrap_or(true);
                action.set_state(&(!on).to_variant());
                let _ = this.imp().settings.set_boolean("notifications", !on);
            })
            .build()]);
    }

    fn show_about(&self) {
        let window = self.active_window().unwrap();
        let dialog = adw::AboutDialog::builder()
            .application_icon(APPLICATION_ID)
            .application_name("Tune")
            .developer_name("the margo project")
            .version(VERSION)
            .developers(vec!["Kenan Pelit", "Emmanuele Bassi (original work)"])
            .copyright("© 2026 Kenan Pelit · original work © 2022–2025 Emmanuele Bassi")
            .website("https://github.com/kenanpelit/margo")
            .issue_url("https://github.com/kenanpelit/margo/issues/new")
            .license_type(gtk::License::Gpl30)
            // Translators: Replace "translator-credits" with your names, one name per line
            .translator_credits(i18n("translator-credits"))
            .build();

        dialog.present(Some(&window));
    }

    fn show_shortcuts(&self) {
        let builder = gtk::Builder::from_resource("/org/margo/Tune/shortcuts-dialog.ui");
        if let Some(dialog) = builder.object::<adw::Dialog>("shortcuts_dialog") {
            dialog.present(self.active_window().as_ref());
        }
    }
}
