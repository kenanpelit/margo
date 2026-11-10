//! The `fs` path index: walking its roots (fd, plocate or a built-in
//! walk, chosen per root), cancelling a walk, and keeping each index on
//! disk so switching scopes back never re-walks.

use super::mounts::is_under;
use super::scope::{Root, RootKind};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Hard cap on indexed paths so a pathological tree can't eat memory.
pub const MAX_INDEX: usize = 400_000;
/// Cap for a whole-system index (~1 M paths on a desktop with a few
/// source trees; ~100 MB while held, dropped after idle).
pub const MAX_INDEX_SYSTEM: usize = 1_500_000;
/// Folders never worth indexing.
pub const EXCLUDES: &[&str] = &["node_modules", "target", ".git", "__pycache__"];
/// Volume metadata of other systems, pruned at every depth: never a user
/// file and can add tens of thousands of records on a stick. (Hidden
/// ones — `.Trash-1000`, `.Spotlight-V100`… — are skipped anyway.)
const VOLUME_NOISE: &[&str] = &[
    "lost+found",
    "$RECYCLE.BIN",
    "$Recycle.Bin",
    "RECYCLER",
    "System Volume Information",
    "Backups.backupdb",
    "Network Trash Folder",
    "Temporary Items",
    "TheVolumeSettingsFolder",
];
/// Depth limit for the built-in walk when it follows symlinks (home and
/// extra folders); without following, a deeper limit is safe.
const WALK_DEPTH: usize = 8;
const WALK_DEPTH_NOFOLLOW: usize = 32;

/// Where an index came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Fd,
    Plocate,
    Walk,
}

impl Backend {
    pub fn label(self) -> &'static str {
        match self {
            Backend::Fd => "fd",
            Backend::Plocate => "plocate",
            Backend::Walk => "built-in walk",
        }
    }

    fn tag(self) -> &'static str {
        match self {
            Backend::Fd => "fd",
            Backend::Plocate => "plocate",
            Backend::Walk => "walk",
        }
    }

    fn from_tag(t: &str) -> Option<Self> {
        match t {
            "fd" => Some(Backend::Fd),
            "plocate" => Some(Backend::Plocate),
            "walk" => Some(Backend::Walk),
            _ => None,
        }
    }
}

/// Best available backend.
pub fn choose_backend(has_fd: bool, has_plocate: bool) -> Backend {
    if has_fd {
        Backend::Fd
    } else if has_plocate {
        Backend::Plocate
    } else {
        Backend::Walk
    }
}

/// The backend for one root. fd walks anything. plocate only knows what
/// updatedb indexed: USB disks (mounted after the nightly run, usually
/// under a pruned path) and roots under updatedb's `PRUNEPATHS` take the
/// built-in walk instead. So does `/`: listing a whole plocate database
/// (3.4 M records here) takes plocate over a minute of access checks,
/// several times the built-in walk.
pub fn root_backend(root: &Root, preferred: Backend, updatedb_prune: &[String]) -> Backend {
    match preferred {
        Backend::Plocate
            if matches!(root.kind, RootKind::Usb | RootKind::System)
                || updatedb_prune.iter().any(|p| is_under(&root.path, p)) =>
        {
            Backend::Walk
        }
        b => b,
    }
}

/// One indexed path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub path: String,
    /// `Some` when the backend told us (fd marks folders with a trailing
    /// `/`); `None` → stat lazily for the few rows shown.
    pub is_dir: Option<bool>,
}

impl Entry {
    pub fn file_name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }
}

/// One root's walk as recorded in the index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Walked {
    pub root: String,
    pub backend: Backend,
}

#[derive(Debug, Default)]
pub struct Index {
    pub entries: Vec<Entry>,
    /// The roots, in walk order, and how each was walked.
    pub walks: Vec<Walked>,
    /// USB mount points this index covered (sorted) — compared with the
    /// mounted ones to call it out of date.
    pub usb: Vec<String>,
    /// Configured extra folders that were missing at build time.
    pub missing: Vec<String>,
    pub built_at: Option<SystemTime>,
    /// The entry cap stopped the walk.
    pub capped: bool,
}

