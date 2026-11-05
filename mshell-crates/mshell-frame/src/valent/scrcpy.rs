//! Phone screen mirroring / control through scrcpy + adb, shown in the
//! Valent menu. Port of the noctalia `phone-operate` plugin's cast
//! section: list adb devices (USB or wireless), start / stop a scrcpy
//! window per device, and wireless debugging pair (host:port + 6-digit
//! code) / connect / disconnect.
//!
//! Parsers and argument building are pure and tested; the process side
//! degrades to "not installed" when `adb` / `scrcpy` are missing
//! (Arch: `android-tools`, `scrcpy`).
//!
//! Each window runs in its own `app-mshell-scrcpy-….scope`, so it outlives
//! a shell restart; the restarted shell finds it again ([`mirroring`]) by
//! uid, that exact scope name, the `scrcpy --serial=…` argv and the
//! process start time, so the menu still shows it and can stop it.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::Path;
use std::process::Stdio;
use std::sync::{LazyLock, Mutex};

use mshell_process::identity::{self, Identity};

/// One `adb devices -l` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AdbDevice {
    pub(crate) serial: String,
    pub(crate) state: AdbState,
    /// `model:` field with underscores as spaces; empty when unknown.
    pub(crate) model: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AdbState {
    /// Authorised and usable.
    Device,
    /// Waiting for the "Allow USB debugging?" prompt on the phone.
    Unauthorized,
    Offline,
    /// udev rules don't let us open the USB device.
    NoPermissions,
    Other(String),
}

impl AdbDevice {
    /// Wireless (TCP / mDNS TLS) rather than USB.
    pub(crate) fn wireless(&self) -> bool {
        self.serial.contains(':') || self.serial.contains("._adb-tls-connect._tcp")
    }

    pub(crate) fn display_name(&self) -> &str {
        if self.model.is_empty() {
            &self.serial
        } else {
            &self.model
        }
    }

    /// Short status line for the row subtitle.
    pub(crate) fn status_text(&self) -> String {
        let link = if self.wireless() { "Wi-Fi" } else { "USB" };
        match &self.state {
            AdbState::Device => format!("{link} · {}", self.serial),
            AdbState::Unauthorized => format!("{link} · allow USB debugging on the phone"),
            AdbState::Offline => format!("{link} · offline"),
            AdbState::NoPermissions => {
                format!("{link} · no permission (udev rules / android-udev)")
            }
            AdbState::Other(s) => format!("{link} · {s}"),
        }
    }
}

/// Parse `adb devices -l`. The header, blank lines and daemon chatter
/// (`* daemon started successfully`) are skipped.
pub(crate) fn parse_adb_devices(out: &str) -> Vec<AdbDevice> {
    out.lines()
        .filter(|l| {
            !l.trim().is_empty() && !l.starts_with("List of devices") && !l.starts_with('*')
        })
        .filter_map(|line| {
            let mut tokens = line.split_whitespace();
            let serial = tokens.next()?.to_string();
            let state = match tokens.next()? {
                "device" => AdbState::Device,
                "unauthorized" => AdbState::Unauthorized,
                "offline" => AdbState::Offline,
                "no" => AdbState::NoPermissions,
                other => AdbState::Other(other.to_string()),
            };
            let model = line
                .split_whitespace()
                .find_map(|t| t.strip_prefix("model:"))
                .map(|m| m.replace('_', " "))
                .unwrap_or_default();
            Some(AdbDevice {
                serial,
                state,
                model,
            })
        })
        .collect()
}

/// `host:port` with a non-empty host and a port in 1..=65535.
pub(crate) fn valid_host_port(s: &str) -> bool {
    let Some((host, port)) = s.trim().rsplit_once(':') else {
        return false;
    };
    !host.is_empty()
        && !host.contains(char::is_whitespace)
        && port.parse::<u16>().is_ok_and(|p| p > 0)
}

/// The 6-digit wireless-debugging pairing code.
pub(crate) fn valid_pair_code(s: &str) -> bool {
    let s = s.trim();
    s.len() == 6 && s.chars().all(|c| c.is_ascii_digit())
}

/// Outcome of `adb pair` / `adb connect` / `adb disconnect`, judged from
/// its output (adb's exit status is 0 for some failures, e.g. connect).
pub(crate) fn adb_outcome(out: &str) -> Result<String, String> {
    let line = out
        .lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .unwrap_or("")
        .to_string();
    let lower = line.to_ascii_lowercase();
    let ok = lower.starts_with("successfully paired")
        || lower.starts_with("connected to")
        || lower.starts_with("already connected")
        || lower.starts_with("disconnected");
    if ok {
        Ok(line)
    } else if line.is_empty() {
        Err("adb gave no answer".to_string())
    } else {
        Err(line)
    }
}

