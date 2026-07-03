# mcal Google Account (OAuth) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Connect a Google account to `mcal` via OAuth so its events appear in the existing `mcal today/agenda/on` CLI and the slice-1 shell surfaces (clock-menu agenda, dashboard calendar), read-only.

**Architecture:** A new `GoogleProvider` implements the existing `Provider` trait against the Google Calendar API v3. `mcal account setup google` runs a loopback+PKCE OAuth flow, stores the refresh token in the OS keyring, and records the account in an mcal-owned `~/.config/mcal/accounts.toml`. `load_all` gains a third source that builds providers from that store, so both the CLI and the shell pick Google up with no extra wiring.

**Tech Stack:** Rust, `ureq` 2.x (blocking HTTP), `serde`/`serde_json` (Calendar API JSON), `toml` (config/account files), `keyring` 3.x (refresh tokens), `sha2` + `base64` (PKCE), `chrono` (dates). Google Calendar API v3 + Google OAuth 2.0.

## Global Constraints

- **Spec:** `docs/superpowers/specs/2026-07-03-mcal-google-account-design.md`. Every task's requirements implicitly include it.
- **Read-only** this slice: scope `https://www.googleapis.com/auth/calendar.readonly`. No event writes.
- **BYO client_id:** credentials from `~/.config/mcal/credentials.toml`; never a shipped/embedded secret.
- **mcal owns accounts** in `~/.config/mcal/accounts.toml`; refresh tokens in the keyring (service `"mcal"`, key = account id). Never write a refresh token to disk in plaintext.
- **Recurrence:** request `singleEvents=true` from Google (server-side expansion); Google events carry empty `recurrence` and skip `recur::expand`.
- **Panic-ratchet:** baseline 370. No new `unwrap()`/`expect()`/`panic!`/`unreachable!` outside `#[cfg(test)]`. Every fallible path returns `Result<_, McalError>`.
- **CLI strings:** English, matching slice 1 and the rest of margo's binaries.
- **Gates before pushing:** `cargo +1.95.0 fmt --all -- --check`, `cargo clippy -p mcal --all-targets -- -D warnings`, `cargo test -p mcal`, `scripts/panic-ratchet.sh`.
- **ureq 2.x API:** `ureq::post(url).send_form(&[(k,v)…])?.into_json::<T>()?`; `ureq::get(url).set("Authorization", …).call()?.into_json::<T>()?`; errors are `ureq::Error::{Status(code, resp), Transport(_)}`.
- **keyring 3.x API:** `keyring::Entry::new(service, user)?` then `.set_password(&str)` / `.get_password()` / `.delete_credential()`.
- **base64 0.21 API:** `use base64::Engine;` + `base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)`.

---

## File Structure

- `mcal/src/error.rs` (modify) — add `Oauth`/`Keyring`/`Json`/`Config` variants.
- `mcal/src/credentials.rs` (create) — `GoogleCredentials`, load `credentials.toml`, setup help text.
- `mcal/src/account.rs` (create) — `StoredAccount`, `AccountStore` (load/save/add/remove/list).
- `mcal/src/secret.rs` (create) — keyring wrapper for refresh tokens.
- `mcal/src/oauth.rs` (create) — PKCE, auth-URL, loopback listener, token exchange/refresh, interactive login.
- `mcal/src/provider/google.rs` (create) — `GoogleProvider` (Calendar API v3, JSON→`Event`).
- `mcal/src/provider/mod.rs` (modify) — `load_all` merges account-store (Google) providers.
- `mcal/src/lib.rs` (modify) — module decls + exports.
- `mcal/src/main.rs` (modify) — `account setup google|list|remove` subcommands.
- `mcal/Cargo.toml` (modify) + root `Cargo.toml` (modify) — dependencies.

---

## Task 1: Dependencies + error variants

**Files:**
- Modify: `Cargo.toml` (`[workspace.dependencies]`)
- Modify: `mcal/Cargo.toml`
- Modify: `mcal/src/error.rs:34`

**Interfaces:**
- Produces: `McalError::Oauth(String)`, `McalError::Keyring(String)`, `McalError::Json(String)`, `McalError::Config(String)`.

- [ ] **Step 1: Add workspace deps** (root `Cargo.toml`, `[workspace.dependencies]` — `serde_json`/`toml` already exist; add the two missing):

```toml
sha2 = "0.10"
base64 = "0.21"
```

- [ ] **Step 2: Add mcal deps** (`mcal/Cargo.toml`, under `[dependencies]`):

```toml
keyring.workspace = true
serde_json.workspace = true
toml.workspace = true
sha2.workspace = true
base64.workspace = true
```

- [ ] **Step 3: Add error variants** (`mcal/src/error.rs`, before the closing `}` of the enum):

```rust
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
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo build -p mcal`
Expected: builds (new deps resolve, enum compiles). Warnings about unused variants are fine for now.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock mcal/Cargo.toml mcal/src/error.rs
git commit -m "feat(mcal): add oauth/keyring deps + error variants"
```

---

## Task 2: Credentials module

**Files:**
- Create: `mcal/src/credentials.rs`
- Modify: `mcal/src/lib.rs` (add `mod credentials;` + exports)

**Interfaces:**
- Produces:
  - `pub struct GoogleCredentials { pub client_id: String, pub client_secret: String }`
  - `pub fn credentials_path() -> std::path::PathBuf`
  - `pub fn load_google() -> Result<Option<GoogleCredentials>, McalError>`
  - `pub fn setup_instructions() -> String`

- [ ] **Step 1: Write the failing test** (`mcal/src/credentials.rs`, bottom):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_google_credentials_from_toml() {
        let text = "\
[google]
client_id = \"abc.apps.googleusercontent.com\"
client_secret = \"s3cr3t\"
";
        let creds = parse_google(text).unwrap().unwrap();
        assert_eq!(creds.client_id, "abc.apps.googleusercontent.com");
        assert_eq!(creds.client_secret, "s3cr3t");
    }

    #[test]
    fn missing_google_table_is_none() {
        assert!(parse_google("[other]\nx = 1\n").unwrap().is_none());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p mcal credentials`