impl Index {
    #[cfg(test)]
    pub fn from_entries(entries: Vec<Entry>) -> Self {
        Self {
            entries,
            built_at: Some(SystemTime::now()),
            ..Self::default()
        }
    }

    /// Age of the index (zero when unknown or in the future).
    pub fn age(&self) -> Duration {
        self.built_at
            .and_then(|t| t.elapsed().ok())
            .unwrap_or_default()
    }

    /// "fd", or e.g. "plocate + built-in walk" when roots differ.
    pub fn backend_label(&self) -> String {
        let mut labels: Vec<&str> = Vec::new();
        for w in &self.walks {
            if !labels.contains(&w.backend.label()) {
                labels.push(w.backend.label());
            }
        }
        labels.join(" + ")
    }
}

/// A name a walk skips: hidden, excluded or volume metadata.
pub fn skipped_name(name: &str) -> bool {
    name.starts_with('.') || EXCLUDES.contains(&name) || VOLUME_NOISE.contains(&name)
}

// ── Cancellation ────────────────────────────────────────────────────

/// Cancels a running build: sets a flag the walks poll and kills the
/// child process a walk is reading from (so a blocked read ends at once).
#[derive(Default)]
pub struct Cancel {
    flag: AtomicBool,
    child: Mutex<Option<Child>>,
}

impl Cancel {
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::Release);
        if let Ok(mut slot) = self.child.lock()
            && let Some(child) = slot.as_mut()
        {
            let _ = child.kill();
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::Acquire)
    }
}

// ── Parsers ─────────────────────────────────────────────────────────

/// One `fd` output line: an absolute path, folders with a trailing `/`
/// (fd ≥ 9).
pub fn fd_entry(l: &str) -> Option<Entry> {
    // Not absolute: the tail of a name with a newline in it.
    if !l.starts_with('/') {
        return None;
    }
    Some(match l.strip_suffix('/') {
        Some(dir) if !dir.is_empty() => Entry {
            path: dir.to_string(),
            is_dir: Some(true),
        },
        _ => Entry {
            path: l.to_string(),
            is_dir: Some(false),
        },
    })
}

/// Parse `fd` output: one absolute path per line, folders with a trailing
/// `/` (fd ≥ 9).
pub fn parse_fd(stdout: &str) -> Vec<Entry> {
    stdout
        .lines()
        .filter_map(fd_entry)
        .take(MAX_INDEX)
        .collect()
}

/// One `plocate -r ^<root>/` line, unless it is hidden, under an
/// excluded folder (plocate has no exclude flag) or under one of
/// `prune` — matching what fd would list. `root` ends with `/`.
pub fn locate_entry(l: &str, root: &str, prune: &[String]) -> Option<Entry> {
    let rel = l.strip_prefix(root)?;
    let keep = !rel.is_empty()
        && rel.split('/').all(|c| !skipped_name(c))
        && !prune.iter().any(|p| is_under(l, p));
    keep.then(|| Entry {
        path: l.to_string(),
        is_dir: None,
    })
}

/// Parse `plocate -r ^<root>/` output (see [`locate_entry`]).
pub fn parse_locate(stdout: &str, root: &str) -> Vec<Entry> {
    stdout
        .lines()
        .filter_map(|l| locate_entry(l, root, &[]))
        .take(MAX_INDEX)
        .collect()
}

// ── Argv ────────────────────────────────────────────────────────────

/// A path as an fd glob that matches only itself.
fn glob_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '*' | '?' | '[' | ']' | '{' | '}' | '\\' | '!') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// fd argv indexing `root`, at most `max` results. Home and extra folders
/// follow symlinked folders (the original behaviour); `/` and USB disks
/// don't. `root.prune` is excluded anchored at the root.
pub fn fd_argv(root: &Root, max: usize) -> Vec<String> {
    let follow = matches!(root.kind, RootKind::Home | RootKind::Folder);
    let mut argv: Vec<String> = vec!["fd".into()];
    if follow {
        argv.push("--follow".into());
    }
    argv.extend(
        [
            "--type",
            "f",
            "--type",
            "d",
            "--color",
            "never",
            "--absolute-path",
            "--max-results",
        ]
        .iter()
        .map(|s| s.to_string()),
    );
    argv.push(max.to_string());
    for ex in EXCLUDES.iter().chain(VOLUME_NOISE) {
        argv.push("--exclude".into());
        argv.push(glob_escape(ex));
    }
    for p in &root.prune {
        if let Some(rel) = anchored(&root.path, p) {
            argv.push("--exclude".into());
            argv.push(format!("/{}", glob_escape(rel)));
        }
    }
    argv.push(".".into());
    argv.push(root.path.clone());
    argv
}

