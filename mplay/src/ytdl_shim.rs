//! Native port of the `yt-dlp-mpv` shim (was an external dotfiles
//! script). mpv's `ytdl_hook` invokes this as its `ytdl_path`; we forward
//! to `yt-dlp` with hardened YouTube options, a browser user-agent, a
//! cookie-file path, and an anti-bot client fallback. yt-dlp's **stdout**
//! (the JSON mpv consumes) is passed through untouched; only **stderr** is
//! inspected to decide whether to retry with the anti-bot client.

use std::io::Read;
use std::process::{Command, Stdio};

const YTDLP: &str = "yt-dlp";

fn config_home() -> String {
    std::env::var("XDG_CONFIG_HOME")
        .unwrap_or_else(|_| format!("{}/.config", std::env::var("HOME").unwrap_or_default()))
}

fn cookie_file() -> String {
    format!("{}/yt-dlp/cookies-youtube.txt", config_home())
}

/// Shared hardening flags for the normal (cookie/no-cookie) path.
fn extra_args() -> Vec<String> {
    [
        "--ignore-config",
        "--extractor-args",
        "youtube:player_client=web_safari,web,android_sdkless",
        "--js-runtimes",
        "deno",
        "--remote-components",
        "ejs:github",
        "--extractor-retries",
        "1",
        "--retries",
        "1",
        "--fragment-retries",
        "1",
        "--socket-timeout",
        "15",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// Flags for the anti-bot fallback client.
fn antibot_args() -> Vec<String> {
    [
        "--ignore-config",
        "--extractor-args",
        "youtube:player_client=tv_simply,web",
        "--js-runtimes",
        "deno",
        "--remote-components",
        "ejs:github",
        "--extractor-retries",
        "1",
        "--retries",
        "1",
        "--fragment-retries",
        "1",
        "--socket-timeout",
        "15",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// A browser user-agent to look less like a bot. Honours
/// `YT_DLP_BROWSER_USER_AGENT`, else derives one from helium-browser's
/// Chromium version.
fn resolve_browser_user_agent() -> Option<String> {
    if let Ok(ua) = std::env::var("YT_DLP_BROWSER_USER_AGENT")
        && !ua.is_empty()
    {
        return Some(ua);
    }
    let out = Command::new("helium-browser")
        .arg("--version")
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let version = parse_chromium_version(&text)?;
    Some(format!(
        "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{version} Safari/537.36"
    ))
}

/// Extract the `Chromium <x.y.z>` version token from a `--version` line.
fn parse_chromium_version(s: &str) -> Option<String> {
    let after = s.split("Chromium ").nth(1)?;
    let v: String = after
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    if v.is_empty() { None } else { Some(v) }
}

/// True when yt-dlp's stderr indicates a YouTube bot/captcha challenge.
fn is_antibot_error(stderr: &str) -> bool {
    const NEEDLES: [&str; 4] = [
        "not a bot",
        "captcha challenge before playback",
        "HTTP Error 429",
        "Precondition check failed",
    ];
    NEEDLES.iter().any(|n| stderr.contains(n))
}

/// Run yt-dlp with `pre` args (+ optional `timeout` seconds) and the
/// passthrough `tail` (mpv's URL + flags). stdout is inherited so the JSON
/// reaches mpv; stderr is captured and returned for inspection.
fn run_ytdlp(pre: &[String], tail: &[String], timeout_s: Option<u32>) -> (bool, String) {
    let mut cmd = match timeout_s {
        Some(secs) => {
            let mut c = Command::new("timeout");
            c.arg(secs.to_string()).arg(YTDLP);
            c
        }
        None => Command::new(YTDLP),
    };
    cmd.args(pre).args(tail);
    cmd.stdout(Stdio::inherit()).stderr(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return (false, format!("spawn yt-dlp failed: {e}")),
    };
    let mut err = String::new();
    if let Some(mut s) = child.stderr.take() {
        let _ = s.read_to_string(&mut err);
    }
    let ok = child.wait().map(|s| s.success()).unwrap_or(false);
    (ok, err)
}

fn try_antibot(tail: &[String]) -> bool {
    eprintln!("[mplay ytdlp] anti-bot fallback: youtube client tv_simply,web");
    let (ok, err) = run_ytdlp(&antibot_args(), tail, Some(7));
    if !ok {
        eprint!("{err}");
    }
    ok
}

/// Entry point for the hidden `mplay ytdlp …` subcommand. Returns a
/// process exit code.
pub fn run(tail: &[String]) -> i32 {
    let mut pre = extra_args();
    if let Some(ua) = resolve_browser_user_agent() {
        pre.push("--user-agent".into());
        pre.push(ua);
    }

    let cookies = cookie_file();
    if std::fs::metadata(&cookies).is_ok() {
        eprintln!("[mplay ytdlp] using cookies file: {cookies}");
        let mut with_cookies = pre.clone();
        with_cookies.push("--cookies".into());
        with_cookies.push(cookies);
        let (ok, err) = run_ytdlp(&with_cookies, tail, None);
        if ok {
            return 0;
        }
        if is_antibot_error(&err) {
            eprintln!("[mplay ytdlp] bot-check: trying anti-bot client…");
            return if try_antibot(tail) { 0 } else { 1 };
        }
        eprint!("{err}");
        return 1;
    }

    if try_antibot(tail) {
        return 0;
    }
    eprintln!("[mplay ytdlp] no cookies available; proceeding without auth");
    let (ok, err) = run_ytdlp(&pre, tail, None);
    if !ok {
        eprint!("{err}");
        return 1;
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_chromium_version() {
        assert_eq!(
            parse_chromium_version("Helium 1.2 Chromium 120.0.6099.109 stable").as_deref(),
            Some("120.0.6099.109")
        );
        assert_eq!(parse_chromium_version("no version here"), None);
    }

    #[test]
    fn detects_antibot_stderr() {
        assert!(is_antibot_error(
            "ERROR: Sign in to confirm you're not a bot"
        ));
        assert!(is_antibot_error("HTTP Error 429: Too Many Requests"));
        assert!(!is_antibot_error("ERROR: video unavailable"));
    }
}
