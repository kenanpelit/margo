//! The wire protocol between mlogind's privileged session runner and whichever
//! greeter is showing — `mgreet` (GTK) or the TUI form inside `mlogind`.
//!
//! The runner owns the PAM conversation. It never receives a password out of
//! the blue; it receives an answer to a question it asked. That inversion is
//! what makes fingerprint readers, U2F, OTP and "your password has expired"
//! work at all, and it is why the plaintext no longer needs a file to travel
//! through.
//!
//! Frames are `u8 tag | u32 len | payload` and ride a `SOCK_SEQPACKET` pair, so
//! the kernel already guarantees one `recv` returns exactly one frame. The
//! length prefix is therefore redundant on the wire — it is kept because it
//! makes the in-memory [`Transport`] used by the tests behave identically, and
//! because a frame that disagrees with its own header is a bug worth catching
//! at the boundary rather than three fields later.

use std::io;

use zeroize::{Zeroize, Zeroizing};

mod transport;

pub use transport::{FdTransport, MemTransport, Transport};

/// Largest frame we will send or accept, header included.
pub const MAX_FRAME: usize = 64 * 1024;

/// Header is a one-byte tag plus a big-endian `u32` payload length.
const HEADER: usize = 5;

/// Largest payload that still fits inside [`MAX_FRAME`].
pub const MAX_PAYLOAD: usize = MAX_FRAME - HEADER;

// Request tags (greeter → runner).
const REQ_BEGIN: u8 = 1;
const REQ_RESPONSE: u8 = 2;
const REQ_CANCEL: u8 = 3;

// Event tags (runner → greeter).
const EV_PROMPT: u8 = 1;
const EV_INFO: u8 = 2;
const EV_ERROR: u8 = 3;
const EV_SUCCESS: u8 = 4;
const EV_FAILURE: u8 = 5;

/// Everything that can go wrong at the protocol boundary.
///
/// None of these ever panic: this is the login gate, and a greeter that aborts
/// leaves the user staring at a black screen.
#[derive(Debug)]
pub enum ProtoError {
    /// The peer closed its end.
    Eof,
    /// A frame ended in the middle of a field, or claimed a length its own
    /// payload cannot cover.
    Truncated,
    /// A frame — or a field inside one — exceeded [`MAX_FRAME`].
    TooLarge(usize),
    /// A tag we do not know. A newer greeter talking to an older runner, or noise.
    UnknownTag(u8),
    /// A string field that was not valid UTF-8.
    NotUtf8,
    Io(io::Error),
}

impl std::fmt::Display for ProtoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Eof => f.write_str("peer closed the connection"),
            Self::Truncated => f.write_str("truncated frame"),
            Self::TooLarge(n) => write!(f, "frame of {n} bytes exceeds the {MAX_FRAME}-byte limit"),
            Self::UnknownTag(t) => write!(f, "unknown frame tag {t}"),
            Self::NotUtf8 => f.write_str("string field is not valid UTF-8"),
            Self::Io(e) => write!(f, "io error: {e}"),
        }
    }
}

impl std::error::Error for ProtoError {}

impl From<io::Error> for ProtoError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// Greeter → runner.
#[derive(Debug, PartialEq, Eq)]
pub enum Request {
    /// Start (or restart) a PAM conversation for `user`, and remember which
    /// session to launch once it succeeds.
    Begin { user: String, session: String },
    /// The answer to the runner's most recent [`Event::Prompt`].
    ///
    /// Bytes, not a `String`: a PAM response is opaque, need not be UTF-8, and
    /// `Zeroizing<Vec<u8>>` can actually scrub it — a `String` would leave a
    /// copy behind at every `from_utf8`.
    Response { secret: Zeroizing<Vec<u8>> },
    /// Abandon the conversation in flight; the greeter is going back to a blank form.
    Cancel,
}

