//! SMART self-tests for the Drives tab: what `smartctl -a` says about a
//! running / finished test, and the tracker that turns the
//! `mshell-privileged smart-selftest` stream (one compact JSON document
//! per line: the `-t` answer, then a `-a` read every poll) into progress
//! and a final result. Pure: tested on documents shaped like smartctl
//! 7.5's (`nvme_self_test_log`, `ata_smart_data.self_test`,
//! `ata_smart_self_test_log`).

use serde_json::Value;

pub(crate) use mshell_privileged::SelfTest;

/// A test the drive is running now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Running {
    /// "Short", "Extended", … as smartctl names it.
    pub kind: String,
    /// Percent done, when the drive reports it.
    pub done_pct: Option<u8>,
}

/// The newest entry of the drive's self-test log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LastTest {
    pub kind: String,
    /// smartctl's result text ("Completed without error", …).
    pub result: String,
    pub passed: bool,
    /// Power-on hours when it ran.
    pub hours: Option<u64>,
}

/// Self-test state from one `smartctl -a` document.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct SelfTestInfo {
    pub running: Option<Running>,
    pub last: Option<LastTest>,
}

fn s(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_string()
}

/// Read the self-test state (NVMe or ATA) from a `smartctl -j -a` document.
pub(crate) fn parse_self_test(v: &Value) -> SelfTestInfo {
    let nv = &v["nvme_self_test_log"];
    if nv.is_object() {
        let op = nv["current_self_test_operation"]["value"]
            .as_u64()
            .unwrap_or(0);
        let running = (op != 0).then(|| Running {
            kind: s(&nv["current_self_test_operation"]["string"])
                .trim_end_matches(" in progress")
                .trim_end_matches(" self-test")
                .to_string(),
            done_pct: nv["current_self_test_completion_percent"]
                .as_u64()
                .map(|p| p.min(100) as u8),
        });
        let last = nv["table"].as_array().and_then(|t| t.first()).map(|e| {
            let res = e["self_test_result"]["value"].as_u64();
            LastTest {
                kind: s(&e["self_test_code"]["string"]),
                result: s(&e["self_test_result"]["string"]),
                passed: res == Some(0),
                hours: e["power_on_hours"].as_u64(),
            }
        });
        return SelfTestInfo { running, last };
    }
    // ATA: status value 0xF? = in progress, low nibble = tenths remaining.
    let st = &v["ata_smart_data"]["self_test"]["status"];
    let running = st["value"]
        .as_u64()
        .filter(|val| val >> 4 == 0xF)
        .map(|_| Running {
            kind: "Self-test".to_string(),
            done_pct: st["remaining_percent"]
                .as_u64()
                .map(|r| 100u64.saturating_sub(r) as u8),
        });
    let log = &v["ata_smart_self_test_log"];
    let table = log["extended"]["table"]
        .as_array()
        .or_else(|| log["standard"]["table"].as_array());
    let last = table.and_then(|t| t.first()).map(|e| LastTest {
        kind: s(&e["type"]["string"]),
        result: s(&e["status"]["string"]),
        passed: e["status"]["passed"].as_bool().unwrap_or(false),
        hours: e["lifetime_hours"].as_u64(),
    });
    SelfTestInfo { running, last }
}

/// What one line of the self-test stream means.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Step {
    /// The drive accepted the test.
    Started,
    /// Still running (percent done, when known).
    Progress(Option<u8>),
    /// Finished: the newest log entry, and the fresh document (a full
    /// SMART read the tab can show).
    Done(Option<LastTest>, Box<Value>),
    /// The start was refused; smartctl's reason.
    Refused(String),
    /// Nothing to report (unparseable line, or no test seen yet).
    Waiting,
}

/// Follows one self-test from the helper's stream.
#[derive(Debug, Default)]
pub(crate) struct Tracker {
    started: bool,
    seen_running: bool,
    /// Progress reads without a running test before it was ever seen.
    idle_reads: u32,
    /// The newest log entry before the test (a new one = this test's);
    /// `None` when unknown (no SMART read before the start).
    before: Option<Option<LastTest>>,
}

