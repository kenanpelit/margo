//! `case <text>` — convert text between naming conventions.
//!
//! Native port of the `case-convert` community plugin: every form of the
//! typed text (camelCase, PascalCase, snake_case, kebab-case,
//! CONSTANT_CASE, dot.case, path/case, Title Case, …) is listed one per
//! row with the convention as the subtitle. **Enter** copies the row,
//! **Ctrl+Enter** types it into the focused window with `wtype`.
//!
//! | Query | Input |
//! |---|---|
//! | `case parse http response` | the typed text |
//! | `case` | the primary selection, else the clipboard |
//! | `case >kebab` | the selection / clipboard, rows filtered by convention name |
//!
//! The selection is read with `wl-paste --primary` on a worker thread
//! ([`BgCache`]) — never on the GTK thread `search` runs on. Without
//! `wl-paste` the empty-query form shows a hint instead.
//!
//! Unlike the plugin (ASCII + Latin-1 only) casing uses Rust's full
//! Unicode mappings, so `ışık` → `IŞIK` and Greek/Cyrillic case too. The
//! word splitter only breaks on an explicit separator set and on
//! lower→upper / digit→upper / acronym boundaries, so `parseHTTPResponse`
//! and `parse-http-response` tokenize identically and multi-byte text is
//! never torn apart.

use crate::providers::bg_cache::BgCache;
use crate::{
    item::LauncherItem,
    notify::toast,
    provider::{Provider, RefreshNotifier},
};
use std::io::Write as _;
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::time::Duration;

const PREFIX: &str = "case";
const PROVIDER: &str = "Case";
const ICON: &str = "format-text-rich-symbolic";
/// How long a read selection stays fresh. Short: the user selects new
/// text, reopens the launcher, and expects that text.
const SELECTION_TTL: Duration = Duration::from_secs(2);
/// Longest input converted. A huge accidental selection would make every
/// row a wall of text; past this the user should narrow it down.
const MAX_INPUT_CHARS: usize = 2000;

/// Characters that separate words. Anything else (apostrophes,
/// brackets, …) stays inside a word.
fn is_separator(c: char) -> bool {
    c.is_whitespace() || matches!(c, '_' | '-' | '.' | '/' | '\\' | ':' | ',' | ';' | '|')
}

/// Words Title Case keeps lowercase unless first or last.
const SMALL_WORDS: &[&str] = &[
    "a", "an", "and", "as", "at", "but", "by", "en", "for", "from", "if", "in", "into", "nor",
    "of", "on", "onto", "or", "over", "per", "so", "the", "to", "up", "v", "via", "vs", "with",
    "yet",
];

/// Split `text` into words: separators are dropped, and camel / digit /
/// acronym boundaries become splits (`fooBar` → `foo Bar`, `utf8Text` →
/// `utf8 Text`, `HTTPCode` → `HTTP Code`).
pub fn tokenize(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    for chunk in text.split(is_separator).filter(|c| !c.is_empty()) {
        let chars: Vec<char> = chunk.chars().collect();
        let mut current = String::new();
        for (i, &c) in chars.iter().enumerate() {
            if i > 0 && c.is_uppercase() {
                let prev = chars[i - 1];
                let next_is_lower = chars.get(i + 1).is_some_and(|n| n.is_lowercase());
                let boundary = prev.is_lowercase()
                    || prev.is_ascii_digit()
                    || (prev.is_uppercase() && next_is_lower);
                if boundary && !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
            }
            current.push(c);
        }
        if !current.is_empty() {
            words.push(current);
        }
    }
    words
}

