//! Internet speed test for the Network menu: drives the Ookla
//! `speedtest` CLI (streams JSON progress lines — a truly live gauge with
//! ping, jitter, packet loss) or, as a fallback, the Python
//! `speedtest-cli` (one JSON blob at the end; the live gauge is fed by
//! sampling `/proc/net/dev` while it runs).
//!
//! On Arch the `speedtest-cli` package also installs a `speedtest`
//! entry point, so a binary named `speedtest` is only trusted as Ookla
//! when its `--version` says so ([`backend_from_version`]).
//!
//! Every parser here is pure and unit-tested; [`run`] only spawns the
//! tool and forwards [`SpeedEvent`]s over a channel.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc::UnboundedSender;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// Ookla `speedtest` (JSON lines with progress).
    Ookla,
    /// Python `speedtest-cli` (final JSON only).
    Legacy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Phase {
    #[default]
    Connecting,
    Ping,
    Download,
    Upload,
}

impl Phase {
    pub fn label(self) -> &'static str {
        match self {
            Phase::Connecting => "Connecting…",
            Phase::Ping => "Ping",
            Phase::Download => "Download",
            Phase::Upload => "Upload",
        }
    }
}

/// Normalised final result (both backends map into this).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SpeedResult {
    pub download_mbps: Option<f64>,
    pub upload_mbps: Option<f64>,
    pub ping_ms: Option<f64>,
    pub jitter_ms: Option<f64>,
    pub packet_loss: Option<f64>,
    pub server_name: String,
    pub server_location: String,
    pub isp: String,
    pub external_ip: String,
    pub result_url: String,
}

