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
    today                    Events happening today
    agenda [DAYS]            Events over the next DAYS days (default 7)
    on <YYYY-MM-DD>          Events on a specific date
    account list             List connected accounts
    account setup google     Connect a Google account (OAuth)
    account remove <id>      Disconnect an account

OPTIONS:
    --dir <PATH>        Local calendar folder (default ~/.config/margo/calendars)
    --ics <URL>         Add a remote .ics subscription (may repeat)
    --no-browser        (account setup) print the URL instead of launching a
                        browser — open it yourself in the profile / private
                        window for the account you want to connect
    -h, --help          Show this help
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    // Split flags (--dir/--ics/--no-browser) from the positional command.
    let mut dir: Option<String> = None;
    let mut ics: Vec<String> = Vec::new();
    let mut no_browser = false;
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
            "--no-browser" | "--manual" => no_browser = true,
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
        Some("account") => return run_account(&positional[1..], no_browser),
        Some(other) => return fail(&format!("unknown command: {other} (try --help)")),
    }

    ExitCode::SUCCESS
}

/// `mcal account <list|setup|remove> …`
fn run_account(args: &[String], no_browser: bool) -> ExitCode {
    match args.first().map(String::as_str) {
        None | Some("list") => account_list(),
        Some("setup") => match args.get(1).map(String::as_str) {
            Some("google") => account_setup_google(!no_browser),
            Some(other) => fail(&format!("unknown provider: {other} (try: google)")),
            None => fail("account setup needs a provider, e.g. mcal account setup google"),
        },
        Some("remove") => match args.get(1) {
            Some(id) => account_remove(id),
            None => fail("account remove needs an id (see mcal account list)"),
        },
        Some(other) => fail(&format!("unknown account command: {other}")),
    }
}

fn account_list() -> ExitCode {
    let store = match mcal::AccountStore::load() {
        Ok(store) => store,
        Err(e) => return fail(&e.to_string()),
    };
    if store.accounts.is_empty() {
        println!("No accounts. Add one with: mcal account setup google");
        return ExitCode::SUCCESS;
    }
    for a in &store.accounts {
        println!("{:<8} {:<28} {}", a.kind, a.email, a.id);
    }
    ExitCode::SUCCESS
}

fn account_setup_google(open_browser: bool) -> ExitCode {
    let creds = match mcal::load_google() {
        Ok(Some(creds)) => creds,
        Ok(None) => {
            eprintln!("{}", mcal::setup_instructions());
            return ExitCode::FAILURE;
        }
        Err(e) => return fail(&e.to_string()),
    };

    let tokens = match mcal::interactive_google_login(&creds, open_browser) {
        Ok(tokens) => tokens,
        Err(e) => return fail(&e.to_string()),
    };

    // The account id is the user's email = the id of its `primary` calendar.
    let email = match primary_email(&tokens.access_token) {
        Ok(email) => email,
        Err(e) => return fail(&e.to_string()),
    };
    let id = mcal::AccountStore::google_id(&email);

    if let Err(e) = mcal::store_refresh_token(&id, &tokens.refresh_token) {
        return fail(&e.to_string());
    }
    let mut store = match mcal::AccountStore::load() {
        Ok(store) => store,
        Err(e) => return fail(&e.to_string()),
    };
    store.add(mcal::StoredAccount {
        id: id.clone(),
        kind: "google".into(),
        email: email.clone(),
        display_name: email.split('@').next().unwrap_or(&email).to_string(),
    });
    if let Err(e) = store.save() {
        return fail(&e.to_string());
    }
    println!("Connected {email}. Try: mcal today");
    ExitCode::SUCCESS
}

/// The account's email = the id of its `primary` calendar.
fn primary_email(access_token: &str) -> Result<String, mcal::McalError> {
    #[derive(serde::Deserialize)]
    struct Cal {
        id: String,
    }
    let cal: Cal = ureq::get("https://www.googleapis.com/calendar/v3/calendars/primary")
        .set("Authorization", &format!("Bearer {access_token}"))
        .call()
        .map_err(|e| mcal::McalError::Fetch {
            url: "calendars/primary".into(),
            source: Box::new(e),
        })?
        .into_json()
        .map_err(|e| mcal::McalError::Json(e.to_string()))?;
    Ok(cal.id)
}

fn account_remove(id: &str) -> ExitCode {
    let mut store = match mcal::AccountStore::load() {
        Ok(store) => store,
        Err(e) => return fail(&e.to_string()),
    };
    if !store.remove(id) {
        return fail(&format!("no such account: {id}"));
    }
    if let Err(e) = store.save() {
        return fail(&e.to_string());
    }
    let _ = mcal::delete_refresh_token(id);
    println!("Removed {id}.");
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
