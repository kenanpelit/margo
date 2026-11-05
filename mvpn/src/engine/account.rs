//! The Mullvad account: who is logged in and until when.
//!
//! `mullvad account get` prints the account number; it is parsed away
//! here and never kept, printed or logged.

use mvpn_proto::health::Account;

use super::sys;

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