/// Runner → greeter.
#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    /// PAM wants an answer. `echo` mirrors `PAM_PROMPT_ECHO_ON`, so the greeter
    /// knows whether to show what is typed.
    Prompt { echo: bool, text: String },
    /// `PAM_TEXT_INFO`.
    Info { text: String },
    /// `PAM_ERROR_MSG`.
    Error { text: String },
    /// Authentication and account validation both passed. The greeter should quit.
    Success,
    /// This attempt failed. The connection stays open; the greeter may send a
    /// fresh [`Request::Begin`].
    Failure { reason: String },
}

// ── encoding ───────────────────────────────────────────────────────────────

fn put_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(bytes);
}

fn finish(tag: u8, body: Vec<u8>) -> Result<Zeroizing<Vec<u8>>, ProtoError> {
    if body.len() > MAX_PAYLOAD {
        return Err(ProtoError::TooLarge(body.len() + HEADER));
    }
    let mut frame = Zeroizing::new(Vec::with_capacity(HEADER + body.len()));
    frame.push(tag);
    frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
    frame.extend_from_slice(&body);
    // `body` may have held a secret. Scrub the intermediate.
    let mut body = body;
    body.zeroize();
    Ok(frame)
}

/// Serialise a [`Request`]. The frame is `Zeroizing` because `Response` frames
/// carry the plaintext.
pub fn encode_request(req: &Request) -> Result<Zeroizing<Vec<u8>>, ProtoError> {
    let (tag, body) = match req {
        Request::Begin { user, session } => {
            let mut b = Vec::new();
            put_bytes(&mut b, user.as_bytes());
            put_bytes(&mut b, session.as_bytes());
            (REQ_BEGIN, b)
        }
        Request::Response { secret } => {
            let mut b = Vec::with_capacity(4 + secret.len());
            put_bytes(&mut b, secret);
            (REQ_RESPONSE, b)
        }
        Request::Cancel => (REQ_CANCEL, Vec::new()),
    };
    finish(tag, body)
}

/// Serialise an [`Event`]. Events never carry secrets, but the return type
/// matches [`encode_request`] so callers need only one code path.
pub fn encode_event(ev: &Event) -> Result<Zeroizing<Vec<u8>>, ProtoError> {
    let (tag, body) = match ev {
        Event::Prompt { echo, text } => {
            let mut b = Vec::new();
            b.push(u8::from(*echo));
            put_bytes(&mut b, text.as_bytes());
            (EV_PROMPT, b)
        }
        Event::Info { text } => {
            let mut b = Vec::new();
            put_bytes(&mut b, text.as_bytes());
            (EV_INFO, b)
        }
        Event::Error { text } => {
            let mut b = Vec::new();
            put_bytes(&mut b, text.as_bytes());
            (EV_ERROR, b)
        }
        Event::Success => (EV_SUCCESS, Vec::new()),
        Event::Failure { reason } => {
            let mut b = Vec::new();
            put_bytes(&mut b, reason.as_bytes());
            (EV_FAILURE, b)
        }
    };
    finish(tag, body)
}

// ── decoding ───────────────────────────────────────────────────────────────

/// A cursor over one frame's payload. Every read is bounds-checked; a field
/// that claims more bytes than remain is [`ProtoError::Truncated`], never a panic.
struct Cursor<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], ProtoError> {
        let end = self.pos.checked_add(n).ok_or(ProtoError::Truncated)?;
        let slice = self.buf.get(self.pos..end).ok_or(ProtoError::Truncated)?;
        self.pos = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, ProtoError> {
        Ok(self.take(1)?[0])
    }

    fn bytes(&mut self) -> Result<&'a [u8], ProtoError> {
        let len = u32::from_be_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| ProtoError::Truncated)?,
        );
        let len = usize::try_from(len).map_err(|_| ProtoError::Truncated)?;
        if len > MAX_PAYLOAD {
            return Err(ProtoError::TooLarge(len));
        }
        self.take(len)
    }

    fn string(&mut self) -> Result<String, ProtoError> {
        std::str::from_utf8(self.bytes()?)
            .map(str::to_owned)
            .map_err(|_| ProtoError::NotUtf8)
    }
}