/// `path` relative to `root` when strictly below it.
fn anchored<'a>(root: &str, path: &'a str) -> Option<&'a str> {
    if root == "/" {
        path.strip_prefix('/').filter(|r| !r.is_empty())
    } else {
        path.strip_prefix(root)?
            .strip_prefix('/')
            .filter(|r| !r.is_empty())
    }
}

/// `root` as plocate's anchored prefix (always ends with `/`).
pub fn locate_prefix(root: &str) -> String {
    format!("{}/", root.trim_end_matches('/'))
}

/// plocate argv listing everything under `root`.
pub fn plocate_argv(root: &str) -> Vec<String> {
    vec![
        "plocate".into(),
        "-r".into(),
        format!("^{}", regex::escape(&locate_prefix(root))),
    ]
}

// ── Running ─────────────────────────────────────────────────────────

/// Read entries line by line (one reused buffer), at most `max`. Returns
/// the entries and whether the cap stopped the read; stops early (empty
/// result is fine) once `cancel` fires.
fn collect_entries(
    mut reader: impl BufRead,
    max: usize,
    cancel: &Cancel,
    mut entry: impl FnMut(&str) -> Option<Entry>,
) -> (Vec<Entry>, bool) {
    let mut out = Vec::new();
    let mut buf = Vec::new();
    while out.len() < max {
        if cancel.is_cancelled() {
            return (out, false);
        }
        buf.clear();
        match reader.read_until(b'\n', &mut buf) {
            Ok(0) | Err(_) => return (out, false),
            Ok(_) => {}
        }
        let line = String::from_utf8_lossy(&buf);
        if let Some(e) = entry(line.trim_end_matches('\n')) {
            out.push(e);
        }
    }
    (out, true)
}

/// Outcome of one root's command.
#[derive(Debug, PartialEq, Eq)]
pub enum Run {
    Done(Vec<Entry>, bool),
    Failed,
    Cancelled,
}

/// Run `argv` and turn its stdout into entries as it streams in (at most
/// `max`). The output (~20 MB for a 200k-path home) is never held whole.
/// The child is registered with `cancel`, which kills it.
pub fn run_entries(
    argv: &[String],
    max: usize,
    cancel: &Cancel,
    entry: impl FnMut(&str) -> Option<Entry>,
) -> Run {
    let Ok(mut child) = Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return Run::Failed;
    };
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Run::Failed;
    };
    if let Ok(mut slot) = cancel.child.lock() {
        *slot = Some(child);
    }
    // A cancel that landed before the child was registered.
    if cancel.is_cancelled() {
        cancel.cancel();
    }
    let (mut entries, capped) = collect_entries(BufReader::new(stdout), max, cancel, entry);
    let Some(mut child) = cancel.child.lock().ok().and_then(|mut s| s.take()) else {
        return Run::Failed;
    };
    if capped || cancel.is_cancelled() {
        // The rest is not wanted; don't wait for the walk to finish.
        let _ = child.kill();
    }
    let status = child.wait();
    if cancel.is_cancelled() {
        return Run::Cancelled;
    }
    match status {
        Ok(s) if capped || s.success() => {
            entries.shrink_to_fit();
            Run::Done(entries, capped)
        }
        _ => Run::Failed,
    }
}

struct WalkCtx<'a> {
    out: Vec<Entry>,
    max: usize,
    prune: &'a [String],
    follow: bool,
    depth_limit: usize,
    cancel: &'a Cancel,
}

