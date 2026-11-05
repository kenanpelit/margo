//! The Notes Hub document (`$XDG_DATA_HOME/mshell/notes.json`) and the
//! pure logic the menu runs on it: todo pages, ordering, export.
//!
//! Compatibility: every field added after the first release is
//! `#[serde(default)]` and skipped on save while it still holds its
//! default, so a notes.json written by an older mshell parses unchanged
//! (its todos land on the implicit "General" page) and a document that
//! never used a new feature is saved byte-for-byte in the old shape.

use serde::{Deserialize, Serialize};

/// Id of the implicit first todo page. It always exists, cannot be
/// deleted, and owns every todo whose page is unknown (older files have
/// no page at all).
pub(crate) const GENERAL_PAGE_ID: &str = "";
pub(crate) const GENERAL_PAGE_NAME: &str = "General";

fn yes() -> bool {
    true
}

fn is_true(v: &bool) -> bool {
    *v
}

/// Persistent state for the Notes Hub: scratchpad text, note cards and
/// paged todos.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct NotesState {
    #[serde(default)]
    pub(crate) scratchpad: String,
    #[serde(default)]
    pub(crate) notes: Vec<Note>,
    #[serde(default)]
    pub(crate) todos: Vec<Todo>,
    /// Named todo pages after the implicit General page, in tab order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) todo_pages: Vec<TodoPage>,
    /// The page the Todos tab shows ([`GENERAL_PAGE_ID`] = General).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) current_page: String,
    /// Show completed todos in the list (they always count in the header).
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub(crate) show_completed: bool,
}

