//! The command palette: one fuzzy-searchable list of everything the TUI can
//! do from anywhere.
//!
//! mdots exposes 27 CLI command modules and a growing pile of screens; the
//! sidebar only reaches the screens, and only by name. The palette is the
//! keyboard route to all of it — `Ctrl+P`, type a few letters, Enter.
//!
//! Entries are built from the live sidebar list rather than a second const
//! table (see `App::palette_entries`), so adding a screen adds its palette
//! entry for free and the two can't drift.

/// What activating a palette entry does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PaletteCommand {
    /// Navigate to the screen at this sidebar index.
    Navigate(usize),
    /// Open the read-only doctor health-check overlay.
    Doctor,
    /// Open the keybinding help overlay.
    Help,
    /// Reload the active screen's data.
    RefreshScreen,
    /// Leave the TUI.
    Quit,
}

/// One row in the palette.
#[derive(Clone, Debug)]
pub struct PaletteEntry {
    /// What the user reads and what the query matches against.
    pub label: String,
    /// Right-aligned hint — usually the equivalent direct keybinding.
    pub hint: &'static str,
    pub command: PaletteCommand,
}

/// Score `needle` against `haystack` as a fuzzy subsequence match, higher is
/// better. `None` when `haystack` doesn't contain every character of
/// `needle` in order.
///
/// The weighting favours what people actually type: characters that land on
/// word starts ("gs" → **G**o to **S**ync) and runs of consecutive
/// characters both beat the same letters scattered through the string.
pub fn fuzzy_score(haystack: &str, needle: &str) -> Option<i32> {
    if needle.is_empty() {
        return Some(0);
    }
    let hay: Vec<char> = haystack.to_lowercase().chars().collect();
    let pat: Vec<char> = needle.to_lowercase().chars().collect();

    let mut score = 0;
    let mut pat_idx = 0;
    let mut last_match: Option<usize> = None;

    for (i, &c) in hay.iter().enumerate() {
        if pat_idx >= pat.len() {
            break;
        }
        if c != pat[pat_idx] {
            continue;
        }
        score += 10;
        // Start of the string or just after a separator: a word boundary.
        let at_word_start = i == 0
            || hay
                .get(i.wrapping_sub(1))
                .is_some_and(|p| matches!(p, ' ' | '-' | '_' | '/' | '.'));
        if at_word_start {
            score += 8;
        }
        if last_match == Some(i.wrapping_sub(1)) {
            score += 5;
        }
        last_match = Some(i);
        pat_idx += 1;
    }

    (pat_idx == pat.len()).then_some(score)
}

/// Indices of `entries` matching `query`, best score first. Ties keep the
/// original order, so an empty query lists everything as declared.
pub fn filter_entries(entries: &[PaletteEntry], query: &str) -> Vec<usize> {
    let mut scored: Vec<(usize, i32)> = entries
        .iter()
        .enumerate()
        .filter_map(|(i, e)| fuzzy_score(&e.label, query).map(|s| (i, s)))
        .collect();
    // The sort is stable, so equal scores keep declaration order.
    scored.sort_by_key(|&(_, score)| std::cmp::Reverse(score));
    scored.into_iter().map(|(i, _)| i).collect()
}

/// Open-palette state: the query being typed and where the highlight sits.
pub struct PaletteState {
    pub query: String,
    /// All entries, rebuilt when the palette opens.
    pub entries: Vec<PaletteEntry>,
    /// Indices into `entries` that survive the current query, best first.
    pub matches: Vec<usize>,
    /// Index *into `matches`* of the highlighted row.
    pub selected: usize,
}

impl PaletteState {
    pub fn new(entries: Vec<PaletteEntry>) -> Self {
        let matches = filter_entries(&entries, "");
        Self {
            query: String::new(),
            entries,
            matches,
            selected: 0,
        }
    }

    /// Re-run the filter after the query changed, pinning the highlight to
    /// the best match (the top row) as is conventional for a palette.
    fn refilter(&mut self) {
        self.matches = filter_entries(&self.entries, &self.query);
        self.selected = 0;
    }

    pub fn push_char(&mut self, c: char) {
        self.query.push(c);
        self.refilter();
    }

    pub fn pop_char(&mut self) {
        self.query.pop();
        self.refilter();
    }

    pub fn select_next(&mut self) {
        if self.matches.is_empty() {
            return;
        }
        self.selected = (self.selected + 1) % self.matches.len();
    }

    pub fn select_prev(&mut self) {
        if self.matches.is_empty() {
            return;
        }
        self.selected = self
            .selected
            .checked_sub(1)
            .unwrap_or(self.matches.len() - 1);
    }

    /// The highlighted entry, if the query matched anything at all.
    pub fn selected_entry(&self) -> Option<&PaletteEntry> {
        self.entries.get(*self.matches.get(self.selected)?)
    }

