// SPDX-FileCopyrightText: 2022  Emmanuele Bassi
// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    str::FromStr,
    time::Instant,
};

use adw::prelude::*;
use adw::subclass::prelude::*;
use glib::{clone, closure_local};
use gtk::{CompositeTemplate, gdk, gio, glib, pango};
use log::debug;

use crate::{
    audio::{AudioPlayer, RepeatMode, ReplayGainMode, Song},
    config::APPLICATION_ID,
    drag_overlay::DragOverlay,
    i18n::{i18n, i18n_f, i18n_k, ni18n_f, ni18n_k},
    playback_control::PlaybackControl,
    playlist_view::PlaylistView,
    queue_row::QueueRow,
    search::FuzzyFilter,
    song_cover::SongCover,
    song_details::SongDetails,
    sort::FuzzySorter,
    utils,
    volume_control::VolumeControl,
    waveform_view::WaveformView,
};

// `impl Window` is split by area: view_mode, playlist, queue, signals and
// now_playing in window/*.rs; the constructor, actions and public API stay here.
mod now_playing;
mod playlist;
mod queue;
mod signals;
mod view_mode;

pub enum WindowMode {
    InitialView,
    MainView,
}

/// The window skin. Cycled full → mini → strip → full (Ctrl+M), stored
/// in GSettings `view-mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewMode {
    #[default]
    Full,
    Mini,
    Strip,
}

impl ViewMode {
    fn next(self) -> Self {
        match self {
            Self::Full => Self::Mini,
            Self::Mini => Self::Strip,
            Self::Strip => Self::Full,
        }
    }
    fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Mini => "mini",
            Self::Strip => "strip",
        }
    }
    fn from_str(s: &str) -> Self {
        match s {
            "mini" => Self::Mini,
            "strip" => Self::Strip,
            _ => Self::Full,
        }
    }
}

/// Where playback should land after the folder library finishes loading
/// into a previously-empty queue. Maps from `[playback] on_start`.
#[derive(Debug, Clone)]
pub enum StartIntent {
    /// Select the top of the queue, paused.
    Top,
    /// Skip to the track with this URI and seek to this position (secs),
    /// paused; falls back to `Top` if the URI is no longer in the library.
    Resume(String, u64),
    /// Skip to this queue index (clamped to the queue's bounds), paused.
    /// Used to resume a saved playlist at its remembered position.
    AtIndex(u32),
    /// Leave the queue untouched — no current song.
    Nothing,
}

const ATTRIBUTE_HOST_PATH: &str = "xattr::document-portal.host-path";

mod imp {
    use glib::{ParamSpec, ParamSpecBoolean, ParamSpecEnum, Value};
    use once_cell::sync::Lazy;

    use super::*;

    #[derive(CompositeTemplate)]
    #[template(resource = "/org/margo/Tune/window.ui")]
    pub struct Window {
        // Template widgets
        #[template_child]
        pub drag_overlay: TemplateChild<DragOverlay>,
        #[template_child]
        pub toast_overlay: TemplateChild<adw::ToastOverlay>,
        #[template_child]
        pub main_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub status_page: TemplateChild<adw::StatusPage>,
        #[template_child]
        pub song_cover: TemplateChild<SongCover>,
        #[template_child]
        pub song_details: TemplateChild<SongDetails>,
        #[template_child]
        pub waveform_view: TemplateChild<WaveformView>,
        #[template_child]
        pub elapsed_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub remaining_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub playback_control: TemplateChild<PlaybackControl>,
        #[template_child]
        pub split_view: TemplateChild<adw::OverlaySplitView>,
        #[template_child]
        pub playlist_view: TemplateChild<PlaylistView>,
        #[template_child]
        pub add_folder_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub restore_playlist_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub speed_button: TemplateChild<gtk::MenuButton>,
        #[template_child]
        pub speed_scale: TemplateChild<gtk::Scale>,
        #[template_child]
        pub speed_adjustment: TemplateChild<gtk::Adjustment>,
        #[template_child]
        pub speed_presets: TemplateChild<gtk::Box>,
        #[template_child]
        pub compact_box: TemplateChild<gtk::Box>,
        #[template_child]
        pub mini_cover: TemplateChild<gtk::Image>,
        #[template_child]
        pub mini_title: TemplateChild<gtk::Label>,
        #[template_child]
        pub mini_artist: TemplateChild<gtk::Label>,
        #[template_child]
        pub mini_album: TemplateChild<gtk::Label>,
        #[template_child]
        pub mini_queue_scroll: TemplateChild<gtk::ScrolledWindow>,
        #[template_child]
        pub mini_queue_sep: TemplateChild<gtk::Separator>,
        #[template_child]
        pub mini_queue: TemplateChild<gtk::ListView>,
        #[template_child]
        pub mini_play_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub mini_seek: TemplateChild<gtk::Scale>,
        #[template_child]
        pub mini_seek_adjustment: TemplateChild<gtk::Adjustment>,
        #[template_child]
        pub mini_time: TemplateChild<gtk::Label>,
        #[template_child]
        pub mini_volume: TemplateChild<gtk::ScaleButton>,
        #[template_child]
        pub mini_volume_adjustment: TemplateChild<gtk::Adjustment>,

