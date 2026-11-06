// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use zbus::object_server::Interface;

#[test]
#[ignore = "requires a private session bus: dbus-run-session cargo test -p mtune dbus::tests -- --include-ignored"]
fn radio_roundtrip_dispatch_errors_and_changed_snapshot() {
    use crate::bridge::RadioCommand;
    use futures::{StreamExt, join};
    gtk::glib::MainContext::new().block_on(async {
        let server = Connection::session().await.unwrap();
        let (tx, rx) = async_channel::unbounded();
        let snap = crate::bridge::new_shared();
        let _connection = serve_on(&server, snap.clone(), tx).await.unwrap();
        let client = Connection::session().await.unwrap();
        let proxy = zbus::proxy::Builder::<zbus::Proxy>::new(&client)
            .destination(server.unique_name().unwrap().to_owned())
            .unwrap()
            .path(OBJECT_PATH)
            .unwrap()
            .interface(IFACE_NAME)
            .unwrap()
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build()
            .await
            .unwrap();
        let mut changed = proxy.receive_signal("Changed").await.unwrap();
        {
            let mut state = snap.lock().unwrap();
            state.radio_stations = vec![(
                "url:https://radio.example/live".into(),
                "Radio".into(),
                "https://radio.example/live".into(),
                "radio.example/live".into(),
            )];
            state.is_live = true;
            state.station_name = "Radio".into();
            state.stream_error = "not-negotiated".into();
            state.playing = false;
        }
        emit_changed(&server).await;
        changed.next().await.unwrap();
        assert!(proxy.get_property::<bool>("IsLive").await.unwrap());
        assert!(!proxy.get_property::<bool>("Playing").await.unwrap());
        assert_eq!(
            proxy.get_property::<String>("StationName").await.unwrap(),
            "Radio"
        );
        assert_eq!(
            proxy.get_property::<String>("StreamError").await.unwrap(),
            "not-negotiated"
        );
        assert_eq!(
            proxy
                .get_property::<Vec<crate::bridge::RadioStation>>("RadioStations")
                .await
                .unwrap()
                .len(),
            1
        );
        let (result, ()) = join!(proxy.call_method("SearchRadio", &("jazz", "TR")), async {
            let AppCommand::SearchRadio(query, reply) = rx.recv().await.unwrap() else {
                panic!("wrong command")
            };
            assert_eq!(query.countrycode, "TR");
            reply
                .send(Err("directory unavailable".into()))
                .await
                .unwrap();
        });
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("directory unavailable")
        );
        for method in ["AddRadioStation", "PreviewRadioStation"] {
            let (result, ()) = join!(
                proxy.call_method(method, &("Radio", "https://radio.example/live")),
                async {
                    let AppCommand::Radio(command, reply) = rx.recv().await.unwrap() else {
                        panic!("wrong command")
                    };
                    assert!(matches!(
                        (method, command),
                        ("AddRadioStation", RadioCommand::Add(_))
                            | ("PreviewRadioStation", RadioCommand::Preview(_))
                    ));
                    reply.send(Ok(())).await.unwrap();
                }
            );
            result.unwrap();
        }
        for method in ["RemoveRadioStation", "PlayRadioStation"] {
            let (result, ()) = join!(proxy.call_method(method, &("missing",)), async {
                let AppCommand::Radio(command, reply) = rx.recv().await.unwrap() else {
                    panic!("wrong command")
                };
                assert!(matches!(
                    (method, command),
                    ("RemoveRadioStation", RadioCommand::Remove(_))
                        | ("PlayRadioStation", RadioCommand::Play(_))
                ));
                reply
                    .send(Err("Station no longer exists".into()))
                    .await
                    .unwrap();
            });
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("Station no longer exists")
            );
        }
        assert!(
            proxy
                .call_method("PreviewRadioStation", &("Bad", "file:///etc/passwd"))
                .await
                .is_err()
        );
        assert!(proxy.call_method("SetVolume", &(f64::NAN,)).await.is_err());
        assert!(
            rx.is_empty(),
            "invalid input must not reach main-context commands"
        );
    });
}

#[test]
fn radio_contract_is_exported_on_existing_interface() {
    let (tx, _) = async_channel::unbounded();
    let service = TuneService {
        snap: crate::bridge::new_shared(),
        tx,
    };
    let mut xml = String::new();
    service.introspect_to_writer(&mut xml, 0);
    for name in [
        "SearchRadio",
        "AddRadioStation",
        "RemoveRadioStation",
        "PlayRadioStation",
        "PreviewRadioStation",
        "RadioStations",
        "IsLive",
        "StreamError",
        "StationName",
    ] {
        assert!(
            xml.contains(&format!("name=\"{name}\"")),
            "missing {name}: {xml}"
        );
    }
}
