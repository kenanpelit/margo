//! Removable drives over udisks2 — the model behind the USB bar pill and
//! menu. Ports the noctalia `usb-manager` plugin natively: instead of
//! scraping sysfs + `/run/udev/data` and shelling out to `udisksctl` /
//! `gdbus`, mshell talks to the udisks2 daemon on the system bus (zbus).
//! udisks decides through polkit what the user may do; the shell's own
//! polkit agent answers any prompt.
//!
//! * **State** — `ObjectManager.GetManagedObjects` → [`RawObject`]s →
//!   [`build_drives`] (pure, unit-tested). Re-read on udisks
//!   `InterfacesAdded` / `InterfacesRemoved` and on `PropertiesChanged`
//!   of the drive / block / filesystem / LUKS interfaces (debounced), so
//!   mounts made elsewhere show up too — but not on the periodic SMART
//!   updates of internal disks ([`match_rules`]). No polling. The watcher
//!   runs only while something subscribes (a USB pill, an open USB menu).
//! * **Actions** — `Filesystem.Mount/Unmount`, `Encrypted.Unlock/Lock`,
//!   `Drive.PowerOff/Eject`. Safe removal follows [`removal_plan`]:
//!   unmount everything, lock unlocked LUKS containers, then power the
//!   drive off (USB) or eject the medium (card / disc).
//! * **Notifications** — [`diff`] between two snapshots raises a toast on
//!   plug / unplug (a drive yanked while mounted is a warning).
//!
//! When udisks2 isn't running the service publishes `available = false`
//! and the pill stays hidden.

use mshell_osd::toast::{ToastEvent, ToastSeverity, push_toast};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::sync::{Notify, watch};
use tracing::warn;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};
use zbus::{Connection, MatchRule, MessageStream};

const UDISKS: &str = "org.freedesktop.UDisks2";
const UDISKS_ROOT: &str = "/org/freedesktop/UDisks2";
const IF_DRIVE: &str = "org.freedesktop.UDisks2.Drive";
const IF_BLOCK: &str = "org.freedesktop.UDisks2.Block";
const IF_FS: &str = "org.freedesktop.UDisks2.Filesystem";
const IF_ENCRYPTED: &str = "org.freedesktop.UDisks2.Encrypted";
/// Signals arrive in bursts (a plug emits a dozen); coalesce them.
const DEBOUNCE: Duration = Duration::from_millis(400);

