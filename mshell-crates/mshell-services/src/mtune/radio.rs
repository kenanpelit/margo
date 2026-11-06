//! Radio controls are native Tune IPC, never a shell-side station library.
use super::*;

type Station = (String, String, String, String);

/// Wait for a hidden player's interface before sending the original action.
/// Proxy construction alone succeeds even when the bus name has no owner.
async fn ready_proxy() -> Result<zbus::Proxy<'static>, String> {
    // Concurrent search/control requests must not launch multiple instances:
    // GApplication interprets the second --hidden invocation as Raise.
    static STARTUP: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _startup = STARTUP.lock().await;
    let conn = Connection::session().await.map_err(|e| e.to_string())?;
    let dbus = DBusProxy::new(&conn).await.map_err(|e| e.to_string())?;
    let name = BusName::try_from(BUS_NAME).map_err(|e| e.to_string())?;
    if !dbus
        .name_has_owner(name.clone())
        .await
        .map_err(|e| e.to_string())?
    {
        std::process::Command::new("mtune")
            .arg("--hidden")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn_reaped()
            .map_err(|e| format!("Cannot start Tune: {e}"))?;
    }
    let proxy = zbus::proxy::Builder::<zbus::Proxy>::new(&conn)
        .destination(BUS_NAME)
        .map_err(|e| e.to_string())?
        .path(OBJECT_PATH)
        .map_err(|e| e.to_string())?
        .interface(IFACE)
        .map_err(|e| e.to_string())?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await
        .map_err(|e| e.to_string())?;
    // The MPRIS name may appear before supplementary interface registration.
    for _ in 0..100 {
        if proxy.get_property::<bool>("Playing").await.is_ok() {
            return Ok(proxy);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err("Tune did not become ready".into())
}

async fn radio_call<T>(method: &'static str, body: T) -> Result<zbus::Message, String>
where
    T: serde::Serialize + zbus::zvariant::DynamicType + Send + Sync + 'static,
{
    // Public methods can be awaited from GLib. All timers and D-Bus work
    // live on the service runtime, not a potentially absent Tokio context.
    tokio_rt()
        .spawn(async move {
            let proxy = ready_proxy().await?;
            proxy
                .call_method(method, &body)
                .await
                .map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| e.to_string())?
}

impl MtunePlayer {
    pub async fn search_radio(&self, query: &str, country: &str) -> Result<Vec<Station>, String> {
        radio_call("SearchRadio", (query.to_owned(), country.to_owned()))
            .await?
            .body()
            .deserialize()
            .map_err(|e| e.to_string())
    }

    pub async fn add_radio_station(&self, name: &str, url: &str) -> Result<(), String> {
        radio_call("AddRadioStation", (name.to_owned(), url.to_owned()))
            .await
            .map(|_| ())
    }

    pub async fn remove_radio_station(&self, key: &str) -> Result<(), String> {
        radio_call("RemoveRadioStation", (key.to_owned(),))
            .await
            .map(|_| ())
    }

    pub async fn play_radio_station(&self, key: &str) -> Result<(), String> {
        radio_call("PlayRadioStation", (key.to_owned(),))
            .await
            .map(|_| ())
    }

    pub async fn preview_radio_station(&self, name: &str, url: &str) -> Result<(), String> {
        radio_call("PreviewRadioStation", (name.to_owned(), url.to_owned()))
            .await
            .map(|_| ())
    }

    pub async fn set_volume(&self, volume: f64) {
        if volume.is_finite()
            && let Err(error) = radio_call("SetVolume", (volume.clamp(0.0, 1.0),)).await
        {
            debug!(%error, "mtune: setting volume failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct LegacyTune;
    #[zbus::interface(name = "org.margo.Tune")]
    impl LegacyTune {
        #[zbus(property)]
        fn playing(&self) -> bool {
            false
        }

        async fn search_radio(
            &self,
            _query: &str,
            _country: &str,
        ) -> zbus::fdo::Result<Vec<Station>> {
            Err(zbus::fdo::Error::Failed("directory unavailable".into()))
        }
    }

    #[test]
    fn disconnect_clears_radio_state() {
        let player = MtunePlayer::new();
        player.is_live.set(true);
        player.station_name.set("Old station".into());
        player.stream_error.set("old failure".into());
        player.radio_stations.set(vec![(
            "key".into(),
            "name".into(),
            "url".into(),
            "subtitle".into(),
        )]);
        reset(&player);
        assert!(!player.is_live.get());
        assert!(player.station_name.get().is_empty());
        assert!(player.stream_error.get().is_empty());
        assert!(player.radio_stations.get().is_empty());
    }

    #[test]
    #[ignore = "requires a private session bus"]
    fn older_tune_properties_default_and_search_errors_reach_caller() {
        tokio_rt().block_on(async {
            let _server = zbus::connection::Builder::session()
                .unwrap()
                .name(BUS_NAME)
                .unwrap()
                .serve_at(OBJECT_PATH, LegacyTune)
                .unwrap()
                .build()
                .await
                .unwrap();
            let player = MtunePlayer::new();
            player.is_live.set(true);
            player.station_name.set("stale".into());
            player.stream_error.set("stale".into());
            let proxy = ready_proxy().await.unwrap();
            refresh(&proxy, &player).await;
            assert!(!player.is_live.get());
            assert!(player.station_name.get().is_empty());
            assert!(player.stream_error.get().is_empty());
            assert!(player.radio_stations.get().is_empty());
            assert_eq!(player.volume.get(), 1.0);
            assert!(
                player
                    .search_radio("jazz", "TR")
                    .await
                    .unwrap_err()
                    .contains("directory unavailable")
            );
            assert!(
                player
                    .preview_radio_station("Radio", "https://radio.example/live")
                    .await
                    .is_err(),
                "old Tune must report unsupported controls"
            );
        });
    }
}
