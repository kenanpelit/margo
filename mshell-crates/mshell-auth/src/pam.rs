//! Minimal libpam FFI for password authentication.
//!
//! Ported verbatim from `mlock/src/auth.rs` so mshell-lockscreen
//! can share the same auth backend without us maintaining two
//! parallel copies. We avoid the upstream `pam` crate (0.8) +
//! `pam-sys 1.0.0-alpha5` chain because that builds through
//! `bindgen` → `clang-sys`, and clang-sys's thread-local libclang
//! autoload trips on Arch even with `LIBCLANG_PATH` set. A bare
//! `#[link(name = "pam")]` of three functions sidesteps the
//! whole problem.
//!
//! The PAM service id defaults to `"login"` to match `mlock`.
//! Most distros configure that service to accept the unlocked
//! user's password — same behaviour as the system console
//! login.

use std::ffi::{CString, c_char, c_int, c_void};

/// PAM service id we authenticate against. `"login"` is the
/// standard console-login service on most distros and is what
/// `mlock` already uses; keeping the two in sync means a working
/// `mlock` lockscreen also means a working mshell-lockscreen.
pub const SERVICE_LOGIN: &str = "login";

#[link(name = "pam")]
unsafe extern "C" {
    fn pam_start(
        service: *const c_char,
        user: *const c_char,
        conv: *const PamConv,
        pamh: *mut *mut c_void,
    ) -> c_int;
    fn pam_authenticate(pamh: *mut c_void, flags: c_int) -> c_int;
    fn pam_end(pamh: *mut c_void, status: c_int) -> c_int;
}

const PAM_SUCCESS: c_int = 0;
const PAM_PROMPT_ECHO_OFF: c_int = 1;

#[repr(C)]
struct PamMessage {
    msg_style: c_int,
    msg: *const c_char,
}

#[repr(C)]
struct PamResponse {
    resp: *mut c_char,
    resp_retcode: c_int,
}

#[repr(C)]
struct PamConv {
    conv: extern "C" fn(
        num_msg: c_int,
        msg: *const *const PamMessage,
        resp: *mut *mut PamResponse,
        appdata: *mut c_void,
    ) -> c_int,
    appdata: *mut c_void,
}

extern "C" fn conv_callback(
    num_msg: c_int,
    msg: *const *const PamMessage,
    resp: *mut *mut PamResponse,
    appdata: *mut c_void,
) -> c_int {
    if num_msg <= 0 || msg.is_null() || resp.is_null() {
        return -1;
    }
    let size = std::mem::size_of::<PamResponse>();
    let responses = unsafe { libc::calloc(num_msg as usize, size) as *mut PamResponse };
    if responses.is_null() {
        return -1;
    }
    let password = appdata as *const c_char;
    for i in 0..num_msg as isize {
        let m = unsafe { *msg.offset(i) };
        if m.is_null() {
            continue;
        }
        let style = unsafe { (*m).msg_style };
        let slot = unsafe { &mut *responses.offset(i) };
        if style == PAM_PROMPT_ECHO_OFF && !password.is_null() {
            slot.resp = unsafe { libc::strdup(password) };
            slot.resp_retcode = 0;
        } else {
            slot.resp = std::ptr::null_mut();
            slot.resp_retcode = 0;
        }
    }
    unsafe { *resp = responses };
    PAM_SUCCESS
}

#[derive(Debug)]
pub enum AuthError {
    BadInput,
    Failed(c_int),
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthError::BadInput => write!(f, "bad input"),
            AuthError::Failed(c) => write!(f, "PAM failed (code {c})"),
        }
    }
}

impl std::error::Error for AuthError {}

