//! Category table and the GTK-free choice logic of the Default Apps page.
//!
//! Each category has a *primary* MIME (whose current default the row
//! shows — the same type `xdg-mime query default` would be asked about)
//! and the full set of MIMEs written when the user picks an app. A row
//! lists the apps that *declare* one of its MIMEs (GIO's recommended
//! list: exact `MimeType=` entries plus the user's own associations), so
//! a file manager that merely opens zip-based documents does not show up
//! under Spreadsheet. Web browsers only appear in the Web Browser row —
//! they declare PDFs, images and source files as a side-effect of being
//! browsers — except when one already is the default, which always stays
//! in its row so the dropdown never lies about the current state.

/// One row of the page.
#[derive(Debug, Clone, Copy)]
pub(super) struct Category {
    pub label: &'static str,
    /// The type whose default the row shows. Always `mimes[0]`.
    pub primary: &'static str,
    /// Every type set when an app is picked (only those it can open).
    pub mimes: &'static [&'static str],
    /// Extra types whose handlers are offered too, but never written:
    /// any plain-text editor can edit source code.
    pub also_list: &'static [&'static str],
    /// The Web Browser row itself keeps browsers; every other row drops
    /// them (unless one is the current default).
    pub browsers: bool,
}

const fn row(label: &'static str, mimes: &'static [&'static str]) -> Category {
    Category {
        label,
        primary: mimes[0],
        mimes,
        also_list: &[],
        browsers: false,
    }
}

/// Source-code types owned by the Code Editor row (canonical
/// shared-mime-info names, not aliases). `text/html` stays with
/// the Web Browser row: picking an editor must not make links to local
/// HTML files open in it.
const CODE_MIMES: &[&str] = &[
    "text/x-python",
    "text/x-csrc",
    "text/x-chdr",
    "text/x-c++src",
    "text/x-c++hdr",
    "text/x-java",
    "text/rust",
    "text/x-go",
    "text/javascript",
    "application/json",
    "text/x-shellscript",
    "text/x-lua",
    "text/css",
    "application/xml",
    "text/markdown",
    "application/yaml",
    "application/toml",
];

pub(super) const CATEGORIES: &[Category] = &[
    Category {
        browsers: true,
        ..row(
            "Web Browser",
            &[
                "x-scheme-handler/http",
                "x-scheme-handler/https",
                "text/html",
            ],
        )
    },
    row("Email", &["x-scheme-handler/mailto"]),
    row("Calendar", &["text/calendar", "x-scheme-handler/webcal"]),
    row("Files", &["inode/directory"]),
    row(
        "Archive Manager",
        &[
            "application/zip",
            "application/x-tar",
            "application/x-compressed-tar",
            "application/x-7z-compressed",
            "application/x-xz",
            "application/vnd.rar",
        ],
    ),
    row(
        "Music",
        &[
            "audio/mpeg",
            "audio/flac",
            "audio/x-vorbis+ogg",
            "audio/ogg",
            "audio/vnd.wave",
        ],
    ),
    row("Video", &["video/mp4", "video/x-matroska", "video/webm"]),
    row(
        "Photos",
        &["image/jpeg", "image/png", "image/webp", "image/gif"],
    ),
    row("Text Editor", &["text/plain"]),
    Category {
        also_list: &["text/plain"],
        ..row("Code Editor", CODE_MIMES)
    },
    row("PDF Viewer", &["application/pdf"]),
    row(
        "Documents",
        &[
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "application/msword",
            "application/vnd.oasis.opendocument.text",
        ],
    ),
    row(
        "Spreadsheet",
        &[
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            "application/vnd.ms-excel",
            "application/vnd.oasis.opendocument.spreadsheet",
            "application/wps-office.xlsx",
            "application/wps-office.xls",
        ],
    ),
    row(
        "Presentation",
        &[
            "application/vnd.openxmlformats-officedocument.presentationml.presentation",
            "application/vnd.ms-powerpoint",
            "application/vnd.oasis.opendocument.presentation",
            "application/wps-office.pptx",
            "application/wps-office.ppt",
        ],
    ),
    row(
        "BitTorrent",
        &["application/x-bittorrent", "x-scheme-handler/magnet"],
    ),
];

/// An installed app as the choice logic sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct App {
    pub id: String,
    pub name: String,
    /// Registered as an `x-scheme-handler/http` handler.
    pub browser: bool,
}