Expected: FAIL — `parse_google` / `GoogleCredentials` not found.

- [ ] **Step 3: Write the implementation** (`mcal/src/credentials.rs`, top):

```rust
//! BYO Google OAuth credentials, read from `~/.config/mcal/credentials.toml`:
//!
//! ```toml
//! [google]
//! client_id = "…apps.googleusercontent.com"
//! client_secret = "…"
//! ```
//!
//! The installed-app `client_secret` is not confidential per Google's own
//! model (PKCE + loopback protect the flow); it lives in a plain file, the
//! refresh token does not (see [`crate::secret`]).

use crate::error::McalError;
use serde::Deserialize;
use std::path::PathBuf;

/// A Google OAuth client the user created in their own Google Cloud project.
#[derive(Debug, Clone, Deserialize)]
pub struct GoogleCredentials {
    pub client_id: String,
    pub client_secret: String,
}

#[derive(Debug, Deserialize)]
struct CredentialsFile {
    google: Option<GoogleCredentials>,
}

/// `~/.config/mcal/credentials.toml` (honours `$XDG_CONFIG_HOME`).
pub fn credentials_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("mcal")
        .join("credentials.toml")
}

/// Parse the `[google]` table out of a credentials-file string.
fn parse_google(text: &str) -> Result<Option<GoogleCredentials>, McalError> {
    let file: CredentialsFile =
        toml::from_str(text).map_err(|e| McalError::Config(e.to_string()))?;
    Ok(file.google)
}

/// Load the Google credentials, or `None` if the file/table is absent.
pub fn load_google() -> Result<Option<GoogleCredentials>, McalError> {
    let path = credentials_path();
    match std::fs::read_to_string(&path) {
        Ok(text) => parse_google(&text),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(McalError::Io {
            path: path.display().to_string(),
            source: err,
        }),
    }
}

/// The step-by-step message shown when credentials are missing.
pub fn setup_instructions() -> String {
    format!(
        "No Google credentials found at {}.\n\
         Create your own OAuth client (one-time):\n\
         1. https://console.cloud.google.com → create a project.\n\
         2. APIs & Services → Library → enable \"Google Calendar API\".\n\
         3. OAuth consent screen → User type \"External\" → add your email as a Test user.\n\
         4. Credentials → Create credentials → OAuth client ID → type \"Desktop app\".\n\
         5. Copy the client ID + secret into that file:\n\
         \n\
         [google]\n\
         client_id = \"…apps.googleusercontent.com\"\n\
         client_secret = \"…\"\n",
        credentials_path().display()
    )
}
```

- [ ] **Step 4: Wire the module** (`mcal/src/lib.rs`): add `mod credentials;` with the others and `pub use credentials::{GoogleCredentials, credentials_path, load_google, setup_instructions};`

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p mcal credentials`
Expected: PASS (both tests).

- [ ] **Step 6: Commit**

```bash
git add mcal/src/credentials.rs mcal/src/lib.rs
git commit -m "feat(mcal): read BYO Google credentials from credentials.toml"
```

---

## Task 3: Account store

**Files:**
- Create: `mcal/src/account.rs`
- Modify: `mcal/src/lib.rs`

**Interfaces:**
- Produces:
  - `pub struct StoredAccount { pub id: String, pub kind: String, pub email: String, pub display_name: String }`
  - `pub fn accounts_path() -> PathBuf`
  - `pub struct AccountStore { pub accounts: Vec<StoredAccount> }` with `load()`, `save(&self)`, `add(&mut self, StoredAccount)`, `remove(&mut self, id) -> bool`, `google_id(email) -> String`.

- [ ] **Step 1: Write the failing test** (`mcal/src/account.rs`, bottom):

```rust
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
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p mcal account`
Expected: FAIL — `AccountStore` not found.

- [ ] **Step 3: Write the implementation** (`mcal/src/account.rs`, top):

```rust
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

/// `~/.config/mcal/accounts.toml`.
pub fn accounts_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("mcal")
        .join("accounts.toml")
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
```

- [ ] **Step 4: Wire the module** (`mcal/src/lib.rs`): `mod account;` + `pub use account::{AccountStore, StoredAccount, accounts_path};`

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p mcal account`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add mcal/src/account.rs mcal/src/lib.rs
git commit -m "feat(mcal): mcal-owned account store (accounts.toml)"
```

---

## Task 4: Keyring wrapper

**Files:**
- Create: `mcal/src/secret.rs`
- Modify: `mcal/src/lib.rs`

**Interfaces:**
- Produces:
  - `pub fn store_refresh_token(account_id: &str, token: &str) -> Result<(), McalError>`
  - `pub fn get_refresh_token(account_id: &str) -> Result<String, McalError>`
  - `pub fn delete_refresh_token(account_id: &str) -> Result<(), McalError>`

- [ ] **Step 1: Write an ignored integration test** (keyring needs a live Secret Service, so it is `#[ignore]` in CI; it documents the round-trip):

```rust
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
```

- [ ] **Step 2: Write the implementation** (`mcal/src/secret.rs`, top):

```rust
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
```

- [ ] **Step 3: Wire the module** (`mcal/src/lib.rs`): `mod secret;` + `pub use secret::{delete_refresh_token, get_refresh_token, store_refresh_token};`

- [ ] **Step 4: Verify build + non-ignored tests**

Run: `cargo test -p mcal secret`
Expected: PASS (0 run, 1 ignored) — compiles, ignored test skipped.

- [ ] **Step 5: Commit**