        /// `full` / `mini` / `strip` — the current window skin.
        pub view_mode: Cell<super::ViewMode>,
        /// Guards the mini seek scale against feedback while syncing.
        pub mini_seek_guard: Cell<bool>,

        pub provider: gtk::CssProvider,
        pub settings: gio::Settings,

        pub playlist_shuffled: Cell<bool>,
        pub playlist_visible: Cell<bool>,
        pub playlist_selection: Cell<bool>,
        pub playlist_search: Cell<bool>,
        pub replaygain_mode: Cell<ReplayGainMode>,

        pub playlist_filtermodel: RefCell<Option<gio::ListModel>>,

        pub notify_playing_id: RefCell<Option<glib::SignalHandlerId>>,
        pub notify_position_id: RefCell<Option<glib::SignalHandlerId>>,
        pub notify_song_id: RefCell<Option<glib::SignalHandlerId>>,
        pub notify_cover_id: RefCell<Option<glib::SignalHandlerId>>,
        pub notify_nsongs_id: RefCell<Option<glib::SignalHandlerId>>,
        pub notify_current_id: RefCell<Option<glib::SignalHandlerId>>,
        pub notify_peaks_id: RefCell<Option<glib::SignalHandlerId>>,

        /// What to do once a library load finishes filling an empty queue
        /// (set by `load_library_files`, consumed in `queue_songs`).
        pub pending_start: RefCell<Option<super::StartIntent>>,

        /// The saved playlist backing the current queue, if any -- set by
        /// `open_playlist_file` when the path is one of the named
        /// saved playlists (`playlist::library_dir()`), cleared on
        /// folder/library load or loading a different playlist. Drives
        /// live resume-index persistence (see `persist_resume` call sites).
        pub active_playlist: RefCell<Option<PathBuf>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Window {
        const NAME: &'static str = "TuneWindow";
        type Type = super::Window;
        type ParentType = adw::ApplicationWindow;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);