fn upper_first(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn lower_first(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// With `preserve` a word keeps its inner casing (`NASA rocks` →
/// `NASA_Rocks`); without it every word is lowercased first.
fn norm(word: &str, preserve: bool) -> String {
    if preserve {
        word.to_string()
    } else {
        word.to_lowercase()
    }
}

fn join(words: &[String], sep: &str, f: impl Fn(&str, usize, usize) -> String) -> String {
    let total = words.len();
    words
        .iter()
        .enumerate()
        .map(|(i, w)| f(w, i, total))
        .collect::<Vec<_>>()
        .join(sep)
}

fn swap_case(text: &str) -> String {
    text.chars()
        .flat_map(|c| -> Box<dyn Iterator<Item = char>> {
            if c.is_uppercase() {
                Box::new(c.to_lowercase())
            } else if c.is_lowercase() {
                Box::new(c.to_uppercase())
            } else {
                Box::new(std::iter::once(c))
            }
        })
        .collect()
}

fn alternating_case(text: &str) -> String {
    let mut letters = 0usize;
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_alphabetic() {
            letters += 1;
            if letters % 2 == 1 {
                out.extend(c.to_uppercase());
            } else {
                out.extend(c.to_lowercase());
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Row group, shown after the convention name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Programming,
    Text,
    Fun,
}

impl Group {
    fn label(self) -> &'static str {
        match self {
            Group::Programming => "Programming",
            Group::Text => "Text",
            Group::Fun => "Fun",
        }
    }
}

/// One conversion of the input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conversion {
    /// Stable key (row id).
    pub key: &'static str,
    /// Convention name, written in its own style (`kebab-case`).
    pub name: &'static str,
    pub group: Group,
    pub value: String,
}

/// Every conversion of `text` in display order, skipping empty results
/// (a separators-only input has no words, so the word-based forms vanish
/// while `UPPER CASE` etc. still show it).
pub fn convert_all(text: &str, preserve: bool) -> Vec<Conversion> {
    let text = text.trim();
    let words = tokenize(text);
    let p = preserve;
    let lower = |w: &str, _: usize, _: usize| w.to_lowercase();
    let upper = |w: &str, _: usize, _: usize| w.to_uppercase();
    let cap = |w: &str, _: usize, _: usize| upper_first(&norm(w, p));
    use Group::*;
    let rows: Vec<(&'static str, &'static str, Group, String)> = vec![
        (
            "camel",
            "camelCase",
            Programming,
            join(&words, "", |w, i, _| {
                if i == 0 {
                    lower_first(&norm(w, p))
                } else {
                    upper_first(&norm(w, p))
                }
            }),
        ),
        ("pascal", "PascalCase", Programming, join(&words, "", cap)),
        ("kebab", "kebab-case", Programming, join(&words, "-", lower)),
        ("snake", "snake_case", Programming, join(&words, "_", lower)),
        (
            "constant",
            "CONSTANT_CASE",
            Programming,
            join(&words, "_", upper),
        ),
        ("dot", "dot.case", Programming, join(&words, ".", lower)),
        ("path", "path/case", Programming, join(&words, "/", lower)),
        (
            "pascal_snake",
            "Pascal_Snake_Case",
            Programming,
            join(&words, "_", cap),
        ),
        (
            "kebab_upper",
            "KEBAB-UPPER-CASE",
            Programming,
            join(&words, "-", upper),
        ),
        ("header", "Header-Case", Programming, join(&words, "-", cap)),
        (
            "title",
            "Title Case",
            Text,
            join(&words, " ", |w, i, total| {
                let lowered = w.to_lowercase();
                if i > 0 && i + 1 < total && SMALL_WORDS.contains(&lowered.as_str()) {
                    lowered
                } else {
                    upper_first(&norm(w, p))
                }
            }),
        ),
        ("capital", "Capital Case", Text, join(&words, " ", cap)),
        (
            "sentence",
            "Sentence case",
            Text,
            join(&words, " ", |w, i, _| {
                if i == 0 {
                    upper_first(&norm(w, p))
                } else {
                    norm(w, p)
                }
            }),
        ),
        ("no_case", "no case", Text, join(&words, " ", lower)),
        ("lower", "lower case", Text, text.to_lowercase()),
        ("upper", "UPPER CASE", Text, text.to_uppercase()),
        ("swap", "sWAP cASE", Fun, swap_case(text)),
        (
            "alternating",
            "AlTeRnAtInG cAsE",
            Fun,
            alternating_case(text),
        ),
    ];
    rows.into_iter()
        .filter(|(_, _, _, v)| !v.is_empty())
        .map(|(key, name, group, value)| Conversion {
            key,
            name,
            group,
            value,
        })
        .collect()
}

/// What a `case …` query asks for.
#[derive(Debug, PartialEq, Eq)]
pub enum CaseQuery<'a> {
    /// Convert this typed text.
    Text(&'a str),
    /// Convert the selection / clipboard, showing conventions whose name
    /// contains this (lowercase) filter (`""` = all).
    External(String),
}

/// Parse the part after the `case` prefix.
pub fn parse_query(rest: &str) -> CaseQuery<'_> {
    let rest = rest.trim();
    match rest.strip_prefix('>') {
        Some(filter) => CaseQuery::External(filter.trim().to_lowercase()),
        None if rest.is_empty() => CaseQuery::External(String::new()),
        None => CaseQuery::Text(rest),
    }
}

/// Primary selection, else the clipboard (blocking — worker thread only).
/// Empty when `wl-paste` is missing or both are empty.
fn read_selection() -> String {
    if !margo_fs::path::on_path("wl-paste") {
        return String::new();
    }
    let read = |primary: bool| {
        let mut cmd = Command::new("wl-paste");
        if primary {
            cmd.arg("--primary");
        }
        cmd.args(["--no-newline", "--type", "text/plain"])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default()
    };
    let primary = read(true);
    if primary.is_empty() {
        read(false)
    } else {
        primary
    }
}

/// Run `argv` with `text` on stdin (off the GTK thread), toasting `done`
/// on success.
fn pipe_off_thread(argv: &'static [&'static str], text: String, delay: Duration, done: String) {
    std::thread::spawn(move || {
        std::thread::sleep(delay);
        let result = Command::new(argv[0])
            .args(&argv[1..])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .and_then(|mut child| {
                if let Some(mut stdin) = child.stdin.take() {
                    stdin.write_all(text.as_bytes())?;
                }
                child.wait()
            });
        match result {
            Ok(status) if status.success() => toast(PROVIDER, done),
            Ok(_) | Err(_) => {
                tracing::warn!(tool = argv[0], "case: {} failed", argv[0]);
                toast(PROVIDER, format!("{} failed", argv[0]));
            }
        }
    });
}

