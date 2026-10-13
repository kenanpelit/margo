//! Device-slot management for Mullvad's 5-device account limit across machines.
//!
//! Each machine records "the device name I own" in `~/.mullvad/slot.state`
//! under a per-machine key (`DEV_<hostname>_<machine-id prefix>=<device
//! name>`). `recycle` can revoke the *other* machines' recorded devices
//! (never the current one) to free a slot, then log in + connect + record
//! itself.
//!
//! The key used to be `DEV_<os-id>` (collides when two machines run the same
//! distro), then `DEV_<hostname>` alone (still collides when two machines
//! share a hostname — a user-chosen label, not guaranteed unique). The
//! `/etc/machine-id` suffix is what actually disambiguates: systemd generates
//! it once per OS install and it stays constant across reboots, so two
//! separate installs get different ids even with identical distro + hostname
//! (the one exception: a disk cloned/imaged without regenerating it).
//!
//! Originally format-compatible with osc-mullvad's `slot.state`
//! (`DEV_<os-id>`); now retired/unused, so the key scheme above is free to
//! diverge. The account number comes from `$MULLVAD_ACCOUNT_NUMBER` or
//! `pass show <entry>`.

use std::path::PathBuf;

use super::{actions, sys};

/// OS identity, e.g. "cachyos" / "arch", from /etc/os-release.
pub fn os_id() -> String {
    let body = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    for line in body.lines() {
        if let Some(v) = line.strip_prefix("ID=") {
            return v.trim().trim_matches('"').to_string();
        }
    }
    for line in body.lines() {
        if let Some(v) = line.strip_prefix("NAME=") {
            return v.trim().trim_matches('"').to_string();
        }
    }
    "unknown".into()
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

/// This machine's hostname (`/etc/hostname`, falling back to `uname -n`).
pub fn hostname() -> String {
    if let Ok(h) = std::fs::read_to_string("/etc/hostname") {
        let h = h.trim();
        if !h.is_empty() {
            return h.to_string();
        }
    }
    sys::out("uname", &["-n"]).trim().to_string()
}

/// This install's `/etc/machine-id` (generated once by systemd at install
/// time; stable across reboots, distinct per install — unlike the hostname
/// label, which a user can duplicate across machines).
pub fn machine_id() -> String {
    std::fs::read_to_string("/etc/machine-id")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_default()
}

/// This machine's state key: hostname (readable) + a machine-id prefix
/// (actually unique), so two machines sharing a hostname still don't
/// collide.
pub fn state_key() -> String {
    let host = sanitize(&hostname());
    let mid = machine_id();
    if mid.is_empty() {
        format!("DEV_{host}")
    } else {
        format!("DEV_{host}_{}", &mid[..mid.len().min(8)])
    }
}

/// Slot-state path. Precedence (mirrors osc-mullvad):
/// `$OSC_MULLVAD_SLOT_STATE_FILE` → `$OSC_MULLVAD_STATE_DIR`/slot.state →
/// `$OSC_MULLVAD_DIR`/slot.state → `~/.mullvad/slot.state`.
pub fn state_path() -> PathBuf {
    if let Ok(p) = std::env::var("OSC_MULLVAD_SLOT_STATE_FILE") {
        let p = PathBuf::from(p);
        return if p.is_dir() { p.join("slot.state") } else { p };
    }
    let dir = std::env::var("OSC_MULLVAD_STATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| sys::mullvad_dir());
    dir.join("slot.state")
}

/// Parse `DEV_xxx=value` lines → (key, value) pairs.
pub fn parse_state(body: &str) -> Vec<(String, String)> {
    body.lines()
        .filter_map(|l| {
            let l = l.trim();
            if !l.starts_with("DEV_") {
                return None;
            }
            l.split_once('=')
                .map(|(k, v)| (k.to_string(), v.to_string()))
        })
        .collect()
}

fn read_state() -> Vec<(String, String)> {
    parse_state(&std::fs::read_to_string(state_path()).unwrap_or_default())
}

fn write_state(pairs: &[(String, String)]) {
    let p = state_path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let body = pairs
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    let _ = std::fs::write(&p, body);
}

/// Upsert this machine's device name into the state.
pub fn record(key: &str, value: &str) {
    let mut pairs = read_state();
    if let Some(e) = pairs.iter_mut().find(|(k, _)| k == key) {
        e.1 = value.to_string();
    } else {
        pairs.push((key.to_string(), value.to_string()));
    }
    write_state(&pairs);
}

/// Current device name (`mullvad account get` → "Device name: …").
pub fn current_device() -> String {
    parse_device_name(&sys::mullvad(&["account", "get"]))
}

pub fn parse_device_name(s: &str) -> String {
    for line in s.lines() {
        let t = line.trim();
        if let Some(n) = t.strip_prefix("Device name:") {
            return n.trim().to_string();
        }
    }
    String::new()
}

pub fn is_logged_in() -> bool {
    !current_device().is_empty()
}

/// Device names on the account (`account list-devices`, header dropped).
/// `account`, given explicitly via `-a`, works even when not logged in
/// here — the CLI's "current account" fallback (used when `account` is
/// `None`) needs a login to already exist, which recycling a slot can't
/// assume.
pub fn list_devices(account: Option<&str>) -> Vec<String> {
    let mut args = vec!["account", "list-devices"];
    if let Some(acc) = account {
        args.push("-a");
        args.push(acc);
    }
    parse_devices(&sys::mullvad(&args))
}

pub fn parse_devices(s: &str) -> Vec<String> {
    s.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.ends_with(':')) // drop "Devices on the account:"
        .map(str::to_string)
        .collect()
}

/// Account number: `$MULLVAD_ACCOUNT_NUMBER` or `pass show <entry>` (first line).
pub fn account_number(pass_entry: &str) -> Option<String> {
    if let Ok(n) = std::env::var("MULLVAD_ACCOUNT_NUMBER") {
        let n = n.trim().to_string();
        if !n.is_empty() {
            return Some(n);
        }
    }
    let out = sys::out("pass", &["show", pass_entry]);
    let first = out.lines().next().unwrap_or("").trim().to_string();
    if first.is_empty() { None } else { Some(first) }
}

/// Revoke a device by name. Refuses to revoke the current device.
/// `account`, if given, is passed via `-a` (see [`list_devices`]).
pub fn revoke(account: Option<&str>, name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("no device name".into());
    }
    if name == current_device() {
        return Err(format!("refusing to revoke the current device '{name}'"));
    }
    let mut args = vec!["account", "revoke-device"];
    if let Some(acc) = account {
        args.push("-a");
        args.push(acc);
    }
    args.push(name);
    if sys::mullvad_ok(&args) {
        Ok(())
    } else {
        Err(format!("revoke failed for '{name}'"))
    }
}

