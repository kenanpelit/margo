//! Pure source/URL helpers (no I/O). Clipboard reads + yt-dlp spawning
//! live in `control.rs`; this module only classifies + normalizes.

/// Trim surrounding whitespace and stray carriage returns from a source
/// string (clipboard contents often arrive with a trailing newline).
pub fn normalize_source(s: &str) -> String {
    s.replace('\r', "").trim().to_string()
}

/// True when `s` is a YouTube watch/share URL (youtube.com,
/// youtube-nocookie.com, youtu.be), with or without a leading `www.`.
pub fn is_youtube_url(s: &str) -> bool {
    let s = s.trim();
    let rest = match s
        .strip_prefix("https://")
        .or_else(|| s.strip_prefix("http://"))
    {
        Some(r) => r,
        None => return false,
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.strip_prefix("www.").unwrap_or(host);
    matches!(
        host,
        "youtube.com" | "m.youtube.com" | "youtube-nocookie.com" | "youtu.be"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_youtube() {
        assert!(is_youtube_url("https://youtu.be/abc"));
        assert!(is_youtube_url("https://www.youtube.com/watch?v=x"));
        assert!(is_youtube_url("https://m.youtube.com/watch?v=x"));
        assert!(is_youtube_url("http://youtube-nocookie.com/embed/x"));
        assert!(!is_youtube_url("https://example.com/v.mp4"));
        assert!(!is_youtube_url("https://notyoutube.com.evil.test/x"));
        assert!(!is_youtube_url("/home/u/clip.mkv"));
        assert!(!is_youtube_url(""));
    }

    #[test]
    fn normalizes_clipboard_noise() {
        assert_eq!(normalize_source("  https://x/v.mp4\n"), "https://x/v.mp4");
        // CRs are stripped (clipboard noise); interior content is kept.
        assert_eq!(normalize_source("a\r\nb"), "a\nb");
        assert_eq!(normalize_source("\r\n url \r\n"), "url");
    }
}