pub struct CaseConvertProvider {
    selection: BgCache<String>,
}

impl CaseConvertProvider {
    pub fn new() -> Self {
        Self {
            selection: BgCache::new(SELECTION_TTL),
        }
    }

    fn hint(id: &str, name: &str, description: String) -> LauncherItem {
        LauncherItem {
            id: format!("case:{id}"),
            name: name.into(),
            description,
            icon: "dialog-information-symbolic".into(),
            icon_is_path: false,
            score: 100.0,
            provider_name: PROVIDER.into(),
            usage_key: None,
            on_activate: Rc::new(|| {}),
        }
    }

    fn rows(text: &str, filter: &str) -> Vec<LauncherItem> {
        let text: String = text.chars().take(MAX_INPUT_CHARS).collect();
        let rows: Vec<LauncherItem> = convert_all(&text, true)
            .into_iter()
            .filter(|c| filter.is_empty() || c.name.to_lowercase().contains(filter))
            .enumerate()
            .map(|(idx, c)| {
                let value = c.value.clone();
                let name = c.name;
                LauncherItem {
                    id: format!("case:{}", c.key),
                    name: c.value,
                    description: format!("{} · {}", c.name, c.group.label()),
                    icon: ICON.into(),
                    icon_is_path: false,
                    // Fixed declaration order: the runtime sorts by score.
                    score: 200.0 - idx as f64,
                    provider_name: PROVIDER.into(),
                    usage_key: None,
                    on_activate: Rc::new(move || {
                        pipe_off_thread(
                            &["wl-copy"],
                            value.clone(),
                            Duration::ZERO,
                            format!("Copied {name}"),
                        );
                    }),
                }
            })
            .collect();
        if rows.is_empty() {
            return vec![Self::hint(
                "nomatch",
                "No matching conversion",
                format!("Filtering on: {filter}"),
            )];
        }
        rows
    }
}

impl Default for CaseConvertProvider {
    fn default() -> Self {
        Self::new()
    }
}

