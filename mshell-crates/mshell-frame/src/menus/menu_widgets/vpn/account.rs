//! The VPN menu's Account section: this device and its paid-until date,
//! the other devices with Revoke, a voucher field, Log out, the account
//! page — or, when logged out, a masked account-number field and Log in.
//!
//! Reads `mvpn account show --json` / `devices --json` when the section
//! opens; every action shells out to `mvpn account …` as
//! `mvpn_proto::account` decides (shared with the `mvpn menu` panel).

use mshell_utils::mvpn::account::{AccountAction, AccountState, paid_line};
use mshell_utils::mvpn::health::{Account, Device};
use relm4::gtk;
use relm4::gtk::prelude::{BoxExt, ButtonExt, EditableExt, EntryExt, WidgetExt};
use std::rc::Rc;

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
    let paid = gtk::Label::new(Some(&paid_line(expiry, expired)));
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
