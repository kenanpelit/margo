//! Pure state for directory searches; late responses never replace a newer query.
pub(super) type Station = (String, String, String, String);

#[derive(Debug, Default)]
pub(super) struct RadioState {
    pub generation: u64,
    pub loading: bool,
    pub searched: bool,
    pub results: Vec<Station>,
    pub error: String,
}

impl RadioState {
    pub fn begin(&mut self) -> u64 {
        self.generation += 1;
        self.loading = true;
        self.searched = true;
        self.error.clear();
        self.generation
    }
    pub fn complete(&mut self, generation: u64, result: Result<Vec<Station>, String>) {
        if generation != self.generation {
            return;
        }
        self.loading = false;
        match result {
            Ok(results) => {
                self.results = results;
                self.error.clear();
            }
            Err(error) => self.error = error,
        }
    }
}

pub(super) fn station_current(live: bool, current: &str, station: &Station) -> bool {
    live && current == station.1
}

#[cfg(test)]
mod tests {
    use super::*;
    fn station() -> Station {
        (
            "key".into(),
            "Radio One".into(),
            "https://radio.test/live".into(),
            "Jazz · TR".into(),
        )
    }
    #[test]
    fn new_search_has_immediate_loading_feedback() {
        let mut state = RadioState::default();
        state.begin();
        assert!(state.loading);
        assert!(state.searched);
    }
    #[test]
    fn stale_search_cannot_replace_latest_results_or_loading() {
        let mut state = RadioState::default();
        let old = state.begin();
        let latest = state.begin();
        state.complete(old, Ok(vec![station()]));
        assert!(state.results.is_empty());
        assert!(state.loading);
        state.complete(latest, Ok(vec![station()]));
        assert_eq!(state.results.len(), 1);
        assert!(!state.loading);
    }
    #[test]
    fn failed_search_is_explicit_and_keeps_previous_results() {
        let mut state = RadioState::default();
        let id = state.begin();
        state.complete(id, Ok(vec![station()]));
        let id = state.begin();
        state.complete(id, Err("Radio requires a newer Tune".into()));
        assert!(!state.loading);
        assert!(state.error.contains("newer Tune"));
        assert_eq!(state.results.len(), 1);
    }
    #[test]
    fn current_station_requires_live_mode() {
        assert!(station_current(true, "Radio One", &station()));
        assert!(!station_current(false, "Radio One", &station()));
        assert!(!station_current(true, "Other", &station()));
    }
}