// ── Raw udisks objects (decoded from D-Bus) ──────────────────────────

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct RawDrive {
    pub(crate) vendor: String,
    pub(crate) model: String,
    pub(crate) size: u64,
    pub(crate) connection_bus: String,
    pub(crate) removable: bool,
    pub(crate) media_removable: bool,
    pub(crate) ejectable: bool,
    pub(crate) can_power_off: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct RawBlock {
    pub(crate) device: String,
    /// Object path of the owning drive (`/` for none, e.g. dm devices).
    pub(crate) drive: String,
    pub(crate) size: u64,
    pub(crate) id_label: String,
    pub(crate) id_type: String,
    pub(crate) hint_ignore: bool,
    /// For a LUKS cleartext device: the encrypted block it unlocks.
    pub(crate) crypto_backing_device: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct RawObject {
    pub(crate) path: String,
    pub(crate) drive: Option<RawDrive>,
    pub(crate) block: Option<RawBlock>,
    /// `Filesystem.MountPoints` when the object has a Filesystem.
    pub(crate) mount_points: Option<Vec<String>>,
    /// `Encrypted.CleartextDevice` when the object is a LUKS container
    /// (`/` while locked).
    pub(crate) cleartext: Option<String>,
}

// ── Model ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Crypt {
    /// A plain filesystem.
    None,
    /// LUKS container, locked.
    Locked,
    /// LUKS container, unlocked; the cleartext block's object path.
    Unlocked(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Volume {
    /// udisks object path of this block (the LUKS container for crypt).
    pub(crate) path: String,
    pub(crate) device: String,
    pub(crate) label: String,
    pub(crate) fs_type: String,
    pub(crate) size: u64,
    pub(crate) crypt: Crypt,
    /// Object carrying the mountable Filesystem (self, or the cleartext
    /// block of an unlocked container). `None` while locked / when the
    /// cleartext holds no filesystem.
    pub(crate) fs_path: Option<String>,
    pub(crate) mount_points: Vec<String>,
}

impl Volume {
    pub(crate) fn is_mounted(&self) -> bool {
        !self.mount_points.is_empty()
    }

    /// Display name: label, else `<size> <fs>` volume.
    pub(crate) fn title(&self) -> String {
        if !self.label.is_empty() {
            return self.label.clone();
        }
        let fs = if self.fs_type.is_empty() {
            "volume".to_string()
        } else {
            self.fs_type.clone()
        };
        format!("{} {fs}", format_size(self.size))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Drive {
    pub(crate) path: String,
    pub(crate) name: String,
    pub(crate) size: u64,
    pub(crate) can_power_off: bool,
    pub(crate) ejectable: bool,
    pub(crate) volumes: Vec<Volume>,
}

impl Drive {
    pub(crate) fn any_mounted(&self) -> bool {
        self.volumes.iter().any(Volume::is_mounted)
    }

    /// What "Remove safely" ends with, for its tooltip.
    pub(crate) fn removal_hint(&self) -> &'static str {
        if self.can_power_off {
            "Unmount, lock and power off"
        } else if self.ejectable {
            "Unmount, lock and eject"
        } else {
            "Unmount and lock — then unplug"
        }
    }
}

/// Is this drive a removable one the pill should list? USB-attached, or
/// one the kernel flags removable (SD / card readers / optical).
fn is_removable(d: &RawDrive) -> bool {
    d.connection_bus == "usb" || d.removable || d.media_removable
}

/// Build the removable drive list from udisks' managed objects. Pure.
pub(crate) fn build_drives(objects: &[RawObject]) -> Vec<Drive> {
    let mut drives: Vec<Drive> = Vec::new();
    for obj in objects {
        let Some(d) = &obj.drive else { continue };
        if !is_removable(d) {
            continue;
        }
        let mut volumes: Vec<Volume> = objects
            .iter()
            .filter_map(|o| {
                let b = o.block.as_ref()?;
                if b.drive != obj.path || b.hint_ignore {
                    return None;
                }
                volume_of(o, b, objects)
            })
            .collect();
        volumes.sort_by(|a, b| a.device.cmp(&b.device));
        // An empty card reader / drive without media: nothing to show.
        if volumes.is_empty() && d.size == 0 {
            continue;
        }
        let name = format!("{} {}", d.vendor.trim(), d.model.trim())
            .trim()
            .to_string();
        drives.push(Drive {
            path: obj.path.clone(),
            name: if name.is_empty() {
                "Removable drive".to_string()
            } else {
                name
            },
            size: d.size,
            can_power_off: d.can_power_off,
            ejectable: d.ejectable,
            volumes,
        });
    }
    drives.sort_by(|a, b| a.path.cmp(&b.path));
    drives
}

fn volume_of(o: &RawObject, b: &RawBlock, objects: &[RawObject]) -> Option<Volume> {
    let base = Volume {
        path: o.path.clone(),
        device: b.device.clone(),
        label: b.id_label.clone(),
        fs_type: b.id_type.clone(),
        size: b.size,
        crypt: Crypt::None,
        fs_path: None,
        mount_points: Vec::new(),
    };
    if let Some(cleartext) = &o.cleartext {
        if cleartext == "/" || cleartext.is_empty() {
            return Some(Volume {
                crypt: Crypt::Locked,
                ..base
            });
        }
        // Unlocked: the filesystem lives on the cleartext block.
        let clear = objects.iter().find(|c| &c.path == cleartext);
        let fs_type = clear
            .and_then(|c| c.block.as_ref())
            .map(|cb| cb.id_type.clone())
            .unwrap_or_default();
        let label = clear
            .and_then(|c| c.block.as_ref())
            .map(|cb| cb.id_label.clone())
            .filter(|l| !l.is_empty())
            .unwrap_or(base.label.clone());
        let mount_points = clear
            .and_then(|c| c.mount_points.clone())
            .unwrap_or_default();
        let fs_path = clear
            .filter(|c| c.mount_points.is_some())
            .map(|c| c.path.clone());
        return Some(Volume {
            crypt: Crypt::Unlocked(cleartext.clone()),
            fs_type,
            label,
            fs_path,
            mount_points,
            ..base
        });
    }
    let mount_points = o.mount_points.clone()?;
    Some(Volume {
        fs_path: Some(o.path.clone()),
        mount_points,
        ..base
    })
}

/// One step of a safe removal, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Step {
    Unmount(String),
    Lock(String),
    PowerOff(String),
    Eject(String),
}

/// Safe-removal plan: unmount every mounted filesystem, lock every
/// unlocked container, then power off (USB) or eject (media). Pure.
pub(crate) fn removal_plan(d: &Drive) -> Vec<Step> {
    let mut steps = Vec::new();
    for v in &d.volumes {
        if v.is_mounted()
            && let Some(fs) = &v.fs_path
        {
            steps.push(Step::Unmount(fs.clone()));
        }
    }
    for v in &d.volumes {
        if matches!(v.crypt, Crypt::Unlocked(_)) {
            steps.push(Step::Lock(v.path.clone()));
        }
    }
    if d.can_power_off {
        steps.push(Step::PowerOff(d.path.clone()));
    } else if d.ejectable {
        steps.push(Step::Eject(d.path.clone()));
    }
    steps
}

/// Plug / unplug events between two snapshots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DriveEvent {
    Added(String),
    /// `was_mounted`: unplugged without being unmounted first.
    Removed {
        name: String,
        was_mounted: bool,
    },
}

pub(crate) fn diff(old: &[Drive], new: &[Drive]) -> Vec<DriveEvent> {
    let mut events: Vec<DriveEvent> = new
        .iter()
        .filter(|n| !old.iter().any(|o| o.path == n.path))
        .map(|n| DriveEvent::Added(n.name.clone()))
        .collect();
    events.extend(
        old.iter()
            .filter(|o| !new.iter().any(|n| n.path == o.path))
            .map(|o| DriveEvent::Removed {
                name: o.name.clone(),
                was_mounted: o.any_mounted(),
            }),
    );
    events
}

/// Bar pill tooltip: one line per drive + its volumes.
pub(crate) fn tooltip(drives: &[Drive]) -> String {
    let mut lines = Vec::new();
    for d in drives {
        lines.push(format!("{} · {}", d.name, format_size(d.size)));
        for v in &d.volumes {
            let state = match (&v.crypt, v.mount_points.first()) {
                (Crypt::Locked, _) => "locked".to_string(),
                (_, Some(mp)) => mp.clone(),
                _ => "not mounted".to_string(),
            };
            lines.push(format!("    {} — {state}", v.title()));
        }
    }
    lines.push(String::new());
    lines.push("Click: drives  ·  Right-click: remove all safely".to_string());
    lines.join("\n")
}

/// `14.9 GB` (decimal, like file managers and udisks).
pub(crate) fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "kB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1000.0 && i < UNITS.len() - 1 {
        v /= 1000.0;
        i += 1;
    }
    if i == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}

