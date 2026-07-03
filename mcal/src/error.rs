//! The one error type for mcal-core.
//!
//! The panic-ratchet CI gate forbids new `unwrap`/`expect`/`panic` in this
//! crate — every fallible path returns `Result<_, McalError>` instead.

use thiserror::Error;

/// Anything that can go wrong loading or parsing calendar data.
#[derive(Debug, Error)]
pub enum McalError {
    /// A local calendar path could not be read (missing, not a dir, perms).
    #[error("calendar i/o at {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },

    /// An `.ics` payload did not parse as RFC 5545.
    #[error("ics parse: {0}")]
    Ics(String),

    /// An RRULE / RDATE / EXDATE could not be interpreted.
    #[error("recurrence: {0}")]
    Recurrence(String),

    /// A remote subscription could not be fetched.
    #[error("fetch {url}: {source}")]
    Fetch {
        url: String,
        #[source]
        source: Box<ureq::Error>,
    },

    /// An OAuth step (browser flow, token exchange/refresh) failed.
    #[error("oauth: {0}")]
    Oauth(String),

    /// A keyring read/write failed (no Secret Service, locked, etc.).
    #[error("keyring: {0}")]
    Keyring(String),

    /// A JSON payload (Google API / token endpoint) did not parse.
    #[error("json: {0}")]
    Json(String),

    /// A `credentials.toml` / `accounts.toml` read/parse failed.
    #[error("config: {0}")]
    Config(String),
}
