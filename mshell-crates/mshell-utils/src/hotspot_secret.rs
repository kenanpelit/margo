//! The hotspot password lives in NetworkManager, not in the shell config.
//!
//! It used to be `network.hotspot_password` in the profile YAML (world-
//! readable, and the profiles directory is often a dotfiles git repo) and
//! went to NetworkManager on the `nmcli … password <pw>` argv, which every
//! local user can read from `/proc/<pid>/cmdline`.
//!
//! Now it is a secret of the `margo-hotspot` connection profile
//! (`802-11-wireless-security.psk`, system-owned, so NetworkManager keeps
//! it in its root-only `system-connections` store). It travels to
//! NetworkManager only in the settings dict of a D-Bus `Update2` /
//! `AddConnection2` call, never on an argv. Starting the hotspot then
//! activates that profile and needs no password at all.
//!
//! A value still in the config is moved here once and cleared
//! ([`super::hotspot::migrate_config_password`]).

use std::collections::HashMap;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

use crate::hotspot::CONNECTION_NAME;

const NM: &str = "org.freedesktop.NetworkManager";
const SETTINGS_PATH: &str = "/org/freedesktop/NetworkManager/Settings";
const SETTINGS_IFACE: &str = "org.freedesktop.NetworkManager.Settings";
const CONNECTION_IFACE: &str = "org.freedesktop.NetworkManager.Settings.Connection";
/// `NM_SETTINGS_UPDATE2_FLAG_TO_DISK` / `NM_SETTINGS_ADD_CONNECTION2_FLAG_TO_DISK`.
const TO_DISK: u32 = 0x1;
const WSEC: &str = "802-11-wireless-security";

/// A connection's settings, `a{sa{sv}}`.
pub type Settings = HashMap<String, HashMap<String, OwnedValue>>;

fn owned(v: Value<'_>) -> Result<OwnedValue, String> {
    OwnedValue::try_from(v).map_err(|e| e.to_string())
}

/// `connection.id` of `settings`, if it is a string.
fn connection_id(settings: &Settings) -> Option<&str> {
    settings
        .get("connection")?
        .get("id")
        .and_then(|v| <&str>::try_from(v).ok())
}

/// `settings` with `psk` as its WPA-PSK secret, system-owned (flags 0, so
/// NetworkManager stores it rather than asking an agent each time).
pub fn with_psk(mut settings: Settings, psk: &str) -> Result<Settings, String> {
    let wsec = settings.entry(WSEC.to_string()).or_default();
    wsec.insert("key-mgmt".into(), owned(Value::from("wpa-psk"))?);
    wsec.insert("psk".into(), owned(Value::from(psk))?);
    wsec.insert("psk-flags".into(), owned(Value::from(0u32))?);
    Ok(settings)
}

/// A new `margo-hotspot` profile: what `nmcli device wifi hotspot` creates
/// (AP mode, IPv4 shared, IPv6 ignored, never autoconnect) with `psk` as
/// its WPA2 key.
pub fn new_profile(ssid: &str, psk: &str, uuid: &str) -> Result<Settings, String> {
    let mut connection = HashMap::new();
    connection.insert("id".into(), owned(Value::from(CONNECTION_NAME))?);
    connection.insert("uuid".into(), owned(Value::from(uuid))?);
    connection.insert("type".into(), owned(Value::from("802-11-wireless"))?);
    connection.insert("autoconnect".into(), owned(Value::from(false))?);
    let mut wifi = HashMap::new();
    wifi.insert("ssid".into(), owned(Value::from(ssid.as_bytes().to_vec()))?);
    wifi.insert("mode".into(), owned(Value::from("ap"))?);
    let mut wsec = HashMap::new();
    wsec.insert("proto".into(), owned(Value::from(vec!["rsn"]))?);
    wsec.insert("pairwise".into(), owned(Value::from(vec!["ccmp"]))?);
    wsec.insert("group".into(), owned(Value::from(vec!["ccmp"]))?);
    let mut ipv4 = HashMap::new();
    ipv4.insert("method".into(), owned(Value::from("shared"))?);
    let mut ipv6 = HashMap::new();
    ipv6.insert("method".into(), owned(Value::from("ignore"))?);
    let settings: Settings = [
        ("connection".to_string(), connection),
        ("802-11-wireless".to_string(), wifi),
        (WSEC.to_string(), wsec),
        ("ipv4".to_string(), ipv4),
        ("ipv6".to_string(), ipv6),
    ]
    .into_iter()
    .collect();
    with_psk(settings, psk)
}

async fn system_bus() -> Result<zbus::Connection, String> {
    zbus::Connection::system()
        .await
        .map_err(|e| format!("system bus: {e}"))
}

