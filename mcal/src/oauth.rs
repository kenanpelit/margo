//! Google OAuth 2.0 for installed apps: loopback redirect + PKCE.
//!
//! `mcal account setup google` opens the browser, catches the redirect on a
//! throwaway `127.0.0.1` port, exchanges the code for tokens, and hands back a
//! refresh token (stored in the keyring by the caller).

use crate::credentials::GoogleCredentials;
use crate::error::McalError;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::TcpListener;

/// The minimum read-only calendar scope.
pub const SCOPE: &str = "https://www.googleapis.com/auth/calendar.readonly";

const AUTH_ENDPOINT: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";

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
    Ok(PkcePair {
        verifier,
        challenge,
    })
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
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(b);
            i += 3;
            continue;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_challenge_matches_rfc7636_vector() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(
            code_challenge(verifier),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
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

    #[test]
    fn parses_redirect_query() {
        let line = "GET /?code=4%2F0Ab&state=xyz&scope=cal HTTP/1.1";
        let params = parse_redirect_query(line);
        let get = |k: &str| {
            params
                .iter()
                .find(|(kk, _)| kk == k)
                .map(|(_, v)| v.clone())
        };
        assert_eq!(get("code").as_deref(), Some("4/0Ab"));
        assert_eq!(get("state").as_deref(), Some("xyz"));
    }

    #[test]
    fn parses_error_redirect() {
        let params = parse_redirect_query("GET /?error=access_denied&state=x HTTP/1.1");
        assert!(
            params
                .iter()
                .any(|(k, v)| k == "error" && v == "access_denied")
        );
    }
}
