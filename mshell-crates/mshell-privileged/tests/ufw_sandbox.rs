//! The real helper driving the real `ufw` inside a throwaway bwrap
//! sandbox: a private copy of /etc/ufw bind-mounted over the real one,
//! a private network namespace, fake root in a user namespace, and a
//! fake `systemctl` so nothing can reach the host's systemd. The host's
//! rules are never read or changed (checked at the end).
//!
//! Proves the argv the helper builds is what ufw 0.36 accepts: every
//! add lands in the rule files, a duplicate is reported, and the delete
//! [`Rule::delete_spec`] rebuilds from the parsed files removes exactly
//! that rule (both IPv4 and IPv6 copies, or the one that exists).
//!
//! Skips without bwrap or ufw, or as root.

use mshell_privileged::ufw::rules::{self, Rule};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_mshell-privileged");

fn available() -> bool {
    // SAFETY: plain getters.
    let root = unsafe { libc::getuid() == 0 || libc::geteuid() == 0 };
    !root
        && Path::new("/usr/bin/bwrap").is_file()
        && Path::new("/usr/bin/ufw").is_file()
        && Path::new("/etc/default/ufw").is_file()
        && Path::new("/etc/ufw").is_dir()
}

const USER_RULES: &str = "*filter\n:ufw-user-input - [0:0]\n:ufw-user-output - [0:0]\n:ufw-user-forward - [0:0]\n:ufw-user-limit - [0:0]\n:ufw-user-limit-accept - [0:0]\n### RULES ###\n### END RULES ###\nCOMMIT\n";
const USER6_RULES: &str = "*filter\n:ufw6-user-input - [0:0]\n:ufw6-user-output - [0:0]\n:ufw6-user-forward - [0:0]\n### RULES ###\n### END RULES ###\nCOMMIT\n";
const DEFAULTS: &str = "IPV6=yes\nDEFAULT_INPUT_POLICY=\"DROP\"\nDEFAULT_OUTPUT_POLICY=\"ACCEPT\"\nDEFAULT_FORWARD_POLICY=\"DROP\"\nDEFAULT_APPLICATION_POLICY=\"SKIP\"\nMANAGE_BUILTINS=no\nIPT_SYSCTL=/etc/ufw/sysctl.conf\nIPT_MODULES=\"\"\n";

struct Sandbox {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl Sandbox {
    fn new() -> Sandbox {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().to_path_buf();
        let etc = root.join("etc-ufw");
        std::fs::create_dir_all(&etc).expect("mkdir");
        std::fs::write(etc.join("ufw.conf"), "ENABLED=no\nLOGLEVEL=low\n").expect("conf");
        std::fs::write(etc.join("user.rules"), USER_RULES).expect("v4");
        std::fs::write(etc.join("user6.rules"), USER6_RULES).expect("v6");
        std::fs::write(root.join("default-ufw"), DEFAULTS).expect("defaults");
        // The stock framework rules (world-readable), never the user's.
        for entry in std::fs::read_dir("/etc/ufw").expect("/etc/ufw").flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let stock = (name.starts_with("before") || name.starts_with("after"))
                && name.ends_with(".rules")
                || name == "sysctl.conf";
            if stock && entry.path().is_file() {
                std::fs::copy(entry.path(), etc.join(&name)).expect("copy stock rules");
            }
        }
        std::fs::create_dir_all(etc.join("applications.d")).expect("apps");
        let fake = root.join("systemctl");
        std::fs::write(&fake, "#!/bin/sh\necho LoadState=not-found\nexit 0\n").expect("fake");
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        Sandbox { _dir: dir, root }
    }

