//! Minimal libpam FFI — same shape as mshell's lockscreen::pam.
//!
//! Kept inline (rather than shared via a crate) because PAM bindings
//! are tiny and the duplication is cheaper than spinning up a fourth
//! workspace member just for two functions. If a third locker ever
//! appears we'll factor this out.

use std::ffi::{CString, c_char, c_int, c_void};

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