/// Reads with no running test after the start before the tracker trusts
/// "idle" (a drive may take a moment to begin): then the test is over.
const IDLE_READS_TO_FINISH: u32 = 2;

impl Tracker {
    /// `before`: the newest log entry from the last SMART read (`None` =
    /// no read yet, so a log entry alone can't prove the test finished).
    pub(crate) fn new(before: Option<Option<LastTest>>) -> Self {
        Self {
            before,
            ..Self::default()
        }
    }

    pub(crate) fn step(&mut self, line: &str) -> Step {
        let Ok(v) = serde_json::from_str::<Value>(line.trim()) else {
            return Step::Waiting;
        };
        if !self.started {
            let exit = v["smartctl"]["exit_status"].as_u64().unwrap_or(0);
            if exit & 0b111 != 0 {
                let why = v["smartctl"]["messages"]
                    .as_array()
                    .and_then(|m| m.iter().find_map(|m| m["string"].as_str()))
                    .unwrap_or("the drive refused the self-test (is one already running?)");
                return Step::Refused(why.to_string());
            }
            self.started = true;
            return Step::Started;
        }
        let info = parse_self_test(&v);
        if let Some(r) = info.running {
            self.seen_running = true;
            return Step::Progress(r.done_pct);
        }
        let new_entry =
            info.last.is_some() && self.before.as_ref().is_some_and(|b| *b != info.last);
        if self.seen_running || new_entry {
            return Step::Done(info.last, Box::new(v));
        }
        self.idle_reads += 1;
        if self.idle_reads >= IDLE_READS_TO_FINISH {
            Step::Done(info.last, Box::new(v))
        } else {
            Step::Waiting
        }
    }
}