    /// Run the helper once per `steps` entry, in ONE sandbox (one
    /// network namespace, so `ufw enable` holds for the later steps).
    /// Returns (exit code, stdout+stderr) per step.
    fn run(&self, steps: &[Vec<String>]) -> Vec<(i32, String)> {
        let quote = |s: &str| format!("'{}'", s.replace('\'', "'\\''"));
        let mut script = String::new();
        for step in steps {
            let words: Vec<String> = step.iter().map(|w| quote(w)).collect();
            script.push_str(&format!(
                "{} {} 2>&1; echo \"@@rc $?\"\n",
                quote(BIN),
                words.join(" ")
            ));
        }
        let out = Command::new("bwrap")
            .args(["--dev-bind", "/", "/"])
            .arg("--bind")
            .arg(self.root.join("etc-ufw"))
            .arg("/etc/ufw")
            .arg("--bind")
            .arg(self.root.join("default-ufw"))
            .arg("/etc/default/ufw")
            .arg("--ro-bind")
            .arg(self.root.join("systemctl"))
            .arg("/usr/bin/systemctl")
            .args(["--tmpfs", "/run"])
            .args([
                "--unshare-user",
                "--uid",
                "0",
                "--gid",
                "0",
                "--unshare-net",
            ])
            .args(["--cap-add", "ALL", "--die-with-parent"])
            .args(["sh", "-c", &script])
            .env_remove("MSHELL_PRIVILEGED_SYSROOT")
            .output()
            .expect("bwrap");
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        let mut res = Vec::new();
        let mut buf = String::new();
        for line in text.lines() {
            if let Some(rc) = line.strip_prefix("@@rc ") {
                res.push((rc.parse().unwrap_or(-1), std::mem::take(&mut buf)));
            } else {
                buf.push_str(line);
                buf.push('\n');
            }
        }
        assert_eq!(
            res.len(),
            steps.len(),
            "every step ran: {text}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        res
    }

    fn rules(&self) -> rules::RuleSet {
        let v4 = std::fs::read_to_string(self.root.join("etc-ufw/user.rules")).expect("v4");
        let v6 = std::fs::read_to_string(self.root.join("etc-ufw/user6.rules")).expect("v6");
        rules::parse_rules(&v4, Some(&v6), true)
    }
}

fn step(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

fn add(fields: &[&str]) -> Vec<String> {
    let mut v = vec!["ufw-add"];
    v.extend(fields);
    step(&v)
}

#[test]
fn the_helper_drives_real_ufw() {
    if !available() {
        eprintln!("skipped: needs bwrap and ufw, as an ordinary user");
        return;
    }
    let real = std::fs::metadata("/etc/ufw/user.rules")
        .and_then(|m| m.modified())
        .ok();
    let sb = Sandbox::new();

    let adds = [
        add(&["allow", "in", "-", "-", "tcp", "any", "any", "22", "SSH"]),
        add(&[
            "allow",
            "in",
            "-",
            "-",
            "any",
            "192.168.0.0/24",
            "any",
            "1714:1764",
            "KDE Connect",
        ]),
        add(&["deny", "in", "-", "-", "udp", "any", "any", "5353", "-"]),
        add(&[
            "reject",
            "in",
            "-",
            "-",
            "tcp",
            "2001:db8::/32",
            "any",
            "8080",
            "Yazıcı ağ",
        ]),
        add(&[
            "allow",
            "in",
            "-",
            "-",
            "any",
            "10.1.2.3",
            "any",
            "53317",
            "LocalSend",
        ]),
        add(&["allow", "in", "-", "-", "tcp", "any", "any", "80,443", "-"]),
        add(&[
            "allow", "in", "eth9", "-", "any", "any", "any", "43117", "-",
        ]),
        add(&["allow", "out", "-", "-", "udp", "any", "any", "53", "-"]),
    ];
    let mut steps = vec![step(&["ufw-switch", "on"])];
    steps.extend(adds.iter().cloned());
    steps.push(adds[0].clone()); // a duplicate
    steps.push(step(&["ufw-status"]));
    let res = sb.run(&steps);
    for (i, (rc, out)) in res.iter().enumerate().take(adds.len() + 1) {
        assert_eq!(*rc, 0, "step {i} {:?}: {out}", steps[i]);
    }
    let (rc, out) = &res[adds.len() + 1];
    assert_eq!(*rc, 1, "the duplicate add is reported: {out}");
    assert!(
        out.trim_end().ends_with("That rule already exists"),
        "{out}"
    );

    let set = sb.rules();
    assert!(set.complete);
    let shown: Vec<String> = set
        .rules
        .iter()
        .map(|r| format!("{} | {}", r.target(), r.summary(true)))
        .collect();
    assert_eq!(
        shown,
        [
            "22/tcp | Allow in",
            "1714:1764/tcp | Allow in · from 192.168.0.0/24",
            "1714:1764/udp | Allow in · from 192.168.0.0/24",
            "5353/udp | Deny in",
            "53317 | Allow in · from 10.1.2.3",
            "80,443/tcp | Allow in",
            "43117 | Allow in · on eth9",
            "53/udp | Allow out",
            "8080/tcp | Reject in · from 2001:db8::/32",
        ]
    );
    assert_eq!(set.rules[0].comment.as_deref(), Some("SSH"));
    assert_eq!(set.rules[8].comment.as_deref(), Some("Yazıcı ağ"));
    // The root dump says the same as the files.
    let (_, status) = res.last().expect("status");
    let dump = rules::parse_dump(status).expect("dump");
    assert_eq!(
        mshell_privileged::ufw::saved_from_dump(&dump).rules,
        Ok(set.clone())
    );

    // Delete each rule by the spec rebuilt from the files: the set
    // shrinks by exactly that rule every time.
    let deletes: Vec<(Rule, Vec<String>)> = set
        .rules
        .iter()
        .map(|r| {
            let spec = r.delete_spec(set.complete).expect("deletable");
            let mut v = vec!["ufw-delete".to_string()];
            v.extend(spec.fields());
            (r.clone(), v)
        })
        .collect();
    let mut steps = vec![step(&["ufw-switch", "on"])];
    steps.extend(deletes.iter().map(|(_, v)| v.clone()));
    let res = sb.run(&steps);
    for (i, (rc, out)) in res.iter().enumerate() {
        assert_eq!(*rc, 0, "step {i} {:?}: {out}", steps[i]);
    }
    assert_eq!(
        sb.rules().rules,
        Vec::<Rule>::new(),
        "every rule is gone, nothing else"
    );

    // One copy at a time: deleting a both-family rule by its v4-only
    // spec leaves exactly the v6 copy.
    let res = sb.run(&[
        step(&["ufw-switch", "on"]),
        add(&["allow", "in", "-", "-", "tcp", "any", "any", "22", "-"]),
        step(&[
            "ufw-delete",
            "allow",
            "in",
            "-",
            "-",
            "tcp",
            "0.0.0.0/0",
            "0.0.0.0/0",
            "22",
        ]),
        step(&["ufw-switch", "off"]),
    ]);
    assert!(res.iter().all(|(rc, _)| *rc == 0), "{res:?}");
    let left = sb.rules();
    assert_eq!(left.rules.len(), 1);
    assert!(!left.rules[0].v4 && left.rules[0].v6);
    assert_eq!(left.rules[0].summary(true), "Allow in · IPv6 only");
    let conf = std::fs::read_to_string(sb.root.join("etc-ufw/ufw.conf")).expect("conf");
    assert_eq!(
        rules::parse_conf(&conf).enabled,
        Some(false),
        "switch off wrote ENABLED=no"
    );

    let after = std::fs::metadata("/etc/ufw/user.rules")
        .and_then(|m| m.modified())
        .ok();
    assert_eq!(real, after, "the host's rules were not touched");
}
