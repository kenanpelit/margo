//! Fuzzy scoring helpers used by every provider.
//!
//! Backed by [`nucleo_matcher`] — the same crate Helix uses for its
//! pickers, so behaviour matches what users get in any modern Rust
//! TUI. Scores are returned as raw `f64` values (nucleo's native
//! scoring scale) — typically in `0..200` for short queries. We do
//! not normalise per-query because nucleo's gap penalties and
//! prefix/boundary bonuses already produce a meaningful ordering
//! that normalisation would flatten.
//!
//! On top of fuzzy score we apply a **usage boost** sourced from
//! the on-disk [`FrecencyStore`]: `5.0 * log2(1 + count)`. With
//! that multiplier 100 uses adds ~33 to the score — a strong nudge
//! that can break a near-tie but never enough to flip a clearly
//! better fuzzy match.
//!
//! [`FrecencyStore`]: crate::frecency::FrecencyStore

use nucleo_matcher::{
    Config, Matcher, Utf32Str,
    pattern::{CaseMatching, Normalization, Pattern},
};

/// Compute a fuzzy score for `query` matching `haystack`. Returns
/// `None` when nucleo found no match. The raw score scale runs
/// roughly `0..200` for short queries — bigger means better.
///
/// The matcher is passed in (rather than constructed per call)
/// because nucleo amortises its internal buffers across calls —
/// reusing one matcher across an entire keystroke is meaningfully
/// faster than allocating per item.
pub fn fuzzy_score(matcher: &mut Matcher, query: &str, haystack: &str) -> Option<f64> {
    if query.is_empty() {
        return Some(0.0);
    }

    let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);

    let mut buf = Vec::new();
    let haystack_u32 = Utf32Str::new(haystack, &mut buf);

    let raw = pattern.score(haystack_u32, matcher)?;
    Some(raw as f64)
}

/// Frecency boost added on top of a fuzzy score. The `5.0`
/// multiplier puts a single use at ~5, ten uses at ~17, a
/// hundred uses at ~33 — comparable to a typical nucleo gap
/// penalty so it can break near-ties but never overrides a clearly
/// stronger match.
pub fn usage_boost(count: u64) -> f64 {
    5.0 * ((1.0 + count as f64).log2())
}

/// Construct a matcher with the same config the runtime uses, so
/// providers writing one-off scoring loops don't drift from the
/// runtime's defaults.
pub fn make_matcher() -> Matcher {
    Matcher::new(Config::DEFAULT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_returns_zero_not_none() {
        let mut m = make_matcher();
        assert_eq!(fuzzy_score(&mut m, "", "firefox"), Some(0.0));
    }

    #[test]
    fn perfect_match_outscores_gappy() {
        let mut m = make_matcher();
        // Same query, two haystacks: one with the chars in a
        // tight prefix, one with gaps between every char. Nucleo's
        // gap penalty should make the prefix outscore the gappy
        // version.
        let prefix = fuzzy_score(&mut m, "abc", "abcdef").unwrap();
        let gappy = fuzzy_score(&mut m, "abc", "aXbXcdef").unwrap();
        assert!(prefix > gappy, "prefix={prefix} gappy={gappy}");
    }

    #[test]
    fn nonmatching_query_returns_none() {
        let mut m = make_matcher();
        assert!(fuzzy_score(&mut m, "xyzzzz", "firefox").is_none());
    }

    #[test]
    fn usage_boost_monotonic() {
        assert!(usage_boost(0) < usage_boost(1));
        assert!(usage_boost(1) < usage_boost(10));
        assert!(usage_boost(10) < usage_boost(100));
    }

    #[test]
    fn usage_boost_does_not_dominate_perfect_match() {
        // 1000 uses on the wrong app should not beat a perfect
        // match on the right one. With 5.0 * log2(1001) ~ 50, a
        // perfect 3-char nucleo match (~80+) still wins.
        let mut m = make_matcher();
        let perfect = fuzzy_score(&mut m, "vim", "vim").unwrap();
        assert!(perfect > usage_boost(1000), "perfect={perfect} boost={}", usage_boost(1000));
    }
}
