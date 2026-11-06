//! One-click AI diagnosis for a crash or a journal error.
//!
//! The diagnose button writes a prompt file and opens the user's terminal
//! ([`mshell_process::terminal`]) on a small fixed `sh` script that picks
//! an agent and becomes it:
//!
//! * The prompt (facts, `coredumpctl info` capped at [`INFO_CAP`], our
//!   investigation guide, the user's extra instructions) goes to
//!   `$XDG_RUNTIME_DIR/mshell/crash-diagnose/*.md` — dir 0700, file 0600,
//!   pruned after a day. Only its *path* reaches argv: the command line
//!   and stack of a crashed program are nobody else's business.
//! * The script tries `crashes.agent_order` (Claude Code, Codex). With
//!   `crashes.agent_probe` on it first asks each a throwaway question
//!   and skips one that fails or answers with a quota / rate-limit
//!   message, so an exhausted subscription falls through to the next
//!   agent instead of opening a dead session. The choice happens in the
//!   terminal, so the user sees which agent was picked and why.
//! * The data (the script, its argv, the prompt) is built by pure
//!   functions tested below — the script itself against fake agents.

use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use super::{Crash, JournalEntry, format_time, signal_name};

/// Bytes of `coredumpctl info` kept in the prompt (its head: the facts and
/// the crashing thread's stack come first; a big C++ program prints 50 KiB+).
pub(crate) const INFO_CAP: usize = 16 * 1024;
/// How long `coredumpctl info` may take before the prompt goes without it.
const INFO_TIMEOUT: Duration = Duration::from_secs(15);
/// Prompt files older than this are deleted when a new one is written.
const PROMPT_MAX_AGE: Duration = Duration::from_secs(24 * 3600);

/// Where agent installers put their binary, relative to `$HOME`. mshell
/// runs under systemd, whose PATH has none of these; the script prepends
/// the same list.
pub(crate) const AGENT_DIRS: &[&str] = &[
    ".local/bin",
    ".local/share/npm/bin",
    ".npm-global/bin",
    ".bun/bin",
];

/// The terminal-side picker. `$1` prompt file, `$2` `1` to probe, `$3…`
/// the agents in order. Fixed text: nothing user-controlled is spliced in.
pub(crate) const SCRIPT: &str = r#"export PATH="$HOME/.local/bin:$HOME/.local/share/npm/bin:$HOME/.npm-global/bin:$HOME/.bun/bin:$PATH"
file=$1; probe=$2; shift 2
cd "$HOME" 2>/dev/null
ask="Diagnose what this file describes: $file. Read it first and follow its guide; investigate read-only."
quota='usage limit|rate limit|rate_limit|quota|limit reached|hit your limit|too many requests|insufficient credit|credit balance'
for agent in "$@"; do
  command -v "$agent" >/dev/null 2>&1 || continue
  if [ "$probe" = 1 ]; then
    printf 'Checking that %s can answer...\n' "$agent"
    case $agent in
      claude) out=$(timeout 60 claude -p 'Reply with the single word OK.' </dev/null 2>&1) ;;
      codex) out=$(timeout 90 codex exec --skip-git-repo-check 'Reply with the single word OK.' </dev/null 2>&1) ;;
      *) continue ;;
    esac
    rc=$?
    if [ "$rc" -ne 0 ] || printf '%s\n' "$out" | grep -Eqi "$quota"; then
      printf '%s cannot answer right now: %s\nTrying the next agent.\n\n' "$agent" "$(printf '%s\n' "$out" | grep -v '^[[:space:]]*$' | tail -n 1 | cut -c1-200)"
      continue
    fi
  fi
  printf 'Starting %s on %s\n' "$agent" "$file"
  case $agent in
    claude) exec claude "$ask" ;;
    codex) exec codex "$ask" ;;
  esac
done
printf '\nNo AI agent could take this (tried: %s).\nInstall Claude Code or Codex and sign in. The prompt is in %s\n' "$*" "$file"
printf 'Press Enter to close.'
read -r _
"#;