```bash
git add mcal/src/secret.rs mcal/src/lib.rs
git commit -m "feat(mcal): keyring wrapper for refresh tokens"
```

---

## Task 5: OAuth — PKCE + auth URL (pure)

**Files:**
- Create: `mcal/src/oauth.rs`
- Modify: `mcal/src/lib.rs`

**Interfaces:**
- Produces:
  - `pub const SCOPE: &str` (calendar.readonly)
  - `pub struct PkcePair { pub verifier: String, pub challenge: String }`
  - `pub fn pkce_pair() -> Result<PkcePair, McalError>`
  - `pub fn code_challenge(verifier: &str) -> String`
  - `pub fn random_token(bytes: usize) -> Result<String, McalError>`
  - `pub struct AuthRequest<'a> { pub client_id: &'a str, pub redirect_uri: &'a str, pub scope: &'a str, pub challenge: &'a str, pub state: &'a str }`
  - `pub fn auth_url(req: &AuthRequest) -> String`

- [ ] **Step 1: Write the failing test** (uses the RFC 7636 Appendix-B PKCE test vector):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_challenge_matches_rfc7636_vector() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(code_challenge(verifier), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn auth_url_has_required_params() {
        let url = auth_url(&AuthRequest {
            client_id: "cid",
            redirect_uri: "http://127.0.0.1:5555",
            scope: SCOPE,
            challenge: "chal",
            state: "st8",
        });
        assert!(url.starts_with("https://accounts.google.com/o/oauth2/v2/auth?"));
        assert!(url.contains("client_id=cid"));
        assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A5555"));
        assert!(url.contains("code_challenge=chal"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("access_type=offline"));
        assert!(url.contains("prompt=consent"));
        assert!(url.contains("state=st8"));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p mcal oauth`
Expected: FAIL — `code_challenge` / `auth_url` not found.

- [ ] **Step 3: Write the implementation** (`mcal/src/oauth.rs`, top):

```rust
//! Google OAuth 2.0 for installed apps: loopback redirect + PKCE.
//!
//! `mcal account setup google` opens the browser, catches the redirect on a
//! throwaway `127.0.0.1` port, exchanges the code for tokens, and hands back a
//! refresh token (stored in the keyring by the caller).

use crate::credentials::GoogleCredentials;
use crate::error::McalError;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};
use std::io::Read;

/// The minimum read-only calendar scope.
pub const SCOPE: &str = "https://www.googleapis.com/auth/calendar.readonly";

const AUTH_ENDPOINT: &str = "https://accounts.google.com/o/oauth2/v2/auth";

/// A PKCE verifier + its S256 challenge.
pub struct PkcePair {
    pub verifier: String,
    pub challenge: String,
}

/// `base64url(sha256(verifier))` — the S256 code challenge.
pub fn code_challenge(verifier: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(hasher.finalize())
}

/// A fresh PKCE pair (32 random bytes → base64url verifier).
pub fn pkce_pair() -> Result<PkcePair, McalError> {
    let verifier = random_token(32)?;
    let challenge = code_challenge(&verifier);
    Ok(PkcePair { verifier, challenge })
}

/// `n` random bytes from `/dev/urandom`, base64url-encoded.
pub fn random_token(bytes: usize) -> Result<String, McalError> {
    let mut file = std::fs::File::open("/dev/urandom")
        .map_err(|e| McalError::Oauth(format!("urandom: {e}")))?;
    let mut buf = vec![0u8; bytes];
    file.read_exact(&mut buf)
        .map_err(|e| McalError::Oauth(format!("urandom: {e}")))?;
    Ok(URL_SAFE_NO_PAD.encode(buf))
}

/// Percent-encode a query-parameter value (RFC 3986 unreserved kept).
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Everything needed to build the authorization URL.
pub struct AuthRequest<'a> {
    pub client_id: &'a str,
    pub redirect_uri: &'a str,
    pub scope: &'a str,
    pub challenge: &'a str,
    pub state: &'a str,
}

/// Build the Google authorization URL.
pub fn auth_url(req: &AuthRequest) -> String {
    let params = [
        ("client_id", req.client_id),
        ("redirect_uri", req.redirect_uri),
        ("response_type", "code"),
        ("scope", req.scope),
        ("access_type", "offline"),
        ("prompt", "consent"),
        ("code_challenge", req.challenge),
        ("code_challenge_method", "S256"),
        ("state", req.state),
    ];
    let query = params
        .iter()
        .map(|(k, v)| format!("{k}={}", percent_encode(v)))
        .collect::<Vec<_>>()
        .join("&");
    format!("{AUTH_ENDPOINT}?{query}")
}
```

- [ ] **Step 4: Wire the module** (`mcal/src/lib.rs`): `mod oauth;` (exports added in Task 7).

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p mcal oauth`
Expected: PASS (both tests).

- [ ] **Step 6: Commit**

```bash
git add mcal/src/oauth.rs mcal/src/lib.rs
git commit -m "feat(mcal): OAuth PKCE + auth-URL builder"
```

---

## Task 6: OAuth — token exchange + refresh (network)

**Files:**
- Modify: `mcal/src/oauth.rs`

**Interfaces:**
- Consumes: `GoogleCredentials`, `McalError`.
- Produces:
  - `pub struct TokenResponse { pub access_token: String, pub refresh_token: Option<String>, pub expires_in: Option<i64> }` (derives `Deserialize`)
  - `pub fn exchange_code(creds: &GoogleCredentials, code: &str, verifier: &str, redirect_uri: &str) -> Result<TokenResponse, McalError>`
  - `pub fn refresh_access_token(creds: &GoogleCredentials, refresh_token: &str) -> Result<TokenResponse, McalError>`

- [ ] **Step 1: Write the failing test** (parse-only; no network):

```rust
    #[test]
    fn parses_token_response_json() {
        let json = r#"{"access_token":"at1","expires_in":3599,"refresh_token":"rt1","scope":"x","token_type":"Bearer"}"#;
        let tr: TokenResponse = serde_json::from_str(json).unwrap();
        assert_eq!(tr.access_token, "at1");
        assert_eq!(tr.refresh_token.as_deref(), Some("rt1"));
        assert_eq!(tr.expires_in, Some(3599));
    }

    #[test]
    fn token_response_without_refresh_is_ok() {
        let json = r#"{"access_token":"at2","expires_in":10,"token_type":"Bearer"}"#;
        let tr: TokenResponse = serde_json::from_str(json).unwrap();
        assert!(tr.refresh_token.is_none());
    }
```

Add `use serde::Deserialize;` to the test module (or rely on the top-level import added in Step 3).

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p mcal oauth::tests::parses_token_response_json`
Expected: FAIL — `TokenResponse` not found.

- [ ] **Step 3: Write the implementation** (`mcal/src/oauth.rs`, append; add `use serde::Deserialize;` to the file's imports):

```rust
const TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";

/// The subset of Google's token response we use.
#[derive(Debug, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_in: Option<i64>,
}