/// Authenticate `user` against PAM service `SERVICE_LOGIN` with
/// the given `password`. Blocking — call from a non-UI thread
/// (e.g. inside `tokio::task::spawn_blocking`).
pub fn authenticate(user: &str, password: &str) -> Result<(), AuthError> {
    let service_c = CString::new(SERVICE_LOGIN).map_err(|_| AuthError::BadInput)?;
    let user_c = CString::new(user).map_err(|_| AuthError::BadInput)?;
    let password_c = CString::new(password).map_err(|_| AuthError::BadInput)?;

    let conv = PamConv {
        conv: conv_callback,
        appdata: password_c.as_ptr() as *mut c_void,
    };

    let mut pamh: *mut c_void = std::ptr::null_mut();
    let rc = unsafe { pam_start(service_c.as_ptr(), user_c.as_ptr(), &conv, &mut pamh) };
    if rc != PAM_SUCCESS {
        return Err(AuthError::Failed(rc));
    }

    let auth_rc = unsafe { pam_authenticate(pamh, 0) };
    unsafe { pam_end(pamh, auth_rc) };

    // Scrub our plaintext copy from the heap before it's freed. PAM has
    // finished with `appdata` by now (pam_end ran); consume the CString into
    // its backing bytes and zeroize them. (The strdup'd copy PAM makes in the
    // conversation callback is owned + freed by PAM and can't be scrubbed
    // here without risking a use-after-free.)
    {
        use zeroize::Zeroize;
        let mut bytes = password_c.into_bytes_with_nul();
        bytes.zeroize();
    }

    if auth_rc == PAM_SUCCESS {
        Ok(())
    } else {
        Err(AuthError::Failed(auth_rc))
    }
}

