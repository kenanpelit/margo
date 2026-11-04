//! man/mctl.1 documents every subcommand `mctl --help` lists, and the
//! checked-in completions match what the binary generates.

use std::process::Command;

fn mctl(args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_mctl"))
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "mctl {args:?} failed");
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn man_page_lists_every_subcommand() {
    let help = mctl(&["--help"]);
    let man = include_str!("../../man/mctl.1");
    let mut missing = Vec::new();
    let commands = help
        .lines()
        .skip_while(|l| !l.starts_with("Commands:"))
        .skip(1)
        .take_while(|l| l.starts_with("  "));
    for line in commands {
        let name = line.split_whitespace().next().unwrap_or("");
        if name.is_empty() || name == "help" {
            continue;
        }
        let documented = man.lines().any(|l| {
            l.strip_prefix(".B ")
                .is_some_and(|rest| rest.split_whitespace().next() == Some(name))
        });
        if !documented {
            missing.push(name.to_string());
        }
    }
    assert!(missing.is_empty(), "man/mctl.1 lacks: {missing:?}");
}

#[test]
fn checked_in_completions_are_current() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../contrib/completions");
    for (shell, file) in [
        ("bash", "mctl.bash"),
        ("zsh", "_mctl"),
        ("fish", "mctl.fish"),
    ] {
        let generated = mctl(&["completions", shell]);
        let checked_in = std::fs::read_to_string(format!("{dir}/{file}")).unwrap();
        assert!(
            generated == checked_in,
            "contrib/completions/{file} is stale — run contrib/completions/generate.sh"
        );
    }
}
