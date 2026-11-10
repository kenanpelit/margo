//! The one UFW watcher per shell process, and the firewall's root
//! actions.
//!
//! The state is read without root and without a prompt: the saved
//! configuration (`ufw.conf`, `/etc/default/ufw`, the rule files) and
//! `systemctl show ufw.service`, every [`POLL`] while anything shows it
//! (the bar pill, the open menu) and at once after every action
//! ([`refresh`]). One reader serves every bar and menu
//! ([`subscribe`]), parked while nobody listens.
//!
//! Every change goes through the audited root helper
//! (`mshell-privileged ufw-*`, one polkit prompt per action), one at a
//! time per process ([`run`]).

use mshell_config::toast::{ToastEvent, ToastSeverity, push_toast};
use mshell_privileged::Request;
use mshell_privileged::ufw::{
    self, Saved, UfwOp,
    argv::SHOW_ARGS,
    rules,
    state::{self, ReadError, Snapshot, UnitRead},
};
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tokio::process::Command;
use tokio::sync::{Notify, watch};

/// How often the state is re-read while shown (four small files and
/// one `systemctl show`).
const POLL: Duration = Duration::from_secs(15);
/// `systemctl show` may take this long.
const SHOW_TIMEOUT: Duration = Duration::from_secs(5);

struct Service {
    /// Its receiver count is the number of live subscribers.
    tx: watch::Sender<Arc<Snapshot>>,
    /// Wakes the reader: a subscriber arrived or an action finished.
    wake: Arc<Notify>,
}

static SERVICE: OnceLock<Service> = OnceLock::new();

/// Rules read as root (`ufw-status`), used while the files can't be
/// read without root. Dropped after every change (they'd be stale).
static ROOT_SAVED: Mutex<Option<Saved>> = Mutex::new(None);

fn service() -> &'static Service {
    SERVICE.get_or_init(|| {
        let (tx, _) = watch::channel(Arc::new(Snapshot::loading()));
        let wake = Arc::new(Notify::new());
        relm4::spawn(run_reader(tx.clone(), wake.clone()));
        Service { tx, wake }
    })
}

/// Subscribe to the firewall state; the reader runs while at least one
/// receiver exists. Hold it only while showing it.
pub(crate) fn subscribe() -> watch::Receiver<Arc<Snapshot>> {
    let s = service();
    let rx = s.tx.subscribe();
    s.wake.notify_one();
    rx
}

/// The last snapshot read.
pub(crate) fn current() -> Arc<Snapshot> {
    SERVICE
        .get()
        .map(|s| s.tx.borrow().clone())
        .unwrap_or_else(|| Arc::new(Snapshot::loading()))
}

/// Re-read now (a menu opened, an action finished).
pub(crate) fn refresh() {
    service().wake.notify_one();
}

async fn run_reader(tx: watch::Sender<Arc<Snapshot>>, wake: Arc<Notify>) {
    loop {
        while tx.receiver_count() == 0 {
            wake.notified().await;
        }
        let snap = read().await;
        tx.send_if_modified(|cur| {
            let changed = **cur != snap;
            if changed {
                *cur = Arc::new(snap);
            }
            changed
        });
        tokio::select! {
            () = tokio::time::sleep(POLL) => {}
            () = wake.notified() => {}
        }
    }
}

/// `ufw` is installed: on `$PATH`, or in `/usr/sbin` (Debian puts it
/// there, outside an ordinary user's `$PATH`).
pub(crate) fn installed() -> bool {
    margo_fs::path::on_path("ufw") || Path::new("/usr/sbin/ufw").is_file()
}

/// One full read, off the GTK thread.
pub(crate) async fn read() -> Snapshot {
    let installed = installed();
    let saved = tokio::task::spawn_blocking(|| ufw::read_saved(Path::new("/")))
        .await
        .unwrap_or_else(|e| Saved {
            conf: Err(ReadError::Other(e.to_string())),
            defaults: None,
            rules: Err(ReadError::Other(e.to_string())),
        });
    let saved = with_root_rules(saved);
    Snapshot {
        installed,
        conf: saved.conf,
        defaults: saved.defaults,
        unit: read_unit().await,
        rules: saved.rules,
    }
}

/// Fill what the files didn't give from the last root read.
fn with_root_rules(mut saved: Saved) -> Saved {
    let Ok(guard) = ROOT_SAVED.lock() else {
        return saved;
    };
    let Some(root) = guard.as_ref() else {
        return saved;
    };
    if saved.rules == Err(ReadError::Denied) {
        saved.rules = root.rules.clone();
    }
    if saved.conf == Err(ReadError::Denied) {
        saved.conf = root.conf.clone();
    }
    if saved.defaults.is_none() {
        saved.defaults = root.defaults.clone();
    }
    saved
}

async fn read_unit() -> UnitRead {
    if !margo_fs::path::on_path("systemctl") {
        return UnitRead::NoSystemctl;
    }
    let run = Command::new("systemctl")
        .args(SHOW_ARGS)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .output();
    match tokio::time::timeout(SHOW_TIMEOUT, run).await {
        Err(_) => UnitRead::Failed("systemctl timed out".into()),
        Ok(Err(e)) => UnitRead::Failed(format!("systemctl: {e}")),
        Ok(Ok(out)) if !out.status.success() => {
            UnitRead::Failed(format!("systemctl exited {}", out.status))
        }
        Ok(Ok(out)) => UnitRead::Unit(state::parse_show(&String::from_utf8_lossy(&out.stdout))),
    }
}

