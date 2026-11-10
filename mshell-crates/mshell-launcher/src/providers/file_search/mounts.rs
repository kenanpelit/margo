//! Mount tables the `fs` index plans from: the mounted USB / removable
//! volumes (for "Include USB disks"), the mounts a whole-system walk must
//! not enter, and updatedb's pruned paths (for the plocate backend).
//!
//! The shell's USB menu reads udisks2 over D-Bus (`mshell-frame`'s
//! `removable_drives`), but that watcher only runs while a USB pill or
//! menu subscribes to it and lives in a crate the launcher can't depend
//! on. The index only needs the mount points, once per build, off the GTK
//! thread, so it asks `lsblk` (util-linux) with the same removable rule as
//! the USB menu — USB transport or the kernel's removable flag — and falls
//! back to the udisks2 mount layout in `/proc/self/mounts` without it.
//!
//! Everything but [`removable_mounts`] / [`read_mounts`] /
//! [`updatedb_prunepaths`] is pure and unit-tested on captured fixtures.

use margo_fs::path::on_path;
use serde_json::Value;
use std::process::{Command, Stdio};

/// One `/proc/self/mounts` line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountEntry {
    pub path: String,
    pub fstype: String,
    pub options: String,
}

/// Filesystems a whole-system walk may enter: local disk filesystems.
/// Every other mount (pseudo, tmpfs, network, FUSE, overlay, squashfs…)
/// is pruned by its mount point.
const LOCAL_FS: &[&str] = &[
    "ext2", "ext3", "ext4", "btrfs", "xfs", "f2fs", "bcachefs", "zfs", "jfs", "reiserfs", "nilfs2",
    "vfat", "msdos", "exfat", "ntfs", "ntfs3", "fuseblk", "hfs", "hfsplus",
];

/// Always pruned from a whole-system walk (pseudo and scratch trees,
/// snapshot folders of snapper / Timeshift).
pub const SYSTEM_PRUNE: &[&str] = &[
    "/proc",
    "/sys",
    "/dev",
    "/run",
    "/tmp",
    "/.snapshots",
    "/timeshift",
    "/timeshift-btrfs",
];

