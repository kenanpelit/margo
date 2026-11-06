// SPDX-License-Identifier: GPL-3.0-or-later

//! Blocking HTTP for the radio directory, podcast search/feeds and artwork.
//!
//! Every call here blocks: run it through [`off_main`] (a `gio` worker
//! thread) — never on the GTK thread. Bodies are capped so a hostile or
//! broken server cannot balloon memory.

use std::{io::Read, time::Duration};

use gtk::gio;

/// Sent on every request; radio-browser.info asks clients to identify.
pub fn user_agent() -> String {
    format!("mtune/{} (margo)", crate::config::VERSION)
}

const TIMEOUT: Duration = Duration::from_secs(15);

/// JSON API answers (station search, iTunes search).
pub const JSON_CAP: u64 = 4 * 1024 * 1024;
/// Podcast feeds: long-running shows ship multi-megabyte RSS.
pub const FEED_CAP: u64 = 24 * 1024 * 1024;
/// Station logos / podcast covers.
pub const IMAGE_CAP: u64 = 6 * 1024 * 1024;

/// Why a fetch failed, worded for a toast.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetError {
    /// DNS / TCP / TLS / timeout: nothing came back.
    Unreachable(String),
    /// The server answered with a non-2xx status.
    Status(u16),
    /// The body was larger than the cap for its kind.
    TooLarge,
    /// The body did not parse as what we asked for.
    Malformed(String),
}

impl std::fmt::Display for NetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NetError::Unreachable(e) => write!(f, "could not connect ({e})"),
            NetError::Status(code) => write!(f, "the server answered HTTP {code}"),
            NetError::TooLarge => write!(f, "the answer was too large"),
            NetError::Malformed(e) => write!(f, "unexpected answer ({e})"),
        }
    }
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(TIMEOUT)
        .user_agent(&user_agent())
        .redirects(8)
        .build()
}

/// GET `url` with `query` pairs (ureq percent-encodes them) and return the
/// body bytes, at most `cap` of them.
pub fn get_bytes(url: &str, query: &[(&str, String)], cap: u64) -> Result<Vec<u8>, NetError> {
    let mut req = agent().get(url);
    for (k, v) in query {
        req = req.query(k, v);
    }
    let resp = match req.call() {
        Ok(r) => r,
        Err(ureq::Error::Status(code, _)) => return Err(NetError::Status(code)),
        Err(e) => return Err(NetError::Unreachable(short_transport(&e))),
    };
    let mut body = Vec::new();
    resp.into_reader()
        .take(cap + 1)
        .read_to_end(&mut body)
        .map_err(|e| NetError::Unreachable(e.kind().to_string()))?;
    if body.len() as u64 > cap {
        return Err(NetError::TooLarge);
    }
    Ok(body)
}

/// GET and parse JSON.
pub fn get_json(url: &str, query: &[(&str, String)]) -> Result<serde_json::Value, NetError> {
    let body = get_bytes(url, query, JSON_CAP)?;
    serde_json::from_slice(&body).map_err(|e| NetError::Malformed(e.to_string()))
}

/// GET as (lossy) UTF-8 text.
pub fn get_text(url: &str, cap: u64) -> Result<String, NetError> {
    let body = get_bytes(url, &[], cap)?;
    Ok(String::from_utf8_lossy(&body).into_owned())
}

/// The transport error without the URL (which may carry a query the user
/// typed): just the kind.
fn short_transport(e: &ureq::Error) -> String {
    match e {
        ureq::Error::Transport(t) => t.kind().to_string(),
        ureq::Error::Status(code, _) => format!("HTTP {code}"),
    }
}

/// Run blocking `f` on a worker thread and await its result from the GTK
/// main context.
pub async fn off_main<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    gio::spawn_blocking(f).await.ok()
}

/// `http://` / `https://` with a host, no whitespace — the only URLs mtune
/// hands to GStreamer or fetches. Returns the trimmed URL.
pub fn validate_http_url(raw: &str) -> Option<String> {
    let url = raw.trim();
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    if host.is_empty() || url.chars().any(char::is_whitespace) || host.starts_with(':') {
        return None;
    }
    Some(url.to_string())
}

/// The host part of an http(s) URL, without a port or credentials.
pub fn url_host(url: &str) -> Option<&str> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?;
    let host = host.split(':').next()?;
    (!host.is_empty()).then_some(host)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_http_and_https_with_a_host() {
        assert_eq!(
            validate_http_url("  https://icecast.walmradio.com:8443/jazz \n").as_deref(),
            Some("https://icecast.walmradio.com:8443/jazz")
        );
        assert!(validate_http_url("http://jking.cdnstream1.com/b22139_128mp3").is_some());
        for bad in [
            "",
            "ftp://host/x",
            "file:///etc/passwd",
            "https://",
            "https:///path",
            "https://:8000/x",
            "http://host/with space",
            "icecast.walmradio.com/jazz",
        ] {
            assert_eq!(validate_http_url(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn host_drops_port_path_and_credentials() {
        assert_eq!(
            url_host("https://icecast.walmradio.com:8443/jazz"),
            Some("icecast.walmradio.com")
        );
        assert_eq!(
            url_host("http://u:p@radio.example/x?y"),
            Some("radio.example")
        );
        assert_eq!(url_host("ftp://x"), None);
    }
}
