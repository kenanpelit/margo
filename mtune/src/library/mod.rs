// SPDX-License-Identifier: GPL-3.0-or-later
//! Folder-first library subsystem: persistent roots, recursive scan,
//! inotify watch, on-disk tag index.
//!
//! See `docs/superpowers/plans/2026-09-03-mtune-app-foundation.md` phase 2.

// The pieces land task-by-task (config -> scanner -> index -> watcher) and
// are only consumed once `Application` startup is wired (Task 10), which
// removes this allow.
#![allow(dead_code)]

pub mod config;
pub mod index;
pub mod scanner;
pub mod watcher;

use std::path::PathBuf;

/// A live change to the library while mtune is running.
#[derive(Debug, Clone)]
pub enum LibraryEvent {
    Added(PathBuf),
    Removed(PathBuf),
}
