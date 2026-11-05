//! Lyric text formats → [`LyricLine`]s.
//!
//! - **LRC** (`[mm:ss.xx]line`), including repeated stamps
//!   (`[00:12.00][01:45.00]chorus`), the `[offset:±ms]` header and the
//!   *enhanced* (A2) word tags `[00:12.00]<00:12.00>Word <00:12.40>word`.
//! - **YRC** — NetEase's word-timed format:
//!   `[start,duration](word_start,word_duration,0)Word(…)word`, absolute ms.
//!
//! Plus the helpers built on the parsed lines: pairing a translation /
//! romanization layer with the original lines, the karaoke split point for
//! a playback position, and the NetEase credit / placeholder filters. All
//! pure string work, cheap enough for any thread.

use super::{LyricLine, Word};

/// How far (ms) a translation line's stamp may sit from the original line it
/// belongs to. NetEase's `tlyric` is timed against `lrc`, its `ytlrc` against
/// `yrc`; both land within a few hundred ms of the line they translate.
const LAYER_TOLERANCE_MS: u64 = 500;

/// Parse an LRC body into sorted, timestamped lines. Skips metadata tags
/// (`[ar:…]`, `[length:…]`) — anything whose bracket isn't a real `mm:ss` —
/// and applies the `[offset:]` header. Enhanced word tags become
/// [`LyricLine::words`] when they are complete and in order; otherwise the
/// line falls back to line mode with the tags stripped from its text.
pub(crate) fn parse_lrc(raw: &str) -> Vec<LyricLine> {
    let offset = header_offset(raw);
    let mut lines = Vec::new();
    for raw_line in raw.lines() {
        let mut rest = raw_line.trim_start();
        let mut stamps = Vec::new();
        while rest.starts_with('[') {
            let Some(close) = rest.find(']') else { break };
            if let Some(ms) = parse_timestamp(&rest[1..close]) {
                stamps.push(apply_offset(ms, offset));
            }
            rest = &rest[close + 1..];
        }
        if stamps.is_empty() {
            continue;
        }
        let (text, words) = parse_enhanced_body(rest, offset);
        // Word stamps are absolute, so they only make sense for a line that is
        // sung once; a repeated chorus line keeps line mode.
        let words = if stamps.len() == 1 { words } else { Vec::new() };
        for ms in stamps {
            lines.push(LyricLine {
                time_ms: ms,
                text: text.clone(),
                words: words.clone(),
                ..LyricLine::default()
            });
        }
    }
    lines.sort_by_key(|l| l.time_ms);
    close_open_words(&mut lines);
    lines
}

/// The `[offset:±N]` header, in ms (0 when absent or malformed).
fn header_offset(raw: &str) -> i64 {
    for line in raw.lines() {
        let l = line.trim();
        let Some(body) = l.strip_prefix('[').and_then(|b| b.strip_suffix(']')) else {
            continue;
        };
        let Some((tag, value)) = body.split_once(':') else {
            continue;
        };
        if tag.trim().eq_ignore_ascii_case("offset") {
            return value.trim().trim_start_matches('+').parse().unwrap_or(0);
        }
    }
    0
}

/// Apply an LRC `[offset:]` value. Per the LRC convention a *positive* offset
/// makes the lyrics appear sooner, so it is subtracted from every stamp.
fn apply_offset(ms: u64, offset: i64) -> u64 {
    if offset >= 0 {
        ms.saturating_sub(offset as u64)
    } else {
        ms.saturating_add(offset.unsigned_abs())
    }
}