/// Decode the octal escapes (`\040` = space) of a mount table field;
/// `None` when one decodes to a control byte.
fn unescape_octal(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\'
            && i + 4 <= bytes.len()
            && bytes[i + 1..i + 4]
                .iter()
                .all(|b| (b'0'..=b'7').contains(b))
        {
            let code = (bytes[i + 1] - b'0') as u32 * 64
                + (bytes[i + 2] - b'0') as u32 * 8
                + (bytes[i + 3] - b'0') as u32;
            let byte = u8::try_from(code).ok()?;
            if byte < 32 || byte == 127 {
                return None;
            }
            out.push(byte);
            i += 4;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Parse `/proc/self/mounts`. Lines with an undecodable mount point are
/// skipped.
pub fn parse_proc_mounts(text: &str) -> Vec<MountEntry> {
    text.lines()
        .filter_map(|line| {
            let mut f = line.split(' ');
            let _source = f.next()?;
            let path = unescape_octal(f.next()?)?;
            let fstype = f.next()?.to_string();
            let options = f.next().unwrap_or_default().to_string();
            path.starts_with('/').then_some(MountEntry {
                path,
                fstype,
                options,
            })
        })
        .collect()
}

/// A mount point worth indexing as a USB root: absolute, not `/`, no
/// control character, not the boot partition.
pub fn usable_mount(path: &str) -> bool {
    path.starts_with('/')
        && path != "/"
        && !path.chars().any(char::is_control)
        && path != "/boot"
        && !path.starts_with("/boot/")
}

fn json_flag(v: Option<&Value>) -> bool {
    match v {
        Some(Value::Bool(b)) => *b,
        // util-linux before 2.33 printed flags as "0" / "1".
        Some(Value::String(s)) => s == "1",
        _ => false,
    }
}

fn collect_removable(node: &Value, inherited: bool, out: &mut Vec<String>) {
    let tran = node.get("tran").and_then(Value::as_str).unwrap_or_default();
    let removable = inherited || tran == "usb" || json_flag(node.get("rm"));
    if removable {
        let points = match node.get("mountpoints") {
            Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).collect(),
            _ => node
                .get("mountpoint")
                .and_then(Value::as_str)
                .into_iter()
                .collect::<Vec<_>>(),
        };
        out.extend(
            points
                .into_iter()
                .filter(|p| usable_mount(p))
                .map(|p| p.trim_end_matches('/').to_string()),
        );
    }
    if let Some(Value::Array(children)) = node.get("children") {
        for child in children {
            collect_removable(child, removable, out);
        }
    }
}

/// Mount points of USB / removable volumes in `lsblk -J -o
/// NAME,RM,TRAN,MOUNTPOINTS` output. A partition or an unlocked LUKS
/// mapping counts when any ancestor disk is USB-attached or removable —
/// the rule the shell's USB menu applies to udisks drives. Sorted,
/// deduplicated; `None` when the text isn't lsblk JSON.
pub fn parse_lsblk(json: &str) -> Option<Vec<String>> {
    let v: Value = serde_json::from_str(json).ok()?;
    let devices = v.get("blockdevices")?.as_array()?;
    let mut out = Vec::new();
    for d in devices {
        collect_removable(d, false, &mut out);
    }
    out.sort();
    out.dedup();
    Some(out)
}

/// Without lsblk: the udisks2 mount layout (`/run/media/<user>/<label>`,
/// `/media/<label>`) read from the mount table.
pub fn fallback_removable(mounts: &[MountEntry]) -> Vec<String> {
    let mut out: Vec<String> = mounts
        .iter()
        .map(|m| m.path.as_str())
        .filter(|p| {
            let rest = p
                .strip_prefix("/run/media/")
                .and_then(|r| r.split_once('/').map(|(_, label)| label))
                .or_else(|| p.strip_prefix("/media/"));
            rest.is_some_and(|r| !r.is_empty()) && usable_mount(p)
        })
        .map(str::to_string)
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Drop paths nested under another entry of the (sorted) list.
fn drop_nested(mut list: Vec<String>) -> Vec<String> {
    list.sort();
    list.dedup();
    let mut kept: Vec<String> = Vec::new();
    for p in list {
        let nested = kept.iter().any(|k| is_under(&p, k));
        if !nested {
            kept.push(p);
        }
    }
    kept
}

/// `path` equals `root` or lies below it.
pub fn is_under(path: &str, root: &str) -> bool {
    if root == "/" {
        return path.starts_with('/');
    }
    path == root
        || path
            .strip_prefix(root)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// Mount points a whole-system walk of `/` must not enter:
/// [`SYSTEM_PRUNE`], every non-local filesystem (pseudo, tmpfs, network,
/// FUSE…), btrfs snapshot subvolumes and the removable volumes (searched
/// only through "Include USB disks"). Nested entries are dropped.
pub fn system_prunes(mounts: &[MountEntry], removable: &[String]) -> Vec<String> {
    let mut list: Vec<String> = SYSTEM_PRUNE.iter().map(|s| s.to_string()).collect();
    for m in mounts {
        if m.path == "/" {
            continue;
        }
        let snapshot = m.fstype == "btrfs"
            && m.options
                .split(',')
                .any(|o| o.strip_prefix("subvol=").is_some_and(is_snapshot_subvol));
        if snapshot || !LOCAL_FS.contains(&m.fstype.as_str()) {
            list.push(m.path.clone());
        }
    }
    list.extend(removable.iter().cloned());
    drop_nested(list)
}

/// A btrfs subvolume holding snapshots (`/@snapshots`, `/.snapshots`,
/// `/timeshift-btrfs/snapshots/…`).
fn is_snapshot_subvol(subvol: &str) -> bool {
    let lower = subvol.to_ascii_lowercase();
    lower.contains("snapshot") || lower.contains("timeshift")
}

/// `PRUNEPATHS` of an `updatedb.conf`: folders plocate's database leaves
/// out.
pub fn parse_updatedb_prunepaths(conf: &str) -> Vec<String> {
    conf.lines()
        .filter_map(|l| {
            let (k, v) = l.split_once('=')?;
            (k.trim() == "PRUNEPATHS").then(|| v.trim().trim_matches('"').to_string())
        })
        .flat_map(|v| {
            v.split_whitespace()
                .map(|p| p.trim_end_matches('/').to_string())
                .collect::<Vec<_>>()
        })
        .filter(|p| p.starts_with('/'))
        .collect()
}

/// The current mount table (blocking read; worker threads only).
pub fn read_mounts() -> Vec<MountEntry> {
    std::fs::read_to_string("/proc/self/mounts")
        .map(|t| parse_proc_mounts(&t))
        .unwrap_or_default()
}

/// updatedb's pruned folders (blocking read; worker threads only).
pub fn updatedb_prunepaths() -> Vec<String> {
    std::fs::read_to_string("/etc/updatedb.conf")
        .map(|t| parse_updatedb_prunepaths(&t))
        .unwrap_or_default()
}

/// Mount points of the mounted USB / removable volumes, sorted (blocking:
/// runs `lsblk`; worker threads only). Mounts nothing.
pub fn removable_mounts() -> Vec<String> {
    if on_path("lsblk")
        && let Ok(out) = Command::new("lsblk")
            .args(["-J", "-o", "NAME,RM,TRAN,MOUNTPOINTS"])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
        && out.status.success()
        && let Some(list) = parse_lsblk(&String::from_utf8_lossy(&out.stdout))
    {
        return list;
    }
    fallback_removable(&read_mounts())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `lsblk -J -o NAME,RM,TRAN,MOUNTPOINTS` on this machine (util-linux
    /// 2.42.4): internal NVMe disks and zram only.
    const LSBLK_INTERNAL: &str = include_str!("testdata/lsblk_internal.json");
    /// The same format with a USB stick (a plain partition plus an
    /// unlocked LUKS mapping), an SD card, an internal disk and a
    /// removable disk mounted on /boot.
    const LSBLK_USB: &str = include_str!("testdata/lsblk_usb.json");
    /// `/proc/self/mounts` on this machine.
    const PROC_MOUNTS: &str = include_str!("testdata/proc_mounts.txt");
    /// `/etc/updatedb.conf` on this machine (Arch's mlocate default).
    const UPDATEDB: &str = include_str!("testdata/updatedb.conf");

    #[test]
    fn internal_disks_are_not_removable() {
        assert_eq!(parse_lsblk(LSBLK_INTERNAL), Some(vec![]));
    }

    #[test]
    fn usb_partitions_luks_and_cards_are_removable() {
        assert_eq!(
            parse_lsblk(LSBLK_USB).unwrap(),
            vec![
                "/run/media/u/KINGSTON".to_string(),
                "/run/media/u/SDCARD".into(),
                "/run/media/u/Secure Disk".into(),
            ]
        );
        assert_eq!(parse_lsblk("not json"), None);
        // Old util-linux: string flags and a single `mountpoint`.
        let old =
            r#"{"blockdevices":[{"name":"sdc","rm":"1","tran":null,"mountpoint":"/media/OLD"}]}"#;
        assert_eq!(parse_lsblk(old).unwrap(), vec!["/media/OLD".to_string()]);
    }

    #[test]
    fn proc_mounts_parse_and_unescape() {
        let m = parse_proc_mounts(PROC_MOUNTS);
        assert!(m.iter().any(|e| e.path == "/home" && e.fstype == "btrfs"));
        assert!(
            m.iter()
                .any(|e| e.path == "/run/user/1000/gvfs" && e.fstype == "fuse.gvfsd-fuse")
        );
        let m = parse_proc_mounts(
            "/dev/sdb1 /run/media/u/My\\040Disk vfat rw 0 0\n/dev/x /bad\\012name ext4 rw 0 0\n",
        );
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].path, "/run/media/u/My Disk");
    }

    #[test]
    fn fallback_uses_the_udisks_layout() {
        let m = parse_proc_mounts(
            "/dev/sdb1 /run/media/u/STICK vfat rw 0 0\n/dev/sdc1 /media/CARD exfat rw 0 0\n\
             tmpfs /run/media tmpfs rw 0 0\n/dev/x /run/media/u ext4 rw 0 0\n/dev/y /mnt/data ext4 rw 0 0\n",
        );
        assert_eq!(
            fallback_removable(&m),
            vec!["/media/CARD".to_string(), "/run/media/u/STICK".into()]
        );
    }

    #[test]
    fn whole_system_prunes_pseudo_network_snapshots_and_usb() {
        let mut mounts = parse_proc_mounts(PROC_MOUNTS);
        mounts.extend(parse_proc_mounts(
            "/dev/nvme1n1p2 /.snaps btrfs rw,subvolid=300,subvol=/@snapshots 0 0\n\
             server:/srv /mnt/nfs nfs4 rw 0 0\n\
             /dev/loop0 /var/lib/snapd/snap/core squashfs ro 0 0\n",
        ));
        let p = system_prunes(&mounts, &["/mnt/usb".to_string()]);
        for want in [
            "/proc",
            "/sys",
            "/dev",
            "/run",
            "/tmp",
            "/.snaps",
            "/mnt/nfs",
            "/mnt/usb",
            "/var/lib/snapd/snap/core",
        ] {
            assert!(p.contains(&want.to_string()), "{want} not pruned: {p:?}");
        }
        // Local disks stay searchable; nested pseudo mounts collapse.
        for keep in ["/home", "/repo", "/boot/efi", "/var/tmp", "/cachy", "/"] {
            assert!(!p.contains(&keep.to_string()), "{keep} pruned");
        }
        assert!(
            !p.iter()
                .any(|x| x.starts_with("/sys/") || x.starts_with("/run/"))
        );
    }

    #[test]
    fn updatedb_prunepaths_parse() {
        let p = parse_updatedb_prunepaths(UPDATEDB);
        assert!(p.contains(&"/media".to_string()));
        assert!(p.contains(&"/mnt".to_string()));
        assert!(p.contains(&"/tmp".to_string()));
        assert!(!p.iter().any(|x| x.contains('"')));
    }

    #[test]
    fn usable_and_under() {
        assert!(usable_mount("/run/media/u/X"));
        assert!(!usable_mount("/"));
        assert!(!usable_mount("/boot/efi"));
        assert!(!usable_mount("[SWAP]"));
        assert!(is_under("/a/b", "/a"));
        assert!(is_under("/a", "/a"));
        assert!(!is_under("/ab", "/a"));
        assert!(is_under("/x", "/"));
    }
}
