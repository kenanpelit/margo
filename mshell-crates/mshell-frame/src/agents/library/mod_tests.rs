//! Tests for the saved-session scan on a temp-dir Claude config dir.

use super::*;
use meta::tests::{AI_TITLE, CUSTOM_TITLE, PROMPT};

/// A `/clear` with no folder recorded.
const CLEAR_ONLY: &str =
    r#"{"type":"user","message":{"role":"user","content":"<command-name>/clear</command-name>"}}"#;

/// A plain OK session for the pure tests.
pub(crate) fn session(title: &str, cwd: &str, mtime_ms: i64) -> LibSession {
    LibSession {
        id: format!("00000000-0000-0000-0000-{mtime_ms:012}"),
        config_dir: PathBuf::from("/h/.claude"),
        account: "default".into(),
        primary: true,
        project_dir: encode::encode_cwd(cwd),
        cwd: cwd.into(),
        title: title.into(),
        custom_title: String::new(),
        first_prompt: String::new(),
        branch: String::new(),
        mtime_ms,
        size: 1,
        empty: false,
        status: LibStatus::Ok,
        running: false,
        locked: false,
    }
}

const A: &str = "aaaaaaaa-0000-4000-8000-000000000001";
const B: &str = "bbbbbbbb-0000-4000-8000-000000000002";
const C: &str = "cccccccc-0000-4000-8000-000000000003";
const D: &str = "dddddddd-0000-4000-8000-000000000004";
const E: &str = "eeeeeeee-0000-4000-8000-000000000005";
const F: &str = "ffffffff-0000-4000-8000-000000000006";

/// A transcript line with `cwd` recorded.
fn with_cwd(cwd: &str) -> String {
    PROMPT.replace("\"cwd\":\"/home/kenan\"", &format!("\"cwd\":\"{cwd}\""))
}

struct Fx {
    _d: tempfile::TempDir,
    root: PathBuf,
    cfg: PathBuf,
}

impl Fx {
    fn new() -> Fx {
        let d = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(d.path()).unwrap();
        let cfg = root.join(".claude");
        std::fs::create_dir_all(cfg.join("projects")).unwrap();
        Fx { _d: d, root, cfg }
    }

    fn write(&self, folder: &str, id: &str, lines: &[&str]) -> PathBuf {
        let dir = self.cfg.join("projects").join(folder);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(format!("{id}.jsonl"));
        let mut body = lines.join("\n");
        body.push('\n');
        std::fs::write(&p, body).unwrap();
        p
    }

    fn dir(&self, rel: &str) -> String {
        let p = self.root.join(rel);
        std::fs::create_dir_all(&p).unwrap();
        p.to_string_lossy().into_owned()
    }

    fn accounts(&self) -> Vec<Account> {
        vec![Account {
            dir: self.cfg.clone(),
            label: "default".into(),
            primary: true,
        }]
    }
}

fn by_id(v: &[LibSession], id: &str) -> LibSession {
    v.iter().find(|s| s.id == id).cloned().unwrap()
}

