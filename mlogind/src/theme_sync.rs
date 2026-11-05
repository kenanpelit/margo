//! The greeter's backdrop, palette and face: baked by the user, placed by root.
//!
//! `mgreet` renders a blurred copy of the desktop's wallpaper, the matugen
//! colours that go with it, and the user's `~/.face`. None of them can live
//! where it is written: the greeter runs as `mlogind-greeter` (A2) and `$HOME`
//! is `0710`, so it cannot read a thing out of the user's home. Root must carry
//! them across.
//!
//! Root must not *open* them, though. `~/.cache/mshell/wallpaper.raw` is a path
//! the user controls; a symlink pointing at `/etc/shadow` would be read as root
//! and its derivative written world-readable. So root opens only the outputs, in
//! its own directory, and runs a reader that drops to the user to do the
//! reading:
//!
//! ```text
//! root                              reader (uid = the user)
//!   open(background.raw.tmp)  → fd
//!   open(theme.css.tmp)       → fd
//!   spawn /proc/self/exe ─────────▶ pre_exec: alarm(5), clear CLOEXEC on the
//!     __theme-sync-reader             fds, setgroups → setgid → setuid
//!                                   exec: read ~/.cache/mshell/wallpaper.raw
//!                                   downscale + box_blur
//!                                   write(inherited fd)
//!   wait ◀────────────────────────  exit(0)
//!   validate header vs byte count
//!   rename(tmp → final)
//! ```
//!
//! The reader is an `exec`, not a bare `fork`. By the time the runner syncs, the
//! session's log thread is running, so the runner is multi-threaded, and a fork
//! child may only make async-signal-safe calls until it execs: no allocating
//! (the group list, the blur, the file reads all do), since another thread may
//! have held the allocator lock at the moment of the fork. `pre_exec` therefore
//! does nothing but syscalls on data built beforehand, and all the real work
//! happens in a freshly exec'd, single-threaded process.
//!
//! A symlink now buys the attacker exactly the privilege they already had.
//!
//! `mshell` caches the wallpaper already decoded, already theme-filtered, as
//! `[u32 LE width][u32 LE height][RGBA]` — the very pixels the desktop shows. So
//! nothing here decodes an image, and neither this crate nor `mgreet` depends on
//! an image library. Downscaling and blurring are arithmetic over a byte slice.
//!
//! Baking the blur here rather than at render time buys a privacy property for
//! free: what reaches the greeter is a small, blurred derivative. The sharp
//! original never leaves `$HOME`.
//!
//! The avatar is the one thing copied through byte for byte, because GTK will
//! decode it and we will not link an image library to do it first. So it is
//! bounded and sniffed instead: at most [`MAX_AVATAR_BYTES`], and it must open
//! with a PNG or JPEG signature — by the child, which chose it, and again by the
//! parent, which does not trust the child. That keeps `~/.face → ~/.ssh/id_ed25519`
//! from publishing a private key under `/var/lib`.
//!
//! There is one avatar file, not one per user: it belongs to whoever logged in
//! last, which is exactly the user the greeter's cache pre-fills. `mgreet` draws
//! it only while the typed name still matches that user, and a monogram
//! otherwise — so a second user never wears the first one's face.

use std::fs::{self, File, OpenOptions, Permissions};
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use log::warn;

use crate::auth::UserInfo;

/// Machine-written greeter assets. Not `/etc` — see `docs/config-conventions.md`.
pub const STATE_DIR: &str = "/var/lib/mgreet";

const BACKGROUND_NAME: &str = "background.raw";
const THEME_CSS_NAME: &str = "theme.css";
const AVATAR_NAME: &str = "avatar";
const VARIABLES_NAME: &str = "variables.toml";

/// The TUI greeter's synced palette, `/var/lib/mgreet/variables.toml`: the
/// last user's `mlogind-variables.toml`, sanitised by
/// [`crate::config::Variables::sanitize_synced`]. Machine-written state, so not
/// under `/etc` — the admin's `/etc/mlogind/variables.toml` stays theirs, and
/// mlogind overlays this file on it.
pub fn variables_path() -> PathBuf {
    Path::new(STATE_DIR).join(VARIABLES_NAME)
}

/// The baked backdrop's published path, `/var/lib/mgreet/background.raw`. The
/// GUI greeter's throwaway margo config is pointed here (see
/// `write_greeter_conf`) so the compositor's first frame is the same blurred
/// wallpaper `mgreet` paints — no flash of the packaged default in between.
/// World-readable (see [`sync`]), so the unprivileged greeter can load it. The
/// `[u32 LE w][u32 LE h][RGBA]` contract is shared with margo's raw loader.
pub fn background_path() -> PathBuf {
    Path::new(STATE_DIR).join(BACKGROUND_NAME)
}

