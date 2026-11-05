//! The Mullvad account: who is logged in, until when, on which devices —
//! and log out / redeem a voucher / list devices.
//!
//! `mullvad account get` prints the account number; it is parsed away
//! here and never kept, printed or logged. Logging in takes the number
//! on stdin (see `slot::login_with`), never in an argv.

pub use mvpn_proto::health::{Account, Device};

use super::{slot, sys};

/// `account get` (mullvad-cli 2026.5) → the account without its number.
///
/// ```text
/// Mullvad account:    1234123412341234
/// Expires at:         2027-02-06 09:59:30 +03:00
/// Device name:        Fast Goat
/// ```
/// Logged out prints "Not logged in on any account"; a revoked device
/// "The current device has been revoked" (then the number, dropped here).
pub fn parse_account(text: &str, now_unix: i64) -> Account {
    if text.contains("Not logged in on any account") {
        return Account::LoggedOut;
    }
    if text.contains("The current device has been revoked") {
        return Account::Revoked;
    }
    let field = |key: &str| {
        text.lines().find_map(|l| {
            l.trim()
                .strip_prefix(key)
                .map(|v| v.trim_start_matches(':').trim().to_string())
        })
    };
    if field("Mullvad account").is_none() {
        return Account::Unknown;
    }
    let raw_expiry = field("Expires at").unwrap_or_default();
    Account::LoggedIn {
        device: field("Device name").unwrap_or_default(),
        expiry: raw_expiry.chars().take(10).collect(),
        expired: expires_at(&raw_expiry).is_some_and(|at| at <= now_unix),
    }
}

/// Days since 1970-01-01 of a proleptic Gregorian date (Howard Hinnant's
/// `days_from_civil`).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// `2027-02-06 09:59:30 +03:00` (how `account get` prints the expiry: local
/// time with its offset; seconds may carry a fraction) → Unix seconds.
pub fn expires_at(text: &str) -> Option<i64> {
    let mut it = text.split_whitespace();
    let (date, time, offset) = (it.next()?, it.next()?, it.next()?);
    let mut d = date.split('-').map(|p| p.parse::<i64>().ok());
    let (y, mo, da) = (d.next()??, d.next()??, d.next()??);
    let mut t = time.split(':');
    let h: i64 = t.next()?.parse().ok()?;
    let mi: i64 = t.next()?.parse().ok()?;
    let s: i64 = t.next()?.split('.').next()?.parse().ok()?;
    let sign = match offset.chars().next()? {
        '+' => 1,
        '-' => -1,
        _ => return None,
    };
    let (oh, om) = offset[1..].split_once(':')?;
    let off = sign * (oh.parse::<i64>().ok()? * 3600 + om.parse::<i64>().ok()? * 60);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&da) {
        return None;
    }
    Some(days_from_civil(y, mo, da) * 86_400 + h * 3600 + mi * 60 + s - off)
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// The account now. `Unknown` when `mullvad` cannot answer.
pub fn query() -> Account {
    let out = sys::run("mullvad", &["account", "get"], None);
    parse_account(&out.text(), now_unix())
}

/// `account list-devices` names with the current one marked.
pub fn devices() -> Result<Vec<Device>, String> {
    let current = slot::current_device();
    let out = sys::run("mullvad", &["account", "list-devices"], None);
    if !out.success {
        return Err(command_error(&out));
    }
    Ok(slot::parse_devices(&out.stdout)
        .into_iter()
        .map(|name| Device {
            current: name == current,
            name,
        })
        .collect())
}

/// A failed `mullvad account …` → the sentence to show. Never echoes
/// the output itself (it can hold the account number).
pub fn command_error(out: &sys::Output) -> String {
    let text = out.text();
    let known = [
        (
            "There is no such device",
            "That device is not on this account.",
        ),
        (
            "This voucher code is invalid",
            "That voucher code is not valid.",
        ),
        (
            "This voucher code has already been used",
            "That voucher code has already been used.",
        ),
        (
            "There are too many devices on the account",
            "This account already has the most devices it can have. Revoke one first.",
        ),
        (
            "The account does not exist",
            "There is no Mullvad account with that number.",
        ),
        ("Not logged in", "You are not logged in."),
        (
            "Management RPC server or client error",
            "The Mullvad service is not running.",
        ),
        (
            "Failed to connect to mullvad-daemon",
            "The Mullvad service is not running.",
        ),
    ];
    if !out.spawned {
        return "The mullvad command is not installed.".into();
    }
    known
        .iter()
        .find(|(needle, _)| text.contains(needle))
        .map_or_else(
            || "The Mullvad command failed.".into(),
            |(_, s)| s.to_string(),
        )
}

/// A voucher code as Mullvad prints them: letters / digits in groups,
/// dashes or spaces allowed, 16 characters without separators.
pub fn valid_voucher(code: &str) -> bool {
    let bare: String = code.chars().filter(|c| !matches!(c, '-' | ' ')).collect();
    bare.len() == 16 && bare.chars().all(|c| c.is_ascii_alphanumeric())
}