// ── D-Bus decoding ───────────────────────────────────────────────────

type Props = HashMap<String, OwnedValue>;

fn prop_str(p: &Props, k: &str) -> String {
    match p.get(k).map(|v| &**v) {
        Some(Value::Str(s)) => s.to_string(),
        Some(Value::ObjectPath(o)) => o.to_string(),
        _ => String::new(),
    }
}

fn prop_bool(p: &Props, k: &str) -> bool {
    matches!(p.get(k).map(|v| &**v), Some(Value::Bool(true)))
}

fn prop_u64(p: &Props, k: &str) -> u64 {
    match p.get(k).map(|v| &**v) {
        Some(Value::U64(n)) => *n,
        _ => 0,
    }
}

/// udisks byte-string (`ay`, NUL-terminated) → String.
fn bytes_to_string(v: &Value<'_>) -> Option<String> {
    let Value::Array(a) = v else { return None };
    let bytes: Vec<u8> = a
        .iter()
        .filter_map(|b| match b {
            Value::U8(x) => Some(*x),
            _ => None,
        })
        .take_while(|b| *b != 0)
        .collect();
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

fn prop_bytes(p: &Props, k: &str) -> String {
    p.get(k)
        .and_then(|v| bytes_to_string(v))
        .unwrap_or_default()
}

/// `aay` mount points → Vec<String>.
fn prop_mount_points(p: &Props) -> Vec<String> {
    match p.get("MountPoints").map(|v| &**v) {
        Some(Value::Array(a)) => a.iter().filter_map(bytes_to_string).collect(),
        _ => Vec::new(),
    }
}

fn decode(path: String, ifaces: &HashMap<String, Props>) -> RawObject {
    RawObject {
        drive: ifaces.get(IF_DRIVE).map(|p| RawDrive {
            vendor: prop_str(p, "Vendor"),
            model: prop_str(p, "Model"),
            size: prop_u64(p, "Size"),
            connection_bus: prop_str(p, "ConnectionBus"),
            removable: prop_bool(p, "Removable"),
            media_removable: prop_bool(p, "MediaRemovable"),
            ejectable: prop_bool(p, "Ejectable"),
            can_power_off: prop_bool(p, "CanPowerOff"),
        }),
        block: ifaces.get(IF_BLOCK).map(|p| RawBlock {
            device: Some(prop_bytes(p, "PreferredDevice"))
                .filter(|d| !d.is_empty())
                .unwrap_or_else(|| prop_bytes(p, "Device")),
            drive: prop_str(p, "Drive"),
            size: prop_u64(p, "Size"),
            id_label: prop_str(p, "IdLabel"),
            id_type: prop_str(p, "IdType"),
            hint_ignore: prop_bool(p, "HintIgnore"),
            crypto_backing_device: prop_str(p, "CryptoBackingDevice"),
        }),
        mount_points: ifaces.get(IF_FS).map(prop_mount_points),
        cleartext: ifaces
            .get(IF_ENCRYPTED)
            .map(|p| prop_str(p, "CleartextDevice")),
        path,
    }
}

type ManagedObjects = HashMap<OwnedObjectPath, HashMap<String, Props>>;

async fn fetch(conn: &Connection) -> zbus::Result<Vec<Drive>> {
    let proxy = zbus::fdo::ObjectManagerProxy::builder(conn)
        .destination(UDISKS)?
        .path(UDISKS_ROOT)?
        .build()
        .await?;
    let objects = proxy.get_managed_objects().await?;
    let objects: ManagedObjects = objects
        .into_iter()
        .map(|(p, ifs)| {
            (
                p,
                ifs.into_iter()
                    .map(|(name, props)| (name.to_string(), props))
                    .collect(),
            )
        })
        .collect();
    let raw: Vec<RawObject> = objects
        .iter()
        .map(|(p, ifs)| decode(p.to_string(), ifs))
        .collect();
    Ok(build_drives(&raw))
}

// ── Service ──────────────────────────────────────────────────────────

/// Snapshot the widgets render.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct DrivesState {
    /// udisks2 answered on the system bus.
    pub(crate) available: bool,
    pub(crate) drives: Vec<Drive>,
}

