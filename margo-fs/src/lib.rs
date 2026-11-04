//! std-only file helpers shared by margo's tools and mshell.
//!
//! * [`atomic_write`] — symlink-preserving, fsynced tmp + rename.
//! * [`side_file`] — load/save for small side files that never replaces a
//!   file that does not parse.
//! * [`xdg`] — XDG base dirs, one rule for margo and mshell.

pub mod atomic_write;
pub mod side_file;
pub mod xdg;
