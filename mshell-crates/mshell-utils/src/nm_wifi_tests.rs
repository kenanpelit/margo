//! Drive the real D-Bus reads against a private mock NetworkManager on a
//! throwaway `dbus-daemon`, never the live system bus. Synthetic keys only.
use super::*;
use std::collections::HashMap;
use std::io::BufRead;
use zbus::zvariant::{OwnedValue, Value};

const CLIENT: &str = "/org/freedesktop/NetworkManager/Settings/1";
const HOTSPOT: &str = "/org/freedesktop/NetworkManager/Settings/2";
const ACTIVE: &str = "/org/freedesktop/NetworkManager/ActiveConnection/7";

fn v(x: Value<'_>) -> OwnedValue {
    OwnedValue::try_from(x).unwrap()
}

/// A profile as `GetSettings` returns it: no secret inside.
fn profile(id: &str, ssid: &str, mode: &str, key_mgmt: &str) -> Settings {
    let mut conn = HashMap::new();
    conn.insert("id".to_string(), v(Value::from(id)));
    conn.insert("type".to_string(), v(Value::from(WIFI)));
    let mut wifi = HashMap::new();
    wifi.insert("ssid".to_string(), v(Value::from(ssid.as_bytes().to_vec())));
    wifi.insert("mode".to_string(), v(Value::from(mode)));
    let mut wsec = HashMap::new();
    wsec.insert("key-mgmt".to_string(), v(Value::from(key_mgmt)));
    [
        ("connection".to_string(), conn),
        (WIFI.to_string(), wifi),
        (WSEC.to_string(), wsec),
    ]
    .into_iter()
    .collect()
}

fn secrets(psk: &str) -> Settings {
    let mut wsec = HashMap::new();
    wsec.insert("psk".to_string(), v(Value::from(psk)));
    [(WSEC.to_string(), wsec)].into_iter().collect()
}

struct MockNm {
    active: Vec<OwnedObjectPath>,
}
#[zbus::interface(name = "org.freedesktop.NetworkManager")]
impl MockNm {
    #[zbus(property)]
    fn active_connections(&self) -> Vec<OwnedObjectPath> {
        self.active.clone()
    }
}

struct MockActive;
#[zbus::interface(name = "org.freedesktop.NetworkManager.Connection.Active")]
impl MockActive {
    #[zbus(property, name = "Type")]
    fn kind(&self) -> String {
        WIFI.to_string()
    }
    #[zbus(property)]
    fn connection(&self) -> OwnedObjectPath {
        OwnedObjectPath::try_from(CLIENT).unwrap()
    }
}

struct MockSettings;
#[zbus::interface(name = "org.freedesktop.NetworkManager.Settings")]
impl MockSettings {
    fn get_connection_by_uuid(&self, uuid: &str) -> zbus::fdo::Result<OwnedObjectPath> {
        if uuid == "client-uuid" {
            Ok(OwnedObjectPath::try_from(CLIENT).unwrap())
        } else {
            Err(zbus::fdo::Error::Failed("no such connection".into()))
        }
    }

    fn list_connections(&self) -> Vec<OwnedObjectPath> {
        vec![
            OwnedObjectPath::try_from(CLIENT).unwrap(),
            OwnedObjectPath::try_from(HOTSPOT).unwrap(),
        ]
    }
}

type Saved = std::sync::Arc<std::sync::Mutex<Option<Settings>>>;

struct MockConn {
    settings: Settings,
    secret: String,
    saved: Saved,
}
#[zbus::interface(name = "org.freedesktop.NetworkManager.Settings.Connection")]
impl MockConn {
    fn get_settings(&self) -> Settings {
        self.settings
            .iter()
            .map(|(k, m)| {
                (
                    k.clone(),
                    m.iter()
                        .map(|(k, v)| (k.clone(), v.try_clone().unwrap()))
                        .collect(),
                )
            })
            .collect()
    }
    fn get_secrets(&self, setting: &str) -> zbus::fdo::Result<Settings> {
        assert_eq!(setting, WSEC, "only the Wi-Fi security secrets are asked");
        Ok(secrets(&self.secret))
    }
    fn update2(
        &self,
        settings: Settings,
        flags: u32,
        args: HashMap<String, OwnedValue>,
    ) -> HashMap<String, OwnedValue> {
        assert_eq!(flags, TO_DISK);
        assert!(args.is_empty());
        *self.saved.lock().unwrap() = Some(settings);
        HashMap::new()
    }
}