struct Service {
    /// Its receiver count is the number of live subscribers.
    tx: watch::Sender<DrivesState>,
    /// Wakes the parked watcher on subscribe.
    wake: Arc<Notify>,
}

static SERVICE: OnceLock<Service> = OnceLock::new();

/// Subscribe to the removable-drive state. Starts the single udisks
/// watcher on first use (per process, not per bar / monitor); it runs
/// while at least one receiver exists, so hold it only while showing it.
pub(crate) fn subscribe() -> watch::Receiver<DrivesState> {
    let service = SERVICE.get_or_init(|| {
        let (tx, _) = watch::channel(DrivesState::default());
        let wake = Arc::new(Notify::new());
        relm4::spawn(run(tx.clone(), wake.clone(), watch_boxed));
        Service { tx, wake }
    });
    let rx = service.tx.subscribe();
    service.wake.notify_one();
    rx
}

/// Current snapshot (empty before the first scan).
pub(crate) fn current() -> DrivesState {
    SERVICE
        .get()
        .map(|s| s.tx.borrow().clone())
        .unwrap_or_default()
}

type WatchFuture = Pin<Box<dyn Future<Output = ()> + Send>>;

fn watch_boxed(tx: watch::Sender<DrivesState>) -> WatchFuture {
    Box::pin(watch_udisks(tx))
}

/// Run `watch` while `tx` has receivers; drop it (and its bus streams)
/// when the last one goes, start it again on the next subscribe. A watch
/// that gives up (no udisks) idles until then too.
async fn run(
    tx: watch::Sender<DrivesState>,
    wake: Arc<Notify>,
    watch: fn(watch::Sender<DrivesState>) -> WatchFuture,
) {
    loop {
        while tx.receiver_count() == 0 {
            wake.notified().await;
        }
        let watching = watch(tx.clone());
        tokio::select! {
            () = async {
                watching.await;
                std::future::pending::<()>().await;
            } => {}
            () = tx.closed() => {}
        }
    }
}

/// The udisks signals that can change the drive list: objects appearing
/// or going, and property changes of the interfaces [`decode`] reads.
/// Everything else udisks emits (SMART / ATA / NVMe health updates of
/// internal disks every few minutes) is never delivered.
fn match_rules() -> zbus::Result<Vec<MatchRule<'static>>> {
    let base = || {
        MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender(UDISKS)
    };
    let mut rules = Vec::new();
    for member in ["InterfacesAdded", "InterfacesRemoved"] {
        rules.push(
            base()?
                .interface("org.freedesktop.DBus.ObjectManager")?
                .member(member)?
                .build(),
        );
    }
    for iface in [IF_DRIVE, IF_BLOCK, IF_FS, IF_ENCRYPTED] {
        rules.push(
            base()?
                .interface("org.freedesktop.DBus.Properties")?
                .member("PropertiesChanged")?
                .arg(0, iface)?
                .build(),
        );
    }
    Ok(rules)
}

