//! Built-in providers: Calculator, Command, Session, Settings.
//!
//! Apps is intentionally absent — it depends on
//! `gtk::gio::DesktopAppInfo` and so lives in `mshell-frame`
//! alongside the rest of the GTK code.

pub mod calculator;
pub mod command;
pub mod mctl;
pub mod session;
pub mod settings;

pub use calculator::CalculatorProvider;
pub use command::CommandProvider;
pub use mctl::MctlProvider;
pub use session::{SessionAction, SessionActionId, SessionProvider};
pub use settings::{SettingsProvider, SettingsSection};