/// Sources, relative to the user's home. The reader reads them all, taking the
/// first name in each list that both exists and looks like what it claims to be.
const WALLPAPER_REL: &[&str] = &[".cache/mshell/wallpaper.raw"];
const THEME_CSS_REL: &[&str] = &[".cache/mshell/last_theme.css"];
const VARIABLES_REL: &[&str] = &[".config/margo/mlogind-variables.toml"];
/// `~/.face` is the freedesktop convention; `~/.face.icon` is KDE's older spelling.
const AVATAR_REL: &[&str] = &[".face", ".face.icon"];

/// A blurred 960 px image upscaled to 4K is indistinguishable from a blurred 4K
/// one, and costs 2.3 MB instead of 33 MB.
pub const MAX_EDGE: u32 = 960;
/// ≈ 60 px once `Cover` upscales it to a 4K panel.
pub const BLUR_RADIUS: u32 = 12;
/// Three box passes approximate a gaussian. The fourth is not visible.
pub const BLUR_PASSES: usize = 3;

/// 4K RGBA is 33 MB; twice that is generous, and it bounds the child.
const MAX_INPUT_BYTES: u64 = 64 * 1024 * 1024;
const MAX_TEXT_BYTES: u64 = 1024 * 1024;
/// A face, not a photo album. GTK decodes this one in the greeter's address space.
const MAX_AVATAR_BYTES: u64 = 8 * 1024 * 1024;

/// The reader's watchdog, armed before the exec (an alarm survives it). The work
/// is ~10 ms; a wedged reader must never stall the next greeter.
const CHILD_TIMEOUT_SECS: libc::c_uint = 5;

/// `argv[1]` that turns mlogind into the theme-sync reader. Followed by the
/// user's home and one inherited fd number per entry of [`READS`].
pub const READER_ARG: &str = "__theme-sync-reader";
const EXIT_BAD_ARGS: i32 = 64;

const HEADER: usize = 8;

// ── The pure core ────────────────────────────────────────────────────────────