            klass.install_action("win.play", None, move |win, _, _| {
                debug!("Window::win.play()");
                if let Some(p) = win.player() {
                    p.toggle_play();
                }
            });
            klass.install_action("win.seek-backwards", None, move |win, _, _| {
                debug!("Window::win.seek-backwards");
                if let Some(p) = win.player() {
                    p.seek_backwards();
                }
            });
            klass.install_action("win.seek-forward", None, move |win, _, _| {
                debug!("Window::win.seek-forward");
                if let Some(p) = win.player() {
                    p.seek_forward();
                }
            });
            klass.install_action("win.previous", None, move |win, _, _| {
                debug!("Window::win.previous()");
                if let Some(p) = win.player() {
                    p.skip_previous();
                }
            });
            klass.install_action("win.next", None, move |win, _, _| {
                debug!("Window::win.next()");
                if let Some(p) = win.player() {
                    p.skip_next();
                }
            });
            klass.install_action("queue.repeat-mode", None, move |win, _, _| {
                debug!("Window::queue.repeat()");
                if let Some(p) = win.player() {
                    p.toggle_repeat_mode();
                }
            });
            klass.install_action("win.repeat-count-increment", None, move |win, _, _| {
                win.adjust_repeat_count(1);
            });
            klass.install_action("win.repeat-count-decrement", None, move |win, _, _| {
                win.adjust_repeat_count(-1);
            });
            klass.install_action_async("queue.add-song", None, |win, _, _| async move {
                debug!("Window::win.add-song()");
                let filters = gio::ListStore::new::<gtk::FileFilter>();
                let filter = gtk::FileFilter::new();
                gtk::FileFilter::set_name(&filter, Some(&i18n("Audio files")));
                filter.add_mime_type("audio/*");
                filters.append(&filter);

                let xdg_music = match glib::user_special_dir(glib::UserDirectory::Music) {
                    Some(p) => p,
                    None => glib::current_dir(),
                };

                let dialog = gtk::FileDialog::builder()
                    .accept_label(i18n("_Add Song"))
                    .filters(&filters)
                    .initial_folder(&gio::File::for_path(&xdg_music))
                    .modal(true)
                    .title(i18n("Open File"))
                    .build();

                if let Ok(files) = dialog.open_multiple_future(Some(&win)).await {
                    if files.n_items() == 0 {
                        win.add_toast(i18n("Unable to access files"));
                    } else {
                        win.add_files_to_queue(&files);
                    }
                }
            });
            klass.install_action_async("queue.add-folder", None, |win, _, _| async move {
                debug!("Window::win.add-folder()");
                let xdg_music = match glib::user_special_dir(glib::UserDirectory::Music) {
                    Some(p) => p,
                    None => glib::current_dir(),
                };

                let dialog = gtk::FileDialog::builder()
                    .accept_label(i18n("_Add Folder"))
                    .initial_folder(&gio::File::for_path(&xdg_music))
                    .modal(true)
                    .title(i18n("Open Folder"))
                    .build();

                if let Ok(files) = dialog.select_multiple_folders_future(Some(&win)).await {
                    if files.n_items() == 0 {
                        win.add_toast(i18n("Unable to access files"));
                    } else {
                        win.add_files_to_queue(&files);
                    }
                }
            });
            klass.install_action("queue.restore-playlist", None, move |win, _, _| {
                debug!("Window::queue.restore-playlist()");
                win.restore_playlist();
            });
            klass.install_action_async("queue.open-playlist", None, |win, _, _| async move {
                let filters = gio::ListStore::new::<gtk::FileFilter>();
                let filter = gtk::FileFilter::new();
                gtk::FileFilter::set_name(&filter, Some(&i18n("Playlists")));
                for m in ["audio/x-mpegurl", "audio/mpegurl", "audio/x-scpls"] {
                    filter.add_mime_type(m);
                }
                for p in ["*.m3u", "*.m3u8", "*.pls"] {
                    filter.add_pattern(p);
                }
                filters.append(&filter);

                let dialog = gtk::FileDialog::builder()
                    .accept_label(i18n("_Open"))
                    .filters(&filters)
                    .initial_folder(&gio::File::for_path(crate::playlist::library_dir()))
                    .modal(true)
                    .title(i18n("Open Playlist"))
                    .build();

                if let Ok(file) = dialog.open_future(Some(&win)).await
                    && let Some(path) = file.path()
                {
                    win.open_playlist_file(&path);
                }
            });
            klass.install_action_async("queue.save-playlist", None, |win, _, _| async move {
                let entry = gtk::Entry::builder()
                    .placeholder_text(i18n("Playlist name"))
                    .activates_default(true)
                    .build();
                let dialog = adw::AlertDialog::builder()
                    .heading(i18n("Save Playlist"))
                    .body(i18n("Store the current queue in your playlist library"))
                    .extra_child(&entry)
                    .default_response("save")
                    .close_response("cancel")
                    .build();
                dialog.add_responses(&[("cancel", &i18n("_Cancel")), ("save", &i18n("_Save"))]);
                dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);
                if dialog.choose_future(Some(&win)).await == "save" {
                    let name = entry.text().trim().to_string();
                    if !name.is_empty() {
                        win.save_current_playlist(&name);
                    }
                }
            });
            klass.install_action(
                "queue.load-saved",
                Some(glib::VariantTy::STRING),
                move |win, _, param| {
                    if let Some(name) = param.and_then(glib::Variant::get::<String>) {
                        win.load_saved_playlist(&name);
                    }
                },
            );
            klass.install_action("win.copy", None, move |win, _, _| {
                debug!("Window::win.copy()");
                win.copy_song();
            });
            klass.install_action_async("win.open-directory", None, async move |win, _, _| {
                debug!("Window::win.open-directory()");
                win.open_song_directory().await
            });
            klass.install_action("queue.clear", None, move |win, _, _| {
                debug!("Window::queue.clear()");
                win.clear_queue();
            });
            klass.install_property_action("queue.toggle", "playlist-visible");
            klass.install_property_action("queue.shuffle", "playlist-shuffled");
            klass.install_property_action("queue.select", "playlist-selection");
            klass.install_property_action("queue.search", "playlist-search");
            klass.install_property_action("win.replaygain", "replaygain-mode");

