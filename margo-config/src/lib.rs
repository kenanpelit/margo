mod parser;
mod types;

pub mod diagnostics;
pub mod validator;

pub use parser::parse_config;
pub use types::*;
