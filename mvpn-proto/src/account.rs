//! The Account section's logic, shared by the shell VPN menu and the
//! `mvpn menu` panel: what a click runs (`mvpn account …` argv + stdin),
//! the two-click confirmation for Log out / Revoke, and the sentence shown
//! afterwards. GTK-free; each surface only draws [`AccountState`].
//!
//! The account number and the voucher go to `mvpn` on stdin, never in an
//! argv, and the number is never shown (Mullvad's own CLI prints it; mvpn
//! drops it).

use crate::health::{Account, Device};

/// What a click in the section asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountAction {
    /// Log in with the typed number (empty = the saved one: env / pass).
    Login(String),
    /// First click on Log out arms it; the second logs out.
    Logout,
    Redeem(String),
    /// First click arms the device's Revoke; the second revokes it.
    Revoke(String),
    OpenPage,
}

/// The section's state, as a surface holds it.
#[derive(Debug, Default)]
pub struct AccountState {
    /// `None` until `mvpn account show` answers.
    pub account: Option<Account>,
    /// `None` until listed; `Err` = the sentence why not.
    pub devices: Option<Result<Vec<Device>, String>>,
    /// The last action's outcome: (worked, sentence).
    pub notice: Option<(bool, String)>,
    pub confirm_logout: bool,
    pub confirm_revoke: Option<String>,
    /// An action is running (buttons insensitive).
    pub busy: bool,
}

/// What a surface does after [`AccountState::click`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Click {
    /// Run `mvpn <args>`, writing `stdin` (a secret) to its stdin, then
    /// report with [`AccountState::finish`].
    Run {
        args: Vec<String>,
        stdin: Option<String>,
    },
    /// A destructive action is armed: redraw, and call
    /// [`AccountState::disarm`] after a few seconds.
    Armed,
    /// Open [`crate::health::ACCOUNT_URL`].
    OpenPage,
    /// Nothing to do.
    None,
}

impl AccountState {
    /// Apply a click: run it (disarming any confirmation and marking the
    /// section busy), arm it, or open the account page.
    pub fn click(&mut self, action: AccountAction) -> Click {
        if action == AccountAction::OpenPage {
            return Click::OpenPage;
        }
        if let Some((args, stdin)) = command(&action, self) {
            self.confirm_logout = false;
            self.confirm_revoke = None;
            self.busy = true;
            self.notice = None;
            return Click::Run { args, stdin };
        }
        match action {
            AccountAction::Logout => self.confirm_logout = true,
            AccountAction::Revoke(name) => self.confirm_revoke = Some(name),
            _ => return Click::None,
        }
        Click::Armed
    }

    /// Drop an armed confirmation; `true` when one was armed (redraw).
    pub fn disarm(&mut self) -> bool {
        let armed = self.confirm_logout || self.confirm_revoke.is_some();
        self.confirm_logout = false;
        self.confirm_revoke = None;
        armed
    }

    /// A [`Click::Run`] finished.
    pub fn finish(&mut self, ok: bool, stdout: &str, stderr: &str) {
        self.busy = false;
        self.notice = Some((ok, outcome(ok, stdout, stderr)));
    }
}

/// The `mvpn` argv and stdin an action runs (`None` for the clicks that
/// only arm a confirmation, or open the page).
pub fn command(
    action: &AccountAction,
    state: &AccountState,
) -> Option<(Vec<String>, Option<String>)> {
    let args = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    match action {
        AccountAction::Login(number) if number.trim().is_empty() => {
            Some((args(&["account", "login"]), None))
        }
        AccountAction::Login(number) => Some((
            args(&["account", "login", "--stdin"]),
            Some(format!("{}\n", number.trim())),
        )),
        AccountAction::Logout => state
            .confirm_logout
            .then(|| (args(&["account", "logout"]), None)),
        AccountAction::Redeem(code) => Some((
            args(&["account", "redeem"]),
            Some(format!("{}\n", code.trim())),
        )),
        AccountAction::Revoke(name) => (state.confirm_revoke.as_deref() == Some(name))
            .then(|| (vec!["account".into(), "revoke".into(), name.clone()], None)),
        AccountAction::OpenPage => None,
    }
}

/// The line shown after an action: mvpn's own first output line, minus
/// its `mvpn account <verb>: ` prefix (mvpn prints plain sentences and
/// never the account number).
pub fn outcome(ok: bool, stdout: &str, stderr: &str) -> String {
    let text = if ok { stdout } else { stderr };
    let line = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let line = match line.split_once(": ") {
        Some((head, rest)) if head.starts_with("mvpn ") => rest,
        _ => line,
    };
    match (line.trim(), ok) {
        ("", true) => "Done.".into(),
        ("", false) => "That didn't work.".into(),
        (l, _) => {
            let mut c = l.chars();
            c.next()
                .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                .unwrap_or_default()
        }
    }
}

