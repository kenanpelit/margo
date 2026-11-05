//! Incremental, bounded-memory reading of Claude Code's append-only JSONL
//! files (`history.jsonl`, session transcripts).
//!
//! Transcripts reach hundreds of MB, so nothing here reads a file whole:
//! lines stream through one reused buffer, a file's first visit of the day
//! seeks backwards from the end to the first line of the day (the files
//! are chronological), and later visits only read what was appended.

use serde::Deserialize;
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use super::limits::parse_iso_ms;

/// Backward-scan step.
const CHUNK: u64 = 64 * 1024;
/// A line longer than this is skipped by the backward scan (its timestamp
/// is not looked at; the scan carries on to the line before it).
const MAX_SCAN_LINE: usize = 4 * 1024 * 1024;
/// The forward line buffer is dropped after a line longer than this, so one
/// huge tool result doesn't stay allocated.
const KEEP_BUF: usize = 1024 * 1024;

/// Where an incremental reader is in one file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct JsonlTail {
    /// `(dev, ino)` of the file the offset belongs to.
    id: Option<(u64, u64)>,
    /// Bytes consumed (always the end of a complete line).
    offset: u64,
}

/// What one [`JsonlTail::read`] did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ReadOutcome {
    /// The file was read from a fresh start point (first visit, replaced
    /// or truncated): anything derived from earlier reads is stale.
    pub(crate) restarted: bool,
    /// Bytes actually read from disk.
    pub(crate) bytes: u64,
}

impl JsonlTail {
    /// Feed every complete line appended since the last call to `f`. On a
    /// fresh start, reading begins at the first line whose `timestamp` is
    /// at or after `since_ms` (`None` = the start of the file).
    pub(crate) fn read(
        &mut self,
        path: &Path,
        since_ms: Option<i64>,
        f: impl FnMut(&str),
    ) -> io::Result<ReadOutcome> {
        use std::os::unix::fs::MetadataExt;
        let file = std::fs::File::open(path)?;
        let meta = file.metadata()?;
        let id = Some((meta.dev(), meta.ino()));
        let len = meta.len();
        let resume = (self.id == id && self.offset <= len).then_some(self.offset);
        let mut counted = Counting::new(file);
        let next = read_from(&mut counted, len, resume, since_ms, f)?;
        self.id = id;
        self.offset = next;
        Ok(ReadOutcome {
            restarted: resume.is_none(),
            bytes: counted.read,
        })
    }
}

/// The reading core, generic so tests can count the bytes it touches.
/// Starts at `resume`, or (when `None`) at the first line since `since_ms`.
/// Returns the offset after the last complete line.
pub(crate) fn read_from<R: Read + Seek>(
    r: &mut R,
    len: u64,
    resume: Option<u64>,
    since_ms: Option<i64>,
    mut f: impl FnMut(&str),
) -> io::Result<u64> {
    let start = match (resume, since_ms) {
        (Some(o), _) => o,
        (None, Some(since)) => first_line_since(r, len, since)?,
        (None, None) => 0,
    };
    if start >= len {
        return Ok(start);
    }
    r.seek(SeekFrom::Start(start))?;
    let mut reader = BufReader::with_capacity(CHUNK as usize, r);
    let mut buf = Vec::new();
    let mut consumed = start;
    loop {
        buf.clear();
        let n = reader.read_until(b'\n', &mut buf)?;
        if n == 0 || buf.last() != Some(&b'\n') {
            // EOF, or a line still being written: leave it for next time.
            break;
        }
        consumed += n as u64;
        let line = &buf[..n - 1];
        f(&String::from_utf8_lossy(line));
        if buf.capacity() > KEEP_BUF {
            buf = Vec::new();
        }
    }
    Ok(consumed)
}

