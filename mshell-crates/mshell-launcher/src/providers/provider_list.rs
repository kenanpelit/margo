//! `;` palette — discoverable cheatsheet of every prefix the
//! launcher understands.
//!
//! Walker calls this "providerlist". When the user types `;` the
//! provider shows one row per known launcher prefix with a short
//! example query. Activating a row sets the search text to that
//! prefix (UI hooks the row's `on_activate` to forward a callback
//! the widget supplies on construction).
//!
//! Note that this provider *cannot* introspect the runtime's
//! registered providers — that would require a back-reference
//! that breaks `dyn Provider` object safety. We instead carry a
//! static catalogue of every prefix that *might* be registered;
//! prefixes the user's runtime doesn't actually have just won't
//! produce results when activated, which the user discovers via
//! the empty result list and corrects by tweaking the prefix.

use crate::{item::LauncherItem, provider::Provider};
use std::rc::Rc;

/// One catalogue entry.
struct Entry {
    /// What the user types to invoke this provider.
    prefix: &'static str,
    /// Example query the cheatsheet shows.
    example: &'static str,
    /// Description / label of the provider.
    description: &'static str,
    /// Icon name.
    icon: &'static str,
}

/// Static catalogue. Order is "most commonly reached for" —
/// users skim from the top.
const ENTRIES: &[Entry] = &[
    Entry {
        prefix: "",
        example: "firefox",
        description: "Apps — fuzzy-search desktop entries",
        icon: "view-app-grid-symbolic",
    },
    Entry {
        prefix: "",
        example: "kitty",
        description: "Windows — open client list (focus by title)",
        icon: "window-symbolic",
    },
    Entry {
        prefix: "",
        example: "2+2",
        description: "Calculator — inline math (sqrt, sin, pi …)",
        icon: "accessories-calculator-symbolic",
    },
    Entry {
        prefix: "",
        example: "lock / reboot / shutdown",
        description: "Session — power actions",
        icon: "system-shutdown-symbolic",
    },
    Entry {
        prefix: "",
        example: "wallpaper next / night / screenshot region",
        description: "Margo — compositor quick-actions",
        icon: "preferences-desktop-symbolic",
    },
    Entry {
        prefix: "",
        example: "display / theme / fonts / launcher",
        description: "Settings — jump to a sidebar section",
        icon: "preferences-system-symbolic",
    },
    Entry {
        prefix: ">cmd",
        example: ">cmd echo hi",
        description: "Run a shell command (history-aware)",
        icon: "utilities-terminal-symbolic",
    },
    Entry {
        prefix: ">start",
        example: ">start brave",
        description: "Run a start-* script from $PATH",
        icon: "utilities-terminal-symbolic",
    },
    Entry {
        prefix: ">clip",
        example: ">clip url",
        description: "Browse clipboard history",
        icon: "edit-paste-symbolic",
    },
    Entry {
        prefix: ">clear",
        example: ">clear",
        description: "Wipe clipboard history",
        icon: "edit-clear-all-symbolic",
    },
    Entry {
        prefix: ".",
        example: ".arrow",
        description: "Pick a special character (→ ± π ©)",
        icon: "format-text-symbolic",
    },
    Entry {
        prefix: ":",
        example: ":smile",
        description: "Emoji picker (Unicode keyword search)",
        icon: "face-smile-symbolic",
    },
    Entry {
        prefix: "g <query>",
        example: "g rust async",
        description: "Web search — Google (also y / ddg / gh / aur / arch / wiki)",
        icon: "system-search-symbolic",
    },
    Entry {
        prefix: "p <query>",
        example: "p firefox",
        description: "Arch / AUR package search",
        icon: "package-x-generic-symbolic",
    },
    Entry {
        prefix: "audio",
        example: "audio",
        description: "Switch audio default sink / source",
        icon: "audio-volume-high-symbolic",
    },
    Entry {
        prefix: "bt",
        example: "bt",
        description: "Bluetooth devices — connect / disconnect",
        icon: "bluetooth-symbolic",
    },
    Entry {
        prefix: "ssh <host>",
        example: "ssh vhay",
        description: "Open a terminal SSH connection to an assh.yml host",
        icon: "network-server-symbolic",
    },
    Entry {
        prefix: "player",
        example: "player",
        description: "MPRIS players — play / pause / next",
        icon: "media-playback-start-symbolic",
    },
    Entry {
        prefix: ";",
        example: ";",
        description: "(this list)",
        icon: "help-browser-symbolic",
    },
];