fn walk(dir: &Path, depth: usize, ctx: &mut WalkCtx<'_>) {
    if depth > ctx.depth_limit || ctx.out.len() >= ctx.max || ctx.cancel.is_cancelled() {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        if ctx.out.len() >= ctx.max {
            return;
        }
        let name = e.file_name();
        let name = name.to_string_lossy();
        if skipped_name(&name) || name.contains('\n') {
            continue;
        }
        let path = e.path();
        let path_s = path.to_string_lossy().into_owned();
        if ctx.prune.iter().any(|p| is_under(&path_s, p)) {
            continue;
        }
        // Home and extra folders follow symlinks (the owner's ~/Documents
        // etc. are links; the depth cap bounds any loop); `/` and USB
        // disks don't.
        let is_dir = if ctx.follow {
            path.is_dir()
        } else {
            e.file_type().is_ok_and(|t| t.is_dir())
        };
        ctx.out.push(Entry {
            path: path_s,
            is_dir: Some(is_dir),
        });
        if is_dir {
            walk(&path, depth + 1, ctx);
        }
    }
}

/// The built-in walk of one root.
pub fn walk_root(root: &Root, max: usize, cancel: &Cancel) -> (Vec<Entry>, bool) {
    let follow = matches!(root.kind, RootKind::Home | RootKind::Folder);
    let mut ctx = WalkCtx {
        out: Vec::new(),
        max,
        prune: &root.prune,
        follow,
        depth_limit: if follow {
            WALK_DEPTH
        } else {
            WALK_DEPTH_NOFOLLOW
        },
        cancel,
    };
    walk(Path::new(&root.path), 0, &mut ctx);
    let capped = ctx.out.len() >= max;
    (ctx.out, capped)
}

/// Inputs of one build.
pub struct BuildSpec {
    pub roots: Vec<Root>,
    pub usb: Vec<String>,
    pub missing: Vec<String>,
    /// Best backend installed.
    pub preferred: Backend,
    pub updatedb_prune: Vec<String>,
    pub max: usize,
}

/// Walk every root (blocking; worker thread). `None` when cancelled.
pub fn build_index(spec: &BuildSpec, cancel: &Cancel) -> Option<Index> {
    let mut entries: Vec<Entry> = Vec::new();
    let mut walks = Vec::new();
    let mut capped = false;
    for root in &spec.roots {
        let left = spec.max.saturating_sub(entries.len());
        if left == 0 {
            capped = true;
            break;
        }
        let mut backend = root_backend(root, spec.preferred, &spec.updatedb_prune);
        let run = match backend {
            Backend::Fd => run_entries(&fd_argv(root, left), left, cancel, fd_entry),
            Backend::Plocate => {
                let prefix = locate_prefix(&root.path);
                match run_entries(&plocate_argv(&root.path), left, cancel, |l| {
                    locate_entry(l, &prefix, &root.prune)
                }) {
                    // Nothing in the database for a root that has
                    // content: it's outside updatedb's reach.
                    Run::Done(v, _) if v.is_empty() && root.kind != RootKind::System => Run::Failed,
                    r => r,
                }
            }
            Backend::Walk => Run::Failed,
        };
        let (mut got, hit_cap) = match run {
            Run::Cancelled => return None,
            Run::Done(v, c) => (v, c),
            Run::Failed => {
                backend = Backend::Walk;
                walk_root(root, left, cancel)
            }
        };
        if cancel.is_cancelled() {
            return None;
        }
        capped |= hit_cap;
        entries.append(&mut got);
        walks.push(Walked {
            root: root.path.clone(),
            backend,
        });
    }
    entries.shrink_to_fit();
    Some(Index {
        entries,
        walks,
        usb: spec.usb.clone(),
        missing: spec.missing.clone(),
        built_at: Some(SystemTime::now()),
        capped,
    })
}

// ── On disk ─────────────────────────────────────────────────────────

const MAGIC: &str = "margo-fs-index 1";
/// Index files kept in the cache folder (least recently written dropped).
const KEEP_FILES: usize = 8;

/// FNV-1a: a stable file name per key (std's hasher is not stable
/// across releases).
fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// The index cache folder: `~/.cache/margo/file-search`.
pub fn cache_dir() -> PathBuf {
    margo_fs::xdg::cache_home()
        .join("margo")
        .join("file-search")
}

pub fn index_file(dir: &Path, key: &str) -> PathBuf {
    dir.join(format!("{:016x}.idx", fnv1a(key)))
}