impl Default for NotesState {
    fn default() -> Self {
        Self {
            scratchpad: String::new(),
            notes: Vec::new(),
            todos: Vec::new(),
            todo_pages: Vec::new(),
            current_page: GENERAL_PAGE_ID.to_string(),
            show_completed: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub(crate) struct Note {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub(crate) struct Todo {
    pub(crate) id: String,
    pub(crate) text: String,
    #[serde(default)]
    pub(crate) completed: bool,
    /// Free-form detail text shown under the todo when expanded.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) details: String,
    /// Owning page id ([`GENERAL_PAGE_ID`] when absent).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) page: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub(crate) struct TodoPage {
    pub(crate) id: String,
    pub(crate) name: String,
}

impl NotesState {
    /// Every page in tab order, General first: `(id, name)`.
    pub(crate) fn pages(&self) -> Vec<(&str, &str)> {
        std::iter::once((GENERAL_PAGE_ID, GENERAL_PAGE_NAME))
            .chain(
                self.todo_pages
                    .iter()
                    .map(|p| (p.id.as_str(), p.name.as_str())),
            )
            .collect()
    }

    fn page_exists(&self, id: &str) -> bool {
        id == GENERAL_PAGE_ID || self.todo_pages.iter().any(|p| p.id == id)
    }

    /// The page the Todos tab shows; a stale id (page deleted elsewhere,
    /// hand edit) falls back to General.
    pub(crate) fn current_page(&self) -> &str {
        if self.page_exists(&self.current_page) {
            &self.current_page
        } else {
            GENERAL_PAGE_ID
        }
    }

    /// Display name of a page id (General for an unknown one).
    pub(crate) fn page_name(&self, id: &str) -> &str {
        self.todo_pages
            .iter()
            .find(|p| p.id == id)
            .map_or(GENERAL_PAGE_NAME, |p| p.name.as_str())
    }

    /// The page a todo is shown on: its own, or General when that page no
    /// longer exists — a todo is never lost because its page went away.
    pub(crate) fn todo_page<'a>(&self, todo: &'a Todo) -> &'a str {
        if self.page_exists(&todo.page) {
            &todo.page
        } else {
            GENERAL_PAGE_ID
        }
    }

    /// Todos of `page` in list order, completed ones only when shown.
    pub(crate) fn page_todos(&self, page: &str) -> Vec<&Todo> {
        self.todos
            .iter()
            .filter(|t| self.todo_page(t) == page)
            .filter(|t| self.show_completed || !t.completed)
            .collect()
    }

    /// `(open, total)` todos on `page`, ignoring the completed filter.
    pub(crate) fn page_counts(&self, page: &str) -> (usize, usize) {
        let on_page = self.todos.iter().filter(|t| self.todo_page(t) == page);
        on_page.fold((0, 0), |(open, total), t| {
            (open + usize::from(!t.completed), total + 1)
        })
    }

    /// Add a named page and switch to it. Blank names are refused.
    pub(crate) fn add_page(&mut self, id: String, name: &str) -> bool {
        let name = name.trim();
        if name.is_empty() || id.is_empty() || self.page_exists(&id) {
            return false;
        }
        self.todo_pages.push(TodoPage {
            id: id.clone(),
            name: name.to_string(),
        });
        self.current_page = id;
        true
    }

    /// Rename a named page (General keeps its name).
    pub(crate) fn rename_page(&mut self, id: &str, name: &str) -> bool {
        let name = name.trim();
        match self.todo_pages.iter_mut().find(|p| p.id == id) {
            Some(p) if !name.is_empty() => {
                p.name = name.to_string();
                true
            }
            _ => false,
        }
    }

    /// Delete a named page; its todos move to General (appended in their
    /// order), never deleted. General itself cannot be deleted.
    pub(crate) fn delete_page(&mut self, id: &str) -> bool {
        if id == GENERAL_PAGE_ID {
            return false;
        }
        let before = self.todo_pages.len();
        self.todo_pages.retain(|p| p.id != id);
        if self.todo_pages.len() == before {
            return false;
        }
        let (moved, mut kept): (Vec<Todo>, Vec<Todo>) = std::mem::take(&mut self.todos)
            .into_iter()
            .partition(|t| t.page == id);
        kept.extend(moved.into_iter().map(|mut t| {
            t.page = GENERAL_PAGE_ID.to_string();
            t
        }));
        self.todos = kept;
        if self.current_page == id {
            self.current_page = GENERAL_PAGE_ID.to_string();
        }
        true
    }

    /// Move todo `id` by `delta` positions within the list its page shows
    /// right now (hidden completed todos don't count as positions). The
    /// move is clamped to the list; returns whether anything moved.
    pub(crate) fn move_todo(&mut self, id: &str, delta: i32) -> bool {
        let Some(todo) = self.todos.iter().find(|t| t.id == id) else {
            return false;
        };
        let page = self.todo_page(todo).to_string();
        let visible: Vec<String> = self
            .page_todos(&page)
            .iter()
            .map(|t| t.id.clone())
            .collect();
        let Some(from) = visible.iter().position(|v| v == id) else {
            return false;
        };
        let last = visible.len().saturating_sub(1) as i64;
        let to = (from as i64 + i64::from(delta)).clamp(0, last) as usize;
        if to == from {
            return false;
        }
        let target = visible[to].clone();
        let Some(src) = self.todos.iter().position(|t| t.id == id) else {
            return false;
        };
        let moving = self.todos.remove(src);
        let Some(dst) = self.todos.iter().position(|t| t.id == target) else {
            self.todos.insert(src, moving);
            return false;
        };
        // Moving down lands after the target, moving up before it.
        let at = if to > from { dst + 1 } else { dst };
        self.todos.insert(at, moving);
        true
    }
}

/// How a todo list is exported (copy / save).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExportFormat {
    /// GitHub task-list Markdown: `## Page` + `- [x] text`.
    Markdown,
    /// Plain text: `Page` + `[x] text`.
    Text,
}

impl ExportFormat {
    pub(crate) fn extension(self) -> &'static str {
        match self {
            Self::Markdown => "md",
            Self::Text => "txt",
        }
    }
}

