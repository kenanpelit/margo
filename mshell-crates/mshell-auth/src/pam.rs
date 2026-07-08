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
}
