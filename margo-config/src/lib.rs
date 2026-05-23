mod parser;
mod types;

pub mod diagnostics;
pub mod validator;

pub use parser::{apply_first_party_defaults, parse_config, parse_config_with_defaults};
pub use types::*;