#[test]
fn classifies_every_kind_of_session() {
    let fx = Fx::new();
    // OK: recorded folder exists and is the one the transcript is in.
    let app = fx.dir("work/app");
    fx.write(&encode::encode_cwd(&app), A, &[&with_cwd(&app), AI_TITLE]);
    // Moved: recorded folder is gone, the project name's folder exists.
    let renamed = fx.dir("work/app-renamed");
    let old = fx.root.join("work/app-old").to_string_lossy().into_owned();
    fx.write(&encode::encode_cwd(&renamed), B, &[&with_cwd(&old)]);
    // Misplaced: project name matches no folder, recorded folder exists.
    fx.write("-zz-no-such-root-app", C, &[&with_cwd(&app)]);
    // Orphan: neither exists; the project folder keeps a memory file.
    let gone = fx.root.join("work/gone").to_string_lossy().into_owned();
    fx.write("-zz-no-such-root-gone", D, &[&with_cwd(&gone)]);
    std::fs::create_dir_all(fx.cfg.join("projects/-zz-no-such-root-gone/memory")).unwrap();
    std::fs::write(
        fx.cfg.join("projects/-zz-no-such-root-gone/memory/m.md"),
        "x",
    )
    .unwrap();
    // Empty: only a /clear; also running and locked.
    fx.write(&encode::encode_cwd(&app), E, &[CLEAR_ONLY]);
    let live: HashSet<String> = [E.to_string()].into();
    let mut cache = ScanCache::default();
    let v = scan(&fx.accounts(), &live, &|id| id == A, &mut cache);
    assert_eq!(v.len(), 5);
    let a = by_id(&v, A);
    assert_eq!(
        (a.status.clone(), a.title.as_str()),
        (LibStatus::Ok, "Screenshot key combo")
    );
    assert!(a.locked && !a.can_delete());
    assert_eq!(a.resume_dir().as_deref(), Some(app.as_str()));
    let b = by_id(&v, B);
    assert_eq!(b.status, LibStatus::Moved(PathBuf::from(&renamed)));
    assert_eq!(b.folder(), renamed);
    assert_eq!(b.resume_dir().as_deref(), Some(renamed.as_str()));
    let c = by_id(&v, C);
    assert_eq!(c.status, LibStatus::Misplaced);
    assert!(c.can_repair() && c.resume_dir().is_none());
    let d = by_id(&v, D);
    assert_eq!(d.status, LibStatus::Orphan { memory: true });
    assert!(!d.can_rename());
    assert!(d.no_resume_reason().unwrap().contains("memory files"));
    let e = by_id(&v, E);
    assert!(e.empty && e.running && !e.can_delete() && !e.can_rename());
    assert_eq!(e.status, LibStatus::Ok, "no cwd recorded");
}

#[test]
fn a_symlinked_project_folder_is_one_folder() {
    // This machine: `-repo-archive--eng-android -> -home-kenan--eng-android`,
    // whose transcripts record /repo/archive/.eng/android.
    let fx = Fx::new();
    let app = fx.dir("repo/app");
    let real = "-zz-home-app";
    fx.write(real, A, &[&with_cwd(&app)]);
    std::os::unix::fs::symlink(
        fx.cfg.join("projects").join(real),
        fx.cfg.join("projects").join(encode::encode_cwd(&app)),
    )
    .unwrap();
    let v = scan(
        &fx.accounts(),
        &HashSet::new(),
        &|_| false,
        &mut ScanCache::default(),
    );
    assert_eq!(v.len(), 1, "the alias is not scanned again");
    assert_eq!(v[0].status, LibStatus::Ok);
    assert_eq!(v[0].project_dir, real);
}

#[test]
fn rescans_read_only_what_was_appended() {
    let fx = Fx::new();
    let app = fx.dir("w");
    let p = fx.write(&encode::encode_cwd(&app), F, &[&with_cwd(&app)]);
    let mut cache = ScanCache::default();
    let v = scan(&fx.accounts(), &HashSet::new(), &|_| false, &mut cache);
    assert_eq!(v[0].title, "Write a short essay on rivers.");
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
    writeln!(
        f,
        "{}",
        CUSTOM_TITLE.replace("9f9ccc81-8d2d-461e-ad48-637aa098ba64", F)
    )
    .unwrap();
    drop(f);
    let v = scan(&fx.accounts(), &HashSet::new(), &|_| false, &mut cache);
    assert_eq!(
        (v[0].title.as_str(), v[0].custom_title.as_str()),
        ("My name", "My name")
    );
    std::fs::remove_file(&p).unwrap();
    assert!(scan(&fx.accounts(), &HashSet::new(), &|_| false, &mut cache).is_empty());
    assert!(cache.files.is_empty(), "gone transcripts are forgotten");
}

#[test]
fn resume_line_and_reasons() {
    let mut s = session("t", "/r/my app", 1);
    assert_eq!(
        s.resume_line().unwrap(),
        format!("cd '/r/my app' && claude --resume {}", s.id)
    );
    s.primary = false;
    s.config_dir = PathBuf::from("/h/.claude-work");
    assert!(
        s.resume_line()
            .unwrap()
            .contains("&& CLAUDE_CONFIG_DIR=/h/.claude-work claude --resume")
    );
    assert_eq!(s.no_resume_reason(), None);
    s.status = LibStatus::Misplaced;
    assert!(s.resume_line().is_none());
    assert!(s.no_resume_reason().unwrap().starts_with("Misplaced"));
    s.status = LibStatus::Ok;
    s.empty = true;
    assert!(s.no_resume_reason().unwrap().starts_with("Empty"));
}