async fn watch_udisks(tx: watch::Sender<DrivesState>) {
    use futures::StreamExt;
    let conn = match mshell_services::systemd::system_bus().await {
        Ok(c) => c,
        Err(e) => {
            warn!(error = %e, "usb: no system bus — removable drives disabled");
            return;
        }
    };
    let rules = match match_rules() {
        Ok(r) => r,
        Err(e) => {
            warn!(error = %e, "usb: bad match rule");
            return;
        }
    };
    let mut streams = Vec::with_capacity(rules.len());
    for rule in rules {
        match MessageStream::for_match_rule(rule, &conn, Some(64)).await {
            Ok(s) => streams.push(s),
            Err(e) => {
                warn!(error = %e, "usb: cannot watch udisks2 signals");
                return;
            }
        }
    }
    let mut stream = futures::stream::select_all(streams);
    let mut previous: Option<Vec<Drive>> = None;
    loop {
        match fetch(&conn).await {
            Ok(drives) => {
                if let Some(prev) = &previous {
                    for ev in diff(prev, &drives) {
                        notify(&ev);
                    }
                }
                previous = Some(drives.clone());
                tx.send_if_modified(|s| {
                    let next = DrivesState {
                        available: true,
                        drives,
                    };
                    let changed = *s != next;
                    *s = next;
                    changed
                });
            }
            Err(e) => {
                // udisks2 not running / not installed: hide the pill.
                warn!(error = %e, "usb: udisks2 GetManagedObjects failed");
                tx.send_if_modified(|s| {
                    let changed = s.available;
                    *s = DrivesState::default();
                    changed
                });
            }
        }
        // Wait for the next udisks signal, then drain the burst.
        if stream.next().await.is_none() {
            break;
        }
        loop {
            match tokio::time::timeout(DEBOUNCE, stream.next()).await {
                Ok(Some(_)) => continue,
                Ok(None) => return,
                Err(_) => break,
            }
        }
    }
}

fn notify(ev: &DriveEvent) {
    use mshell_config::config_manager::config_manager;
    use mshell_config::schema::config::{ConfigStoreFields, ToastsStoreFields};
    use reactive_graph::prelude::GetUntracked;
    let toasts = config_manager().config().toasts();
    let (title, body, severity) = match ev {
        DriveEvent::Added(name) => {
            if !toasts.removable_drives().get_untracked() {
                return;
            }
            ("Drive connected", name.clone(), ToastSeverity::Calm)
        }
        DriveEvent::Removed {
            name,
            was_mounted: false,
        } => {
            if !toasts.removable_drives().get_untracked() {
                return;
            }
            ("Drive removed", name.clone(), ToastSeverity::Calm)
        }
        // Always shown: data may be lost.
        DriveEvent::Removed {
            name,
            was_mounted: true,
        } => (
            "Drive unplugged while mounted",
            format!("{name} — unmount it first next time"),
            ToastSeverity::Warn,
        ),
    };
    push_toast(ToastEvent {
        icon: "drive-removable-media-symbolic".to_string(),
        title: title.to_string(),
        body: Some(body),
        severity,
    });
}

// ── Actions ──────────────────────────────────────────────────────────

type Opts<'a> = HashMap<&'a str, Value<'a>>;

async fn proxy<'a>(
    conn: &Connection,
    path: &'a str,
    iface: &'a str,
) -> zbus::Result<zbus::Proxy<'a>> {
    zbus::Proxy::new(conn, UDISKS, path, iface).await
}

/// Mount a filesystem object; returns the mount path.
pub(crate) async fn mount(fs_path: &str) -> zbus::Result<String> {
    let conn = Connection::system().await?;
    let p = proxy(&conn, fs_path, IF_FS).await?;
    p.call("Mount", &(Opts::new(),)).await
}

pub(crate) async fn unmount(fs_path: &str) -> zbus::Result<()> {
    let conn = Connection::system().await?;
    let p = proxy(&conn, fs_path, IF_FS).await?;
    p.call("Unmount", &(Opts::new(),)).await
}

/// Unlock a LUKS container with `passphrase`, then mount the cleartext
/// filesystem (when it has one). Returns the mount path, if mounted.
/// The passphrase only travels to udisks over the system bus; it is never
/// logged or stored.
pub(crate) async fn unlock_and_mount(
    enc_path: &str,
    passphrase: &str,
) -> zbus::Result<Option<String>> {
    let conn = Connection::system().await?;
    let p = proxy(&conn, enc_path, IF_ENCRYPTED).await?;
    let cleartext: OwnedObjectPath = p.call("Unlock", &(passphrase, Opts::new())).await?;
    // The cleartext block's Filesystem interface can appear a moment
    // after Unlock returns; retry the mount briefly.
    for _ in 0..10 {
        match mount(cleartext.as_str()).await {
            Ok(mp) => return Ok(Some(mp)),
            Err(zbus::Error::MethodError(name, _, _))
                if name.as_str() == "org.freedesktop.DBus.Error.UnknownMethod"
                    || name.as_str() == "org.freedesktop.DBus.Error.UnknownInterface" =>
            {
                tokio::time::sleep(Duration::from_millis(150)).await;
            }
            Err(e) => return Err(e),
        }
    }
    Ok(None)
}

