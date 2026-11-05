//! GTK-side helpers shared by the lyrics bar pill and the lyrics menu: the
//! live source options / offset from config, and the karaoke rendering of a
//! word-timed line. The engine in [`super`] stays GTK-free.

use super::{FetchOptions, LyricLine, lrc};
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    BarWidgetsStoreFields, BarsStoreFields, ConfigStoreFields, LyricsBarWidgetStoreFields,
};
use reactive_graph::traits::GetUntracked;
use relm4::gtk::pango;
use std::time::Duration;

/// Repaint cadence while a word-timed line is being sung. Only runs while the
/// surface shows such a line *and* the player is playing.
pub(crate) const KARAOKE_TICK: Duration = Duration::from_millis(80);

/// Alpha (0..=65535) of the not-yet-sung part of a karaoke line: the line
/// keeps its themed colour, the unsung words just recede.
const UNSUNG_ALPHA: u16 = 0x7333; // ≈ 45 %

/// The source chain the user configured (read at fetch time).
pub(crate) fn fetch_options() -> FetchOptions {
    FetchOptions {
        netease: config_manager()
            .config()
            .bars()
            .widgets()
            .lyrics()
            .netease_fallback()
            .get_untracked(),
    }
}

/// Pango attributes for `line` at `position_ms`: the sung prefix at full
/// opacity, the rest dimmed. Empty for a line without word timings.
pub(crate) fn karaoke_attrs(line: &LyricLine, position_ms: u64) -> pango::AttrList {
    let list = pango::AttrList::new();
    if line.words.is_empty() {
        return list;
    }
    let split = lrc::karaoke_split(line, position_ms);
    if split < line.text.len() {
        let mut attr = pango::AttrInt::new_foreground_alpha(UNSUNG_ALPHA);
        attr.set_start_index(split as u32);
        attr.set_end_index(line.text.len() as u32);
        list.insert(attr);
    }
    list
}