/// Turn a ureq error into a readable string, reading the error body if present.
fn http_err(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(code, resp) => {
            let body = resp.into_string().unwrap_or_default();
            format!("HTTP {code}: {body}")
        }
        ureq::Error::Transport(t) => t.to_string(),
    }
}

/// Exchange an authorization `code` for tokens.
pub fn exchange_code(
    creds: &GoogleCredentials,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<TokenResponse, McalError> {
    ureq::post(TOKEN_ENDPOINT)
        .send_form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("code_verifier", verifier),
            ("client_id", &creds.client_id),
            ("client_secret", &creds.client_secret),
            ("redirect_uri", redirect_uri),
        ])
        .map_err(|e| McalError::Oauth(http_err(e)))?
        .into_json()
        .map_err(|e| McalError::Json(e.to_string()))
}

/// Trade a refresh token for a fresh access token.
pub fn refresh_access_token(
    creds: &GoogleCredentials,
    refresh_token: &str,
) -> Result<TokenResponse, McalError> {
    ureq::post(TOKEN_ENDPOINT)
        .send_form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", &creds.client_id),
            ("client_secret", &creds.client_secret),
        ])
        .map_err(|e| McalError::Oauth(http_err(e)))?
        .into_json()
        .map_err(|e| McalError::Json(e.to_string()))
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p mcal oauth`
Expected: PASS (all oauth tests).

- [ ] **Step 5: Commit**

```bash
git add mcal/src/oauth.rs
git commit -m "feat(mcal): OAuth token exchange + refresh"
```

---

## Task 7: OAuth — loopback + interactive login

**Files:**
- Modify: `mcal/src/oauth.rs`
- Modify: `mcal/src/lib.rs` (exports)

**Interfaces:**
- Produces:
  - `pub struct GoogleTokens { pub refresh_token: String, pub access_token: String }`
  - `pub fn interactive_google_login(creds: &GoogleCredentials) -> Result<GoogleTokens, McalError>`
  - (internal, tested) `fn parse_redirect_query(request_line: &str) -> Vec<(String, String)>`

- [ ] **Step 1: Write the failing test:**

```rust
    #[test]
    fn parses_redirect_query() {
        let line = "GET /?code=4%2F0Ab&state=xyz&scope=cal HTTP/1.1";
        let params = parse_redirect_query(line);
        let get = |k: &str| params.iter().find(|(kk, _)| kk == k).map(|(_, v)| v.clone());
        assert_eq!(get("code").as_deref(), Some("4/0Ab")); // %2F decoded to '/'
        assert_eq!(get("state").as_deref(), Some("xyz"));
    }

    #[test]
    fn parses_error_redirect() {
        let params = parse_redirect_query("GET /?error=access_denied&state=x HTTP/1.1");
        assert!(params.iter().any(|(k, v)| k == "error" && v == "access_denied"));
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p mcal oauth::tests::parses_redirect_query`
Expected: FAIL — `parse_redirect_query` not found.

- [ ] **Step 3: Write the implementation** (`mcal/src/oauth.rs`, append; add `use std::io::Write;` and `use std::net::TcpListener;` to imports):

```rust
/// A refresh + access token pair from a completed login.
pub struct GoogleTokens {
    pub refresh_token: String,
    pub access_token: String,
}

/// Percent-decode a query value (`%2F` → `/`, `+` → space).
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(if bytes[i] == b'+' { b' ' } else { bytes[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Pull the query params out of an HTTP request line
/// (`GET /?code=…&state=… HTTP/1.1`).
fn parse_redirect_query(request_line: &str) -> Vec<(String, String)> {
    let Some(path) = request_line.split_whitespace().nth(1) else {
        return Vec::new();
    };
    let query = path.split_once('?').map(|(_, q)| q).unwrap_or("");
    query
        .split('&')
        .filter(|s| !s.is_empty())
        .filter_map(|kv| {
            let (k, v) = kv.split_once('=')?;
            Some((k.to_string(), percent_decode(v)))
        })
        .collect()
}

/// Accept one redirect, answer the browser, and return the `code`.
fn run_loopback_once(listener: &TcpListener, expected_state: &str) -> Result<String, McalError> {
    let (mut stream, _) = listener
        .accept()
        .map_err(|e| McalError::Oauth(format!("accept: {e}")))?;

    let mut reader = std::io::BufReader::new(
        stream
            .try_clone()
            .map_err(|e| McalError::Oauth(e.to_string()))?,
    );
    let mut line = String::new();
    std::io::BufRead::read_line(&mut reader, &mut line)
        .map_err(|e| McalError::Oauth(e.to_string()))?;

    let params = parse_redirect_query(line.trim_end());
    let get = |k: &str| {
        params
            .iter()
            .find(|(kk, _)| kk == k)
            .map(|(_, v)| v.clone())
    };

    let body = "<html><body>mcal: signed in. You can close this tab.</body></html>";
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(response.as_bytes());

    if let Some(err) = get("error") {
        return Err(McalError::Oauth(format!("google denied: {err}")));
    }
    if get("state").as_deref() != Some(expected_state) {
        return Err(McalError::Oauth("state mismatch (possible CSRF)".into()));
    }
    get("code").ok_or_else(|| McalError::Oauth("redirect had no code".into()))
}

/// Run the full browser login and return tokens.
pub fn interactive_google_login(creds: &GoogleCredentials) -> Result<GoogleTokens, McalError> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|e| McalError::Oauth(format!("bind loopback: {e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| McalError::Oauth(e.to_string()))?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}");

    let pkce = pkce_pair()?;
    let state = random_token(16)?;
    let url = auth_url(&AuthRequest {
        client_id: &creds.client_id,
        redirect_uri: &redirect_uri,
        scope: SCOPE,
        challenge: &pkce.challenge,
        state: &state,
    });

    println!("Opening your browser to authorize mcal…");
    println!("If it doesn't open, visit:\n{url}");
    let _ = std::process::Command::new("xdg-open").arg(&url).spawn();

    let code = run_loopback_once(&listener, &state)?;
    let tokens = exchange_code(creds, &code, &pkce.verifier, &redirect_uri)?;
    let refresh_token = tokens.refresh_token.ok_or_else(|| {
        McalError::Oauth(
            "Google returned no refresh_token — remove mcal from your account's \
             third-party access and retry."
                .into(),
        )
    })?;
    Ok(GoogleTokens {
        refresh_token,
        access_token: tokens.access_token,
    })
}
```

- [ ] **Step 4: Wire exports** (`mcal/src/lib.rs`): `pub use oauth::{GoogleTokens, interactive_google_login, refresh_access_token};`

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p mcal oauth`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add mcal/src/oauth.rs mcal/src/lib.rs
git commit -m "feat(mcal): OAuth loopback listener + interactive login"
```

---

## Task 8: GoogleProvider

**Files:**
- Create: `mcal/src/provider/google.rs`
- Modify: `mcal/src/provider/mod.rs` (add `mod google;` + `pub use google::GoogleProvider;`)

**Interfaces:**
- Consumes: `Provider`, `Window`, `Calendar`, `Event`, `GoogleCredentials`, `crate::secret`, `crate::oauth::refresh_access_token`.
- Produces: `pub struct GoogleProvider` with `pub fn new(account_id: impl Into<String>, credentials: GoogleCredentials) -> Self`, and its `Provider` impl. Internal `fn map_event(raw: &GoogleEvent, calendar_id: &str) -> Option<Event>`.

- [ ] **Step 1: Write the failing test** (pure JSON→Event mapping):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn parse(json: &str) -> GoogleEvent {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn maps_a_timed_event() {
        let raw = parse(
            r#"{"id":"e1","summary":"Standup","location":"Meet",
                "start":{"dateTime":"2026-07-03T09:00:00+03:00"},
                "end":{"dateTime":"2026-07-03T09:30:00+03:00"}}"#,
        );
        let ev = map_event(&raw, "google:primary").unwrap();
        assert_eq!(ev.summary, "Standup");
        assert_eq!(ev.location.as_deref(), Some("Meet"));
        assert!(!ev.all_day);
        assert_eq!(ev.start.to_rfc3339(), "2026-07-03T06:00:00+00:00");
        assert_eq!(ev.calendar_id, "google:primary");
    }

    #[test]
    fn maps_an_all_day_event() {
        let raw = parse(
            r#"{"id":"e2","summary":"Holiday",
                "start":{"date":"2026-07-04"},"end":{"date":"2026-07-05"}}"#,
        );
        let ev = map_event(&raw, "google:primary").unwrap();
        assert!(ev.all_day);
        assert_eq!(ev.start.to_rfc3339(), "2026-07-04T00:00:00+00:00");
    }

    #[test]
    fn skips_cancelled_events() {
        let raw = parse(r#"{"id":"e3","status":"cancelled","start":{"date":"2026-07-04"}}"#);
        assert!(map_event(&raw, "google:primary").is_none());
    }

    #[test]
    fn skips_events_without_a_start() {
        let raw = parse(r#"{"id":"e4","summary":"x"}"#);
        assert!(map_event(&raw, "google:primary").is_none());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p mcal google`
Expected: FAIL — `GoogleEvent` / `map_event` not found.

- [ ] **Step 3: Write the implementation** (`mcal/src/provider/google.rs`, top):

```rust
//! Google Calendar API v3 provider (read-only).
//!
//! `singleEvents=true` makes Google expand recurrence server-side, so mapped
//! events carry no RRULE and skip [`crate::recur`]. Token handling is per-fetch:
//! read the refresh token from the keyring, mint an access token, then page
//! through each of the account's calendars.

use super::{Provider, Window};
use crate::credentials::GoogleCredentials;
use crate::error::McalError;
use crate::model::{Calendar, Event};
use chrono::{DateTime, NaiveDate, NaiveTime, TimeZone, Utc};
use serde::Deserialize;

const API: &str = "https://www.googleapis.com/calendar/v3";

/// A Google account as an mcal calendar source.
pub struct GoogleProvider {
    account_id: String,
    credentials: GoogleCredentials,
}

#[derive(Debug, Deserialize)]
struct CalendarListResponse {
    #[serde(default)]
    items: Vec<GoogleCalendar>,
}

#[derive(Debug, Deserialize)]
struct GoogleCalendar {
    id: String,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default, rename = "backgroundColor")]
    background_color: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EventsResponse {
    #[serde(default)]
    items: Vec<GoogleEvent>,
    #[serde(default, rename = "nextPageToken")]
    next_page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoogleEvent {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    location: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default, rename = "htmlLink")]
    html_link: Option<String>,
    #[serde(default)]
    start: Option<GoogleDate>,
    #[serde(default)]
    end: Option<GoogleDate>,
}

#[derive(Debug, Deserialize)]
struct GoogleDate {
    #[serde(default)]
    date: Option<String>,
    #[serde(default, rename = "dateTime")]
    date_time: Option<String>,
}

/// Resolve a Google date/time to UTC; `true` if it was an all-day `date`.
fn resolve(d: &GoogleDate) -> Option<(DateTime<Utc>, bool)> {
    if let Some(dt) = &d.date_time {
        let parsed = DateTime::parse_from_rfc3339(dt).ok()?;
        Some((parsed.with_timezone(&Utc), false))
    } else if let Some(date) = &d.date {
        let day = NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()?;
        Some((Utc.from_utc_datetime(&day.and_time(NaiveTime::MIN)), true))
    } else {
        None
    }
}

/// Map one Google event to an mcal [`Event`], or `None` to skip it.
fn map_event(raw: &GoogleEvent, calendar_id: &str) -> Option<Event> {
    if raw.status.as_deref() == Some("cancelled") {
        return None;
    }
    let (start, all_day) = resolve(raw.start.as_ref()?)?;
    let end = raw
        .end
        .as_ref()
        .and_then(resolve)
        .map(|(dt, _)| dt)
        .unwrap_or(start);
    let id = raw.id.clone().unwrap_or_default();
    Some(Event {
        id: format!("{calendar_id}:{id}"),
        calendar_id: calendar_id.to_string(),
        uid: id,
        summary: raw.summary.clone().unwrap_or_else(|| "(no title)".into()),
        description: raw.description.clone(),
        location: raw.location.clone(),
        url: raw.html_link.clone(),
        status: raw.status.clone(),
        start,
        end,
        all_day,
        recurrence: Vec::new(),
        attendees: Vec::new(),
        categories: Vec::new(),
    })
}

impl GoogleProvider {
    pub fn new(account_id: impl Into<String>, credentials: GoogleCredentials) -> Self {
        Self {
            account_id: account_id.into(),
            credentials,
        }
    }

    /// A fresh access token from the stored refresh token.
    fn access_token(&self) -> Result<String, McalError> {
        let refresh = crate::secret::get_refresh_token(&self.account_id)?;
        let tokens = crate::oauth::refresh_access_token(&self.credentials, &refresh)?;
        Ok(tokens.access_token)
    }

    fn calendar_ids(&self, token: &str) -> Result<Vec<GoogleCalendar>, McalError> {
        let url = format!("{API}/users/me/calendarList");
        let resp: CalendarListResponse = ureq::get(&url)
            .set("Authorization", &format!("Bearer {token}"))
            .call()
            .map_err(|e| McalError::Fetch {
                url: url.clone(),
                source: Box::new(e),
            })?
            .into_json()
            .map_err(|e| McalError::Json(e.to_string()))?;
        Ok(resp.items)
    }

    fn events_for(
        &self,
        token: &str,
        calendar_id: &str,
        window: Window,
    ) -> Result<Vec<Event>, McalError> {
        let mapped_id = format!("google:{calendar_id}");
        let mut out = Vec::new();
        let mut page: Option<String> = None;
        loop {
            let mut req = ureq::get(&format!(
                "{API}/calendars/{}/events",
                urlencode(calendar_id)
            ))
            .set("Authorization", &format!("Bearer {token}"))
            .query("singleEvents", "true")
            .query("orderBy", "startTime")
            .query("maxResults", "2500")
            .query("timeMin", &window.0.to_rfc3339())
            .query("timeMax", &window.1.to_rfc3339());
            if let Some(tok) = &page {
                req = req.query("pageToken", tok);
            }
            let resp: EventsResponse = req
                .call()
                .map_err(|e| McalError::Fetch {
                    url: format!("{API}/calendars/{calendar_id}/events"),
                    source: Box::new(e),
                })?
                .into_json()
                .map_err(|e| McalError::Json(e.to_string()))?;
            for raw in &resp.items {
                if let Some(ev) = map_event(raw, &mapped_id) {
                    out.push(ev);
                }
            }
            match resp.next_page_token {
                Some(tok) => page = Some(tok),
                None => break,
            }
        }
        Ok(out)
    }
}

/// Percent-encode a calendar id for use in a path segment.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

impl Provider for GoogleProvider {
    fn calendars(&self) -> Result<Vec<Calendar>, McalError> {
        let token = self.access_token()?;
        Ok(self
            .calendar_ids(&token)?
            .into_iter()
            .map(|c| Calendar {
                account_id: self.account_id.clone(),
                remote_id: format!("google:{}", c.id),
                name: c.summary.unwrap_or_else(|| c.id.clone()),
                color: c.background_color,
            })
            .collect())
    }

    fn events(&self, window: Window) -> Result<Vec<Event>, McalError> {
        let token = self.access_token()?;
        let mut out = Vec::new();
        for cal in self.calendar_ids(&token)? {
            out.extend(self.events_for(&token, &cal.id, window)?);
        }
        Ok(out)
    }
}
```

- [ ] **Step 4: Wire the submodule** (`mcal/src/provider/mod.rs`): add `mod google;` and `pub use google::GoogleProvider;` near the other `pub use`s.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p mcal google`
Expected: PASS (all four mapping tests).

- [ ] **Step 6: Commit**

```bash
git add mcal/src/provider/google.rs mcal/src/provider/mod.rs
git commit -m "feat(mcal): GoogleProvider over Calendar API v3"
```

---

## Task 9: Merge Google accounts into `load_all`

**Files:**
- Modify: `mcal/src/provider/mod.rs:35-49`

**Interfaces:**
- Consumes: `AccountStore`, `load_google`, `GoogleProvider`.
- Produces: `load_all` unchanged signature; now also loads account-store (Google) providers.

- [ ] **Step 1: Write the failing test** (`mcal/src/provider/mod.rs`, bottom — a regression guard that local still loads and an empty store adds nothing; full Google needs network and is covered manually):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    // Regression guard: the account-store path must not panic when
    // `~/.config/mcal/accounts.toml` is absent (it returns an empty vec),
    // and local events still load. Full Google needs network → manual verify.
    #[test]
    fn load_all_still_returns_local_events() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("a.ics"),
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:u@x\r\nSUMMARY:S\r\nDTSTART:20260703T090000Z\r\nDTEND:20260703T093000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
        )
        .unwrap();
        let config = crate::config::CalendarConfig {
            local_dir: tmp.path().to_path_buf(),
            subscriptions: Vec::new(),
            refresh_secs: 0,
        };
        let window = (
            Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2026, 12, 31, 0, 0, 0).unwrap(),
        );
        let events = load_all(&config, window);
        assert_eq!(events.len(), 1);
    }
}
```

- [ ] **Step 2: Run test to verify it fails/passes**

Run: `cargo test -p mcal provider::tests::load_all_still_returns_local_events`
Expected: initially PASS (local path unchanged) — this is a regression guard so it stays green through the edit.

- [ ] **Step 3: Add the account-store merge** (`mcal/src/provider/mod.rs`): extend imports and `load_all`:

```rust
use crate::account::AccountStore;
use crate::credentials::load_google;
```

Insert into `load_all`, just before `events` is returned:

```rust
    load_account_providers(window, &mut events);
```

Add the helper below `load_all`:

```rust
/// Build providers from the mcal account store (Google this slice) and collect
/// their events. A missing store, missing credentials, or a dead token is
/// logged and skipped — never a hard failure.
fn load_account_providers(window: Window, out: &mut Vec<Event>) {
    let store = match AccountStore::load() {
        Ok(store) => store,
        Err(err) => {
            tracing::warn!(%err, "mcal: account store unreadable");
            return;
        }
    };
    if store.accounts.iter().all(|a| a.kind != "google") {
        return;
    }
    let credentials = match load_google() {
        Ok(Some(creds)) => creds,
        Ok(None) => {
            tracing::warn!("mcal: google accounts configured but no credentials.toml");
            return;
        }
        Err(err) => {
            tracing::warn!(%err, "mcal: credentials unreadable");
            return;
        }
    };
    for account in store.accounts.iter().filter(|a| a.kind == "google") {
        let provider = GoogleProvider::new(account.id.clone(), credentials.clone());
        collect(&provider, window, out);
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p mcal provider`
Expected: PASS (regression guard still green; no panic on absent store).

- [ ] **Step 5: Commit**

```bash
git add mcal/src/provider/mod.rs
git commit -m "feat(mcal): merge Google accounts into load_all"
```

---

## Task 10: CLI `account` subcommands

**Files:**
- Modify: `mcal/src/main.rs`

**Interfaces:**
- Consumes: `mcal::{AccountStore, StoredAccount, load_google, setup_instructions, interactive_google_login, store_refresh_token, delete_refresh_token}`.
- Produces: `mcal account setup google` / `mcal account list` / `mcal account remove <id>`.

- [ ] **Step 1: Route `account` in the command match** (`mcal/src/main.rs`, in the `match positional.first()…` block, add an arm before the catch-all `Some(other)`):

```rust
        Some("account") => return run_account(&positional[1..]),
```

- [ ] **Step 2: Implement the account subcommand** (`mcal/src/main.rs`, add these functions):

```rust
/// `mcal account <list|setup|remove> …`
fn run_account(args: &[String]) -> ExitCode {
    match args.first().map(String::as_str) {
        None | Some("list") => account_list(),
        Some("setup") => match args.get(1).map(String::as_str) {
            Some("google") => account_setup_google(),
            Some(other) => fail(&format!("unknown provider: {other} (try: google)")),
            None => fail("account setup needs a provider, e.g. mcal account setup google"),
        },
        Some("remove") => match args.get(1) {
            Some(id) => account_remove(id),
            None => fail("account remove needs an id (see mcal account list)"),
        },
        Some(other) => fail(&format!("unknown account command: {other}")),
    }
}

fn account_list() -> ExitCode {
    let store = match mcal::AccountStore::load() {
        Ok(store) => store,
        Err(e) => return fail(&e.to_string()),
    };
    if store.accounts.is_empty() {
        println!("No accounts. Add one with: mcal account setup google");
        return ExitCode::SUCCESS;
    }
    for a in &store.accounts {
        println!("{:<8} {:<28} {}", a.kind, a.email, a.id);
    }
    ExitCode::SUCCESS
}

fn account_setup_google() -> ExitCode {
    let creds = match mcal::load_google() {
        Ok(Some(creds)) => creds,
        Ok(None) => {
            eprintln!("{}", mcal::setup_instructions());
            return ExitCode::FAILURE;
        }
        Err(e) => return fail(&e.to_string()),
    };

    let tokens = match mcal::interactive_google_login(&creds) {
        Ok(tokens) => tokens,
        Err(e) => return fail(&e.to_string()),
    };

    // The account id is the user's email; discover it from the token via the
    // primary calendar's id (always the account email for a Google account).
    let email = match primary_email(&tokens.access_token) {
        Ok(email) => email,
        Err(e) => return fail(&e.to_string()),
    };
    let id = mcal::AccountStore::google_id(&email);

    if let Err(e) = mcal::store_refresh_token(&id, &tokens.refresh_token) {
        return fail(&e.to_string());
    }
    let mut store = match mcal::AccountStore::load() {
        Ok(store) => store,
        Err(e) => return fail(&e.to_string()),
    };
    store.add(mcal::StoredAccount {
        id: id.clone(),
        kind: "google".into(),
        email: email.clone(),
        display_name: email.split('@').next().unwrap_or(&email).to_string(),
    });
    if let Err(e) = store.save() {
        return fail(&e.to_string());
    }
    println!("Connected {email}. Try: mcal today");
    ExitCode::SUCCESS
}

/// The account's email = the id of its `primary` calendar.
fn primary_email(access_token: &str) -> Result<String, mcal::McalError> {
    #[derive(serde::Deserialize)]
    struct Cal {
        id: String,
    }
    let cal: Cal = ureq::get("https://www.googleapis.com/calendar/v3/calendars/primary")
        .set("Authorization", &format!("Bearer {access_token}"))
        .call()
        .map_err(|e| mcal::McalError::Fetch {
            url: "calendars/primary".into(),
            source: Box::new(e),
        })?
        .into_json()
        .map_err(|e| mcal::McalError::Json(e.to_string()))?;
    Ok(cal.id)
}

fn account_remove(id: &str) -> ExitCode {
    let mut store = match mcal::AccountStore::load() {
        Ok(store) => store,
        Err(e) => return fail(&e.to_string()),
    };
    if !store.remove(id) {
        return fail(&format!("no such account: {id}"));
    }
    if let Err(e) = store.save() {
        return fail(&e.to_string());
    }
    let _ = mcal::delete_refresh_token(id);
    println!("Removed {id}.");
    ExitCode::SUCCESS
}
```

- [ ] **Step 3: Add `account` to the CLI help** (`mcal/src/main.rs`, in the `USAGE` const's COMMANDS section):

```
    account list             List connected accounts
    account setup google     Connect a Google account (OAuth)
    account remove <id>      Disconnect an account
```

Also export the needed items — confirm `mcal/src/lib.rs` re-exports `McalError` (it does) and everything used above (added in Tasks 2/3/4/7).

- [ ] **Step 4: Build + verify empty-store output**

Run: `cargo build -p mcal && ./target/debug/mcal account list`
Expected: builds; prints `No accounts. Add one with: mcal account setup google` (assuming no accounts.toml yet).

- [ ] **Step 5: Commit**

```bash
git add mcal/src/main.rs
git commit -m "feat(mcal): account setup google / list / remove CLI"
```

---

## Task 11: Exports, gates, and manual verification

**Files:**
- Modify: `mcal/src/lib.rs` (final export audit)

- [ ] **Step 1: Audit `lib.rs` exports** — confirm all of these are `pub use`d (add any missing):

```rust
pub use account::{AccountStore, StoredAccount, accounts_path};
pub use credentials::{GoogleCredentials, credentials_path, load_google, setup_instructions};
pub use oauth::{GoogleTokens, interactive_google_login, refresh_access_token};
pub use provider::{GoogleProvider, LocalProvider, Provider, RemoteIcsProvider, Window, load_all};
pub use secret::{delete_refresh_token, get_refresh_token, store_refresh_token};
```

- [ ] **Step 2: Run the full gate set**

```bash
cargo +1.95.0 fmt --all
cargo +1.95.0 fmt --all -- --check
cargo clippy -p mcal --all-targets -- -D warnings
cargo test -p mcal
bash scripts/panic-ratchet.sh
```
Expected: fmt clean, clippy exit 0, all tests pass (keyring test ignored), panic-ratchet at baseline 370.

- [ ] **Step 3: Commit any formatting**

```bash
git add -A
git commit -m "chore(mcal): fmt + export audit for the Google slice"
```

- [ ] **Step 4: Manual end-to-end verification** (with the user's real Google account — this is the only path the unit tests can't cover):

  1. Create the Google Cloud OAuth client (Desktop app) per `mcal account setup google`'s printed instructions; put id/secret in `~/.config/mcal/credentials.toml`.
  2. `mcal account setup google` → browser opens → approve → "Connected …".
  3. `mcal account list` → shows the Google account.
  4. `mcal today` / `mcal agenda 14` → shows Google events (compecta calendar).
  5. Restart mshell (user's rebuild flow) → clock-menu agenda + dashboard show Google events too (no code change — `load_all` already merges them).
  6. `mcal account remove google:<email>` → gone from `list` and keyring entry deleted.

- [ ] **Step 5: Push**

```bash
git push
```

---

## Self-Review

**Spec coverage:**
- OAuth loopback+PKCE → Tasks 5–7. ✓
- BYO credentials (`credentials.toml`) + guided instructions → Task 2. ✓
- Account store (`accounts.toml`) → Task 3. ✓
- Keyring refresh tokens → Task 4. ✓
- GoogleProvider (Calendar API v3, `singleEvents=true`, JSON→Event) → Task 8. ✓
- Unified `load_all` → Task 9. ✓
- CLI `account setup/list/remove` → Task 10. ✓
- New deps (keyring/serde_json/sha2/base64; toml already present) → Task 1. ✓
- Error handling (per-source skip, guided-missing-creds, invalid_grant surfaced) → Tasks 1/2/9. ✓
- Read-only scope, no panics, English CLI strings → Global Constraints, enforced Task 11. ✓

**Placeholder scan:** no `TBD`/`add error handling`/vague steps; every code step carries full code.

**Type consistency:** `TokenResponse` (Task 6) consumed by `refresh_access_token` (Task 6) and `interactive_google_login` (Task 7); `GoogleTokens.{refresh_token,access_token}` produced Task 7, consumed Tasks 8/10; `AccountStore::{google_id,add,remove,load,save}` defined Task 3, used Tasks 9/10; `GoogleProvider::new(account_id, credentials)` defined Task 8, used Task 9. Consistent.