/// Callback the UI registers — sets the launcher's search entry
/// text. Activating a `;` entry calls this so the user lands on
/// the chosen prefix without retyping.
pub type SetSearchText = Rc<dyn Fn(&str) + 'static>;

pub struct ProviderListProvider {
    set_search: SetSearchText,
}

impl ProviderListProvider {
    /// Construct with a callback that swaps the launcher's
    /// search text. The UI typically wires this to a
    /// `gtk::Entry::set_text` call.
    pub fn new(set_search: SetSearchText) -> Self {
        Self { set_search }
    }
}

impl Provider for ProviderListProvider {
    fn name(&self) -> &str {
        "Providers"
    }

    fn handles_search(&self) -> bool {
        false
    }

    fn handles_command(&self, query: &str) -> bool {
        query.trim_start().starts_with(';')
    }

    fn commands(&self) -> Vec<LauncherItem> {
        vec![LauncherItem {
            id: "providers:palette".into(),
            name: "; (provider cheatsheet)".into(),
            description: "Browse every launcher prefix".into(),
            icon: "help-browser-symbolic".into(),
            icon_is_path: false,
            score: 0.0,
            provider_name: "Providers".into(),
            usage_key: None,
            on_activate: Rc::new(|| {}),
        }]
    }

    fn search(&self, query: &str) -> Vec<LauncherItem> {
        if !query.trim_start().starts_with(';') {
            return Vec::new();
        }
        let filter = query
            .trim_start()
            .trim_start_matches(';')
            .trim()
            .to_ascii_lowercase();

        ENTRIES
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                filter.is_empty()
                    || e.description.to_ascii_lowercase().contains(&filter)
                    || e.prefix.to_ascii_lowercase().contains(&filter)
            })
            .map(|(idx, entry)| {
                let setter = self.set_search.clone();
                let target = entry.example.to_string();
                LauncherItem {
                    id: format!("providers:{idx}"),
                    name: if entry.prefix.is_empty() {
                        format!("(no prefix)  e.g. `{}`", entry.example)
                    } else {
                        format!("{}  e.g. `{}`", entry.prefix, entry.example)
                    },
                    description: entry.description.into(),
                    icon: entry.icon.into(),
                    icon_is_path: false,
                    // Stable order — the catalogue's hand-curated
                    // sequence is the right ordering for a
                    // cheatsheet.
                    score: 200.0 - idx as f64,
                    provider_name: "Providers".into(),
                    usage_key: None,
                    on_activate: Rc::new(move || setter(&target)),
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn make(captured: Rc<RefCell<String>>) -> ProviderListProvider {
        let c = captured.clone();
        ProviderListProvider::new(Rc::new(move |s: &str| {
            *c.borrow_mut() = s.to_string();
        }))
    }

    #[test]
    fn semicolon_lists_every_entry() {
        let captured = Rc::new(RefCell::new(String::new()));
        let p = make(captured);
        let items = p.search(";");
        assert_eq!(items.len(), ENTRIES.len());
    }

    #[test]
    fn does_not_handle_regular_search() {
        let captured = Rc::new(RefCell::new(String::new()));
        let p = make(captured);
        assert!(p.search("firefox").is_empty());
    }

    #[test]
    fn activation_writes_example_to_search() {
        let captured = Rc::new(RefCell::new(String::new()));
        let p = make(captured.clone());
        let items = p.search(";");
        let google = items.iter().find(|i| i.name.starts_with("g <query>")).unwrap();
        (google.on_activate)();
        assert_eq!(*captured.borrow(), "g rust async");
    }

    #[test]
    fn filter_narrows_results() {
        let captured = Rc::new(RefCell::new(String::new()));
        let p = make(captured);
        let items = p.search(";emoji");
        assert!(items.iter().all(|i| i.description.to_lowercase().contains("emoji")));
    }
}
