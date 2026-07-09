//! Best-effort proxy applier. margo has no runtime gsettings proxy daemon,
//! so this writes env vars that the next session inherits. Not a live
//! system-wide proxy, and deliberately not a live *this-process* one either:
//! see [`apply`].

use std::io::Write;

/// Write proxy env to `~/.config/environment.d/99-margo-proxy.conf`.
///
/// This used to also `std::env::set_var` the same variables so that apps
/// launched from the running shell inherited them. That was unsound. The
/// safety requirement on `set_var` is not "the caller is the GTK main thread"
/// (which is what the old comment claimed) but "no other thread reads the
/// environment concurrently" — and mshell runs ~65 threads. glibc's `getenv`
/// is called from under our feet by `getaddrinfo` (every ureq request the AI
/// and weather widgets make), `tzset`, and locale setup, all of which race the
/// environ-pointer swap.
///
/// Consequence: a proxy set here reaches processes started after the next
/// login, not the ones already running. Making it live again would mean
/// threading the variables into each `Command` we spawn, not mutating our own
/// environment.
pub fn apply(http: &str, https: &str, socks: &str, ignore: &str) -> std::io::Result<()> {
    let dir = dirs::config_dir().unwrap_or_default().join("environment.d");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("99-margo-proxy.conf");
    let mut lines = String::new();
    if !http.is_empty() {
        lines.push_str(&format!(
            "http_proxy=http://{http}\nHTTP_PROXY=http://{http}\n"
        ));
    }
    if !https.is_empty() {
        lines.push_str(&format!(
            "https_proxy=http://{https}\nHTTPS_PROXY=http://{https}\n"
        ));
    }
    if !socks.is_empty() {
        lines.push_str(&format!(
            "all_proxy=socks5://{socks}\nALL_PROXY=socks5://{socks}\n"
        ));
    }
    if !ignore.is_empty() {
        lines.push_str(&format!("no_proxy={ignore}\nNO_PROXY={ignore}\n"));
    }
    std::fs::File::create(&path)?.write_all(lines.as_bytes())?;
    Ok(())
}

/// Remove the proxy env file (mode None).
pub fn clear() -> std::io::Result<()> {
    let path = dirs::config_dir()
        .unwrap_or_default()
        .join("environment.d/99-margo-proxy.conf");
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}