/// `len == 8 + w*h*4`, and neither dimension is zero. Overflow is a rejection,
/// not a wrap: `w` and `h` come off a file the user can write.
fn validate(len: usize, width: u32, height: u32) -> Option<()> {
    if width == 0 || height == 0 {
        return None;
    }
    let body = (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(4)?;
    if len != body.checked_add(HEADER)? {
        return None;
    }
    Some(())
}

/// `(width, height)` of a `[u32 LE w][u32 LE h][RGBA]` buffer whose length agrees.
fn parse_header(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < HEADER {
        return None;
    }
    let width = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let height = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    validate(bytes.len(), width, height)?;
    Some((width, height))
}

/// Fit inside a `max_edge` square, aspect preserved, never zero.
fn target_dims(width: u32, height: u32, max_edge: u32) -> (u32, u32) {
    let long = width.max(height);
    if long <= max_edge || long == 0 {
        return (width, height);
    }
    let scale =
        |v: u32| -> u32 { ((u64::from(v) * u64::from(max_edge)) / u64::from(long)).max(1) as u32 };
    (scale(width), scale(height))
}

/// Area-average resample. Each output pixel is the mean of the source rectangle
/// it covers, so downscaling by a large factor does not alias — which matters,
/// because a blur cannot undo aliasing it was handed.
fn downscale(src: &[u8], width: u32, height: u32, max_edge: u32) -> (Vec<u8>, u32, u32) {
    let (out_w, out_h) = target_dims(width, height, max_edge);
    if (out_w, out_h) == (width, height) {
        return (src.to_vec(), width, height);
    }

    let (w, h) = (width as usize, height as usize);
    let (ow, oh) = (out_w as usize, out_h as usize);
    let mut out = vec![0u8; ow * oh * 4];

    for y in 0..oh {
        let y0 = y * h / oh;
        let y1 = (((y + 1) * h) / oh).max(y0 + 1).min(h);
        for x in 0..ow {
            let x0 = x * w / ow;
            let x1 = (((x + 1) * w) / ow).max(x0 + 1).min(w);

            let mut acc = [0u32; 4];
            let mut n = 0u32;
            for sy in y0..y1 {
                let row = sy * w * 4;
                for sx in x0..x1 {
                    let i = row + sx * 4;
                    for (c, a) in acc.iter_mut().enumerate() {
                        *a += u32::from(src[i + c]);
                    }
                    n += 1;
                }
            }
            let o = (y * ow + x) * 4;
            for (c, a) in acc.iter().enumerate() {
                out[o + c] = (a / n) as u8;
            }
        }
    }
    (out, out_w, out_h)
}

/// One horizontal box pass, via a per-row prefix sum. Edges clamp.
fn blur_h(src: &[u8], dst: &mut [u8], width: usize, height: usize, radius: usize) {
    let mut prefix = vec![0u32; width + 1];
    for y in 0..height {
        let row = y * width * 4;
        for c in 0..3 {
            for x in 0..width {
                prefix[x + 1] = prefix[x] + u32::from(src[row + x * 4 + c]);
            }
            for x in 0..width {
                let lo = x.saturating_sub(radius);
                let hi = (x + radius).min(width - 1);
                let n = (hi - lo + 1) as u32;
                dst[row + x * 4 + c] = ((prefix[hi + 1] - prefix[lo]) / n) as u8;
            }
        }
        for x in 0..width {
            dst[row + x * 4 + 3] = src[row + x * 4 + 3];
        }
    }
}

/// One vertical box pass, via a per-column prefix sum. Edges clamp.
fn blur_v(src: &[u8], dst: &mut [u8], width: usize, height: usize, radius: usize) {
    let mut prefix = vec![0u32; height + 1];
    for x in 0..width {
        for c in 0..3 {
            for y in 0..height {
                prefix[y + 1] = prefix[y] + u32::from(src[(y * width + x) * 4 + c]);
            }
            for y in 0..height {
                let lo = y.saturating_sub(radius);
                let hi = (y + radius).min(height - 1);
                let n = (hi - lo + 1) as u32;
                dst[(y * width + x) * 4 + c] = ((prefix[hi + 1] - prefix[lo]) / n) as u8;
            }
        }
        for y in 0..height {
            dst[(y * width + x) * 4 + 3] = src[(y * width + x) * 4 + 3];
        }
    }
}

/// Separable box blur, `passes` times. Alpha rides along untouched; the caller
/// forces it opaque. Sums are `u32`, which holds any row this ever sees
/// (`MAX_EDGE * 255`, four orders of magnitude short of overflow).
fn box_blur(buf: &mut [u8], width: u32, height: u32, radius: u32, passes: usize) {
    if radius == 0 || passes == 0 || width == 0 || height == 0 {
        return;
    }
    let (w, h, r) = (width as usize, height as usize, radius as usize);
    let mut scratch = vec![0u8; buf.len()];
    for _ in 0..passes {
        blur_h(buf, &mut scratch, w, h, r);
        blur_v(&scratch, buf, w, h, r);
    }
}

/// `wallpaper.raw` → the greeter's backdrop, in the same header format.
///
/// `None` when the input is not a well-formed raw image, which is the only
/// verdict a greeter needs: no background, rather than a broken one.
fn bake(raw: &[u8]) -> Option<Vec<u8>> {
    let (width, height) = parse_header(raw)?;
    let (mut pixels, width, height) = downscale(&raw[HEADER..], width, height, MAX_EDGE);
    box_blur(&mut pixels, width, height, BLUR_RADIUS, BLUR_PASSES);

    // The backdrop is opaque by definition, so no premultiplication question
    // arises and the blur above could ignore alpha entirely.
    for pixel in pixels.chunks_exact_mut(4) {
        pixel[3] = 255;
    }

    let mut out = Vec::with_capacity(HEADER + pixels.len());
    out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&height.to_le_bytes());
    out.extend_from_slice(&pixels);
    Some(out)
}

// ── The privileged half ──────────────────────────────────────────────────────

/// A PNG or JPEG signature. Both loaders GTK reaches for first, and the only two
/// formats a face is ever stored in.
fn is_image(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x89PNG\r\n\x1a\n") || bytes.starts_with(&[0xff, 0xd8, 0xff])
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// A raw image. Baked by the child, re-validated by the parent.
    Background,
    /// An encoded image, copied through verbatim and sniffed at both ends.
    Avatar,
    /// The matugen palette CSS. Must already be a palette for the child to
    /// take it; the parent then publishes its own re-serialisation of the
    /// allowlisted declarations ([`mlogind_proto::theme_css::sanitize`]) —
    /// never the user's bytes, which the next user's greeter would render.
    ThemeCss,
    /// The TUI palette. Copied by the child, then parsed and reduced to the
    /// whitelisted keys by the parent, which publishes its own re-serialised
    /// copy — never the user's bytes.
    Variables,
}

impl Kind {
    /// The most this source may be. The child stops reading here; the parent
    /// refuses to publish anything longer.
    fn cap(self) -> u64 {
        match self {
            Kind::Background => MAX_INPUT_BYTES,
            Kind::Avatar => MAX_AVATAR_BYTES,
            Kind::ThemeCss | Kind::Variables => MAX_TEXT_BYTES,
        }
    }

