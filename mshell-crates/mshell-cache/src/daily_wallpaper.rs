//! Daily wallpaper: Bing's or NASA's image of the day.
//!
//! Port of the noctalia `daily-wallpaper` plugin: fetch today's image, cache
//! it under `~/Pictures/Wallpapers`, and prune downloads older than five
//! days. Next to each image a `<image>.json` sidecar keeps the caption that
//! came with it (Bing's headline + copyright line split into description and
//! credit, NASA's gallery caption) and the "Learn more" link (Bing's search
//! page about the place, NASA's image article), so the wallpaper menu can
//! show them whenever that image is the wallpaper — also after a restart.
//!
//! Everything here blocks (`curl`): call it off the UI thread, then apply the
//! returned path with [`crate::wallpaper::set_wallpaper`] on the main thread.

use relm4::gtk::glib;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// The caption and link published with a daily image.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DailyCaption {
    /// `"bing"` or `"nasa"`.
    pub source: String,
    /// Bing's headline (e.g. "One step closer to the summit"); empty for NASA.
    pub title: String,
    /// What the image shows: Bing's copyright line without the credit, or
    /// NASA's caption.
    pub description: String,
    /// Photographer / agency (Bing only), e.g. "© Alex Treadway/Getty Images".
    pub credit: String,
    /// "Learn more" page: Bing's search about the place, or NASA's article.
    /// Always an `http(s)` URL or empty.
    pub link: String,
}

impl DailyCaption {
    /// The line to lead with: the headline, else the description.
    pub fn headline(&self) -> &str {
        if self.title.is_empty() {
            &self.description
        } else {
            &self.title
        }
    }

    /// The description, unless it already is the [`Self::headline`].
    pub fn detail(&self) -> &str {
        if self.title.is_empty() {
            ""
        } else {
            &self.description
        }
    }

    /// True when there is nothing to show.
    pub fn is_empty(&self) -> bool {
        self.title.is_empty() && self.description.is_empty() && self.credit.is_empty()
    }
}

/// Where today's image comes from, plus its caption.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DailyImage {
    /// Preferred image URL (Bing UHD, NASA gallery image); may be empty.
    primary: String,
    /// Fallback image URL (Bing 1080p, NASA `og:image`); may be empty.
    fallback: String,
    caption: DailyCaption,
}

/// Downloaded dailies live here (matches the original plugin).
fn daily_wallpaper_dir() -> PathBuf {
    glib::home_dir().join("Pictures").join("Wallpapers")
}