/// argv (for the terminal) that runs [`SCRIPT`] on `file`.
pub(crate) fn launch_command(file: &Path, probe: bool, agents: &[&str]) -> Vec<String> {
    let mut v: Vec<String> = vec![
        "sh".into(),
        "-c".into(),
        SCRIPT.into(),
        "mshell-diagnose".into(),
        file.to_string_lossy().into_owned(),
        if probe { "1" } else { "0" }.into(),
    ];
    v.extend(agents.iter().map(|a| a.to_string()));
    v
}

/// `agent` on PATH or in one of [`AGENT_DIRS`] under `home`.
pub(crate) fn agent_installed_in(agent: &str, home: Option<&Path>) -> bool {
    margo_fs::path::on_path(agent)
        || home.is_some_and(|h| {
            AGENT_DIRS
                .iter()
                .any(|d| margo_fs::path::is_executable(&h.join(d).join(agent)))
        })
}

/// Any of `agents` is installed (the button is disabled otherwise).
pub(crate) fn any_agent_installed(agents: &[&str]) -> bool {
    let home = margo_fs::xdg::home();
    agents
        .iter()
        .any(|a| agent_installed_in(a, home.as_deref()))
}

/// `text` cut to at most `cap` bytes on a char boundary, with a note when
/// something was dropped.
pub(crate) fn cap_text(text: &str, cap: usize) -> String {
    if text.len() <= cap {
        return text.trim_end().to_string();
    }
    let mut end = cap;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{}\n[… {} more bytes cut; run the command yourself for the rest]",
        text[..end].trim_end(),
        text.len() - end
    )
}

/// A Markdown code fence longer than any backtick run inside `text`.
fn fence_for(text: &str) -> String {
    let mut longest = 0;
    let mut run = 0;
    for c in text.chars() {
        run = if c == '`' { run + 1 } else { 0 };
        longest = longest.max(run);
    }
    "`".repeat((longest + 1).max(3))
}

fn code_block(out: &mut String, text: &str) {
    let fence = fence_for(text);
    out.push_str(&format!("{fence}text\n{text}\n{fence}\n"));
}

const CRASH_GUIDE: &str = "\
## How to investigate

Work from evidence: the aim is an honest account of what happened, not a
plausible story.

1. **Facts.** Start from the `coredumpctl info` output above. The command
   line often says what the program was busy with. `coredumpctl list` shows
   whether this program crashes repeatedly, or whether several programs died
   together (that points at the system, not the program). If the info above
   shows a different time than the one recorded, find the right dump in
   `coredumpctl list` first.
2. **Boring causes first.** Memory pressure and OOM kills (`journalctl -k`
   around the crash time, `free -h`), a full disk, and a package update
   shortly before the crash (`/var/log/pacman.log` on Arch).
3. **Timeline.** Read the journal a few minutes either side of the crash
   (`journalctl --since … --until …`) for related warnings from the same or
   neighbouring processes. Files modified in the same second can show what
   triggered it.
4. **Whole core, not just frame 0.** The other threads' stacks show what
   work was in flight. Note third-party code loaded into the process
   (plugins, extensions, out-of-tree drivers), but do not blame it without
   evidence.
