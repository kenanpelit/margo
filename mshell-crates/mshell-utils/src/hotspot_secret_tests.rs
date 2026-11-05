//! Exercise the real `a{sa{sv}}` D-Bus write, never the live system bus.
use super::*;
use std::io::BufRead;
use std::sync::{Arc, Mutex};

const PROFILE: &str = "/org/freedesktop/NetworkManager/Settings/1";
type Saved = Arc<Mutex<Option<Settings>>>;

struct MockSettings {
    existing: bool,
    saved: Saved,
}

#[zbus::interface(name = "org.freedesktop.NetworkManager.Settings")]
impl MockSettings {
    fn list_connections(&self) -> Vec<OwnedObjectPath> {
        if self.existing {
            vec![OwnedObjectPath::try_from(PROFILE).unwrap()]
        } else {
            vec![]
        }
    }

    fn add_connection2(
        &self,
        settings: Settings,
        flags: u32,
        args: HashMap<String, OwnedValue>,
    ) -> (OwnedObjectPath, HashMap<String, OwnedValue>) {
        assert_eq!(flags, TO_DISK);
        assert!(args.is_empty());
        *self.saved.lock().unwrap() = Some(settings);
        (OwnedObjectPath::try_from(PROFILE).unwrap(), HashMap::new())
    }
}

struct MockConnection {
    saved: Saved,
}

#[zbus::interface(name = "org.freedesktop.NetworkManager.Settings.Connection")]
impl MockConnection {
    fn get_settings(&self) -> Settings {
        let mut settings = new_profile("old-ssid", "old-test-key", "test-uuid").unwrap();
        // NetworkManager GetSettings never exposes the stored secret.
        settings.get_mut(WSEC).unwrap().remove("psk");
        settings
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

async fn exercise(existing: bool) {
    // A private bus with no network service, no session changes, no secrets.
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
    let address = address.trim();
    let saved: Saved = Arc::default();
    let _server = zbus::connection::Builder::address(address)
        .unwrap()
        .name(NM)
        .unwrap()
        .serve_at(
            SETTINGS_PATH,
            MockSettings {
                existing,
                saved: saved.clone(),
            },
        )
        .unwrap()
        .serve_at(
            PROFILE,
            MockConnection {
                saved: saved.clone(),
            },
        )
        .unwrap()
        .build()
        .await
        .unwrap();
    let client = zbus::connection::Builder::address(address)
        .unwrap()
        .build()
        .await
        .unwrap();
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        store_on(&client, "new-ssid", "synthetic-test-key"),
    )
    .await
    .expect("D-Bus test timed out")
    .unwrap();
    let saved = saved.lock().unwrap();
    let settings = saved
        .as_ref()
        .expect("NetworkManager did not receive settings");
    let key: &str = (&settings[WSEC]["psk"]).try_into().unwrap();
    assert_eq!(key, "synthetic-test-key");
    assert_eq!(settings[WSEC]["psk-flags"], OwnedValue::from(0u32));
    let ssid: Vec<u8> = settings["802-11-wireless"]["ssid"]
        .try_clone()
        .unwrap()
        .try_into()
        .unwrap();
    assert_eq!(
        ssid,
        if existing {
            b"old-ssid".as_slice()
        } else {
            b"new-ssid".as_slice()
        }
    );
}

#[tokio::test]
async fn a_new_password_reaches_add_connection2_over_dbus() {
    exercise(false).await;
}

#[tokio::test]
async fn an_existing_password_reaches_update2_over_dbus() {
    exercise(true).await;
}
