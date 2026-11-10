//! The one-line timer / reminder parser: `"10m tea"`, `"1h 30m"`,
//! `"14:30 meeting"`, `"11am standup"`, `"tomorrow 09:00 - coffee"`,
//! `"in 10m tea"`, `"10 dk sonra çay"`, `"yarın 09:00 kahve"`, …
//!
//! [`parse`] is a pure function of `(now, input)`: no clock reads, no I/O,
//! so every form is pinned by the table in `parse_tests.rs`. `now` carries
//! its time zone, and a clock time is resolved in that zone (a DST gap moves
//! the time forward, an ambiguous fold picks the earlier instant).
//!
//! Grammar, loosely:
//!
//! * a **duration** — one or more `<number><unit>` parts, glued (`1h30m`,
//!   `10dk`) or spaced (`1 h 30 min`, `1 saat 30 dakika`), optionally joined
//!   by `and` / `ve`; units s/sec/seconds/sn/saniye, m/min/minutes/dk/dakika,
//!   h/hr/hours/sa/saat, d/days/gün; decimals (`1.5h`, `1,5 saat`), articles
//!   (`an hour`, `bir saat`), `half an hour`, `yarım saat`, `çeyrek saat`,
//!   `1 buçuk saat`. A bare number at the start is minutes (`25 Pomodoro`).
//!   Optional `in` / `after` / `for` before it, `later` / `from now` /
//!   `sonra` / `içinde` after it. `in 1:30` is a duration (1 h 30 min).
//! * a **clock time** — `14:30`, `14.30`, `9:05:30`, `11am`, `11:30 pm`,
//!   `noon`, `midnight`, `öğlen`, `gece yarısı`, a Turkish case suffix
//!   (`14:30'da`) is ignored; a bare hour needs `at` / `saat` (`at 3`
//!   rings at the next 3 o'clock, morning or afternoon). Optional `today` /
//!   `bugün` / `tomorrow` / `yarın` before (or after) it. A time already
//!   past today rings tomorrow — unless `today` was said, which is an error.
//! * a **label** — everything else, or the text after ` - ` / ` – ` / ` — `.
//!   The time may come first (`10m tea`) or later (`tea in 10m`,
//!   `toplantı saat 14:30`).

use chrono::{DateTime, Duration, LocalResult, NaiveDateTime, NaiveTime, TimeZone};
use regex::Regex;
use std::fmt;
use std::sync::LazyLock;

/// The furthest ahead a timer may ring: one year.
pub const MAX_AHEAD_SECS: i64 = 365 * 24 * 3600;

/// Longest label kept (characters); longer input is cut.
pub const MAX_LABEL_CHARS: usize = 120;

/// Examples shown with every parse error.
pub const EXAMPLES: &str = "Try: 10m tea · 1h 30m · 14:30 meeting · tomorrow 9am · 10 dk sonra çay";

/// What kind of entry a line made — drives the wording and the tone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TimerKind {
    /// A relative countdown (`10m tea`).
    Timer,
    /// A clock-time reminder (`14:30 meeting`).
    Reminder,
    /// A snoozed timer coming back.
    Snooze,
}

/// A successfully parsed line.
#[derive(Debug, Clone)]
pub struct Parsed<Tz: TimeZone> {
    /// When it rings.
    pub due: DateTime<Tz>,
    /// The label (may be empty).
    pub label: String,
    pub kind: TimerKind,
}

/// Why a line did not parse. `Display` is the user-facing message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// Nothing typed.
    Empty,
    /// No duration or clock time anywhere in the line.
    NoTime,
    /// A duration of zero (`0m`).
    Zero,
    /// More than [`MAX_AHEAD_SECS`] ahead.
    TooFar,
    /// A clock-shaped token that is not a time of day (`25:00`, `13pm`).
    BadClock(String),
    /// `today 09:00` said after 09:00.
    Passed(String),
    /// `today` / `tomorrow` with a duration (`tomorrow 10m`).
    DayNeedsClock,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::Empty => write!(f, "Type a time or a countdown. {EXAMPLES}"),
            ParseError::NoTime => write!(f, "No time found. {EXAMPLES}"),
            ParseError::Zero => write!(f, "A timer needs more than zero time."),
            ParseError::TooFar => write!(f, "That's too far ahead: timers go up to 365 days."),
            ParseError::BadClock(t) => {
                write!(f, "“{t}” isn't a time of day (00:00–23:59, 1–12 am/pm).")
            }
            ParseError::Passed(t) => {
                write!(
                    f,
                    "{t} has already passed today; leave out “today” to ring tomorrow."
                )
            }
            ParseError::DayNeedsClock => {
                write!(
                    f,
                    "“today” / “tomorrow” needs a clock time, e.g. tomorrow 09:00."
                )
            }
        }
    }
}

