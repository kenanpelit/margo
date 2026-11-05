//! The Notes Hub "Files" tab's logic: a folder of Markdown notes (the
//! quill layout — one `.md` per note, first `# heading` is the title,
//! `Inbox.md` for captures), path containment, search, the AI prompts and
//! the git-history commit. GTK-free, so it is unit-tested here.

use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

/// Notes bigger than this are listed but never loaded into a prompt or the
/// editor (a stray log file must not stall the menu).
pub(crate) const MAX_NOTE_BYTES: u64 = 256 * 1024;
/// How deep the scan descends below the folder.
pub(crate) const MAX_DEPTH: usize = 3;
/// The capture target inside the folder.
pub(crate) const INBOX: &str = "Inbox.md";
/// Per-note body budget inside an AI prompt.
const PROMPT_NOTE_CHARS: usize = 4000;

/// One Markdown note found in the folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NoteFile {
    /// Path relative to the folder, `/`-separated (`Daily/2026-11-05.md`).
    pub(crate) rel: String,
    pub(crate) title: String,
    /// The file text (empty when over [`MAX_NOTE_BYTES`]).
    pub(crate) body: String,
    pub(crate) too_big: bool,
    pub(crate) modified: Option<SystemTime>,
}

/// Expand a leading `~` / `~/` against `home`.
pub(crate) fn expand_tilde(raw: &str, home: Option<&Path>) -> PathBuf {
    let raw = raw.trim();
    match (raw.strip_prefix('~'), home) {
        (Some(rest), Some(home)) if rest.is_empty() || rest.starts_with('/') => {
            home.join(rest.trim_start_matches('/'))
        }
        _ => PathBuf::from(raw),
    }
}

/// `rel` as a path inside `root`, or `None` when it could escape it or
/// reach a hidden file: absolute paths, `..`, `~`, empty and dot-prefixed
/// segments (`.git/config`) are all refused, and only `.md` files qualify.
pub(crate) fn contained(root: &Path, rel: &str) -> Option<PathBuf> {
    if rel.is_empty() || rel.starts_with('~') || rel.contains('\0') || rel.contains('\\') {
        return None;
    }
    // Check the raw segments too: `Path::components` folds `a//b` and a
    // leading `./` away, which would hide an empty or dot segment.
    if rel
        .split('/')
        .any(|seg| seg.is_empty() || seg.starts_with('.'))
    {
        return None;
    }
    let p = Path::new(rel);
    if p.is_absolute() || p.extension().and_then(|e| e.to_str()) != Some("md") {
        return None;
    }
    for c in p.components() {
        match c {
            Component::Normal(seg) => {
                let seg = seg.to_str()?;
                if seg.is_empty() || seg.starts_with('.') {
                    return None;
                }
            }
            _ => return None,
        }
    }
    Some(root.join(p))
}

/// The note's title: YAML front-matter `title:`, else the first `# `
/// heading, else the file stem.
pub(crate) fn note_title(rel: &str, body: &str) -> String {
    let mut lines = body.lines();
    let mut rest: Vec<&str> = Vec::new();
    if body.starts_with("---") {
        lines.next();
        for line in lines.by_ref() {
            if line.trim() == "---" {
                break;
            }
            if let Some(t) = line.strip_prefix("title:") {
                let t = t.trim().trim_matches(['"', '\'']);
                if !t.is_empty() {
                    return t.to_string();
                }
            }
        }
    }
    rest.extend(lines);
    if let Some(h) = rest
        .iter()
        .find_map(|l| l.strip_prefix("# ").map(str::trim))
        .filter(|h| !h.is_empty())
    {
        return h.to_string();
    }
    Path::new(rel)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(rel)
        .to_string()
}

/// A file name for a new note titled `title`: lowercase words joined by
/// `-`, safe for any filesystem; `taken` decides collisions (`-2`, `-3`…).
pub(crate) fn new_note_name(title: &str, taken: impl Fn(&str) -> bool) -> String {
    let mut slug = String::new();
    for c in title.trim().chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            slug.push(c);
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
    }
    let slug = slug.trim_end_matches('-');
    let base: String = if slug.is_empty() {
        "note".into()
    } else {
        slug.chars().take(60).collect()
    };
    let mut name = format!("{base}.md");
    let mut n = 2;
    while taken(&name) {
        name = format!("{base}-{n}.md");
        n += 1;
    }
    name
}