/// scrcpy argument list: the device, optional screen-off, then the
/// user's extra arguments (whitespace-split; no shell involved).
pub(crate) fn scrcpy_args(serial: &str, screen_off: bool, extra: &str) -> Vec<String> {
    let mut args = vec![format!("--serial={serial}")];
    if screen_off {
        args.push("--turn-screen-off".to_string());
    }
    args.extend(extra.split_whitespace().map(str::to_string));
    args
}

/// Whether `bin` is an executable on `PATH`.
pub(crate) fn on_path(bin: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|dir| {
            let p = dir.join(bin);
            std::fs::metadata(&p).is_ok_and(|m| {
                use std::os::unix::fs::PermissionsExt;
                m.is_file() && m.permissions().mode() & 0o111 != 0
            })
        })
    })
}

async fn adb(args: &[&str]) -> Result<String, String> {
    let out = tokio::process::Command::new("adb")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|e| format!("adb: {e}"))?;
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push('\n');
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    Ok(text)
}

/// `adb devices -l`, parsed (starts the adb server if needed).
pub(crate) async fn devices() -> Result<Vec<AdbDevice>, String> {
    adb(&["devices", "-l"]).await.map(|o| parse_adb_devices(&o))
}

pub(crate) async fn pair(host_port: &str, code: &str) -> Result<String, String> {
    adb_outcome(&adb(&["pair", host_port.trim(), code.trim()]).await?)
}

pub(crate) async fn connect(host_port: &str) -> Result<String, String> {
    adb_outcome(&adb(&["connect", host_port.trim()]).await?)
}

pub(crate) async fn disconnect(serial: &str) -> Result<String, String> {
    adb_outcome(&adb(&["disconnect", serial]).await?)
}

/// One scrcpy window the menu can stop.
enum Mirror {
    /// Started by this shell.
    Child(std::process::Child),
    /// Started by an earlier shell and still running in its own scope
    /// (see [`survivors`]).
    Adopted(Identity),
}

/// Running scrcpy windows by device serial.
#[derive(Default)]
struct Mirrors {
    map: HashMap<String, Mirror>,
    /// Survivors of an earlier shell were looked for already.
    rediscovered: bool,
}

impl Mirrors {
    /// Serials with a live window, dropping (and reaping) exited ones. The
    /// first call adopts the windows an earlier shell left running.
    fn live(&mut self, proc_root: &Path, uid: Option<u32>) -> Vec<String> {
        if !self.rediscovered {
            self.rediscovered = true;
            if let Some(uid) = uid {
                for (serial, id) in survivors(proc_root, uid) {
                    self.map.entry(serial).or_insert(Mirror::Adopted(id));
                }
            }
        }
        self.map.retain(|_, mirror| match mirror {
            Mirror::Child(child) => matches!(child.try_wait(), Ok(None)),
            Mirror::Adopted(id) => uid.is_some_and(|uid| identity::alive_in(proc_root, *id, uid)),
        });
        let mut serials: Vec<String> = self.map.keys().cloned().collect();
        serials.sort();
        serials
    }
}