    /// Is this candidate the thing we came for? Applied to each source name in
    /// turn, so `~/.face` being a text file falls through to `~/.face.icon`
    /// rather than poisoning the slot.
    fn accepts(self, bytes: &[u8]) -> bool {
        match self {
            Kind::Background => parse_header(bytes).is_some(),
            Kind::Avatar => is_image(bytes),
            Kind::ThemeCss => std::str::from_utf8(bytes)
                .is_ok_and(|text| mlogind_proto::theme_css::sanitize(text).is_some()),
            Kind::Variables => std::str::from_utf8(bytes)
                .is_ok_and(|text| crate::config::Variables::sanitize_synced(text).is_some()),
        }
    }
}

/// One file in flight: opened by root, written by the child, renamed by root.
struct Output {
    tmp: PathBuf,
    dest: PathBuf,
    file: File,
    kind: Kind,
}

impl Output {
    fn open(dest: PathBuf, kind: Kind) -> io::Result<Self> {
        let mut tmp = dest.clone().into_os_string();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);

        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o644)
            .open(&tmp)?;
        // `mode` is masked by the umask we inherited. A permission model is not
        // a thing to leave to whoever started us.
        fs::set_permissions(&tmp, Permissions::from_mode(0o644))?;

        Ok(Self {
            tmp,
            dest,
            file,
            kind,
        })
    }

    /// Publish the child's work, or discard it. `Ok(None)` means the source was
    /// absent — leave whatever is already published alone.
    fn commit(self) -> io::Result<Option<PathBuf>> {
        let len = self.file.metadata()?.len();
        if len == 0 {
            let _ = fs::remove_file(&self.tmp);
            return Ok(None);
        }

        let sane = len <= self.kind.cap()
            && match self.kind {
                Kind::Background => baked_is_sane(&self.tmp, len),
                Kind::Avatar => starts_with_image_signature(&self.tmp),
                Kind::ThemeCss => rewrite_sanitized_theme_css(&self.tmp),
                Kind::Variables => rewrite_sanitized_variables(&self.tmp),
            };
        if !sane {
            let _ = fs::remove_file(&self.tmp);
            return Err(io::Error::other(format!(
                "{} is not what the reader promised; discarded",
                self.tmp.display()
            )));
        }

        self.file.sync_all()?;
        drop(self.file);
        fs::rename(&self.tmp, &self.dest)?;
        Ok(Some(self.dest))
    }
}

/// The parent does not trust the child's output any more than the child trusted
/// its input: a header that disagrees with the byte count, or dimensions the
/// downscale was supposed to have bounded, means the file is discarded.
fn baked_is_sane(path: &Path, len: u64) -> bool {
    let Ok(mut file) = File::open(path) else {
        return false;
    };
    let mut head = [0u8; HEADER];
    if file.read_exact(&mut head).is_err() {
        return false;
    }
    let width = u32::from_le_bytes([head[0], head[1], head[2], head[3]]);
    let height = u32::from_le_bytes([head[4], head[5], head[6], head[7]]);

    if width > MAX_EDGE || height > MAX_EDGE {
        return false;
    }
    let Ok(len) = usize::try_from(len) else {
        return false;
    };
    validate(len, width, height).is_some()
}

/// Replace the child's copy of the palette with the parent's own sanitised
/// serialisation of it. `false` (discard) when nothing valid survives.
fn rewrite_sanitized_variables(path: &Path) -> bool {
    let Ok(text) = fs::read_to_string(path) else {
        return false;
    };
    let Some(variables) = crate::config::Variables::sanitize_synced(&text) else {
        return false;
    };
    // `path` is our own `.tmp` in the root-owned STATE_DIR.
    fs::write(path, variables.to_toml_string()).is_ok()
}

/// Replace the child's copy of the theme CSS with the parent's own sanitised
/// serialisation of it. `false` (discard) when it is anything but a palette.
fn rewrite_sanitized_theme_css(path: &Path) -> bool {
    let Ok(text) = fs::read_to_string(path) else {
        return false;
    };
    let Some(css) = mlogind_proto::theme_css::sanitize(&text) else {
        return false;
    };
    // `path` is our own `.tmp` in the root-owned STATE_DIR.
    fs::write(path, css).is_ok()
}

/// The parent sniffs the published avatar itself. The child chose the file, and
/// the child is the user.
fn starts_with_image_signature(path: &Path) -> bool {
    let Ok(mut file) = File::open(path) else {
        return false;
    };
    let mut head = [0u8; 8];
    if file.read_exact(&mut head).is_err() {
        return false;
    }
    is_image(&head)
}

fn read_capped(path: &Path, cap: u64) -> Option<Vec<u8>> {
    let file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    if len == 0 || len > cap {
        return None;
    }
    let mut buf = Vec::with_capacity(len as usize);
    file.take(cap).read_to_end(&mut buf).ok()?;
    Some(buf)
}

