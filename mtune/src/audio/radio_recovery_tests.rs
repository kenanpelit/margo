// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::audio::gst_backend::tests::{GST_TEST_LOCK, RadioFixture, stop_player};
use gst::prelude::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};

fn pump(player: &AudioPlayer) {
    let context = glib::MainContext::default();
    while context.pending() {
        context.iteration(false);
    }
    while let Ok(action) = player.receiver.borrow().as_ref().unwrap().try_recv() {
        player.process_action(action);
    }
    std::thread::sleep(Duration::from_millis(10));
}
fn until(player: &AudioPlayer, seconds: u64, predicate: impl Fn() -> bool) {
    let end = Instant::now() + Duration::from_secs(seconds);
    while !predicate() {
        pump(player);
        assert!(
            Instant::now() < end,
            "radio stalled: playing={} error={}",
            player.state.playing(),
            player.state.stream_error()
        );
    }
}
fn setup(fixture: &RadioFixture) -> (AudioPlayer, Arc<AtomicUsize>) {
    setup_kind(fixture, StreamKind::Radio)
}
fn setup_kind(fixture: &RadioFixture, kind: StreamKind) -> (AudioPlayer, Arc<AtomicUsize>) {
    let player = test_player();
    let sink = gst::ElementFactory::make("fakesink")
        .property("sync", true)
        .property("signal-handoffs", true)
        .build()
        .unwrap();
    let buffers = Arc::new(AtomicUsize::new(0));
    let count = buffers.clone();
    sink.connect("handoff", false, move |_| {
        count.fetch_add(1, Ordering::Relaxed);
        None
    });
    player
        .backend
        .test_player()
        .pipeline()
        .set_property("audio-sink", &sink);
    for name in ["transient", "aac"] {
        player.queue.add_song(&Song::from_stream(StreamInfo {
            kind,
            key: name.into(),
            uri: fixture.uri(name),
            ..StreamInfo::default()
        }));
    }
    player.skip_to(0);
    (player, buffers)
}
#[test]
fn a_live_eos_does_not_silently_advance_to_another_station() {
    let _lock = GST_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let fixture = RadioFixture::new();
    let (player, buffers) = setup(&fixture);
    player.play();
    until(&player, 5, || buffers.load(Ordering::Relaxed) > 10);
    player.play_next();
    assert_eq!(player.queue.current_song_index(), Some(0));
    assert!(!player.state.playing());
    assert!(!player.state.stream_error().is_empty());
    player.stop();
    stop_player(player.backend.test_player());
}

#[test]
fn files_and_podcast_errors_are_not_automatically_retried() {
    let _lock = GST_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    for kind in [StreamKind::File, StreamKind::Episode] {
        let fixture = RadioFixture::new();
        fixture.failures.store(1, Ordering::Relaxed);
        let (player, buffers) = setup_kind(&fixture, kind);
        player.play();
        until(&player, 5, || !player.play_requested.get());
        let end = Instant::now() + Duration::from_secs(2);
        while Instant::now() < end {
            pump(&player);
        }
        assert_eq!(fixture.requests.load(Ordering::Relaxed), 1);
        assert_eq!(buffers.load(Ordering::Relaxed), 0);
        assert_eq!(player.retry_count.get(), 0);
        assert!(!player.state.playing());
        stop_player(player.backend.test_player());
    }
}

#[test]
fn temporary_http_failure_reconnects_and_clears_error_after_audio() {
    let _lock = GST_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let fixture = RadioFixture::new();
    // A transient denial is exposed to GstPlay (unlike Soup's built-in 500 retries).
    fixture.failures.store(1, Ordering::Relaxed);
    let (player, buffers) = setup(&fixture);
    player.play();
    until(&player, 5, || !player.state.stream_error().is_empty());
    assert!(
        !player.state.playing(),
        "fatal error falsely advertises Playing"
    );
    until(&player, 12, || {
        buffers.load(Ordering::Relaxed) >= 10 && player.state.stream_error().is_empty()
    });
    assert!(player.state.playing());
    assert_eq!(fixture.requests.load(Ordering::Relaxed), 2);
    assert_eq!(player.queue.current_song_index(), Some(0));
    stop_player(player.backend.test_player());
}

#[test]
fn permanently_failed_radio_exhausts_three_retries_without_advancing() {
    let _lock = GST_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let fixture = RadioFixture::new();
    fixture.failures.store(100, Ordering::Relaxed);
    let (player, buffers) = setup(&fixture);
    player.play();
    until(&player, 14, || {
        fixture.requests.load(Ordering::Relaxed) == 4 && !player.play_requested.get()
    });
    let end = Instant::now() + Duration::from_secs(2);
    while Instant::now() < end {
        pump(&player);
    }
    assert_eq!(fixture.requests.load(Ordering::Relaxed), 4);
    assert_eq!(buffers.load(Ordering::Relaxed), 0);
    assert_eq!(player.retry_count.get(), 3);
    assert!(!player.state.playing());
    assert!(!player.state.stream_error().is_empty());
    assert_eq!(player.queue.current_song_index(), Some(0));
    stop_player(player.backend.test_player());
}

#[test]
fn delayed_radio_retry_is_cancelled_by_stop_pause_or_station_selection() {
    let _lock = GST_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    for command in ["stop", "pause", "selection"] {
        let fixture = RadioFixture::new();
        fixture.failures.store(1, Ordering::Relaxed);
        let (player, buffers) = setup(&fixture);
        player.play();
        until(&player, 5, || !player.state.stream_error().is_empty());
        let generation = player.retry_generation.get();
        assert_eq!(
            player.retry_count.get(),
            1,
            "test must cancel an armed retry"
        );
        match command {
            "stop" => player.stop(),
            "pause" => player.pause(),
            _ => {
                player.skip_to(1);
                player.play();
            }
        }
        // Also deliver an already-enqueued timeout after the command.
        player.process_action(PlaybackAction::RetryRadio(generation));
        let end = Instant::now() + Duration::from_secs(2);
        while Instant::now() < end {
            pump(&player);
        }
        assert_eq!(
            fixture.requests.load(Ordering::Relaxed),
            1,
            "{command} restarted failed station"
        );
        if command == "selection" {
            assert_eq!(player.queue.current_song_index(), Some(1));
            assert!(buffers.load(Ordering::Relaxed) > 10);
            assert!(player.state.stream_error().is_empty());
            assert!(player.state.playing());
            // Old failure / ready events cannot stop or clear another station.
            player.process_action(PlaybackAction::StreamFailed {
                uri: fixture.uri("transient"),
                message: "stale failure".into(),
            });
            assert!(player.state.playing());
            assert!(player.state.stream_error().is_empty());
        } else {
            assert_eq!(buffers.load(Ordering::Relaxed), 0);
            assert!(!player.state.playing());
            player.process_action(PlaybackAction::StreamPlaying(fixture.uri("transient")));
            assert!(
                !player.state.playing(),
                "late Playing ignored after {command}"
            );
        }
        stop_player(player.backend.test_player());
    }
}
