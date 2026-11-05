//! NetworkManager Wi-Fi secrets over D-Bus, for the places that must
//! touch a Wi-Fi key:
//!
//! * **sharing** — the QR code of the network the laptop is on, or of the
//!   hotspot it runs ([`active_wifi`], [`hotspot`]): `GetSettings` names
//!   the network, `GetSecrets("802-11-wireless-security")` hands back the
//!   key, which only ever lives in a [`Zeroizing`] buffer on its way into
//!   the QR payload (qrencode's stdin);
//!
//! Nothing here logs, prints or `Debug`-formats a key. The pure halves
//! (settings → [`WifiShare`]) are unit-tested; the D-Bus halves run against
//! a private mock daemon in `nm_wifi_tests.rs`, never the live system bus.

use zbus::zvariant::OwnedObjectPath;
use zeroize::Zeroizing;

use crate::hotspot::CONNECTION_NAME;
use crate::hotspot_secret::Settings;
use crate::qr::{WifiAuth, wifi_payload};

const NM: &str = "org.freedesktop.NetworkManager";
const NM_PATH: &str = "/org/freedesktop/NetworkManager";
const NM_IFACE: &str = "org.freedesktop.NetworkManager";
const ACTIVE_IFACE: &str = "org.freedesktop.NetworkManager.Connection.Active";
const SETTINGS_PATH: &str = "/org/freedesktop/NetworkManager/Settings";
const SETTINGS_IFACE: &str = "org.freedesktop.NetworkManager.Settings";
const CONNECTION_IFACE: &str = "org.freedesktop.NetworkManager.Settings.Connection";
const WIFI: &str = "802-11-wireless";
const WSEC: &str = "802-11-wireless-security";

/// A Wi-Fi network as a phone needs it to join. The key is zeroized on
/// drop and kept out of `Debug`.
pub struct WifiShare {
    pub ssid: String,
    pub auth: WifiAuth,
    password: Zeroizing<String>,
    pub hidden: bool,
}

impl std::fmt::Debug for WifiShare {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WifiShare")
            .field("ssid", &self.ssid)
            .field("auth", &self.auth)
            .field("password", &"<redacted>")
            .field("hidden", &self.hidden)
            .finish()
    }
}

impl WifiShare {
    /// The `WIFI:` QR payload (zeroized on drop).
    pub fn payload(&self) -> Zeroizing<String> {
        wifi_payload(&self.ssid, self.auth, &self.password, self.hidden)
    }
}

fn str_of<'a>(s: &'a Settings, section: &str, key: &str) -> Option<&'a str> {
    s.get(section)?
        .get(key)
        .and_then(|v| <&str>::try_from(v).ok())
}

/// The SSID (`ay`) of a Wi-Fi profile, lossily as text.
fn ssid_of(s: &Settings) -> Option<String> {
    let v = s.get(WIFI)?.get("ssid")?.try_clone().ok()?;
    let bytes: Vec<u8> = v.try_into().ok()?;
    (!bytes.is_empty()).then(|| String::from_utf8_lossy(&bytes).into_owned())
}

/// `802-11-wireless.mode`; NetworkManager omits it for the default
/// (`infrastructure`).
pub fn wifi_mode(s: &Settings) -> &str {
    str_of(s, WIFI, "mode").unwrap_or("infrastructure")
}

/// Build the share from a profile's (secret-free) `settings` and its
/// `secrets` (the `GetSecrets("802-11-wireless-security")` reply).
pub fn share_from(settings: &Settings, secrets: &Settings) -> Result<WifiShare, String> {
    let ssid = ssid_of(settings).ok_or_else(|| "Not a Wi-Fi connection".to_string())?;
    let key_mgmt = str_of(settings, WSEC, "key-mgmt").unwrap_or("");
    let wep_index = settings
        .get(WSEC)
        .and_then(|w| w.get("wep-tx-keyidx"))
        .and_then(|v| u32::try_from(v).ok())
        .unwrap_or(0);
    let wep_key = str_of(secrets, WSEC, &format!("wep-key{wep_index}"));
    let auth = WifiAuth::from_key_mgmt(key_mgmt, wep_key.is_some())
        .ok_or_else(|| "Enterprise (802.1X) networks can't be shared as a QR code".to_string())?;
    let password = match auth {
        WifiAuth::Open => Zeroizing::new(String::new()),
        WifiAuth::Wep => Zeroizing::new(wep_key.unwrap_or_default().to_string()),
        WifiAuth::Wpa | WifiAuth::Wpa3Only => Zeroizing::new(
            str_of(secrets, WSEC, "psk")
                .filter(|p| !p.is_empty())
                .ok_or_else(|| {
                    "NetworkManager has no stored password for this network \
                     (it may be kept in a keyring agent)"
                        .to_string()
                })?
                .to_string(),
        ),
    };
    let hidden = settings
        .get(WIFI)
        .and_then(|w| w.get("hidden"))
        .and_then(|v| bool::try_from(v).ok())
        .unwrap_or(false);
    Ok(WifiShare {
        ssid,
        auth,
        password,
        hidden,
    })
}

