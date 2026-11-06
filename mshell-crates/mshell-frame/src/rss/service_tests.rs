//! The notifier loop against a fake fetch, and the real conditional GET
//! against a local HTTP server serving a captured feed.

use super::*;
use mshell_config::schema::config::RssFeed;
use std::io::{BufRead, BufReader, Write};

const ARCH: &str = include_str!("testdata/arch-news.xml");
const ARCH_URL: &str = "https://archlinux.org/feeds/news/";

fn fetch_arch(_: &str, _: Option<&str>, _: Option<&str>) -> Result<Fetched, String> {
    Ok(Fetched::Feed {
        feed: parse_feed(ARCH)?,
        etag: Some("\"v2\"".into()),
        last_modified: None,
    })
}

fn fetch_fails(_: &str, _: Option<&str>, _: Option<&str>) -> Result<Fetched, String> {
    Err("HTTP 503".into())
}

fn cfg(urls: &[&str]) -> RssConfig {
    RssConfig {
        feeds: urls
            .iter()
            .map(|u| RssFeed {
                name: String::new(),
                url: u.to_string(),
            })
            .collect(),
        ..Default::default()
    }
}

async fn settle_until(path: &Path, pred: impl Fn(&SeenStore) -> bool) -> SeenStore {
    for _ in 0..200 {
        if let Ok(t) = std::fs::read_to_string(path)
            && let Ok(s) = SeenStore::from_json(&t)
            && pred(&s)
        {
            return s;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("store never matched");
}

#[tokio::test]
async fn first_poll_primes_silently_then_new_items_notify() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rss-seen.json");
    let (cfg_tx, cfg_rx) = watch::channel(cfg(&[ARCH_URL, "ftp://ignored.test/x"]));
    let (ntx, mut nrx) = mpsc::unbounded_channel();
    let task = tokio::spawn(worker(cfg_rx, path.clone(), fetch_arch, ntx));
    let s = settle_until(&path, |s| s.feeds.contains_key(ARCH_URL)).await;
    assert_eq!(s.feeds.len(), 1, "invalid URLs are never fetched");
    assert_eq!(s.feeds[ARCH_URL].ids.len(), 3);
    assert_eq!(s.feeds[ARCH_URL].etag.as_deref(), Some("\"v2\""));
    assert!(
        nrx.try_recv().is_err(),
        "adding a feed doesn't dump its backlog"
    );
    task.abort();
    drop(cfg_tx);

    // Restart with the newest item forgotten: it is new now.
    let mut st = SeenStore::from_json(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let f = st.feeds.get_mut(ARCH_URL).unwrap();
    f.ids.remove(0);
    f.last_checked = 0;
    std::fs::write(&path, st.to_json()).unwrap();
    let (cfg_tx, cfg_rx) = watch::channel(cfg(&[ARCH_URL]));
    let (ntx, mut nrx) = mpsc::unbounded_channel();
    let task = tokio::spawn(worker(cfg_rx, path.clone(), fetch_arch, ntx));
    let n = tokio::time::timeout(Duration::from_secs(5), nrx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(n.title.starts_with("Update to php"), "{}", n.title);
    assert_eq!(
        n.feed, "Arch Linux: Recent news updates",
        "the feed's title"
    );
    assert!(n.link.unwrap().starts_with("https://archlinux.org/news/"));
    assert!(nrx.try_recv().is_err(), "one new item, one notification");

    // Removing the feed forgets it.
    cfg_tx.send(cfg(&[])).unwrap();
    settle_until(&path, |s| s.feeds.is_empty()).await;
    task.abort();
}

#[tokio::test]
async fn a_failing_feed_is_retried_later_and_notifies_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rss-seen.json");
    let (_cfg_tx, cfg_rx) = watch::channel(cfg(&[ARCH_URL]));
    let (ntx, mut nrx) = mpsc::unbounded_channel();
    let task = tokio::spawn(worker(cfg_rx, path.clone(), fetch_fails, ntx));
    let s = settle_until(&path, |s| s.feeds.contains_key(ARCH_URL)).await;
    assert!(s.feeds[ARCH_URL].last_checked > 0);
    assert!(!s.feeds[ARCH_URL].primed);
    assert!(nrx.try_recv().is_err());
    task.abort();
}

#[tokio::test]
async fn no_feeds_no_store_and_no_timer() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rss-seen.json");
    let (cfg_tx, cfg_rx) = watch::channel(RssConfig::default());
    let (ntx, _nrx) = mpsc::unbounded_channel();
    let task = tokio::spawn(worker(cfg_rx, path.clone(), fetch_fails, ntx));
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!path.exists(), "nothing configured, nothing written");
    drop(cfg_tx);
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("the worker ends with the config channel")
        .unwrap();
}