/// Scan `root` for Markdown notes (depth ≤ [`MAX_DEPTH`], hidden entries
/// and symlinks skipped), newest first.
pub(crate) fn scan(root: &Path) -> std::io::Result<Vec<NoteFile>> {
    let mut out = Vec::new();
    walk(root, root, 0, &mut out)?;
    out.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.rel.cmp(&b.rel)));
    Ok(out)
}

fn walk(root: &Path, dir: &Path, depth: usize, out: &mut Vec<NoteFile>) -> std::io::Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        // A sub-folder we can't read is skipped; the root must be readable.
        Err(e) if depth > 0 => {
            tracing::debug!(dir = %dir.display(), error = %e, "notes: skipping folder");
            return Ok(());
        }
        Err(e) => return Err(e),
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if name.starts_with('.') {
            continue;
        }
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            if depth < MAX_DEPTH {
                walk(root, &path, depth + 1, out)?;
            }
            continue;
        }
        if !ft.is_file() || !name.ends_with(".md") {
            continue;
        }
        let Ok(rel) = path.strip_prefix(root) else {
            continue;
        };
        let rel = rel.to_string_lossy().replace('\\', "/");
        let meta = entry.metadata().ok();
        let too_big = meta.as_ref().is_some_and(|m| m.len() > MAX_NOTE_BYTES);
        let body = if too_big {
            String::new()
        } else {
            std::fs::read_to_string(&path).unwrap_or_default()
        };
        out.push(NoteFile {
            title: note_title(&rel, &body),
            rel,
            body,
            too_big,
            modified: meta.and_then(|m| m.modified().ok()),
        });
    }
    Ok(())
}

/// Case-insensitive filter over titles, paths and contents.
pub(crate) fn matches_query(note: &NoteFile, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    q.is_empty()
        || q.split_whitespace().all(|w| {
            note.title.to_lowercase().contains(w)
                || note.rel.to_lowercase().contains(w)
                || note.body.to_lowercase().contains(w)
        })
}

const STOPWORDS: &[&str] = &[
    "the", "and", "for", "are", "was", "what", "when", "where", "which", "who", "how", "why",
    "with", "that", "this", "from", "have", "has", "had", "you", "your", "about", "into", "did",
    "does", "can", "should", "would", "could", "there", "their", "them", "then", "than", "any",
    "all", "not", "but", "out", "our", "its", "his", "her", "she", "him", "they", "were", "will",
    "been", "being", "notes", "note", "my", "me",
];

fn terms(question: &str) -> Vec<String> {
    question
        .split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|w| w.chars().count() >= 3 && !STOPWORDS.contains(&w.as_str()))
        .collect()
}

/// The notes to send along with `question`: the `limit` best keyword
/// matches (a title hit weighs 5, body hits count once each up to 5 per
/// term), or — when
/// nothing matches — the most recently modified notes. Oversized notes are
/// never picked.
pub(crate) fn pick_context<'a>(
    notes: &'a [NoteFile],
    question: &str,
    limit: usize,
) -> Vec<&'a NoteFile> {
    let terms = terms(question);
    let usable: Vec<&NoteFile> = notes.iter().filter(|n| !n.too_big).collect();
    let mut scored: Vec<(usize, &NoteFile)> = usable
        .iter()
        .map(|n| {
            let title = n.title.to_lowercase();
            let body = n.body.to_lowercase();
            let score = terms
                .iter()
                .map(|t| {
                    title.matches(t.as_str()).count() * 5 + body.matches(t.as_str()).count().min(5)
                })
                .sum();
            (score, *n)
        })
        .filter(|(s, _)| *s > 0)
        .collect();
    if scored.is_empty() {
        // `notes` is newest-first already.
        return usable.into_iter().take(limit).collect();
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.modified.cmp(&a.1.modified)));
    scored.into_iter().take(limit).map(|(_, n)| n).collect()
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max).collect();
        t.push_str("\n…");
        t
    }
}

/// System + user prompt for a question over `context`.
pub(crate) fn ask_prompt(context: &[&NoteFile], question: &str) -> (String, String) {
    let system = "You answer questions using only the user's notes below. Be concise. \
Cite the note titles you used in square brackets, e.g. [Project ideas]. \
If the notes don't contain the answer, say so."
        .to_string();
    let mut user = String::new();
    for n in context {
        user.push_str(&format!(
            "### {} ({})\n{}\n\n",
            n.title,
            n.rel,
            clip(&n.body, PROMPT_NOTE_CHARS)
        ));
    }
    user.push_str(&format!("Question: {}", question.trim()));
    (system, user)
}