5. **Symbolise when possible.** Extract the core to a fresh `mktemp` file
   (`coredumpctl dump <pid> --output=\"$core\"`), run
   `gdb -batch -ex 'set debuginfod enabled on' -ex 'thread apply all bt' <exe> \"$core\"`
   with `DEBUGINFOD_URLS` set (Arch: https://debuginfod.archlinux.org), and
   delete the core afterwards: it is a verbatim copy of the process's memory
   and may hold passwords or tokens. If symbols are not available, say so;
   never invent function names.

## Report

1. What crashed and what it was doing at the time.
2. The most likely mechanism, keeping what the evidence proves apart from
   what you infer.
3. Whether user data was lost and where it may be recovered from.
4. Whether it is likely to happen again, and what would avoid or fix it.

If the cause stays ambiguous, say so.

**Read-only.** Diagnosis does not fix, tidy or reconfigure anything; the
only thing to clean up is the core you extracted. Do not mute anything
yourself — if the user is tired of this program's crash notifications, the
bell button on its row in the Crashes menu mutes it.
";

const JOURNAL_GUIDE: &str = "\
## How to investigate

Work from evidence, not guesswork:

1. Pull the surrounding context: `journalctl -b` filtered by
   `SYSLOG_IDENTIFIER=` / `_COMM=` for this source, and the whole journal a
   few minutes either side of the time above (`--since` / `--until`).
2. Check whether the message repeats across this boot or earlier ones
   (`journalctl -b -1`), and which unit logged it (`_SYSTEMD_UNIT`).
3. Correlate with related units, hardware events and recent package updates
   (`/var/log/pacman.log` on Arch).

Then explain what the message means, how serious it is, and what — if
anything — should be done, keeping what the evidence proves apart from what
you infer.

**Read-only.** Diagnosis does not fix, tidy or reconfigure anything.
";

fn push_instructions(out: &mut String, instructions: &str) {
    let instructions = instructions.trim();
    if !instructions.is_empty() {
        out.push_str("\n## Extra instructions from the user\n\n");
        out.push_str(instructions);
        out.push('\n');
    }
}

/// The prompt for a crash. `info` is `coredumpctl info` output (already
/// capped) or why it could not be read.
pub(crate) fn crash_prompt(c: &Crash, info: Result<&str, &str>, instructions: &str) -> String {
    let mut out = String::from(
        "# Crash diagnosis\n\nA process crashed on this machine and I want to know why.\n\n\
         ## What systemd-coredump recorded\n\n",
    );
    out.push_str(&format!("- program: {}\n", c.program()));
    out.push_str(&format!("- binary: {}\n", c.exe));
    out.push_str(&format!("- PID: {}\n", c.pid));
    out.push_str(&format!("- signal: {} ({})\n", signal_name(c.sig), c.sig));
    out.push_str(&format!(
        "- time: {} (unix {} s)\n",
        format_time(c.time_us),
        c.time_us / 1_000_000
    ));
    out.push_str(if c.core_present {
        "- core file: present\n"
    } else {
        "- core file: gone (rotated away) — gdb on the core is not possible; the \
         stack trace systemd-coredump logged at crash time is in the info below\n"
    });
    out.push_str(&format!("\n## `coredumpctl info {}`\n\n", c.pid));
    match info {
        Ok(text) => code_block(&mut out, text),
        Err(why) => out.push_str(&format!(
            "It could not be read here ({why}); run `coredumpctl info {}` yourself.\n",
            c.pid
        )),
    }
    out.push('\n');
    out.push_str(CRASH_GUIDE);
    push_instructions(&mut out, instructions);
    out
}

/// The prompt for a journal entry.
pub(crate) fn journal_prompt(e: &JournalEntry, priority: &str, instructions: &str) -> String {
    let mut out = String::from(
        "# Journal error diagnosis\n\nAn error appeared in the systemd journal on this \
         machine and I want to understand it.\n\n## The entry\n\n",
    );
    out.push_str(&format!("- source: {}\n", e.ident));
    out.push_str(&format!("- priority: {priority} ({})\n", e.priority));
    out.push_str(&format!(
        "- time: {} (unix {} s, `__REALTIME_TIMESTAMP={}`)\n",
        format_time(e.time_us),
        e.time_us / 1_000_000,
        e.time_us
    ));
    out.push_str("- message:\n\n");
    code_block(&mut out, &e.message);
    out.push('\n');
    out.push_str(JOURNAL_GUIDE);
    push_instructions(&mut out, instructions);
    out
}

/// `$XDG_RUNTIME_DIR/mshell/crash-diagnose` (never a shared /tmp: the
/// prompt holds a crashed program's command line).
pub(crate) fn prompt_dir() -> Option<PathBuf> {
    margo_fs::xdg::runtime_dir().map(|d| d.join("mshell").join("crash-diagnose"))
}

/// Write `text` to a new 0600 file `<stem>-<ms>.md` in `dir` (created 0700),
/// after deleting prompt files older than [`PROMPT_MAX_AGE`].
pub(crate) fn write_prompt(dir: &Path, stem: &str, text: &str) -> std::io::Result<PathBuf> {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    // An older mshell (or the user) may have made it wider.
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    prune(dir, SystemTime::now());
    let ms = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let stem: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let path = dir.join(format!("{stem}-{ms}.md"));
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)?;
    f.write_all(text.as_bytes())?;
    Ok(path)
}