/// `pkexec` argv for starting `test` on `dev` (validated like root will).
pub(crate) fn start_argv(dev: &str, test: SelfTest) -> Option<Vec<String>> {
    let req = mshell_privileged::Request::SmartSelfTest {
        device: dev.to_string(),
        test,
    }
    .validated()
    .ok()?;
    Some(mshell_privileged::lossy(&req.argv()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NVME_RUNNING: &str = r#"{"smartctl":{"exit_status":0},"device":{"name":"/dev/nvme0"},
        "nvme_self_test_log":{"nsid":-1,"current_self_test_operation":{"value":1,"string":"Short self-test in progress"},
        "current_self_test_completion_percent":40,
        "table":[{"self_test_code":{"value":2,"string":"Extended"},"self_test_result":{"value":0,"string":"Completed without error"},"power_on_hours":2900}]}}"#;
    const NVME_DONE: &str = r#"{"smartctl":{"exit_status":0},"device":{"name":"/dev/nvme0"},
        "nvme_self_test_log":{"nsid":-1,"current_self_test_operation":{"value":0,"string":"No self-test in progress"},
        "table":[{"self_test_code":{"value":1,"string":"Short"},"self_test_result":{"value":0,"string":"Completed without error"},"power_on_hours":2961},
                 {"self_test_code":{"value":2,"string":"Extended"},"self_test_result":{"value":0,"string":"Completed without error"},"power_on_hours":2900}]}}"#;
    const ATA_RUNNING: &str = r#"{"smartctl":{"exit_status":0},
        "ata_smart_data":{"self_test":{"status":{"value":249,"string":"in progress, 90% remaining","remaining_percent":90},"polling_minutes":{"short":2,"extended":222}}},
        "ata_smart_self_test_log":{"standard":{"table":[{"type":{"value":1,"string":"Short offline"},"status":{"value":0,"string":"Completed without error","passed":true},"lifetime_hours":51312}]}}}"#;
    const ATA_FAILED: &str = r#"{"smartctl":{"exit_status":64},
        "ata_smart_data":{"self_test":{"status":{"value":115,"string":"Completed: read failure","passed":false}}},
        "ata_smart_self_test_log":{"standard":{"table":[{"type":{"value":2,"string":"Extended offline"},"status":{"value":115,"string":"Completed: read failure","remaining_percent":30,"passed":false},"lifetime_hours":5127,"lba":3209432544}]}}}"#;
    const STARTED: &str = r#"{"smartctl":{"exit_status":0},"device":{"name":"/dev/nvme0"}}"#;
    const REFUSED: &str = r#"{"smartctl":{"exit_status":4,"messages":[{"string":"Can't start self-test without aborting current test (40% completed)","severity":"error"}]}}"#;

    fn v(s: &str) -> Value {
        serde_json::from_str(s).expect("json")
    }

    #[test]
    fn nvme_running_and_last_entry() {
        let i = parse_self_test(&v(NVME_RUNNING));
        assert_eq!(
            i.running,
            Some(Running {
                kind: "Short".into(),
                done_pct: Some(40)
            })
        );
        let last = i.last.expect("log entry");
        assert_eq!((last.kind.as_str(), last.passed), ("Extended", true));
        let i = parse_self_test(&v(NVME_DONE));
        assert_eq!(i.running, None);
        assert_eq!(i.last.expect("entry").hours, Some(2961));
    }

    #[test]
    fn ata_running_failed_and_absent() {
        let i = parse_self_test(&v(ATA_RUNNING));
        assert_eq!(i.running.expect("running").done_pct, Some(10));
        assert!(i.last.expect("entry").passed);
        let i = parse_self_test(&v(ATA_FAILED));
        assert_eq!(i.running, None);
        let last = i.last.expect("entry");
        assert_eq!(last.result, "Completed: read failure");
        assert!(!last.passed);
        assert_eq!(parse_self_test(&v("{}")), SelfTestInfo::default());
    }

    #[test]
    fn tracker_follows_start_progress_and_done() {
        let before = parse_self_test(&v(NVME_RUNNING)).last;
        let mut t = Tracker::new(Some(before));
        assert_eq!(t.step("garbage"), Step::Waiting);
        assert_eq!(t.step(STARTED), Step::Started);
        assert_eq!(t.step(NVME_RUNNING), Step::Progress(Some(40)));
        match t.step(NVME_DONE) {
            Step::Done(Some(last), doc) => {
                assert_eq!(last.kind, "Short");
                assert!(last.passed);
                assert!(doc["nvme_self_test_log"].is_object());
            }
            other => panic!("expected Done, got {other:?}"),
        }
    }

    #[test]
    fn tracker_finishes_a_test_too_quick_to_see_running() {
        // The log gained an entry: done at the first read.
        let before = parse_self_test(&v(NVME_RUNNING)).last;
        let mut t = Tracker::new(Some(before));
        t.step(STARTED);
        assert!(matches!(t.step(NVME_DONE), Step::Done(Some(_), _)));
        // Same log as before and never running: give it a read, then done.
        let mut t = Tracker::new(Some(parse_self_test(&v(NVME_DONE)).last));
        t.step(STARTED);
        assert_eq!(t.step(NVME_DONE), Step::Waiting);
        assert!(matches!(t.step(NVME_DONE), Step::Done(..)));
        // Unknown log before: an old entry alone is not this test's.
        let mut t = Tracker::new(None);
        t.step(STARTED);
        assert_eq!(t.step(NVME_DONE), Step::Waiting);
        assert_eq!(t.step(NVME_RUNNING), Step::Progress(Some(40)));
        assert!(matches!(t.step(NVME_DONE), Step::Done(..)));
    }

    #[test]
    fn tracker_reports_a_refused_start() {
        let mut t = Tracker::new(None);
        assert_eq!(
            t.step(REFUSED),
            Step::Refused(
                "Can't start self-test without aborting current test (40% completed)".into()
            )
        );
        assert_eq!(
            Tracker::new(None).step(r#"{"smartctl":{"exit_status":2}}"#),
            Step::Refused("the drive refused the self-test (is one already running?)".into())
        );
    }

    #[test]
    fn start_argv_is_the_helper_verb() {
        assert_eq!(
            start_argv("/dev/nvme0", SelfTest::Long).expect("argv"),
            [
                mshell_privileged::HELPER,
                "smart-selftest",
                "/dev/nvme0",
                "long"
            ]
        );
        assert_eq!(start_argv("/dev/x y", SelfTest::Short), None);
    }
}
