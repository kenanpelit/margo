//! Unique IDs for screencast sessions + streams.
//!
//! Direct port of niri/src/utils/mod.rs `CastSessionId` /
//! `CastStreamId`. We keep the same shape (u64 wrapper, monotonic
//! counter) so the ported D-Bus interfaces don't need adaptation
//! beyond `use` paths.

use std::fmt::{self, Display};
use std::sync::atomic::{AtomicU64, Ordering};

/// Unique ID for a screencast session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CastSessionId(u64);

impl CastSessionId {
    pub fn next() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(1);
        Self(COUNTER.fetch_add(1, Ordering::Relaxed))
    }

    pub fn get(self) -> u64 {
        self.0
    }
}

impl Display for CastSessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<u64> for CastSessionId {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

/// Unique ID for a screencast stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CastStreamId(u64);

impl CastStreamId {
    pub fn next() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(1);
        Self(COUNTER.fetch_add(1, Ordering::Relaxed))
    }

    pub fn get(self) -> u64 {
        self.0
    }
}

impl Display for CastStreamId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
