//! Best-effort proxy applier. margo has no runtime gsettings proxy daemon,
//! so this writes env vars that apps launched afterward (and the next
//! session) inherit. Not a live system-wide proxy.

use std::io::Write;

/// Write proxy env to ~/.config/environment.d/99-margo-proxy.conf and set it
/// on the current process so children inherit it.
pub fn apply(http: &str, https: &str, socks: &str, ignore: &str) -> std::io::Result<()> {
    let dir = dirs::config_dir().unwrap_or_default().join("environment.d");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("99-margo-proxy.conf");
    let mut lines = String::new();
    if !http.is_empty() {
        lines.push_str(&format!("http_proxy=http://{http}\nHTTP_PROXY=http://{http}\n"));
    }
    if !https.is_empty() {
        lines.push_str(&format!("https_proxy=http://{https}\nHTTPS_PROXY=http://{https}\n"));
    }
    if !socks.is_empty() {
        lines.push_str(&format!("all_proxy=socks5://{socks}\nALL_PROXY=socks5://{socks}\n"));
    }
    if !ignore.is_empty() {
        lines.push_str(&format!("no_proxy={ignore}\nNO_PROXY={ignore}\n"));
    }
    std::fs::File::create(&path)?.write_all(lines.as_bytes())?;
    // SAFETY: single-threaded GTK main thread; setting our own proxy env vars only.
    unsafe {
        if !http.is_empty() {
            std::env::set_var("http_proxy", format!("http://{http}"));
            std::env::set_var("HTTP_PROXY", format!("http://{http}"));
        }
        if !https.is_empty() {
            std::env::set_var("https_proxy", format!("http://{https}"));
            std::env::set_var("HTTPS_PROXY", format!("http://{https}"));
        }
    }
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
