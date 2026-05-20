use std::io::Read;
use std::os::unix::io::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;

use time::OffsetDateTime;
use tokio::sync::broadcast;
use tracing::{debug, error, info, warn};
use wayland_client::protocol::wl_registry;
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::{
    Connection, Dispatch, EventQueue, QueueHandle, delegate_noop, event_created_child,
};

use wayland_protocols::ext::data_control::v1::client::{
    ext_data_control_device_v1::{self, ExtDataControlDeviceV1},
    ext_data_control_manager_v1::ExtDataControlManagerV1,
    ext_data_control_offer_v1::{self, ExtDataControlOfferV1},
    ext_data_control_source_v1::{self, ExtDataControlSourceV1},
};

use crate::entry::{ClipboardEntry, EntryPreview};
use crate::history::ClipboardHistory;
use crate::persist;
use crate::settings::{ClearPolicy, ClipboardSettings};

/// MIME hint password managers (KeePassXC, Bitwarden, KDE Klipper,
/// …) attach to a copy they consider secret. When `skip_sensitive`
/// is on we drop any offer advertising it so passwords never enter
/// history.
const SENSITIVE_HINT_MIME: &str = "x-kde-passwordManagerHint";

/// Live, shareable settings — read by the watcher thread on each
/// new offer and by the UI thread on each mutation (to decide what
/// to persist).
type SharedSettings = Arc<Mutex<ClipboardSettings>>;

/// Write the current history to disk *synchronously*, per the active
/// persist mode. Used only at startup (before the background writer
/// thread exists), when the calling thread is the sole writer. Every
/// runtime mutation goes through [`Persister::request`] instead, so
/// the on-disk store is never rewritten more than once per burst.
fn persist_now(history: &ClipboardHistory, settings: &SharedSettings) {
    let mode = settings.lock().unwrap().persist;
    persist::save(&history.entries(), mode);
}

/// Message to the background persist thread.
enum PersistMsg {
    /// Mark the on-disk store dirty. The persist thread coalesces a
    /// burst of these into a single write once the burst settles.
    Save,
}

/// Background, debounced disk writer — the *sole* owner of clipboard
/// disk writes once the watcher is up.
///
/// `persist::save` is `O(n)`: it re-serialises the whole history to
/// `history.json` every call. Doing that synchronously on every copy
/// (and every pin/delete click) put an `O(n)` rewrite on the hot
/// path. This collapses a burst of mutations into one write, and —
/// because it snapshots `history` + the active mode at *flush* time,
/// not request time — a coalesced write always reflects the latest
/// state (e.g. a "clear" that lands mid-burst is honoured, never
/// overwritten by a stale snapshot). Being the only writer also means
/// no two threads race on the `history.json.tmp` temp file.
#[derive(Clone)]
struct Persister {
    tx: mpsc::Sender<PersistMsg>,
}

impl Persister {
    /// Trailing debounce: after the last edit in a burst, wait this
    /// long with no further edits before writing.
    const DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(800);
    /// Hard cap from the burst's first edit, so a continuous stream
    /// of copies still reaches disk on a bounded cadence.
    const MAX_WINDOW: std::time::Duration = std::time::Duration::from_secs(4);

    fn spawn(history: ClipboardHistory, settings: SharedSettings) -> Self {
        let (tx, rx) = mpsc::channel::<PersistMsg>();
        let _ = thread::Builder::new()
            .name("mshell-clipboard-persist".into())
            .spawn(move || persist_loop(history, settings, rx));
        Self { tx }
    }

    /// Request a (debounced) save. Cheap and non-blocking — never
    /// touches the disk on the caller's thread.
    fn request(&self) {
        let _ = self.tx.send(PersistMsg::Save);
    }
}

