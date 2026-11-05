//! `efibootmgr`'s listing, parsed. Shared by both sides of `boot-next`:
//! the shell lists the entries for its "Reboot into…" menu from the
//! unprivileged listing, and the helper, as root, re-reads the listing
//! and only sets `BootNext` to an entry that is in it.
//!
//! The listing (efibootmgr 18, no `-v`) looks like:
//!
//! ```text
//! BootCurrent: 0001
//! BootNext: 0003
//! Timeout: 0 seconds
//! BootOrder: 0001,0003,001F
//! Boot0001* cachyos-nvme1\tHD(1,GPT,…)/\EFI\cachyos-nvme1\grubx64.efi
//! Boot0010  Setup\tFvFile(…)
//! ```
//!
//! `*` marks an active entry; the label runs up to the tab before the
//! device path (older efibootmgr prints no path at all).

/// One `Boot####` variable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootEntry {
    /// The `####` (hex) number.
    pub num: u16,
    /// The firmware's label, control characters dropped.
    pub label: String,
    /// `LOAD_OPTION_ACTIVE`: the firmware offers it.
    pub active: bool,
    /// The device path efibootmgr printed after the label (may be empty).
    pub path: String,
}

impl BootEntry {
    /// `Boot0003` style id.
    pub fn id(&self) -> String {
        format!("Boot{:04X}", self.num)
    }
}

/// The parsed listing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BootList {
    /// The entry this boot started from.
    pub current: Option<u16>,
    /// The one-shot entry the next boot takes, if set.
    pub next: Option<u16>,
    /// The persistent boot order.
    pub order: Vec<u16>,
    /// Every entry, in listing order.
    pub entries: Vec<BootEntry>,
}

impl BootList {
    pub fn entry(&self, num: u16) -> Option<&BootEntry> {
        self.entries.iter().find(|e| e.num == num)
    }
}

/// Exactly four hex digits (either case).
pub fn parse_num(s: &str) -> Option<u16> {
    (s.len() == 4 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| u16::from_str_radix(s, 16).ok())
        .flatten()
}

fn header(line: &str, key: &str) -> Option<u16> {
    line.strip_prefix(key)?
        .strip_prefix(':')
        .and_then(|v| parse_num(v.trim()))
}

/// Parse `efibootmgr`'s (non-verbose or verbose) output. Unknown lines
/// (the `-v` `dp:` dumps, `Timeout`, `MirroredPercentage…`) are skipped.
pub fn parse(text: &str) -> BootList {
    let mut list = BootList::default();
    for line in text.lines() {
        if let Some(n) = header(line, "BootCurrent") {
            list.current = Some(n);
        } else if let Some(n) = header(line, "BootNext") {
            list.next = Some(n);
        } else if let Some(order) = line.strip_prefix("BootOrder:") {
            list.order = order.trim().split(',').filter_map(parse_num).collect();
        } else if let Some(e) = parse_entry(line) {
            list.entries.push(e);
        }
    }
    list
}

fn parse_entry(line: &str) -> Option<BootEntry> {
    let rest = line.strip_prefix("Boot")?;
    let num = parse_num(rest.get(..4)?)?;
    let rest = &rest[4..];
    let (active, rest) = match rest.strip_prefix('*') {
        Some(r) => (true, r),
        None => (false, rest),
    };
    // A label is separated from the number by whitespace; `BootOrder` and
    // `BootCurrent` never get here (their 4 chars aren't hex digits).
    if !rest.starts_with([' ', '\t']) {
        return None;
    }
    let rest = rest.trim_start_matches(' ');
    let (label, path) = rest.split_once('\t').unwrap_or((rest, ""));
    let label: String = label.trim().chars().filter(|c| !c.is_control()).collect();
    Some(BootEntry {
        num,
        label,
        active,
        path: path.trim().to_string(),
    })
}