/// Write `index` for `key` (blocking). The folder is private (0700), the
/// file 0600 and replaced by rename — never written through a symlink.
pub fn save(dir: &Path, key: &str, index: &Index) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    let target = index_file(dir, key);
    // Unique per process and call: two builds may save at once.
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let tmp = dir.join(format!(
        ".{}.{}-{}.tmp",
        target
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&tmp);
    let written = (|| {
        let f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        let mut w = BufWriter::new(f);
        let built = index
            .built_at
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        writeln!(w, "{MAGIC}")?;
        writeln!(w, "key {key}")?;
        writeln!(w, "built {built}")?;
        writeln!(w, "capped {}", u8::from(index.capped))?;
        for u in &index.usb {
            writeln!(w, "usb {u}")?;
        }
        for m in &index.missing {
            writeln!(w, "missing {m}")?;
        }
        for wk in &index.walks {
            writeln!(w, "walk {} {}", wk.backend.tag(), wk.root)?;
        }
        writeln!(w, "--")?;
        for e in &index.entries {
            let t = match e.is_dir {
                Some(true) => 'd',
                Some(false) => 'f',
                None => '?',
            };
            writeln!(w, "{t} {}", e.path)?;
        }
        w.into_inner().map_err(|e| e.into_error())?.sync_all()?;
        std::fs::rename(&tmp, &target)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

/// Read the index saved for `key` (blocking). `None` when missing,
/// another format, another key, or damaged. Every record is checked
/// against the roots it claims to come from.
pub fn load(dir: &Path, key: &str) -> Option<Index> {
    let f = std::fs::File::open(index_file(dir, key)).ok()?;
    let mut lines = BufReader::new(f).lines();
    if lines.next()?.ok()? != MAGIC {
        return None;
    }
    let mut index = Index::default();
    let mut key_ok = false;
    for line in lines.by_ref() {
        let line = line.ok()?;
        if line == "--" {
            break;
        }
        let (k, v) = line.split_once(' ')?;
        match k {
            "key" => key_ok = v == key,
            "built" => {
                index.built_at = Some(UNIX_EPOCH + Duration::from_secs(v.parse().ok()?));
            }
            "capped" => index.capped = v == "1",
            "usb" => index.usb.push(v.to_string()),
            "missing" => index.missing.push(v.to_string()),
            "walk" => {
                let (b, root) = v.split_once(' ')?;
                index.walks.push(Walked {
                    root: root.to_string(),
                    backend: Backend::from_tag(b)?,
                });
            }
            _ => {}
        }
    }
    if !key_ok {
        return None;
    }
    for line in lines {
        let line = line.ok()?;
        let (t, path) = line.split_once(' ')?;
        let is_dir = match t {
            "d" => Some(true),
            "f" => Some(false),
            "?" => None,
            _ => return None,
        };
        let valid = path.starts_with('/')
            && !path.split('/').any(|c| c == "..")
            && !path.chars().any(char::is_control)
            && index.walks.iter().any(|w| is_under(path, &w.root));
        if !valid {
            return None;
        }
        index.entries.push(Entry {
            path: path.to_string(),
            is_dir,
        });
    }
    index.entries.shrink_to_fit();
    Some(index)
}

/// Keep the [`KEEP_FILES`] most recently written index files (and drop
/// temp files a crash left behind).
pub fn prune_cache_dir(dir: &Path) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(SystemTime, PathBuf)> = Vec::new();
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Ok(meta) = e.metadata() else { continue };
        let mtime = meta.modified().unwrap_or(UNIX_EPOCH);
        if name.ends_with(".tmp") && name.starts_with('.') {
            if mtime.elapsed().is_ok_and(|a| a > Duration::from_secs(3600)) {
                let _ = std::fs::remove_file(e.path());
            }
        } else if name.ends_with(".idx") && meta.is_file() {
            files.push((mtime, e.path()));
        }
    }
    files.sort_by_key(|f| std::cmp::Reverse(f.0));
    for (_, p) in files.into_iter().skip(KEEP_FILES) {
        let _ = std::fs::remove_file(p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real `fd --follow … . /home/kenan` lines (fd 10.5) with folder marks.
    pub(crate) const FD_OUT: &str = "/home/kenan/Documents/\n/home/kenan/Documents/Alpemix/\n/home/kenan/Documents/Alpemix/AlpemixListem.ini\n/home/kenan/alpakademi.txt\n/home/kenan/margo-git-r2525.34b3f55c-1-x86_64.pkg.tar.zst\n";

    fn root(path: &str, kind: RootKind, prune: &[&str]) -> Root {
        Root {
            path: path.into(),
            kind,
            prune: prune.iter().map(|p| p.to_string()).collect(),
        }
    }

    fn sh(script: &str) -> Vec<String> {
        vec!["sh".to_string(), "-c".into(), script.into()]
    }

    #[test]
    fn streamed_command_output_matches_the_parsers() {
        let c = Cancel::default();
        let script = format!("printf '%s' '{FD_OUT}'");
        assert_eq!(
            run_entries(&sh(&script), MAX_INDEX, &c, fd_entry),
            Run::Done(parse_fd(FD_OUT), false)
        );
        // A failing command is no index (the walk fallback takes over).
        assert_eq!(
            run_entries(&sh("echo /x; exit 3"), MAX_INDEX, &c, fd_entry),
            Run::Failed
        );
        // An endless producer stops at the cap and is killed, not waited on.
        match run_entries(&sh("yes /home/x"), 1000, &c, fd_entry) {
            Run::Done(v, capped) => {
                assert_eq!(v.len(), 1000);
                assert!(capped);
            }
            other => panic!("{other:?}"),
        }
        // A last line without a newline still counts.
        assert!(matches!(
            run_entries(&sh("printf '/a/\\n/a/b'"), MAX_INDEX, &c, fd_entry),
            Run::Done(v, false) if v.len() == 2
        ));
    }

    #[test]
    fn cancel_kills_a_blocked_walk() {
        let c = std::sync::Arc::new(Cancel::default());
        let c2 = std::sync::Arc::clone(&c);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            c2.cancel();
        });
        let t = std::time::Instant::now();
        // Prints one line then blocks far longer than the test waits.
        let r = run_entries(&sh("echo /a; exec sleep 30"), MAX_INDEX, &c, fd_entry);
        assert_eq!(r, Run::Cancelled);
        assert!(t.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn fd_output_marks_folders() {
        let v = parse_fd(FD_OUT);
        assert_eq!(v.len(), 5);
        assert_eq!(v[0].path, "/home/kenan/Documents");
        assert_eq!(v[0].is_dir, Some(true));
        assert_eq!(v[2].is_dir, Some(false));
        assert_eq!(v[2].file_name(), "AlpemixListem.ini");
    }

    #[test]
    fn locate_output_drops_hidden_excluded_pruned_and_root() {
        let out = "/home/u/\n/home/u/a.txt\n/home/u/.cache/x\n/home/u/p/node_modules/y\n/home/u/p/src/main.rs\n/home/uu/other\n/home/u/lost+found/z\n";
        let v = parse_locate(out, "/home/u/");
        let paths: Vec<&str> = v.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, vec!["/home/u/a.txt", "/home/u/p/src/main.rs"]);
        let prune = vec!["/proc".to_string()];
        assert!(locate_entry("/proc/1/x", "/", &prune).is_none());
        assert!(locate_entry("/usr/bin/ls", "/", &prune).is_some());
    }

    #[test]
    fn backend_preference_and_per_root_choice() {
        assert_eq!(choose_backend(true, true), Backend::Fd);
        assert_eq!(choose_backend(false, true), Backend::Plocate);
        assert_eq!(choose_backend(false, false), Backend::Walk);
        let pruned = vec!["/media".to_string(), "/mnt".into()];
        let home = root("/home/u", RootKind::Home, &[]);
        let usb = root("/run/media/u/X", RootKind::Usb, &[]);
        let media = root("/media/data", RootKind::Folder, &[]);
        let srv = root("/srv", RootKind::Folder, &[]);
        assert_eq!(
            root_backend(&home, Backend::Plocate, &pruned),
            Backend::Plocate
        );
        assert_eq!(
            root_backend(&srv, Backend::Plocate, &pruned),
            Backend::Plocate
        );
        assert_eq!(root_backend(&usb, Backend::Plocate, &pruned), Backend::Walk);
        assert_eq!(
            root_backend(&media, Backend::Plocate, &pruned),
            Backend::Walk
        );
        assert_eq!(root_backend(&usb, Backend::Fd, &pruned), Backend::Fd);
        let system = root("/", RootKind::System, &["/proc"]);
        assert_eq!(
            root_backend(&system, Backend::Plocate, &pruned),
            Backend::Walk
        );
        assert_eq!(root_backend(&system, Backend::Fd, &pruned), Backend::Fd);
    }

    #[test]
    fn fd_argv_per_root() {
        let a = fd_argv(&root("/home/u", RootKind::Home, &[]), MAX_INDEX);
        assert!(a.contains(&"--follow".to_string()));
        assert!(
            a.windows(2)
                .any(|w| w[0] == "--exclude" && w[1] == "node_modules")
        );
        assert!(
            a.windows(2)
                .any(|w| w[0] == "--exclude" && w[1] == "lost+found")
        );
        assert_eq!(a.last().map(String::as_str), Some("/home/u"));

        let a = fd_argv(&root("/", RootKind::System, &["/proc", "/mnt/My [x]"]), 7);
        assert!(!a.contains(&"--follow".to_string()));
        assert!(a.windows(2).any(|w| w[0] == "--exclude" && w[1] == "/proc"));
        assert!(
            a.windows(2)
                .any(|w| w[0] == "--exclude" && w[1] == "/mnt/My \\[x\\]")
        );
        assert!(a.windows(2).any(|w| w[0] == "--max-results" && w[1] == "7"));
        assert_eq!(a.last().map(String::as_str), Some("/"));

        let a = fd_argv(&root("/run/media/u/X", RootKind::Usb, &[]), 9);
        assert!(!a.contains(&"--follow".to_string()));
    }

    /// fd itself honours the anchored, escaped prunes (skipped when fd is
    /// not installed).
    #[test]
    fn fd_prunes_on_a_temp_tree() {
        if !margo_fs::path::on_path("fd") {
            return;
        }
        let dir = std::env::temp_dir().join(format!("fs-fd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for d in ["keep", "skip [1]", "deep/skip [1]", "lost+found"] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
            std::fs::write(dir.join(d).join("f.txt"), "").unwrap();
        }
        let base = dir.to_string_lossy().into_owned();
        let r = Root {
            path: base.clone(),
            kind: RootKind::System,
            prune: vec![format!("{base}/skip [1]")],
        };
        let Run::Done(v, _) = run_entries(&fd_argv(&r, 100), 100, &Cancel::default(), fd_entry)
        else {
            panic!("fd failed");
        };
        let rel: Vec<String> = v
            .iter()
            .map(|e| e.path.strip_prefix(&base).unwrap_or(&e.path).to_string())
            .collect();
        assert!(rel.contains(&"/keep/f.txt".to_string()), "{rel:?}");
        assert!(rel.contains(&"/deep/skip [1]/f.txt".to_string()), "{rel:?}");
        assert!(!rel.iter().any(|p| p.starts_with("/skip [1]")), "{rel:?}");
        assert!(!rel.iter().any(|p| p.contains("lost+found")), "{rel:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn plocate_argv_per_root() {
        assert_eq!(
            plocate_argv("/home/u"),
            vec!["plocate".to_string(), "-r".into(), "^/home/u/".into()]
        );
        assert_eq!(plocate_argv("/")[2], "^/");
        assert_eq!(plocate_argv("/media/a.b (1)")[2], "^/media/a\\.b \\(1\\)/");
    }

    #[test]
    fn walk_prunes_and_caps() {
        let dir = std::env::temp_dir().join(format!("fs-walk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for d in ["a/b", "pruned/x", ".hidden", "node_modules/m"] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        }
        std::fs::write(dir.join("a/b/f.txt"), "").unwrap();
        let base = dir.to_string_lossy().into_owned();
        let r = Root {
            path: base.clone(),
            kind: RootKind::Usb,
            prune: vec![format!("{base}/pruned")],
        };
        let (v, capped) = walk_root(&r, 100, &Cancel::default());
        let mut rel: Vec<String> = v
            .iter()
            .map(|e| e.path.strip_prefix(&base).unwrap().to_string())
            .collect();
        rel.sort();
        assert_eq!(rel, vec!["/a", "/a/b", "/a/b/f.txt"]);
        assert!(!capped);
        let (v, capped) = walk_root(&r, 2, &Cancel::default());
        assert_eq!(v.len(), 2);
        assert!(capped);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn build_walks_every_root_within_one_cap() {
        let dir = std::env::temp_dir().join(format!("fs-build-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("one/sub")).unwrap();
        std::fs::create_dir_all(dir.join("two")).unwrap();
        std::fs::write(dir.join("two/t.txt"), "").unwrap();
        let b = |p: &str, kind| Root {
            path: dir.join(p).to_string_lossy().into_owned(),
            kind,
            prune: vec![],
        };
        let spec = BuildSpec {
            roots: vec![b("one", RootKind::Home), b("two", RootKind::Usb)],
            usb: vec!["/run/media/u/X".into()],
            missing: vec![],
            preferred: Backend::Walk,
            updatedb_prune: vec![],
            max: 100,
        };
        let idx = build_index(&spec, &Cancel::default()).unwrap();
        assert_eq!(idx.entries.len(), 2);
        assert_eq!(idx.walks.len(), 2);
        assert_eq!(idx.backend_label(), "built-in walk");
        assert!(!idx.capped);
        let one = BuildSpec { max: 1, ..spec };
        let idx = build_index(&one, &Cancel::default()).unwrap();
        assert!(idx.capped);
        assert_eq!(idx.entries.len(), 1);
        let cancelled = Cancel::default();
        cancelled.cancel();
        assert!(build_index(&one, &cancelled).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_load_round_trip_and_validation() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("fs-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let idx = Index {
            entries: vec![
                Entry {
                    path: "/home/u/docs".into(),
                    is_dir: Some(true),
                },
                Entry {
                    path: "/run/media/u/X/a b.txt".into(),
                    is_dir: None,
                },
            ],
            walks: vec![
                Walked {
                    root: "/home/u".into(),
                    backend: Backend::Fd,
                },
                Walked {
                    root: "/run/media/u/X".into(),
                    backend: Backend::Walk,
                },
            ],
            usb: vec!["/run/media/u/X".into()],
            missing: vec!["/gone".into()],
            built_at: Some(UNIX_EPOCH + Duration::from_secs(1_700_000_000)),
            capped: true,
        };
        save(&dir, "k1", &idx).unwrap();
        let file = index_file(&dir, "k1");
        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let back = load(&dir, "k1").unwrap();
        assert_eq!(back.entries, idx.entries);
        assert_eq!(back.walks, idx.walks);
        assert_eq!(back.usb, idx.usb);
        assert_eq!(back.missing, idx.missing);
        assert_eq!(back.built_at, idx.built_at);
        assert!(back.capped);
        assert!(load(&dir, "k2").is_none());
        // A record outside the recorded roots poisons the file.
        let text = std::fs::read_to_string(&file).unwrap() + "f /etc/passwd\n";
        std::fs::write(&file, text).unwrap();
        assert!(load(&dir, "k1").is_none());
        // A symlink at the target is replaced, not written through.
        let victim = dir.join("victim");
        std::fs::write(&victim, "keep").unwrap();
        std::fs::remove_file(&file).unwrap();
        std::os::unix::fs::symlink(&victim, &file).unwrap();
        save(&dir, "k1", &idx).unwrap();
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep");
        assert!(load(&dir, "k1").is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cache_dir_keeps_the_newest_files() {
        let dir = std::env::temp_dir().join(format!("fs-prune-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..(KEEP_FILES + 3) {
            let p = dir.join(format!("{i:016x}.idx"));
            std::fs::write(&p, "x").unwrap();
            let t = UNIX_EPOCH + Duration::from_secs(1_000_000 + i as u64);
            let f = std::fs::File::options().write(true).open(&p).unwrap();
            f.set_modified(t).unwrap();
        }
        std::fs::write(dir.join("other.txt"), "x").unwrap();
        prune_cache_dir(&dir);
        let mut left: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left.len(), KEEP_FILES + 1);
        assert!(!left.contains(&format!("{:016x}.idx", 0)));
        assert!(left.contains(&"other.txt".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