/// Render todos as text. `page` = one page, `None` = every page as its own
/// section (empty pages skipped). Completed todos are always included —
/// an export is the full record, whatever the view filter.
pub(crate) fn export_todos(state: &NotesState, page: Option<&str>, fmt: ExportFormat) -> String {
    let pages: Vec<(&str, &str)> = match page {
        Some(id) => vec![(id, state.page_name(id))],
        None => state.pages(),
    };
    let mut sections = Vec::new();
    for (id, name) in pages {
        let todos: Vec<&Todo> = state
            .todos
            .iter()
            .filter(|t| state.todo_page(t) == id)
            .collect();
        if todos.is_empty() && page.is_none() {
            continue;
        }
        let mut out = String::new();
        match fmt {
            ExportFormat::Markdown => out.push_str(&format!("## {name}\n\n")),
            ExportFormat::Text => out.push_str(&format!("{name}\n\n")),
        }
        for t in todos {
            let mark = if t.completed { "x" } else { " " };
            let text = one_line(&t.text);
            match fmt {
                ExportFormat::Markdown => out.push_str(&format!("- [{mark}] {text}\n")),
                ExportFormat::Text => out.push_str(&format!("[{mark}] {text}\n")),
            }
            for line in t.details.lines().filter(|l| !l.trim().is_empty()) {
                out.push_str("    ");
                out.push_str(line.trim_end());
                out.push('\n');
            }
        }
        sections.push(out);
    }
    sections.join("\n")
}