/// A one-thread HTTP/1.1 server: 200 + ETag/Last-Modified, or 304 when
/// the request carries the ETag. Returns the URL and the request heads.
fn serve(requests: usize) -> (String, std::thread::JoinHandle<Vec<String>>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/feeds/news/", listener.local_addr().unwrap());
    let handle = std::thread::spawn(move || {
        let mut heads = Vec::new();
        for _ in 0..requests {
            let (mut sock, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(sock.try_clone().unwrap());
            let mut head = String::new();
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" || line.is_empty() {
                    break;
                }
                head.push_str(&line);
            }
            let resp = if head.to_lowercase().contains("if-none-match: \"abc\"") {
                "HTTP/1.1 304 Not Modified\r\nETag: \"abc\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string()
            } else {
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/rss+xml; charset=utf-8\r\nETag: \"abc\"\r\nLast-Modified: Wed, 21 Oct 2026 19:29:36 GMT\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{ARCH}",
                    ARCH.len()
                )
            };
            sock.write_all(resp.as_bytes()).unwrap();
            heads.push(head);
        }
        heads
    });
    (url, handle)
}

#[test]
fn conditional_get_round_trip() {
    let (url, server) = serve(2);
    let Fetched::Feed {
        feed,
        etag,
        last_modified,
    } = fetch(&url, None, None).unwrap()
    else {
        panic!("first fetch returns the feed");
    };
    assert_eq!(feed.items.len(), 3);
    assert_eq!(etag.as_deref(), Some("\"abc\""));
    assert_eq!(
        last_modified.as_deref(),
        Some("Wed, 21 Oct 2026 19:29:36 GMT")
    );
    let again = fetch(&url, etag.as_deref(), last_modified.as_deref()).unwrap();
    assert!(matches!(again, Fetched::NotModified), "{again:?}");
    let heads = server.join().unwrap();
    assert!(heads[0].contains("User-Agent: mshell-rss/"));
    let second = heads[1].to_lowercase();
    assert!(second.contains("if-none-match: \"abc\""));
    assert!(second.contains("if-modified-since: wed, 21 oct 2026 19:29:36 gmt"));
}

#[test]
fn http_errors_and_non_feeds_are_errors() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        for body in [
            "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 13\r\nConnection: close\r\n\r\n<html></html>",
        ] {
            let (mut sock, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(sock.try_clone().unwrap());
            let mut line = String::new();
            while reader.read_line(&mut line).is_ok() && line != "\r\n" && !line.is_empty() {
                line.clear();
            }
            sock.write_all(body.as_bytes()).unwrap();
        }
    });
    assert_eq!(fetch(&url, None, None).unwrap_err(), "HTTP 404");
    assert!(
        fetch(&url, None, None).is_err(),
        "an HTML page is not a feed"
    );
    server.join().unwrap();
}

#[test]
fn charset_header_and_watched_feeds() {
    assert_eq!(
        charset("application/rss+xml; charset=ISO-8859-1"),
        Some("ISO-8859-1")
    );
    assert_eq!(charset("text/xml;Charset=\"utf-8\""), Some("utf-8"));
    assert_eq!(charset("text/xml"), None);
    let mut c = cfg(&[" https://a.test/f ", "https://a.test/f", "not a url"]);
    c.feeds[0].name = " A ".into();
    assert_eq!(
        watched(&c),
        [Watched {
            url: "https://a.test/f".into(),
            name: "A".into()
        }]
    );
    let mut c = RssConfig {
        interval_minutes: 1,
        ..Default::default()
    };
    assert_eq!(interval_secs(&c), 300, "at least 5 minutes");
    c.interval_minutes = 100_000;
    assert_eq!(interval_secs(&c), 86_400);
    assert_eq!(host("https://x.test/feed?token=secret"), "https://x.test");
    assert_eq!(escape_markup("A <b> & C"), "A &lt;b&gt; &amp; C");
}