fn is_case_query(query: &str) -> bool {
    let q = query.trim_start();
    q == PREFIX || q.starts_with("case ")
}

impl Provider for CaseConvertProvider {
    fn name(&self) -> &str {
        PROVIDER
    }

    fn handles_search(&self) -> bool {
        false
    }

    fn handles_command(&self, query: &str) -> bool {
        is_case_query(query)
    }

    fn set_refresh_notifier(&mut self, notifier: RefreshNotifier) {
        self.selection.set_notifier(notifier);
    }

    fn commands(&self) -> Vec<LauncherItem> {
        vec![LauncherItem {
            id: "case:palette".into(),
            name: "case <text>".into(),
            description: "Convert text to camelCase, snake_case, kebab-case, …".into(),
            icon: ICON.into(),
            icon_is_path: false,
            score: 0.0,
            provider_name: PROVIDER.into(),
            usage_key: None,
            on_activate: Rc::new(|| {}),
        }]
    }

    fn search(&self, query: &str) -> Vec<LauncherItem> {
        if !is_case_query(query) {
            return Vec::new();
        }
        let rest = &query.trim_start()[PREFIX.len()..];
        match parse_query(rest) {
            CaseQuery::Text(text) => Self::rows(text, ""),
            CaseQuery::External(filter) => {
                if !margo_fs::path::on_path("wl-paste") {
                    return vec![Self::hint(
                        "nopaste",
                        "Type text to convert",
                        "Install wl-clipboard to convert the selection".into(),
                    )];
                }
                match self.selection.get(read_selection) {
                    None => vec![Self::hint(
                        "reading",
                        "Reading the selection…",
                        "Or type the text after `case `".into(),
                    )],
                    Some(text) if text.is_empty() => vec![Self::hint(
                        "empty",
                        "Type text to convert",
                        "Or select text first, then reopen with `case`".into(),
                    )],
                    Some(text) => Self::rows(&text, &filter),
                }
            }
        }
    }