/// What the reader fills, in the order of its fd arguments.
const READS: [(&[&str], Kind); 4] = [
    (WALLPAPER_REL, Kind::Background),
    (THEME_CSS_REL, Kind::ThemeCss),
    (AVATAR_REL, Kind::Avatar),
    (VARIABLES_REL, Kind::Variables),
];

/// Read every source under `home` into its output. A missing or malformed
/// source leaves its output empty, which the parent reads as "nothing to
/// publish". There is no way to report *why* from here that is worth the
/// interleaved log lines: the parent names every file it published, and the
/// absent ones are the difference.
fn read_sources(home: &Path, outputs: &mut [(&[&str], Kind, File)]) {
    for (sources, kind, file) in outputs.iter_mut() {
        let kind = *kind;
        let Some(raw) = sources
            .iter()
            .filter_map(|rel| read_capped(&home.join(rel), kind.cap()))
            .find(|raw| kind.accepts(raw))
        else {
            continue;
        };
        let bytes = match kind {
            Kind::Background => match bake(&raw) {
                Some(baked) => baked,
                None => continue,
            },
            Kind::Avatar | Kind::ThemeCss | Kind::Variables => raw,
        };
        let _ = file.write_all(&bytes);
    }
}

/// `home` and one fd per [`READS`] entry, from the reader's argv (after
/// [`READER_ARG`]). Only descriptors above stderr are accepted.
fn parse_reader_args(args: &[std::ffi::OsString]) -> Option<(PathBuf, Vec<RawFd>)> {
    let (home, fds) = args.split_first()?;
    if fds.len() != READS.len() {
        return None;
    }
    let fds = fds
        .iter()
        .map(|fd| fd.to_str()?.parse::<RawFd>().ok().filter(|fd| *fd > 2))
        .collect::<Option<Vec<_>>>()?;
    Some((PathBuf::from(home), fds))
}

/// `mlogind __theme-sync-reader <home> <fd>…`: the unprivileged half, already
/// running as the user (see [`sync`]). Returns the exit code.
pub fn reader_main(args: &[std::ffi::OsString]) -> i32 {
    let Some((home, fds)) = parse_reader_args(args) else {
        return EXIT_BAD_ARGS;
    };
    let mut outputs = Vec::with_capacity(READS.len());
    for (fd, (sources, kind)) in fds.into_iter().zip(READS) {
        // SAFETY: F_GETFD only asks whether `fd` is open.
        if unsafe { libc::fcntl(fd, libc::F_GETFD) } < 0 {
            return EXIT_BAD_ARGS;
        }
        // SAFETY: `fd` is open, was inherited for exactly this purpose, and
        // nothing else in this process refers to it.
        let file = unsafe { File::from_raw_fd(fd) };
        outputs.push((sources, kind, file));
    }
    read_sources(&home, &mut outputs);
    0
}

/// Set FD_CLOEXEC on every descriptor above stderr. Async-signal-safe: only
/// `close_range` or, on kernels before 5.11, `fcntl` per descriptor.
fn mark_all_cloexec() {
    const CLOSE_RANGE_CLOEXEC: libc::c_uint = 1 << 2;
    // SAFETY: plain syscalls on descriptor numbers; none dereferences memory.
    unsafe {
        if libc::syscall(libc::SYS_close_range, 3u32, u32::MAX, CLOSE_RANGE_CLOEXEC) == 0 {
            return;
        }
        let max = libc::sysconf(libc::_SC_OPEN_MAX);
        let max = if max > 0 {
            max.min(65_536) as RawFd
        } else {
            1024
        };
        for fd in 3..max {
            let flags = libc::fcntl(fd, libc::F_GETFD);
            if flags >= 0 {
                libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC);
            }
        }
    }
}

