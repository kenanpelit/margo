//! `mshellctl ufw …` — the UFW firewall from a terminal, through the same
//! pieces the bar pill and menu use: the state is read without root
//! (`/etc/ufw` + `systemctl show ufw.service`), every change runs the
//! audited root helper (`mshell-privileged ufw-*`, one polkit prompt; a
//! terminal polkit agent answers when no graphical one runs).
//!
//! To open the shell's firewall menu, use `mshellctl menu ufw`.

use clap::{Args, Subcommand, ValueEnum};
use mshell_privileged::Request;
use mshell_privileged::ufw::rules::{self, RuleSet};
use mshell_privileged::ufw::spec::{Action, AddForm, Proto};
use mshell_privileged::ufw::state::{self, ReadError, Snapshot, Status, UnitRead};
use mshell_privileged::ufw::{self, Switch, UfwOp};
use std::io::{BufRead, IsTerminal, Write};
use std::path::Path;

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProtoArg {
    Tcp,
    Udp,
    /// TCP and UDP (one rule for a single port, two for a range or list).
    Both,
}

#[derive(Args, Debug)]
pub struct RuleArgs {
    /// Port, range or list: 22, 22/tcp, 1714:1764, 80,443.
    pub port: String,
    /// Protocol (a `/tcp` or `/udp` suffix on PORT picks it too).
    #[arg(long, value_enum, default_value = "both")]
    pub proto: ProtoArg,
    /// Only from this address or network (192.168.1.0/24); default anywhere.
    #[arg(long)]
    pub from: Option<String>,
    /// A label shown next to the rule.
    #[arg(long)]
    pub comment: Option<String>,
}

#[derive(Subcommand, Debug)]
pub enum UfwCommands {
    /// Print the firewall's state and its numbered rules (read without
    /// root; `--root` reads rule files only root can read, one prompt).
    Status {
        #[arg(long)]
        root: bool,
    },
    /// Turn the firewall on (also starts ufw.service when it isn't running).
    Enable,
    /// Turn the firewall off (`ufw disable`: until turned on again, also
    /// after a reboot).
    Disable,
    /// Allow incoming traffic to a port.
    Allow(RuleArgs),
    /// Drop incoming traffic to a port.
    Deny(RuleArgs),
    /// Refuse incoming traffic to a port (the sender is told).
    Reject(RuleArgs),
    /// Allow, but refuse an address that opens 6+ connections in 30 s.
    Limit(RuleArgs),
    /// Remove rule NUMBER as `mshellctl ufw status` numbers it (asks
    /// first unless --yes).
    Delete {
        number: usize,
        #[arg(long)]
        yes: bool,
    },
}

/// The add form a rule command fills in.
pub fn add_form(action: Action, a: &RuleArgs) -> AddForm {
    AddForm {
        action,
        port: a.port.clone(),
        proto: match a.proto {
            ProtoArg::Tcp => Proto::Tcp,
            ProtoArg::Udp => Proto::Udp,
            ProtoArg::Both => Proto::Any,
        },
        from: a.from.clone().unwrap_or_default(),
        comment: a.comment.clone().unwrap_or_default(),
    }
}

fn installed() -> bool {
    margo_fs::path::on_path("ufw") || Path::new("/usr/sbin/ufw").is_file()
}

fn read_unit() -> UnitRead {
    if !margo_fs::path::on_path("systemctl") {
        return UnitRead::NoSystemctl;
    }
    match std::process::Command::new("systemctl")
        .args(ufw::argv::SHOW_ARGS)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
    {
        Ok(out) if out.status.success() => {
            UnitRead::Unit(state::parse_show(&String::from_utf8_lossy(&out.stdout)))
        }
        Ok(out) => UnitRead::Failed(format!("systemctl exited {}", out.status)),
        Err(e) => UnitRead::Failed(format!("systemctl: {e}")),
    }
}

fn snapshot(saved: ufw::Saved) -> Snapshot {
    Snapshot {
        installed: installed(),
        conf: saved.conf,
        defaults: saved.defaults,
        unit: read_unit(),
        rules: saved.rules,
    }
}