/// Log in if needed using the account number; returns Ok when logged in.
pub fn login_if_needed(pass_entry: &str) -> Result<(), String> {
    if is_logged_in() {
        return Ok(());
    }
    let acc = account_number(pass_entry).ok_or_else(|| {
        "not logged in; set MULLVAD_ACCOUNT_NUMBER or store it in pass".to_string()
    })?;
    if sys::mullvad_ok(&["account", "login", &acc]) {
        Ok(())
    } else {
        Err("mullvad account login failed (too many devices? revoke one first)".into())
    }
}

/// Free a slot for this machine: optionally revoke every account device
/// except this exact OS install's own (if it was ever recorded here
/// before), log in, connect, and record this device. `dry_run` only
/// reports what it would revoke.
///
/// "Others" comes from the account's live device list
/// ([`list_devices`]), not the local `slot.state` file: a dual-boot
/// machine has a *different* `/etc/machine-id` (hence a different
/// `state_key()`) per OS install, so the two installs' state files never
/// see each other's recorded name — the other OS's device is invisible
/// to a purely-local lookup. The remote list is account-wide truth, seen
/// identically from either install.
pub fn recycle(revoke_others: bool, pass_entry: &str, dry_run: bool) -> Result<String, String> {
    let my_key = state_key();
    if revoke_others {
        // list-devices / revoke-device need an account number up front —
        // their "current account" fallback needs a prior login, which is
        // exactly what we don't have yet (that's the whole reason we're
        // here). Resolve it once and pass it explicitly to both calls.
        let acc = account_number(pass_entry);

        // Protect this exact install's own last-recorded name, if any —
        // everything else on the account is fair game. (A dual-boot pair
        // of installs sharing one account: every device *is* "me, from
        // the other OS", never a genuinely separate machine.)
        let my_name = read_state()
            .into_iter()
            .find(|(k, _)| *k == my_key)
            .map(|(_, v)| v)
            .unwrap_or_default();
        for dev in list_devices(acc.as_deref()) {
            if dev == my_name {
                continue;
            }
            if dry_run {
                println!("would revoke: {dev}");
                continue;
            }
            match revoke(acc.as_deref(), &dev) {
                Ok(()) => println!("revoked: {dev}"),
                Err(e) => eprintln!("skip: {e}"),
            }
        }
    }
    if dry_run {
        return Ok("dry-run".into());
    }
    login_if_needed(pass_entry)?;
    actions::connect();
    let dev = current_device();
    if !dev.is_empty() {
        record(&my_key, &dev);
    }
    Ok(dev)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_device_name() {
        let s = "Mullvad account: 1234\nDevice name:        Live Coral\nExpires: 2027";
        assert_eq!(parse_device_name(s), "Live Coral");
    }

    #[test]
    fn parses_device_list_drops_header() {
        let s = "Devices on the account:\nCute Puffer\nDreamy Hen\n";
        assert_eq!(parse_devices(s), vec!["Cute Puffer", "Dreamy Hen"]);
    }

    #[test]
    fn state_roundtrip() {
        let pairs = parse_state("DEV_arch=Live Coral\nDEV_cachyos=Cute Puffer\njunk\n");
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[0], ("DEV_arch".into(), "Live Coral".into()));
    }

    #[test]
    fn sanitize_key() {
        assert_eq!(sanitize("cachy os/1"), "cachy_os_1");
    }
}