            klass.install_action(
                "win.skip-to",
                Some(glib::VariantTy::UINT32),
                move |win, _, param| {
                    if let Some(pos) = param.and_then(u32::from_variant)
                        && let Some(player) = win.player()
                    {
                        player.skip_to(pos);
                        player.play();
                    }
                },
            );
            klass.install_action("win.cycle-view", None, move |win, _, _| {
                win.set_view_mode(win.imp().view_mode.get().next());
            });
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }

        fn new() -> Self {
            Self {
                song_details: TemplateChild::default(),
                song_cover: TemplateChild::default(),
                split_view: TemplateChild::default(),
                toast_overlay: TemplateChild::default(),
                drag_overlay: TemplateChild::default(),
                playback_control: TemplateChild::default(),
                waveform_view: TemplateChild::default(),
                elapsed_label: TemplateChild::default(),
                remaining_label: TemplateChild::default(),
                main_stack: TemplateChild::default(),
                status_page: TemplateChild::default(),
                add_folder_button: TemplateChild::default(),
                restore_playlist_button: TemplateChild::default(),
                speed_button: TemplateChild::default(),
                speed_scale: TemplateChild::default(),
                speed_adjustment: TemplateChild::default(),
                speed_presets: TemplateChild::default(),
                compact_box: TemplateChild::default(),
                mini_cover: TemplateChild::default(),
                mini_title: TemplateChild::default(),
                mini_artist: TemplateChild::default(),
                mini_album: TemplateChild::default(),
                mini_queue_scroll: TemplateChild::default(),
                mini_queue_sep: TemplateChild::default(),
                mini_queue: TemplateChild::default(),
                mini_play_button: TemplateChild::default(),
                mini_seek: TemplateChild::default(),
                mini_seek_adjustment: TemplateChild::default(),
                mini_time: TemplateChild::default(),
                mini_volume: TemplateChild::default(),
                mini_volume_adjustment: TemplateChild::default(),
                view_mode: Cell::new(super::ViewMode::Full),
                mini_seek_guard: Cell::new(false),
                playlist_view: TemplateChild::default(),
                playlist_shuffled: Cell::new(false),
                playlist_visible: Cell::new(true),
                playlist_selection: Cell::new(false),
                playlist_search: Cell::new(false),
                playlist_filtermodel: RefCell::default(),
                replaygain_mode: Cell::new(ReplayGainMode::default()),
                provider: gtk::CssProvider::new(),
                settings: utils::settings_manager(),
                notify_playing_id: RefCell::new(None),
                notify_position_id: RefCell::new(None),
                notify_song_id: RefCell::new(None),
                notify_cover_id: RefCell::new(None),
                notify_nsongs_id: RefCell::new(None),
                notify_current_id: RefCell::new(None),
                notify_peaks_id: RefCell::new(None),
                pending_start: RefCell::new(None),
                active_playlist: RefCell::new(None),
            }
        }
    }

    impl ObjectImpl for Window {
        fn constructed(&self) {
            self.parent_constructed();

            if APPLICATION_ID.ends_with("Devel") {
                self.obj().add_css_class("devel");
            }
        }

        fn properties() -> &'static [ParamSpec] {
            static PROPERTIES: Lazy<Vec<ParamSpec>> = Lazy::new(|| {
                vec![
                    ParamSpecBoolean::builder("playlist-shuffled").build(),
                    ParamSpecBoolean::builder("playlist-visible").build(),
                    ParamSpecBoolean::builder("playlist-selection").build(),
                    ParamSpecBoolean::builder("playlist-search").build(),
                    ParamSpecEnum::builder::<ReplayGainMode>("replaygain-mode").build(),
                ]
            });
            PROPERTIES.as_ref()
        }

        fn set_property(&self, _id: usize, value: &Value, pspec: &ParamSpec) {
            let obj = self.obj();
            match pspec.name() {
                "playlist-shuffled" => obj.set_playlist_shuffled(value.get::<bool>().unwrap()),
                "playlist-visible" => obj.set_playlist_visible(value.get::<bool>().unwrap()),
                "playlist-selection" => obj.set_playlist_selection(value.get::<bool>().unwrap()),
                "playlist-search" => obj.set_playlist_search(value.get::<bool>().unwrap()),
                "replaygain-mode" => obj.set_replaygain(value.get::<ReplayGainMode>().unwrap()),
                _ => unimplemented!(),
            }
        }

        fn property(&self, _id: usize, pspec: &ParamSpec) -> Value {
            let obj = self.obj();
            match pspec.name() {
                "playlist-shuffled" => obj.playlist_shuffled().to_value(),
                "playlist-visible" => obj.playlist_visible().to_value(),
                "playlist-selection" => obj.playlist_selection().to_value(),
                "playlist-search" => obj.playlist_search().to_value(),
                "replaygain-mode" => obj.replaygain().to_value(),
                _ => unimplemented!(),
            }
        }
    }

    impl WidgetImpl for Window {}
    impl WindowImpl for Window {}
    impl ApplicationWindowImpl for Window {}
    impl AdwApplicationWindowImpl for Window {}
}