/// One dropdown entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Choice {
    /// No default is set for the primary type: shown first so the row
    /// does not pretend the first app is already the default.
    NotSet,
    App(App),
}

/// The dropdown model for one row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Choices {
    pub items: Vec<Choice>,
    pub selected: u32,
}

/// Build a row's dropdown: `lists` are the per-MIME declared handlers
/// (already filtered to apps that should be shown), `default` the current
/// default of the primary type. Deduplicated by id, browsers dropped
/// unless the row wants them, sorted by name, the current default always
/// present and selected.
pub(super) fn choices(category: &Category, lists: Vec<Vec<App>>, default: Option<App>) -> Choices {
    let mut apps: Vec<App> = Vec::new();
    for app in lists.into_iter().flatten() {
        if !category.browsers && app.browser {
            continue;
        }
        if !apps.iter().any(|a| a.id == app.id) {
            apps.push(app);
        }
    }
    if let Some(def) = &default
        && !apps.iter().any(|a| a.id == def.id)
    {
        apps.push(def.clone());
    }
    apps.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.id.cmp(&b.id))
    });

    match default {
        Some(def) => {
            let selected = apps.iter().position(|a| a.id == def.id).unwrap_or(0) as u32;
            Choices {
                items: apps.into_iter().map(Choice::App).collect(),
                selected,
            }
        }
        None => {
            let mut items = vec![Choice::NotSet];
            items.extend(apps.into_iter().map(Choice::App));
            Choices { items, selected: 0 }
        }
    }
}