impl SpeedResult {
    /// `indasys IT Systemhaus AG · Stuttgart, Germany`.
    pub fn server_line(&self) -> String {
        [self.server_name.as_str(), self.server_location.as_str()]
            .iter()
            .filter(|s| !s.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join(" · ")
    }
}

/// Streamed while a test runs.
#[derive(Debug, Clone, PartialEq)]
pub enum SpeedEvent {
    /// Server / ISP known (Ookla `testStart`).
    Started {
        server: String,
        isp: String,
    },
    /// Live gauge sample: current phase, Mbit/s, and 0..=1 phase progress
    /// (`None` when the backend can't tell).
    Live {
        phase: Phase,
        mbps: f64,
        progress: Option<f64>,
    },
    /// Live ping/jitter during the ping phase.
    Ping {
        latency_ms: f64,
        jitter_ms: Option<f64>,
    },
    Done(SpeedResult),
    Failed(String),
}

/// `speedtest --version` output → which backend that binary is.
pub fn backend_from_version(version_output: &str) -> Backend {
    if version_output.contains("Ookla") {
        Backend::Ookla
    } else {
        Backend::Legacy
    }
}

/// Bytes/s → Mbit/s (Ookla reports `bandwidth` in bytes per second).
pub fn bytes_per_sec_to_mbps(bps: f64) -> f64 {
    bps * 8.0 / 1_000_000.0
}

// ── Ookla JSON lines ───────────────────────────────────────────────────────

#[derive(Deserialize, Default)]
#[serde(default)]
struct OoklaPing {
    latency: Option<f64>,
    jitter: Option<f64>,
    progress: Option<f64>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct OoklaTransfer {
    bandwidth: Option<f64>,
    progress: Option<f64>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct OoklaServer {
    name: String,
    location: String,
    country: String,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct OoklaIface {
    external_ip: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct OoklaResultLink {
    url: String,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct OoklaLine {
    #[serde(rename = "type")]
    kind: String,
    ping: Option<OoklaPing>,
    download: Option<OoklaTransfer>,
    upload: Option<OoklaTransfer>,
    packet_loss: Option<f64>,
    isp: String,
    server: Option<OoklaServer>,
    interface: Option<OoklaIface>,
    result: Option<OoklaResultLink>,
    error: Option<String>,
    message: Option<String>,
}

fn location(city: &str, country: &str) -> String {
    match (city.is_empty(), country.is_empty()) {
        (false, false) => format!("{city}, {country}"),
        (false, true) => city.to_string(),
        (true, false) => country.to_string(),
        (true, true) => String::new(),
    }
}

/// One `speedtest --format=jsonl --progress=yes` line → event.
pub fn parse_ookla_line(line: &str) -> Option<SpeedEvent> {
    let l: OoklaLine = serde_json::from_str(line.trim()).ok()?;
    match l.kind.as_str() {
        "testStart" => {
            let s = l.server.unwrap_or_default();
            Some(SpeedEvent::Started {
                server: [s.name, location(&s.location, &s.country)]
                    .into_iter()
                    .filter(|x| !x.is_empty())
                    .collect::<Vec<_>>()
                    .join(" · "),
                isp: l.isp,
            })
        }
        "ping" => {
            let p = l.ping?;
            Some(SpeedEvent::Ping {
                latency_ms: p.latency?,
                jitter_ms: p.jitter,
            })
        }
        "download" | "upload" => {
            let (phase, t) = if l.kind == "download" {
                (Phase::Download, l.download?)
            } else {
                (Phase::Upload, l.upload?)
            };
            Some(SpeedEvent::Live {
                phase,
                mbps: bytes_per_sec_to_mbps(t.bandwidth.unwrap_or(0.0)),
                progress: t.progress,
            })
        }
        "result" => {
            let s = l.server.unwrap_or_default();
            let ping = l.ping.unwrap_or_default();
            Some(SpeedEvent::Done(SpeedResult {
                download_mbps: l
                    .download
                    .and_then(|d| d.bandwidth)
                    .map(bytes_per_sec_to_mbps),
                upload_mbps: l
                    .upload
                    .and_then(|u| u.bandwidth)
                    .map(bytes_per_sec_to_mbps),
                ping_ms: ping.latency,
                jitter_ms: ping.jitter,
                packet_loss: l.packet_loss,
                server_name: s.name,
                server_location: location(&s.location, &s.country),
                isp: l.isp,
                external_ip: l.interface.unwrap_or_default().external_ip,
                result_url: l.result.unwrap_or_default().url,
            }))
        }
        "log" | "error" => {
            let msg = l.error.or(l.message)?;
            (l.kind == "error").then_some(SpeedEvent::Failed(msg))
        }
        _ => None,
    }
}

// ── speedtest-cli --json ───────────────────────────────────────────────────

#[derive(Deserialize, Default)]
#[serde(default)]
struct LegacyServer {
    name: String,
    country: String,
    sponsor: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct LegacyClient {
    ip: String,
    isp: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct LegacyJson {
    download: f64,
    upload: f64,
    ping: f64,
    server: LegacyServer,
    client: LegacyClient,
    share: Option<String>,
}

/// `speedtest-cli --json` output. Download/upload are already in bit/s;
/// a skipped direction (`--no-upload`) reports `0` and maps to `None`.
/// No jitter / packet loss in this backend.
pub fn parse_legacy_json(s: &str) -> Option<SpeedResult> {
    let j: LegacyJson = serde_json::from_str(s.trim()).ok()?;
    let mbps = |bits: f64| (bits > 0.0).then(|| bits / 1_000_000.0);
    Some(SpeedResult {
        download_mbps: mbps(j.download),
        upload_mbps: mbps(j.upload),
        ping_ms: (j.ping > 0.0).then_some(j.ping),
        jitter_ms: None,
        packet_loss: None,
        server_name: j.server.sponsor,
        server_location: location(&j.server.name, &j.server.country),
        isp: j.client.isp,
        external_ip: j.client.ip,
        result_url: j.share.unwrap_or_default(),
    })
}

// ── Live gauge for the legacy backend ──────────────────────────────────────

/// `/proc/net/dev` → per-interface (rx, tx) byte counters, `lo` excluded.
pub fn parse_proc_net_dev(s: &str) -> BTreeMap<String, (u64, u64)> {
    let mut map = BTreeMap::new();
    for line in s.lines().skip(2) {
        let Some((name, rest)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim();
        if name == "lo" {
            continue;
        }
        let f: Vec<u64> = rest
            .split_whitespace()
            .filter_map(|x| x.parse().ok())
            .collect();
        if f.len() >= 9 {
            map.insert(name.to_string(), (f[0], f[8]));
        }
    }
    map
}

/// Live (rx, tx) bytes/s between two snapshots: the busiest interface per
/// direction, not the sum — with a VPN up the same bytes cross both the
/// tunnel and the physical link, so summing would double the reading.
pub fn busiest_rates(
    prev: &BTreeMap<String, (u64, u64)>,
    cur: &BTreeMap<String, (u64, u64)>,
    secs: f64,
) -> (f64, f64) {
    if secs <= 0.0 {
        return (0.0, 0.0);
    }
    let mut rx: f64 = 0.0;
    let mut tx: f64 = 0.0;
    for (name, &(r, t)) in cur {
        if let Some(&(pr, pt)) = prev.get(name) {
            rx = rx.max(r.saturating_sub(pr) as f64 / secs);
            tx = tx.max(t.saturating_sub(pt) as f64 / secs);
        }
    }
    (rx, tx)
}

/// Which phase the legacy test is in, judged by the dominant direction.
/// Below ~1 Mbit/s both ways it is still pinging / picking a server.
pub fn infer_phase(rx_bps: f64, tx_bps: f64) -> Phase {
    const FLOOR: f64 = 125_000.0; // 1 Mbit/s in bytes/s
    if rx_bps < FLOOR && tx_bps < FLOOR {
        Phase::Ping
    } else if tx_bps > rx_bps {
        Phase::Upload
    } else {
        Phase::Download
    }
}

/// Gauge full-scale (Mbit/s): the smallest step that fits `peak`, so a
/// 20 Mbit/s line doesn't crawl along the bottom of a 1 Gbit/s dial.
pub fn gauge_scale(peak_mbps: f64) -> f64 {
    const STEPS: [f64; 8] = [10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1000.0, 2500.0];
    STEPS
        .iter()
        .copied()
        .find(|s| peak_mbps <= *s)
        .unwrap_or(10_000.0)
}

/// `93.8 Mbit/s` / `940 Mbit/s` / `1.25 Gbit/s`.
pub fn format_mbps(mbps: f64) -> String {
    if mbps >= 1000.0 {
        format!("{:.2} Gbit/s", mbps / 1000.0)
    } else if mbps >= 100.0 {
        format!("{mbps:.0} Mbit/s")
    } else {
        format!("{mbps:.1} Mbit/s")
    }
}

// ── Runner ─────────────────────────────────────────────────────────────────

/// The backend available on this machine, or `None` (with neither
/// installed the menu shows an install hint: Arch `speedtest-cli`, or
/// the AUR `ookla-speedtest-bin`).
pub async fn detect_backend() -> Option<Backend> {
    if crate::tools::on_path("speedtest")
        && let Ok(out) = Command::new("speedtest")
            .arg("--version")
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output()
            .await
        && backend_from_version(&String::from_utf8_lossy(&out.stdout)) == Backend::Ookla
    {
        return Some(Backend::Ookla);
    }
    crate::tools::on_path("speedtest-cli").then_some(Backend::Legacy)
}

/// Hard cap on one test (a stalled server must not leave the menu
/// "running" forever).
const RUN_TIMEOUT: Duration = Duration::from_secs(120);
const LIVE_INTERVAL: Duration = Duration::from_millis(500);

/// Run one test, streaming events into `tx`; always ends with `Done` or
/// `Failed`. Dropping the future (menu closed / cancelled) kills the child.
pub async fn run(backend: Backend, tx: UnboundedSender<SpeedEvent>) {
    let res = tokio::time::timeout(RUN_TIMEOUT, async {
        match backend {
            Backend::Ookla => run_ookla(&tx).await,
            Backend::Legacy => run_legacy(&tx).await,
        }
    })
    .await;
    let outcome = match res {
        Ok(r) => r,
        Err(_) => Err("Speed test timed out".to_string()),
    };
    match outcome {
        Ok(result) => {
            let _ = tx.send(SpeedEvent::Done(result));
        }
        Err(e) => {
            let _ = tx.send(SpeedEvent::Failed(e));
        }
    }
}

async fn run_ookla(tx: &UnboundedSender<SpeedEvent>) -> Result<SpeedResult, String> {
    // stdbuf forces line buffering so progress lines arrive live, not in
    // one burst at exit.
    let args = [
        "--accept-license",
        "--accept-gdpr",
        "--format=jsonl",
        "--progress=yes",
    ];
    let mut cmd = if crate::tools::on_path("stdbuf") {
        let mut c = Command::new("stdbuf");
        c.arg("-oL").arg("speedtest").args(args);
        c
    } else {
        let mut c = Command::new("speedtest");
        c.args(args);
        c
    };
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("speedtest: {e}"))?;
    let stdout = child.stdout.take().ok_or("speedtest: no stdout")?;
    let mut lines = BufReader::new(stdout).lines();
    let mut result = None;
    let mut failure = None;
    while let Ok(Some(line)) = lines.next_line().await {
        match parse_ookla_line(&line) {
            Some(SpeedEvent::Done(r)) => result = Some(r),
            Some(SpeedEvent::Failed(e)) => failure = Some(e),
            Some(ev) => {
                let _ = tx.send(ev);
            }
            None => {}
        }
    }
    let out = child.wait_with_output().await.map_err(|e| e.to_string())?;
    result.ok_or_else(|| {
        failure.unwrap_or_else(|| {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            if err.is_empty() {
                "speedtest returned no result".to_string()
            } else {
                err
            }
        })
    })
}

async fn run_legacy(tx: &UnboundedSender<SpeedEvent>) -> Result<SpeedResult, String> {
    let child = Command::new("speedtest-cli")
        .args(["--json", "--secure"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("speedtest-cli: {e}"))?;
    let output = child.wait_with_output();
    tokio::pin!(output);
    let mut prev = read_net_dev().await;
    let mut prev_at = Instant::now();
    loop {
        tokio::select! {
            out = &mut output => {
                let out = out.map_err(|e| e.to_string())?;
                if !out.status.success() {
                    let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
                    return Err(if err.is_empty() { format!("speedtest-cli exited with {}", out.status) } else { last_line(&err) });
                }
                return parse_legacy_json(&String::from_utf8_lossy(&out.stdout))
                    .ok_or_else(|| "speedtest-cli returned unreadable output".to_string());
            }
            _ = tokio::time::sleep(LIVE_INTERVAL) => {
                let cur = read_net_dev().await;
                let now = Instant::now();
                let (rx, txb) = busiest_rates(&prev, &cur, now.duration_since(prev_at).as_secs_f64());
                prev = cur;
                prev_at = now;
                let phase = infer_phase(rx, txb);
                let mbps = match phase {
                    Phase::Upload => bytes_per_sec_to_mbps(txb),
                    Phase::Download => bytes_per_sec_to_mbps(rx),
                    _ => 0.0,
                };
                let _ = tx.send(SpeedEvent::Live { phase, mbps, progress: None });
            }
        }
    }
}

fn last_line(s: &str) -> String {
    s.lines().last().unwrap_or(s).trim().to_string()
}

async fn read_net_dev() -> BTreeMap<String, (u64, u64)> {
    tokio::fs::read_to_string("/proc/net/dev")
        .await
        .map(|s| parse_proc_net_dev(&s))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real `speedtest-cli --json --secure --no-upload --single` run on the
    // owner's machine over Mullvad (client IP / coordinates anonymised).
    const LEGACY: &str = r#"{"download": 22948706.387221366, "upload": 0, "ping": 66.679, "server": {"url": "http://speedtest.indasys.cloud:8080/speedtest/upload.php", "lat": "48.7758", "lon": "9.1829", "name": "Stuttgart", "country": "Germany", "cc": "DE", "sponsor": "indasys IT Systemhaus AG", "id": "50623", "host": "speedtest.indasys.cloud:8080", "d": 153.10872089605095, "latency": 66.679}, "timestamp": "2026-11-04T16:51:25.143227Z", "bytes_sent": 0, "bytes_received": 28846115, "share": null, "client": {"ip": "203.0.113.7", "lat": "0", "lon": "0", "isp": "31173 Services AB", "isprating": "3.7", "rating": "0", "ispdlavg": "0", "ispulavg": "0", "loggedin": "0", "country": "DE"}}"#;

    // Ookla CLI 1.2 `--format=jsonl --progress=yes` lines. The Ookla CLI
    // isn't installed on the owner's machine, so these follow Ookla's
    // documented schema (bandwidth in bytes/s) rather than a capture.
    const OOKLA_START: &str = r#"{"type":"testStart","timestamp":"2026-11-04T17:00:00Z","isp":"Mullvad","interface":{"internalIp":"10.64.0.2","name":"wg0-mullvad","isVpn":true,"externalIp":"203.0.113.7"},"server":{"id":50623,"host":"speedtest.indasys.cloud","port":8080,"name":"indasys IT Systemhaus AG","location":"Stuttgart","country":"Germany","ip":"198.51.100.1"}}"#;
    const OOKLA_PING: &str = r#"{"type":"ping","timestamp":"2026-11-04T17:00:01Z","ping":{"jitter":1.25,"latency":20.5,"progress":0.5}}"#;
    const OOKLA_DOWN: &str = r#"{"type":"download","timestamp":"2026-11-04T17:00:03Z","download":{"bandwidth":12500000,"bytes":5000000,"elapsed":400,"progress":0.25}}"#;
    const OOKLA_UP: &str = r#"{"type":"upload","timestamp":"2026-11-04T17:00:09Z","upload":{"bandwidth":2500000,"bytes":100000,"elapsed":40,"progress":0.75}}"#;
    const OOKLA_RESULT: &str = r#"{"type":"result","timestamp":"2026-11-04T17:00:12Z","ping":{"jitter":1.1,"latency":19.9,"low":18.0,"high":22.0},"download":{"bandwidth":11718750,"bytes":120000000,"elapsed":10000},"upload":{"bandwidth":2343750,"bytes":25000000,"elapsed":10000},"packetLoss":0.5,"isp":"Mullvad","interface":{"externalIp":"203.0.113.7"},"server":{"name":"indasys IT Systemhaus AG","location":"Stuttgart","country":"Germany"},"result":{"id":"abc","url":"https://www.speedtest.net/result/c/abc","persisted":true}}"#;

    #[test]
    fn version_gate() {
        assert_eq!(
            backend_from_version("Speedtest by Ookla 1.2.0.84 (ea6b6773cf) Linux/x86_64"),
            Backend::Ookla
        );
        // Arch's speedtest-cli ships a `speedtest` alias.
        assert_eq!(
            backend_from_version("speedtest-cli 2.1.3\nPython 3.14.9"),
            Backend::Legacy
        );
    }

    #[test]
    fn legacy_real_capture() {
        let r = parse_legacy_json(LEGACY).expect("parses");
        let down = r.download_mbps.unwrap();
        assert!((down - 22.9487).abs() < 0.001, "{down}");
        assert_eq!(r.upload_mbps, None, "skipped upload is None, not 0");
        assert_eq!(r.ping_ms, Some(66.679));
        assert_eq!(r.jitter_ms, None);
        assert_eq!(r.packet_loss, None);
        assert_eq!(
            r.server_line(),
            "indasys IT Systemhaus AG · Stuttgart, Germany"
        );
        assert_eq!(r.isp, "31173 Services AB");
        assert_eq!(r.result_url, "");
        assert!(parse_legacy_json("not json").is_none());
        assert!(parse_legacy_json("").is_none());
    }

    #[test]
    fn ookla_stream() {
        assert_eq!(
            parse_ookla_line(OOKLA_START),
            Some(SpeedEvent::Started {
                server: "indasys IT Systemhaus AG · Stuttgart, Germany".into(),
                isp: "Mullvad".into()
            })
        );
        assert_eq!(
            parse_ookla_line(OOKLA_PING),
            Some(SpeedEvent::Ping {
                latency_ms: 20.5,
                jitter_ms: Some(1.25)
            })
        );
        assert_eq!(
            parse_ookla_line(OOKLA_DOWN),
            Some(SpeedEvent::Live {
                phase: Phase::Download,
                mbps: 100.0,
                progress: Some(0.25)
            })
        );
        assert_eq!(
            parse_ookla_line(OOKLA_UP),
            Some(SpeedEvent::Live {
                phase: Phase::Upload,
                mbps: 20.0,
                progress: Some(0.75)
            })
        );
        let Some(SpeedEvent::Done(r)) = parse_ookla_line(OOKLA_RESULT) else {
            panic!("result line");
        };
        assert!((r.download_mbps.unwrap() - 93.75).abs() < 1e-9);
        assert!((r.upload_mbps.unwrap() - 18.75).abs() < 1e-9);
        assert_eq!(r.ping_ms, Some(19.9));
        assert_eq!(r.jitter_ms, Some(1.1));
        assert_eq!(r.packet_loss, Some(0.5));
        assert_eq!(r.external_ip, "203.0.113.7");
        assert_eq!(r.result_url, "https://www.speedtest.net/result/c/abc");
        assert_eq!(
            parse_ookla_line(r#"{"type":"error","error":"Cannot read from socket"}"#),
            Some(SpeedEvent::Failed("Cannot read from socket".into()))
        );
        assert_eq!(parse_ookla_line(r#"{"type":"log","message":"x"}"#), None);
        assert_eq!(parse_ookla_line("garbage"), None);
    }

    // `/proc/net/dev` layout from the owner's machine (counters trimmed).
    const NET_DEV_A: &str = "Inter-|   Receive                                                |  Transmit\n face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed\n    lo: 1000 10 0 0 0 0 0 0 1000 10 0 0 0 0 0 0\n wlan0: 1000000 900 0 0 0 0 0 0 200000 300 0 0 0 0 0 0\nwg0-mullvad: 900000 800 0 0 0 0 0 0 150000 250 0 0 0 0 0 0\n";
    const NET_DEV_B: &str = "Inter-|   Receive                                                |  Transmit\n face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed\n    lo: 9000 10 0 0 0 0 0 0 9000 10 0 0 0 0 0 0\n wlan0: 3600000 900 0 0 0 0 0 0 300000 300 0 0 0 0 0 0\nwg0-mullvad: 3400000 800 0 0 0 0 0 0 240000 250 0 0 0 0 0 0\n";

    #[test]
    fn live_rates_take_busiest_interface_not_sum() {
        let a = parse_proc_net_dev(NET_DEV_A);
        assert!(!a.contains_key("lo"));
        assert_eq!(a.get("wg0-mullvad"), Some(&(900_000, 150_000)));
        let b = parse_proc_net_dev(NET_DEV_B);
        let (rx, tx) = busiest_rates(&a, &b, 2.0);
        assert_eq!(rx, 1_300_000.0); // wlan0: 2.6 MB / 2 s
        assert_eq!(tx, 50_000.0);
        assert_eq!(busiest_rates(&a, &b, 0.0), (0.0, 0.0));
        assert_eq!(infer_phase(rx, tx), Phase::Download);
        assert_eq!(infer_phase(10_000.0, 2_000_000.0), Phase::Upload);
        assert_eq!(infer_phase(1000.0, 1000.0), Phase::Ping);
    }

    #[test]
    fn gauge_scale_and_format() {
        assert_eq!(gauge_scale(0.0), 10.0);
        assert_eq!(gauge_scale(22.9), 25.0);
        assert_eq!(gauge_scale(93.8), 100.0);
        assert_eq!(gauge_scale(940.0), 1000.0);
        assert_eq!(gauge_scale(5000.0), 10_000.0);
        assert_eq!(format_mbps(22.948), "22.9 Mbit/s");
        assert_eq!(format_mbps(940.2), "940 Mbit/s");
        assert_eq!(format_mbps(1250.0), "1.25 Gbit/s");
    }
}