impl std::error::Error for ParseError {}

/// Parse `input` relative to `now`.
pub fn parse<Tz: TimeZone>(now: &DateTime<Tz>, input: &str) -> Result<Parsed<Tz>, ParseError> {
    let input = input.trim();
    if input.is_empty() {
        return Err(ParseError::Empty);
    }
    let (spec, sep_label) = split_label(input);
    let toks = tokenize(spec);
    if toks.is_empty() {
        return Err(ParseError::Empty);
    }

    // The time at the start, possibly after command words ("set a timer
    // for …", "remind me in …"). Only the first non-filler position may
    // carry a bare number.
    let mut first = 0;
    let mut found = None;
    while first < toks.len() {
        if let Some(m) = try_at(&toks, first, true)? {
            found = Some(m);
            break;
        }
        if !is_filler(&toks[first].key) {
            break;
        }
        first += 1;
    }
    // Otherwise the first strong time expression later in the line
    // ("tea in 10m", "meeting at 14:30").
    if found.is_none() {
        for j in (first + 1)..toks.len() {
            if let Some(m) = try_at(&toks, j, false)? {
                found = Some(m);
                break;
            }
        }
    }
    let Some(m) = found else {
        // "tomorrow coffee": a day with no clock time.
        return Err(if toks.iter().any(|t| day_word(&t.key).is_some()) {
            ParseError::DayNeedsClock
        } else {
            ParseError::NoTime
        });
    };

    let label = match sep_label {
        Some(l) => clean_label(l),
        None => {
            let words: Vec<&str> = toks[first.min(m.start)..m.start]
                .iter()
                .chain(&toks[m.end..])
                .map(|t| t.raw)
                .collect();
            clean_label(&words.join(" "))
        }
    };

    let (due, kind) = match m.what {
        What::Relative(secs) => (resolve_relative(now, secs)?, TimerKind::Timer),
        What::Clock(day, clock) => (resolve_clock(now, day, clock)?, TimerKind::Reminder),
    };
    Ok(Parsed { due, label, kind })
}

// ── Tokens ──────────────────────────────────────────────────────────────

struct Tok<'a> {
    /// The word as typed (labels keep it).
    raw: &'a str,
    /// Lower-cased with Turkish letters folded to ASCII (`yarın` → `yarin`).
    key: String,
}

fn tokenize(s: &str) -> Vec<Tok<'_>> {
    s.split_whitespace()
        .map(|raw| Tok {
            raw,
            key: key_of(raw),
        })
        .collect()
}

/// The match key of a word: folded, without trailing punctuation
/// (`10m,` → `10m`), keeping the dots of `a.m.` / `p.m.`.
fn key_of(raw: &str) -> String {
    let folded = fold(raw);
    let trimmed = folded.trim_end_matches([',', ';', '!', '?']);
    if trimmed.ends_with("a.m.") || trimmed.ends_with("p.m.") {
        trimmed.to_string()
    } else {
        trimmed.trim_end_matches('.').to_string()
    }
}

/// Lower-case and fold Turkish letters so `YARIN`, `yarın` and `yarin`
/// match the same keyword.
fn fold(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| *c != '\u{307}') // `İ`.to_lowercase() = "i\u{307}"
        .map(|c| match c {
            'ı' | 'î' => 'i',
            'ş' => 's',
            'ğ' => 'g',
            'ü' | 'û' => 'u',
            'ö' => 'o',
            'ç' => 'c',
            'â' => 'a',
            '’' | '‘' | '`' | '´' => '\'',
            c => c,
        })
        .collect()
}

