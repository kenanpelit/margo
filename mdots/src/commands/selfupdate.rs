//! `mdots self-update`: says how mdots is updated. It does not update anything.
//!
//! This used to rebuild mdots from a git checkout (`~/mdots`, … and a
//! hardcoded upstream `/home/don/mdots`) and `sudo install` the result to
//! /usr/local/bin. That is left over from the upstream project. mdots now
//! lives in the margo workspace and the margo package installs it as
//! /usr/bin/mdots. A self-built copy in /usr/local/bin comes first on PATH,
//! so it would hide every later packaged update. The subcommand is kept so
//! old muscle memory gets an answer instead of a clap error.

use anyhow::{Result, bail};
use std::path::Path;

/// Where the old self-update installed its copy.
const OLD_SELF_UPDATE_COPY: &str = "/usr/local/bin/mdots";

/// The advice `mdots self-update` prints. `exe` is the running binary and
/// `old_copy_exists` says whether [`OLD_SELF_UPDATE_COPY`] is on disk.
fn update_advice(exe: Option<&Path>, old_copy_exists: bool) -> String {
    let mut advice = String::from(
        "mdots ships with margo and is updated with it, not on its own:\n  \
         Arch / CachyOS: update the margo-git package (e.g. `paru -Syu margo-git`, or \
         `./install.sh` from a margo checkout)\n  \
         Debian / Ubuntu: re-run `./install.sh` from an updated margo checkout",
    );
    let running_old_copy = exe.is_some_and(|p| p == Path::new(OLD_SELF_UPDATE_COPY));
    if old_copy_exists || running_old_copy {
        advice.push_str(&format!(
            "\n\n{OLD_SELF_UPDATE_COPY} is a copy left by the old `mdots self-update`. It comes \
             before the packaged /usr/bin/mdots on PATH, so updates never reach you. Remove \
             it: `sudo rm {OLD_SELF_UPDATE_COPY}`"
        ));
    }
    advice
}

pub fn run() -> Result<()> {
    let exe = std::env::current_exe().ok();
    let advice = update_advice(exe.as_deref(), Path::new(OLD_SELF_UPDATE_COPY).exists());
    bail!("{advice}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn points_at_the_package_and_never_at_a_git_build() {
        let advice = update_advice(Some(Path::new("/usr/bin/mdots")), false);
        assert!(advice.contains("margo-git"));
        assert!(advice.contains("./install.sh"));
        assert!(!advice.contains("/usr/local/bin"));
        assert!(!advice.contains("cargo"));
    }

    #[test]
    fn flags_a_leftover_local_copy() {
        let advice = update_advice(Some(Path::new("/usr/bin/mdots")), true);
        assert!(advice.contains("sudo rm /usr/local/bin/mdots"));
    }

    #[test]
    fn flags_running_from_the_local_copy() {
        let advice = update_advice(Some(Path::new(OLD_SELF_UPDATE_COPY)), false);
        assert!(advice.contains("sudo rm /usr/local/bin/mdots"));
    }
}