/// Run the reader as `user`, writing into `outputs`. Returns its exit code.
fn run_reader(user: &UserInfo, outputs: &[Output]) -> io::Result<i32> {
    use std::os::unix::process::{CommandExt, ExitStatusExt};

    // Everything the pre_exec hook touches is built here, before the fork.
    let groups: Vec<libc::gid_t> = user.all_gids.clone();
    let (uid, gid) = (user.uid, user.primary_gid);
    let fds: Vec<RawFd> = outputs.iter().map(|o| o.file.as_raw_fd()).collect();

    let mut cmd = Command::new("/proc/self/exe");
    cmd.arg(READER_ARG).arg(&user.home_dir);
    for fd in &fds {
        cmd.arg(fd.to_string());
    }
    cmd.env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: the hook runs between fork and exec in a possibly multi-threaded
    // process, so it only makes async-signal-safe syscalls on the vectors
    // captured above, and allocates nothing (`last_os_error` reads errno).
    unsafe {
        cmd.pre_exec(move || {
            libc::alarm(CHILD_TIMEOUT_SECS);
            // The reader runs as the user, who may ptrace it once it has
            // exec'd. Mark every other descriptor close-on-exec first, so no fd
            // of the root runner (a PAM module's, a socket) crosses the exec.
            mark_all_cloexec();
            for &fd in &fds {
                // Inherit only these, and only into this child.
                if libc::fcntl(fd, libc::F_SETFD, 0) < 0 {
                    return Err(io::Error::last_os_error());
                }
            }
            // Supplementary groups, then gid, then uid. `setgroups` and
            // `setgid` both need the privilege we are about to give away, so
            // `setuid` goes last.
            if libc::setgroups(groups.len(), groups.as_ptr()) < 0
                || libc::setgid(gid) < 0
                || libc::setuid(uid) < 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let status = cmd.status()?;
    Ok(status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(0)))
}

/// Refresh everything the pre-login greeters render from `user`'s desktop.
///
/// Returns the files that were actually published. A source the user never
/// created is not an error — it simply does not appear.
pub fn sync(user: &UserInfo) -> io::Result<Vec<PathBuf>> {
    let state = Path::new(STATE_DIR);
    fs::create_dir_all(state)?;
    // World-readable: `mlogind-greeter` has to get in here.
    fs::set_permissions(state, Permissions::from_mode(0o755))?;

    // Same order as READS: the reader matches fds to sources by position.
    let outputs = vec![
        Output::open(state.join(BACKGROUND_NAME), Kind::Background)?,
        Output::open(state.join(THEME_CSS_NAME), Kind::ThemeCss)?,
        Output::open(state.join(AVATAR_NAME), Kind::Avatar)?,
        Output::open(variables_path(), Kind::Variables)?,
    ];

    match run_reader(user, &outputs) {
        Ok(0) => {}
        Ok(code) => warn!("theme sync: the unprivileged reader exited with {code}"),
        Err(err) => warn!("theme sync: cannot run the unprivileged reader: {err}"),
    }

    let mut written = Vec::new();
    for output in outputs {
        match output.commit() {
            Ok(Some(path)) => written.push(path),
            Ok(None) => {}
            Err(err) => warn!("theme sync: {err}"),
        }
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `[w][h][RGBA]` buffer whose body is `fill`.
    fn raw(width: u32, height: u32, fill: u8) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&width.to_le_bytes());
        buf.extend_from_slice(&height.to_le_bytes());
        buf.resize(HEADER + (width as usize) * (height as usize) * 4, fill);
        buf
    }

    #[test]
    fn a_well_formed_header_round_trips() {
        assert_eq!(parse_header(&raw(3, 5, 0)), Some((3, 5)));
    }

    #[test]
    fn a_header_that_disagrees_with_the_byte_count_is_rejected() {
        let mut buf = raw(3, 5, 0);
        buf.pop();
        assert_eq!(parse_header(&buf), None);
    }

    #[test]
    fn a_truncated_file_is_rejected_rather_than_indexed() {
        assert_eq!(parse_header(&[1, 2, 3]), None);
    }

    #[test]
    fn a_zero_dimension_is_rejected() {
        assert_eq!(parse_header(&raw(0, 5, 0)), None);
        assert_eq!(parse_header(&raw(5, 0, 0)), None);
    }

    #[test]
    fn a_dimension_product_that_would_overflow_is_rejected_not_wrapped() {
        // The user can write this file. `w*h*4` must not wrap into a small number
        // that happens to match the real length.
        let mut buf = Vec::new();
        buf.extend_from_slice(&u32::MAX.to_le_bytes());
        buf.extend_from_slice(&u32::MAX.to_le_bytes());
        buf.resize(HEADER + 4, 0);
        assert_eq!(parse_header(&buf), None);
    }

    #[test]
    fn an_image_already_within_the_bound_is_not_resampled() {
        let (out, w, h) = downscale(&[7u8; 4 * 4], 2, 2, MAX_EDGE);
        assert_eq!((w, h), (2, 2));
        assert_eq!(out, vec![7u8; 16]);
    }

    #[test]
    fn downscaling_preserves_the_aspect_ratio_and_bounds_the_long_edge() {
        assert_eq!(target_dims(1733, 1080, 960), (960, 598));
        assert_eq!(target_dims(1080, 1733, 960), (598, 960));
        assert_eq!(target_dims(100, 50, 960), (100, 50));
    }

    #[test]
    fn a_dimension_never_rounds_down_to_zero() {
        // A 4000×1 strip must not become 960×0.
        let (w, h) = target_dims(4000, 1, 960);
        assert!(w > 0 && h > 0, "got {w}x{h}");
    }

    #[test]
    fn downscaling_averages_the_source_rectangle() {
        // 2×1: black then white. One output pixel is the mean.
        let src = [0, 0, 0, 255, 255, 255, 255, 255];
        let (out, w, h) = downscale(&src, 2, 1, 1);
        assert_eq!((w, h), (1, 1));
        assert_eq!(&out[..3], &[127, 127, 127]);
    }

    #[test]
    fn a_uniform_image_survives_the_blur_unchanged() {
        let mut buf = vec![80u8; 8 * 8 * 4];
        box_blur(&mut buf, 8, 8, 2, 3);
        assert!(buf.chunks_exact(4).all(|p| p[..3] == [80, 80, 80]));
    }

    #[test]
    fn a_zero_radius_blur_is_the_identity() {
        let mut buf: Vec<u8> = (0..64u8).collect();
        let before = buf.clone();
        box_blur(&mut buf, 4, 4, 0, 3);
        assert_eq!(buf, before);
    }

    /// 5×5, the centre pixel lit, red channel only.
    fn impulse(radius: u32, passes: usize) -> impl Fn(usize, usize) -> u8 {
        let mut buf = vec![0u8; 5 * 5 * 4];
        buf[(2 * 5 + 2) * 4] = 255;
        box_blur(&mut buf, 5, 5, radius, passes);
        move |x, y| buf[(y * 5 + x) * 4]
    }

    #[test]
    fn one_box_pass_is_a_flat_kernel_not_a_peak() {
        // A box kernel is flat. One pass turns an impulse into a uniform 3×3
        // block, so the centre is *equal* to its neighbours, not brighter. This
        // is why BLUR_PASSES is 3: a single pass is a box, three is a gaussian.
        let red = impulse(1, 1);
        assert_eq!(red(2, 2), red(1, 2));
        assert_eq!(red(2, 2), red(2, 1));
        assert!(red(2, 2) > 0);
        assert_eq!(red(0, 0), 0, "radius 1 cannot reach two cells away");
    }

    #[test]
    fn three_passes_build_a_symmetric_peak() {
        // An asymmetric kernel is the classic off-by-one in a prefix-sum blur,
        // and it shows up as a light source that has drifted half a pixel.
        let red = impulse(1, 3);
        assert_eq!(red(1, 2), red(3, 2));
        assert_eq!(red(2, 1), red(2, 3));
        assert!(red(2, 2) > red(1, 2));
    }

    #[test]
    fn baking_bounds_the_long_edge_and_forces_the_backdrop_opaque() {
        let baked = bake(&raw(2000, 1000, 40)).expect("a well-formed image bakes");
        let (w, h) = parse_header(&baked).expect("the baked header agrees with its length");
        assert!(w <= MAX_EDGE && h <= MAX_EDGE, "got {w}x{h}");
        assert!(baked[HEADER..].chunks_exact(4).all(|p| p[3] == 255));
    }

    #[test]
    fn baking_refuses_an_image_it_cannot_trust() {
        assert!(bake(&[0u8; 4]).is_none());
        assert!(bake(&raw(0, 0, 0)).is_none());
    }

    #[test]
    fn the_published_palette_is_the_parents_sanitised_copy() {
        let dir = std::env::temp_dir().join(format!("mlogind-vars-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let tmp = dir.join("variables.toml.tmp");
        fs::write(
            &tmp,
            "bg = \"#101010\"\nbg2 = \"#202020\"\npassword_title = \"Type root password here\"\n",
        )
        .unwrap();
        assert!(rewrite_sanitized_variables(&tmp));
        let published = fs::read_to_string(&tmp).unwrap();
        assert!(published.contains("#101010"), "{published}");
        assert!(!published.contains("bg2"), "{published}");
        assert!(!published.contains("root"), "{published}");

        fs::write(&tmp, "bg = 1\n").unwrap();
        assert!(!rewrite_sanitized_variables(&tmp), "nothing valid: discard");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_published_theme_css_is_the_parents_sanitised_copy() {
        let dir = std::env::temp_dir().join(format!("mlogind-css-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let tmp = dir.join("theme.css.tmp");
        fs::write(
            &tmp,
            ":root {\n  --secondary: #102030;\n  --phish: #000;\n}\n",
        )
        .unwrap();
        assert!(rewrite_sanitized_theme_css(&tmp));
        assert_eq!(
            fs::read_to_string(&tmp).unwrap(),
            ":root {\n    --secondary: #102030;\n}\n"
        );

        // A palette that would hide the login text: nothing is published, so
        // the greeter keeps the last good palette or its default.
        fs::write(&tmp, ":root { --on-surface: #0000; }").unwrap();
        assert!(!rewrite_sanitized_theme_css(&tmp));

        // Anything beyond the palette: nothing is published.
        fs::write(
            &tmp,
            "label { background-image: url(\"file:///var/lib/mgreet/avatar\"); }",
        )
        .unwrap();
        assert!(!rewrite_sanitized_theme_css(&tmp));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn reader_args_carry_the_home_and_one_fd_per_output() {
        use std::ffi::OsString;
        let args: Vec<OsString> = ["/home/u", "7", "8", "9", "10"]
            .iter()
            .map(OsString::from)
            .collect();
        assert_eq!(
            parse_reader_args(&args),
            Some((PathBuf::from("/home/u"), vec![7, 8, 9, 10]))
        );
        // Wrong count, non-numbers and the std streams are refused.
        assert_eq!(parse_reader_args(&args[..4]), None);
        let bad: Vec<OsString> = ["/home/u", "7", "x", "9", "10"]
            .iter()
            .map(OsString::from)
            .collect();
        assert_eq!(parse_reader_args(&bad), None);
        let stdio: Vec<OsString> = ["/home/u", "1", "8", "9", "10"]
            .iter()
            .map(OsString::from)
            .collect();
        assert_eq!(parse_reader_args(&stdio), None);
    }

    #[test]
    fn the_reader_fills_each_output_from_the_first_acceptable_source() {
        let base = std::env::temp_dir().join(format!("mlogind-reader-{}", std::process::id()));
        let home = base.join("home");
        fs::create_dir_all(home.join(".cache/mshell")).unwrap();
        fs::create_dir_all(home.join(".config/margo")).unwrap();
        fs::write(home.join(".cache/mshell/wallpaper.raw"), raw(4, 2, 9)).unwrap();
        fs::write(
            home.join(".cache/mshell/last_theme.css"),
            ":root { --primary: #abcdef; }",
        )
        .unwrap();
        // `.face` is not an image, `.face.icon` is: the second one wins.
        fs::write(home.join(".face"), "-----BEGIN OPENSSH PRIVATE KEY-----").unwrap();
        fs::write(home.join(".face.icon"), [0xff, 0xd8, 0xff, 0xe0]).unwrap();
        fs::write(
            home.join(".config/margo/mlogind-variables.toml"),
            "bg = \"#123456\"\n",
        )
        .unwrap();

        let out = |name: &str| {
            OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(true)
                .open(base.join(name))
                .unwrap()
        };
        let mut outputs: Vec<(&[&str], Kind, File)> = READS
            .iter()
            .zip(["bg", "css", "avatar", "vars"])
            .map(|((sources, kind), name)| (*sources, *kind, out(name)))
            .collect();
        read_sources(&home, &mut outputs);

        let read = |name: &str| fs::read(base.join(name)).unwrap();
        assert_eq!(parse_header(&read("bg")), Some((4, 2)));
        assert_eq!(read("css"), b":root { --primary: #abcdef; }");
        assert_eq!(read("avatar"), [0xff, 0xd8, 0xff, 0xe0]);
        assert_eq!(read("vars"), b"bg = \"#123456\"\n");
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn the_palette_is_published_under_the_greeter_state_dir_not_etc() {
        assert_eq!(
            variables_path(),
            Path::new("/var/lib/mgreet/variables.toml")
        );
    }

    #[test]
    fn an_avatar_must_open_with_a_png_or_jpeg_signature() {
        assert!(is_image(b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR"));
        assert!(is_image(&[0xff, 0xd8, 0xff, 0xe0]));
    }

    #[test]
    fn an_avatar_that_is_not_an_image_is_refused() {
        // `~/.face` is a path the user owns. Pointed at a private key, a shell
        // history or a mail spool, it would otherwise be republished under
        // /var/lib, world-readable, by root.
        assert!(!is_image(b"-----BEGIN OPENSSH PRIVATE KEY-----"));
        assert!(!is_image(b"GIF89a"));
        assert!(!is_image(b""));
        assert!(!is_image(b"\x89PNG"), "a truncated signature is not a PNG");
    }

    #[test]
    fn a_source_that_fails_its_own_sniff_falls_through_to_the_next_name() {
        // `.face` before `.face.icon`: the first name that *is* an image wins,
        // not merely the first that exists.
        assert!(!Kind::Avatar.accepts(b"not an image"));
        assert!(Kind::Avatar.accepts(&[0xff, 0xd8, 0xff]));
        assert!(Kind::Background.accepts(&raw(2, 2, 0)));
        assert!(!Kind::Background.accepts(b"short"));
        assert!(Kind::ThemeCss.accepts(b":root { --primary: #fff; }"));
        assert!(!Kind::ThemeCss.accepts(b"label { opacity: 0; }"));
    }
}