    /// Entries to draw, in match order, paired with whether each is the
    /// highlighted one.
    pub fn visible_entries(&self) -> impl Iterator<Item = (&PaletteEntry, bool)> {
        self.matches
            .iter()
            .enumerate()
            .filter_map(move |(row, &idx)| self.entries.get(idx).map(|e| (e, row == self.selected)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(label: &str, command: PaletteCommand) -> PaletteEntry {
        PaletteEntry {
            label: label.to_string(),
            hint: "",
            command,
        }
    }

    fn sample() -> Vec<PaletteEntry> {
        vec![
            entry("Go to Overview", PaletteCommand::Navigate(0)),
            entry("Go to Modules", PaletteCommand::Navigate(1)),
            entry("Go to Sync", PaletteCommand::Navigate(3)),
            entry("Run health check (doctor)", PaletteCommand::Doctor),
            entry("Quit", PaletteCommand::Quit),
        ]
    }

    #[test]
    fn empty_query_matches_everything_in_declaration_order() {
        let entries = sample();
        assert_eq!(filter_entries(&entries, ""), vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn non_subsequence_does_not_match() {
        assert_eq!(fuzzy_score("Go to Sync", "zzz"), None);
    }

    #[test]
    fn out_of_order_characters_do_not_match() {
        assert_eq!(fuzzy_score("sync", "nys"), None);
    }

    #[test]
    fn matching_is_case_insensitive() {
        assert!(fuzzy_score("Go to Sync", "SYNC").is_some());
        assert!(fuzzy_score("GO TO SYNC", "sync").is_some());
    }

    #[test]
    fn word_start_initials_outrank_scattered_letters() {
        // "gs" as the initials of "Go ... Sync" must beat the same two
        // letters buried mid-word.
        let initials = fuzzy_score("Go to Sync", "gs").unwrap();
        let scattered = fuzzy_score("baggages", "gs").unwrap();
        assert!(
            initials > scattered,
            "initials {initials} should beat scattered {scattered}"
        );
    }

    #[test]
    fn consecutive_run_outranks_scattered_match() {
        let consecutive = fuzzy_score("sync now", "sync").unwrap();
        let scattered = fuzzy_score("xsxyxnxc", "sync").unwrap();
        assert!(
            consecutive > scattered,
            "consecutive {consecutive} should beat scattered {scattered}"
        );
    }

    /// Deliberate ranking choice, documented by test: a character landing on
    /// a word boundary is worth more than one continuing a run, so typing
    /// initials finds the thing you meant. Same order fzf-style finders use.
    #[test]
    fn word_start_bonus_outweighs_the_consecutive_bonus() {
        let all_word_starts = fuzzy_score("s y n c", "sync").unwrap();
        let one_run = fuzzy_score("sync now", "sync").unwrap();
        assert!(all_word_starts > one_run);
    }

    #[test]
    fn filter_puts_the_best_match_first() {
        let entries = sample();
        let matches = filter_entries(&entries, "sync");
        assert_eq!(entries[matches[0]].command, PaletteCommand::Navigate(3));
    }

    #[test]
    fn typing_narrows_and_resets_the_highlight_to_the_best_match() {
        let mut state = PaletteState::new(sample());
        state.select_next();
        assert_eq!(state.selected, 1);
        state.push_char('s');
        state.push_char('y');
        assert_eq!(state.selected, 0, "highlight snaps back to the top match");
        assert_eq!(
            state.selected_entry().map(|e| e.command),
            Some(PaletteCommand::Navigate(3))
        );
    }

    #[test]
    fn backspace_widens_the_match_set_again() {
        let mut state = PaletteState::new(sample());
        state.push_char('q');
        let narrowed = state.matches.len();
        state.pop_char();
        assert!(state.matches.len() > narrowed);
        assert_eq!(state.matches.len(), sample().len());
    }

    #[test]
    fn selection_wraps_in_both_directions() {
        let mut state = PaletteState::new(sample());
        state.select_prev();
        assert_eq!(state.selected, sample().len() - 1);
        state.select_next();
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn no_matches_yields_no_selected_entry_and_ignores_navigation() {
        let mut state = PaletteState::new(sample());
        for c in "zzzz".chars() {
            state.push_char(c);
        }
        assert!(state.matches.is_empty());
        assert!(state.selected_entry().is_none());
        // Must not panic or move the highlight off the end.
        state.select_next();
        state.select_prev();
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn visible_entries_flags_exactly_one_row_as_selected() {
        let state = PaletteState::new(sample());
        let flagged: Vec<bool> = state.visible_entries().map(|(_, sel)| sel).collect();
        assert_eq!(flagged.iter().filter(|&&s| s).count(), 1);
        assert!(flagged[0], "the top row starts highlighted");
    }
}
