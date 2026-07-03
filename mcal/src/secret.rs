//! Refresh tokens in the OS keyring (Secret Service on Linux) — service
//! `"mcal"`, key = the account id. Never written to disk in plaintext.

use crate::error::McalError;

const SERVICE: &str = "mcal";

fn entry(account_id: &str) -> Result<keyring::Entry, McalError> {
    keyring::Entry::new(SERVICE, account_id).map_err(|e| McalError::Keyring(e.to_string()))
}

/// Store (or overwrite) the refresh token for `account_id`.
pub fn store_refresh_token(account_id: &str, token: &str) -> Result<(), McalError> {
    entry(account_id)?
        .set_password(token)
        .map_err(|e| McalError::Keyring(e.to_string()))
}

/// Read the refresh token for `account_id`.
pub fn get_refresh_token(account_id: &str) -> Result<String, McalError> {
    entry(account_id)?
        .get_password()
        .map_err(|e| McalError::Keyring(e.to_string()))
}

/// Delete the refresh token for `account_id` (ignores "not found").
pub fn delete_refresh_token(account_id: &str) -> Result<(), McalError> {
    match entry(account_id)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(McalError::Keyring(e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "needs a live Secret Service (gnome-keyring/kwallet)"]
    fn round_trips_a_token() {
        let id = "google:test@example.com";
        store_refresh_token(id, "tok-123").unwrap();
        assert_eq!(get_refresh_token(id).unwrap(), "tok-123");
        delete_refresh_token(id).unwrap();
        assert!(get_refresh_token(id).is_err());
    }
}
