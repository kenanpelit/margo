/// `ut_line` and `ut_id` for VT `tty`: `"tty10"` and `"10"`. The old code
/// added the number to `b'0'` as one character, which only works for 0–9:
/// VTs 10–12 were recorded as `tty:`, `tty;` and `tty<`. `None` outside 1–12.
fn line_and_id(tty: u8) -> Option<(String, String)> {
    (1..=12)
        .contains(&tty)
        .then(|| (format!("tty{tty}"), tty.to_string()))
}

pub struct UtmpxSession {
    #[cfg(target_env = "gnu")]
    session: libc::utmpx,
}

#[cfg(target_env = "gnu")]
pub fn add_utmpx_entry(username: &str, tty: u8, pid: u32) -> UtmpxSession {
    log::info!("Adding UTMPX record");

    // Check the MAN page for utmp for more information
    // `man utmp`
    //
    // https://man7.org/linux/man-pages/man0/utmpx.h.0p.html
    // https://github.com/fairyglade/ly/blob/master/src/login.c
    let entry = {
        // SAFETY: None of the fields in libc::utmpx have a drop implementation.
        let mut s: libc::utmpx = unsafe { std::mem::zeroed() };

        // ut_line    --- Device name of tty - "/dev/"
        // ut_id      --- Terminal name suffix
        // ut_user    --- Username
        // ut_host    --- Hostname for remote login, or kernel version for run-level messages
        // ut_exit    --- Exit status of a process marked as DEAD_PROCESS; not used by Linux init(1)
        // ut_session --- Session ID (getsid(2)) used for windowing
        // ut_tv {    --- Time entry was made
        //     tv_sec     --- Seconds
        //     tv_usec    --- Microseconds
        // }
        // ut_addr_v6 --- Internet address of remote

        s.ut_type = libc::USER_PROCESS;
        s.ut_pid = pid as libc::pid_t;

        for (i, b) in username.as_bytes().iter().take(32).enumerate() {
            s.ut_user[i] = *b as libc::c_char;
        }

        let Some((line, id)) = line_and_id(tty) else {
            log::error!("Invalid TTY");
            std::process::exit(1);
        };
        for (slot, b) in s.ut_line.iter_mut().zip(line.bytes()) {
            *slot = b as libc::c_char;
        }
        for (slot, b) in s.ut_id.iter_mut().zip(id.bytes()) {
            *slot = b as libc::c_char;
        }

        use std::time::SystemTime;

        let epoch_duration = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_else(|_| {
                log::error!("Invalid System Time");
                std::process::exit(1);
            })
            .as_micros();

        s.ut_tv.tv_sec = (epoch_duration / 1_000_000).try_into().unwrap_or_else(|_| {
            log::error!("Invalid System Time (TV_SEC Overflow)");
            std::process::exit(1);
        });
        s.ut_tv.tv_usec = (epoch_duration % 1_000_000).try_into().unwrap_or_else(|_| {
            log::error!("Invalid System Time (TV_USEC Overflow)");
            std::process::exit(1);
        });

        s
    };

    unsafe {
        libc::setutxent();
        libc::pututxline(&entry as *const libc::utmpx);
    };

    log::info!("Added UTMPX record");

    UtmpxSession { session: entry }
}

#[cfg(not(target_env = "gnu"))]
pub fn add_utmpx_entry(_username: &str, _tty: u8, _pid: u32) -> UtmpxSession {
    log::info!("Incompatible platform for UTMPX. Skipping...");

    UtmpxSession {}
}

#[cfg(target_env = "gnu")]
impl Drop for UtmpxSession {
    fn drop(&mut self) {
        let entry = &mut self.session;

        log::info!("Removing UTMPX record");

        entry.ut_type = libc::DEAD_PROCESS;

        entry.ut_line = <[libc::c_char; 32]>::default();
        entry.ut_user = <[libc::c_char; 32]>::default();

        entry.ut_tv.tv_usec = 0;
        entry.ut_tv.tv_sec = 0;

        unsafe {
            libc::setutxent();
            libc::pututxline(entry as *const libc::utmpx);
            libc::endutxent();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::line_and_id;

    #[test]
    fn single_digit_vts() {
        assert_eq!(line_and_id(2), Some(("tty2".into(), "2".into())));
    }

    #[test]
    fn two_digit_vts_are_spelled_out_not_offset_past_nine() {
        assert_eq!(line_and_id(10), Some(("tty10".into(), "10".into())));
        assert_eq!(line_and_id(12), Some(("tty12".into(), "12".into())));
    }

    #[test]
    fn out_of_range_vts_are_refused() {
        assert_eq!(line_and_id(0), None);
        assert_eq!(line_and_id(13), None);
    }
}
