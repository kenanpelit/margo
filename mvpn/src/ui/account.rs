//! The panel's Account section: this device and its paid-until date, the
//! other devices with a two-click Revoke, a voucher field, a two-click Log
//! out and the account page — or, when logged out, a masked account-number
//! field and Log in. What a click does (and the two-click confirmation) is
//! `mvpn_proto::account`, shared with the shell VPN menu; this only draws
//! its state. The number and the voucher are cleared from their fields as
//! soon as they are handed over.

use std::rc::Rc;

use gtk4::prelude::*;

use mvpn_proto::account::{AccountAction, AccountState, paid_line};
use mvpn_proto::health::{Account, Device};

/// Clear and refill `b` for `state`.
pub fn rebuild(b: &gtk4::Box, state: &AccountState, on: Rc<dyn Fn(AccountAction)>) {
    super::clear_box(b);
    match &state.account {
        None => b.append(&super::dim("Loading account…")),
        Some(Account::Unknown) => b.append(&super::dim(
            "The account can't be read (is the Mullvad service running?).",
        )),
        Some(Account::LoggedOut | Account::Revoked) => logged_out(b, state, &on),
        Some(Account::LoggedIn {
            device,
            expiry,
            expired,
        }) => logged_in(b, state, device, expiry, *expired, &on),
    }
    if let Some((ok, text)) = &state.notice {
        let l = super::dim(text);
        l.add_css_class(if *ok { "mvpn-ok" } else { "mvpn-error" });
        b.append(&l);
    }
    b.set_sensitive(!state.busy);
}

fn logged_out(b: &gtk4::Box, state: &AccountState, on: &Rc<dyn Fn(AccountAction)>) {
    b.append(&super::dim(
        if matches!(state.account, Some(Account::Revoked)) {
            "This device was removed from the account. Log in again."
        } else {
            "Not logged in."
        },
    ));
    let entry = gtk4::PasswordEntry::new();
    entry.set_show_peek_icon(true);
    entry.set_placeholder_text(Some("Account number (empty: the saved one)"));
    entry.set_hexpand(true);
    entry.update_property(&[gtk4::accessible::Property::Label("Mullvad account number")]);
    let login = action_button("Log in");
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
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    row.append(&entry);
    row.append(&login);
    b.append(&row);
    b.append(&page_button(on));
}

fn logged_in(
    b: &gtk4::Box,
    state: &AccountState,
    device: &str,
    expiry: &str,
    expired: bool,
    on: &Rc<dyn Fn(AccountAction)>,
) {
    let info = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    info.add_css_class("mvpn-card");
    let name = gtk4::Label::new(Some(&format!("This device: {device}")));
    name.set_halign(gtk4::Align::Start);
    let paid = super::dim(&paid_line(expiry, expired));
    if expired {
        paid.add_css_class("mvpn-error");
    }
    info.append(&name);
    info.append(&paid);
    b.append(&info);

    // Other devices, each with a two-step Revoke.
    match &state.devices {
        None => b.append(&super::dim("Loading devices…")),
        Some(Err(e)) => b.append(&super::dim(e)),
        Some(Ok(list)) => {
            let others: Vec<&Device> = list.iter().filter(|d| !d.current).collect();
            if others.is_empty() {
                b.append(&super::dim("No other devices on this account."));
            }
            for d in others {
                let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 10);
                row.add_css_class("mvpn-card");
                let l = gtk4::Label::new(Some(&d.name));
                l.set_halign(gtk4::Align::Start);
                l.set_hexpand(true);
                l.set_ellipsize(gtk4::pango::EllipsizeMode::End);
                let armed = state.confirm_revoke.as_deref() == Some(d.name.as_str());
                let revoke = confirm_button(if armed { "Confirm" } else { "Revoke" }, armed);
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
    let voucher = gtk4::Entry::new();
    voucher.set_placeholder_text(Some("Voucher code"));
    voucher.set_hexpand(true);
    voucher.update_property(&[gtk4::accessible::Property::Label("Mullvad voucher code")]);
    let redeem = action_button("Redeem");
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
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    row.append(&voucher);
    row.append(&redeem);
    b.append(&row);

    // Footer: account page + a two-step Log out.
    let foot = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    foot.set_homogeneous(true);
    foot.append(&page_button(on));
    let logout = confirm_button(
        if state.confirm_logout {
            "Confirm log out"
        } else {
            "Log out"
        },
        state.confirm_logout,
    );
    let on2 = on.clone();
    logout.connect_clicked(move |_| on2(AccountAction::Logout));
    foot.append(&logout);
    b.append(&foot);
}

fn action_button(label: &str) -> gtk4::Button {
    let b = gtk4::Button::with_label(label);
    b.add_css_class("mvpn-action");
    b
}

/// A destructive button; armed (waiting for the second click) it turns red.
fn confirm_button(label: &str, armed: bool) -> gtk4::Button {
    let b = action_button(label);
    if armed {
        b.add_css_class("mvpn-danger");
    }
    b
}

fn page_button(on: &Rc<dyn Fn(AccountAction)>) -> gtk4::Button {
    let b = action_button("Account page");
    b.set_tooltip_text(Some("Open your account on mullvad.net"));
    let on = on.clone();
    b.connect_clicked(move |_| on(AccountAction::OpenPage));
    b
}