glib::wrapper! {
    pub struct Window(ObjectSubclass<imp::Window>)
        @extends gtk::Widget, gtk::Window, gtk::ApplicationWindow, adw::ApplicationWindow,
        @implements gio::ActionGroup, gio::ActionMap, gtk::Accessible, gtk::Buildable,
                    gtk::ConstraintTarget, gtk::Native, gtk::Root, gtk::ShortcutManager;
}

impl Window {
    pub fn new<P: IsA<gtk::Application>>(application: &P) -> Self {
        let win = glib::Object::builder::<Window>()
            .property("application", application)
            .build();

        win.setup_waveform();
        win.setup_actions();
        win.setup_playlist();
        win.setup_speed();
        win.setup_compact();
        win.setup_drop_target();
        win.setup_provider();
        win.bind_state();
        win.bind_queue();
        win.connect_signals();
        win.restore_window_state();
        win.set_initial_state();

        // After the saved full-view size is restored: apply the saved skin.
        win.apply_view_mode(ViewMode::from_str(&win.imp().settings.string("view-mode")));

        win
    }

    /// Change the `RepeatMode::RepeatEach` count by `delta`, clamped to a
    /// sane 1..=20 range (0 would silently degrade the mode to "always
    /// advance" -- see `next_index_repeat_each`'s own reasoning). Applied
    /// through `Application::apply_repeat_count` so it persists to
    /// `mtune.toml` the same way the D-Bus/CLI path already does.
    fn adjust_repeat_count(&self, delta: i32) {
        let Some(player) = self.player() else { return };
        let cur = player.queue().repeat_count();
        let next = (cur as i32 + delta).clamp(1, 20) as u32;
        if let Some(app) = self
            .application()
            .and_then(|a| a.downcast::<crate::application::Application>().ok())
        {
            app.apply_repeat_count(next);
        }
    }

