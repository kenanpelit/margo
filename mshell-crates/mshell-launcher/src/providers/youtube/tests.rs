use super::*;
use std::cell::RefCell as StdRefCell;

const PROGRESS: &str = include_str!("../../../tests/fixtures/youtube-progress.json");
const SEARCH: &str = include_str!("../../../tests/fixtures/yt-search.json");

fn tmp() -> PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let d = std::env::temp_dir().join(format!(
        "mshell-yt-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A provider over temp files, offline, recording seeded queries.
fn provider(dir: &std::path::Path) -> (YoutubeProvider, Rc<StdRefCell<Vec<String>>>) {
    std::fs::write(dir.join("progress.json"), PROGRESS).unwrap();
    let seeded = Rc::new(StdRefCell::new(Vec::new()));
    let s = Rc::clone(&seeded);
    let p = YoutubeProvider::build(
        LibraryFile::new(dir.join("library.json")),
        dir.join("progress.json"),
        Thumbs::offline(dir.join("thumbs")),
    )
    .with_search_setter(Rc::new(move |t: &str| s.borrow_mut().push(t.to_string())));
    (p, seeded)
}

fn ids(rows: &[LauncherItem]) -> Vec<String> {
    rows.iter().map(|r| r.id.clone()).collect()
}

#[test]
fn routes() {
    assert_eq!(route(""), Route::Home);
    assert_eq!(route("  rust lang "), Route::Search("rust lang".into()));
    assert!(matches!(route("r"), Route::Hint(_)));
    assert_eq!(route(":later  fire"), Route::Later("fire".into()));
    assert_eq!(route(":history"), Route::History(String::new()));
    assert_eq!(route(":feed"), Route::Feed(String::new()));
    assert_eq!(route(":channels"), Route::Channels(String::new()));
    assert!(matches!(route(":nope"), Route::Hint(_)));
    assert_eq!(
        route("@UCaYhcUwRBNscFNUKTjgPFiA async"),
        Route::Channel {
            id: "UCaYhcUwRBNscFNUKTjgPFiA".into(),
            filter: "async".into()
        }
    );
    assert!(matches!(route("@nope"), Route::Hint(_)));
    assert_eq!(route("!5C_HPTJg5ek"), Route::Video("5C_HPTJg5ek".into()));
    assert_eq!(
        route("https://youtu.be/5C_HPTJg5ek"),
        Route::Url("https://youtu.be/5C_HPTJg5ek".into())
    );
}

#[test]
fn prefix_is_exact() {
    let dir = tmp();
    let (p, _) = provider(&dir);
    assert!(p.handles_command("yt"));
    assert!(p.handles_command("yt rust"));
    assert!(!p.handles_command("ytop"));
    assert!(p.search("firefox").is_empty());
    assert!(!p.handles_search());
    assert!(p.bypasses_category_for_query("yt rust"));
    assert!(!p.bypasses_category_for_query("yt"));
}

#[test]
fn home_lists_continue_watching_then_the_sections() {
    let dir = tmp();
    let (p, seeded) = provider(&dir);
    let rows = p.search("yt");
    // Only the half-watched video (61 s of 149) is "continue".
    assert_eq!(rows[0].id, "yt:v:c:5C_HPTJg5ek");
    assert!(
        rows[0].description.contains("▶ from 1:01"),
        "{}",
        rows[0].description
    );
    let all = ids(&rows);
    assert!(all.contains(&"yt:nav:later".to_string()));
    assert!(all.contains(&"yt:nav:history".to_string()));
    // No favourites yet → no feed row.
    assert!(!all.contains(&"yt:nav:feed".to_string()));
    let later = rows.iter().find(|r| r.id == "yt:nav:later").unwrap();
    (later.on_activate)();
    assert_eq!(
        seeded.borrow().last().map(String::as_str),
        Some("yt :later ")
    );
    assert!(p.keeps_open(later, false));
}

#[test]
fn search_rows_from_a_cached_listing() {
    let dir = tmp();
    let (p, _) = provider(&dir);
    let entries = listing::parse_listing(SEARCH).unwrap();
    p.fetcher.insert("s:rust lang", Fetch::Ready(entries));
    let rows = p.search("yt rust lang");
    let all = ids(&rows);
    assert!(all.contains(&"yt:v:s:5C_HPTJg5ek".to_string()));
    assert!(all.contains(&"yt:nav:ch:UCaYhcUwRBNscFNUKTjgPFiA".to_string()));
    assert!(all.iter().any(|i| i.starts_with("yt:p:")));
    let v = rows.iter().find(|r| r.id == "yt:v:s:5C_HPTJg5ek").unwrap();
    assert_eq!(v.name, "Rust in 100 Seconds");
    // Channel · duration · resume point from progress.json.
    assert_eq!(v.description, "Fireship · 2:29 · ▶ from 1:01");
    // Results keep yt-dlp's order.
    assert!(rows.windows(2).all(|w| w[0].score > w[1].score));
}

#[test]
fn ctrl_enter_on_a_video_opens_its_actions_page() {
    let dir = tmp();
    let (p, seeded) = provider(&dir);
    let entries = listing::parse_listing(SEARCH).unwrap();
    p.fetcher.insert("s:rust lang", Fetch::Ready(entries));
    let rows = p.search("yt rust lang");
    let v = rows.iter().find(|r| r.id == "yt:v:s:5C_HPTJg5ek").unwrap();
    (p.alt_action(v).unwrap())();
    assert_eq!(
        seeded.borrow().last().map(String::as_str),
        Some("yt !5C_HPTJg5ek")
    );
    assert!(
        p.keeps_open(v, true),
        "alt navigates, the launcher stays open"
    );
    assert!(!p.keeps_open(v, false), "Enter plays and closes");

    let page = p.search("yt !5C_HPTJg5ek");
    let page_ids = ids(&page);
    assert_eq!(page_ids[0], "yt:act:play:5C_HPTJg5ek");
    assert_eq!(page[0].description, "Resumes from 1:01");
    for want in ["start", "queue", "later", "copy", "browser"] {
        assert!(
            page_ids.contains(&format!("yt:act:{want}:5C_HPTJg5ek")),
            "{want}"
        );
    }
    assert!(page_ids.contains(&"yt:nav:vch:UCsBjURrPoezykLs9EqgamOA".to_string()));
}

#[test]
fn watch_later_toggle_saves_and_ctrl_delete_removes() {
    let dir = tmp();
    let (mut p, _) = provider(&dir);
    let page = p.search("yt !IT0187cHGfI");
    let save = page
        .iter()
        .find(|r| r.id == "yt:act:later:IT0187cHGfI")
        .unwrap();
    assert_eq!(save.name, "Save to Watch later");
    assert!(p.keeps_open(save, false));
    (save.on_activate)();
    // Persisted to library.json.
    let on_disk = LibraryFile::new(dir.join("library.json")).load().library;
    assert!(on_disk.is_saved("IT0187cHGfI"));
    let page = p.search("yt !IT0187cHGfI");
    assert!(page.iter().any(|r| r.name == "Remove from Watch later"));

    let later = p.search("yt :later");
    assert_eq!(later[0].id, "yt:v:l:IT0187cHGfI");
    assert!(p.can_delete(&later[0]));
    let row = clone_item(&later[0]);
    p.delete_item(&row);
    assert!(
        !LibraryFile::new(dir.join("library.json"))
            .load()
            .library
            .is_saved("IT0187cHGfI")
    );
    assert!(p.search("yt :later")[0].id.starts_with("yt:info:"));
}

#[test]
fn channel_page_toggles_the_favourite_and_feeds_the_home_view() {
    let dir = tmp();
    let (p, _) = provider(&dir);
    p.fetcher
        .insert("ch:UCaYhcUwRBNscFNUKTjgPFiA", Fetch::Ready(Vec::new()));
    let rows = p.search("yt @UCaYhcUwRBNscFNUKTjgPFiA");
    let fav = &rows[0];
    assert!(fav.name.starts_with("☆ Add"));
    (fav.on_activate)();
    assert!(
        p.state
            .borrow()
            .library
            .is_favourite("UCaYhcUwRBNscFNUKTjgPFiA")
    );
    assert!(
        p.search("yt @UCaYhcUwRBNscFNUKTjgPFiA")[0]
            .name
            .starts_with("★ Remove")
    );
    let home = ids(&p.search("yt"));
    assert!(home.contains(&"yt:nav:feed".to_string()));
    let chans = p.search("yt :channels");
    assert_eq!(chans[0].id, "yt:nav:ch:UCaYhcUwRBNscFNUKTjgPFiA");
    assert!(p.can_delete(&chans[0]));
}

#[test]
fn history_lists_by_recency_with_a_clear_step() {
    let dir = tmp();
    let (p, seeded) = provider(&dir);
    let rows = p.search("yt :history");
    assert_eq!(
        ids(&rows)[..3],
        [
            "yt:v:h:Quf05tmWjaQ".to_string(),
            "yt:v:h:5C_HPTJg5ek".to_string(),
            "yt:v:h:IT0187cHGfI".to_string()
        ]
    );
    assert!(rows[0].description.contains("watched"));
    assert!(p.can_delete(&rows[0]));
    let clear = rows.last().unwrap();
    assert_eq!(clear.id, "yt:nav:clearhist");
    (clear.on_activate)();
    assert_eq!(
        seeded.borrow().last().map(String::as_str),
        Some("yt :clear-history")
    );
    let confirm = p.search("yt :clear-history");
    assert_eq!(confirm[0].id, "yt:act:clearhist");
    assert!(confirm[0].name.contains("3 videos"));
    // Filter narrows by title / channel.
    let f = p.search("yt :history fireship");
    assert_eq!(ids(&f), ["yt:v:h:5C_HPTJg5ek"]);
}

#[test]
fn pasted_links() {
    let dir = tmp();
    let (p, _) = provider(&dir);
    let rows = p.search("yt https://youtu.be/5C_HPTJg5ek?t=30");
    assert_eq!(rows[0].id, "yt:act:play:5C_HPTJg5ek");
    let pl =
        p.search("yt https://www.youtube.com/playlist?list=PLai5B987bZ9CoVR-QEIN9foz4QCJ0H2Y8");
    assert_eq!(ids(&pl), ["yt:u:play", "yt:u:queue"]);
    let other = p.search("yt https://example.com/x");
    assert!(other[0].id.starts_with("yt:info:"));
}

#[test]
fn a_broken_library_is_shown_but_never_saved_over() {
    let dir = tmp();
    std::fs::write(dir.join("library.json"), "{ nope").unwrap();
    let (p, _) = provider(&dir);
    assert!(p.search("yt")[0].id == "yt:info:broken");
    let page = p.search("yt !IT0187cHGfI");
    let save = page
        .iter()
        .find(|r| r.id == "yt:act:later:IT0187cHGfI")
        .unwrap();
    (save.on_activate)();
    assert_eq!(
        std::fs::read_to_string(dir.join("library.json")).unwrap(),
        "{ nope"
    );
}

#[test]
fn search_tab_row_seeds_the_prefix() {
    let dir = tmp();
    let (p, seeded) = provider(&dir);
    let rows = p.browse("");
    assert_eq!(rows.len(), 1);
    (rows[0].on_activate)();
    assert_eq!(seeded.borrow().last().map(String::as_str), Some("yt "));
    assert!(p.browse("firefox").is_empty());
}

fn clone_item(i: &LauncherItem) -> LauncherItem {
    LauncherItem {
        id: i.id.clone(),
        name: i.name.clone(),
        description: i.description.clone(),
        icon: i.icon.clone(),
        icon_is_path: i.icon_is_path,
        score: i.score,
        provider_name: i.provider_name.clone(),
        usage_key: i.usage_key.clone(),
        on_activate: Rc::clone(&i.on_activate),
    }
}
