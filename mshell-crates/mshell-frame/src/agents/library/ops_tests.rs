//! Tests for [`super`] on temp-dir copies of a Claude config dir.

use super::*;
use crate::agents::library::meta::tests::{AI_TITLE, PROMPT};

const ID: &str = "cae70d77-c84d-45b9-b9ef-3a39caf81fcb";
const OTHER: &str = "0097bc96-d15a-4b4f-81b2-31fc139fc111";

struct Fixture {
    _dir: tempfile::TempDir,
    cfg: std::path::PathBuf,
    root: std::path::PathBuf,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let cfg = root.join(".claude");
    for d in ["projects/-w", "session-env", "file-history"] {
        std::fs::create_dir_all(cfg.join(d)).unwrap();
    }
    Fixture {
        _dir: dir,
        cfg,
        root,
    }
}

fn transcript_at(f: &Fixture, project: &str, id: &str, body: &str) -> std::path::PathBuf {
    let p = f.cfg.join("projects").join(project);
    std::fs::create_dir_all(&p).unwrap();
    let file = p.join(format!("{id}.jsonl"));
    std::fs::write(&file, body).unwrap();
    file
}

#[test]
fn rename_appends_the_rename_record_and_keeps_every_byte() {
    let f = fixture();
    let body = format!("{PROMPT}\n{AI_TITLE}\n");
    let file = transcript_at(&f, "-w", ID, &body);
    let msg = rename(&f.cfg, "-w", ID, "  New\nname  ", false).unwrap();
    assert!(msg.contains("New name"));
    let after = std::fs::read_to_string(&file).unwrap();
    assert!(
        after.starts_with(&body),
        "nothing before the append changed"
    );
    assert_eq!(
        &after[body.len()..],
        format!(
            "{{\"type\":\"custom-title\",\"customTitle\":\"New name\",\"sessionId\":\"{ID}\"}}\n"
        )
    );
    // Every line still parses, and the scan reads the new title.
    let mut m = crate::agents::library::meta::TranscriptMeta::default();
    for l in after.lines() {
        serde_json::from_str::<serde_json::Value>(l).unwrap();
        m.add_line(l);
    }
    assert_eq!(m.title(ID), "New name");
    // The sidecar `/rename` writes too, private.
    let side = f.cfg.join(format!("projects/-w/{ID}/custom-title.json"));
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&side).unwrap()).unwrap();
    assert_eq!(v["customTitle"], "New name");
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(&side).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn rename_never_glues_onto_an_unfinished_last_line() {
    let f = fixture();
    let file = transcript_at(&f, "-w", ID, PROMPT);
    rename(&f.cfg, "-w", ID, "x", false).unwrap();
    let after = std::fs::read_to_string(&file).unwrap();
    let lines: Vec<&str> = after.lines().collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0], PROMPT);
    assert!(lines[1].contains("\"customTitle\":\"x\""));
    // An empty transcript gets the record alone.
    let empty = transcript_at(&f, "-w", OTHER, "");
    rename(&f.cfg, "-w", OTHER, "y", false).unwrap();
    assert_eq!(std::fs::read_to_string(&empty).unwrap().lines().count(), 1);
}

#[test]
fn rename_refusals_leave_the_file_alone() {
    let f = fixture();
    let file = transcript_at(&f, "-w", ID, &format!("{PROMPT}\n"));
    let before = std::fs::read(&file).unwrap();
    assert!(rename(&f.cfg, "-w", ID, "x", true).is_err(), "running");
    assert!(rename(&f.cfg, "-w", ID, " \n ", false).is_err(), "empty");
    assert!(rename(&f.cfg, "-w", ID, &"a".repeat(121), false).is_err());
    assert!(rename(&f.cfg, "-w", OTHER, "x", false).is_err(), "missing");
    assert!(rename(&f.cfg, "../w", ID, "x", false).is_err());
    assert_eq!(std::fs::read(&file).unwrap(), before);
    assert!(
        !f.cfg.join("projects/-w").join(OTHER).exists(),
        "no file created"
    );
    assert_eq!(clean_title("a\r\n\tb").unwrap(), "a b");
    assert_eq!(clean_title(&"ş".repeat(120)).unwrap().chars().count(), 120);
}

#[test]
fn delete_removes_the_session_and_its_folders() {
    let f = fixture();
    let file = transcript_at(&f, "-w", ID, "{}\n");
    let side = f.cfg.join(format!("projects/-w/{ID}/subagents"));
    std::fs::create_dir_all(&side).unwrap();
    std::fs::write(side.join("agent-a.jsonl"), "{}\n").unwrap();
    let env = f.cfg.join("session-env").join(ID);
    let hist = f.cfg.join("file-history").join(ID);
    std::fs::create_dir_all(&env).unwrap();
    std::fs::create_dir_all(&hist).unwrap();
    std::fs::write(hist.join("v1"), "x").unwrap();
    let keep = transcript_at(&f, "-w", OTHER, "{}\n");
    delete(&f.cfg, "-w", ID, false, false).unwrap();
    assert!(!file.exists());
    assert!(!f.cfg.join(format!("projects/-w/{ID}")).exists());
    assert!(!env.exists() && !hist.exists());
    assert!(keep.exists(), "other sessions untouched");
    assert!(f.cfg.join("projects/-w").is_dir(), "project folder kept");
}