/// Split `"<spec> - <label>"` at the first ` - ` / ` – ` / ` — `.
fn split_label(input: &str) -> (&str, Option<&str>) {
    let cut = [" - ", " – ", " — "]
        .iter()
        .filter_map(|sep| input.find(sep).map(|i| (i, sep.len())))
        .min_by_key(|(i, _)| *i);
    match cut {
        Some((i, len)) if !input[..i].trim().is_empty() => {
            let label = input[i + len..].trim();
            (input[..i].trim(), (!label.is_empty()).then_some(label))
        }
        _ => (input, None),
    }
}

/// Command words that may precede the time and are not part of the label.
fn is_filler(key: &str) -> bool {
    matches!(
        key,
        "set"
            | "add"
            | "new"
            | "start"
            | "create"
            | "a"
            | "an"
            | "timer"
            | "reminder"
            | "alarm"
            | "remind"
            | "me"
            | "please"
            | "kur"
            | "hatirlat"
            | "hatirlatici"
            | "zamanlayici"
    )
}

fn clean_label(s: &str) -> String {
    let s = s.trim_matches(|c: char| c.is_whitespace() || "-–—,;:".contains(c));
    // "remind me in 10m to call mom" → "call mom".
    let s = ["to ", "for ", "about "]
        .iter()
        .find_map(|w| {
            s.get(..w.len())
                .filter(|head| head.eq_ignore_ascii_case(w))
                .map(|_| &s[w.len()..])
        })
        .unwrap_or(s)
        .trim();
    s.chars()
        .filter(|c| !c.is_control())
        .take(MAX_LABEL_CHARS)
        .collect::<String>()
        .trim()
        .to_string()
}

// ── Matching ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Day {
    Today,
    Tomorrow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Clock {
    hour: u32,
    minute: u32,
    second: u32,
    /// `at 3` — no minutes, no am/pm: the next 3 o'clock, either half of
    /// the day.
    bare_hour: bool,
}

#[derive(Debug, Clone, Copy)]
enum What {
    /// Seconds from now (not yet rounded or range-checked).
    Relative(f64),
    Clock(Option<Day>, Clock),
}

struct Match {
    /// First token of the expression (markers included).
    start: usize,
    /// One past its last token.
    end: usize,
    what: What,
}

fn day_word(key: &str) -> Option<Day> {
    match key {
        "today" | "bugun" => Some(Day::Today),
        "tomorrow" | "tmr" | "tmrw" | "yarin" => Some(Day::Tomorrow),
        _ => None,
    }
}