/// Delete `*.md` files in `dir` last modified before `now - PROMPT_MAX_AGE`.
fn prune(dir: &Path, now: SystemTime) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let path = e.path();
        let old = e
            .metadata()
            .ok()
            .filter(|m| m.is_file())
            .and_then(|m| m.modified().ok())
            .and_then(|t| now.duration_since(t).ok())
            .is_some_and(|age| age > PROMPT_MAX_AGE);
        if old && path.extension().is_some_and(|x| x == "md") {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// What to diagnose.
#[derive(Debug, Clone)]
pub(crate) enum Target {
    Crash(Crash),
    /// The entry and its priority name.
    Journal(JournalEntry, &'static str),
}

/// The user's agent settings, read on the GTK thread.
#[derive(Debug, Clone)]
pub(crate) struct AgentSettings {
    pub(crate) agents: Vec<&'static str>,
    pub(crate) probe: bool,
    pub(crate) instructions: String,
}

impl AgentSettings {
    pub(crate) fn from_config() -> Self {
        use mshell_config::config_manager::config_manager;
        use mshell_config::schema::config::{
            ConfigStoreFields, CrashesConfigStoreFields, diagnosis_agents,
        };
        use reactive_graph::prelude::GetUntracked;
        let c = || config_manager().config().crashes();
        Self {
            agents: diagnosis_agents(&c().agent_order().get_untracked()),
            probe: c().agent_probe().get_untracked(),
            instructions: c().agent_instructions().get_untracked(),
        }
    }
}

/// `coredumpctl info` for `pid`'s most recent dump, capped; `Err` is a
/// short reason.
async fn coredump_info(pid: u32) -> Result<String, String> {
    let run = tokio::process::Command::new("coredumpctl")
        .args(["info", "--no-pager", "-1", &pid.to_string()])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    match tokio::time::timeout(INFO_TIMEOUT, run).await {
        Err(_) => Err("coredumpctl timed out".into()),
        Ok(Err(e)) => Err(format!("coredumpctl: {e}")),
        Ok(Ok(o)) if o.stdout.is_empty() => Err("coredumpctl printed nothing".into()),
        Ok(Ok(o)) => Ok(cap_text(&String::from_utf8_lossy(&o.stdout), INFO_CAP)),
    }
}

/// Build the prompt, write it and open the terminal. Runs on the tokio
/// runtime (`relm4::spawn`). `Err` is a short user-facing reason.
pub(crate) async fn diagnose(target: Target, settings: AgentSettings) -> Result<(), String> {
    use mshell_process::SpawnReaped;
    if !any_agent_installed(&settings.agents) {
        return Err("Install Claude Code or Codex to diagnose".into());
    }
    let (stem, prompt) = match &target {
        Target::Crash(c) => {
            let info = coredump_info(c.pid).await;
            let info = info.as_deref().map_err(String::as_str);
            (
                format!("crash-{}-{}", c.program(), c.pid),
                crash_prompt(c, info, &settings.instructions),
            )
        }
        Target::Journal(e, priority) => (
            format!("journal-{}", e.ident),
            journal_prompt(e, priority, &settings.instructions),
        ),
    };
    let dir = prompt_dir().ok_or("XDG_RUNTIME_DIR is not set")?;
    let file = write_prompt(&dir, &stem, &prompt)
        .map_err(|e| format!("cannot write the prompt in {}: {e}", dir.display()))?;
    let argv = launch_command(&file, settings.probe, &settings.agents);
    let (term, mut cmd) =
        mshell_process::terminal::command(&argv, "diagnose").map_err(|e| e.to_string())?;
    cmd.spawn_reaped()
        .map(|_| ())
        .map_err(|e| format!("{term}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    const INFO: &str = include_str!("testdata/coredumpctl-info.txt");

    fn lld() -> Crash {
        Crash {
            time_us: 1793786206000000,
            pid: 1642539,
            uid: 1000,
            sig: 7,
            exe: "/home/u/.rustup/toolchains/x/bin/rust-lld".into(),
            core_present: true,
        }
    }

    #[test]
    fn crash_prompt_carries_facts_info_guide_and_instructions() {
        let p = crash_prompt(&lld(), Ok(INFO), "Answer in Turkish.");
        assert!(p.contains("- program: rust-lld\n"));
        assert!(p.contains("- PID: 1642539\n"));
        assert!(p.contains("- signal: SIGBUS (7)\n"));
        assert!(p.contains("- core file: present\n"));
        assert!(p.contains("## `coredumpctl info 1642539`"));
        assert!(p.contains("Signal: 7 (BUS) si_code: BUS_ADRERR"));
        assert!(p.contains("Stack trace of thread 1642734:"));
        assert!(p.contains("debuginfod"));
        assert!(p.contains("**Read-only.**"));
        assert!(p.trim_end().ends_with("Answer in Turkish."));
        // No instructions: no empty section.
        assert!(!crash_prompt(&lld(), Ok(INFO), "  ").contains("Extra instructions"));
    }

    #[test]
    fn crash_prompt_without_core_or_info_says_so() {
        let mut c = lld();
        c.core_present = false;
        let p = crash_prompt(&c, Err("coredumpctl timed out"), "");
        assert!(p.contains("core file: gone"));
        assert!(p.contains("It could not be read here (coredumpctl timed out)"));
        assert!(!p.contains("```text"));
    }

    #[test]
    fn journal_prompt_quotes_the_message() {
        let e = JournalEntry {
            time_us: 1793790480247274,
            priority: 3,
            ident: "mlogind".into(),
            message: "gkr-pam: unable to locate daemon control file".into(),
        };
        let p = journal_prompt(&e, "err", "");
        assert!(p.contains("- source: mlogind\n"));
        assert!(p.contains("- priority: err (3)\n"));
        assert!(p.contains("__REALTIME_TIMESTAMP=1793790480247274"));
        assert!(p.contains("```text\ngkr-pam: unable to locate daemon control file\n```"));
    }

    #[test]
    fn a_backtick_run_in_quoted_text_gets_a_longer_fence() {
        assert_eq!(fence_for("plain"), "```");
        assert_eq!(fence_for("a ```` b"), "`````");
        let mut out = String::new();
        code_block(&mut out, "x ``` y");
        assert!(out.starts_with("````text\n") && out.ends_with("\n````\n"));
    }

    #[test]
    fn cap_keeps_the_head_on_a_char_boundary() {
        assert_eq!(cap_text("short\n", 100), "short");
        let s = "ağaç".repeat(10); // multi-byte chars
        let c = cap_text(&s, 5);
        assert!(c.starts_with("ağ"));
        assert!(c.contains("more bytes cut"));
        let big = INFO.repeat(20);
        let c = cap_text(&big, INFO_CAP);
        assert!(c.len() < INFO_CAP + 100);
        assert!(c.trim_start().starts_with("PID: 1642539 (rust-lld)"));
    }

    #[test]
    fn prompt_file_is_private_and_old_ones_are_pruned() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("mshell/crash-diagnose");
        let a = write_prompt(&dir, "crash-rust lld/1", "hello").unwrap();
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "hello");
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&a), 0o600);
        assert_eq!(mode(&dir), 0o700);
        let name = a.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with("crash-rust_lld_1-") && name.ends_with(".md"));

        // Two days later: the old prompt goes, other files stay.
        std::fs::write(dir.join("keep.txt"), "x").unwrap();
        prune(&dir, SystemTime::now() + Duration::from_secs(2 * 24 * 3600));
        assert!(!a.exists());
        assert!(dir.join("keep.txt").exists());

        // A dir left wider is tightened.
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        write_prompt(&dir, "x", "y").unwrap();
        assert_eq!(mode(&dir), 0o700);
    }

    #[test]
    fn launch_command_passes_data_as_positional_args() {
        let v = launch_command(Path::new("/run/user/1000/p.md"), true, &["codex", "claude"]);
        assert_eq!(&v[..2], ["sh", "-c"]);
        assert_eq!(v[2], SCRIPT);
        assert_eq!(
            &v[3..],
            [
                "mshell-diagnose",
                "/run/user/1000/p.md",
                "1",
                "codex",
                "claude"
            ]
        );
        assert_eq!(launch_command(Path::new("/f"), false, &[])[5], "0");
    }

    #[test]
    fn the_script_prepends_every_agent_dir() {
        for d in AGENT_DIRS {
            assert!(SCRIPT.contains(&format!("$HOME/{d}:")), "{d} missing");
        }
    }

    #[test]
    fn agent_lookup_finds_home_installs() {
        let home = tempfile::tempdir().unwrap();
        let name = "mshell-test-agent-xyz";
        assert!(!agent_installed_in(name, Some(home.path())));
        fake_bin(&home.path().join(".local/share/npm/bin"), name, "exit 0");
        assert!(agent_installed_in(name, Some(home.path())));
        assert!(!agent_installed_in(name, None));
    }

    fn fake_bin(dir: &Path, name: &str, body: &str) {
        std::fs::create_dir_all(dir).unwrap();
        let p = dir.join(name);
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// Run [`SCRIPT`] with a fake `$HOME` whose `~/.local/bin` holds the
    /// given agents; `/usr/bin:/bin` stays on PATH for sh, grep, timeout.
    fn run_script(agents: &[(&str, &str)], probe: bool, order: &[&str]) -> String {
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join(".local/bin");
        for (name, body) in agents {
            fake_bin(&bin, name, body);
        }
        let out = Command::new("sh")
            .args(
                launch_command(Path::new("/tmp/prompt.md"), probe, order)
                    .iter()
                    .skip(1),
            )
            .env("HOME", home.path())
            .env("PATH", "/usr/bin:/bin")
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr)
    }

    /// Interactive run: print what we were started with. Probe (`-p` /
    /// `exec`): answer per `probe_reply`.
    fn agent(probe_reply: &str) -> String {
        format!(
            "case \"$1\" in -p|exec) {probe_reply} ;; *) echo \"RAN $(basename \"$0\") $*\" ;; esac"
        )
    }

    #[test]
    fn script_skips_an_agent_out_of_quota_and_runs_the_next() {
        let claude = agent("echo 'Claude AI usage limit reached|1793790000'; exit 0");
        let codex = agent("echo OK");
        let out = run_script(
            &[("claude", &claude), ("codex", &codex)],
            true,
            &["claude", "codex"],
        );
        assert!(
            out.contains("claude cannot answer right now: Claude AI usage limit reached"),
            "{out}"
        );
        assert!(
            out.contains("RAN codex Diagnose what this file describes: /tmp/prompt.md."),
            "{out}"
        );
        assert!(!out.contains("RAN claude"), "{out}");
    }

    #[test]
    fn script_skips_a_failing_probe_and_honours_the_order() {
        let codex = agent("echo 'error: not logged in' >&2; exit 1");
        let claude = agent("echo OK");
        let out = run_script(
            &[("claude", &claude), ("codex", &codex)],
            true,
            &["codex", "claude"],
        );
        assert!(
            out.contains("codex cannot answer right now: error: not logged in"),
            "{out}"
        );
        assert!(out.contains("RAN claude"), "{out}");
    }

    #[test]
    fn script_without_probe_execs_the_first_installed_agent() {
        // claude is listed first but not installed; codex would fail a
        // probe, but probing is off.
        let codex = agent("exit 1");
        let out = run_script(&[("codex", &codex)], false, &["claude", "codex"]);
        assert!(!out.contains("Checking"), "{out}");
        assert!(out.contains("RAN codex"), "{out}");
    }

    #[test]
    fn script_reports_when_no_agent_can_answer() {
        let claude = agent("echo 'You have hit your limit · resets 4pm'");
        let out = run_script(&[("claude", &claude)], true, &["claude", "codex"]);
        assert!(
            out.contains("No AI agent could take this (tried: claude codex)"),
            "{out}"
        );
        assert!(out.contains("The prompt is in /tmp/prompt.md"), "{out}");
    }
}