/// Split an LRC line body into its display text and (when present and
/// well-formed) enhanced word timings. A word's end is the next word's start;
/// a trailing empty tag (`…word <00:14.00>`) closes the last word, otherwise
/// [`close_open_words`] closes it at the next line.
fn parse_enhanced_body(body: &str, offset: i64) -> (String, Vec<Word>) {
    let mut words: Vec<Word> = Vec::new();
    let mut lead = String::new();
    let mut rest = body;
    let mut tail_end: Option<u64> = None;
    let mut any_tag = false;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix('<')
            && let Some(close) = after.find('>')
            && let Some(ms) = parse_timestamp(&after[..close])
        {
            any_tag = true;
            let ms = apply_offset(ms, offset);
            rest = &after[close + 1..];
            let next = rest.find('<').unwrap_or(rest.len());
            let text = &rest[..next];
            rest = &rest[next..];
            if let Some(prev) = words.last_mut()
                && prev.end_ms == u64::MAX
            {
                prev.end_ms = ms;
            }
            if text.is_empty() {
                tail_end = Some(ms);
            } else {
                words.push(Word {
                    start_ms: ms,
                    end_ms: u64::MAX,
                    text: text.to_string(),
                });
            }
            continue;
        }
        // Text before the first tag (or a stray '<'): plain content.
        let mut chars = rest.chars();
        if let Some(c) = chars.next() {
            if words.is_empty() {
                lead.push(c);
            } else if let Some(last) = words.last_mut() {
                last.text.push(c);
            }
        }
        rest = chars.as_str();
    }

    if !any_tag {
        return (body.trim().to_string(), Vec::new());
    }
    if let (Some(end), Some(last)) = (tail_end, words.last_mut())
        && last.end_ms == u64::MAX
    {
        last.end_ms = end;
    }
    let plain: String = lead.clone() + &words.iter().map(|w| w.text.as_str()).collect::<String>();
    // Untimed leading text can't be highlighted word-by-word, and neither can
    // words that run backwards: keep the text, drop the timing.
    if !lead.trim().is_empty() || !words_in_order(&words) {
        return (plain.trim().to_string(), Vec::new());
    }
    trim_words(&mut words);
    let text = words.iter().map(|w| w.text.as_str()).collect::<String>();
    if text.is_empty() {
        return (String::new(), Vec::new());
    }
    (text, words)
}

/// Parse NetEase YRC into sorted lines with word timings. JSON credit rows
/// (`{"t":0,"c":[…]}`) and anything that isn't a `[start,duration]` row are
/// skipped.
pub(crate) fn parse_yrc(raw: &str) -> Vec<LyricLine> {
    let mut lines = Vec::new();
    for raw_line in raw.lines() {
        let l = raw_line.trim();
        let Some(body) = l.strip_prefix('[') else {
            continue;
        };
        let Some(close) = body.find(']') else {
            continue;
        };
        let Some((start, _dur)) = body[..close].split_once(',') else {
            continue;
        };
        let Ok(start) = start.trim().parse::<u64>() else {
            continue;
        };
        let mut words = Vec::new();
        let mut rest = &body[close + 1..];
        while let Some(after) = rest.strip_prefix('(') {
            let Some(close) = after.find(')') else { break };
            let mut parts = after[..close].split(',');
            let ws = parts.next().and_then(|p| p.trim().parse::<u64>().ok());
            let wd = parts.next().and_then(|p| p.trim().parse::<u64>().ok());
            rest = &after[close + 1..];
            let next = rest.find('(').unwrap_or(rest.len());
            let text = &rest[..next];
            rest = &rest[next..];
            if let (Some(ws), Some(wd)) = (ws, wd)
                && !text.is_empty()
            {
                words.push(Word {
                    start_ms: ws,
                    end_ms: ws + wd,
                    text: text.to_string(),
                });
            }
        }
        if !words_in_order(&words) {
            let text: String = words.iter().map(|w| w.text.as_str()).collect();
            words.clear();
            if !text.trim().is_empty() {
                lines.push(LyricLine {
                    time_ms: start,
                    text: text.trim().to_string(),
                    ..LyricLine::default()
                });
            }
            continue;
        }
        trim_words(&mut words);
        let text: String = words.iter().map(|w| w.text.as_str()).collect();
        if text.is_empty() {
            continue;
        }
        lines.push(LyricLine {
            time_ms: start,
            text,
            words,
            ..LyricLine::default()
        });
    }
    lines.sort_by_key(|l| l.time_ms);
    lines
}

fn words_in_order(words: &[Word]) -> bool {
    words.windows(2).all(|w| w[0].start_ms <= w[1].start_ms)
}

/// Trim the outer whitespace of a word list so `text == concat(words)` holds
/// with no leading/trailing blanks — the karaoke split indexes that string.
fn trim_words(words: &mut Vec<Word>) {
    if let Some(first) = words.first_mut() {
        first.text = first.text.trim_start().to_string();
    }
    if let Some(last) = words.last_mut() {
        last.text = last.text.trim_end().to_string();
    }
    words.retain(|w| !w.text.is_empty());
}