    /// Ctrl+Enter types the conversion into the focused window.
    fn alt_action(&self, item: &LauncherItem) -> Option<Rc<dyn Fn() + 'static>> {
        let key = item.id.strip_prefix("case:")?;
        let name = convert_all("x", true)
            .into_iter()
            .find(|c| c.key == key)?
            .name;
        if !margo_fs::path::on_path("wtype") {
            return None;
        }
        let value = item.name.clone();
        Some(Rc::new(move || {
            // Let the launcher close and focus return before typing.
            pipe_off_thread(
                &["wtype", "-"],
                value.clone(),
                Duration::from_millis(250),
                format!("Typed {name}"),
            );
        }))
    }

    fn preview(&self, item: &LauncherItem) -> Option<crate::LauncherPreview> {
        Some(crate::LauncherPreview::mono(
            item.description.clone(),
            item.name.clone(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(text: &str, key: &str) -> String {
        convert_all(text, true)
            .into_iter()
            .find(|c| c.key == key)
            .map(|c| c.value)
            .unwrap_or_default()
    }

    /// The plugin README's table for `this is an example`.
    #[test]
    fn readme_example_table() {
        let expected = [
            ("camel", "thisIsAnExample"),
            ("pascal", "ThisIsAnExample"),
            ("kebab", "this-is-an-example"),
            ("snake", "this_is_an_example"),
            ("constant", "THIS_IS_AN_EXAMPLE"),
            ("dot", "this.is.an.example"),
            ("path", "this/is/an/example"),
            ("pascal_snake", "This_Is_An_Example"),
            ("kebab_upper", "THIS-IS-AN-EXAMPLE"),
            ("header", "This-Is-An-Example"),
            ("title", "This Is an Example"),
            ("capital", "This Is An Example"),
            ("sentence", "This is an example"),
            ("no_case", "this is an example"),
            ("lower", "this is an example"),
            ("upper", "THIS IS AN EXAMPLE"),
            ("swap", "THIS IS AN EXAMPLE"),
            ("alternating", "ThIs Is An ExAmPlE"),
        ];
        let all = convert_all("this is an example", true);
        assert_eq!(all.len(), expected.len());
        for ((key, want), got) in expected.iter().zip(&all) {
            assert_eq!(got.key, *key);
            assert_eq!(got.value, *want, "{key}");
        }
    }

    #[test]
    fn camel_acronym_and_digit_boundaries_split() {
        assert_eq!(tokenize("parseHTTPResponse"), ["parse", "HTTP", "Response"]);
        assert_eq!(
            tokenize("parse-http-response"),
            ["parse", "http", "response"]
        );
        assert_eq!(tokenize("utf8Text"), ["utf8", "Text"]);
        assert_eq!(
            tokenize("  a__b..c//d\\e:f,g;h|i "),
            ["a", "b", "c", "d", "e", "f", "g", "h", "i"]
        );
        assert_eq!(value("parseHTTPResponse", "snake"), "parse_http_response");
    }

    #[test]
    fn preserve_keeps_acronyms_in_mixed_case_forms() {
        assert_eq!(value("NASA rocks", "pascal_snake"), "NASA_Rocks");
        let flat = convert_all("NASA rocks", false);
        let ps = flat.iter().find(|c| c.key == "pascal_snake").unwrap();
        assert_eq!(ps.value, "Nasa_Rocks");
        // Lowercase-by-definition forms ignore it.
        assert_eq!(value("NASA rocks", "kebab"), "nasa-rocks");
    }

    #[test]
    fn title_case_keeps_inner_small_words_lowercase_only() {
        assert_eq!(
            value("the lord of the rings", "title"),
            "The Lord of the Rings"
        );
        assert_eq!(
            value("what are you looking at", "title"),
            "What Are You Looking At"
        );
    }

    #[test]
    fn unicode_is_cased_not_corrupted() {
        assert_eq!(value("ışık çağ", "constant"), "IŞIK_ÇAĞ");
        assert_eq!(value("café olé", "pascal"), "CaféOlé");
        assert_eq!(value("Straße", "upper"), "STRASSE");
        assert_eq!(value("Ωmega αlpha", "swap"), "ωMEGA ΑLPHA");
        assert_eq!(tokenize("größeÄnderung"), ["größe", "Änderung"]);
    }

    #[test]
    fn character_forms_keep_punctuation_word_forms_drop_it() {
        assert_eq!(value("hello, world!", "upper"), "HELLO, WORLD!");
        assert_eq!(value("hello, world!", "snake"), "hello_world!");
        // Separators only: no words, but the character forms survive.
        let all = convert_all("--", true);
        assert!(all.iter().all(|c| c.group != Group::Programming));
        assert_eq!(all.iter().find(|c| c.key == "lower").unwrap().value, "--");
    }

    #[test]
    fn query_forms() {
        assert_eq!(parse_query(" foo bar "), CaseQuery::Text("foo bar"));
        assert_eq!(parse_query(""), CaseQuery::External(String::new()));
        assert_eq!(parse_query(" >Kebab"), CaseQuery::External("kebab".into()));
    }

    #[test]
    fn typed_text_lists_every_form_in_order_with_copy_rows() {
        let p = CaseConvertProvider::new();
        let rows = p.search("case foo bar");
        assert_eq!(rows.len(), 18);
        assert_eq!(rows[0].name, "fooBar");
        assert!(rows[0].description.starts_with("camelCase"));
        assert!(rows.windows(2).all(|w| w[0].score > w[1].score));
        assert!(rows.iter().all(|r| r.usage_key.is_none()));
    }

    #[test]
    fn prefix_is_exact() {
        let p = CaseConvertProvider::new();
        assert!(p.handles_command("case"));
        assert!(p.handles_command("case foo"));
        assert!(!p.handles_command("cases"));
        assert!(!p.handles_command("casefoo"));
        assert!(p.search("firefox").is_empty());
        assert!(!p.handles_search());
    }

    #[test]
    fn hint_rows_have_no_alt_action() {
        let p = CaseConvertProvider::new();
        let hint = CaseConvertProvider::hint("empty", "x", String::new());
        assert!(p.alt_action(&hint).is_none());
    }
}
