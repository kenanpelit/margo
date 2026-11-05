//! Tests for the SSH-sessions `~/.ssh/config` and `pgrep -af` parsers.

use super::*;

fn names(hosts: &[SshHost]) -> Vec<&str> {
    hosts.iter().map(|h| h.name.as_str()).collect()
}

fn host(name: &str, hostname: &str, user: &str) -> SshHost {
    SshHost {
        name: name.into(),
        hostname: hostname.into(),
        user: user.into(),
        port: String::new(),
    }
}

#[test]
fn parses_host_blocks() {
    let hosts = parse_hosts(
        "# comment\n\nHost agu_admin\n  User root\n  # HostName 10.0.0.1\n\n\
         Host draco d2\n  Port 45652\n  User root\n  HostName draco.example\n\n\
         Host=vhay\n  Hostname = localhost\n  Port=2288\n\n\
         HOST Upper\n  HOSTNAME Up.Example\n",
    );
    assert_eq!(names(&hosts), ["agu_admin", "draco", "vhay", "Upper"]);
    assert_eq!(hosts[0].subtitle(), "root@agu_admin");
    assert_eq!(hosts[1].subtitle(), "root@draco.example:45652");
    assert_eq!(hosts[2].subtitle(), "localhost:2288");
    assert_eq!(hosts[3].hostname, "Up.Example", "values keep their case");
}

#[test]
fn wildcard_blocks_are_skipped_with_their_keys() {
    let hosts = parse_hosts(
        "Host a\n  User ua\nHost *\n  User everyone\n  Port 2200\nHost *.lan\n  User lan\n",
    );
    assert_eq!(names(&hosts), ["a"]);
    assert_eq!(hosts[0].user, "ua");
    assert!(hosts[0].port.is_empty());
}

/// Negated and wildcard patterns never name a host; the first literal
/// alias on the line does.
#[test]
fn host_line_named_by_first_literal_alias() {
    let hosts =
        parse_hosts("Host !bad *.lan good other\n  User u\nHost !only\n  User x\nHost h?st\n");
    assert_eq!(names(&hosts), ["good"]);
    assert_eq!(hosts[0].user, "u");
}

/// ssh_config(5): "for each parameter, the first obtained value will be
/// used" — a later `HostName` in the same block does not win.
#[test]
fn first_value_wins() {
    let hosts = parse_hosts("Host d\n  HostName first.example\n  HostName second.example\n");
    assert_eq!(hosts[0].hostname, "first.example");
}

/// A `Match` block ends the previous `Host` block: its keys must not
/// be shown as that host's settings.
#[test]
fn match_block_does_not_leak_into_previous_host() {
    let hosts =
        parse_hosts("Host a\n  HostName a.example\nMatch host b\n  User matchuser\n  Port 9\n");
    assert_eq!(names(&hosts), ["a"]);
    assert_eq!(hosts[0].user, "");
    assert_eq!(hosts[0].port, "");
}

#[test]
fn keys_before_any_host_are_global() {
    let hosts = parse_hosts("User global\nHost a\n  HostName a.example\n");
    assert_eq!(hosts[0].user, "");
}

#[test]
fn targets_from_pgrep() {
    let out = "\
1201 ssh draco
1202 /usr/bin/ssh -p 2222 -i /home/k/.ssh/id_ed25519 root@vhay
1203 ssh -o ServerAliveInterval=30 agu_admin
1204 ssh-agent -D
1205 /usr/bin/sshd -D
1206 kitty -e ssh draco
1207 ssh -W localhost:22 jump
1208 ssh draco
1209 /usr/bin/bash -c source /tmp/x.sh; echo ssh done
";
    assert_eq!(
        parse_targets(out),
        ["draco", "root@vhay", "agu_admin"],
        "deduped, noise filtered, option values skipped"
    );
}

/// `ssh [options] destination [command]`: the destination is the first
/// operand, not the last word of a remote command.
#[test]
fn target_is_the_destination_not_the_remote_command() {
    assert_eq!(parse_targets("1 ssh draco uptime\n"), ["draco"]);
    assert_eq!(parse_targets("1 ssh -t draco htop -d 10\n"), ["draco"]);
    assert_eq!(parse_targets("1 ssh -l root -p 2222 draco\n"), ["draco"]);
    assert_eq!(
        parse_targets("1 ssh -J bastion -i key draco tmux attach\n"),
        ["draco"]
    );
    assert_eq!(parse_targets("1 ssh -4 -A -- draco\n"), ["draco"]);
    assert_eq!(parse_targets("1 ssh -p2222 draco ls\n"), ["draco"]);
    assert_eq!(
        parse_targets("1 ssh git@github.com git-upload-pack 'k/margo.git'\n"),
        ["git@github.com"]
    );
    assert!(parse_targets("1 ssh -V\n").is_empty());
}

#[test]
fn targets_match_hosts_by_alias_hostname_or_user() {
    let h = host("draco", "draco.example", "root");
    assert!(host_is_active(&h, &["draco".into()]));
    assert!(host_is_active(&h, &["draco.example".into()]));
    assert!(host_is_active(&h, &["root@draco.example".into()]));
    assert!(host_is_active(&h, &["root@draco".into()]));
    assert!(!host_is_active(
        &h,
        &["other@draco".into(), "dracon".into()]
    ));
}