#[test]
fn delete_keeps_shared_id_folders_while_another_copy_exists() {
    let f = fixture();
    transcript_at(&f, "-w", ID, "{}\n");
    let copy = transcript_at(&f, "-v", ID, "{}\n");
    let env = f.cfg.join("session-env").join(ID);
    std::fs::create_dir_all(&env).unwrap();
    delete(&f.cfg, "-w", ID, false, false).unwrap();
    assert!(copy.exists());
    assert!(env.exists(), "the -v copy still uses it");
    delete(&f.cfg, "-v", ID, false, false).unwrap();
    assert!(!env.exists());
}

#[test]
fn delete_refuses_running_locked_and_symlinks() {
    let f = fixture();
    let file = transcript_at(&f, "-w", ID, "{}\n");
    assert!(delete(&f.cfg, "-w", ID, true, false).is_err());
    assert!(delete(&f.cfg, "-w", ID, false, true).is_err());
    assert!(file.exists());
    // A symlinked session-env/<id> loses the link only.
    let outside = f.root.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("precious"), "x").unwrap();
    std::os::unix::fs::symlink(&outside, f.cfg.join("session-env").join(ID)).unwrap();
    delete(&f.cfg, "-w", ID, false, false).unwrap();
    assert!(outside.join("precious").exists());
    assert!(std::fs::symlink_metadata(f.cfg.join("session-env").join(ID)).is_err());
    // A transcript reached through a symlinked project folder is refused.
    let elsewhere = f.root.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::fs::write(elsewhere.join(format!("{ID}.jsonl")), "{}\n").unwrap();
    std::os::unix::fs::symlink(&elsewhere, f.cfg.join("projects/-alias")).unwrap();
    assert!(delete(&f.cfg, "-alias", ID, false, false).is_err());
    assert!(elsewhere.join(format!("{ID}.jsonl")).exists());
}

#[test]
fn repair_moves_a_misplaced_transcript_into_its_folder() {
    let f = fixture();
    let cwd = f.root.join("work/app");
    std::fs::create_dir_all(&cwd).unwrap();
    let cwd = cwd.to_str().unwrap().to_string();
    let file = transcript_at(&f, "-old-machine-app", ID, "{}\n");
    let side = f.cfg.join(format!("projects/-old-machine-app/{ID}"));
    std::fs::create_dir_all(&side).unwrap();
    repair(&f.cfg, "-old-machine-app", ID, &cwd, false).unwrap();
    let target = f.cfg.join("projects").join(encode_cwd(&cwd));
    assert!(target.join(format!("{ID}.jsonl")).is_file());
    assert!(target.join(ID).is_dir(), "sidecar moved along");
    assert!(!file.exists() && !side.exists());
    // Already in place now.
    assert!(repair(&f.cfg, &encode_cwd(&cwd), ID, &cwd, false).is_err());
}

#[test]
fn repair_never_overwrites_or_creates_the_working_folder() {
    let f = fixture();
    let cwd = f.root.join("work/app");
    std::fs::create_dir_all(&cwd).unwrap();
    let cwd = cwd.to_str().unwrap().to_string();
    let file = transcript_at(&f, "-old", ID, "old\n");
    let there = transcript_at(&f, &encode_cwd(&cwd), ID, "there\n");
    assert!(repair(&f.cfg, "-old", ID, &cwd, false).is_err());
    assert_eq!(std::fs::read_to_string(&there).unwrap(), "there\n");
    assert!(file.exists());
    // The working folder is gone: refused, nothing created.
    let gone = f.root.join("work/gone").to_str().unwrap().to_string();
    assert!(repair(&f.cfg, "-old", ID, &gone, false).is_err());
    assert!(!std::path::Path::new(&gone).exists());
    assert!(!f.cfg.join("projects").join(encode_cwd(&gone)).exists());
    assert!(repair(&f.cfg, "-old", ID, &cwd, true).is_err(), "running");
    assert!(file.exists());
}

#[test]
fn repair_target_refuses_odd_paths() {
    for bad in [
        "relative", "/", "/a/", "/a//b", "/a/../b", "/a/./b", "/a\nb",
    ] {
        assert!(repair_target(bad, "-x").is_err(), "{bad:?}");
    }
    assert_eq!(repair_target("/a/b", "-x").unwrap(), "-a-b");
    assert!(repair_target("/a/b", "-a-b").is_err(), "same folder");
    assert!(repair_target(&format!("/{}", "a".repeat(250)), "-x").is_err());
}
