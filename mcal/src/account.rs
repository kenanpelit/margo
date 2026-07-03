//! The mcal-owned account registry at `~/.config/mcal/accounts.toml`.
//!
//! mcal is the source of truth for connected accounts (this slice: Google).
//! Refresh tokens live in the keyring ([`crate::secret`]); only non-secret
//! metadata is written here:
//!
//! ```toml
//! [[account]]
//! id = "google:kenan@compecta.com"
//! kind = "google"
//! email = "kenan@compecta.com"
//! display_name = "compecta"
//! ```

use crate::error::McalError;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// One connected account (metadata only — no secrets).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredAccount {
    pub id: String,
    pub kind: String,
    pub email: String,
    pub display_name: String,
}

/// The whole registry.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AccountStore {
    #[serde(default, rename = "account")]
    pub accounts: Vec<StoredAccount>,
}

/// `~/.config/margo/mcal/accounts.toml`.
pub fn accounts_path() -> PathBuf {
    crate::config::config_dir().join("accounts.toml")
}

impl AccountStore {
    /// The stable id for a Google account.
    pub fn google_id(email: &str) -> String {
        format!("google:{email}")
    }

    /// Load from disk (empty store if the file is absent).
    pub fn load() -> Result<Self, McalError> {
        let path = accounts_path();
        match std::fs::read_to_string(&path) {
            Ok(text) => Self::from_toml(&text),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(McalError::Io {
                path: path.display().to_string(),
                source: err,
            }),
        }
    }

    /// Persist to disk, creating the parent dir if needed.
    pub fn save(&self) -> Result<(), McalError> {
        let path = accounts_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| McalError::Io {
                path: parent.display().to_string(),
                source,
            })?;
        }
        std::fs::write(&path, self.to_toml()?).map_err(|source| McalError::Io {
            path: path.display().to_string(),
            source,
        })
    }

    /// Add or replace the account with the same id.
    pub fn add(&mut self, account: StoredAccount) {
        self.accounts.retain(|a| a.id != account.id);
        self.accounts.push(account);
    }

    /// Remove by id; returns whether an account was removed.
    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.accounts.len();
        self.accounts.retain(|a| a.id != id);
        self.accounts.len() != before
    }

    fn from_toml(text: &str) -> Result<Self, McalError> {
        toml::from_str(text).map_err(|e| McalError::Config(e.to_string()))
    }

    fn to_toml(&self) -> Result<String, McalError> {
        toml::to_string_pretty(self).map_err(|e| McalError::Config(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_toml() {
        let mut store = AccountStore::default();
        store.add(StoredAccount {
            id: AccountStore::google_id("kenan@compecta.com"),
            kind: "google".into(),
            email: "kenan@compecta.com".into(),
            display_name: "compecta".into(),
        });
        let text = store.to_toml().unwrap();
        let back = AccountStore::from_toml(&text).unwrap();
        assert_eq!(back.accounts.len(), 1);
        assert_eq!(back.accounts[0].id, "google:kenan@compecta.com");
        assert_eq!(back.accounts[0].kind, "google");
    }

    #[test]
    fn remove_returns_whether_it_existed() {
        let mut store = AccountStore::default();
        store.add(StoredAccount {
            id: "google:a@b.com".into(),
            kind: "google".into(),
            email: "a@b.com".into(),
            display_name: "a".into(),
        });
        assert!(store.remove("google:a@b.com"));
        assert!(!store.remove("google:a@b.com"));
        assert!(store.accounts.is_empty());
    }
}
