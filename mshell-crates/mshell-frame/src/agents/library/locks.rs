//! Locked sessions: protected from deletion (the row's delete is off and
//! "Delete empty" skips them). Kept in mshell's own state dir
//! (`$XDG_STATE_HOME/mshell/agents-session-locks.json`), never in a
//! Claude config dir.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Locks {
    /// Session ids (UUIDs).
    locked: BTreeSet<String>,
}

impl Locks {
    pub(crate) fn contains(&self, id: &str) -> bool {
        self.locked.contains(id)
    }

    /// Lock or unlock `id`; only UUIDs are kept.
    pub(crate) fn set(&mut self, id: &str, on: bool) -> bool {
        if !super::meta::is_uuid(id) {
            return false;
        }
        if on {
            self.locked.insert(id.to_string());
        } else {
            self.locked.remove(id);
        }
        true
    }

    pub(crate) fn load_from(path: &Path) -> Locks {
        let mut l: Locks = std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        l.locked.retain(|id| super::meta::is_uuid(id));
        l
    }

    pub(crate) fn save_to(&self, path: &Path) -> std::io::Result<()> {
        let json = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        margo_fs::atomic_write::atomic_write(path, &json)
    }
}

pub(crate) fn path() -> PathBuf {
    margo_fs::xdg::state_home()
        .join("mshell")
        .join("agents-session-locks.json")
}

pub(crate) fn load() -> Locks {
    Locks::load_from(&path())
}

/// Lock or unlock `id` on disk. Blocking.
pub(crate) fn set(id: &str, on: bool) -> Result<(), String> {
    let mut l = load();
    if !l.set(id, on) {
        return Err("Not a session id".into());
    }
    l.save_to(&path())
        .map_err(|e| format!("Couldn't save the lock: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "cae70d77-c84d-45b9-b9ef-3a39caf81fcb";

    #[test]
    fn round_trips_and_drops_junk() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("mshell/locks.json");
        let mut l = Locks::default();
        assert!(l.set(ID, true));
        assert!(!l.set("../x", true));
        l.save_to(&p).unwrap();
        let back = Locks::load_from(&p);
        assert!(back.contains(ID));
        std::fs::write(&p, format!(r#"{{"locked":["{ID}","../../etc"]}}"#)).unwrap();
        let back = Locks::load_from(&p);
        assert!(back.contains(ID) && !back.contains("../../etc"));
        let mut back = back;
        back.set(ID, false);
        assert!(!back.contains(ID));
        assert_eq!(
            Locks::load_from(&d.path().join("missing")),
            Locks::default()
        );
        std::fs::write(&p, "not json").unwrap();
        assert_eq!(Locks::load_from(&p), Locks::default());
    }
}
