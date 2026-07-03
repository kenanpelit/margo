//! mcal — margo's calendar engine (GTK-free).
//!
//! Ported (not 1:1) from dankcalendar's Go core. This crate owns the calendar
//! *domain*: the event/calendar/account model, RFC 5545 ICS parsing, RRULE
//! recurrence expansion, the read-only providers (local files + remote ICS
//! subscriptions), and a reactive store the GTK shell subscribes to. It pulls
//! in no GTK — everything here is unit-testable headlessly (`cargo test`).
//!
//! See `docs/superpowers/specs/2026-07-03-mcal-calendar-design.md`.

mod account;
mod agenda;
mod config;
mod credentials;
mod error;
mod ics;
mod model;
mod oauth;
mod provider;
mod recur;
mod secret;

pub use account::{AccountStore, StoredAccount, accounts_path};
pub use agenda::{days_with_events, events_on_day, sort_agenda};
pub use config::{CalendarConfig, Subscription, default_local_dir};
pub use credentials::{GoogleCredentials, credentials_path, load_google, setup_instructions};
pub use error::McalError;
pub use ics::parse_ics;
pub use model::{Account, AccountKind, Attendee, Calendar, Event};
pub use oauth::{GoogleTokens, interactive_google_login, refresh_access_token};
pub use provider::{GoogleProvider, LocalProvider, Provider, RemoteIcsProvider, Window, load_all};
pub use recur::expand;
pub use secret::{delete_refresh_token, get_refresh_token, store_refresh_token};
