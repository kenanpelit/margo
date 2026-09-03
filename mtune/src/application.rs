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
    config::{APPLICATION_ID, VERSION},
    i18n::i18n,
    library::LibraryEvent,
    library::config::{MtuneConfig, OnStart},
    library::index::{IndexEntry, LibraryIndex, mtime_of},
    library::scanner,
    library::watcher::LibraryWatcher,
    utils,
    window::{StartIntent, Window},
};

pub enum ApplicationAction {
    Present,
    /// Hold the GApplication alive with no window (background playback, and —
    /// a later phase — while the tray item is registered), or release it.
    /// Sent by the player on every playback-state transition.
    BackgroundHold(bool),
}

mod imp {
    use super::*;

    #[derive(Debug)]
    pub struct Application {
        pub player: Rc<AudioPlayer>,
        pub receiver: RefCell<Option<Receiver<ApplicationAction>>>,
        pub background_hold: RefCell<Option<gio::ApplicationHoldGuard>>,
        pub settings: gio::Settings,
        pub config: RefCell<MtuneConfig>,
        pub watcher: RefCell<Option<LibraryWatcher>>,
        /// Guard so `load_library` only ever runs once per process.
        pub library_loaded: std::cell::Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Application {
        const NAME: &'static str = "TuneApplication";
        type Type = super::Application;
        type ParentType = adw::Application;

        fn new() -> Self {
            let (sender, r) = async_channel::unbounded();
            let receiver = RefCell::new(Some(r));

            Self {
                player: AudioPlayer::new(sender),
                receiver,
                background_hold: RefCell::default(),
                settings: utils::settings_manager(),
                config: RefCell::new(MtuneConfig::load()),
                watcher: RefCell::default(),
                library_loaded: std::cell::Cell::new(false),
            }
        }
    }

    impl ObjectImpl for Application {
        fn constructed(&self) {
            self.parent_constructed();

            let obj = self.obj();
            obj.setup_channel();
            obj.setup_gactions();
            obj.setup_settings();

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
        }
    }

    impl ApplicationImpl for Application {
        fn startup(&self) {
            self.parent_startup();

            gtk::Window::set_default_icon_name(APPLICATION_ID);
        }

        fn activate(&self) {
            debug!("Application::activate");

            self.obj().present_main_window();
            self.obj().load_library();
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
        let roots = cfg.library.resolved_roots();
        if roots.is_empty() {
            debug!("mtune: no library roots configured");
            return;
        }

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
}
