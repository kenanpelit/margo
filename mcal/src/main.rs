//! `mcal` — a tiny read-only calendar CLI over the mcal domain crate.
//!
//! Shows events from a local `.ics` folder (default `~/.config/margo/calendars`,
//! override with `--dir`) plus any number of ad-hoc `--ics <url>` subscriptions.
//! It intentionally does NOT read the shell's YAML config — that coupling lives
//! in `mshell-frame`; this binary stays as pure/standalone as the library.
//!
//! Usage:
//!   mcal today                 events for today
//!   mcal agenda [DAYS]         next DAYS days (default 7)
//!   mcal on YYYY-MM-DD         events on a specific date
//!   mcal --dir ~/cal today     use a different local folder
//!   mcal --ics URL today       also pull a remote .ics (repeatable)

use chrono::{Duration, Local, NaiveDate, NaiveTime, TimeZone, Utc};
use mcal::{CalendarConfig, Event, Subscription};
use std::process::ExitCode;

const USAGE: &str = "\
mcal — read-only calendar viewer

USAGE:
    mcal [OPTIONS] <COMMAND>

COMMANDS:
    today               Events happening today
    agenda [DAYS]       Events over the next DAYS days (default 7)
    on <YYYY-MM-DD>     Events on a specific date

OPTIONS:
    --dir <PATH>        Local calendar folder (default ~/.config/margo/calendars)
    --ics <URL>         Add a remote .ics subscription (may repeat)
    -h, --help          Show this help
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    // Split flags (--dir/--ics) from the positional command + operand.
    let mut dir: Option<String> = None;
    let mut ics: Vec<String> = Vec::new();
    let mut positional: Vec<String> = Vec::new();
    let mut it = args.into_iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-h" | "--help" | "help" => {
                print!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            "--dir" => match it.next() {
                Some(v) => dir = Some(v),
                None => return fail("--dir needs a path"),
            },
            "--ics" => match it.next() {
                Some(v) => ics.push(v),
                None => return fail("--ics needs a URL"),
            },
            other if other.starts_with('-') => {
                return fail(&format!("unknown option: {other}"));
            }
            other => positional.push(other.to_string()),
        }
    }

    let config = build_config(dir, ics);
    let today = Local::now().date_naive();

    match positional.first().map(String::as_str) {
        None | Some("today") => print_range(&config, today, today),
        Some("agenda") => {
            let days = match positional.get(1) {
                Some(n) => match n.parse::<i64>() {
                    Ok(d) if d >= 1 => d,
                    _ => return fail("agenda DAYS must be a positive integer"),
                },
                None => 7,
            };
            print_range(&config, today, today + Duration::days(days - 1))
        }
        Some("on") => match positional.get(1) {
            Some(s) => match NaiveDate::parse_from_str(s, "%Y-%m-%d") {
                Ok(date) => print_range(&config, date, date),
                Err(_) => return fail("date must be YYYY-MM-DD"),
            },
            None => return fail("on needs a date, e.g. mcal on 2026-07-04"),
        },
        Some(other) => return fail(&format!("unknown command: {other} (try --help)")),
    }

    ExitCode::SUCCESS
}

/// Build the load config from the `--dir` / `--ics` flags.
fn build_config(dir: Option<String>, ics: Vec<String>) -> CalendarConfig {
    let local_dir = dir
        .map(expand_tilde)
        .unwrap_or_else(mcal::default_local_dir);
    let subscriptions = ics
        .into_iter()
        .map(|url| Subscription {
            name: url.clone(),
            url,
            color: None,
        })
        .collect();
    CalendarConfig {
        local_dir,
        subscriptions,
        refresh_secs: 0,
    }
}

/// Load and print every day in `[from, to]` (inclusive) that has events.
fn print_range(config: &CalendarConfig, from: NaiveDate, to: NaiveDate) {
    // Load a UTC window one day wider on each side, so timezone offsets never
    // clip an edge day.
    let window = (
        Utc.from_utc_datetime(&(from - Duration::days(1)).and_time(NaiveTime::MIN)),
        Utc.from_utc_datetime(&(to + Duration::days(2)).and_time(NaiveTime::MIN)),
    );
    let events = mcal::load_all(config, window);

    let mut day = from;
    let mut printed = 0usize;
    while day <= to {
        let on_day = mcal::events_on_day(&events, day);
        if !on_day.is_empty() {
            if printed > 0 {
                println!();
            }
            print_day(day, &on_day);
            printed += 1;
        }
        day = match day.succ_opt() {
            Some(next) => next,
            None => break,
        };
    }
    if printed == 0 {
        println!("No events.");
    }
}

/// Print one day's heading and its sorted events.
fn print_day(date: NaiveDate, events: &[Event]) {
    println!("{}", date.format("%A, %B %-d, %Y"));
    for event in events {
        let time = time_label(event);
        let mut line = format!("  {time:<7} {}", event.summary);
        if let Some(loc) = event.location.as_deref().filter(|l| !l.is_empty()) {
            line.push_str(&format!("  ({loc})"));
        }
        println!("{line}");
    }
}

/// "All day" or a local `HH:MM` start.
fn time_label(event: &Event) -> String {
    if event.all_day {
        "all-day".to_string()
    } else {
        event
            .start
            .with_timezone(&Local)
            .format("%H:%M")
            .to_string()
    }
}

/// Expand a leading `~` / `~/` against `$HOME`.
fn expand_tilde(path: String) -> std::path::PathBuf {
    if path == "~" {
        if let Ok(home) = std::env::var("HOME") {
            return std::path::PathBuf::from(home);
        }
    } else if let Some(rest) = path.strip_prefix("~/")
        && let Ok(home) = std::env::var("HOME")
    {
        return std::path::PathBuf::from(home).join(rest);
    }
    std::path::PathBuf::from(path)
}

/// Print an error to stderr and return a failure exit code.
fn fail(msg: &str) -> ExitCode {
    eprintln!("mcal: {msg}");
    ExitCode::FAILURE
}