/// Unmount the cleartext filesystem (if mounted) and lock the container.
pub(crate) async fn lock(v: &Volume) -> zbus::Result<()> {
    if v.is_mounted()
        && let Some(fs) = &v.fs_path
    {
        unmount(fs).await?;
    }
    let conn = Connection::system().await?;
    let p = proxy(&conn, &v.path, IF_ENCRYPTED).await?;
    p.call("Lock", &(Opts::new(),)).await
}

/// Run a safe-removal [`removal_plan`]; stops at the first failure (a
/// busy filesystem) and reports it.
pub(crate) async fn safely_remove(d: &Drive) -> zbus::Result<()> {
    let conn = Connection::system().await?;
    for step in removal_plan(d) {
        match step {
            Step::Unmount(fs) => {
                proxy(&conn, &fs, IF_FS)
                    .await?
                    .call::<_, _, ()>("Unmount", &(Opts::new(),))
                    .await?
            }
            Step::Lock(enc) => {
                proxy(&conn, &enc, IF_ENCRYPTED)
                    .await?
                    .call::<_, _, ()>("Lock", &(Opts::new(),))
                    .await?
            }
            Step::PowerOff(drive) => {
                proxy(&conn, &drive, IF_DRIVE)
                    .await?
                    .call::<_, _, ()>("PowerOff", &(Opts::new(),))
                    .await?
            }
            Step::Eject(drive) => {
                proxy(&conn, &drive, IF_DRIVE)
                    .await?
                    .call::<_, _, ()>("Eject", &(Opts::new(),))
                    .await?
            }
        }
    }
    Ok(())
}

/// Safely remove `drives` one by one off the main loop, toasting each
/// outcome ("can be unplugged" / why it's still busy).
pub(crate) fn spawn_remove_all(drives: Vec<Drive>) {
    if drives.is_empty() {
        return;
    }
    relm4::spawn(async move {
        for d in drives {
            match safely_remove(&d).await {
                Ok(()) => report("Safe to remove", Ok(format!("{} can be unplugged", d.name))),
                Err(e) => report(&format!("Couldn't remove {}", d.name), Err(error_text(&e))),
            }
        }
    });
}

/// Short user-facing text for a udisks error (`Device is busy`, a
/// polkit refusal, a wrong passphrase…).
pub(crate) fn error_text(e: &zbus::Error) -> String {
    match e {
        zbus::Error::MethodError(_, Some(msg), _) => msg
            .split_once(": ")
            .map(|(_, rest)| rest)
            .unwrap_or(msg)
            .trim()
            .to_string(),
        other => other.to_string(),
    }
}

