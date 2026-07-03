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

/// `~/.config/margo/mcal/credentials.toml` (honours `$XDG_CONFIG_HOME`).
pub fn credentials_path() -> PathBuf {
    crate::config::config_dir().join("credentials.toml")
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
