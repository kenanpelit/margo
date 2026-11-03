//! Minimal libpam FFI.
//!
//! Kept inline (rather than shared via a crate) because PAM bindings
//! are tiny and the duplication is cheaper than spinning up a fourth
//! workspace member just for two functions. mshell's lock screen carries
//! the same code in `mshell-crates/mshell-auth/src/pam.rs` (that crate
//! pulls tokio and zbus, which mlock does not want): change the service
//! choice or the conversation rule in both. If a third locker ever
//! appears we'll factor this out.

use std::cell::Cell;
use std::ffi::{CString, c_char, c_int, c_void};
use std::path::Path;

/// mlock's own PAM service, `/etc/pam.d/mlock` (shipped as
/// `mlock/extra/mlock.pam`). Only `pam_unix`: the `login` stack mlock used
/// to borrow carries `pam_faillock`, so anyone at the locked screen could
/// type wrong passwords until the owner's account was locked out — for
/// every service, not just the locker.
pub const SERVICE_MLOCK: &str = "mlock";
/// Fallback when the mlock service file isn't installed (binary updated
/// without the package files). PAM routes an unknown service to `other`,
/// which is deny-all on most distros — the user could never unlock.
pub const SERVICE_LOGIN: &str = "login";

/// Where libpam looks for a service file (`/etc` wins, then the vendor
/// directory).
const PAM_DIRS: [&str; 2] = ["/etc/pam.d", "/usr/lib/pam.d"];

/// Pick the PAM service: mlock's own when its file exists in any PAM
/// directory, else `login` so the screen can still be unlocked.
pub(crate) fn pick_service(exists: impl Fn(&Path) -> bool) -> &'static str {
    if PAM_DIRS
        .iter()
        .any(|dir| exists(&Path::new(dir).join(SERVICE_MLOCK)))
    {
        SERVICE_MLOCK
    } else {
        SERVICE_LOGIN
    }
}

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
const PAM_CONV_ERR: c_int = 19;
const PAM_PROMPT_ECHO_OFF: c_int = 1;

/// What the conversation does with one PAM message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Reply {
    /// The first hidden prompt: answer with the typed password.
    Password,
    /// Echoed prompts and info/error text: no answer.
    Empty,
    /// A second hidden prompt (an OTP, a "new password", a module asking
    /// again): mlock has only the one secret, so fail the conversation
    /// rather than hand the password to a prompt that asked for something
    /// else.
    Fail,
}

pub(crate) fn reply_for(style: c_int, password_given: bool) -> Reply {
    match style {
        PAM_PROMPT_ECHO_OFF if password_given => Reply::Fail,
        PAM_PROMPT_ECHO_OFF => Reply::Password,
        _ => Reply::Empty,
    }
}

/// Conversation state behind `appdata`. Lives on `authenticate`'s stack
/// for the whole `pam_start`..`pam_end` span; PAM may call the
/// conversation several times, so the "already answered" flag has to
/// outlive one call.
struct ConvData {
    password: *const c_char,
    answered: Cell<bool>,
}

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
    if num_msg <= 0 || msg.is_null() || resp.is_null() || appdata.is_null() {
        return PAM_CONV_ERR;
    }
    // SAFETY: `appdata` is the `&ConvData` `authenticate` passed to
    // pam_start, alive until pam_end.
    let data = unsafe { &*(appdata as *const ConvData) };
    let size = std::mem::size_of::<PamResponse>();
    let responses = unsafe { libc::calloc(num_msg as usize, size) as *mut PamResponse };
    if responses.is_null() {
        return PAM_CONV_ERR;
    }
    for i in 0..num_msg as isize {
        let m = unsafe { *msg.offset(i) };
        if m.is_null() {
            continue;
        }
        let style = unsafe { (*m).msg_style };
        let slot = unsafe { &mut *responses.offset(i) };
        slot.resp_retcode = 0;
        match reply_for(style, data.answered.get()) {
            Reply::Password => {
                slot.resp = unsafe { libc::strdup(data.password) };
                data.answered.set(true);
            }
            Reply::Empty => slot.resp = std::ptr::null_mut(),
            Reply::Fail => {
                // On error PAM does not take the array: free it here,
                // scrubbing any password copy already made for an earlier
                // message of this same call.
                unsafe { free_responses(responses, i) };
                return PAM_CONV_ERR;
            }
        }
    }
    unsafe { *resp = responses };
    PAM_SUCCESS
}