/// Try a time expression starting at token `start`. `leading` = the line
/// starts here, which also allows a bare number (minutes) and `14.30`.
fn try_at(toks: &[Tok<'_>], start: usize, leading: bool) -> Result<Option<Match>, ParseError> {
    let key = |i: usize| toks.get(i).map(|t| t.key.as_str()).unwrap_or("");
    let mut j = start;
    let mut day = None;
    let mut relative = false;
    let mut at = false;
    while j < toks.len() {
        let k = key(j);
        if !relative && !at && matches!(k, "in" | "after" | "within" | "for") {
            relative = true;
        } else if !relative && !at && matches!(k, "at" | "@" | "by") {
            at = true;
        } else if !relative && !at && k == "saat" && clock_at(toks, j + 1, true, true)?.is_some() {
            // "saat 14:30" = at 14:30 (but "1 saat" is an hour).
            at = true;
        } else if day.is_none()
            && let Some(d) = day_word(k)
        {
            day = Some(d);
        } else {
            break;
        }
        j += 1;
    }

    if day.is_none() && !at {
        if relative && let Some(secs) = hm_duration(key(j)) {
            let end = skip_relative_tail(toks, j + 1);
            return Ok(Some(Match {
                start,
                end,
                what: What::Relative(secs),
            }));
        }
        // A bare number (minutes) only at the start of the line: "table
        // for 2 at 19:00" is not a two-minute timer.
        if let Some((secs, next)) = duration_at(toks, j, leading) {
            let end = skip_relative_tail(toks, next);
            return Ok(Some(Match {
                start,
                end,
                what: What::Relative(secs),
            }));
        }
    }

    let dot_ok = leading || at || day.is_some();
    if let Some((clock, next)) = clock_at(toks, j, at, dot_ok)? {
        let mut end = next;
        if day.is_none()
            && let Some(d) = day_word(key(end))
        {
            day = Some(d);
            end += 1;
        }
        // Turkish clitic written apart: "14:30 da".
        if matches!(key(end), "da" | "de" | "ta" | "te") {
            end += 1;
        }
        return Ok(Some(Match {
            start,
            end,
            what: What::Clock(day, clock),
        }));
    }

    if day.is_some() && duration_at(toks, j, true).is_some() {
        return Err(ParseError::DayNeedsClock);
    }
    Ok(None)
}

/// Words after a duration that mean "from now".
fn skip_relative_tail(toks: &[Tok<'_>], mut i: usize) -> usize {
    let key = |i: usize| toks.get(i).map(|t| t.key.as_str()).unwrap_or("");
    if key(i) == "from" && key(i + 1) == "now" {
        i += 2;
    } else if matches!(key(i), "later" | "sonra" | "icinde" | "sonrasi") {
        i += 1;
    }
    i
}

/// Seconds per unit word.
fn unit_secs(key: &str) -> Option<f64> {
    Some(match key {
        "s" | "sec" | "secs" | "second" | "seconds" | "sn" | "sny" | "saniye" => 1.0,
        "m" | "min" | "mins" | "minute" | "minutes" | "dk" | "dak" | "dakika" => 60.0,
        "h" | "hr" | "hrs" | "hour" | "hours" | "sa" | "saat" | "st" => 3600.0,
        "d" | "day" | "days" | "gun" => 86_400.0,
        _ => return None,
    })
}

/// A plain non-negative number: `10`, `1.5`, `1,5`.
fn number(s: &str) -> Option<f64> {
    if s.is_empty()
        || !s
            .chars()
            .all(|c| c.is_ascii_digit() || c == '.' || c == ',')
        || !s.starts_with(|c: char| c.is_ascii_digit())
        || !s.ends_with(|c: char| c.is_ascii_digit())
        || s.chars().filter(|c| *c == '.' || *c == ',').count() > 1
    {
        return None;
    }
    s.replace(',', ".")
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
}

/// A glued duration token: `10m`, `1h30m`, `1h30`, `10dk`, `1sa30dk`,
/// `1.5h`, `2m30s`.
fn glued_secs(key: &str) -> Option<f64> {
    let mut total = 0.0;
    let mut rest = key;
    let mut last_unit: Option<f64> = None;
    let mut parts = 0;
    while !rest.is_empty() {
        let num_len = rest
            .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == ','))
            .unwrap_or(rest.len());
        let value = number(&rest[..num_len])?;
        rest = &rest[num_len..];
        let unit_len = rest
            .find(|c: char| !c.is_alphabetic())
            .unwrap_or(rest.len());
        if unit_len == 0 {
            // A trailing bare number takes the next unit down: 1h30 = 1h 30m.
            if !rest.is_empty() {
                return None;
            }
            let unit = match last_unit? {
                u if u >= 86_400.0 => 3600.0,
                u if u >= 3600.0 => 60.0,
                u if u >= 60.0 => 1.0,
                _ => return None,
            };
            total += value * unit;
            parts += 1;
            break;
        }
        let unit = unit_secs(&rest[..unit_len])?;
        rest = &rest[unit_len..];
        total += value * unit;
        last_unit = Some(unit);
        parts += 1;
    }
    // At least one unit: a bare `10` is not glued.
    (parts > 0 && last_unit.is_some()).then_some(total)
}

/// `1:30` after "in" — hours and minutes.
fn hm_duration(key: &str) -> Option<f64> {
    let (h, m) = key.split_once(':')?;
    if h.is_empty() || h.len() > 3 || m.len() != 2 {
        return None;
    }
    let h: u32 = h.parse().ok()?;
    let m: u32 = m.parse().ok()?;
    (m < 60).then_some(f64::from(h) * 3600.0 + f64::from(m) * 60.0)
}