/// Run the helper through pkexec with the terminal attached (a text
/// polkit agent can ask there). `capture`: return its stdout.
fn pkexec(op: UfwOp, capture: bool) -> anyhow::Result<String> {
    let req = Request::Ufw(op)
        .validated()
        .map_err(|e| anyhow::anyhow!("{}", e.message()))?;
    if !mshell_privileged::installed() {
        anyhow::bail!("{}", mshell_privileged::MISSING);
    }
    if !margo_fs::path::on_path("pkexec") {
        anyhow::bail!("pkexec isn't installed (package polkit)");
    }
    let mut cmd = std::process::Command::new("pkexec");
    cmd.args(mshell_privileged::lossy(&req.argv()));
    // The terminal stays attached: a text polkit agent asks there, and
    // the helper's own messages (ufw's output, its error line) show.
    let (code, stdout) = if capture {
        let out = cmd
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .output()?;
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
        )
    } else {
        (cmd.status()?.code(), String::new())
    };
    match code {
        Some(0) => Ok(stdout),
        Some(126) => anyhow::bail!("authorization was cancelled"),
        Some(127) => anyhow::bail!("authorization failed"),
        Some(c) => anyhow::bail!("the firewall change failed (exit {c})"),
        None => anyhow::bail!("the helper was killed"),
    }
}

/// The saved state: the files, or (`root`, or when the files are
/// root-only) the helper's dump.
fn saved(root: bool) -> anyhow::Result<ufw::Saved> {
    let files = ufw::read_saved(Path::new("/"));
    if !root && files.rules != Err(ReadError::Denied) {
        return Ok(files);
    }
    let out = pkexec(UfwOp::Status, true)?;
    let dump = rules::parse_dump(&out)
        .ok_or_else(|| anyhow::anyhow!("the helper's answer was cut short"))?;
    Ok(ufw::saved_from_dump(&dump))
}

/// `status`'s text: the state, the policies, the numbered rules.
pub fn render_status(snap: &Snapshot) -> String {
    let h = state::classify(snap);
    let mut out = format!("UFW: {}\n", h.headline);
    if !h.detail.is_empty() {
        out.push_str(&format!("  {}\n", h.detail));
    }
    match h.status {
        Status::NotRunning | Status::Failed => {
            out.push_str("  fix: mshellctl ufw enable (starts ufw.service)\n");
        }
        Status::OnNotAtBoot => out.push_str("  fix: sudo systemctl enable ufw.service\n"),
        _ => {}
    }
    if let Some(d) = &snap.defaults {
        let p = |v: &Option<String>| v.as_deref().map_or("?", rules::policy_word);
        out.push_str(&format!(
            "Default: incoming {}, outgoing {}, routed {}\n",
            p(&d.input),
            p(&d.output),
            p(&d.forward)
        ));
    }
    if let Some(u) = snap.unit() {
        out.push_str(&format!(
            "Start at boot: {}\n",
            if u.starts_at_boot() { "yes" } else { "no" }
        ));
    }
    match &snap.rules {
        Ok(set) if set.rules.is_empty() => out.push_str("No rules.\n"),
        Ok(set) => out.push_str(&rule_table(set)),
        Err(ReadError::Denied) => {
            out.push_str("The rule files need root to read: mshellctl ufw status --root\n")
        }
        Err(ReadError::Missing) => out.push_str("No rule file (/etc/ufw/user.rules).\n"),
        Err(ReadError::Other(e)) => out.push_str(&format!("The rules can't be read: {e}\n")),
    }
    out
}

fn rule_table(set: &RuleSet) -> String {
    let width = set
        .rules
        .iter()
        .map(|r| r.target().len())
        .max()
        .unwrap_or(0);
    let mut out = String::new();
    let mut fixed = false;
    for (i, r) in set.rules.iter().enumerate() {
        let mark = if r.delete_spec(set.complete).is_some() {
            ' '
        } else {
            fixed = true;
            '*'
        };
        let mut sub = r.summary(set.ipv6);
        if let Some(c) = &r.comment {
            sub = format!("{sub} · {c}");
        }
        out.push_str(&format!(
            "{:>3}{mark} {:<width$}  {sub}\n",
            i + 1,
            r.target()
        ));
    }
    if fixed {
        out.push_str("  * remove with `sudo ufw delete` (route, app or source-port rule)\n");
    }
    out
}

fn confirm(question: &str) -> anyhow::Result<bool> {
    if !std::io::stdin().is_terminal() {
        anyhow::bail!("{question} — pass --yes to remove it without asking");
    }
    print!("{question} [y/N] ");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    Ok(matches!(line.trim(), "y" | "Y" | "yes"))
}