/// Offset of the line after the last line stamped before `since_ms`,
/// found by scanning backwards from `len` (0 when every line is newer).
fn first_line_since<R: Read + Seek>(r: &mut R, len: u64, since_ms: i64) -> io::Result<u64> {
    let mut chunk = vec![0u8; CHUNK as usize];
    // The current (partial) line, gathered back to front.
    let mut pieces: Vec<Vec<u8>> = Vec::new();
    let mut piece_len = 0usize;
    let mut overflow = false;
    let mut line = Vec::new();
    // End (exclusive, after its '\n') of the line being gathered.
    let mut line_end = len;
    let mut pos = len;
    let mut is_old = |pieces: &mut Vec<Vec<u8>>, overflow: bool| {
        if overflow {
            pieces.clear();
            return false;
        }
        line.clear();
        for p in pieces.drain(..).rev() {
            line.extend_from_slice(&p);
        }
        line_timestamp(&line).is_some_and(|ts| ts < since_ms)
    };
    while pos > 0 {
        let start = pos.saturating_sub(CHUNK);
        let buf = &mut chunk[..(pos - start) as usize];
        r.seek(SeekFrom::Start(start))?;
        r.read_exact(buf)?;
        let mut seg_end = buf.len();
        for i in (0..buf.len()).rev() {
            if buf[i] != b'\n' {
                continue;
            }
            let seg = &buf[i + 1..seg_end];
            piece_len += seg.len();
            if !overflow && piece_len <= MAX_SCAN_LINE {
                pieces.push(seg.to_vec());
            } else {
                overflow = true;
            }
            if is_old(&mut pieces, overflow) {
                return Ok(line_end);
            }
            (piece_len, overflow) = (0, false);
            line_end = start + i as u64 + 1;
            seg_end = i;
        }
        let seg = &buf[..seg_end];
        piece_len += seg.len();
        if !overflow && piece_len <= MAX_SCAN_LINE {
            pieces.push(seg.to_vec());
        } else {
            overflow = true;
        }
        pos = start;
    }
    // The file's first line.
    Ok(if is_old(&mut pieces, overflow) {
        line_end
    } else {
        0
    })
}

/// A line's top-level `timestamp`: an ISO string (transcripts) or unix ms
/// (`history.jsonl`). Nested objects' timestamps are not looked at.
pub(crate) fn line_timestamp(line: &[u8]) -> Option<i64> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Stamp {
        Ms(i64),
        Iso(String),
    }
    #[derive(Deserialize)]
    struct Line {
        timestamp: Option<Stamp>,
    }
    match serde_json::from_slice::<Line>(line).ok()?.timestamp? {
        Stamp::Ms(ms) => Some(ms),
        Stamp::Iso(s) => parse_iso_ms(&s),
    }
}

/// Counts the bytes read through it.
pub(crate) struct Counting<R> {
    inner: R,
    pub(crate) read: u64,
}

impl<R> Counting<R> {
    pub(crate) fn new(inner: R) -> Self {
        Counting { inner, read: 0 }
    }
}

impl<R: Read> Read for Counting<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.read += n as u64;
        Ok(n)
    }
}