/// One duration part at `j`: `(seconds, next index)`.
fn part_at(toks: &[Tok<'_>], j: usize) -> Option<(f64, usize)> {
    let key = |i: usize| toks.get(i).map(|t| t.key.as_str()).unwrap_or("");
    let k = key(j);
    if k.is_empty() {
        return None;
    }
    if let Some(secs) = glued_secs(k) {
        return Some((secs, j + 1));
    }
    if let Some(n) = number(k) {
        if let Some(u) = unit_secs(key(j + 1)) {
            return Some((n * u, j + 2));
        }
        // "1 buçuk saat" = 1.5 hours.
        if key(j + 1) == "bucuk"
            && let Some(u) = unit_secs(key(j + 2))
        {
            return Some(((n + 0.5) * u, j + 3));
        }
        return None;
    }
    // Spelled-out amounts take a whole-word unit only ("an hour", never
    // "a m"), so a label word can't turn into a duration.
    let word_unit = |i: usize| unit_secs(key(i)).filter(|_| key(i).len() >= 3);
    match k {
        "an" | "a" | "one" | "bir" => word_unit(j + 1).map(|u| (u, j + 2)),
        "half" if matches!(key(j + 1), "an" | "a") && key(j + 2) == "hour" => Some((1800.0, j + 3)),
        "yarim" => word_unit(j + 1).map(|u| (u / 2.0, j + 2)),
        "ceyrek" if key(j + 1) == "saat" => Some((900.0, j + 2)),
        _ => None,
    }
}

/// A duration of one or more parts at `j`. `allow_bare`: a lone integer
/// means minutes.
fn duration_at(toks: &[Tok<'_>], j: usize, allow_bare: bool) -> Option<(f64, usize)> {
    let key = |i: usize| toks.get(i).map(|t| t.key.as_str()).unwrap_or("");
    let mut total = 0.0;
    let mut next = j;
    let mut parts = 0;
    loop {
        if let Some((secs, n)) = part_at(toks, next) {
            total += secs;
            next = n;
            parts += 1;
            continue;
        }
        // "1 hour and 30 minutes", "1 saat ve 30 dakika".
        if parts > 0
            && matches!(key(next), "and" | "ve" | "&" | "+")
            && let Some((secs, n)) = part_at(toks, next + 1)
        {
            total += secs;
            next = n;
            parts += 1;
            continue;
        }
        break;
    }
    if parts > 0 {
        return Some((total, next));
    }
    let k = key(j);
    // "11 am" is a clock time, not eleven minutes.
    if allow_bare
        && !k.is_empty()
        && k.chars().all(|c| c.is_ascii_digit())
        && ampm_word(key(j + 1)).is_none()
    {
        return number(k).map(|n| (n * 60.0, j + 1));
    }
    None
}

static CLOCK: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"^(\d{1,2})(?:([:.])(\d{2})(?::(\d{2}))?)?(am|pm|a\.m\.|p\.m\.)?$").ok()
});

fn ampm_word(key: &str) -> Option<bool> {
    match key {
        "am" | "a.m." | "a.m" => Some(false),
        "pm" | "p.m." | "p.m" => Some(true),
        _ => None,
    }
}