/// Set while a root action runs: one at a time per process.
static BUSY: AtomicBool = AtomicBool::new(false);

/// Holds [`BUSY`]; released on drop (also when the caller is dropped
/// mid-action).
struct Busy;

impl Busy {
    fn take() -> Option<Busy> {
        (!BUSY.swap(true, Ordering::AcqRel)).then_some(Busy)
    }
}

impl Drop for Busy {
    fn drop(&mut self) {
        BUSY.store(false, Ordering::Release);
    }
}

/// Whether a root action is running now.
pub(crate) fn busy() -> bool {
    BUSY.load(Ordering::Acquire)
}

/// What the user is told while another change runs.
pub(crate) const BUSY_MSG: &str = "Another firewall change is still running";

/// The helper argv (after `pkexec`) for `op`, validated the way root
/// will check it, so a refused argument never raises a prompt.
pub(crate) fn pkexec_argv(op: UfwOp) -> Result<Vec<String>, String> {
    let req = Request::Ufw(op)
        .validated()
        .map_err(|e| e.message().to_string())?;
    Ok(mshell_privileged::lossy(&req.argv()))
}

/// Run `op` as root through the helper (one polkit prompt), then
/// re-read the state. `Ok` carries the helper's stdout.
pub(crate) async fn run(op: UfwOp) -> Result<String, String> {
    let argv = pkexec_argv(op)?;
    if !mshell_privileged::installed() {
        return Err(mshell_privileged::MISSING.to_string());
    }
    if !margo_fs::path::on_path("pkexec") {
        return Err("pkexec isn't installed (package polkit)".into());
    }
    let Some(_busy) = Busy::take() else {
        return Err(BUSY_MSG.into());
    };
    let out = Command::new("pkexec")
        .args(&argv)
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output()
        .await;
    let changed = argv.get(1).is_some_and(|sub| sub != "ufw-status");
    if changed && let Ok(mut root) = ROOT_SAVED.lock() {
        *root = None;
    }
    refresh();
    let out = out.map_err(|e| format!("pkexec: {e}"))?;
    mshell_privileged::outcome(out.status.code(), &String::from_utf8_lossy(&out.stderr))?;
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Read the rule files as root (when they aren't readable without),
/// keep them for the next reads, and refresh.
pub(crate) async fn load_rules_as_root() -> Result<(), String> {
    let out = run(UfwOp::Status).await?;
    let dump = rules::parse_dump(&out).ok_or("the helper's answer was cut short")?;
    if let Ok(mut root) = ROOT_SAVED.lock() {
        *root = Some(ufw::saved_from_dump(&dump));
    }
    refresh();
    Ok(())
}

/// A toast for an action's outcome: calm on success, warn on failure.
pub(crate) fn report(title: &str, result: Result<String, String>) {
    let (body, severity) = match result {
        Ok(b) => (b, ToastSeverity::Calm),
        Err(e) => (e, ToastSeverity::Warn),
    };
    push_toast(ToastEvent {
        icon: "firewall-symbolic".to_string(),
        title: title.to_string(),
        body: (!body.is_empty()).then_some(body),
        severity,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use mshell_privileged::ufw::{Switch, UnitOp, spec};

    #[test]
    fn the_argv_is_the_helpers_fixed_verb() {
        let h = mshell_privileged::HELPER;
        assert_eq!(
            pkexec_argv(UfwOp::Switch(Switch::On)).unwrap(),
            [h, "ufw-switch", "on"]
        );
        assert_eq!(
            pkexec_argv(UfwOp::Unit(UnitOp::Enable)).unwrap(),
            [h, "ufw-unit", "enable"]
        );
        let add = spec::AddForm {
            action: spec::Action::Allow,
            port: "22".into(),
            proto: spec::Proto::Tcp,
            from: String::new(),
            comment: "SSH".into(),
        }
        .validate()
        .unwrap();
        assert_eq!(
            pkexec_argv(UfwOp::Add(add)).unwrap(),
            [
                h, "ufw-add", "allow", "in", "-", "-", "tcp", "any", "any", "22", "SSH"
            ]
        );
        assert_eq!(pkexec_argv(UfwOp::Status).unwrap(), [h, "ufw-status"]);
    }

    #[test]
    fn root_rules_fill_only_what_was_denied() {
        let set = rules::parse_rules(
            "### tuple ### allow tcp 22 0.0.0.0/0 any 0.0.0.0/0 in\n### END RULES ###\n",
            None,
            false,
        );
        *ROOT_SAVED.lock().unwrap() = Some(Saved {
            conf: Ok(rules::Conf::default()),
            defaults: None,
            rules: Ok(set.clone()),
        });
        let denied = Saved {
            conf: Ok(rules::Conf {
                enabled: Some(true),
                loglevel: None,
            }),
            defaults: None,
            rules: Err(ReadError::Denied),
        };
        let got = with_root_rules(denied);
        assert_eq!(got.rules, Ok(set));
        assert_eq!(
            got.conf.unwrap().enabled,
            Some(true),
            "a readable file wins"
        );
        let missing = Saved {
            conf: Err(ReadError::Missing),
            defaults: None,
            rules: Err(ReadError::Missing),
        };
        assert_eq!(with_root_rules(missing).rules, Err(ReadError::Missing));
        *ROOT_SAVED.lock().unwrap() = None;
    }
}
