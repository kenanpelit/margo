// SPDX-License-Identifier: GPL-3.0-or-later

//! The radio and podcast libraries for this process, shared by the
//! browser dialog and the playback tracker (GTK thread only), plus a
//! single writer thread that saves them off the GTK thread, in order.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::mpsc,
};

use crate::{podcasts::store::PodcastLibrary, radio::store::RadioLibrary};

/// What changed, for open pages to refresh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Stations,
    Subscriptions,
    /// An episode's played state / resume point.
    Progress,
}

/// A loaded library and whether it may be saved (a file that did not
/// parse is never written over).
pub struct Lib<T> {
    pub data: Rc<RefCell<T>>,
    pub writable: bool,
}

impl<T> Clone for Lib<T> {
    fn clone(&self) -> Self {
        Self {
            data: self.data.clone(),
            writable: self.writable,
        }
    }
}

enum Job {
    Radio(RadioLibrary),
    Podcasts(PodcastLibrary),
}

type Listener = Box<dyn Fn(Change)>;

thread_local! {
    static RADIO: RefCell<Option<Lib<RadioLibrary>>> = const { RefCell::new(None) };
    static PODCASTS: RefCell<Option<Lib<PodcastLibrary>>> = const { RefCell::new(None) };
    static WRITER: RefCell<Option<mpsc::Sender<Job>>> = const { RefCell::new(None) };
    static WRITER_THREAD: RefCell<Option<std::thread::JoinHandle<()>>> = const { RefCell::new(None) };
    static LISTENERS: RefCell<Vec<(u64, Rc<Listener>)>> = const { RefCell::new(Vec::new()) };
    static NEXT_ID: Cell<u64> = const { Cell::new(1) };
}

pub fn radio() -> Lib<RadioLibrary> {
    RADIO.with(|r| {
        r.borrow_mut()
            .get_or_insert_with(|| {
                let (data, writable) = RadioLibrary::load();
                Lib {
                    data: Rc::new(RefCell::new(data)),
                    writable,
                }
            })
            .clone()
    })
}

pub fn podcasts() -> Lib<PodcastLibrary> {
    PODCASTS.with(|r| {
        r.borrow_mut()
            .get_or_insert_with(|| {
                let (data, writable) = PodcastLibrary::load();
                Lib {
                    data: Rc::new(RefCell::new(data)),
                    writable,
                }
            })
            .clone()
    })
}

fn send(job: Job) {
    WRITER.with(|w| {
        let mut w = w.borrow_mut();
        let tx = w.get_or_insert_with(|| {
            let (tx, rx) = mpsc::channel::<Job>();
            match std::thread::Builder::new()
                .name("mtune-store".into())
                .spawn(move || writer(rx))
            {
                Ok(handle) => WRITER_THREAD.with(|t| t.replace(Some(handle))),
                Err(e) => {
                    log::warn!("mtune: cannot start the library writer: {e}");
                    None
                }
            };
            tx
        });
        if tx.send(job).is_err() {
            log::warn!("mtune: library writer is gone; change not saved");
        }
    });
}

/// Finish pending saves (at shutdown): closing the channel ends the writer
/// once it has drained it.
pub fn flush() {
    WRITER.with(|w| w.borrow_mut().take());
    if let Some(handle) = WRITER_THREAD.with(|t| t.borrow_mut().take()) {
        let _ = handle.join();
    }
}

/// Save jobs in order; a burst of saves of the same file collapses into
/// the newest.
fn writer(rx: mpsc::Receiver<Job>) {
    while let Ok(first) = rx.recv() {
        let (mut radio, mut pods) = (None, None);
        for job in std::iter::once(first).chain(rx.try_iter()) {
            match job {
                Job::Radio(l) => radio = Some(l),
                Job::Podcasts(l) => pods = Some(l),
            }
        }
        if let Some(l) = radio
            && let Err(e) = l.save()
        {
            log::warn!("mtune: saving radio.json: {e}");
        }
        if let Some(l) = pods
            && let Err(e) = l.save()
        {
            log::warn!("mtune: saving podcasts.json: {e}");
        }
    }
}

/// Persist the radio library (no-op when its file was unreadable) and tell
/// open pages. Returns `false` when the change cannot be saved.
pub fn radio_changed() -> bool {
    let lib = radio();
    if lib.writable {
        send(Job::Radio(lib.data.borrow().clone()));
    }
    emit(Change::Stations);
    lib.writable
}

/// Persist the podcast library and tell open pages what changed.
pub fn podcasts_changed(change: Change) -> bool {
    let lib = podcasts();
    if lib.writable {
        send(Job::Podcasts(lib.data.borrow().clone()));
    }
    emit(change);
    lib.writable
}

/// Tell open pages without saving (in-memory progress between saves).
pub fn emit(change: Change) {
    // Clone out first: a listener may (un)register listeners.
    let all: Vec<Rc<Listener>> =
        LISTENERS.with(|l| l.borrow().iter().map(|(_, f)| f.clone()).collect());
    for f in all {
        f(change);
    }
}

/// Register a page refresher; returns the id for [`unlisten`].
pub fn listen(f: impl Fn(Change) + 'static) -> u64 {
    let id = NEXT_ID.with(|n| {
        let id = n.get();
        n.set(id + 1);
        id
    });
    LISTENERS.with(|l| l.borrow_mut().push((id, Rc::new(Box::new(f)))));
    id
}

pub fn unlisten(id: u64) {
    LISTENERS.with(|l| l.borrow_mut().retain(|(i, _)| *i != id));
}