pub(crate) fn summarize_prompt(title: &str, body: &str) -> (String, String) {
    (
        "Summarise the note in at most five short Markdown bullet points. Reply with the bullets only."
            .to_string(),
        format!("# {title}\n\n{}", clip(body, PROMPT_NOTE_CHARS * 4)),
    )
}

pub(crate) fn rewrite_prompt(body: &str, instruction: &str) -> (String, String) {
    (
        "Rewrite the user's Markdown note as instructed. Keep its meaning, headings, links and \
`- [ ]` checkboxes unless told otherwise. Reply with the full rewritten note only, no preamble \
and no code fence."
            .to_string(),
        format!(
            "Instruction: {}\n\nNote:\n{}",
            instruction.trim(),
            clip(body, PROMPT_NOTE_CHARS * 4)
        ),
    )
}

pub(crate) fn capture_prompt(text: &str) -> (String, String) {
    (
        "Turn the user's text into one Markdown todo line of the form `- [ ] <task>`. Fix spelling, \
keep it short, keep any date or #tag the user wrote. Reply with that single line only."
            .to_string(),
        text.trim().to_string(),
    )
}

/// The one line a capture appends to the inbox. The model's reply (or the
/// raw text) is collapsed to a single `- [ ] …` line, so a reply can never
/// inject extra lines, headings or front-matter into the file.
pub(crate) fn capture_line(reply_or_text: &str) -> Option<String> {
    let first = reply_or_text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with("```"))?;
    let task = first
        .trim_start_matches(['-', '*'])
        .trim_start()
        .trim_start_matches("[ ]")
        .trim_start_matches("[x]")
        .trim_start_matches("[X]")
        .trim()
        .trim_matches('`')
        .trim();
    let task = task.split_whitespace().collect::<Vec<_>>().join(" ");
    let task: String = task
        .trim_start_matches('#')
        .trim()
        .chars()
        .take(300)
        .collect();
    (!task.is_empty()).then(|| format!("- [ ] {task}"))
}

/// `existing` inbox text with `line` appended (a fresh inbox gets a title).
pub(crate) fn append_to_inbox(existing: Option<&str>, line: &str) -> String {
    let mut out = match existing {
        Some(t) if !t.trim().is_empty() => t.to_string(),
        _ => "# Inbox\n\n".to_string(),
    };
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(line);
    out.push('\n');
    out
}

/// Git history may run in `dir`: never the home directory itself (or `/`).
pub(crate) fn git_allowed(dir: &Path, home: Option<&Path>) -> bool {
    let dir = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    let home = home.map(|h| h.canonicalize().unwrap_or_else(|_| h.to_path_buf()));
    dir.parent().is_some() && home.as_deref() != Some(dir.as_path())
}

/// The commit message for the staged `files`.
pub(crate) fn commit_message(files: &[&str]) -> String {
    match files {
        [] => "notes: update".to_string(),
        [one] => format!("notes: update {one}"),
        [first, rest @ ..] => format!("notes: update {first} (+{} more)", rest.len()),
    }
}