pub async fn execute(command: UfwCommands) -> anyhow::Result<()> {
    if !installed() {
        anyhow::bail!("ufw isn't installed (package ufw)");
    }
    match command {
        UfwCommands::Status { root } => {
            print!("{}", render_status(&snapshot(saved(root)?)));
        }
        UfwCommands::Enable => {
            pkexec(UfwOp::Switch(Switch::On), false)?;
        }
        UfwCommands::Disable => {
            pkexec(UfwOp::Switch(Switch::Off), false)?;
        }
        UfwCommands::Allow(a) => add(Action::Allow, &a)?,
        UfwCommands::Deny(a) => add(Action::Deny, &a)?,
        UfwCommands::Reject(a) => add(Action::Reject, &a)?,
        UfwCommands::Limit(a) => add(Action::Limit, &a)?,
        UfwCommands::Delete { number, yes } => {
            let set = saved(false)?
                .rules
                .map_err(|e| anyhow::anyhow!("the rules can't be read ({e:?})"))?;
            let rule = number
                .checked_sub(1)
                .and_then(|i| set.rules.get(i))
                .ok_or_else(|| {
                    anyhow::anyhow!("no rule {number} (there are {})", set.rules.len())
                })?;
            let spec = rule.delete_spec(set.complete).ok_or_else(|| {
                anyhow::anyhow!(
                    "rule {number} ({}) can't be removed from here; use `sudo ufw delete`",
                    rule.target()
                )
            })?;
            let question = format!("Remove {} ({})?", rule.target(), rule.summary(set.ipv6));
            if yes || confirm(&question)? {
                pkexec(UfwOp::Delete(spec), false)?;
            }
        }
    }
    Ok(())
}

fn add(action: Action, a: &RuleArgs) -> anyhow::Result<()> {
    let add = add_form(action, a)
        .validate()
        .map_err(|e| anyhow::anyhow!("{}", e.message))?;
    pkexec(UfwOp::Add(add), false)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mshell_privileged::ufw::rules::{Conf, parse_defaults, parse_rules};

    #[test]
    fn rule_args_go_through_the_helpers_validator() {
        let a = RuleArgs {
            port: "1714:1764".into(),
            proto: ProtoArg::Both,
            from: Some("192.168.0.0/24".into()),
            comment: Some("KDE Connect".into()),
        };
        let add = add_form(Action::Allow, &a).validate().unwrap();
        let argv = mshell_privileged::lossy(&Request::Ufw(UfwOp::Add(add)).argv());
        assert_eq!(
            argv[1..],
            [
                "ufw-add",
                "allow",
                "in",
                "-",
                "-",
                "any",
                "192.168.0.0/24",
                "any",
                "1714:1764",
                "KDE Connect"
            ]
        );
        let bad = RuleArgs {
            port: "22".into(),
            proto: ProtoArg::Udp,
            from: None,
            comment: Some("a;b".into()),
        };
        assert!(add_form(Action::Deny, &bad).validate().is_err());
    }

    #[test]
    fn status_numbers_the_rules() {
        let snap = Snapshot {
            installed: true,
            conf: Ok(Conf {
                enabled: Some(true),
                loglevel: None,
            }),
            defaults: Some(parse_defaults(
                "DEFAULT_INPUT_POLICY=\"DROP\"\nDEFAULT_OUTPUT_POLICY=\"ACCEPT\"\nDEFAULT_FORWARD_POLICY=\"DROP\"\n",
            )),
            unit: UnitRead::Unit(state::parse_show(
                "LoadState=loaded\nActiveState=active\nUnitFileState=enabled\n",
            )),
            rules: Ok(parse_rules(
                "### tuple ### allow tcp 22 0.0.0.0/0 any 0.0.0.0/0 in comment=535348\n\
                 ### tuple ### allow tcp 80 0.0.0.0/0 any 0.0.0.0/0 WWW - in\n### END RULES ###\n",
                None,
                false,
            )),
        };
        assert_eq!(
            render_status(&snap),
            "UFW: On\n\
             Default: incoming deny, outgoing allow, routed deny\n\
             Start at boot: yes\n  \
             1  22/tcp  Allow in · SSH\n  \
             2* WWW     Allow in\n  \
             * remove with `sudo ufw delete` (route, app or source-port rule)\n"
        );
    }
}