/// Toast the outcome of a user action.
pub(crate) fn report(title: &str, result: Result<String, String>) {
    let (body, severity) = match result {
        Ok(b) => (b, ToastSeverity::Calm),
        Err(e) => (e, ToastSeverity::Warn),
    };
    push_toast(ToastEvent {
        icon: "drive-removable-media-symbolic".to_string(),
        title: title.to_string(),
        body: Some(body),
        severity,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn match_rules_skip_smart_updates() {
        let rules: Vec<String> = match_rules()
            .expect("rules")
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(rules.len(), 6);
        // Every rule names a member: no catch-all for udisks2 signals.
        assert!(rules.iter().all(|r| r.contains("member=")), "{rules:?}");
        assert!(rules.iter().any(|r| r.contains("member='InterfacesAdded'")));
        for iface in [IF_DRIVE, IF_BLOCK, IF_FS, IF_ENCRYPTED] {
            assert!(
                rules.iter().any(|r| r.contains(&format!("arg0='{iface}'"))),
                "{iface}: {rules:?}"
            );
        }
        assert!(!rules.iter().any(|r| r.contains("Ata")));
    }

    static WATCHES: AtomicUsize = AtomicUsize::new(0);
    static LIVE: AtomicUsize = AtomicUsize::new(0);

    struct Live;
    impl Drop for Live {
        fn drop(&mut self) {
            LIVE.fetch_sub(1, Ordering::SeqCst);
        }
    }

    fn fake_watch(_tx: watch::Sender<DrivesState>) -> WatchFuture {
        WATCHES.fetch_add(1, Ordering::SeqCst);
        LIVE.fetch_add(1, Ordering::SeqCst);
        let live = Live;
        Box::pin(async move {
            let _live = live;
            std::future::pending::<()>().await;
        })
    }

    async fn settle() {
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
    }

    /// The udisks watch (bus connection + signal matches) exists only
    /// while someone holds a receiver.
    #[tokio::test(start_paused = true)]
    async fn watcher_lives_only_while_subscribed() {
        let (tx, _) = watch::channel(DrivesState::default());
        let wake = Arc::new(Notify::new());
        tokio::spawn(run(tx.clone(), wake.clone(), fake_watch));
        tokio::time::sleep(Duration::from_secs(60)).await;
        settle().await;
        assert_eq!(WATCHES.load(Ordering::SeqCst), 0, "nobody subscribed");

        let pill = tx.subscribe();
        wake.notify_one();
        settle().await;
        assert_eq!(LIVE.load(Ordering::SeqCst), 1);
        let menu = tx.subscribe();
        wake.notify_one();
        settle().await;
        assert_eq!(WATCHES.load(Ordering::SeqCst), 1, "one watch for all");

        drop(pill);
        drop(menu);
        settle().await;
        assert_eq!(
            LIVE.load(Ordering::SeqCst),
            0,
            "last receiver gone: dropped"
        );

        let _again = tx.subscribe();
        wake.notify_one();
        settle().await;
        assert_eq!(WATCHES.load(Ordering::SeqCst), 2, "restarts on subscribe");
    }

    fn usb_drive(path: &str, can_power_off: bool) -> RawObject {
        RawObject {
            path: path.to_string(),
            drive: Some(RawDrive {
                vendor: "SanDisk".into(),
                model: "Ultra".into(),
                size: 32_000_000_000,
                connection_bus: "usb".into(),
                removable: true,
                media_removable: false,
                ejectable: false,
                can_power_off,
            }),
            ..Default::default()
        }
    }

    fn block(path: &str, drive: &str, dev: &str) -> RawBlock {
        let _ = path;
        RawBlock {
            device: dev.into(),
            drive: drive.into(),
            size: 16_000_000_000,
            ..Default::default()
        }
    }

    fn fs_obj(path: &str, drive: &str, dev: &str, label: &str, mounts: &[&str]) -> RawObject {
        let mut b = block(path, drive, dev);
        b.id_label = label.into();
        b.id_type = "vfat".into();
        RawObject {
            path: path.into(),
            block: Some(b),
            mount_points: Some(mounts.iter().map(|m| m.to_string()).collect()),
            ..Default::default()
        }
    }

    /// The owner's machine as udisks reports it (two internal NVMe
    /// drives, `ConnectionBus` empty, not removable): nothing to show.
    fn internal_nvme() -> Vec<RawObject> {
        let d = "/org/freedesktop/UDisks2/drives/Timetec_35TT2280GEN4E_2TB_QY250310F2T0001";
        vec![
            RawObject {
                path: d.into(),
                drive: Some(RawDrive {
                    vendor: String::new(),
                    model: "Timetec 35TT2280GEN4E-2TB".into(),
                    size: 2_048_408_248_320,
                    ..Default::default()
                }),
                ..Default::default()
            },
            fs_obj(
                "/org/freedesktop/UDisks2/block_devices/nvme0n1p3",
                d,
                "/dev/nvme0n1p3",
                "",
                &["/repo"],
            ),
        ]
    }

    #[test]
    fn internal_drives_are_not_listed() {
        assert!(build_drives(&internal_nvme()).is_empty());
    }

    #[test]
    fn usb_stick_with_a_mounted_partition() {
        let d = "/org/freedesktop/UDisks2/drives/SanDisk_Ultra_1";
        let mut objs = internal_nvme();
        objs.push(usb_drive(d, true));
        objs.push(fs_obj(
            "/b/sdb1",
            d,
            "/dev/sdb1",
            "STICK",
            &["/run/media/kenan/STICK"],
        ));
        // The whole-disk block (partition table, no filesystem) is skipped.
        objs.push(RawObject {
            path: "/b/sdb".into(),
            block: Some(block("/b/sdb", d, "/dev/sdb")),
            ..Default::default()
        });
        let drives = build_drives(&objs);
        assert_eq!(drives.len(), 1);
        let drive = &drives[0];
        assert_eq!(drive.name, "SanDisk Ultra");
        assert_eq!(drive.volumes.len(), 1);
        assert_eq!(drive.volumes[0].title(), "STICK");
        assert!(drive.any_mounted());
        assert_eq!(drive.volumes[0].fs_path.as_deref(), Some("/b/sdb1"));
    }

    #[test]
    fn hint_ignore_blocks_and_empty_readers_are_hidden() {
        let d = "/d/reader";
        let mut reader = usb_drive(d, false);
        if let Some(rd) = reader.drive.as_mut() {
            rd.size = 0;
        }
        assert!(build_drives(&[reader.clone()]).is_empty(), "no medium");
        let mut ignored = fs_obj("/b/sdc1", d, "/dev/sdc1", "", &[]);
        if let Some(b) = ignored.block.as_mut() {
            b.hint_ignore = true;
        }
        assert!(build_drives(&[reader, ignored]).is_empty());
    }

    #[test]
    fn luks_locked_and_unlocked() {
        let d = "/d/stick";
        let enc = RawObject {
            path: "/b/sdb1".into(),
            block: Some(block("/b/sdb1", d, "/dev/sdb1")),
            cleartext: Some("/".into()),
            ..Default::default()
        };
        let drives = build_drives(&[usb_drive(d, true), enc.clone()]);
        assert_eq!(drives[0].volumes[0].crypt, Crypt::Locked);
        assert_eq!(drives[0].volumes[0].fs_path, None);

        // Unlocked: the cleartext dm block (no drive) carries the fs.
        let mut unlocked = enc;
        unlocked.cleartext = Some("/b/dm_2d0".into());
        let mut clear = fs_obj(
            "/b/dm_2d0",
            "/",
            "/dev/dm-0",
            "SECRET",
            &["/run/media/k/SECRET"],
        );
        if let Some(b) = clear.block.as_mut() {
            b.crypto_backing_device = "/b/sdb1".into();
            b.id_type = "ext4".into();
        }
        let drives = build_drives(&[usb_drive(d, true), unlocked, clear]);
        assert_eq!(
            drives.len(),
            1,
            "the cleartext block is not a drive of its own"
        );
        let v = &drives[0].volumes[0];
        assert_eq!(v.crypt, Crypt::Unlocked("/b/dm_2d0".into()));
        assert_eq!(v.label, "SECRET");
        assert_eq!(v.fs_type, "ext4");
        assert_eq!(v.fs_path.as_deref(), Some("/b/dm_2d0"));
        assert_eq!(v.mount_points, vec!["/run/media/k/SECRET".to_string()]);

        // Safe removal: unmount the cleartext fs, lock, power off.
        assert_eq!(
            removal_plan(&drives[0]),
            vec![
                Step::Unmount("/b/dm_2d0".into()),
                Step::Lock("/b/sdb1".into()),
                Step::PowerOff(d.into()),
            ]
        );
    }

    #[test]
    fn removal_ends_with_eject_or_nothing() {
        let mut drive = Drive {
            path: "/d/sd".into(),
            name: "SD".into(),
            size: 1,
            can_power_off: false,
            ejectable: true,
            volumes: vec![],
        };
        assert_eq!(removal_plan(&drive), vec![Step::Eject("/d/sd".into())]);
        assert_eq!(drive.removal_hint(), "Unmount, lock and eject");
        drive.ejectable = false;
        assert!(removal_plan(&drive).is_empty());
        assert_eq!(drive.removal_hint(), "Unmount and lock — then unplug");
    }

    #[test]
    fn diff_reports_plug_and_unsafe_unplug() {
        let d = "/d/stick";
        let objs = vec![
            usb_drive(d, true),
            fs_obj("/b/sdb1", d, "/dev/sdb1", "STICK", &["/m"]),
        ];
        let plugged = build_drives(&objs);
        assert_eq!(
            diff(&[], &plugged),
            vec![DriveEvent::Added("SanDisk Ultra".into())]
        );
        assert_eq!(
            diff(&plugged, &[]),
            vec![DriveEvent::Removed {
                name: "SanDisk Ultra".into(),
                was_mounted: true
            }]
        );
        assert!(diff(&plugged, &plugged).is_empty());
    }

    #[test]
    fn decodes_udisks_byte_strings() {
        let v = Value::new(b"/dev/sdb1\0".to_vec());
        assert_eq!(bytes_to_string(&v).as_deref(), Some("/dev/sdb1"));
        let mut props: Props = HashMap::new();
        props.insert(
            "MountPoints".into(),
            OwnedValue::try_from(Value::new(vec![b"/run/media/k/A\0".to_vec()])).unwrap(),
        );
        assert_eq!(
            prop_mount_points(&props),
            vec!["/run/media/k/A".to_string()]
        );
    }

    #[test]
    fn sizes_and_tooltip() {
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(32_000_000_000), "32.0 GB");
        let d = "/d/stick";
        let drives = build_drives(&[
            usb_drive(d, true),
            fs_obj("/b/sdb1", d, "/dev/sdb1", "", &[]),
        ]);
        let t = tooltip(&drives);
        assert!(
            t.starts_with("SanDisk Ultra · 32.0 GB\n    16.0 GB vfat — not mounted"),
            "{t}"
        );
        assert!(t.ends_with("Click: drives  ·  Right-click: remove all safely"));
    }
}