/// Commit every change under `dir` (blocking — run it off the GTK
/// thread). `git init` once when `dir` is not a repository yet; scoped to
/// the folder with `-- .`; never pushes. A missing identity falls back to
/// a local "mshell notes" one for this commit only, and these automatic
/// commits are never GPG-signed. `Ok(None)` = nothing to commit.
pub(crate) fn git_commit(dir: &Path) -> Result<Option<String>, String> {
    use std::process::{Command, Stdio};
    if !margo_fs::path::on_path("git") {
        return Err("git is not installed".into());
    }
    if !git_allowed(dir, margo_fs::xdg::home().as_deref()) {
        return Err("git history is refused for the home directory".into());
    }
    let git = |args: &[&str]| {
        Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(Stdio::null())
            .output()
            .map_err(|e| e.to_string())
    };
    let fail = |o: &std::process::Output| {
        String::from_utf8_lossy(&o.stderr)
            .lines()
            .next()
            .unwrap_or("git failed")
            .trim()
            .to_string()
    };
    if !dir.join(".git").exists() {
        let o = git(&["init", "-q"])?;
        if !o.status.success() {
            return Err(fail(&o));
        }
    }
    let o = git(&["add", "-A", "--", "."])?;
    if !o.status.success() {
        return Err(fail(&o));
    }
    let o = git(&["diff", "--cached", "--name-only", "--", "."])?;
    let names = String::from_utf8_lossy(&o.stdout).into_owned();
    let files: Vec<&str> = names.lines().filter(|l| !l.is_empty()).collect();
    if files.is_empty() {
        return Ok(None);
    }
    let msg = commit_message(&files);
    // Background commits never sign: a pinentry popping up on every
    // autosave would be worse than useless.
    let o = git(&[
        "-c",
        "commit.gpgsign=false",
        "commit",
        "-q",
        "-m",
        &msg,
        "--",
        ".",
    ])?;
    if o.status.success() {
        return Ok(Some(msg));
    }
    let err = String::from_utf8_lossy(&o.stderr).to_lowercase();
    if err.contains("tell me who you are") || err.contains("user.email") {
        let o = git(&[
            "-c",
            "user.name=mshell notes",
            "-c",
            "user.email=notes@localhost",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "-m",
            &msg,
            "--",
            ".",
        ])?;
        if o.status.success() {
            return Ok(Some(msg));
        }
        return Err(fail(&o));
    }
    Err(fail(&o))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn containment_rejects_escapes_and_hidden_files() {
        let root = Path::new("/n");
        assert_eq!(contained(root, "a.md"), Some(PathBuf::from("/n/a.md")));
        assert_eq!(
            contained(root, "Daily/2026-11-05.md"),
            Some(PathBuf::from("/n/Daily/2026-11-05.md"))
        );
        for bad in [
            "",
            "/etc/x.md",
            "../x.md",
            "a/../../x.md",
            "~/x.md",
            ".git/config.md",
            "a/.hidden.md",
            "a.txt",
            "a\\b.md",
            "./a.md",
            "a//b.md",
        ] {
            assert_eq!(contained(root, bad), None, "{bad}");
        }
    }

    #[test]
    fn tilde_expansion() {
        let home = Path::new("/home/u");
        assert_eq!(
            expand_tilde("~/notes", Some(home)),
            PathBuf::from("/home/u/notes")
        );
        assert_eq!(expand_tilde("~", Some(home)), PathBuf::from("/home/u"));
        assert_eq!(
            expand_tilde(" /srv/n ", Some(home)),
            PathBuf::from("/srv/n")
        );
        assert_eq!(expand_tilde("~bob/x", Some(home)), PathBuf::from("~bob/x"));
    }

    #[test]
    fn titles_from_front_matter_heading_or_stem() {
        assert_eq!(
            note_title("a.md", "---\ntitle: \"Plan\"\ntags: [x]\n---\n# Other"),
            "Plan"
        );
        assert_eq!(
            note_title("a.md", "---\ntags: [x]\n---\n\n# Heading\nbody"),
            "Heading"
        );
        assert_eq!(note_title("Daily/2026-11-05.md", "just text"), "2026-11-05");
        assert_eq!(note_title("x.md", "#hashtag\n#  \n"), "x");
    }

    #[test]
    fn new_note_names_are_slugs_and_unique() {
        let taken = |n: &str| n == "project-ideas.md" || n == "project-ideas-2.md";
        assert_eq!(
            new_note_name("Project  Ideas!", taken),
            "project-ideas-3.md"
        );
        assert_eq!(new_note_name("Çay & Kahve", |_| false), "çay-kahve.md");
        assert_eq!(new_note_name("  ../  ", |_| false), "note.md");
    }

    fn nf(rel: &str, title: &str, body: &str, age: u64) -> NoteFile {
        NoteFile {
            rel: rel.into(),
            title: title.into(),
            body: body.into(),
            too_big: false,
            modified: Some(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1000 - age)),
        }
    }

    #[test]
    fn context_prefers_title_hits_and_falls_back_to_recent() {
        let notes = vec![
            nf("new.md", "Groceries", "milk eggs", 1),
            nf("b.md", "Dentist", "call the dentist friday", 5),
            nf(
                "c.md",
                "Misc",
                "dentist dentist dentist dentist dentist dentist dentist",
                9,
            ),
            NoteFile {
                too_big: true,
                ..nf("big.md", "Dentist log", "", 0)
            },
        ];
        let picked = pick_context(&notes, "When is the dentist?", 2);
        let rels: Vec<&str> = picked.iter().map(|n| n.rel.as_str()).collect();
        // A title hit (5) + body hit (1) outranks body spam (capped at 5).
        assert_eq!(rels, ["b.md", "c.md"]);
        let picked = pick_context(&notes, "the what?", 1);
        assert_eq!(picked[0].rel, "new.md", "no terms → most recent usable");
        assert!(matches_query(&notes[1], "DENT fri"));
        assert!(!matches_query(&notes[1], "dentist milk"));
    }

    #[test]
    fn ask_prompt_names_sources_and_clips_bodies() {
        let long = "x".repeat(PROMPT_NOTE_CHARS + 50);
        let n = nf("a.md", "Alpha", &long, 0);
        let (system, user) = ask_prompt(&[&n], "  what?  ");
        assert!(system.contains("[Project ideas]"));
        assert!(user.starts_with("### Alpha (a.md)\n"));
        assert!(user.contains("…"));
        assert!(user.ends_with("Question: what?"));
    }

    #[test]
    fn capture_lines_are_one_sanitised_todo() {
        assert_eq!(capture_line("buy milk").as_deref(), Some("- [ ] buy milk"));
        assert_eq!(
            capture_line("```\n- [ ] Buy milk tomorrow #groceries\n# injected\n---").as_deref(),
            Some("- [ ] Buy milk tomorrow #groceries")
        );
        assert_eq!(
            capture_line("* [x]   call\tmom ").as_deref(),
            Some("- [ ] call mom")
        );
        assert_eq!(
            capture_line("## heading?").as_deref(),
            Some("- [ ] heading?")
        );
        assert_eq!(capture_line("  \n ").as_deref(), None);
    }

    #[test]
    fn inbox_append() {
        assert_eq!(append_to_inbox(None, "- [ ] a"), "# Inbox\n\n- [ ] a\n");
        assert_eq!(
            append_to_inbox(Some("# Inbox\n\n- [ ] a"), "- [ ] b"),
            "# Inbox\n\n- [ ] a\n- [ ] b\n"
        );
    }

    #[test]
    fn git_is_refused_for_home_and_root() {
        let home = Path::new("/home/u");
        assert!(!git_allowed(home, Some(home)));
        assert!(!git_allowed(Path::new("/"), Some(home)));
        assert!(git_allowed(Path::new("/home/u/notes"), Some(home)));
        assert_eq!(commit_message(&[]), "notes: update");
        assert_eq!(commit_message(&["a.md"]), "notes: update a.md");
        assert_eq!(
            commit_message(&["a.md", "b.md", "c.md"]),
            "notes: update a.md (+2 more)"
        );
    }

    #[test]
    fn git_commit_inits_commits_and_reports_clean_tree() {
        if !margo_fs::path::on_path("git") {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let notes = dir.path().join("notes");
        std::fs::create_dir_all(&notes).unwrap();
        std::fs::write(notes.join("a.md"), "# A").unwrap();
        let msg = git_commit(&notes).unwrap();
        assert_eq!(msg.as_deref(), Some("notes: update a.md"));
        assert!(notes.join(".git").is_dir());
        assert_eq!(git_commit(&notes).unwrap(), None, "nothing new to commit");
        std::fs::write(notes.join("b.md"), "# B").unwrap();
        std::fs::write(notes.join("a.md"), "# A2").unwrap();
        assert_eq!(
            git_commit(&notes).unwrap().as_deref(),
            Some("notes: update a.md (+1 more)")
        );
    }

    #[test]
    fn scan_finds_markdown_skips_hidden_and_deep() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("Daily")).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::create_dir_all(root.join("a/b/c/d")).unwrap();
        std::fs::write(root.join("Inbox.md"), "# Inbox\n- [ ] x").unwrap();
        std::fs::write(root.join("Daily/2026-11-05.md"), "today").unwrap();
        std::fs::write(root.join(".git/HEAD.md"), "no").unwrap();
        std::fs::write(root.join(".hidden.md"), "no").unwrap();
        std::fs::write(root.join("readme.txt"), "no").unwrap();
        std::fs::write(root.join("a/b/c/three.md"), "depth 3").unwrap();
        std::fs::write(root.join("a/b/c/d/four.md"), "too deep").unwrap();
        let mut rels: Vec<String> = scan(root).unwrap().into_iter().map(|n| n.rel).collect();
        rels.sort();
        assert_eq!(rels, ["Daily/2026-11-05.md", "Inbox.md", "a/b/c/three.md"]);
        assert!(scan(&root.join("missing")).is_err());
    }
}
