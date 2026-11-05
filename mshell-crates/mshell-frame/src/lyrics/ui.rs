//! GTK-side helpers shared by the lyrics bar pill and the lyrics menu: the
//! live source options / offset from config, and the karaoke rendering of a
//! word-timed line. The engine in [`super`] stays GTK-free.

use super::{FetchOptions, LyricLine, lrc, player_allowed};
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    BarWidgetsStoreFields, BarsStoreFields, ConfigStoreFields, LyricsBarWidgetStoreFields,
};
use mshell_services::media_service;
use reactive_graph::traits::GetUntracked;
use relm4::gtk::pango;
use std::sync::Arc;
use std::time::Duration;
use wayle_media::core::player::Player;
use wayle_media::types::PlaybackState;

/// Repaint cadence while a word-timed line is being sung. Only runs while the
/// surface shows such a line *and* the player is playing.
pub(crate) const KARAOKE_TICK: Duration = Duration::from_millis(80);

/// Alpha (0..=65535) of the not-yet-sung part of a karaoke line: the line
/// keeps its themed colour, the unsung words just recede.
const UNSUNG_ALPHA: u16 = 0x7333; // ≈ 45 %

/// The source chain the user configured (read at fetch time).
pub(crate) fn fetch_options() -> FetchOptions {
    let lyr = || config_manager().config().bars().widgets().lyrics();
    FetchOptions::new(
        lyr().netease_fallback().get_untracked(),
        &lyr().local_dir().get_untracked(),
    )
}

/// The player whose track drives the lyrics: among the players the
/// allow/block lists admit, the first actually playing, else wayle's active
/// player, else the first. (Matches the media pill when no list is set.)
pub(crate) fn display_player() -> Option<Arc<Player>> {
    let lyr = || config_manager().config().bars().widgets().lyrics();
    let allow = lyr().player_allowlist().get_untracked();
    let block = lyr().player_blocklist().get_untracked();
    let ok = |p: &Arc<Player>| player_allowed(&p.identity.get(), p.id.bus_name(), &allow, &block);
    let svc = media_service();
    let players: Vec<Arc<Player>> = svc
        .as_ref()
        .map(|s| s.player_list.get())
        .unwrap_or_default()
        .into_iter()
        .filter(ok)
        .collect();
    players
        .iter()
        .find(|p| p.playback_state.get() == PlaybackState::Playing)
        .cloned()
        .or_else(|| svc.as_ref().and_then(|s| s.active_player.get()).filter(ok))
        .or_else(|| players.first().cloned())
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
