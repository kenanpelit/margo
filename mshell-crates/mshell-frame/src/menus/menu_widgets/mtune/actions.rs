use super::*;

pub(super) fn handle(
    model: &mut MtuneMenuWidgetModel,
    widgets: &mut MtuneMenuWidgetModelWidgets,
    message: MtuneMenuInput,
    sender: &ComponentSender<MtuneMenuWidgetModel>,
) {
    match message {
        MtuneMenuInput::Tab(tab) => {
            model.tab = tab;
            if let Some(track) = widgets.tabs_host.first_child().and_downcast::<gtk::Box>() {
                let mut child = track.first_child();
                let mut i = 0;
                while let Some(button) = child {
                    button.set_css_classes(crate::common_widgets::segmented::segment_classes(
                        SegmentedStyle::Capsule,
                        i == tab,
                    ));
                    child = button.next_sibling();
                    i += 1;
                }
            }
        }
        MtuneMenuInput::SetVolume(value) => {
            model.volume = value;
            tokio_rt_spawn(async move {
                mtune_service().player.set_volume(value).await;
            });
        }
        MtuneMenuInput::SearchRadio(query, country) => radio::search(model, query, country, sender),
        MtuneMenuInput::RadioAction(action) => radio::act(model, action, sender),
        MtuneMenuInput::SeekPreview(value) => {
            if model.is_live {
                return;
            }
            // Immediate feedback; the real seek is debounced.
            let pos = model.duration.mul_f64(value.clamp(0.0, 1.0));
            model.pending_seek = Some((pos, Instant::now()));
            model.position = pos;
            widgets.elapsed.set_label(&format_duration(pos));
            if let Some(sig) = &model.seek_signal {
                widgets.seek.block_signal(sig);
                widgets.seek.set_value(value);
                widgets.seek.unblock_signal(sig);
            }
        }
        MtuneMenuInput::Seek(value) => {
            if model.is_live {
                return;
            }
            let secs = model.duration.mul_f64(value.clamp(0.0, 1.0)).as_secs();
            tokio_rt_spawn(async move { mtune_service().player.seek(secs).await });
        }
        MtuneMenuInput::PlayPause => {
            tokio_rt_spawn(async { mtune_service().player.play_pause().await });
        }
        MtuneMenuInput::Next => {
            tokio_rt_spawn(async { mtune_service().player.next().await });
        }
        MtuneMenuInput::Previous => {
            tokio_rt_spawn(async { mtune_service().player.previous().await });
        }
        MtuneMenuInput::ToggleShuffle => {
            let on = !model.shuffle;
            tokio_rt_spawn(async move { mtune_service().player.set_shuffle(on).await });
        }
        MtuneMenuInput::CycleRepeat => {
            let next = match model.repeat.as_str() {
                "consecutive" => "repeat-all",
                "repeat-all" => "repeat-one",
                "repeat-one" => "repeat-each",
                _ => "consecutive",
            };
            tokio_rt_spawn(async move { mtune_service().player.set_repeat_mode(next).await });
        }
        MtuneMenuInput::AdjustRepeatCount(delta) => {
            let next = (model.repeat_count as i32 + delta).clamp(1, 20) as u32;
            tokio_rt_spawn(async move { mtune_service().player.set_repeat_count(next).await });
        }
        MtuneMenuInput::SetRate(r) => {
            tokio_rt_spawn(async move { mtune_service().player.set_rate(r).await });
        }
        MtuneMenuInput::LoadPlaylist(name) => {
            tokio_rt_spawn(async move { mtune_service().player.load_playlist(&name).await });
        }
        MtuneMenuInput::OpenPlaylist => {
            // Parent must be `None`: a layer-shell menu surface has no
            // xdg_toplevel, so handing it to the file-chooser as a
            // parent aborts GTK (crashing the shell).
            let dialog = gtk::FileDialog::builder()
                .title("Open a playlist")
                .modal(true)
                .build();
            dialog.open(gtk::Window::NONE, gtk::gio::Cancellable::NONE, move |res| {
                if let Ok(file) = res
                    && let Some(path) = file.path()
                {
                    let path = path.to_string_lossy().into_owned();
                    tokio_rt_spawn(async move {
                        let svc = mtune_service().player.clone();
                        if !svc.running.get() {
                            spawn_mtune();
                        }
                        svc.open_playlist(&path).await;
                    });
                }
            });
        }
        MtuneMenuInput::Rescan => {
            tokio_rt_spawn(async { mtune_service().player.rescan_library().await });
        }
        MtuneMenuInput::OpenTune => {
            tokio_rt_spawn(async { mtune_service().player.raise().await });
        }
        MtuneMenuInput::Launch => spawn_mtune(),
        MtuneMenuInput::ChooseFolder => {
            let dialog = gtk::FileDialog::builder()
                .title("Choose a music folder")
                .modal(true)
                .build();
            dialog.select_folder(gtk::Window::NONE, gtk::gio::Cancellable::NONE, move |res| {
                if let Ok(folder) = res
                    && let Some(path) = folder.path()
                {
                    let path = path.to_string_lossy().into_owned();
                    tokio_rt_spawn(async move {
                        let svc = mtune_service().player.clone();
                        if !svc.running.get() {
                            spawn_mtune();
                        }
                        svc.set_library_roots(vec![path.clone()]).await;
                        svc.play_folder(&path).await;
                    });
                }
            });
        }
        MtuneMenuInput::FilterQueue(text) => {
            model.queue_filter = text;
            rebuild_queue_rows(widgets, model, sender);
        }
        MtuneMenuInput::PlayQueueIndex(i) => {
            tokio_rt_spawn(async move { mtune_service().player.play_index(i).await });
        }
        MtuneMenuInput::RemoveQueueIndex(i) => {
            tokio_rt_spawn(async move { mtune_service().player.remove_index(i).await });
        }
    }
}