/// The category's types the picked app can open (`handles` asks GIO,
/// including MIME subclasses: a text/plain editor opens text/x-python).
/// The primary type is written even when the app does not declare it, so
/// the row reads back what the user picked.
pub(super) fn mimes_to_set(
    category: &Category,
    handles: impl Fn(&str) -> bool,
) -> Vec<&'static str> {
    category
        .mimes
        .iter()
        .copied()
        .filter(|m| *m == category.primary || handles(m))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn app(id: &str, name: &str) -> App {
        App {
            id: id.into(),
            name: name.into(),
            browser: false,
        }
    }

    fn browser(id: &str, name: &str) -> App {
        App {
            browser: true,
            ..app(id, name)
        }
    }

    fn by_label(label: &str) -> &'static Category {
        CATEGORIES.iter().find(|c| c.label == label).unwrap()
    }

    #[test]
    fn primary_is_first_of_its_own_list() {
        for c in CATEGORIES {
            assert_eq!(c.mimes.first(), Some(&c.primary), "{}", c.label);
        }
    }

    #[test]
    fn primaries_are_unique() {
        let mut seen = HashSet::new();
        for c in CATEGORIES {
            assert!(seen.insert(c.primary), "duplicate primary {}", c.primary);
        }
    }

    #[test]
    fn no_mime_is_written_by_two_rows() {
        // Picking an app in one row must never steal another row's types.
        let mut owner = std::collections::HashMap::new();
        for c in CATEGORIES {
            for m in c.mimes {
                if let Some(prev) = owner.insert(*m, c.label) {
                    panic!("{m} written by both {prev} and {}", c.label);
                }
            }
        }
    }

    #[test]
    fn labels_are_unique_and_only_browser_row_keeps_browsers() {
        let labels: HashSet<_> = CATEGORIES.iter().map(|c| c.label).collect();
        assert_eq!(labels.len(), CATEGORIES.len());
        let keep: Vec<_> = CATEGORIES
            .iter()
            .filter(|c| c.browsers)
            .map(|c| c.label)
            .collect();
        assert_eq!(keep, ["Web Browser"]);
    }

    #[test]
    fn plugin_rows_are_present() {
        for label in [
            "Text Editor",
            "Code Editor",
            "Spreadsheet",
            "Presentation",
            "Documents",
            "PDF Viewer",
            "Archive Manager",
        ] {
            let _ = by_label(label);
        }
        assert_eq!(by_label("Text Editor").primary, "text/plain");
        assert_eq!(by_label("PDF Viewer").primary, "application/pdf");
        assert!(
            by_label("Documents")
                .mimes
                .contains(&"application/vnd.oasis.opendocument.text")
        );
        assert!(!by_label("Code Editor").mimes.contains(&"text/html"));
    }

    #[test]
    fn dedupes_sorts_and_selects_the_default() {
        let c = by_label("Text Editor");
        let got = choices(
            c,
            vec![
                vec![app("nvim.desktop", "Neovim"), app("micro.desktop", "micro")],
                vec![app("micro.desktop", "micro")],
            ],
            Some(app("nvim.desktop", "Neovim")),
        );
        assert_eq!(
            got.items,
            vec![
                Choice::App(app("micro.desktop", "micro")),
                Choice::App(app("nvim.desktop", "Neovim")),
            ]
        );
        assert_eq!(got.selected, 1);
    }

    #[test]
    fn no_default_shows_not_set_first() {
        let c = by_label("Spreadsheet");
        let got = choices(c, vec![vec![app("calc.desktop", "LibreOffice Calc")]], None);
        assert_eq!(got.items[0], Choice::NotSet);
        assert_eq!(got.selected, 0);
        assert_eq!(got.items.len(), 2);
        // No apps at all: only the placeholder.
        let empty = choices(c, vec![vec![]], None);
        assert_eq!(empty.items, vec![Choice::NotSet]);
    }

    #[test]
    fn browsers_dropped_outside_the_browser_row_unless_default() {
        // Captured shape of `gio mime application/pdf` on this machine:
        // zathura plus several browsers.
        let pdf = by_label("PDF Viewer");
        let lists = vec![vec![
            app("org.pwmt.zathura.desktop", "Zathura"),
            browser("firefox.desktop", "Firefox"),
            browser("chromium.desktop", "Chromium"),
        ]];
        let got = choices(
            pdf,
            lists.clone(),
            Some(app("org.pwmt.zathura.desktop", "Zathura")),
        );
        assert_eq!(
            got.items,
            vec![Choice::App(app("org.pwmt.zathura.desktop", "Zathura"))]
        );

        // A browser that *is* the default stays, selected.
        let got = choices(pdf, lists, Some(browser("firefox.desktop", "Firefox")));
        assert_eq!(got.items.len(), 2);
        assert_eq!(
            got.items[got.selected as usize],
            Choice::App(browser("firefox.desktop", "Firefox"))
        );

        let web = by_label("Web Browser");
        let got = choices(
            web,
            vec![vec![browser("firefox.desktop", "Firefox")]],
            Some(browser("firefox.desktop", "Firefox")),
        );
        assert_eq!(got.items.len(), 1);
    }

    #[test]
    fn default_outside_the_declared_list_is_kept() {
        let c = by_label("Music");
        let got = choices(
            c,
            vec![vec![app("mtune.desktop", "Tune")]],
            Some(app("custom.desktop", "Custom Player")),
        );
        assert_eq!(got.items.len(), 2);
        assert_eq!(
            got.items[got.selected as usize],
            Choice::App(app("custom.desktop", "Custom Player"))
        );
    }

    #[test]
    fn only_openable_types_are_written_but_primary_always() {
        let docs = by_label("Documents");
        // An app that only opens .odt (a subclass check says no to docx).
        let set = mimes_to_set(docs, |m| m == "application/vnd.oasis.opendocument.text");
        assert_eq!(
            set,
            vec![
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
                "application/vnd.oasis.opendocument.text",
            ]
        );
        let all = mimes_to_set(docs, |_| true);
        assert_eq!(all, docs.mimes.to_vec());
    }
}