impl<R: Seek> Seek for Counting<R> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.inner.seek(pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};

    fn line(ts_ms: i64, filler: usize) -> String {
        format!(
            r#"{{"display":"{}","timestamp":{ts_ms},"sessionId":"s"}}"#,
            "x".repeat(filler)
        ) + "\n"
    }

    fn lines_in(data: &[u8], resume: Option<u64>, since: Option<i64>) -> (Vec<i64>, u64, u64) {
        let mut r = Counting::new(Cursor::new(data));
        let mut seen = Vec::new();
        let next = read_from(&mut r, data.len() as u64, resume, since, |l| {
            seen.push(line_timestamp(l.as_bytes()).unwrap_or(-1));
        })
        .unwrap();
        (seen, next, r.read)
    }

    #[test]
    fn a_first_visit_reads_only_todays_tail_of_a_big_file() {
        // ~10 MB of yesterday, then three lines of today.
        let mut data = Vec::new();
        for i in 0..40_000 {
            data.extend(line(1_000 + i, 240).as_bytes());
        }
        let old_len = data.len() as u64;
        assert!(old_len > 10_000_000);
        for ts in [100_000, 100_001, 100_002] {
            data.extend(line(ts, 240).as_bytes());
        }
        let (seen, next, read) = lines_in(&data, None, Some(100_000));
        assert_eq!(seen, [100_000, 100_001, 100_002]);
        assert_eq!(next, data.len() as u64);
        // One 64 KiB backward step plus the forward read of the tail.
        assert!(read <= 3 * CHUNK, "read {read} bytes of {}", data.len());
    }

    #[test]
    fn the_backward_scan_finds_the_boundary_across_chunks_and_long_lines() {
        let mut data = Vec::new();
        data.extend(line(1, 10).as_bytes());
        data.extend(line(2, 10).as_bytes());
        let boundary = data.len() as u64;
        // A line longer than a chunk, then one too long to scan (no
        // timestamp is looked at), then lines without a timestamp.
        data.extend(line(5, 3 * CHUNK as usize).as_bytes());
        data.extend(line(6, MAX_SCAN_LINE + 10).as_bytes());
        data.extend(b"{\"type\":\"mode\"}\n");
        data.extend(line(7, 10).as_bytes());
        let mut r = Cursor::new(&data[..]);
        assert_eq!(
            first_line_since(&mut r, data.len() as u64, 3).unwrap(),
            boundary
        );
        // Everything is newer → start of file; everything older → the end.
        assert_eq!(first_line_since(&mut r, data.len() as u64, 0).unwrap(), 0);
        assert_eq!(
            first_line_since(&mut r, data.len() as u64, 100).unwrap(),
            data.len() as u64
        );
        assert_eq!(first_line_since(&mut r, 0, 100).unwrap(), 0);
    }

    #[test]
    fn a_partial_last_line_waits_for_its_newline() {
        let mut data = line(10, 4).into_bytes();
        let first = data.len() as u64;
        data.extend(br#"{"display":"half"#);
        let (seen, next, _) = lines_in(&data, None, None);
        assert_eq!((seen, next), (vec![10], first));
        data.extend(b"\",\"timestamp\":11}\n");
        let (seen, next, _) = lines_in(&data, Some(next), None);
        assert_eq!((seen, next), (vec![11], data.len() as u64));
    }

    #[test]
    fn line_timestamps_are_top_level_only() {
        assert_eq!(line_timestamp(br#"{"timestamp":42}"#), Some(42));
        assert_eq!(
            line_timestamp(br#"{"timestamp":"1970-01-01T00:00:01.000Z"}"#),
            Some(1000)
        );
        assert_eq!(line_timestamp(br#"{"snapshot":{"timestamp":5}}"#), None);
        assert_eq!(line_timestamp(b"not json"), None);
    }

    #[test]
    fn a_replaced_or_truncated_file_restarts() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("h.jsonl");
        std::fs::write(&p, line(1, 1) + &line(2, 1)).unwrap();
        let mut tail = JsonlTail::default();
        let mut n = 0;
        assert!(tail.read(&p, None, |_| n += 1).unwrap().restarted);
        assert_eq!(n, 2);
        let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
        f.write_all(line(3, 1).as_bytes()).unwrap();
        let out = tail.read(&p, None, |_| n += 1).unwrap();
        assert!(!out.restarted);
        assert_eq!(n, 3);
        // Rewritten shorter in place → from the start again.
        std::fs::write(&p, line(9, 1)).unwrap();
        assert!(tail.read(&p, None, |_| n += 1).unwrap().restarted);
        assert_eq!(n, 4);
        // Replaced by rename (new inode) → restart even though it is longer.
        let tmp = dir.path().join("t");
        std::fs::write(&tmp, line(1, 1) + &line(2, 1) + &line(3, 1)).unwrap();
        std::fs::rename(&tmp, &p).unwrap();
        assert!(tail.read(&p, None, |_| n += 1).unwrap().restarted);
        assert_eq!(n, 7);
    }
}
