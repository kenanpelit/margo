//! Built-in providers: Calculator, Command, Session, Settings.
//!
//! Apps is intentionally absent — it depends on
//! `gtk::gio::DesktopAppInfo` and so lives in `mshell-frame`
//! alongside the rest of the GTK code.

pub mod archpkgs;
pub mod bluetooth;
pub mod calculator;
pub mod command;
pub mod emoji;
pub mod mctl;
pub mod playerctl;
pub mod provider_list;
pub mod scripts;
pub mod session;
pub mod settings;
pub mod symbols;
pub mod websearch;
pub mod wireplumber;

pub use archpkgs::ArchLinuxPkgsProvider;
pub use bluetooth::BluetoothProvider;
pub use calculator::CalculatorProvider;
pub use command::CommandProvider;
pub use emoji::EmojiProvider;
pub use mctl::MctlProvider;
pub use playerctl::PlayerctlProvider;
pub use provider_list::ProviderListProvider;
pub use scripts::ScriptsProvider;
pub use session::{SessionAction, SessionActionId, SessionProvider};
pub use settings::{SettingsProvider, SettingsSection};
pub use symbols::SymbolsProvider;
pub use websearch::WebsearchProvider;
pub use wireplumber::WireplumberProvider;