/// Helper: current user's login name from `getpwuid(getuid())`.
/// Returns `None` when the passwd lookup fails (very unusual on
/// a sane Linux system).
pub fn current_user() -> Option<String> {
    unsafe {
        let uid = libc::getuid();
        let pw = libc::getpwuid(uid);
        if pw.is_null() {
            return None;
        }
        let name_ptr = (*pw).pw_name;
        if name_ptr.is_null() {
            return None;
        }
        std::ffi::CStr::from_ptr(name_ptr)
            .to_str()
            .ok()
            .map(String::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_error_display_is_stable() {
        assert_eq!(AuthError::BadInput.to_string(), "bad input");
        assert_eq!(AuthError::Failed(7).to_string(), "PAM failed (code 7)");
        // The raw PAM return code is surfaced so logs are diagnosable.
        assert_eq!(AuthError::Failed(-4).to_string(), "PAM failed (code -4)");
    }

    #[test]
    fn interior_nul_in_password_is_rejected_before_pam() {
        // `CString::new` fails on an embedded NUL, so this returns `BadInput`
        // *before* any FFI/PAM call — a pure input-validation path. It also
        // guards against a NUL smuggling a truncated password past PAM.
        let err = authenticate("someuser", "pass\0word").unwrap_err();
        assert!(matches!(err, AuthError::BadInput), "got {err:?}");
    }

    #[test]
    fn interior_nul_in_username_is_rejected_before_pam() {
        let err = authenticate("us\0er", "pw").unwrap_err();
        assert!(matches!(err, AuthError::BadInput), "got {err:?}");
    }

    // The PAM conversation callback is invoked by libpam, driven by whatever
    // PAM modules the service stacks. A broken or hostile module can hand it a
    // non-positive message count or null arrays; the guard at the top must bail
    // with -1 rather than `calloc(0)` / index into a null pointer. These call
    // the callback directly with the malformed shapes — none reach a deref
    // because the guard short-circuits first.

    #[test]
    fn conv_callback_rejects_non_positive_message_count() {
        let rc = conv_callback(
            0,
            std::ptr::null(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
        assert_eq!(rc, -1, "num_msg == 0 must be rejected");

        let rc = conv_callback(
            -3,
            std::ptr::null(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
        assert_eq!(rc, -1, "negative num_msg must be rejected");
    }

    #[test]
    fn conv_callback_rejects_null_message_array() {
        let rc = conv_callback(
            1,
            std::ptr::null(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
        assert_eq!(rc, -1, "null msg pointer must be rejected");
    }

    #[test]
    fn conv_callback_rejects_null_response_slot() {
        // Non-null msg array (never dereferenced — the null `resp` check
        // short-circuits first), null `resp` out-pointer → -1.
        let msg_arr: [*const PamMessage; 1] = [std::ptr::null()];
        let rc = conv_callback(
            1,
            msg_arr.as_ptr(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
        assert_eq!(rc, -1, "null resp out-pointer must be rejected");
    }

    // PAM message styles beyond the echo-off password prompt (Linux-PAM
    // `_pam_types.h`).
    const PAM_PROMPT_ECHO_ON: c_int = 2;
    const PAM_ERROR_MSG: c_int = 3;
    const PAM_TEXT_INFO: c_int = 4;

    /// Run the conversation callback over `styles` (`None` = a null message
    /// pointer in the array) with `password` as appdata, returning the rc and
    /// each slot's `(response, retcode)`. Frees what the callback allocated.
    fn converse(
        styles: &[Option<c_int>],
        password: Option<&str>,
    ) -> (c_int, Vec<(Option<String>, c_int)>) {
        let text = CString::new("Password: ").unwrap();
        let messages: Vec<PamMessage> = styles
            .iter()
            .map(|s| PamMessage {
                msg_style: s.unwrap_or(0),
                msg: text.as_ptr(),
            })
            .collect();
        let ptrs: Vec<*const PamMessage> = styles
            .iter()
            .zip(&messages)
            .map(|(s, m)| {
                if s.is_some() {
                    m as *const _
                } else {
                    std::ptr::null()
                }
            })
            .collect();
        let password = password.map(|p| CString::new(p).unwrap());
        let appdata = password
            .as_ref()
            .map_or(std::ptr::null_mut(), |p| p.as_ptr() as *mut c_void);
        let mut resp: *mut PamResponse = std::ptr::null_mut();
        let rc = conv_callback(styles.len() as c_int, ptrs.as_ptr(), &mut resp, appdata);
        let mut out = Vec::new();
        if !resp.is_null() {
            for i in 0..styles.len() {
                // SAFETY: the callback calloc'd `styles.len()` slots.
                let slot = unsafe { &*resp.add(i) };
                let text = (!slot.resp.is_null()).then(|| {
                    // SAFETY: a non-null resp is a strdup'd C string.
                    let s = unsafe { std::ffi::CStr::from_ptr(slot.resp) }
                        .to_string_lossy()
                        .into_owned();
                    unsafe { libc::free(slot.resp.cast()) };
                    s
                });
                out.push((text, slot.resp_retcode));
            }
            unsafe { libc::free(resp.cast()) };
        }
        (rc, out)
    }

    #[test]
    fn conv_answers_the_password_prompt_with_the_password() {
        let (rc, out) = converse(&[Some(PAM_PROMPT_ECHO_OFF)], Some("hunter2"));
        assert_eq!(rc, PAM_SUCCESS);
        assert_eq!(out, vec![(Some("hunter2".to_string()), 0)]);
    }

    #[test]
    fn conv_leaves_non_password_messages_unanswered() {
        // A username prompt, an error and an info line (e.g. pam_faillock's
        // "account locked" text) get an empty response, not the password.
        let (rc, out) = converse(
            &[
                Some(PAM_TEXT_INFO),
                Some(PAM_PROMPT_ECHO_ON),
                Some(PAM_ERROR_MSG),
                Some(PAM_PROMPT_ECHO_OFF),
            ],
            Some("pw"),
        );
        assert_eq!(rc, PAM_SUCCESS);
        assert_eq!(
            out,
            vec![(None, 0), (None, 0), (None, 0), (Some("pw".to_string()), 0)]
        );
    }

    #[test]
    fn conv_skips_null_messages_and_tolerates_no_password() {
        let (rc, out) = converse(&[None, Some(PAM_PROMPT_ECHO_OFF)], Some("pw"));
        assert_eq!(rc, PAM_SUCCESS);
        assert_eq!(out, vec![(None, 0), (Some("pw".to_string()), 0)]);

        let (rc, out) = converse(&[Some(PAM_PROMPT_ECHO_OFF)], None);
        assert_eq!(rc, PAM_SUCCESS);
        assert_eq!(out, vec![(None, 0)]);
    }

    #[test]
    fn conv_answers_repeated_password_prompts() {
        // Stacked modules (e.g. pam_unix then pam_gnome_keyring) may each ask.
        let (_, out) = converse(
            &[Some(PAM_PROMPT_ECHO_OFF), Some(PAM_PROMPT_ECHO_OFF)],
            Some("pw"),
        );
        assert_eq!(
            out,
            vec![(Some("pw".to_string()), 0), (Some("pw".to_string()), 0)]
        );
    }

    #[test]
    fn current_user_is_a_plain_name() {
        if let Some(name) = current_user() {
            assert!(!name.is_empty());
            assert!(!name.contains(['\0', '/', '\n']));
        }
    }
}
