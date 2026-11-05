//! The VPN menu's Account section: this device and its paid-until date,
//! the other devices with Revoke, a voucher field, Log out, the account
//! page — or, when logged out, a masked account-number field and Log in.
//!
//! Reads `mvpn account show --json` / `devices --json` when the section
//! opens; every action shells out to `mvpn account …`. The account number
//! and the voucher go to `mvpn` on stdin, never in an argv, and the
//! number is never shown (Mullvad's own CLI prints it; mvpn drops it).

use mshell_utils::mvpn::health::{Account, Device};
use relm4::gtk;
use relm4::gtk::prelude::{BoxExt, ButtonExt, EditableExt, EntryExt, WidgetExt};
use std::rc::Rc;

/// What a click in the section asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AccountAction {
    /// Log in with the typed number (empty = the saved one: env / pass).
    Login(String),
    /// First click on Log out arms it; the second logs out.
    Logout,
    Redeem(String),
    /// First click arms the device's Revoke; the second revokes it.
    Revoke(String),
    OpenPage,
}

/// The section's state, as the menu holds it.
#[derive(Debug, Default)]
pub(crate) struct AccountState {
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

/// The `mvpn` argv and stdin an action runs (`None` for the clicks that
/// only arm a confirmation, or open the page).
pub(crate) fn command(
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

/// Clear and refill `b` for `state`.
pub(crate) fn rebuild(b: &gtk::Box, state: &AccountState, on: Rc<dyn Fn(AccountAction)>) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
    match &state.account {
        None => b.append(&dim("Loading account…")),
        Some(Account::Unknown) => {
            b.append(&dim(
                "The account can't be read (is the Mullvad service running?).",
            ));
        }
        Some(Account::LoggedOut | Account::Revoked) => logged_out(b, state, &on),
        Some(Account::LoggedIn {
            device,
            expiry,
            expired,
        }) => logged_in(b, state, device, expiry, *expired, &on),
    }
    if let Some((ok, text)) = &state.notice {
        let l = gtk::Label::new(Some(text));
        l.add_css_class("label-small");
        l.add_css_class(if *ok {
            "vpn-account-ok"
        } else {
            "vpn-account-error"
        });
        l.set_xalign(0.0);
        l.set_wrap(true);
        b.append(&l);
    }
    b.set_sensitive(!state.busy);
}

fn logged_out(b: &gtk::Box, state: &AccountState, on: &Rc<dyn Fn(AccountAction)>) {
    let msg = if matches!(state.account, Some(Account::Revoked)) {
        "This device was removed from the account. Log in again."
    } else {
        "Not logged in."
    };
    b.append(&dim(msg));
    let entry = gtk::PasswordEntry::new();
    entry.set_show_peek_icon(true);
    entry.set_placeholder_text(Some("Account number (empty: the saved one)"));
    entry.set_hexpand(true);
    mshell_common::a11y::name(&entry, "Mullvad account number");
    let login = gtk::Button::with_label("Log in");
    login.set_css_classes(&["ok-button-surface", "dns-action"]);
    {
        let (on, field) = (on.clone(), entry.clone());
        let go = Rc::new(move || {
            let number = field.text().to_string();
            // Don't keep the number in the widget once it is handed over.
            field.set_text("");
            on(AccountAction::Login(number));
        });
        let g = go.clone();
        entry.connect_activate(move |_| g());
        login.connect_clicked(move |_| go());
    }
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.append(&entry);
    row.append(&login);
    b.append(&row);
    b.append(&page_button(on));
}

fn logged_in(
    b: &gtk::Box,
    state: &AccountState,
    device: &str,
    expiry: &str,
    expired: bool,
    on: &Rc<dyn Fn(AccountAction)>,
) {
    let info = gtk::Box::new(gtk::Orientation::Vertical, 2);
    info.add_css_class("ok-button-surface");
    let name = gtk::Label::new(Some(&format!("This device: {device}")));
    name.set_xalign(0.0);
    let paid = gtk::Label::new(Some(&if expiry.is_empty() {
        "Paid-until date unknown".to_string()
    } else if expired {
        format!("Expired on {expiry}")
    } else {
        format!("Paid until {expiry}")
    }));
    paid.add_css_class("label-small");
    paid.add_css_class(if expired {
        "vpn-account-error"
    } else {
        "dim-label"
    });
    paid.set_xalign(0.0);
    info.append(&name);
    info.append(&paid);
    b.append(&info);

    // Other devices, each with a two-step Revoke.
    match &state.devices {
        None => b.append(&dim("Loading devices…")),
        Some(Err(e)) => b.append(&dim(e)),
        Some(Ok(list)) => {
            let others: Vec<&Device> = list.iter().filter(|d| !d.current).collect();
            if others.is_empty() {
                b.append(&dim("No other devices on this account."));
            }
            for d in others {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
                row.add_css_class("ok-button-surface");
                let l = gtk::Label::new(Some(&d.name));
                l.set_xalign(0.0);
                l.set_hexpand(true);
                l.set_ellipsize(gtk::pango::EllipsizeMode::End);
                let armed = state.confirm_revoke.as_deref() == Some(d.name.as_str());
                let revoke = gtk::Button::with_label(if armed { "Confirm" } else { "Revoke" });
                revoke.set_css_classes(&["ok-button-surface", "dns-action", "vpn-confirm-btn"]);
                if armed {
                    revoke.add_css_class("armed");
                }
                revoke.set_tooltip_text(Some(&format!("Remove {} from the account", d.name)));
                let (on, n) = (on.clone(), d.name.clone());
                revoke.connect_clicked(move |_| on(AccountAction::Revoke(n.clone())));
                row.append(&l);
                row.append(&revoke);
                b.append(&row);
            }
        }
    }

    // Voucher.
    let voucher = gtk::Entry::new();
    voucher.set_placeholder_text(Some("Voucher code"));
    voucher.set_hexpand(true);
    mshell_common::a11y::name(&voucher, "Mullvad voucher code");
    let redeem = gtk::Button::with_label("Redeem");
    redeem.set_css_classes(&["ok-button-surface", "dns-action"]);
    {
        let (on, field) = (on.clone(), voucher.clone());
        let go = Rc::new(move || {
            let code = field.text().to_string();
            if !code.trim().is_empty() {
                field.set_text("");
                on(AccountAction::Redeem(code));
            }
        });
        let g = go.clone();
        voucher.connect_activate(move |_| g());
        redeem.connect_clicked(move |_| go());
    }
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.append(&voucher);
    row.append(&redeem);
    b.append(&row);

    // Footer: account page + a two-step Log out.
    let foot = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    foot.set_homogeneous(true);
    foot.append(&page_button(on));
    let logout = gtk::Button::with_label(if state.confirm_logout {
        "Confirm log out"
    } else {
        "Log out"
    });
    logout.set_css_classes(&["ok-button-surface", "dns-action", "vpn-confirm-btn"]);
    if state.confirm_logout {
        logout.add_css_class("armed");
    }
    let on2 = on.clone();
    logout.connect_clicked(move |_| on2(AccountAction::Logout));
    foot.append(&logout);
    b.append(&foot);
}

/// The line shown after an action: mvpn's own first output line, minus
/// its `mvpn account <verb>: ` prefix (mvpn prints plain sentences and
/// never the account number).
pub(crate) fn outcome(ok: bool, stdout: &str, stderr: &str) -> String {
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

fn page_button(on: &Rc<dyn Fn(AccountAction)>) -> gtk::Button {
    let b = gtk::Button::with_label("Account page");
    b.set_css_classes(&["ok-button-surface", "dns-action"]);
    b.set_tooltip_text(Some("Open your account on mullvad.net"));
    let on = on.clone();
    b.connect_clicked(move |_| on(AccountAction::OpenPage));
    b
}

fn dim(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.add_css_class("label-small");
    l.add_css_class("dim-label");
    l.set_xalign(0.0);
    l.set_wrap(true);
    l
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
}