/// Close any word still open (`end_ms == u64::MAX`) at the next line's start,
/// or a second after it began for the final line.
fn close_open_words(lines: &mut [LyricLine]) {
    let starts: Vec<u64> = lines.iter().map(|l| l.time_ms).collect();
    for (i, line) in lines.iter_mut().enumerate() {
        let next = starts.get(i + 1).copied();
        if let Some(last) = line.words.last_mut()
            && last.end_ms == u64::MAX
        {
            last.end_ms = match next {
                Some(n) if n > last.start_ms => n,
                _ => last.start_ms + 1_000,
            };
        }
    }
}

/// Parse an LRC timestamp tag (`mm:ss`, `mm:ss.xx`, `mm:ss.xxx`) to ms.
/// Returns `None` for metadata tags so they're dropped by the caller.
pub(crate) fn parse_timestamp(tag: &str) -> Option<u64> {
    let (mm, rest) = tag.split_once(':')?;
    let minutes: u64 = mm.trim().parse().ok()?;
    let (ss, frac) = rest.split_once('.').unwrap_or((rest, ""));
    let seconds: u64 = ss.trim().parse().ok()?;
    let frac_ms = if frac.is_empty() {
        0
    } else {
        let mut f = frac.trim().to_string();
        if !f.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        while f.len() < 3 {
            f.push('0');
        }
        f.truncate(3);
        f.parse::<u64>().ok()?
    };
    Some((minutes * 60 + seconds) * 1000 + frac_ms)
}

/// Which secondary layer [`merge_layer`] fills.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Layer {
    Translation,
    Romanization,
}

/// Pair a timed secondary layer (translation or romanization) with the
/// original lines: each layer line goes to the original line whose stamp is
/// nearest, within [`LAYER_TOLERANCE_MS`]. A layer line that only repeats the
/// original (NetEase does that for already-Latin lines) is dropped.
pub(crate) fn merge_layer(lines: &mut [LyricLine], layer: &[LyricLine], kind: Layer) {
    if lines.is_empty() {
        return;
    }
    for item in layer {
        let text = item.text.trim();
        if text.is_empty() {
            continue;
        }
        let idx = match lines.binary_search_by_key(&item.time_ms, |l| l.time_ms) {
            Ok(i) => i,
            Err(i) => {
                let before = i.checked_sub(1);
                let after = (i < lines.len()).then_some(i);
                match (before, after) {
                    (Some(b), Some(a)) => {
                        if item.time_ms - lines[b].time_ms <= lines[a].time_ms - item.time_ms {
                            b
                        } else {
                            a
                        }
                    }
                    (Some(b), None) => b,
                    (None, Some(a)) => a,
                    (None, None) => continue,
                }
            }
        };
        let line = &mut lines[idx];
        if line.time_ms.abs_diff(item.time_ms) > LAYER_TOLERANCE_MS
            || line.text.trim().eq_ignore_ascii_case(text)
        {
            continue;
        }
        let slot = match kind {
            Layer::Translation => &mut line.translation,
            Layer::Romanization => &mut line.romanization,
        };
        if slot.is_none() {
            *slot = Some(text.to_string());
        }
    }
}

/// Byte offset into `line.text` up to which the line has been sung at
/// `position_ms`: whole words that have ended, plus the started share of the
/// current word (by characters, so it always lands on a char boundary). `0`
/// for a line without word timings or before its first word.
pub(crate) fn karaoke_split(line: &LyricLine, position_ms: u64) -> usize {
    let mut offset = 0;
    for word in &line.words {
        if position_ms >= word.end_ms {
            offset += word.text.len();
            continue;
        }
        if position_ms > word.start_ms {
            let span = word.end_ms.saturating_sub(word.start_ms).max(1);
            let chars = word.text.chars().count() as u64;
            let sung = ((position_ms - word.start_ms) * chars / span) as usize;
            offset += word
                .text
                .char_indices()
                .nth(sung)
                .map(|(b, _)| b)
                .unwrap_or(word.text.len());
        }
        break;
    }
    offset.min(line.text.len())
}