/// Free the first `filled` responses (zeroing each answer) and the array.
///
/// # Safety
/// `responses` is a calloc'd array of at least `filled` entries whose
/// `resp` fields are null or strdup'd strings.
unsafe fn free_responses(responses: *mut PamResponse, filled: isize) {
    for j in 0..filled {
        let r = unsafe { (*responses.offset(j)).resp };
        if !r.is_null() {
            let len = unsafe { libc::strlen(r) };
            unsafe { std::ptr::write_bytes(r, 0, len) };
            unsafe { libc::free(r as *mut c_void) };
        }
    }
    unsafe { libc::free(responses as *mut c_void) };
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

pub fn authenticate(user: &str, password: &str) -> Result<(), AuthError> {
    let service = pick_service(|p| p.exists());
    let service_c = CString::new(service).map_err(|_| AuthError::BadInput)?;
    let user_c = CString::new(user).map_err(|_| AuthError::BadInput)?;
    let password_c = CString::new(password).map_err(|_| AuthError::BadInput)?;

    let data = ConvData {
        password: password_c.as_ptr(),
        answered: Cell::new(false),
    };
    let conv = PamConv {
        conv: conv_callback,
        appdata: &data as *const ConvData as *mut c_void,
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

    const PAM_PROMPT_ECHO_ON: c_int = 2;
    const PAM_TEXT_INFO: c_int = 4;

    #[test]
    fn only_the_first_hidden_prompt_gets_the_password() {
        assert_eq!(reply_for(PAM_PROMPT_ECHO_OFF, false), Reply::Password);
        // e.g. pam_oath's "One-time password:" after the password.
        assert_eq!(reply_for(PAM_PROMPT_ECHO_OFF, true), Reply::Fail);
    }

    #[test]
    fn echoed_and_info_prompts_get_no_answer() {
        assert_eq!(reply_for(PAM_PROMPT_ECHO_ON, false), Reply::Empty);
        assert_eq!(reply_for(PAM_TEXT_INFO, true), Reply::Empty);
    }

    fn call_conv(styles: &[c_int], data: &ConvData) -> (c_int, Vec<Option<String>>) {
        let texts: Vec<CString> = styles.iter().map(|_| CString::new("p").unwrap()).collect();
        let msgs: Vec<PamMessage> = styles
            .iter()
            .zip(&texts)
            .map(|(s, t)| PamMessage {
                msg_style: *s,
                msg: t.as_ptr(),
            })
            .collect();
        let ptrs: Vec<*const PamMessage> = msgs.iter().map(|m| m as *const _).collect();
        let mut resp: *mut PamResponse = std::ptr::null_mut();
        let rc = conv_callback(
            styles.len() as c_int,
            ptrs.as_ptr(),
            &mut resp,
            data as *const ConvData as *mut c_void,
        );
        let mut out = Vec::new();
        if rc == PAM_SUCCESS {
            for i in 0..styles.len() {
                let r = unsafe { (*resp.add(i)).resp };
                out.push((!r.is_null()).then(|| {
                    let s = unsafe { std::ffi::CStr::from_ptr(r) }
                        .to_string_lossy()
                        .into_owned();
                    unsafe { libc::free(r as *mut c_void) };
                    s
                }));
            }
            unsafe { libc::free(resp as *mut c_void) };
        }
        (rc, out)
    }

    #[test]
    fn a_second_hidden_prompt_fails_the_conversation() {
        let pw = CString::new("hunter2").unwrap();
        let data = ConvData {
            password: pw.as_ptr(),
            answered: Cell::new(false),
        };
        let (rc, out) = call_conv(&[PAM_TEXT_INFO, PAM_PROMPT_ECHO_OFF], &data);
        assert_eq!(rc, PAM_SUCCESS);
        assert_eq!(out, vec![None, Some("hunter2".to_string())]);
        // A later call (a second module asking for an OTP) must not get it.
        let (rc, _) = call_conv(&[PAM_PROMPT_ECHO_OFF], &data);
        assert_eq!(rc, PAM_CONV_ERR);
    }

    #[test]
    fn two_hidden_prompts_in_one_call_fail_too() {
        let pw = CString::new("hunter2").unwrap();
        let data = ConvData {
            password: pw.as_ptr(),
            answered: Cell::new(false),
        };
        let (rc, _) = call_conv(&[PAM_PROMPT_ECHO_OFF, PAM_PROMPT_ECHO_OFF], &data);
        assert_eq!(rc, PAM_CONV_ERR);
    }

    #[test]
    fn mlock_service_is_used_when_its_file_is_installed() {
        assert_eq!(
            pick_service(|p| p == Path::new("/etc/pam.d/mlock")),
            SERVICE_MLOCK
        );
        assert_eq!(
            pick_service(|p| p == Path::new("/usr/lib/pam.d/mlock")),
            SERVICE_MLOCK
        );
    }

    #[test]
    fn login_is_the_fallback_without_the_mlock_file() {
        // An unknown service would hit PAM's deny-all `other` stack.
        assert_eq!(pick_service(|_| false), SERVICE_LOGIN);
    }
}