/// Collapse a todo's text to one line so a pasted newline can't break the
/// list structure of an export.
fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `todo_YYYYMMDD_HHMMSS.<ext>` for an export written at `now`.
pub(crate) fn export_file_name(now: chrono::NaiveDateTime, fmt: ExportFormat) -> String {
    format!("todo_{}.{}", now.format("%Y%m%d_%H%M%S"), fmt.extension())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn todo(id: &str, text: &str, done: bool, page: &str) -> Todo {
        Todo {
            id: id.into(),
            text: text.into(),
            completed: done,
            details: String::new(),
            page: page.into(),
        }
    }

    /// A notes.json written by mshell 5.0.0 (before pages/details).
    const V5_FIXTURE: &str = r#"{
  "scratchpad": "remember the milk",
  "notes": [
    { "id": "1730000000000", "title": "Ideas", "body": "a\nb" }
  ],
  "todos": [
    { "id": "1730000000001", "text": "ship it", "completed": false },
    { "id": "1730000000002", "text": "write docs", "completed": true }
  ]
}"#;

    #[test]
    fn old_file_parses_and_lands_on_general() {
        let s: NotesState = serde_json::from_str(V5_FIXTURE).unwrap();
        assert_eq!(s.scratchpad, "remember the milk");
        assert_eq!(s.notes.len(), 1);
        assert_eq!(s.todos.len(), 2);
        assert!(s.show_completed, "old files show completed todos");
        assert_eq!(s.current_page(), GENERAL_PAGE_ID);
        assert_eq!(s.page_todos(GENERAL_PAGE_ID).len(), 2);
        assert_eq!(s.page_counts(GENERAL_PAGE_ID), (1, 2));
    }

    #[test]
    fn untouched_old_file_round_trips_in_the_old_shape() {
        let s: NotesState = serde_json::from_str(V5_FIXTURE).unwrap();
        let back: serde_json::Value = serde_json::to_value(&s).unwrap();
        let orig: serde_json::Value = serde_json::from_str(V5_FIXTURE).unwrap();
        assert_eq!(back, orig, "no new keys appear until a feature is used");
    }

    #[test]
    fn new_fields_round_trip() {
        let mut s = NotesState::default();
        assert!(s.add_page("p1".into(), " Work "));
        s.todos.push(Todo {
            details: "call first".into(),
            ..todo("t1", "boss", false, "p1")
        });
        s.show_completed = false;
        let json = serde_json::to_string(&s).unwrap();
        let back: NotesState = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
        assert_eq!(back.page_name("p1"), "Work");
        assert_eq!(back.current_page(), "p1");
    }

    #[test]
    fn unknown_page_falls_back_to_general() {
        let mut s = NotesState::default();
        s.todos.push(todo("t1", "orphan", false, "gone"));
        s.current_page = "gone".into();
        assert_eq!(s.current_page(), GENERAL_PAGE_ID);
        assert_eq!(s.page_todos(GENERAL_PAGE_ID).len(), 1);
    }

    #[test]
    fn page_names_are_validated() {
        let mut s = NotesState::default();
        assert!(!s.add_page("p".into(), "   "));
        assert!(!s.add_page(String::new(), "x"));
        assert!(s.add_page("p".into(), "Home"));
        assert!(!s.add_page("p".into(), "dupe id"));
        assert!(!s.rename_page(GENERAL_PAGE_ID, "Mine"));
        assert!(!s.rename_page("p", " "));
        assert!(s.rename_page("p", "House"));
        assert_eq!(s.pages(), vec![("", "General"), ("p", "House")]);
    }

    #[test]
    fn deleting_a_page_moves_its_todos_to_general() {
        let mut s = NotesState::default();
        s.add_page("p".into(), "Work");
        s.todos = vec![
            todo("a", "general 1", false, ""),
            todo("b", "work 1", false, "p"),
            todo("c", "general 2", true, ""),
            todo("d", "work 2", true, "p"),
        ];
        assert!(!s.delete_page(GENERAL_PAGE_ID));
        assert!(s.delete_page("p"));
        assert_eq!(s.current_page(), GENERAL_PAGE_ID);
        let ids: Vec<&str> = s.todos.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["a", "c", "b", "d"]);
        assert!(s.todos.iter().all(|t| t.page.is_empty()));
        assert!(!s.delete_page("p"));
    }

    #[test]
    fn completed_filter_hides_only_done_todos() {
        let s = NotesState {
            todos: vec![todo("a", "x", true, ""), todo("b", "y", false, "")],
            show_completed: false,
            ..NotesState::default()
        };
        let ids: Vec<&str> = s.page_todos("").iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["b"]);
        assert_eq!(s.page_counts(""), (1, 2));
    }

    fn order(s: &NotesState) -> Vec<&str> {
        s.todos.iter().map(|t| t.id.as_str()).collect()
    }

    #[test]
    fn move_within_a_page_skips_other_pages() {
        let mut s = NotesState::default();
        s.add_page("p".into(), "Work");
        s.todos = vec![
            todo("a", "1", false, ""),
            todo("w1", "w", false, "p"),
            todo("b", "2", false, ""),
            todo("c", "3", false, ""),
        ];
        assert!(s.move_todo("a", 2));
        assert_eq!(order(&s), ["w1", "b", "c", "a"]);
        assert!(s.move_todo("a", -1));
        assert_eq!(order(&s), ["w1", "b", "a", "c"]);
        // Clamped at both ends; a no-op reports false.
        assert!(s.move_todo("a", -10));
        assert_eq!(order(&s), ["w1", "a", "b", "c"]);
        assert!(!s.move_todo("a", -1));
        assert!(!s.move_todo("missing", 1));
    }

    #[test]
    fn move_counts_only_visible_todos() {
        let mut s = NotesState {
            todos: vec![
                todo("a", "1", false, ""),
                todo("done", "2", true, ""),
                todo("b", "3", false, ""),
            ],
            show_completed: false,
            ..NotesState::default()
        };
        // One visible step down = past the hidden done todo, after b.
        assert!(s.move_todo("a", 1));
        assert_eq!(order(&s), ["done", "b", "a"]);
    }

    #[test]
    fn export_markdown_and_text() {
        let mut s = NotesState::default();
        s.add_page("p".into(), "Work");
        s.add_page("e".into(), "Empty");
        s.todos = vec![
            Todo {
                details: "aisle 4\n\n  oat  ".into(),
                ..todo("a", "buy\nmilk", false, "")
            },
            todo("b", "report", true, "p"),
        ];
        s.show_completed = false;
        assert_eq!(
            export_todos(&s, None, ExportFormat::Markdown),
            "## General\n\n- [ ] buy milk\n    aisle 4\n      oat\n\n## Work\n\n- [x] report\n"
        );
        assert_eq!(
            export_todos(&s, Some("p"), ExportFormat::Text),
            "Work\n\n[x] report\n"
        );
        // A single (empty) page still gets its heading.
        assert_eq!(
            export_todos(&s, Some("e"), ExportFormat::Markdown),
            "## Empty\n\n"
        );
    }

    #[test]
    fn export_file_names_are_timestamped() {
        let now = chrono::NaiveDate::from_ymd_opt(2026, 11, 5)
            .unwrap()
            .and_hms_opt(9, 7, 3)
            .unwrap();
        assert_eq!(
            export_file_name(now, ExportFormat::Markdown),
            "todo_20261105_090703.md"
        );
        assert_eq!(
            export_file_name(now, ExportFormat::Text),
            "todo_20261105_090703.txt"
        );
    }
}
