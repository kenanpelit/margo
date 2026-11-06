// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

// Keep native decoder/plugin teardown out of other pipeline tests.
pub(in crate::audio) static GST_TEST_LOCK: Mutex<()> = Mutex::new(());

/// Endless, non-seekable radio responses, independent of public stations/network.
pub(in crate::audio) struct RadioFixture {
    address: std::net::SocketAddr,
    stop: Arc<AtomicBool>,
    server: Option<thread::JoinHandle<()>>,
    pub(in crate::audio) failures: Arc<std::sync::atomic::AtomicUsize>,
    pub(in crate::audio) requests: Arc<std::sync::atomic::AtomicUsize>,
}

impl RadioFixture {
    pub(in crate::audio) fn new() -> Self {
        gst::init().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let mut streams = Vec::new();
        for (name, encoder, rate, channels, mime) in [
            ("mp3", "lamemp3enc ! mpegaudioparse", 44100, 2, "audio/mpeg"),
            (
                "paradise",
                "avenc_aac ! aacparse ! audio/mpeg,stream-format=adts",
                44100,
                2,
                "audio/aac",
            ),
            (
                "lowmp3",
                "lamemp3enc target=bitrate bitrate=64 ! mpegaudioparse",
                22050,
                2,
                "audio/mpeg",
            ),
            (
                "aac",
                "avenc_aac ! aacparse ! audio/mpeg,stream-format=adts",
                48000,
                1,
                "audio/aac",
            ),
        ] {
            let path = dir.path().join(name);
            let pipeline = gst::parse::launch(&format!(
                "audiotestsrc num-buffers=200 ! audio/x-raw,rate={rate},channels={channels} ! audioconvert ! {encoder} ! filesink location={}",
                path.display()
            )).unwrap().downcast::<gst::Pipeline>().unwrap();
            pipeline.set_state(gst::State::Playing).unwrap();
            let msg = pipeline
                .bus()
                .unwrap()
                .timed_pop_filtered(
                    gst::ClockTime::from_seconds(10),
                    &[gst::MessageType::Eos, gst::MessageType::Error],
                )
                .expect("fixture encoding timed out");
            assert!(matches!(msg.view(), gst::MessageView::Eos(_)), "{msg:?}");
            pipeline.set_state(gst::State::Null).unwrap();
            streams.push((name, mime, std::fs::read(path).unwrap()));
        }
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let shutdown = stop.clone();
        let failures = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let requests = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let remaining = failures.clone();
        let observed = requests.clone();
        let server = thread::spawn(move || {
            let mut clients = Vec::new();
            while !shutdown.load(Ordering::Relaxed) {
                let Ok((mut socket, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                };
                let streams = streams.clone();
                let stop = shutdown.clone();
                let remaining = remaining.clone();
                let observed = observed.clone();
                clients.push(thread::spawn(move || {
                    socket
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    socket
                        .set_write_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let mut request = [0; 4096];
                    let Ok(n) = socket.read(&mut request) else {
                        return;
                    };
                    let request = String::from_utf8_lossy(&request[..n]);
                    let transient = request.starts_with("GET /transient ");
                    if transient {
                        observed.fetch_add(1, Ordering::Relaxed);
                    }
                    if request.starts_with("GET /broken ")
                        || (transient
                            && remaining
                                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                                    n.checked_sub(1)
                                })
                                .is_ok())
                    {
                        let _ = socket
                            .write_all(b"HTTP/1.0 403 Forbidden\r\nContent-Length: 0\r\n\r\n");
                        return;
                    }
                    let (_, mime, bytes) = streams
                        .iter()
                        .find(|(name, _, _)| {
                            (transient && *name == "mp3")
                                || request.starts_with(&format!("GET /{name} "))
                        })
                        .unwrap();
                    if write!(
                        socket,
                        "HTTP/1.0 200 OK\r\nContent-Type: {mime}\r\nConnection: close\r\n\r\n"
                    )
                    .is_err()
                    {
                        return;
                    }
                    while !stop.load(Ordering::Relaxed) {
                        for chunk in bytes.chunks(1024) {
                            if stop.load(Ordering::Relaxed) || socket.write_all(chunk).is_err() {
                                return;
                            }
                            thread::sleep(Duration::from_millis(20));
                        }
                    }
                }));
            }
            for client in clients {
                client.join().unwrap();
            }
        });
        Self {
            address,
            stop,
            server: Some(server),
            failures,
            requests,
        }
    }

    pub(in crate::audio) fn uri(&self, name: &str) -> String {
        format!("http://{}/{name}", self.address)
    }
}

impl Drop for RadioFixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.server.take().unwrap().join().unwrap();
    }
}

// GstPlay::stop is asynchronous. Keep its owner alive until the native
// control thread has completed stop, before tearing down the HTTP server.
pub(in crate::audio) fn stop_player(play: &gst_play::Play) {
    if play.pipeline().current_state() == gst::State::Null {
        return;
    }
    let (sender, receiver) = std::sync::mpsc::channel();
    play.message_bus().set_sync_handler(move |_, msg| {
        if let Ok(gst_play::PlayMessage::StateChanged(state)) = gst_play::PlayMessage::parse(msg)
            && state.state() == gst_play::PlayState::Stopped
        {
            let _ = sender.send(());
        }
        gst::BusSyncReply::Drop
    });
    play.stop();
    receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("GstPlay did not finish stopping");
}

#[test]
fn radio_switches_between_codecs_rates_and_channels() {
    let _guard = GST_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let fixture = RadioFixture::new();
    let (sender, receiver) = async_channel::unbounded();
    let backend = GstBackend::new(sender);
    let sink = gst::ElementFactory::make("fakesink")
        .property("sync", true)
        .property("signal-handoffs", true)
        .build()
        .unwrap();
    let received = Arc::new(Mutex::new(Vec::new()));
    let samples = received.clone();
    sink.connect("handoff", false, move |values| {
        let pad = values[2].get::<gst::Pad>().unwrap();
        let caps = pad.current_caps().unwrap();
        let s = caps.structure(0).unwrap();
        samples.lock().unwrap().push((
            s.get::<i32>("rate").unwrap(),
            s.get::<i32>("channels").unwrap(),
        ));
        None
    });
    backend
        .gst_player
        .pipeline()
        .set_property("audio-sink", &sink);
    for (index, (name, expected)) in [
        ("mp3", (44100, 2)),
        ("aac", (48000, 1)),
        ("mp3", (44100, 2)),
        ("aac", (48000, 1)),
    ]
    .into_iter()
    .enumerate()
    {
        if index > 0 {
            backend.pause();
        }
        backend.set_song_uri(Some(&fixture.uri(name)));
        received.lock().unwrap().clear();
        backend.play();
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            while let Ok(action) = receiver.try_recv() {
                assert!(
                    !matches!(action, PlaybackAction::StreamFailed { .. }),
                    "switch to {name}: {action:?}"
                );
            }
            if received
                .lock()
                .unwrap()
                .iter()
                .filter(|&&caps| caps == expected)
                .count()
                >= 5
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "station {name} stalled: {:?}",
                received.lock().unwrap()
            );
            thread::sleep(Duration::from_millis(20));
        }
    }
    stop_player(&backend.gst_player);
}
