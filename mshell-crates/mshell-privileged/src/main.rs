//! `mshell-privileged <subcommand> <args…>`: run as root by pkexec.
//! See the library docs for the vocabulary and the checks.

fn main() -> std::process::ExitCode {
    let argv: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    std::process::ExitCode::from(mshell_privileged::run::main(&argv))
}
