//! mcal — margo's calendar engine (GTK-free).
//!
//! Ported (not 1:1) from dankcalendar's Go core. This crate owns the calendar
//! *domain*: the event/calendar/account model, RFC 5545 ICS parsing, RRULE
//! recurrence expansion, the read-only providers (local files + remote ICS
//! subscriptions), and a reactive store the GTK shell subscribes to. It pulls
//! in no GTK — everything here is unit-testable headlessly (`cargo test`).
//!
//! See `docs/superpowers/specs/2026-07-03-mcal-calendar-design.md`.

mod error;
mod ics;
mod model;
mod recur;

pub use error::McalError;
pub use ics::parse_ics;
pub use model::{Account, AccountKind, Attendee, Calendar, Event};
pub use recur::expand;
