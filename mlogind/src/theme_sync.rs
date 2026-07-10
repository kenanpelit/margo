//! The greeter's backdrop and palette: baked by the user, placed by root.
//!
//! `mgreet` renders a blurred copy of the desktop's wallpaper and the matugen
//! colours that go with it. Neither can live where it is written: the greeter
//! runs as `mlogind-greeter` (A2) and `$HOME` is `0710`, so it cannot read a
//! thing out of the user's home. Root must carry them across.
//!
//! Root must not *open* them, though. `~/.cache/mshell/wallpaper.raw` is a path
//! the user controls; a symlink pointing at `/etc/shadow` would be read as root
//! and its derivative written world-readable. So root opens only the outputs, in
//! its own directory, and forks a child that drops to the user to do the reading:
//!
//! ```text
//! root                              child (uid = the user)
//!   open(background.raw.tmp)  → fd
//!   open(theme.css.tmp)       → fd
//!   fork() ───────────────────────▶ alarm(5)
//!                                   setgroups → setgid → setuid
//!                                   read  ~/.cache/mshell/wallpaper.raw
//!                                   downscale + box_blur
//!                                   write(inherited fd)
//!   waitpid ◀─────────────────────  _exit(0)
//!   validate header vs byte count
//!   rename(tmp → final)
//! ```
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

use std::fs::{self, File, OpenOptions, Permissions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use log::warn;

use crate::auth::UserInfo;

/// Machine-written greeter assets. Not `/etc` — see `docs/config-conventions.md`.
pub const STATE_DIR: &str = "/var/lib/mgreet";

const BACKGROUND_NAME: &str = "background.raw";
const THEME_CSS_NAME: &str = "theme.css";

/// Sources, relative to the user's home. All three are read by the child.
const WALLPAPER_REL: &str = ".cache/mshell/wallpaper.raw";
const THEME_CSS_REL: &str = ".cache/mshell/last_theme.css";
const VARIABLES_REL: &str = ".config/margo/mlogind-variables.toml";

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

/// The child's own watchdog. The work is ~10 ms; a wedged reader must never
/// stall the next greeter.
const CHILD_TIMEOUT_SECS: libc::c_uint = 5;
const EXIT_DROP_FAILED: i32 = 70;

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

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// A raw image. Baked by the child, re-validated by the parent.
    Background,
    /// Copied through verbatim.
    Text,
}

/// One file in flight: opened by root, written by the child, renamed by root.
struct Output {
    source: &'static str,
    tmp: PathBuf,
    dest: PathBuf,
    file: File,
    kind: Kind,
}

impl Output {
    fn open(dest: PathBuf, source: &'static str, kind: Kind) -> io::Result<Self> {
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
            source,
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

        let sane = match self.kind {
            Kind::Background => baked_is_sane(&self.tmp, len),
            Kind::Text => len <= MAX_TEXT_BYTES,
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

/// Supplementary groups, then gid, then uid. `setgroups` and `setgid` both need
/// the privilege we are about to give away, so `setuid` goes last.
fn drop_privileges(user: &UserInfo) -> Result<(), nix::Error> {
    use nix::unistd::{Gid, Uid};

    let groups: Vec<Gid> = user.all_gids.iter().copied().map(Gid::from_raw).collect();
    nix::unistd::setgroups(&groups)?;
    nix::unistd::setgid(Gid::from_raw(user.primary_gid))?;
    nix::unistd::setuid(Uid::from_raw(user.uid))?;
    Ok(())
}

/// The forked reader. Runs as the user, writes only to fds it was handed.
///
/// A missing or malformed source leaves its output empty, which the parent reads
/// as "nothing to publish". There is no way to report *why* from here that is
/// worth the interleaved log lines: the parent names every file it published,
/// and the absent ones are the difference.
fn child(user: &UserInfo, outputs: &mut [Output]) -> ! {
    // SAFETY: `alarm` is async-signal-safe and this process is a fresh fork of a
    // single-threaded parent.
    unsafe { libc::alarm(CHILD_TIMEOUT_SECS) };

    if drop_privileges(user).is_err() {
        // SAFETY: skip every destructor — they belong to the parent's heap.
        unsafe { libc::_exit(EXIT_DROP_FAILED) }
    }

    let home = Path::new(&user.home_dir);
    for output in outputs.iter_mut() {
        let cap = match output.kind {
            Kind::Background => MAX_INPUT_BYTES,
            Kind::Text => MAX_TEXT_BYTES,
        };
        let Some(raw) = read_capped(&home.join(output.source), cap) else {
            continue;
        };
        let bytes = match output.kind {
            Kind::Background => match bake(&raw) {
                Some(baked) => baked,
                None => continue,
            },
            Kind::Text => raw,
        };
        let _ = output.file.write_all(&bytes);
    }

    // SAFETY: `File`'s writes are unbuffered syscalls, so there is nothing to
    // flush; `_exit` keeps the parent's destructors from running twice.
    unsafe { libc::_exit(0) }
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

    let variables = PathBuf::from(crate::DEFAULT_VARIABLES_PATH);
    if let Some(parent) = variables.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut outputs = vec![
        Output::open(state.join(BACKGROUND_NAME), WALLPAPER_REL, Kind::Background)?,
        Output::open(state.join(THEME_CSS_NAME), THEME_CSS_REL, Kind::Text)?,
        Output::open(variables, VARIABLES_REL, Kind::Text)?,
    ];

    // SAFETY: mlogind is single-threaded, and the child calls `_exit` without
    // touching an allocator lock it could have inherited mid-acquisition.
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        return Err(io::Error::last_os_error());
    }
    if pid == 0 {
        child(user, &mut outputs);
    }

    let code = crate::wait_for(pid);
    if code != 0 {
        warn!("theme sync: the unprivileged reader exited with {code}");
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
}