    fn player(&self) -> Option<Rc<AudioPlayer>> {
        if let Some(app) = self.application() {
            let player = app
                .downcast::<crate::application::Application>()
                .unwrap()
                .player();
            return Some(player);
        }

        None
    }

    fn setup_actions(&self) {
        let enable_recoloring = self.imp().settings.boolean("enable-recoloring");
        self.add_action_entries([gio::ActionEntry::builder("enable-recoloring")
            .state(enable_recoloring.to_variant())
            .activate(|this: &Window, action, _| {
                let state = action.state().unwrap();
                let action_state: bool = state.get().unwrap();
                let enable_recoloring = !action_state;
                action.set_state(&enable_recoloring.to_variant());

                this.imp()
                    .settings
                    .set_boolean("enable-recoloring", enable_recoloring)
                    .expect("Unable to store setting");
            })
            .build()])
    }

    fn restore_window_state(&self) {
        let settings = utils::settings_manager();
        let width = settings.int("window-width");
        let height = settings.int("window-height");
        self.set_default_size(width, height);
    }

    fn reset_queue(&self) {
        self.set_playlist_visible(false);
        self.set_playlist_shuffled(false);
        self.set_playlist_selection(false);
        self.update_style(None);
    }

    fn clear_queue(&self) {
        if let Some(p) = self.player() {
            p.clear_queue();
        }
    }

    fn setup_drop_target(&self) {
        let drop_target = gtk::DropTarget::builder()
            .name("file-drop-target")
            .actions(gdk::DragAction::COPY)
            .formats(&gdk::ContentFormats::for_type(gdk::FileList::static_type()))
            .build();

        drop_target.connect_drop(clone!(
            #[weak(rename_to = win)]
            self,
            #[upgrade_or]
            false,
            move |_, value, _, _| {
                if let Ok(file_list) = value.get::<gdk::FileList>() {
                    if file_list.files().is_empty() {
                        win.add_toast(i18n("Unable to access dropped files"));
                        return false;
                    }

                    let model = gio::ListStore::new::<gio::File>();
                    for f in file_list.files() {
                        model.append(&f);
                    }
                    win.add_files_to_queue(model.upcast_ref::<gio::ListModel>());
                    return true;
                }

                false
            }
        ));

        self.imp().drag_overlay.set_drop_target(&drop_target);
    }

    /// Load the folder library into the queue and, once it lands in a
    /// previously-empty queue, apply `intent` (resume / top / nothing)
    /// instead of the blind `skip_to(0)` that `open_files` would do.
    pub fn load_library_files(&self, files: Vec<gio::File>, intent: StartIntent) {
        if files.is_empty() {
            return;
        }
        // A folder/library load is never a saved playlist -- stop treating
        // whatever the previous queue's source was as one.
        self.imp().active_playlist.replace(None);
        self.imp().pending_start.replace(Some(intent));
        self.open_files(&files);
    }

    pub fn open_files(&self, files: &[gio::File]) {
        if files.is_empty() {
            self.add_toast(i18n("Unable to access files"));
            return;
        }

        // A single playlist file → open it as the queue.
        if let [only] = files
            && let Some(path) = only.path()
            && matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("m3u") | Some("m3u8") | Some("pls")
            )
        {
            self.open_playlist_file(&path);
            return;
        }