/// A NetEase credit row (`作词 : …`, `编曲：…`, `吉他：…`, `Lyricist: …`):
/// a short label before a colon, the label all CJK or a known English
/// credit word. NetEase mixes these into the timed lyrics; lrclib does not,
/// so this filter is only applied to NetEase payloads.
pub(crate) fn is_credit_line(text: &str) -> bool {
    const EN: &[&str] = &[
        "lyrics",
        "lyricist",
        "composer",
        "composed by",
        "written by",
        "arranger",
        "arranged by",
        "producer",
        "produced by",
        "mixed by",
        "mastered by",
    ];
    let t = text.trim();
    let Some(idx) = t.find([':', '：']) else {
        return false;
    };
    let label = t[..idx].trim();
    if label.is_empty() {
        return false;
    }
    let cjk = label.chars().count() <= 8 && label.chars().all(is_cjk);
    cjk || EN.contains(&label.to_lowercase().as_str())
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF)
}

/// NetEase's "no lyrics" stand-ins (`纯音乐，请欣赏` — "instrumental, please
/// enjoy", `暂无歌词` — "no lyrics yet"): the whole body is one such phrase.
pub(crate) fn is_placeholder(lines: &[LyricLine]) -> bool {
    let joined: String = lines
        .iter()
        .flat_map(|l| l.text.chars())
        .filter(|c| !c.is_whitespace() && !c.is_ascii_punctuation() && !"，。！？．".contains(*c))
        .collect();
    if joined.is_empty() {
        return false;
    }
    joined == "暂无歌词" || (joined.chars().count() <= 16 && joined.contains("纯音乐"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(time_ms: u64, text: &str) -> LyricLine {
        LyricLine {
            time_ms,
            text: text.to_string(),
            ..LyricLine::default()
        }
    }

    #[test]
    fn parses_synced_lines_in_order() {
        let lines = parse_lrc("[00:12.50]second\n[00:01.00]first\n[ar:Someone]\n[00:30.00]third");
        assert_eq!(lines.len(), 3);
        assert_eq!((lines[0].time_ms, lines[0].text.as_str()), (1000, "first"));
        assert_eq!(lines[1].time_ms, 12_500);
        assert_eq!(lines[2].time_ms, 30_000);
        assert!(lines.iter().all(|l| l.words.is_empty()));
    }

    #[test]
    fn parses_repeated_timestamps() {
        let lines = parse_lrc("[00:10.00][01:10.00]chorus");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1].time_ms, 70_000);
        assert_eq!(lines[1].text, "chorus");
    }

    #[test]
    fn skips_metadata_only_lines() {
        assert!(parse_lrc("[ar:Artist]\n[ti:Title]\n[length:03:00]").is_empty());
    }

    #[test]
    fn offset_header_shifts_lines_sooner() {
        // LRC convention: a positive offset makes the lyrics appear sooner.
        let lines = parse_lrc("[offset:+500]\n[00:10.00]a\n[00:00.20]b");
        assert_eq!(lines[0].time_ms, 0, "saturates at zero");
        assert_eq!(lines[1].time_ms, 9_500);
        let later = parse_lrc("[offset:-250]\n[00:10.00]a");
        assert_eq!(later[0].time_ms, 10_250);
    }

    #[test]
    fn parses_enhanced_word_tags() {
        let lines = parse_lrc(
            "[00:12.00]<00:12.00>Like <00:12.50>the <00:13.00>legend<00:14.00>\n[00:15.00]x",
        );
        let l = &lines[0];
        assert_eq!(l.text, "Like the legend");
        assert_eq!(l.words.len(), 3);
        assert_eq!((l.words[0].start_ms, l.words[0].end_ms), (12_000, 12_500));
        assert_eq!((l.words[2].start_ms, l.words[2].end_ms), (13_000, 14_000));
        let concat: String = l.words.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(concat, l.text, "text is exactly the concatenated words");
    }

    #[test]
    fn open_last_word_closes_at_next_line() {
        let lines = parse_lrc("[00:01.00]<00:01.00>one <00:02.00>two\n[00:05.00]next");
        assert_eq!(lines[0].words[1].end_ms, 5_000);
    }

    #[test]
    fn out_of_order_word_tags_fall_back_to_line_mode() {
        let lines = parse_lrc("[00:05.00]<00:06.00>b <00:05.00>a");
        assert_eq!(lines[0].text, "b a");
        assert!(lines[0].words.is_empty());
    }

    #[test]
    fn parses_netease_yrc_fixture() {
        let v: serde_json::Value =
            serde_json::from_str(include_str!("fixtures/netease_lyric_get_lucky.json")).unwrap();
        let yrc = v["yrc"]["lyric"].as_str().unwrap();
        let lines = parse_yrc(yrc);
        // The two JSON credit rows are skipped.
        assert!(lines.iter().all(|l| !l.text.starts_with('{')));
        let far = lines
            .iter()
            .find(|l| l.text == "We've come too far")
            .expect("word-timed line");
        assert_eq!(far.time_ms, 49_630);
        assert_eq!(far.words.len(), 4);
        assert_eq!(
            (far.words[0].start_ms, far.words[0].end_ms),
            (49_630, 50_920)
        );
        assert!(lines.windows(2).all(|w| w[0].time_ms <= w[1].time_ms));
    }

    #[test]
    fn karaoke_split_walks_words() {
        let lines = parse_lrc("[00:01.00]<00:01.00>ab <00:02.00>cd<00:03.00>");
        let l = &lines[0];
        assert_eq!(l.text, "ab cd");
        assert_eq!(karaoke_split(l, 500), 0);
        assert_eq!(karaoke_split(l, 1_000), 0);
        // Half of "ab " (3 chars) → 1 char.
        assert_eq!(karaoke_split(l, 1_500), 1);
        assert_eq!(karaoke_split(l, 2_000), 3);
        assert_eq!(karaoke_split(l, 2_500), 4);
        assert_eq!(karaoke_split(l, 9_000), 5);
    }

    #[test]
    fn karaoke_split_stays_on_char_boundaries() {
        let lines = parse_yrc("[0,3000](0,3000,0)纯音乐ü");
        for pos in (0..3_000).step_by(37) {
            let at = karaoke_split(&lines[0], pos);
            assert!(lines[0].text.is_char_boundary(at), "pos {pos} → {at}");
        }
    }

    #[test]
    fn merges_translation_by_nearest_stamp() {
        let mut main = vec![
            line(1_000, "hello"),
            line(5_000, "world"),
            line(9_000, "same"),
        ];
        let layer = vec![
            line(1_200, "merhaba"),
            line(4_700, "dünya"),
            line(9_000, "SAME"),
            line(20_000, "orphan"),
        ];
        merge_layer(&mut main, &layer, Layer::Translation);
        assert_eq!(main[0].translation.as_deref(), Some("merhaba"));
        assert_eq!(main[1].translation.as_deref(), Some("dünya"));
        assert_eq!(
            main[2].translation, None,
            "a repeat of the original is dropped"
        );
        assert!(main.iter().all(|l| l.romanization.is_none()));
    }

    #[test]
    fn merges_real_netease_translation() {
        let v: serde_json::Value =
            serde_json::from_str(include_str!("fixtures/netease_lyric_get_lucky.json")).unwrap();
        let mut lines = parse_yrc(v["yrc"]["lyric"].as_str().unwrap());
        let layer = parse_lrc(v["ytlrc"]["lyric"].as_str().unwrap());
        merge_layer(&mut lines, &layer, Layer::Translation);
        let far = lines
            .iter()
            .find(|l| l.text == "We've come too far")
            .unwrap();
        assert_eq!(far.translation.as_deref(), Some("行迹致远"));
    }

    #[test]
    fn detects_credit_rows() {
        assert!(is_credit_line("作词 : Pharrell Williams"));
        assert!(is_credit_line("吉他：Nile Rodgers"));
        assert!(is_credit_line("Lyricist: Someone"));
        assert!(!is_credit_line("Look: we've come too far"));
        assert!(!is_credit_line("Just a line"));
    }

    #[test]
    fn detects_placeholders() {
        assert!(is_placeholder(&[line(0, "纯音乐，请欣赏")]));
        assert!(is_placeholder(&[line(0, "暂无歌词")]));
        assert!(!is_placeholder(&[line(0, "We've come too far")]));
        assert!(!is_placeholder(&[]));
    }
}