/// The device serial of a scrcpy argv as [`scrcpy_args`] builds it
/// (`scrcpy --serial=<serial> …`); `None` for anything else.
pub(crate) fn mirror_serial(argv: &[OsString]) -> Option<String> {
    let program = Path::new(argv.first()?).file_name()?;
    if program != "scrcpy" {
        return None;
    }
    argv.iter()
        .find_map(|a| a.to_str()?.strip_prefix("--serial="))
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// scrcpy windows an earlier shell started that are still running: our
/// uid, our exact `app-mshell-scrcpy-<pid>-<n>.scope` (see
/// [`mshell_process::outliving_shell`]) and a `scrcpy --serial=…` argv. A
/// scrcpy the user ran by hand has no such scope and is left alone.
fn survivors(proc_root: &Path, uid: u32) -> Vec<(String, Identity)> {
    identity::find_in(proc_root, uid, SCOPE_LABEL, |argv| {
        mirror_serial(argv).is_some()
    })
    .into_iter()
    .filter_map(|p| Some((mirror_serial(&p.argv)?, p.identity())))
    .collect()
}

/// Label of the transient scope each scrcpy window runs in.
const SCOPE_LABEL: &str = "scrcpy";

static MIRRORS: LazyLock<Mutex<Mirrors>> = LazyLock::new(|| Mutex::new(Mirrors::default()));

/// Serials with a live scrcpy window, including ones an earlier shell
/// started (reaps the ones that exited).
pub(crate) fn mirroring() -> Vec<String> {
    let Ok(mut mirrors) = MIRRORS.lock() else {
        return Vec::new();
    };
    mirrors.live(Path::new(identity::PROC), identity::own_uid())
}

/// Open a scrcpy window for `serial`.
pub(crate) fn start_mirror(serial: &str, screen_off: bool, extra: &str) -> Result<(), String> {
    // In its own scope, so the mirror window survives an mshell restart
    // (systemd-run execs scrcpy in place: the Child is still scrcpy).
    let child = mshell_process::outliving_shell(
        "scrcpy",
        scrcpy_args(serial, screen_off, extra),
        SCOPE_LABEL,
    )
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .spawn()
    .map_err(|e| format!("scrcpy: {e}"))?;
    if let Ok(mut mirrors) = MIRRORS.lock() {
        mirrors.map.insert(serial.to_string(), Mirror::Child(child));
    }
    Ok(())
}

/// Close the scrcpy window for `serial`, if any.
pub(crate) fn stop_mirror(serial: &str) {
    let mirror = MIRRORS.lock().ok().and_then(|mut m| m.map.remove(serial));
    match mirror {
        Some(Mirror::Child(mut child)) => {
            let _ = child.kill();
            // Reap off the UI thread.
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        // Not our child: SIGTERM, only if the pid is still that window.
        Some(Mirror::Adopted(id)) => {
            identity::terminate(id);
        }
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_empty_list_captured_on_this_machine() {
        // `adb devices -l` right after the server started, no phone attached.
        let out = "* daemon not running; starting now at tcp:5037\n\
                   * daemon started successfully\n\
                   List of devices attached\n\n";
        assert!(parse_adb_devices(out).is_empty());
    }

    #[test]
    fn parses_usb_wireless_and_problem_rows() {
        let out = "List of devices attached\n\
            R5CT1234ABC            device usb:1-2 product:o1sxeea model:SM_G991B device:o1s transport_id:3\n\
            192.168.1.40:41235     device product:panther model:Pixel_7 device:panther transport_id:5\n\
            adb-2A1B-xyz._adb-tls-connect._tcp device product:p model:Pixel_8 device:s transport_id:6\n\
            0123456789             unauthorized usb:1-1 transport_id:4\n\
            ZY22                   no permissions (missing udev rules? user is in the plugdev group); see [http://developer.android.com/tools/device.html] usb:3-1\n\
            emulator-5554\toffline\n";
        let d = parse_adb_devices(out);
        assert_eq!(d.len(), 6);
        assert_eq!(d[0].state, AdbState::Device);
        assert_eq!(d[0].display_name(), "SM G991B");
        assert!(!d[0].wireless());
        assert!(d[1].wireless());
        assert_eq!(d[1].status_text(), "Wi-Fi · 192.168.1.40:41235");
        assert!(d[2].wireless(), "mDNS TLS serial is wireless");
        assert_eq!(d[3].state, AdbState::Unauthorized);
        assert_eq!(d[3].display_name(), "0123456789");
        assert_eq!(d[4].state, AdbState::NoPermissions);
        assert_eq!(d[5].state, AdbState::Offline);
    }

    #[test]
    fn validates_host_port_and_pair_code() {
        assert!(valid_host_port("192.168.1.40:37099"));
        assert!(valid_host_port(" phone.lan:5555 "));
        assert!(!valid_host_port("192.168.1.40"));
        assert!(!valid_host_port(":5555"));
        assert!(!valid_host_port("1.2.3.4:0"));
        assert!(!valid_host_port("1.2.3.4:70000"));
        assert!(!valid_host_port("a b:5555"));
        assert!(valid_pair_code("123456"));
        assert!(valid_pair_code(" 012345 "));
        assert!(!valid_pair_code("12345"));
        assert!(!valid_pair_code("12a456"));
    }

    #[test]
    fn judges_adb_outcomes_by_text() {
        assert!(
            adb_outcome("Successfully paired to 192.168.1.40:37099 [guid=adb-2A1B-xyz]\n").is_ok()
        );
        assert!(adb_outcome("connected to 192.168.1.40:41235\n\n").is_ok());
        assert!(adb_outcome("already connected to 192.168.1.40:41235").is_ok());
        assert!(adb_outcome("disconnected 192.168.1.40:41235").is_ok());
        // adb connect exits 0 on these — the text decides.
        assert_eq!(
            adb_outcome("failed to connect to '192.168.1.40:5555': Connection refused\n"),
            Err("failed to connect to '192.168.1.40:5555': Connection refused".into())
        );
        assert!(adb_outcome("Failed: Unable to start pairing client.").is_err());
        assert!(adb_outcome("\n").is_err());
    }

    #[test]
    fn builds_scrcpy_args_without_a_shell() {
        assert_eq!(scrcpy_args("R5CT", false, ""), vec!["--serial=R5CT"]);
        assert_eq!(
            scrcpy_args("1.2.3.4:5555", true, " --max-size=1280  --stay-awake "),
            vec![
                "--serial=1.2.3.4:5555",
                "--turn-screen-off",
                "--max-size=1280",
                "--stay-awake"
            ]
        );
        // A hostile-looking value stays one literal argument, never a shell.
        assert_eq!(
            scrcpy_args("x;rm -rf ~", false, "")[0],
            "--serial=x;rm -rf ~"
        );
    }

    fn os(v: &[&str]) -> Vec<OsString> {
        v.iter().map(OsString::from).collect()
    }

    #[test]
    fn mirror_serial_only_from_a_scrcpy_argv() {
        assert_eq!(
            mirror_serial(&os(&[
                "scrcpy",
                "--serial=1.2.3.4:5555",
                "--turn-screen-off"
            ])),
            Some("1.2.3.4:5555".into())
        );
        assert_eq!(
            mirror_serial(&os(&[
                "/usr/bin/scrcpy",
                "--max-size=1280",
                "--serial=R5CT"
            ])),
            Some("R5CT".into())
        );
        assert_eq!(mirror_serial(&os(&["scrcpy"])), None);
        assert_eq!(mirror_serial(&os(&["scrcpy", "--serial="])), None);
        assert_eq!(mirror_serial(&os(&["adb", "--serial=R5CT"])), None);
        assert_eq!(mirror_serial(&os(&["notscrcpy", "--serial=R5CT"])), None);
        assert_eq!(mirror_serial(&[]), None);
    }

    /// Review item 10: after a shell restart the scrcpy window an earlier
    /// shell opened is still running in its scope; the menu must list it
    /// (and so be able to stop it), but never adopt a foreign scrcpy.
    #[test]
    fn restarted_shell_adopts_its_surviving_windows_only() {
        let root = tempfile::tempdir().expect("tempdir");
        let add = |pid: u32, start: u64, uid: u32, argv: &[&str], unit: &str| {
            let dir = root.path().join(pid.to_string());
            std::fs::create_dir_all(&dir).expect("mkdir");
            std::fs::write(
                dir.join("stat"),
                format!("{pid} (scrcpy) S 1 1 1 0 -1 0 0 0 0 0 0 0 0 0 20 0 1 0 {start} 1 1"),
            )
            .expect("stat");
            std::fs::write(
                dir.join("status"),
                format!("Uid:\t{uid}\t{uid}\t{uid}\t{uid}\n"),
            )
            .expect("status");
            std::fs::write(dir.join("cmdline"), argv.join("\0") + "\0").expect("cmdline");
            std::fs::write(
                dir.join("cgroup"),
                format!("0::/user.slice/app.slice/{unit}\n"),
            )
            .expect("cgroup");
        };
        add(
            10,
            100,
            1000,
            &["scrcpy", "--serial=R5CT"],
            "app-mshell-scrcpy-7-0.scope",
        );
        add(
            11,
            110,
            1000,
            &["scrcpy", "--serial=HAND"],
            "tmux-spawn-1.scope",
        );
        add(
            12,
            120,
            1001,
            &["scrcpy", "--serial=OTHER"],
            "app-mshell-scrcpy-8-0.scope",
        );
        add(
            13,
            130,
            1000,
            &["adb", "-s", "R5CT", "reverse"],
            "app-mshell-scrcpy-7-0.scope",
        );
        let mut mirrors = Mirrors::default();
        assert_eq!(
            mirrors.live(root.path(), Some(1000)),
            vec!["R5CT".to_string()]
        );
        assert!(matches!(
            mirrors.map.get("R5CT"),
            Some(Mirror::Adopted(Identity {
                pid: 10,
                start_ticks: 100
            }))
        ));
        // The window closed: it drops out of the list.
        std::fs::remove_dir_all(root.path().join("10")).expect("rm");
        assert!(mirrors.live(root.path(), Some(1000)).is_empty());
        // A new process reusing pid 10 is not that window either.
        add(
            10,
            900,
            1000,
            &["scrcpy", "--serial=R5CT"],
            "app-mshell-scrcpy-7-0.scope",
        );
        assert!(mirrors.live(root.path(), Some(1000)).is_empty());
    }
}