/// The paid-until line for a logged-in account.
pub fn paid_line(expiry: &str, expired: bool) -> String {
    if expiry.is_empty() {
        "Paid-until date unknown".to_string()
    } else if expired {
        format!("Expired on {expiry}")
    } else {
        format!("Paid until {expiry}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn secrets_travel_on_stdin_never_in_argv() {
        let st = AccountState::default();
        let (args, stdin) =
            command(&AccountAction::Login(" 1234123412341234 ".into()), &st).expect("runs");
        assert_eq!(args, v(&["account", "login", "--stdin"]));
        assert!(!args.concat().contains("1234"));
        assert_eq!(stdin.as_deref(), Some("1234123412341234\n"));
        // Empty field: the saved number (env / pass), nothing piped.
        assert_eq!(
            command(&AccountAction::Login(String::new()), &st),
            Some((v(&["account", "login"]), None))
        );
        let (args, stdin) =
            command(&AccountAction::Redeem("ABCD-EFGH-1234-5678".into()), &st).expect("runs");
        assert_eq!(args, v(&["account", "redeem"]));
        assert_eq!(stdin.as_deref(), Some("ABCD-EFGH-1234-5678\n"));
    }

    #[test]
    fn outcome_lines_are_mvpns_sentences() {
        assert_eq!(
            outcome(true, "voucher redeemed · paid until 2027-03-01\n", ""),
            "Voucher redeemed · paid until 2027-03-01"
        );
        assert_eq!(
            outcome(
                false,
                "",
                "mvpn account redeem: That voucher code is not valid.\n"
            ),
            "That voucher code is not valid."
        );
        assert_eq!(outcome(false, "", ""), "That didn't work.");
        assert_eq!(outcome(true, "", ""), "Done.");
    }

    #[test]
    fn destructive_actions_need_a_second_click() {
        let mut st = AccountState::default();
        assert_eq!(command(&AccountAction::Logout, &st), None);
        assert_eq!(
            command(&AccountAction::Revoke("Dreamy Hen".into()), &st),
            None
        );
        st.confirm_logout = true;
        assert_eq!(
            command(&AccountAction::Logout, &st),
            Some((v(&["account", "logout"]), None))
        );
        st.confirm_revoke = Some("Dreamy Hen".into());
        assert_eq!(
            command(&AccountAction::Revoke("Dreamy Hen".into()), &st),
            Some((v(&["account", "revoke", "Dreamy Hen"]), None))
        );
        // Armed for one device, a click on another only arms that one.
        assert_eq!(
            command(&AccountAction::Revoke("Live Coral".into()), &st),
            None
        );
        assert_eq!(command(&AccountAction::OpenPage, &st), None);
    }

    #[test]
    fn clicks_arm_then_run_then_report() {
        let mut st = AccountState::default();
        assert_eq!(st.click(AccountAction::Logout), Click::Armed);
        assert!(st.confirm_logout && !st.busy);
        assert_eq!(
            st.click(AccountAction::Logout),
            Click::Run {
                args: v(&["account", "logout"]),
                stdin: None
            }
        );
        assert!(!st.confirm_logout && st.busy);
        st.finish(false, "", "mvpn account logout: You are not logged in.\n");
        assert!(!st.busy);
        assert_eq!(
            st.notice,
            Some((false, "You are not logged in.".to_string()))
        );

        // Arming a Revoke, then letting it time out.
        assert_eq!(
            st.click(AccountAction::Revoke("Live Coral".into())),
            Click::Armed
        );
        assert!(st.disarm());
        assert!(!st.disarm());
        assert_eq!(st.click(AccountAction::OpenPage), Click::OpenPage);
        // A run clears an armed confirmation elsewhere and the old notice.
        st.click(AccountAction::Logout);
        assert!(matches!(
            st.click(AccountAction::Redeem("ABCD".into())),
            Click::Run { .. }
        ));
        assert!(!st.confirm_logout && st.notice.is_none());
    }

    #[test]
    fn paid_line_says_expired() {
        assert_eq!(paid_line("", false), "Paid-until date unknown");
        assert_eq!(paid_line("2027-02-06", false), "Paid until 2027-02-06");
        assert_eq!(paid_line("2026-01-01", true), "Expired on 2026-01-01");
    }
}