/// Redeem a voucher. `mullvad account redeem` only takes the code as an
/// argument (no stdin form), so the code is in that one argv while it runs;
/// it is validated first and never printed. Returns the new expiry date.
pub fn redeem(voucher: &str) -> Result<String, String> {
    redeem_with("mullvad", voucher)
}

pub(crate) fn redeem_with(program: &str, voucher: &str) -> Result<String, String> {
    let code = voucher.trim();
    if !valid_voucher(code) {
        return Err("A voucher code has 16 letters and digits.".into());
    }
    let out = sys::run(program, &["account", "redeem", code], None);
    if !out.success {
        return Err(command_error(&out));
    }
    // A redeemed voucher moves the expiry: read it back.
    Ok(
        match parse_account(
            &sys::run(program, &["account", "get"], None).text(),
            now_unix(),
        ) {
            Account::LoggedIn { expiry, .. } => expiry,
            _ => String::new(),
        },
    )
}

/// Revoke another device by name (never this machine's own).
pub fn revoke(name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("No device name.".into());
    }
    if name == slot::current_device() {
        return Err("That is this device; log out instead.".into());
    }
    let out = sys::run("mullvad", &["account", "revoke-device", name], None);
    if out.success {
        Ok(())
    } else {
        Err(command_error(&out))
    }
}

pub fn logout() -> Result<(), String> {
    let out = sys::run("mullvad", &["account", "logout"], None);
    if out.success {
        Ok(())
    } else {
        Err(command_error(&out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::testutil::Fake;

    /// `mullvad account get` captured here, the number replaced.
    const GET: &str = include_str!("../../tests/fixtures/account-get.txt");

    #[test]
    fn parses_the_account_without_its_number() {
        let at = expires_at("2027-02-06 09:59:30 +03:00").expect("expiry");
        let a = parse_account(GET, at - 1);
        assert_eq!(
            a,
            Account::LoggedIn {
                device: "Fast Goat".into(),
                expiry: "2027-02-06".into(),
                expired: false,
            }
        );
        assert!(!format!("{a:?}").contains("0000"));
        assert!(matches!(
            parse_account(GET, at),
            Account::LoggedIn { expired: true, .. }
        ));
        assert_eq!(
            parse_account("Not logged in on any account\n", 0),
            Account::LoggedOut
        );
        assert_eq!(
            parse_account(
                "The current device has been revoked\nMullvad account: 0000000000000000\n",
                0
            ),
            Account::Revoked
        );
        assert_eq!(
            parse_account("Error: Management RPC server or client error", 0),
            Account::Unknown
        );
    }

    #[test]
    fn expiry_timestamps() {
        assert_eq!(expires_at("1970-01-01 00:00:00 +00:00"), Some(0));
        assert_eq!(expires_at("1970-01-01 03:00:00 +03:00"), Some(0));
        assert_eq!(expires_at("1969-12-31 21:00:00.123 -03:00"), Some(0));
        // 2027-02-06T06:59:30Z
        assert_eq!(
            expires_at("2027-02-06 09:59:30 +03:00"),
            Some(1_801_897_170)
        );
        assert_eq!(
            expires_at("2024-02-29 00:00:00 +00:00"),
            Some(1_709_164_800)
        );
        assert_eq!(expires_at("2027-02-06"), None);
        assert_eq!(expires_at("garbage"), None);
        assert_eq!(expires_at("2027-13-01 00:00:00 +00:00"), None);
    }

    #[test]
    fn account_errors_are_sentences() {
        let failed = |stderr: &str| sys::Output {
            spawned: true,
            success: false,
            stdout: String::new(),
            stderr: stderr.into(),
        };
        assert_eq!(
            command_error(&failed("Error: This voucher code is invalid")),
            "That voucher code is not valid."
        );
        assert_eq!(
            command_error(&failed("Error: There is no such device")),
            "That device is not on this account."
        );
        assert_eq!(
            command_error(&failed("Error: boom 1234123412341234")),
            "The Mullvad command failed."
        );
        assert_eq!(
            command_error(&sys::Output::default()),
            "The mullvad command is not installed."
        );
    }

    #[test]
    fn vouchers_are_validated_before_reaching_mullvad() {
        assert!(valid_voucher("ABCD-EFGH-1234-5678"));
        assert!(valid_voucher("abcd efgh 1234 5678"));
        assert!(!valid_voucher("ABCD-EFGH-1234"));
        assert!(!valid_voucher("--help"));
        assert!(!valid_voucher("ABCD-EFGH-1234-567;"));
        let fake = Fake::new("redeem-bad", 1, "", "Error: This voucher code is invalid");
        assert_eq!(
            redeem_with(fake.program(), "ABCD-EFGH-1234-5678"),
            Err("That voucher code is not valid.".into())
        );
        assert_eq!(fake.argv(), "account\nredeem\nABCD-EFGH-1234-5678\n");
        drop(fake);
        let fake = Fake::new("redeem-short", 0, "", "");
        assert!(redeem_with(fake.program(), "nope").is_err());
        assert_eq!(fake.argv(), "", "an invalid code never runs mullvad");
    }
}
