//! Tests for the speed-test card's event reducer (`SpeedTestModel::apply`).

use super::*;

fn model() -> SpeedTestModel {
    SpeedTestModel {
        backend: Some(Some(Backend::Ookla)),
        probed: true,
        running: None,
        phase: Phase::Connecting,
        live_mbps: 0.0,
        peak_mbps: 0.0,
        ping_ms: None,
        jitter_ms: None,
        server: String::new(),
        result: None,
        error: None,
        gauge: Rc::new(Cell::new(0.0)),
    }
}

fn live(phase: Phase, mbps: f64) -> SpeedEvent {
    SpeedEvent::Live {
        phase,
        mbps,
        progress: None,
    }
}

#[test]
fn a_run_walks_ping_download_upload_done() {
    let mut m = model();
    m.apply(SpeedEvent::Started {
        server: "indasys IT Systemhaus AG".into(),
        isp: "Türk Telekom".into(),
    });
    assert_eq!(m.server, "indasys IT Systemhaus AG · Türk Telekom");
    m.apply(SpeedEvent::Ping {
        latency_ms: 12.5,
        jitter_ms: Some(1.2),
    });
    assert_eq!(
        (m.phase, m.ping_ms, m.jitter_ms),
        (Phase::Ping, Some(12.5), Some(1.2))
    );

    m.apply(live(Phase::Download, 80.0));
    m.apply(live(Phase::Download, 240.0));
    m.apply(live(Phase::Download, 200.0));
    assert_eq!((m.live_mbps, m.peak_mbps), (200.0, 240.0), "peak is kept");
    // A new direction restarts the gauge scale.
    m.apply(live(Phase::Upload, 20.0));
    assert_eq!((m.phase, m.peak_mbps), (Phase::Upload, 20.0));

    m.apply(SpeedEvent::Done(SpeedResult {
        download_mbps: Some(231.4),
        server_name: "other".into(),
        ..Default::default()
    }));
    assert!(m.result.is_some());
    assert_eq!(
        m.server, "indasys IT Systemhaus AG · Türk Telekom",
        "the live server line is not replaced"
    );
    assert!(m.running.is_none());
}

#[test]
fn done_without_start_takes_the_server_from_the_result() {
    let mut m = model();
    m.apply(SpeedEvent::Started {
        server: String::new(),
        isp: String::new(),
    });
    assert_eq!(m.server, "", "no stray separator");
    m.apply(SpeedEvent::Done(SpeedResult {
        server_name: "Vodafone".into(),
        server_location: "Istanbul".into(),
        isp: "Superonline".into(),
        ..Default::default()
    }));
    assert_eq!(m.server, "Vodafone · Istanbul · Superonline");
}

#[test]
fn failure_keeps_the_error_and_reset_clears_it() {
    let mut m = model();
    m.apply(live(Phase::Download, 50.0));
    m.apply(SpeedEvent::Failed("speedtest exited with 1".into()));
    assert_eq!(m.error.as_deref(), Some("speedtest exited with 1"));
    m.reset_run();
    assert_eq!(m.phase, Phase::Connecting);
    assert_eq!((m.live_mbps, m.peak_mbps), (0.0, 0.0));
    assert!(m.error.is_none() && m.result.is_none() && m.server.is_empty());
}