/// A clock time at `j`: `(clock, next index)`. `bare_ok`: a lone hour
/// counts (after "at"). `dot_ok`: `14.30` counts.
fn clock_at(
    toks: &[Tok<'_>],
    j: usize,
    bare_ok: bool,
    dot_ok: bool,
) -> Result<Option<(Clock, usize)>, ParseError> {
    let key = |i: usize| toks.get(i).map(|t| t.key.as_str()).unwrap_or("");
    let k = key(j);
    let named = match k {
        "noon" | "midday" | "ogle" | "oglen" => Some((12, j + 1)),
        "midnight" => Some((0, j + 1)),
        "gece" if matches!(key(j + 1), "yarisi" | "yarisinda") => Some((0, j + 2)),
        _ => None,
    };
    if let Some((hour, next)) = named {
        let clock = Clock {
            hour,
            minute: 0,
            second: 0,
            bare_hour: false,
        };
        return Ok(Some((clock, next)));
    }

    // "14:30'da", "9'a" — drop a Turkish case suffix.
    let core = k.split('\'').next().unwrap_or(k);
    let Some(caps) = CLOCK.as_ref().and_then(|re| re.captures(core)) else {
        return Ok(None);
    };
    let raw = toks.get(j).map(|t| t.raw).unwrap_or(k);
    let field = |i: usize| caps.get(i).map(|m| m.as_str());
    let mut hour: u32 = field(1).and_then(|h| h.parse().ok()).unwrap_or(0);
    let sep = field(2);
    let minute: u32 = field(3).and_then(|m| m.parse().ok()).unwrap_or(0);
    let second: u32 = field(4).and_then(|s| s.parse().ok()).unwrap_or(0);
    let mut next = j + 1;
    let mut pm = match field(5) {
        Some("am" | "a.m.") => Some(false),
        Some(_) => Some(true),
        None => None,
    };
    if pm.is_none()
        && let Some(p) = ampm_word(key(j + 1))
    {
        pm = Some(p);
        next += 1;
    }
    if sep == Some(".") && (!dot_ok || field(4).is_some()) {
        return Ok(None);
    }
    let has_minutes = sep.is_some();
    if !has_minutes && pm.is_none() && !bare_ok {
        return Ok(None);
    }
    let bad = || ParseError::BadClock(raw.to_string());
    if minute > 59 || second > 59 {
        return Err(bad());
    }
    match pm {
        Some(is_pm) => {
            if !(1..=12).contains(&hour) {
                return Err(bad());
            }
            hour = (hour % 12) + if is_pm { 12 } else { 0 };
        }
        None => {
            if hour == 24 && minute == 0 && second == 0 {
                hour = 0;
            } else if hour > 23 {
                return Err(bad());
            }
        }
    }
    Ok(Some((
        Clock {
            hour,
            minute,
            second,
            bare_hour: !has_minutes && pm.is_none(),
        },
        next,
    )))
}

// ── Resolution ──────────────────────────────────────────────────────────

fn resolve_relative<Tz: TimeZone>(
    now: &DateTime<Tz>,
    secs: f64,
) -> Result<DateTime<Tz>, ParseError> {
    let secs = secs.round();
    if secs.is_nan() || secs < 1.0 {
        return Err(ParseError::Zero);
    }
    if secs > MAX_AHEAD_SECS as f64 {
        return Err(ParseError::TooFar);
    }
    Ok(now.clone() + Duration::seconds(secs as i64))
}

/// `naive` (a wall-clock time in `now`'s zone) as an instant. A time that
/// doesn't exist (a DST gap) moves forward an hour at a time; an ambiguous
/// one (the repeated hour) takes the earlier instant.
fn localize<Tz: TimeZone>(now: &DateTime<Tz>, naive: NaiveDateTime) -> Option<DateTime<Tz>> {
    let tz = now.timezone();
    let mut t = naive;
    for _ in 0..4 {
        match tz.from_local_datetime(&t) {
            LocalResult::Single(d) => return Some(d),
            LocalResult::Ambiguous(a, b) => return Some(if a <= b { a } else { b }),
            LocalResult::None => t += Duration::hours(1),
        }
    }
    None
}

fn resolve_clock<Tz: TimeZone>(
    now: &DateTime<Tz>,
    day: Option<Day>,
    clock: Clock,
) -> Result<DateTime<Tz>, ParseError> {
    let today = now.naive_local().date();
    // "at 3" means the next 3 o'clock: 03:00 or 15:00.
    let hours: Vec<u32> = if clock.bare_hour && (1..=12).contains(&clock.hour) {
        vec![clock.hour, (clock.hour + 12) % 24]
    } else {
        vec![clock.hour]
    };
    let shown = format!("{:02}:{:02}", clock.hour, clock.minute);
    let mut best: Option<DateTime<Tz>> = None;
    for (n, hour) in hours.iter().enumerate() {
        let Some(time) = NaiveTime::from_hms_opt(*hour, clock.minute, clock.second) else {
            continue;
        };
        let at_day = |offset: i64| localize(now, (today + Duration::days(offset)).and_time(time));
        let candidate = match day {
            // "tomorrow at 9" is the morning one.
            Some(Day::Tomorrow) if n > 0 => None,
            Some(Day::Tomorrow) => at_day(1),
            Some(Day::Today) => at_day(0).filter(|d| d > now),
            None => match at_day(0) {
                Some(d) if d > *now => Some(d),
                _ => at_day(1),
            },
        };
        if let Some(c) = candidate
            && best.as_ref().is_none_or(|b| c < *b)
        {
            best = Some(c);
        }
    }
    match best {
        Some(due) => Ok(due),
        None if day == Some(Day::Today) => Err(ParseError::Passed(shown)),
        None => Err(ParseError::BadClock(shown)),
    }
}

#[cfg(test)]
#[path = "parse_tests.rs"]
mod tests;