/// Split a frame into its tag and payload, validating the length header.
fn split(frame: &[u8]) -> Result<(u8, Cursor<'_>), ProtoError> {
    if frame.len() > MAX_FRAME {
        return Err(ProtoError::TooLarge(frame.len()));
    }
    let head = frame.get(..HEADER).ok_or(ProtoError::Truncated)?;
    let tag = head[0];
    let len = u32::from_be_bytes([head[1], head[2], head[3], head[4]]);
    let len = usize::try_from(len).map_err(|_| ProtoError::Truncated)?;
    let body = frame.get(HEADER..).ok_or(ProtoError::Truncated)?;
    // The header must describe exactly the bytes that arrived. A frame that
    // under-claims its length would let a trailing field hide behind the cursor.
    if body.len() != len {
        return Err(ProtoError::Truncated);
    }
    Ok((tag, Cursor { buf: body, pos: 0 }))
}

/// Parse a [`Request`] frame.
pub fn decode_request(frame: &[u8]) -> Result<Request, ProtoError> {
    let (tag, mut c) = split(frame)?;
    match tag {
        REQ_BEGIN => Ok(Request::Begin {
            user: c.string()?,
            session: c.string()?,
        }),
        REQ_RESPONSE => Ok(Request::Response {
            secret: Zeroizing::new(c.bytes()?.to_vec()),
        }),
        REQ_CANCEL => Ok(Request::Cancel),
        t => Err(ProtoError::UnknownTag(t)),
    }
}

/// Parse an [`Event`] frame.
pub fn decode_event(frame: &[u8]) -> Result<Event, ProtoError> {
    let (tag, mut c) = split(frame)?;
    match tag {
        EV_PROMPT => Ok(Event::Prompt {
            echo: c.u8()? != 0,
            text: c.string()?,
        }),
        EV_INFO => Ok(Event::Info { text: c.string()? }),
        EV_ERROR => Ok(Event::Error { text: c.string()? }),
        EV_SUCCESS => Ok(Event::Success),
        EV_FAILURE => Ok(Event::Failure {
            reason: c.string()?,
        }),
        t => Err(ProtoError::UnknownTag(t)),
    }
}

// ── connection ─────────────────────────────────────────────────────────────

/// A framed connection over any [`Transport`].
///
/// Both directions are available on one type: the runner calls
/// [`Conn::recv_request`] / [`Conn::send_event`], the greeter the mirror pair.
/// Splitting them into two types would buy nothing — the socket is one fd — and
/// would double the test surface.
pub struct Conn<T: Transport> {
    inner: T,
    scratch: Vec<u8>,
}

impl<T: Transport> Conn<T> {
    pub fn new(inner: T) -> Self {
        Self {
            inner,
            scratch: Vec::with_capacity(MAX_FRAME),
        }
    }

    pub fn get_mut(&mut self) -> &mut T {
        &mut self.inner
    }

    pub fn send_request(&mut self, req: &Request) -> Result<(), ProtoError> {
        let frame = encode_request(req)?;
        self.inner.send(&frame)
    }

    pub fn send_event(&mut self, ev: &Event) -> Result<(), ProtoError> {
        let frame = encode_event(ev)?;
        self.inner.send(&frame)
    }

    /// Read one request. `Ok(None)` means the greeter went away — a clean EOF,
    /// not an error: quitting the greeter is a legitimate way to end a session.
    pub fn recv_request(&mut self) -> Result<Option<Request>, ProtoError> {
        match self.recv_frame()? {
            None => Ok(None),
            Some(()) => {
                let out = decode_request(&self.scratch);
                // The frame just parsed may have been a `Response`, i.e. a
                // plaintext password sitting in our scratch buffer.
                self.scratch.zeroize();
                out.map(Some)
            }
        }
    }