/// Whether an entry boots a loader file from a disk partition (an OS, a
/// firmware updater) rather than a firmware shortcut for a device class
/// (`VenMsg(…)` "USB HDD", a PXE/HTTP boot, an `FvFile(…)` setup app).
/// An entry printed without a path (efibootmgr < 18) counts as a loader.
pub fn is_loader(e: &BootEntry) -> bool {
    e.path.is_empty() || e.path.contains("HD(")
}

/// The entries a "Reboot into…" list offers, in listing order: active
/// ones whose label contains none of `hidden` (case-insensitive) and,
/// unless `all`, that boot a loader from disk ([`is_loader`]).
pub fn offered<'a>(list: &'a BootList, hidden: &[String], all: bool) -> Vec<&'a BootEntry> {
    let hidden: Vec<String> = hidden
        .iter()
        .map(|k| k.trim().to_lowercase())
        .filter(|k| !k.is_empty())
        .collect();
    list.entries
        .iter()
        .filter(|e| e.active && !e.label.is_empty() && (all || is_loader(e)))
        .filter(|e| {
            let l = e.label.to_lowercase();
            !hidden.iter().any(|k| l.contains(k.as_str()))
        })
        .collect()
}

/// Find the entry a user named: `0003`, `Boot0003` or the exact label
/// (case-insensitive). A label two entries share is ambiguous: `Err`.
pub fn resolve<'a>(list: &'a BootList, name: &str) -> Result<&'a BootEntry, String> {
    let name = name.trim();
    let hex = name
        .strip_prefix("Boot")
        .or_else(|| name.strip_prefix("boot"))
        .unwrap_or(name);
    if let Some(e) = parse_num(hex).and_then(|n| list.entry(n)) {
        return Ok(e);
    }
    let matches: Vec<&BootEntry> = list
        .entries
        .iter()
        .filter(|e| e.label.eq_ignore_ascii_case(name))
        .collect();
    match matches.as_slice() {
        [one] => Ok(one),
        [] => Err(format!("no boot entry named {name:?}")),
        _ => Err(format!(
            "{} boot entries are named {name:?}; use the number ({})",
            matches.len(),
            matches
                .iter()
                .map(|e| format!("{:04X}", e.num))
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// One line per entry for `mshellctl session boot-entries` and error
/// messages: `0003  cachyos-nvme0  (current)`.
pub fn describe(list: &BootList, e: &BootEntry) -> String {
    let mut tags = Vec::new();
    if list.current == Some(e.num) {
        tags.push("current");
    }
    if list.next == Some(e.num) {
        tags.push("next boot");
    }
    if !e.active {
        tags.push("inactive");
    }
    let tags = if tags.is_empty() {
        String::new()
    } else {
        format!("  ({})", tags.join(", "))
    };
    format!("{:04X}  {}{tags}", e.num, e.label)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `efibootmgr` on the owner's ThinkPad (two CachyOS installs + fwupd).
    const OWNER: &str = include_str!("../tests/data/efibootmgr.txt");

    #[test]
    fn parses_the_owners_listing() {
        let l = parse(OWNER);
        assert_eq!(l.current, Some(0x0001));
        assert_eq!(l.next, None);
        assert_eq!(l.order[..3], [0x0001, 0x0003, 0x001F]);
        assert_eq!(l.order.len(), 13);
        assert_eq!(l.entries.len(), 30);
        let nvme0 = l.entry(0x0003).unwrap();
        assert_eq!(nvme0.label, "cachyos-nvme0");
        assert!(nvme0.active);
        assert!(nvme0.path.starts_with("HD(1,GPT,"));
        let setup = l.entry(0x0010).unwrap();
        assert_eq!(setup.label, "Setup");
        assert!(!setup.active);
        assert_eq!(
            l.entry(0x0014).unwrap().label,
            "ThinkShield Passwordless Power-On Device Manager"
        );
        assert_eq!(l.entry(0x001A).unwrap().id(), "Boot001A");
    }

    #[test]
    fn boot_next_and_old_formats() {
        let l = parse(
            "BootCurrent: 0002\nBootNext: 000a\nBootOrder: 0002\nBoot0002* Windows Boot Manager\nBoot000A* Linux\n",
        );
        assert_eq!(l.next, Some(0x000A));
        assert_eq!(l.entry(0x0002).unwrap().label, "Windows Boot Manager");
        assert_eq!(l.entry(0x0002).unwrap().path, "");
        // Verbose dumps, junk and look-alikes are skipped.
        let v = parse(
            "Boot0001* a\tHD()\n      dp: 04 01\nBootOrderX\nBoot00G1* x\nBoot0002*\nBoot0003\n",
        );
        assert_eq!(v.entries.len(), 1);
        assert_eq!(parse("").entries.len(), 0);
        // Control characters in a firmware label never reach the UI.
        let c = parse("Boot0004* evil\u{1b}[31mred\u{7}\tHD()\n");
        assert_eq!(c.entries[0].label, "evil[31mred");
    }

    #[test]
    fn offered_entries_are_active_and_not_hidden() {
        let l = parse(OWNER);
        let names =
            |v: Vec<&BootEntry>| -> Vec<String> { v.iter().map(|e| e.label.clone()).collect() };
        // By default: the disk loaders (both CachyOS installs, fwupd).
        assert_eq!(
            names(offered(&l, &[], false)),
            ["cachyos-nvme1", "Linux-Firmware-Updater", "cachyos-nvme0"]
        );
        let hidden: Vec<String> = ["FIRMWARE", " ", ""]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            names(offered(&l, &hidden, false)),
            ["cachyos-nvme1", "cachyos-nvme0"]
        );
        // `all`: every active firmware entry, minus the hidden keywords.
        assert_eq!(offered(&l, &[], true).len(), 15);
        let hidden: Vec<String> = ["pxe", "usb", "cloud", "on-premise", "ider", "ata"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            names(offered(&l, &hidden, true)),
            [
                "cachyos-nvme1",
                "Linux-Firmware-Updater",
                "cachyos-nvme0",
                "NVMe0",
                "NVMe1"
            ]
        );
    }

    #[test]
    fn names_resolve_by_number_or_label() {
        let l = parse(OWNER);
        assert_eq!(resolve(&l, "0003").unwrap().label, "cachyos-nvme0");
        assert_eq!(resolve(&l, "Boot0003").unwrap().num, 3);
        assert_eq!(resolve(&l, "001f").unwrap().label, "NVMe0");
        assert_eq!(resolve(&l, "CachyOS-NVMe0").unwrap().num, 3);
        assert_eq!(resolve(&l, " cachyos-nvme1 ").unwrap().num, 1);
        assert!(resolve(&l, "0099").is_err());
        assert!(resolve(&l, "windows").is_err());
        let dup = parse("Boot0001* Linux\nBoot0002* linux\n");
        let e = resolve(&dup, "linux").unwrap_err();
        assert!(e.contains("0001, 0002"), "{e}");
    }

    #[test]
    fn numbers_are_four_hex_digits() {
        assert_eq!(parse_num("001F"), Some(0x1F));
        assert_eq!(parse_num("ffff"), Some(0xFFFF));
        for bad in ["", "1", "00001", "+001", "00 1", "0x01", "g000"] {
            assert_eq!(parse_num(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn entries_describe_their_state() {
        let l = parse(
            "BootCurrent: 0001\nBootNext: 0003\nBoot0001* cachyos-nvme1\tHD()\nBoot0003* cachyos-nvme0\tHD()\nBoot0010  Setup\tFvFile()\n",
        );
        let d: Vec<String> = l.entries.iter().map(|e| describe(&l, e)).collect();
        assert_eq!(
            d,
            [
                "0001  cachyos-nvme1  (current)",
                "0003  cachyos-nvme0  (next boot)",
                "0010  Setup  (inactive)",
            ]
        );
    }
}