fn curl_text(url: &str) -> Result<String, String> {
    let out = std::process::Command::new("curl")
        .args(["-fsSL", "--max-time", "20", url])
        .output()
        .map_err(|e| format!("curl spawn failed: {e}"))?;
    if !out.status.success() {
        return Err(format!("curl failed for {url}"));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn curl_download(url: &str, dest: &Path) -> bool {
    matches!(
        std::process::Command::new("curl")
            .args(["-fsSL", "--max-time", "60", "-o"])
            .arg(dest)
            .arg(url)
            .status(),
        Ok(s) if s.success()
    )
}

/// The substring between `start` and the first `end` following it.
fn between<'a>(s: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let i = s.find(start)? + start.len();
    let rest = &s[i..];
    rest.find(end).map(|j| &rest[..j])
}

/// Decode the HTML entities these pages use: the five named XML ones and
/// numeric `&#NN;` / `&#xHH;` references. Unknown entities stay as written.
fn decode_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let tail = &rest[amp..];
        let Some(semi) = tail.find(';').filter(|&i| i <= 10) else {
            out.push('&');
            rest = &tail[1..];
            continue;
        };
        let entity = &tail[1..semi];
        let decoded = match entity {
            "amp" => Some('&'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "nbsp" => Some(' '),
            _ => entity.strip_prefix('#').and_then(|num| {
                let code = match num.strip_prefix(['x', 'X']) {
                    Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                    None => num.parse().ok()?,
                };
                char::from_u32(code)
            }),
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &tail[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Keep `url` only when it is a web link; a page could otherwise smuggle a
/// `file:` or custom-scheme URL into the "Learn more" button.
fn web_link(url: &str) -> String {
    let url = url.trim();
    if url.starts_with("https://") || url.starts_with("http://") {
        url.to_string()
    } else {
        String::new()
    }
}

/// Bing's `copyright` is "Caption text (© Photographer/Source)": split off
/// the trailing parenthesised credit.
fn split_copyright(copyright: &str) -> (String, String) {
    let trimmed = copyright.trim();
    if let Some(body) = trimmed.strip_suffix(')')
        && let Some(open) = body.rfind('(')
        && !body[open + 1..].contains(')')
    {
        let credit = body[open + 1..].trim();
        let text = body[..open].trim();
        if !credit.is_empty() && !text.is_empty() {
            return (decode_html(text), decode_html(credit));
        }
    }
    (decode_html(trimmed), String::new())
}

/// Today's image from Bing's `HPImageArchive` JSON.
fn parse_bing(body: &str) -> Option<DailyImage> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let image = v.get("images")?.get(0)?;
    let text = |key: &str| image.get(key).and_then(|x| x.as_str()).unwrap_or("");
    let urlbase = text("urlbase");
    if urlbase.is_empty() {
        return None;
    }
    let (description, credit) = split_copyright(text("copyright"));
    let link = match web_link(text("copyrightlink")) {
        l if !l.is_empty() => l,
        _ => match text("quiz") {
            q if q.starts_with('/') => format!("https://www.bing.com{q}"),
            _ => String::new(),
        },
    };
    Some(DailyImage {
        primary: format!("https://www.bing.com{urlbase}_UHD.jpg"),
        fallback: format!("https://www.bing.com{urlbase}_1920x1080.jpg"),
        caption: DailyCaption {
            source: "bing".to_string(),
            title: decode_html(text("title").trim()),
            description,
            credit,
            link,
        },
    })
}

/// First `content="…"` after `key` (for `og:image` / `twitter:image` metas).
fn meta_content(body: &str, key: &str) -> Option<String> {
    let i = body.find(key)?;
    between(&body[i..], "content=\"", "\"").map(str::to_string)
}

/// An absolute `https://www.nasa.gov` URL for a gallery `src` / `href`.
fn absolute_nasa_url(url: &str) -> String {
    let url = decode_html(url.trim());
    if url.is_empty() || url.starts_with("https://") || url.starts_with("http://") {
        url
    } else if let Some(rest) = url.strip_prefix("//") {
        format!("https://{rest}")
    } else if let Some(rest) = url.strip_prefix('/') {
        format!("https://www.nasa.gov/{rest}")
    } else {
        format!("https://www.nasa.gov/{url}")
    }
}

/// Today's image from NASA's image-of-the-day page: the first gallery item
/// (its `<img>`, the full `alt` text — the overlay caption is cut short with
/// "..." — and the article link), with the `og:image` / `twitter:image` meta
/// as the fallback image.
fn parse_nasa(body: &str) -> Option<DailyImage> {
    // Bound later matches to the first gallery item's markup.
    let item = body
        .find("hds-gallery-image")
        .map(|i| &body[i..body.len().min(i + 4000)]);
    let attr = |key: &str| {
        item.and_then(|it| {
            let img = &it[it.find("<img")?..];
            between(img, key, "\"").map(str::to_string)
        })
    };
    let primary = attr(" src=\"")
        .map(|u| absolute_nasa_url(&u))
        .unwrap_or_default();
    let alt = attr(" alt=\"").map(|a| decode_html(a.trim()));
    let overlay = item
        .and_then(|it| {
            let i = it.find("hds-gallery-item-caption")?;
            between(&it[i..], ">", "<")
        })
        .map(|c| decode_html(c.trim()));
    let link = item
        .and_then(|it| {
            let i = it.find("hds-gallery-item-link")?;
            between(&it[i..], "href=\"", "\"")
        })
        .map(|h| web_link(&absolute_nasa_url(h)))
        .unwrap_or_default();
    let fallback = meta_content(body, "og:image")
        .or_else(|| meta_content(body, "twitter:image"))
        .map(|u| absolute_nasa_url(&u))
        .unwrap_or_default();
    if primary.is_empty() && fallback.is_empty() {
        return None;
    }
    let description = alt
        .filter(|a| !a.is_empty())
        .or(overlay)
        .unwrap_or_default();
    Some(DailyImage {
        primary,
        fallback,
        caption: DailyCaption {
            source: "nasa".to_string(),
            title: String::new(),
            description,
            credit: String::new(),
            link,
        },
    })
}

fn resolve_bing(market: &str) -> Result<DailyImage, String> {
    let url = format!("https://www.bing.com/HPImageArchive.aspx?format=js&idx=0&n=1&mkt={market}");
    parse_bing(&curl_text(&url)?).ok_or_else(|| "Bing: urlbase not found".to_string())
}

fn resolve_nasa() -> Result<DailyImage, String> {
    parse_nasa(&curl_text("https://www.nasa.gov/image-of-the-day/")?)
        .ok_or_else(|| "NASA: no image URL found".to_string())
}

/// Bing market code from the configured locale: lower-case, `_` → `-`,
/// `en-us` when blank.
fn bing_market(locale: &str) -> String {
    let l = locale.trim().to_lowercase().replace('_', "-");
    if l.is_empty() { "en-us".to_string() } else { l }
}

/// The caption sidecar of a daily image.
fn caption_path(image: &Path) -> PathBuf {
    image.with_extension("json")
}

/// The caption saved next to `image`, when it is a daily wallpaper that came
/// with one. A small local read; cheap enough for the menu to call on a
/// wallpaper change.
pub fn daily_caption(image: &Path) -> Option<DailyCaption> {
    let text = fs::read_to_string(caption_path(image)).ok()?;
    let caption: DailyCaption = serde_json::from_str(&text).ok()?;
    let caption = DailyCaption {
        link: web_link(&caption.link),
        ..caption
    };
    (!caption.is_empty()).then_some(caption)
}

fn write_caption(image: &Path, caption: &DailyCaption) {
    match serde_json::to_vec_pretty(caption) {
        Ok(bytes) => {
            if let Err(e) = margo_fs::atomic_write::atomic_write(&caption_path(image), &bytes) {
                tracing::warn!("daily wallpaper: caption write failed: {e}");
            }
        }
        Err(e) => tracing::warn!("daily wallpaper: caption encode failed: {e}"),
    }
}

/// Ensure today's Bing/NASA image-of-the-day is downloaded and return its
/// path (reusing the cached file if already present), pruning downloads older
/// than 5 days. `source` is `"bing"` or `"nasa"`; `locale` is the Bing market
/// (ignored for NASA). The caption is saved next to the image (see
/// [`daily_caption`]); a cached image without one gets it fetched again.
/// Blocking (curl) — call off the UI thread, then apply the returned path
/// with [`crate::wallpaper::set_wallpaper`] on the main thread
/// (`set_wallpaper` touches the reactive config store and must not run on a
/// worker thread).
pub fn fetch_daily_wallpaper(source: &str, locale: &str) -> Result<PathBuf, String> {
    let source = if source.eq_ignore_ascii_case("nasa") {
        "nasa"
    } else {
        "bing"
    };
    let dir = daily_wallpaper_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;

    let date = glib::DateTime::now_local()
        .ok()
        .and_then(|d| d.format("%Y-%m-%d").ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "today".to_string());
    let file = dir.join(format!("{source}-{date}.jpg"));
    let resolve = || {
        if source == "nasa" {
            resolve_nasa()
        } else {
            resolve_bing(&bing_market(locale))
        }
    };

    if file.is_file() {
        // Cached before captions were kept (or the sidecar was deleted):
        // fetch only the metadata; the image itself stays.
        if !caption_path(&file).is_file()
            && let Ok(today) = resolve()
        {
            write_caption(&file, &today.caption);
        }
        prune_daily(&dir, source);
        return Ok(file);
    }

    let today = resolve()?;
    let ok = (!today.primary.is_empty() && curl_download(&today.primary, &file))
        || (!today.fallback.is_empty() && curl_download(&today.fallback, &file));
    if !ok {
        let _ = fs::remove_file(&file); // drop any truncated partial
        return Err(format!("{source}: download failed"));
    }
    write_caption(&file, &today.caption);

    prune_daily(&dir, source);
    Ok(file)
}

/// Delete `<prefix>-*.jpg` (and its caption) in `dir` older than 5 days.
fn prune_daily(dir: &Path, prefix: &str) {
    let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(5 * 24 * 60 * 60);
    prune_daily_before(dir, prefix, cutoff);
}

/// Delete `<prefix>-*.jpg` and `<prefix>-*.json` in `dir` last modified
/// before `cutoff`.
fn prune_daily_before(dir: &Path, prefix: &str, cutoff: std::time::SystemTime) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let needle = format!("{prefix}-");
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !(name.starts_with(&needle) && (name.ends_with(".jpg") || name.ends_with(".json"))) {
            continue;
        }
        if let Ok(meta) = entry.metadata()
            && let Ok(modified) = meta.modified()
            && modified < cutoff
        {
            let _ = fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Captured from `HPImageArchive.aspx?format=js&idx=0&n=1&mkt=en-us`
    /// on 2026-11-05.
    const BING: &str = include_str!("../tests/fixtures/bing_hpimagearchive.json");
    /// The first gallery item + image metas of nasa.gov/image-of-the-day,
    /// captured on 2026-11-05 (the rest of the 280 KiB page trimmed).
    const NASA: &str = include_str!("../tests/fixtures/nasa_image_of_the_day.html");

    #[test]
    fn bing_fixture_yields_urls_caption_credit_and_link() {
        let today = parse_bing(BING).expect("bing");
        assert_eq!(
            today.primary,
            "https://www.bing.com/th?id=OHR.StileCrossing_EN-US9052435517_UHD.jpg"
        );
        assert_eq!(
            today.fallback,
            "https://www.bing.com/th?id=OHR.StileCrossing_EN-US9052435517_1920x1080.jpg"
        );
        let c = today.caption;
        assert_eq!(c.source, "bing");
        assert_eq!(c.title, "One step closer to the summit");
        assert_eq!(
            c.description,
            "Wooden stile at a stone wall, Black Crag, Lake District, England"
        );
        assert_eq!(c.credit, "© Alex Treadway/Getty Images");
        assert!(
            c.link
                .starts_with("https://www.bing.com/search?q=Lake+District"),
            "{}",
            c.link
        );
        assert_eq!(c.headline(), "One step closer to the summit");
        assert_eq!(c.detail(), c.description);
    }

    #[test]
    fn bing_without_an_image_is_none() {
        assert_eq!(parse_bing(r#"{"images":[]}"#), None);
        assert_eq!(parse_bing(r#"{"images":[{"urlbase":""}]}"#), None);
        // Truncated body.
        assert_eq!(parse_bing(r#"{"images":[{"urlbase":"/th?id=OHR"#), None);
    }

    #[test]
    fn bing_link_falls_back_to_the_quiz_and_drops_other_schemes() {
        let body = r#"{"images":[{"urlbase":"/th?id=X","copyright":"Owl","copyrightlink":"javascript:alert(1)","quiz":"/search?q=quiz"}]}"#;
        let c = parse_bing(body).expect("bing").caption;
        assert_eq!(c.link, "https://www.bing.com/search?q=quiz");
        assert_eq!(c.description, "Owl");
        assert_eq!(c.credit, "");
        assert_eq!(c.headline(), "Owl", "no title: the description leads");
        assert_eq!(c.detail(), "");
    }

    #[test]
    fn nasa_fixture_yields_gallery_image_full_caption_and_article() {
        let today = parse_nasa(NASA).expect("nasa");
        assert_eq!(
            today.primary,
            "https://www.nasa.gov/wp-content/uploads/2026/11/iss075e0227352orig.jpg"
        );
        assert_eq!(
            today.fallback,
            "https://www.nasa.gov/wp-content/uploads/2019/08/589552main_as11-40-5875_full.jpg"
        );
        let c = today.caption;
        assert_eq!(c.source, "nasa");
        // The `alt` text, not the overlay caption cut short with "...".
        assert_eq!(
            c.description,
            "City lights from the Midwestern United States to the East Coast sparkle \
             across the landscape a couple of hours before sunrise."
        );
        assert_eq!(c.title, "");
        assert_eq!(c.credit, "");
        assert_eq!(
            c.link,
            "https://www.nasa.gov/image-article/midwestern-city-lights/"
        );
    }

    #[test]
    fn nasa_html_prefers_gallery_then_meta() {
        let page = r#"<meta property="og:image" content="https://nasa.gov/og.jpg">
<div class="hds-gallery-image"><img src="https://nasa.gov/big.jpg" alt=""></div>"#;
        let today = parse_nasa(page).expect("nasa");
        assert_eq!(today.primary, "https://nasa.gov/big.jpg");
        assert_eq!(today.fallback, "https://nasa.gov/og.jpg");
        let meta_only = r#"<meta name="twitter:image" content="https://nasa.gov/tw.jpg">"#;
        let today = parse_nasa(meta_only).expect("meta");
        assert_eq!(today.primary, "");
        assert_eq!(today.fallback, "https://nasa.gov/tw.jpg");
        assert_eq!(parse_nasa("<html>maintenance</html>"), None);
    }

    #[test]
    fn nasa_relative_urls_become_absolute() {
        assert_eq!(
            absolute_nasa_url("/wp-content/a.jpg"),
            "https://www.nasa.gov/wp-content/a.jpg"
        );
        assert_eq!(
            absolute_nasa_url("//cdn.nasa.gov/a.jpg"),
            "https://cdn.nasa.gov/a.jpg"
        );
        assert_eq!(
            absolute_nasa_url("image-article/x/"),
            "https://www.nasa.gov/image-article/x/"
        );
        assert_eq!(absolute_nasa_url("https://a.b/c&amp;d"), "https://a.b/c&d");
        assert_eq!(absolute_nasa_url(""), "");
    }

    #[test]
    fn html_entities_decode() {
        assert_eq!(decode_html("spacesuit&#039;s"), "spacesuit's");
        assert_eq!(decode_html("NASA&#8217;s &amp; ESA"), "NASA\u{2019}s & ESA");
        assert_eq!(decode_html("&#x2014;&quot;x&quot;"), "\u{2014}\"x\"");
        assert_eq!(decode_html("a & b"), "a & b");
        assert_eq!(decode_html("&unknown; &#xZZ;"), "&unknown; &#xZZ;");
        assert_eq!(decode_html("trailing &"), "trailing &");
    }

    #[test]
    fn copyright_splits_off_only_a_trailing_credit() {
        assert_eq!(
            split_copyright("Falls, Zimbabwe (© Byron/Getty Images)"),
            (
                "Falls, Zimbabwe".to_string(),
                "© Byron/Getty Images".to_string()
            )
        );
        assert_eq!(
            split_copyright("No credit here"),
            ("No credit here".to_string(), String::new())
        );
        assert_eq!(
            split_copyright("(© only a credit)"),
            ("(© only a credit)".to_string(), String::new())
        );
        assert_eq!(
            split_copyright("A (b) c"),
            ("A (b) c".to_string(), String::new())
        );
    }

    #[test]
    fn only_web_links_survive() {
        assert_eq!(web_link(" https://a.b/c "), "https://a.b/c");
        assert_eq!(web_link("http://a.b"), "http://a.b");
        assert_eq!(web_link("file:///etc/passwd"), "");
        assert_eq!(web_link("javascript:alert(1)"), "");
        assert_eq!(web_link(""), "");
    }

    #[test]
    fn caption_sidecar_round_trips_next_to_the_image() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let image = tmp.path().join("bing-2026-11-05.jpg");
        assert_eq!(daily_caption(&image), None, "no sidecar yet");
        let caption = parse_bing(BING).expect("bing").caption;
        write_caption(&image, &caption);
        assert!(tmp.path().join("bing-2026-11-05.json").is_file());
        assert_eq!(daily_caption(&image), Some(caption));
    }

    #[test]
    fn caption_sidecar_is_sanitised_on_read() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let image = tmp.path().join("nasa-2026-11-05.jpg");
        fs::write(
            caption_path(&image),
            r#"{"source":"nasa","description":"Moon","link":"file:///x"}"#,
        )
        .expect("write");
        let c = daily_caption(&image).expect("caption");
        assert_eq!(c.description, "Moon");
        assert_eq!(c.link, "");
        fs::write(caption_path(&image), r#"{"source":"nasa"}"#).expect("write");
        assert_eq!(daily_caption(&image), None, "nothing to show");
        fs::write(caption_path(&image), "not json").expect("write");
        assert_eq!(daily_caption(&image), None);
    }

    #[test]
    fn bing_market_normalises_locale() {
        assert_eq!(bing_market("en_US"), "en-us");
        assert_eq!(bing_market("  de-DE "), "de-de");
        assert_eq!(bing_market(""), "en-us");
        assert_eq!(bing_market("   "), "en-us");
    }

    #[test]
    fn prune_removes_only_old_matching_images_and_captions() {
        use std::time::{Duration, SystemTime};
        let tmp = tempfile::tempdir().expect("tempdir");
        let now = SystemTime::now();
        let old = now - Duration::from_secs(10 * 24 * 3600);
        let mk = |name: &str, mtime: SystemTime| {
            let path = tmp.path().join(name);
            let f = fs::File::create(&path).expect("create");
            f.set_modified(mtime).expect("mtime");
        };
        mk("bing-2026-10-01.jpg", old);
        mk("bing-2026-10-01.json", old);
        mk("bing-2026-11-02.jpg", now);
        mk("bing-2026-11-02.json", now);
        mk("nasa-2026-10-01.jpg", old);
        mk("bing-notes.txt", old);
        mk("mybing-2026-10-01.jpg", old);
        prune_daily_before(tmp.path(), "bing", now - Duration::from_secs(5 * 24 * 3600));
        let mut left: Vec<String> = fs::read_dir(tmp.path())
            .expect("dir")
            .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(
            left,
            [
                "bing-2026-11-02.jpg",
                "bing-2026-11-02.json",
                "bing-notes.txt",
                "mybing-2026-10-01.jpg",
                "nasa-2026-10-01.jpg"
            ]
        );
    }
}