    /// Read one event. `Ok(None)` means the runner went away.
    pub fn recv_event(&mut self) -> Result<Option<Event>, ProtoError> {
        match self.recv_frame()? {
            None => Ok(None),
            Some(()) => {
                let out = decode_event(&self.scratch);
                self.scratch.clear();
                out.map(Some)
            }
        }
    }

    /// Fill `self.scratch` with exactly one frame. `Ok(None)` on EOF.
    fn recv_frame(&mut self) -> Result<Option<()>, ProtoError> {
        self.scratch.clear();
        match self.inner.recv(&mut self.scratch)? {
            0 => Ok(None),
            _ => Ok(Some(())),
        }
    }
}

impl<T: Transport> Drop for Conn<T> {
    fn drop(&mut self) {
        self.scratch.zeroize();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn begin() -> Request {
        Request::Begin {
            user: "alice".into(),
            session: "margo".into(),
        }
    }

    #[test]
    fn every_request_round_trips() {
        for req in [
            begin(),
            Request::Response {
                secret: Zeroizing::new(b"hunter2".to_vec()),
            },
            Request::Cancel,
        ] {
            let frame = encode_request(&req).expect("encode");
            assert_eq!(decode_request(&frame).expect("decode"), req);
        }
    }

    #[test]
    fn every_event_round_trips() {
        for ev in [
            Event::Prompt {
                echo: true,
                text: "login:".into(),
            },
            Event::Prompt {
                echo: false,
                text: "Password:".into(),
            },
            Event::Info {
                text: "Touch your key".into(),
            },
            Event::Error {
                text: "no such user".into(),
            },
            Event::Success,
            Event::Failure {
                reason: "Invalid login credentials".into(),
            },
        ] {
            let frame = encode_event(&ev).expect("encode");
            assert_eq!(decode_event(&frame).expect("decode"), ev);
        }
    }

    #[test]
    fn a_secret_may_contain_newlines_and_nuls() {
        // The old file hand-off put the password last precisely so a newline
        // could not shift the parse. Length-prefixed fields make the question
        // moot — any byte string survives, in any position.
        let secret = b"line1\nline2\0\xff\x00trailing".to_vec();
        let req = Request::Response {
            secret: Zeroizing::new(secret.clone()),
        };
        let frame = encode_request(&req).expect("encode");
        match decode_request(&frame).expect("decode") {
            Request::Response { secret: got } => assert_eq!(got.as_slice(), secret.as_slice()),
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn empty_fields_round_trip() {
        let req = Request::Begin {
            user: String::new(),
            session: String::new(),
        };
        let frame = encode_request(&req).expect("encode");
        assert_eq!(decode_request(&frame).expect("decode"), req);

        let req = Request::Response {
            secret: Zeroizing::new(Vec::new()),
        };
        let frame = encode_request(&req).expect("encode");
        assert_eq!(decode_request(&frame).expect("decode"), req);
    }

    #[test]
    fn a_truncated_frame_is_an_error_not_a_panic() {
        let frame = encode_request(&begin()).expect("encode");
        for cut in 0..frame.len() {
            // Every prefix of a valid frame must be rejected, and none may panic.
            assert!(matches!(
                decode_request(&frame[..cut]),
                Err(ProtoError::Truncated)
            ));
        }
    }

    #[test]
    fn a_header_that_lies_about_its_length_is_rejected() {
        let mut frame = encode_request(&begin()).expect("encode").to_vec();
        // Claim one byte fewer than the payload actually holds: without the
        // strict `body.len() != len` check the extra byte would be ignored.
        let len = u32::from_be_bytes([frame[1], frame[2], frame[3], frame[4]]);
        frame[1..5].copy_from_slice(&(len - 1).to_be_bytes());
        assert!(matches!(decode_request(&frame), Err(ProtoError::Truncated)));
    }

    #[test]
    fn an_inner_field_may_not_exceed_the_frame_limit() {
        // A well-formed header, then a field claiming 2 GiB.
        let mut frame = vec![REQ_RESPONSE];
        frame.extend_from_slice(&4u32.to_be_bytes());
        frame.extend_from_slice(&u32::MAX.to_be_bytes());
        assert!(matches!(
            decode_request(&frame),
            Err(ProtoError::TooLarge(_))
        ));
    }

    #[test]
    fn an_oversized_payload_is_refused_at_encode_time() {
        let req = Request::Response {
            secret: Zeroizing::new(vec![0u8; MAX_PAYLOAD + 1]),
        };
        assert!(matches!(encode_request(&req), Err(ProtoError::TooLarge(_))));
    }

    #[test]
    fn a_payload_that_exactly_fills_the_frame_is_accepted() {
        let secret = vec![7u8; MAX_PAYLOAD - 4]; // minus the field's own length prefix
        let req = Request::Response {
            secret: Zeroizing::new(secret),
        };
        let frame = encode_request(&req).expect("encode");
        assert_eq!(frame.len(), MAX_FRAME);
        assert!(decode_request(&frame).is_ok());
    }

    #[test]
    fn an_unknown_tag_is_reported_not_ignored() {
        // A newer greeter sending a verb this runner has never heard of.
        let frame = [99u8, 0, 0, 0, 0];
        assert!(matches!(
            decode_request(&frame),
            Err(ProtoError::UnknownTag(99))
        ));
        assert!(matches!(
            decode_event(&frame),
            Err(ProtoError::UnknownTag(99))
        ));
    }

    #[test]
    fn a_non_utf8_string_field_is_rejected() {
        let mut body = Vec::new();
        put_bytes(&mut body, &[0xff, 0xfe]);
        put_bytes(&mut body, b"margo");
        let frame = finish(REQ_BEGIN, body).expect("encode");
        assert!(matches!(decode_request(&frame), Err(ProtoError::NotUtf8)));
    }

    #[test]
    fn request_and_event_tags_are_read_in_their_own_namespace() {
        // Tag 4 is `Success` as an event and nothing as a request. Decoding a
        // frame with the wrong function must fail loudly rather than silently
        // yielding a neighbouring variant.
        let frame = encode_event(&Event::Success).expect("encode");
        assert!(matches!(
            decode_request(&frame),
            Err(ProtoError::UnknownTag(EV_SUCCESS))
        ));
    }

    #[test]
    fn a_conn_scrubs_the_secret_from_its_scratch_buffer() {
        let secret = b"hunter2".to_vec();
        let frame = encode_request(&Request::Response {
            secret: Zeroizing::new(secret.clone()),
        })
        .expect("encode");

        let mut conn = Conn::new(MemTransport::with_incoming(vec![frame.to_vec()]));
        let got = conn.recv_request().expect("recv").expect("not eof");
        assert!(matches!(got, Request::Response { .. }));

        // The frame we just parsed held the plaintext. It must not still be there.
        assert!(
            !conn
                .scratch
                .windows(secret.len())
                .any(|w| w == secret.as_slice()),
            "plaintext survived in the scratch buffer"
        );
    }

    #[test]
    fn a_closed_transport_reads_as_eof_not_as_an_error() {
        let mut conn = Conn::new(MemTransport::with_incoming(vec![]));
        assert!(conn.recv_request().expect("eof is not an error").is_none());
        assert!(conn.recv_event().expect("eof is not an error").is_none());
    }

    #[test]
    fn frames_survive_a_full_conn_round_trip() {
        let mut out = Conn::new(MemTransport::default());
        out.send_request(&begin()).expect("send");
        out.send_request(&Request::Cancel).expect("send");
        let sent = std::mem::take(&mut out.get_mut().sent);

        let mut back = Conn::new(MemTransport::with_incoming(sent));
        assert_eq!(back.recv_request().expect("recv"), Some(begin()));
        assert_eq!(back.recv_request().expect("recv"), Some(Request::Cancel));
        assert_eq!(back.recv_request().expect("recv"), None);
    }
}