/// The `margo-hotspot` profile's object path and (secret-free) settings.
async fn find_profile(
    bus: &zbus::Connection,
) -> Result<Option<(OwnedObjectPath, Settings)>, String> {
    let settings = zbus::Proxy::new(bus, NM, SETTINGS_PATH, SETTINGS_IFACE)
        .await
        .map_err(|e| e.to_string())?;
    let paths: Vec<OwnedObjectPath> = settings
        .call("ListConnections", &())
        .await
        .map_err(|e| format!("NetworkManager ListConnections: {e}"))?;
    for path in paths {
        let Ok(conn) = zbus::Proxy::new(bus, NM, path.as_str(), CONNECTION_IFACE).await else {
            continue;
        };
        let Ok(s) = conn.call::<_, _, Settings>("GetSettings", &()).await else {
            continue;
        };
        if connection_id(&s) == Some(CONNECTION_NAME) {
            return Ok(Some((path, s)));
        }
    }
    Ok(None)
}

/// Whether NetworkManager has a `margo-hotspot` profile.
pub async fn profile_exists() -> Result<bool, String> {
    let bus = system_bus().await?;
    Ok(find_profile(&bus).await?.is_some())
}

/// Store `password` as the hotspot's WPA2 key in NetworkManager: into the
/// existing `margo-hotspot` profile, or a new one named `ssid`. The secret
/// goes over D-Bus only (never an argv, never the shell config).
pub async fn store_password(ssid: &str, password: &str) -> Result<(), String> {
    tokio::time::timeout(
        std::time::Duration::from_secs(15),
        store_password_inner(ssid, password),
    )
    .await
    .map_err(|_| "NetworkManager password save timed out".to_string())?
}

async fn store_password_inner(ssid: &str, password: &str) -> Result<(), String> {
    crate::hotspot::validate(ssid, password)?;
    if password.is_empty() {
        return Err("Hotspot password is empty".into());
    }
    let bus = system_bus().await?;
    store_on(&bus, ssid, password).await
}

/// Kept separate so the actual D-Bus calls are exercised against a private
/// mock daemon in tests, never the running NetworkManager.
async fn store_on(bus: &zbus::Connection, ssid: &str, password: &str) -> Result<(), String> {
    match find_profile(bus).await? {
        Some((path, settings)) => {
            let conn = zbus::Proxy::new(bus, NM, path.as_str(), CONNECTION_IFACE)
                .await
                .map_err(|e| e.to_string())?;
            let settings = with_psk(settings, password)?;
            conn.call::<_, _, HashMap<String, OwnedValue>>(
                "Update2",
                &(settings, TO_DISK, HashMap::<String, OwnedValue>::new()),
            )
            .await
            .map_err(|_| "NetworkManager could not save the hotspot password".to_string())?;
        }
        None => {
            let uuid = std::fs::read_to_string("/proc/sys/kernel/random/uuid")
                .map_err(|e| format!("uuid: {e}"))?;
            let settings = new_profile(ssid, password, uuid.trim())?;
            let nm = zbus::Proxy::new(bus, NM, SETTINGS_PATH, SETTINGS_IFACE)
                .await
                .map_err(|e| e.to_string())?;
            nm.call::<_, _, (OwnedObjectPath, HashMap<String, OwnedValue>)>(
                "AddConnection2",
                &(settings, TO_DISK, HashMap::<String, OwnedValue>::new()),
            )
            .await
            .map_err(|_| "NetworkManager could not create the hotspot profile".to_string())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn str_of<'a>(s: &'a Settings, section: &str, key: &str) -> Option<&'a str> {
        s.get(section)?
            .get(key)
            .and_then(|v| <&str>::try_from(v).ok())
    }

    #[test]
    fn a_new_profile_is_the_nmcli_hotspot_with_our_key() {
        let s = new_profile("kenp-hotspot", "correct horse", "1234-uuid").unwrap();
        assert_eq!(connection_id(&s), Some(CONNECTION_NAME));
        assert_eq!(str_of(&s, "connection", "type"), Some("802-11-wireless"));
        assert_eq!(
            s["connection"]["autoconnect"],
            OwnedValue::from(false),
            "a reboot must not turn the laptop into an access point"
        );
        assert_eq!(str_of(&s, "802-11-wireless", "mode"), Some("ap"));
        let ssid: Vec<u8> = s["802-11-wireless"]["ssid"]
            .try_clone()
            .unwrap()
            .try_into()
            .unwrap();
        assert_eq!(ssid, b"kenp-hotspot");
        assert_eq!(str_of(&s, "ipv4", "method"), Some("shared"));
        assert_eq!(str_of(&s, WSEC, "key-mgmt"), Some("wpa-psk"));
        assert_eq!(str_of(&s, WSEC, "psk"), Some("correct horse"));
        assert_eq!(s[WSEC]["psk-flags"], OwnedValue::from(0u32));
    }

    #[test]
    fn the_key_replaces_an_existing_profiles_secret_and_keeps_the_rest() {
        let existing = new_profile("old", "old-secret", "u").unwrap();
        let mut existing = existing;
        existing.get_mut(WSEC).unwrap().remove("psk");
        let s = with_psk(existing, "new-secret!").unwrap();
        assert_eq!(str_of(&s, WSEC, "psk"), Some("new-secret!"));
        assert_eq!(str_of(&s, "802-11-wireless", "mode"), Some("ap"));
        assert_eq!(connection_id(&s), Some(CONNECTION_NAME));
    }
}

#[path = "hotspot_secret_tests.rs"]
#[cfg(test)]
mod hotspot_secret_tests;