fn persist_loop(
    history: ClipboardHistory,
    settings: SharedSettings,
    rx: mpsc::Receiver<PersistMsg>,
) {
    use std::sync::mpsc::RecvTimeoutError;
    use std::time::Instant;

    // Block until the first dirty signal; `recv` returning Err means
    // every sender dropped (shell shutting down) — exit the thread.
    while rx.recv().is_ok() {
        // A burst started. Coalesce until it quiets for DEBOUNCE, or
        // until MAX_WINDOW elapses since the burst's first edit.
        let burst_start = Instant::now();
        loop {
            let elapsed = burst_start.elapsed();
            if elapsed >= Persister::MAX_WINDOW {
                break;
            }
            let wait = Persister::DEBOUNCE.min(Persister::MAX_WINDOW - elapsed);
            match rx.recv_timeout(wait) {
                Ok(PersistMsg::Save) => continue, // more edits — keep coalescing
                Err(RecvTimeoutError::Timeout) => break, // settled — write now
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        // Snapshot at flush time so a coalesced write is never stale.
        let mode = settings.lock().unwrap().persist;
        persist::save(&history.entries(), mode);
    }
}

/// 10 MB
const MAX_DATA_SIZE: usize = 10 * 1024 * 1024;

/// Capacity of the broadcast channel.
const BROADCAST_CAPACITY: usize = 64;

/// preferred mine types in priority order
const TEXT_MIME_PRIORITY: &[&str] = &[
    "text/plain;charset=utf-8",
    "text/plain",
    "UTF8_STRING",
    "STRING",
    "TEXT",
];

/// preferred mine types in priority order
const IMAGE_MIME_PRIORITY: &[&str] = &["image/png", "image/jpeg", "image/bmp", "image/tiff"];

#[derive(Clone, Debug)]
pub enum ClipboardEvent {
    NewEntry(u64),
    EntryRemoved(u64),
    Cleared,
}

/// Commands sent from the UI thread to the watcher thread.
enum WatcherCommand {
    /// Set the clipboard selection to this data.
    SetSelection { mime_type: String, data: Vec<u8> },
}

/// Watches the Wayland clipboard via `ext-data-control-v1` and records
/// entries into a [`ClipboardHistory`].
pub struct ClipboardWatcher {
    history: ClipboardHistory,
    event_tx: broadcast::Sender<ClipboardEvent>,
    command_tx: mpsc::Sender<WatcherCommand>,
    settings: SharedSettings,
    persister: Persister,
}

impl ClipboardWatcher {
    /// Spawns a background thread that connects to the Wayland compositor
    /// and listens for clipboard changes via `ext-data-control-v1`.
    pub fn start(settings: ClipboardSettings) -> Result<Self, Box<dyn std::error::Error>> {
        let history = ClipboardHistory::new(settings.max_entries);

        // Restore persisted history, then apply the clear policy so
        // the rolling (non-pinned) entries honour the user's choice
        // on this launch. Pinned entries always survive.
        let restored = persist::load();
        history.load_snapshot(restored);
        match settings.clear_policy {
            ClearPolicy::Never => {}
            ClearPolicy::OnLogout => history.clear_unpinned(),
            ClearPolicy::AfterHours => {
                history.prune_older_than(settings.clear_after_hours as i64 * 3600);
            }
        }

        let shared: SharedSettings = Arc::new(Mutex::new(settings));
        // Re-persist after the load-time prune so disk matches RAM.
        // Synchronous: we're still single-threaded here, before the
        // background writer or watcher threads exist.
        persist_now(&history, &shared);

        // From here on the persist thread is the sole disk writer;
        // all runtime mutations route through it (debounced).
        let persister = Persister::spawn(history.clone(), shared.clone());

        let (event_tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        let (command_tx, command_rx) = mpsc::channel();

        let watcher_history = history.clone();
        let watcher_tx = event_tx.clone();
        let watcher_settings = shared.clone();
        let watcher_persister = persister.clone();
        thread::Builder::new()
            .name("mshell-clipboard-watcher".into())
            .spawn(move || {
                if let Err(e) = run_watcher(
                    watcher_history,
                    watcher_tx,
                    command_rx,
                    watcher_settings,
                    watcher_persister,
                ) {
                    error!("Clipboard watcher died: {e}");
                }
            })?;

        Ok(Self {
            history,
            event_tx,
            command_tx,
            settings: shared,
            persister,
        })
    }

    pub fn history(&self) -> &ClipboardHistory {
        &self.history
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ClipboardEvent> {
        self.event_tx.subscribe()
    }

    pub(crate) fn broadcast(&self, event: ClipboardEvent) {
        let _ = self.event_tx.send(event);
    }

    pub fn copy_entry(&self, id: u64) {
        if let Some(entry) = self.history.get(id) {
            self.history.promote(id);
            self.persister.request();
            self.broadcast(ClipboardEvent::NewEntry(id));
            let _ = self.command_tx.send(WatcherCommand::SetSelection {
                mime_type: entry.mime_type.clone(),
                data: entry.data.clone(),
            });
        }
    }

    pub fn delete_entry(&self, id: u64) {
        if self.history.remove(id) {
            self.persister.request();
            self.broadcast(ClipboardEvent::EntryRemoved(id));
        }
    }

    pub fn clear_history(&self) {
        self.history.clear();
        self.persister.request();
        self.broadcast(ClipboardEvent::Cleared);
    }

    /// Clear everything except pinned (favourite) entries.
    pub fn clear_unpinned(&self) {
        self.history.clear_unpinned();
        self.persister.request();
        self.broadcast(ClipboardEvent::Cleared);
    }

    /// Toggle the pinned flag on an entry, persist, and refresh.
    pub fn toggle_pin(&self, id: u64) {
        if self.history.toggle_pin(id).is_some() {
            self.persister.request();
            self.broadcast(ClipboardEvent::NewEntry(id));
        }
    }

    /// Apply a live settings change (from the Settings UI). Updates
    /// the shared knobs, resizes/prunes history, and re-persists.
    pub fn apply_settings(&self, new: ClipboardSettings) {
        self.history.set_max_entries(new.max_entries);
        if new.clear_policy == ClearPolicy::AfterHours {
            self.history
                .prune_older_than(new.clear_after_hours as i64 * 3600);
        }
        *self.settings.lock().unwrap() = new;
        self.persister.request();
        self.broadcast(ClipboardEvent::Cleared);
    }
}

// ---------------------------------------------------------------------------
// Wayland state & dispatch
// ---------------------------------------------------------------------------

/// Internal state for the Wayland event loop.
struct WatcherState {
    history: ClipboardHistory,
    event_tx: broadcast::Sender<ClipboardEvent>,
    conn: Connection,
    command_rx: mpsc::Receiver<WatcherCommand>,
    settings: SharedSettings,
    persister: Persister,

    // Globals
    manager: Option<ExtDataControlManagerV1>,
    seat: Option<WlSeat>,
    device: Option<ExtDataControlDeviceV1>,

    // Current offer being assembled
    pending_offer: Option<PendingOffer>,

    /// When we set the selection ourselves, the compositor echoes it back
    /// as a Selection event. We need to skip that to avoid reading from
    /// our own source (which would block or produce garbage).
    skip_next_selection: bool,
}

struct PendingOffer {
    offer: ExtDataControlOfferV1,
    mime_types: Vec<String>,
}

fn run_watcher(
    history: ClipboardHistory,
    event_tx: broadcast::Sender<ClipboardEvent>,
    command_rx: mpsc::Receiver<WatcherCommand>,
    settings: SharedSettings,
    persister: Persister,
) -> Result<(), Box<dyn std::error::Error>> {
    let conn = Connection::connect_to_env()?;
    let display = conn.display();

    let mut event_queue: EventQueue<WatcherState> = conn.new_event_queue();
    let qh = event_queue.handle();

    let mut state = WatcherState {
        history,
        event_tx,
        conn: conn.clone(),
        command_rx,
        settings,
        persister,
        manager: None,
        seat: None,
        device: None,
        pending_offer: None,
        skip_next_selection: false,
    };

    // Trigger registry enumeration.
    display.get_registry(&qh, ());
    event_queue.roundtrip(&mut state)?;

    // Bind the data control device for our seat.
    if let (Some(manager), Some(seat)) = (&state.manager, &state.seat) {
        let device = manager.get_data_device(seat, &qh, ());
        state.device = Some(device);
    } else {
        return Err("Missing ext_data_control_manager_v1 or wl_seat".into());
    }

    info!("Clipboard watcher started");

    // We need to wake the event loop when a command arrives, not just
    // when a Wayland event comes in. Use the Wayland fd + a short poll
    // timeout so we can check the command channel periodically.
    let wayland_fd = conn.as_fd();

    loop {
        // Flush any pending outgoing requests.
        conn.flush().ok();

        // Dispatch any events already in the queue.
        event_queue.dispatch_pending(&mut state)?;

        // Prepare for blocking read.
        let read_guard = event_queue.prepare_read().unwrap();

        // Poll the Wayland fd with a short timeout so we also
        // check the command channel. 50ms is responsive enough.
        let mut poll_fd = [libc::pollfd {
            fd: wayland_fd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        }];
        let ret = unsafe { libc::poll(poll_fd.as_mut_ptr(), 1, 50) };

        if ret > 0 && (poll_fd[0].revents & libc::POLLIN != 0) {
            // Wayland data available — read and dispatch.
            read_guard.read().ok();
            event_queue.dispatch_pending(&mut state)?;
        } else {
            // Timeout or no data — cancel the read guard.
            drop(read_guard);
        }

        // Process any pending commands from the UI thread.
        while let Ok(cmd) = state.command_rx.try_recv() {
            match cmd {
                WatcherCommand::SetSelection { mime_type, data } => {
                    if let (Some(manager), Some(device)) = (&state.manager, &state.device) {
                        let source = manager.create_data_source(&qh, data);
                        source.offer(mime_type);
                        device.set_selection(Some(&source));
                        state.skip_next_selection = true;
                        conn.flush().ok();
                        info!("Set clipboard selection from history");
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

impl Dispatch<wl_registry::WlRegistry, ()> for WatcherState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "ext_data_control_manager_v1" => {
                    let manager = registry.bind::<ExtDataControlManagerV1, _, _>(
                        name,
                        version.min(1),
                        qh,
                        (),
                    );
                    state.manager = Some(manager);
                    debug!("Bound ext_data_control_manager_v1");
                }
                "wl_seat" => {
                    let seat = registry.bind::<WlSeat, _, _>(name, version.min(1), qh, ());
                    state.seat = Some(seat);
                    debug!("Bound wl_seat");
                }
                _ => {}
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Data control device — selection events
// ---------------------------------------------------------------------------

impl Dispatch<ExtDataControlDeviceV1, ()> for WatcherState {
    fn event(
        state: &mut Self,
        _proxy: &ExtDataControlDeviceV1,
        event: ext_data_control_device_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            ext_data_control_device_v1::Event::DataOffer { id } => {
                // A new offer is being introduced. Start collecting MIME types.
                state.pending_offer = Some(PendingOffer {
                    offer: id,
                    mime_types: Vec::new(),
                });
            }
            ext_data_control_device_v1::Event::Selection { id } => {
                // The selection is now set to this offer (or NULL to clear).
                let offer_obj = match id {
                    Some(offer) => offer,
                    None => {
                        // Selection cleared — nothing to record.
                        state.pending_offer = None;
                        return;
                    }
                };

                // When we set the selection ourselves via copy_entry(),
                // the compositor echoes it back. Skip that echo.
                if state.skip_next_selection {
                    state.skip_next_selection = false;
                    // Still need to destroy the offer.
                    if let Some(pending) = state.pending_offer.take() {
                        pending.offer.destroy();
                    }
                    debug!("Skipped echoed selection from our own set_selection");
                    return;
                }

                // Take the pending offer that matches this object.
                let pending = match state.pending_offer.take() {
                    Some(p) if p.offer == offer_obj => p,
                    other => {
                        // Put it back if it didn't match (shouldn't happen).
                        state.pending_offer = other;
                        warn!("Selection event without matching pending offer");
                        return;
                    }
                };

                // Read the live filter knobs.
                let (skip_sensitive, image_history) = {
                    let s = state.settings.lock().unwrap();
                    (s.skip_sensitive, s.image_history)
                };

                // Password managers (KeePassXC / Bitwarden / KDE)
                // tag secret copies with this hint mime — drop them
                // entirely so passwords never enter history.
                if skip_sensitive
                    && pending
                        .mime_types
                        .iter()
                        .any(|m| m == SENSITIVE_HINT_MIME)
                {
                    debug!("Skipped sensitive (password-manager) clipboard entry");
                    pending.offer.destroy();
                    return;
                }

                // Pick the best MIME type and read the data.
                if let Some(mime) = pick_best_mime(&pending.mime_types) {
                    // Honour the image-history toggle.
                    if !image_history && mime.starts_with("image/") {
                        pending.offer.destroy();
                        return;
                    }
                    match read_offer_data(&pending.offer, &mime, &state.conn) {
                        Ok(data) => {
                            // Build the entry and push it into history.
                            if let Some(entry) = build_entry(mime, data) {
                                let id = state.history.push(entry);
                                state.persister.request();
                                let _ = state.event_tx.send(ClipboardEvent::NewEntry(id));
                            }
                        }
                        Err(e) => {
                            warn!("Failed to read clipboard data: {e}");
                        }
                    }
                }

                // Destroy the offer now that we've consumed it.
                pending.offer.destroy();
            }
            ext_data_control_device_v1::Event::PrimarySelection { .. } => {
                // We intentionally ignore primary selection for history.
            }
            ext_data_control_device_v1::Event::Finished => {
                info!("Data control device finished");
            }
            _ => {}
        }
    }

    event_created_child!(WatcherState, ExtDataControlDeviceV1, [
        // Opcode 0 = data_offer event, creates an ExtDataControlOfferV1
        0 => (ExtDataControlOfferV1, ()),
    ]);
}

// ---------------------------------------------------------------------------
// Data control offer — MIME type accumulation
// ---------------------------------------------------------------------------

impl Dispatch<ExtDataControlOfferV1, ()> for WatcherState {
    fn event(
        state: &mut Self,
        _proxy: &ExtDataControlOfferV1,
        event: ext_data_control_offer_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let ext_data_control_offer_v1::Event::Offer { mime_type } = event
            && let Some(pending) = &mut state.pending_offer
        {
            pending.mime_types.push(mime_type);
        }
    }
}

// ---------------------------------------------------------------------------
// Data control source — for pasting back from history
// ---------------------------------------------------------------------------

impl Dispatch<ExtDataControlSourceV1, Vec<u8>> for WatcherState {
    fn event(
        _state: &mut Self,
        _proxy: &ExtDataControlSourceV1,
        event: ext_data_control_source_v1::Event,
        data: &Vec<u8>,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            ext_data_control_source_v1::Event::Send { mime_type: _, fd } => {
                // The fd is an OwnedFd — wayland-client transfers ownership
                // to us. Convert to File, write data, and let File close it
                // on drop.
                let mut file = std::fs::File::from(fd);
                let _ = std::io::Write::write_all(&mut file, data);
            }
            ext_data_control_source_v1::Event::Cancelled => {
                debug!("Data source cancelled");
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Manager — no events to handle, just need the dispatch impl
// ---------------------------------------------------------------------------

impl Dispatch<ExtDataControlManagerV1, ()> for WatcherState {
    fn event(
        _state: &mut Self,
        _proxy: &ExtDataControlManagerV1,
        _event: <ExtDataControlManagerV1 as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

// wl_seat — we don't care about its events here
delegate_noop!(WatcherState: ignore WlSeat);

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Pick the best MIME type from the offered set.
///
/// Prefers text types first, then image types, then falls back to the first
/// available type.
fn pick_best_mime(offered: &[String]) -> Option<String> {
    // Try text types first.
    for preferred in TEXT_MIME_PRIORITY {
        if offered.iter().any(|m| m == preferred) {
            return Some(preferred.to_string());
        }
    }

    // Then image types.
    for preferred in IMAGE_MIME_PRIORITY {
        if offered.iter().any(|m| m == preferred) {
            return Some(preferred.to_string());
        }
    }

    // Fall back to the first offered type (if any).
    offered.first().cloned()
}

/// Read data from an offer via pipe.
fn read_offer_data(
    offer: &ExtDataControlOfferV1,
    mime_type: &str,
    conn: &Connection,
) -> Result<Vec<u8>, std::io::Error> {
    let (read_fd, write_fd) = nix_pipe()?;

    // Send the receive request — the source app will write to write_fd.
    offer.receive(mime_type.to_string(), write_fd.as_fd());

    // Flush the connection so the compositor sees the receive request
    // and forwards it to the source app. Without this, the read below
    // will block forever waiting for data that was never requested.
    conn.flush().map_err(std::io::Error::other)?;

    // Close the write end so we get EOF when the source is done.
    drop(write_fd);

    // Read from the pipe.
    let mut data = Vec::new();
    let mut file = std::fs::File::from(read_fd);

    let mut buf = [0u8; 8192];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        data.extend_from_slice(&buf[..n]);
        if data.len() > MAX_DATA_SIZE {
            warn!("Clipboard data exceeds {MAX_DATA_SIZE} bytes, truncating");
            data.truncate(MAX_DATA_SIZE);
            break;
        }
    }

    Ok(data)
}

/// Create a pipe, returning (read_end, write_end) as `OwnedFd`.
fn nix_pipe() -> Result<(OwnedFd, OwnedFd), std::io::Error> {
    let mut fds = [0i32; 2];
    let ret = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) };
    if ret < 0 {
        return Err(std::io::Error::last_os_error());
    }
    unsafe { Ok((OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1]))) }
}

fn build_entry(mime_type: String, data: Vec<u8>) -> Option<ClipboardEntry> {
    if data.is_empty() {
        return None;
    }

    let content_hash = ClipboardEntry::content_hash(&data);
    let preview = EntryPreview::build(&mime_type, &data);

    Some(ClipboardEntry {
        id: 0, // assigned by ClipboardHistory::push
        timestamp: OffsetDateTime::now_utc(),
        mime_type,
        content_hash,
        preview,
        data,
        pinned: false,
    })
}
