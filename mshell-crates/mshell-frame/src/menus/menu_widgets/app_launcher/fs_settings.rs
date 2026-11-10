//! `launcher.file_search` ↔ the `fs` provider's settings. The provider
//! re-reads them on every query and writes the launcher-side changes
//! (scope, USB toggle, added folders) back through the config.

use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    ConfigStoreFields, FileSearch, FileSearchScope, FileSearchStoreFields, LauncherStoreFields,
};
use mshell_launcher::providers::file_search::{FsSettings, SearchScope};
use reactive_graph::traits::GetUntracked;

fn to_launcher(scope: FileSearchScope) -> SearchScope {
    match scope {
        FileSearchScope::Home => SearchScope::Home,
        FileSearchScope::HomeAndFolders => SearchScope::HomeAndFolders,
        FileSearchScope::System => SearchScope::System,
    }
}

fn to_config(scope: SearchScope) -> FileSearchScope {
    match scope {
        SearchScope::Home => FileSearchScope::Home,
        SearchScope::HomeAndFolders => FileSearchScope::HomeAndFolders,
        SearchScope::System => FileSearchScope::System,
    }
}

/// The live `launcher.file_search` settings.
pub(super) fn read() -> FsSettings {
    let fs = config_manager().config().launcher().file_search();
    FsSettings {
        scope: to_launcher(fs.clone().scope().get_untracked()),
        folders: fs.clone().folders().get_untracked(),
        include_usb: fs.include_usb().get_untracked(),
    }
}

/// Write `s` to `launcher.file_search`.
pub(super) fn save(s: FsSettings) {
    let value = FileSearch {
        scope: to_config(s.scope),
        folders: s.folders,
        include_usb: s.include_usb,
    };
    config_manager().update_config(move |c| c.launcher.file_search = value);
}
