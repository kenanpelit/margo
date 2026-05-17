//! Provider-based application launcher runtime for mshell.
//!
//! The launcher is structured as a set of independent **providers**
//! that each contribute search results. The UI layer (mshell-frame)
//! owns a `LauncherRuntime`, hands it a query each keystroke, and
//! receives back a flat, scored, sorted `Vec<LauncherItem>` ready
//! for display.
//!
//! ## Architecture
//!
//! ```text
//!   user types          ┌──────────────────┐
//!     ──────────────►   │ LauncherRuntime  │
//!                       │   ┌────────────┐ │
//!                       │   │ providers  │ │
//!                       │   │ ─ Apps     │ │ ◄── lives in mshell-frame
//!                       │   │ ─ Calculator│ │
//!                       │   │ ─ Command  │ │
//!                       │   │ ─ Session  │ │
//!                       │   │ ─ Settings │ │
//!                       │   └────────────┘ │
//!                       └────────┬─────────┘
//!                                ▼
//!                       Vec<LauncherItem>
//! ```
//!
//! ## Providers vs the runtime
//!
//! The `Apps` provider needs `gtk::gio::DesktopAppInfo` (GTK types
//! that don't cross threads cleanly) so it lives in `mshell-frame`.
//! Pure providers — Calculator, Command, Session, Settings — live
//! here and only depend on plain data + `std::process::Command`.
//!
//! ## Command mode
//!
//! When a query starts with `>` the runtime switches to **command
//! mode**: it asks each provider for its `commands()` and looks for
//! one whose `supports_command()` matches. The matching provider
//! takes over result generation (so e.g. `>cmd ls` runs `ls` via
//! the Command provider, not the Apps fuzzy matcher).

pub mod frecency;
pub mod history;
pub mod item;
pub mod notify;
pub mod pin;
pub mod provider;
pub mod providers;
pub mod runtime;
pub mod scoring;

pub use frecency::FrecencyStore;
pub use history::CommandHistory;
pub use item::{DisplayItem, LauncherItem};
pub use pin::PinStore;
pub use provider::Provider;
pub use runtime::{LauncherRuntime, ProviderCategory};