struct PrivateBus(std::process::Child);
impl Drop for PrivateBus {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Serve the mock on a private bus and return (daemon guard, server, client).
async fn mock(active: bool) -> (PrivateBus, zbus::Connection, zbus::Connection) {
    let (d, s, c, _) = mock_saving(active).await;
    (d, s, c)
}

async fn mock_saving(active: bool) -> (PrivateBus, zbus::Connection, zbus::Connection, Saved) {
    let saved = Saved::default();
    let child = std::process::Command::new("dbus-daemon")
        .args(["--session", "--nofork", "--print-address=1"])
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("dbus-daemon is needed for this isolated integration test");
    let mut daemon = PrivateBus(child);
    let mut address = String::new();
    std::io::BufReader::new(daemon.0.stdout.take().unwrap())
        .read_line(&mut address)
        .unwrap();
    let address = address.trim().to_string();
    let active = if active {
        vec![OwnedObjectPath::try_from(ACTIVE).unwrap()]
    } else {
        Vec::new()
    };
    let server = zbus::connection::Builder::address(address.as_str())
        .unwrap()
        .name(NM)
        .unwrap()
        .serve_at(NM_PATH, MockNm { active })
        .unwrap()
        .serve_at(ACTIVE, MockActive)
        .unwrap()
        .serve_at(SETTINGS_PATH, MockSettings)
        .unwrap()
        .serve_at(
            CLIENT,
            MockConn {
                settings: profile("Home", "Home;Net", "infrastructure", "wpa-psk"),
                secret: "synthetic:key".to_string(),
                saved: saved.clone(),
            },
        )
        .unwrap()
        .serve_at(
            HOTSPOT,
            MockConn {
                settings: profile(CONNECTION_NAME, "laptop-hotspot", "ap", "wpa-psk"),
                secret: "hotspot-test-key".to_string(),
                saved: Saved::default(),
            },
        )
        .unwrap()
        .build()
        .await
        .unwrap();
    let client = zbus::connection::Builder::address(address.as_str())
        .unwrap()
        .build()
        .await
        .unwrap();
    (daemon, server, client, saved)
}

#[tokio::test]
async fn the_active_wifi_becomes_an_escaped_payload() {
    let (_d, _s, client) = mock(true).await;
    let share = tokio::time::timeout(TIMEOUT, active_wifi_on(&client))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(share.ssid, "Home;Net");
    assert_eq!(
        *share.payload(),
        r"WIFI:T:WPA;S:Home\;Net;P:synthetic\:key;;"
    );
    // The key never shows up in Debug output.
    assert!(!format!("{share:?}").contains("synthetic"));
}

#[tokio::test]
async fn no_active_wifi_is_a_plain_error() {
    let (_d, _s, client) = mock(false).await;
    let err = active_wifi_on(&client).await.unwrap_err();
    assert_eq!(err, "Not connected to a Wi-Fi network");
}

#[tokio::test]
async fn the_hotspot_profile_is_found_by_name_when_not_running() {
    let (_d, _s, client) = mock(false).await;
    let share = hotspot_on(&client).await.unwrap();
    assert_eq!(share.ssid, "laptop-hotspot");
    assert_eq!(
        *share.payload(),
        "WIFI:T:WPA;S:laptop-hotspot;P:hotspot-test-key;;"
    );
}

#[test]
fn share_from_covers_open_wpa3_and_enterprise() {
    let mut open = profile("Cafe", "Cafe", "infrastructure", "x");
    open.remove(WSEC);
    let share = share_from(&open, &Settings::new()).unwrap();
    assert_eq!(*share.payload(), "WIFI:T:nopass;S:Cafe;;");

    let sae = profile("Lab", "Lab", "infrastructure", "sae");
    let share = share_from(&sae, &secrets("k3y")).unwrap();
    assert_eq!(*share.payload(), "WIFI:T:WPA;R:1;S:Lab;P:k3y;;");

    let eap = profile("Corp", "Corp", "infrastructure", "wpa-eap");
    assert!(
        share_from(&eap, &Settings::new())
            .unwrap_err()
            .contains("802.1X")
    );

    // An agent-owned key NetworkManager can't hand out.
    let wpa = profile("Home", "Home", "infrastructure", "wpa-psk");
    assert!(
        share_from(&wpa, &Settings::new())
            .unwrap_err()
            .contains("no stored password")
    );
}

#[tokio::test]
async fn a_new_key_reaches_update2_with_the_rest_of_the_profile() {
    let (_d, _s, client, saved) = mock_saving(true).await;
    tokio::time::timeout(TIMEOUT, set_psk_on(&client, "client-uuid", "new key; ok"))
        .await
        .unwrap()
        .unwrap();
    let s = saved
        .lock()
        .unwrap()
        .take()
        .expect("NetworkManager received no settings");
    let psk: &str = (&s[WSEC]["psk"]).try_into().unwrap();
    assert_eq!(psk, "new key; ok");
    // key-mgmt and the rest of the profile are kept, not rewritten.
    let km: &str = (&s[WSEC]["key-mgmt"]).try_into().unwrap();
    assert_eq!(km, "wpa-psk");
    assert_eq!(ssid_of(&s).as_deref(), Some("Home;Net"));
    assert!(set_psk_on(&client, "nope", "k").await.is_err());
}
