// SPDX-License-Identifier: GPL-3.0-or-later
//! Main-context radio operations, sharing Discover's library and writer.
use super::*;
use crate::{
    bridge::{RadioCommand, RadioStation},
    discover::{play, shared},
    radio::{Station, api},
};

pub(super) fn wire(station: &Station) -> RadioStation {
    (
        station.key(),
        station.name.clone(),
        station.url.clone(),
        station.subtitle(),
    )
}

impl Application {
    pub(super) fn radio_command(&self, command: RadioCommand) -> Result<(), String> {
        match command {
            RadioCommand::Preview(station) => {
                self.play_radio_list(&[station], 0);
            }
            RadioCommand::Add(station) => {
                let lib = shared::radio();
                if !lib.writable {
                    return Err(
                        "Radio library is unreadable; repair radio.json before saving".into(),
                    );
                }
                lib.data
                    .borrow_mut()
                    .add_custom(&station.name, &station.url)
                    .map_err(|e| format!("Cannot add station: {e:?}"))?;
                shared::radio_changed();
            }
            RadioCommand::Remove(key) => {
                let lib = shared::radio();
                if !lib.writable {
                    return Err(
                        "Radio library is unreadable; repair radio.json before saving".into(),
                    );
                }
                {
                    let mut data = lib.data.borrow_mut();
                    if !data.contains(&key) {
                        return Err("Station no longer exists".into());
                    }
                    data.remove(&key);
                }
                shared::radio_changed();
            }
            RadioCommand::Play(key) => {
                let lib = shared::radio();
                let stations = lib.data.borrow().stations.clone();
                let index = stations
                    .iter()
                    .position(|s| s.key() == key)
                    .ok_or("Station no longer exists")?;
                // A hand-edited saved library is also an untrusted URL source.
                for station in &stations {
                    dbus::radio::station(&station.name, &station.url)?;
                }
                self.play_radio_list(&stations, index);
            }
        }
        Ok(())
    }

    fn play_radio_list(&self, stations: &[Station], index: usize) {
        let songs: Vec<_> = stations.iter().map(play::station_song).collect();
        if let Some(win) = self.active_window().and_downcast::<Window>() {
            win.play_station_list(&songs, index as u32);
        } else {
            // Same queue semantics as Window::play_station_list, without
            // constructing/presenting a window on a --hidden launch.
            let player = &self.imp().player;
            if !player.queue().is_empty() {
                player.clear_queue();
            }
            player.queue().add_songs(&songs);
            player.skip_to(index as u32);
            player.play();
        }
        if let Some(station) = stations.get(index).filter(|s| !s.custom) {
            let uuid = station.uuid.clone();
            glib::spawn_future_local(async move {
                crate::net::off_main(move || api::count_click(&uuid)).await;
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires xvfb-run, a private D-Bus session and isolated XDG directories"]
    fn hidden_radio_uses_shared_library_and_queue_without_saving_preview() {
        assert_eq!(
            std::env::var("MARGO_MTUNE_TEST_ISOLATED").as_deref(),
            Ok("1"),
            "Set MARGO_MTUNE_TEST_ISOLATED=1 only with fresh isolated XDG directories"
        );
        adw::init().unwrap();
        gst::init().unwrap();
        let resources = gio::Resource::from_data(&glib::Bytes::from_static(include_bytes!(
            concat!(env!("OUT_DIR"), "/mtune.gresource")
        )))
        .unwrap();
        gio::resources_register(&resources);
        let context = glib::MainContext::default();
        let _guard = context.acquire().unwrap();
        let app = Application::new();
        app.register(gio::Cancellable::NONE).unwrap();
        assert!(app.active_window().is_none());
        let first = dbus::radio::station("First", "https://radio.example/first").unwrap();
        let second = dbus::radio::station("Second", "https://radio.example/second").unwrap();
        app.radio_command(RadioCommand::Add(first.clone())).unwrap();
        app.radio_command(RadioCommand::Add(second.clone()))
            .unwrap();
        assert!(app.radio_command(RadioCommand::Add(first.clone())).is_err());
        assert_eq!(shared::radio().data.borrow().stations.len(), 2);
        app.radio_command(RadioCommand::Play(second.key())).unwrap();
        assert_eq!(app.imp().player.queue().n_songs(), 2);
        assert_eq!(app.imp().player.queue().current_song_index(), Some(1));
        assert!(app.imp().player.state().current_song().unwrap().is_live());
        let preview = dbus::radio::station("Preview", "https://radio.example/preview").unwrap();
        app.radio_command(RadioCommand::Preview(preview)).unwrap();
        assert_eq!(app.imp().player.queue().n_songs(), 1);
        assert_eq!(
            shared::radio().data.borrow().stations.len(),
            2,
            "preview must not save"
        );
        app.imp().player.state().set_stream_error("not-negotiated");
        app.refresh_bridge();
        {
            let snapshot = app.imp().snap.lock().unwrap();
            assert!(snapshot.is_live);
            assert_eq!(snapshot.station_name, "Preview");
            assert_eq!(snapshot.stream_error, "not-negotiated");
            assert_eq!(snapshot.radio_stations.len(), 2);
        }
        assert!(
            app.active_window().is_none(),
            "radio must not create a window"
        );
        app.radio_command(RadioCommand::Remove(first.key()))
            .unwrap();
        assert_eq!(shared::radio().data.borrow().stations, vec![second.clone()]);
        assert!(app.radio_command(RadioCommand::Play(first.key())).is_err());
        assert!(
            app.radio_command(RadioCommand::Remove(first.key()))
                .is_err()
        );
        app.imp().player.stop();
        shared::flush();
        let (saved, writable) = crate::radio::store::RadioLibrary::load();
        assert!(writable);
        assert_eq!(saved.stations, vec![second]);
    }
}