// ── D-Bus ──────────────────────────────────────────────────────────────────

async fn system_bus() -> Result<zbus::Connection, String> {
    zbus::Connection::system()
        .await
        .map_err(|e| format!("system bus: {e}"))
}

async fn proxy<'a>(
    bus: &zbus::Connection,
    path: &'a str,
    iface: &'a str,
) -> Result<zbus::Proxy<'a>, String> {
    zbus::Proxy::new(bus, NM, path, iface)
        .await
        .map_err(|e| e.to_string())
}

async fn get_settings(bus: &zbus::Connection, path: &str) -> Result<Settings, String> {
    proxy(bus, path, CONNECTION_IFACE)
        .await?
        .call("GetSettings", &())
        .await
        .map_err(|e| format!("NetworkManager GetSettings: {e}"))
}

async fn share_at(bus: &zbus::Connection, path: &str) -> Result<WifiShare, String> {
    let settings = get_settings(bus, path).await?;
    // An open network has no security section and no secrets to ask for.
    let secrets = if settings.contains_key(WSEC) {
        proxy(bus, path, CONNECTION_IFACE)
            .await?
            .call::<_, _, Settings>("GetSecrets", &(WSEC,))
            .await
            .map_err(|_| "NetworkManager would not hand out the network password".to_string())?
    } else {
        Settings::new()
    };
    share_from(&settings, &secrets)
}

/// Settings paths of the active Wi-Fi connections.
async fn active_wifi_paths(bus: &zbus::Connection) -> Result<Vec<OwnedObjectPath>, String> {
    let nm = proxy(bus, NM_PATH, NM_IFACE).await?;
    let active: Vec<OwnedObjectPath> = nm
        .get_property("ActiveConnections")
        .await
        .map_err(|e| format!("NetworkManager ActiveConnections: {e}"))?;
    let mut out = Vec::new();
    for a in active {
        let Ok(ac) = proxy(bus, a.as_str(), ACTIVE_IFACE).await else {
            continue;
        };
        let kind: String = ac.get_property("Type").await.unwrap_or_default();
        if kind != WIFI {
            continue;
        }
        if let Ok(path) = ac.get_property::<OwnedObjectPath>("Connection").await {
            out.push(path);
        }
    }
    Ok(out)
}

/// The Wi-Fi network this machine is connected to (client mode).
pub async fn active_wifi_on(bus: &zbus::Connection) -> Result<WifiShare, String> {
    for path in active_wifi_paths(bus).await? {
        let settings = get_settings(bus, path.as_str()).await?;
        if wifi_mode(&settings) == "infrastructure" {
            return share_at(bus, path.as_str()).await;
        }
    }
    Err("Not connected to a Wi-Fi network".to_string())
}

/// The hotspot: the running access point, else the saved `margo-hotspot`
/// profile (so the code can be shown before the hotspot is started).
pub async fn hotspot_on(bus: &zbus::Connection) -> Result<WifiShare, String> {
    for path in active_wifi_paths(bus).await? {
        let settings = get_settings(bus, path.as_str()).await?;
        if wifi_mode(&settings) == "ap" {
            return share_at(bus, path.as_str()).await;
        }
    }
    let list: Vec<OwnedObjectPath> = proxy(bus, SETTINGS_PATH, SETTINGS_IFACE)
        .await?
        .call("ListConnections", &())
        .await
        .map_err(|e| format!("NetworkManager ListConnections: {e}"))?;
    for path in list {
        let Ok(s) = get_settings(bus, path.as_str()).await else {
            continue;
        };
        if str_of(&s, "connection", "id") == Some(CONNECTION_NAME) {
            return share_at(bus, path.as_str()).await;
        }
    }
    Err("No hotspot yet — start it once, or set its password in Settings → Network".to_string())
}

const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

async fn bounded<T>(
    fut: impl std::future::Future<Output = Result<T, String>>,
) -> Result<T, String> {
    tokio::time::timeout(TIMEOUT, fut)
        .await
        .map_err(|_| "NetworkManager did not answer".to_string())?
}

/// [`active_wifi_on`] on the system bus.
pub async fn active_wifi() -> Result<WifiShare, String> {
    bounded(async { active_wifi_on(&system_bus().await?).await }).await
}

/// [`hotspot_on`] on the system bus.
pub async fn hotspot() -> Result<WifiShare, String> {
    bounded(async { hotspot_on(&system_bus().await?).await }).await
}

#[path = "nm_wifi_tests.rs"]
#[cfg(test)]
mod nm_wifi_tests;
