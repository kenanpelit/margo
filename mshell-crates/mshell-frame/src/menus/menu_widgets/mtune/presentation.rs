use super::*;

pub(super) fn read(m: &mut MtuneMenuWidgetModel) {
    let p = mtune_service().player.clone();
    m.is_live = p.is_live.get();
    m.station_name = p.station_name.get();
    m.stream_error = p.stream_error.get();
    m.volume = p.volume.get();
    m.radio_stations = p.radio_stations.get();
    m.running = p.running.get();
    m.playing = p.playing.get();
    m.has_song = p.has_song.get();
    m.title = p.title.get();
    m.artist = p.artist.get();
    m.album = p.album.get();
    m.cover_art = p.cover_art.get();
    m.shuffle = p.shuffle.get();
    m.repeat = p.repeat_mode.get();
    m.repeat_count = p.repeat_count.get();
    m.repeat_plays = p.repeat_plays.get();
    m.rate = p.rate.get();
    m.queue_len = p.queue_len.get();
    m.current_index = p.current_index.get();
    m.queue_entries = p.queue_entries.get();
    m.roots = p.library_roots.get();
    m.playlists = p.playlists.get();
    m.scanning = p.scanning.get();
    m.scan_progress = p.scan_progress.get();
    m.position = p.position.get();
    m.duration = p.duration.get();
}

/// Widget updates the `#[watch]` macro can't express: the cover image,
/// the repeat/speed highlight, and the saved-playlist rows.
pub(super) fn apply_dynamic(
    widgets: &MtuneMenuWidgetModelWidgets,
    m: &MtuneMenuWidgetModel,
    _sender: &ComponentSender<MtuneMenuWidgetModel>,
) {
    if let Some(panel) = &m.radio_panel {
        panel.refresh(m);
    }
    if let Some(signal) = &m.volume_signal {
        widgets.volume.block_signal(signal);
    }
    widgets.volume.set_value(m.volume);
    if let Some(signal) = &m.volume_signal {
        widgets.volume.unblock_signal(signal);
    }
    // Cover
    match m.cover_art.as_deref() {
        Some(path) if !path.trim().is_empty() => widgets.cover.set_from_file(Some(path)),
        _ => widgets.cover.set_icon_name(Some("org.margo.Tune-symbolic")),
    }

    // Seek bar + time readouts
    let frac = if m.duration.is_zero() {
        0.0
    } else {
        (m.position.as_secs_f64() / m.duration.as_secs_f64()).clamp(0.0, 1.0)
    };
    match &m.seek_signal {
        Some(sig) => {
            widgets.seek.block_signal(sig);
            widgets.seek.set_value(frac);
            widgets.seek.unblock_signal(sig);
        }
        None => widgets.seek.set_value(frac),
    }
    widgets.elapsed.set_label(&format_duration(m.position));
    widgets.total.set_label(&format_duration(m.duration));

    // Repeat active state
    if m.repeat == "consecutive" {
        widgets.repeat_btn.remove_css_class("selected");
    } else {
        widgets.repeat_btn.add_css_class("selected");
    }

    // Speed preset highlight
    let mut child = widgets.speed_row.first_child();
    let mut i = 0;
    while let Some(w) = child {
        child = w.next_sibling();
        if let Some((rate, _)) = RATE_PRESETS.get(i) {
            if (m.rate - rate).abs() < 0.01 {
                w.add_css_class("selected");
            } else {
                w.remove_css_class("selected");
            }
        }
        i += 1;
    }
}

/// Wire the seek scale: immediate readout on drag, the real seek
/// debounced ~280ms after the last move. Returns the `value-changed`
/// handler id so programmatic updates can block it.
pub(super) fn setup_seek(
    scale: &gtk::Scale,
    sender: &ComponentSender<MtuneMenuWidgetModel>,
) -> gtk::glib::SignalHandlerId {
    let pending: Rc<Cell<Option<gtk::glib::SourceId>>> = Rc::new(Cell::new(None));
    let sender = sender.clone();
    scale.connect_value_changed(move |scale| {
        if let Some(id) = pending.take() {
            id.remove();
        }
        let value = scale.value();
        // Fallible send: `value-changed` also fires on a programmatic
        // `set_value` during teardown, where `input()` would panic.
        let _ = sender
            .input_sender()
            .send(MtuneMenuInput::SeekPreview(value));

        let s = sender.clone();
        let p = pending.clone();
        let id = gtk::glib::timeout_add_local_once(Duration::from_millis(280), move || {
            p.set(None);
            let _ = s.input_sender().send(MtuneMenuInput::Seek(value));
        });
        pending.set(Some(id));
    })
}

impl MtuneMenuWidgetModel {
    /// "3 of 240  ·  45:20 played  ·  12:34 left  ·  58:20 total  ·  1.5×" —
    /// position in the queue, time played/left/total across the *whole*
    /// queue (not just this track), and a non-default speed.
    pub(super) fn now_playing_meta(&self) -> String {
        if self.is_live {
            return format!("LIVE · {}", self.station_name);
        }
        let mut parts: Vec<String> = Vec::new();
        if self.queue_len > 0 {
            let n = if self.current_index >= 0 {
                self.current_index as u32 + 1
            } else {
                0
            };
            parts.push(format!("{n} of {}", self.queue_len));

            let (total, elapsed, remaining) = mtune_service().player.playlist_progress();
            if !total.is_zero() {
                parts.push(format!("{} played", format_duration(elapsed)));
                parts.push(format!("{} left", format_duration(remaining)));
                parts.push(format!("{} total", format_duration(total)));
            }
        }
        if (self.rate - 1.0).abs() >= 0.01 {
            parts.push(format!("{:.2}×", self.rate));
        }
        parts.join("  ·  ")
    }

    pub(super) fn library_status(&self) -> String {
        if !self.running {
            return "Tune isn't running.".into();
        }
        if self.scanning {
            let (done, total) = self.scan_progress;
            return if total > 0 {
                format!("Scanning… {done}/{total}")
            } else {
                "Scanning…".into()
            };
        }
        let roots = if self.roots.is_empty() {
            "no folder set".to_string()
        } else {
            self.roots.join(", ")
        };
        format!("{} tracks  ·  {roots}", self.queue_len)
    }
}

pub(super) fn rebuild_playlists(
    widgets: &MtuneMenuWidgetModelWidgets,
    m: &MtuneMenuWidgetModel,
    sender: &ComponentSender<MtuneMenuWidgetModel>,
) {
    // Saved-playlist rows
    while let Some(c) = widgets.playlists_list.first_child() {
        widgets.playlists_list.remove(&c);
    }
    for name in &m.playlists {
        let label = gtk::Label::new(Some(name));
        label.set_xalign(0.0);
        label.set_ellipsize(pango::EllipsizeMode::End);
        let row = gtk::Button::builder()
            .child(&label)
            .css_classes(["mtune-playlist-row"])
            .build();
        let name = name.clone();
        let s = sender.clone();
        row.connect_clicked(move |_| s.input(MtuneMenuInput::LoadPlaylist(name.clone())));
        widgets.playlists_list.append(&row);
    }
}