        let model = gio::ListStore::new::<gio::File>();
        for f in files {
            // Expand any playlist argument into its entries.
            let ext = f
                .path()
                .as_deref()
                .and_then(|p| p.extension())
                .and_then(|e| e.to_str())
                .map(str::to_owned);
            match (ext.as_deref(), f.path()) {
                (Some("m3u" | "m3u8" | "pls"), Some(path)) => {
                    for p in crate::playlist::parse(&path) {
                        model.append(&gio::File::for_path(p));
                    }
                }
                _ => model.append(f),
            }
        }
        self.add_files_to_queue(model.upcast_ref::<gio::ListModel>());
    }

    pub fn remove_song(&self, song: &Song) {
        if let Some(p) = self.player() {
            p.remove_song(song);
        }
        self.update_selected_count();
        self.update_playlist_time();
    }

    pub fn add_toast(&self, msg: String) {
        let toast = adw::Toast::new(&msg);
        self.imp().toast_overlay.add_toast(toast);
    }

    pub fn add_skip_to_toast(&self, msg: String, button: String, pos: u32) {
        let toast = adw::Toast::new(&msg);
        toast.set_button_label(Some(&button));
        toast.set_action_name(Some("win.skip-to"));
        toast.set_action_target_value(Some(&pos.to_variant()));
        self.imp().toast_overlay.add_toast(toast);
    }

    fn copy_song(&self) {
        if let Some(player) = self.player() {
            let state = player.state();
            if let Some(song) = state.current_song() {
                let s = i18n_k(
                    // Translators: `{title}` and `{artist}` must be left
                    // untranslated; they will expand to the title and
                    // artist of the currently playing song, respectively
                    "Currently playing “{title}” by “{artist}”",
                    &[("title", &song.title()), ("artist", &song.artist())],
                );
                self.clipboard().set_text(&s);
            }
        }
    }

    async fn open_song_directory(&self) {
        if let Some(player) = self.player() {
            let state = player.state();

            if let Some(song) = state.current_song() {
                let file = song.file();

                match file
                    .query_info_future(
                        ATTRIBUTE_HOST_PATH,
                        gio::FileQueryInfoFlags::NONE,
                        glib::Priority::DEFAULT,
                    )
                    .await
                {
                    Err(err) => debug!("Unable to get host-path attribute: {err}"),
                    Ok(info) => {
                        let host_path = info
                            .attribute_as_string(ATTRIBUTE_HOST_PATH)
                            .and_then(|x| PathBuf::from_str(x.as_str()).ok());

                        if let Some(path) = host_path.or_else(|| file.path()) {
                            let launcher = gtk::FileLauncher::new(Some(&gio::File::for_path(path)));
                            if let Err(_e) = launcher
                                .open_containing_folder_future(Some(
                                    self.upcast_ref::<gtk::Window>(),
                                ))
                                .await
                            {
                                self.add_toast(i18n("Unable to open folder"))
                            }
                        }
                    }
                }
            }
        }
    }

    pub fn switch_mode(&self, mode: WindowMode) {
        let stack = self.imp().main_stack.get();
        match mode {
            WindowMode::InitialView => {
                stack.set_visible_child_name("initial-view");
                self.set_default_widget(Some(&self.imp().add_folder_button.get()));
            }
            WindowMode::MainView => {
                stack.set_visible_child_name("main-view");
                self.set_default_widget(Some(&self.imp().playback_control.play_button()));
                // A queue now exists — honour the saved compact skin
                // (`apply_view_mode` no-ops while the initial screen is up).
                self.apply_view_mode(self.imp().view_mode.get());
            }
        };
    }

    pub fn set_replaygain(&self, replaygain: ReplayGainMode) {
        let imp = self.imp();

        if replaygain != imp.replaygain_mode.replace(replaygain) {
            if let Some(p) = self.player() {
                p.set_replaygain(replaygain);
            }
            self.imp()
                .settings
                .set_enum("replay-gain", replaygain.into())
                .expect("Unable to store setting");

            self.notify("replaygain-mode");
        }
    }

    pub fn replaygain(&self) -> ReplayGainMode {
        self.imp().replaygain_mode.get()
    }

    pub fn set_song_time(&self, elapsed: Option<u64>, remaining: Option<u64>) {
        if let Some(elapsed) = elapsed {
            self.imp()
                .elapsed_label
                .set_text(&utils::format_time(elapsed as i64));
        } else {
            self.imp().elapsed_label.set_text("0:00");
        }

        if let Some(remaining) = remaining {
            self.imp()
                .remaining_label
                .set_text(&utils::format_remaining_time(remaining as i64));
        } else {
            self.imp().remaining_label.set_text("0:00");
        }
    }

    pub fn set_song_position(&self, position: f64) {
        self.imp().waveform_view.set_position(position);
    }
}
