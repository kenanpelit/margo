//! Inline math evaluator.
//!
//! Detects expressions like `2+2`, `sqrt(16) * 3`, `sin(0.5)`,
//! `(1+2)/3`. Pure-Rust evaluator via [`evalexpr`], so no `bc`
//! subprocess shenanigans. On activation the result is copied to
//! the wl-clipboard via `wl-copy`.
//!
//! Detection is conservative: a bare app name like `firefox` must
//! never accidentally evaluate, so we require either an operator
//! or a function call before attempting to parse.
//!
//! ## qalc backend (optional)
//!
//! When `qalc` (Arch: `libqalculate`) is installed it handles what
//! evalexpr can't: unit conversion (`10 km to mi`), currency
//! (`5 usd to eur`, `€20 to try`) and percentages (`20% of 150`,
//! `50 + 10%`). Plain arithmetic stays on evalexpr (synchronous, no
//! subprocess); only expressions with `%`, ` to ` or a currency symbol go
//! to qalc, on a worker thread, and the row appears when it answers.
//! Without qalc nothing changes (`%` keeps evalexpr's modulo meaning).
//!
//! `= <expr>` forces evaluation (qalc when installed, else evalexpr)
//! even when the auto-detection would not fire.
//!
//! ## History
//!
//! Every copied result is remembered (MRU, on disk next to the command
//! history). A bare `=` lists it: Enter copies the result again, Delete
//! forgets the row.

use crate::history::CommandHistory;
use crate::providers::tools::command_exists;
use crate::{
    item::LauncherItem,
    notify::toast,
    provider::{Provider, RefreshNotifier},
};
use evalexpr::{Value, eval};
use regex::Regex;
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::process::Command;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

/// Separator between expression and result in a history entry.
const HIST_SEP: char = '\t';
/// qalc answers kept per session (keyed by expression).
const QALC_CACHE: usize = 64;
/// Characters that mark a currency amount.
const CURRENCY: &[char] = &['$', '€', '£', '₺', '¥', '₹', '₽'];

/// Should `q` go to qalc (when installed)? It must look numeric (contain
/// a digit, start with a digit / sign / paren / currency symbol) and use
/// something evalexpr can't express: `%`, a ` to ` conversion, or a
/// currency symbol. Plain words never qualify — qalc happily turns
/// "foo bar" into femtobar units.
pub fn qalc_candidate(q: &str) -> bool {
    let q = q.trim();
    let Some(first) = q.chars().next() else {
        return false;
    };
    if !q.chars().any(|c| c.is_ascii_digit()) {
        return false;
    }
    if !(first.is_ascii_digit()
        || matches!(first, '.' | '(' | '-' | '+')
        || CURRENCY.contains(&first))
    {
        return false;
    }
    q.contains('%') || q.to_ascii_lowercase().contains(" to ") || q.contains(CURRENCY)
}

/// Rewrite `20% of 150` → `20% * 150` (qalc 5.x reads "of" as a unit).
pub fn qalc_rewrite(expr: &str) -> String {
    static OF_RE: std::sync::OnceLock<Option<Regex>> = std::sync::OnceLock::new();
    match OF_RE.get_or_init(|| Regex::new(r"(?i)%\s+of\s+").ok()) {
        Some(re) => re.replace_all(expr.trim(), "% * ").into_owned(),
        None => expr.trim().to_string(),
    }
}

/// argv for one terse qalc evaluation. `conv 0` turns off mixed-unit
/// output (`6.2137 mi`, not `6 mi + 376 yd + 4.79 in`).
pub fn qalc_argv(expr: &str) -> Vec<String> {
    vec![
        "qalc".into(),
        "-t".into(),
        "-s".into(),
        "conv 0".into(),
        qalc_rewrite(expr),
    ]
}

/// Pick the result out of `qalc -t` stdout: the last non-empty line. A
/// result without any digit, or one that just echoes the input back
/// unevaluated (`1 / 0`, `x + 1`), or a leftover function call
/// (`rem(20, 1 B)`) is not an answer.
pub fn parse_qalc_output(input: &str, stdout: &str) -> Option<String> {
    let line = stdout.lines().map(str::trim).rfind(|l| !l.is_empty())?;
    if !line.chars().any(|c| c.is_ascii_digit()) || line.contains('(') {
        return None;
    }
    let squash = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
    if squash(line) == squash(&qalc_rewrite(input)) {
        return None;
    }
    Some(line.to_string())
}

fn run_qalc(expr: &str) -> Option<String> {
    let argv = qalc_argv(expr);
    let out = Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_qalc_output(expr, &String::from_utf8_lossy(&out.stdout))
}

#[derive(Default)]
struct QalcState {
    results: HashMap<String, Option<String>>,
    order: VecDeque<String>,
    wanted: Option<String>,
    running: bool,
}

/// Off-thread qalc evaluation with a small per-session cache. One worker
/// at a time, always on the newest expression, so typing doesn't queue
/// a qalc run per keystroke.
#[derive(Default)]
struct QalcRunner {
    state: Arc<Mutex<QalcState>>,
    notifier: Arc<Mutex<Option<RefreshNotifier>>>,
}

impl QalcRunner {
    /// `Some(answer)` when cached (answer may be `None` = no result),
    /// `None` while pending.
    fn get(&self, expr: &str) -> Option<Option<String>> {
        let mut st = self.state.lock().ok()?;
        if let Some(r) = st.results.get(expr) {
            return Some(r.clone());
        }
        st.wanted = Some(expr.to_string());
        if !st.running {
            st.running = true;
            let state = Arc::clone(&self.state);
            let notifier = Arc::clone(&self.notifier);
            std::thread::spawn(move || {
                loop {
                    let job = state.lock().ok().and_then(|mut s| {
                        let job = s.wanted.take();
                        if job.is_none() {
                            s.running = false;
                        }
                        job
                    });
                    let Some(expr) = job else { break };
                    let answer = run_qalc(&expr);
                    if let Ok(mut s) = state.lock() {
                        s.results.insert(expr.clone(), answer);
                        s.order.push_back(expr);
                        while s.order.len() > QALC_CACHE {
                            if let Some(old) = s.order.pop_front() {
                                s.results.remove(&old);
                            }
                        }
                    }
                    if let Ok(slot) = notifier.lock()
                        && let Some(notify) = slot.as_ref()
                    {
                        notify();
                    }
                }
            });
        }
        None
    }

    #[cfg(test)]
    fn seed(&self, expr: &str, answer: Option<&str>) {
        if let Ok(mut s) = self.state.lock() {
            s.results.insert(expr.into(), answer.map(str::to_string));
        }
    }
}

fn history_path() -> PathBuf {
    margo_fs::xdg::cache_home()
        .join("margo")
        .join("launcher_calc_history.json")
}

/// `expr<TAB>result` → (expr, result).
fn split_history(entry: &str) -> Option<(&str, &str)> {
    entry.split_once(HIST_SEP)
}

pub struct CalculatorProvider {
    /// Compiled once at construction so the regex doesn't get
    /// re-parsed on every keystroke.
    candidate_re: Regex,
    operator_re: Regex,
    function_call_re: Regex,
    /// Rewrites bare function names like `sqrt(` to evalexpr's
    /// namespaced form `math::sqrt(`. Stored on the struct so the
    /// regex compiles once.
    function_prefix_re: Regex,
    /// Promotes integer literals to floats so e.g. `10/4` yields
    /// `2.5` instead of `2`.
    int_literal_re: Regex,
    /// qalc found on PATH at construction.
    qalc: bool,
    runner: QalcRunner,
    /// Copied results, MRU (`expr\tresult`).
    history: Rc<RefCell<CommandHistory>>,
}

impl Default for CalculatorProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl CalculatorProvider {
    pub fn new() -> Self {
        Self::with_options(
            command_exists("qalc"),
            Rc::new(RefCell::new(CommandHistory::load_from(history_path()))),
        )
    }

    /// Explicit backend + history store (tests, or a UI that shares it).
    pub fn with_options(qalc: bool, history: Rc<RefCell<CommandHistory>>) -> Self {
        Self {
            qalc,
            runner: QalcRunner::default(),
            history,
            // Digits, ops, parens, dot, percent, caret, letters
            // (for math functions), commas, whitespace.
            candidate_re: Regex::new(r"^[\d\s\+\-\*\/\(\)\.\%\^a-zA-Z,]+$").unwrap(),
            // At least one binary operator.
            operator_re: Regex::new(r"[+\-*/%\^]").unwrap(),
            // Letters followed by an opening paren = function call
            // (sqrt, sin, cos, etc.).
            function_call_re: Regex::new(r"[a-zA-Z]\s*\(").unwrap(),
            // Captures bare function names so we can rewrite them
            // to evalexpr's `math::*` namespace before evaluation.
            function_prefix_re: Regex::new(
                r"\b(sqrt|sin|cos|tan|asin|acos|atan|ln|log|log2|log10|exp|abs|floor|ceil|round)\(",
            )
            .unwrap(),
            int_literal_re: Regex::new(r"(?P<n>\b\d+\b)(?P<tail>\.\d)?").unwrap(),
        }
    }

    fn looks_like_math(&self, expr: &str) -> bool {
        if !self.candidate_re.is_match(expr) {
            return false;
        }
        if !self.operator_re.is_match(expr) && !self.function_call_re.is_match(expr) {
            return false;
        }
        // Reject trailing operators — incomplete expressions just
        // produce confusing zero-valued results.
        if let Some(last) = expr.trim_end().chars().last()
            && "+-*/%^".contains(last)
        {
            return false;
        }
        // Reject pure-letters (would match every short app name).
        if expr
            .chars()
            .all(|c| c.is_ascii_alphabetic() || c.is_whitespace())
        {
            return false;
        }
        true
    }

    fn evaluate(&self, expr: &str) -> Option<String> {
        // Rewrite `sqrt(`, `sin(`, … to evalexpr's `math::sqrt(` /
        // `math::sin(` form so users can type the natural name.
        // Then substitute `pi` and `e` with their literal values so
        // we don't need a custom variable context.
        let rewritten = self.function_prefix_re.replace_all(expr, "math::$1(");
        let rewritten = substitute_constant(&rewritten, "pi", std::f64::consts::PI);
        let rewritten = substitute_constant(&rewritten, "e", std::f64::consts::E);
        let rewritten = self
            .int_literal_re
            .replace_all(&rewritten, |c: &regex::Captures| {
                // Already has a fractional tail like `1.5` → leave it.
                if c.name("tail").is_some() {
                    return c.get(0).unwrap().as_str().to_string();
                }
                format!("{}.0", &c["n"])
            });

        match eval(&rewritten).ok()? {
            Value::Int(i) => Some(i.to_string()),
            Value::Float(f) => Some(format_float(f)),
            other => Some(other.to_string()),
        }
    }
}

/// Substitute a bare identifier `name` with `value.to_string()` —
/// only when surrounded by non-word characters so we don't mangle
/// e.g. `expression` when `name == "ex"`.
fn substitute_constant(input: &str, name: &str, value: f64) -> String {
    let pattern = format!(r"\b{name}\b");
    Regex::new(&pattern)
        .map(|re| {
            re.replace_all(input, value.to_string().as_str())
                .into_owned()
        })
        .unwrap_or_else(|_| input.to_string())
}

/// Format a float without trailing zeros — `4` instead of `4.0`,
/// `0.5` instead of `0.5000000000`. Long results get rounded to 8
/// significant digits which is plenty for a calculator pill.
fn format_float(f: f64) -> String {
    if f.is_finite() && f == f.trunc() && f.abs() < 1e15 {
        return format!("{}", f as i64);
    }
    let rounded = format!("{f:.8}");
    let trimmed = rounded.trim_end_matches('0').trim_end_matches('.');
    trimmed.to_string()
}

impl Provider for CalculatorProvider {
    fn name(&self) -> &str {
        "Calculator"
    }

    fn category(&self) -> &str {
        "Run"
    }

    fn handles_command(&self, query: &str) -> bool {
        query.trim_start().starts_with('=')
    }

    fn commands(&self) -> Vec<LauncherItem> {
        vec![LauncherItem {
            id: "calc:palette".into(),
            name: "= <expression>".into(),
            description: if self.qalc {
                "Calculate with qalc (units, currency, %); bare = shows history".into()
            } else {
                "Calculate; bare = shows history (install libqalculate for units/currency)".into()
            },
            icon: "accessories-calculator-symbolic".into(),
            icon_is_path: false,
            score: 0.0,
            provider_name: "Calculator".into(),
            usage_key: None,
            on_activate: Rc::new(|| {}),
        }]
    }

    fn search(&self, query: &str) -> Vec<LauncherItem> {
        let trimmed = query.trim();
        if let Some(rest) = trimmed.strip_prefix('=') {
            let expr = rest.trim();
            if expr.is_empty() {
                return self.history_rows();
            }
            let answer = if self.qalc {
                match self.runner.get(expr) {
                    Some(answer) => answer,
                    None => {
                        return vec![self.info_row(
                            "Calculating…",
                            expr,
                            "content-loading-symbolic",
                        )];
                    }
                }
            } else {
                self.evaluate(expr)
            };
            return match answer {
                Some(result) => vec![self.result_row(expr, result)],
                None => vec![self.info_row("No result", expr, "dialog-warning-symbolic")],
            };
        }
        if self.qalc && qalc_candidate(trimmed) {
            // Pending or no answer → no row (implicit mode stays quiet).
            return match self.runner.get(trimmed) {
                Some(Some(result)) => vec![self.result_row(trimmed, result)],
                _ => Vec::new(),
            };
        }
        if !self.looks_like_math(trimmed) {
            return Vec::new();
        }
        let Some(result) = self.evaluate(trimmed) else {
            return Vec::new();
        };
        vec![self.result_row(trimmed, result)]
    }

    fn can_delete(&self, item: &LauncherItem) -> bool {
        item.id.starts_with("calc:history:")
    }

    /// Delete on a history row forgets it.
    fn delete_item(&mut self, item: &LauncherItem) {
        if let Some(entry) = item.id.strip_prefix("calc:history:") {
            self.history.borrow_mut().forget(entry);
        }
    }

    fn set_refresh_notifier(&mut self, notifier: RefreshNotifier) {
        if let Ok(mut slot) = self.runner.notifier.lock() {
            *slot = Some(notifier);
        }
    }

    fn on_closed(&mut self) {
        self.history.borrow_mut().flush();
    }

    /// Show the result large + monospace in the preview pane. The
    /// result lives in `name`; `description` is the copy hint.
    fn preview(&self, item: &LauncherItem) -> Option<crate::LauncherPreview> {
        Some(crate::LauncherPreview::mono("Result", item.name.clone()))
    }
}

impl CalculatorProvider {
    /// The copyable result row; activation copies and records history.
    fn result_row(&self, expr: &str, result: String) -> LauncherItem {
        let copy_payload = result.clone();
        let history = Rc::clone(&self.history);
        let entry = format!("{}{HIST_SEP}{result}", expr.trim());
        LauncherItem {
            id: "calc:result".into(),
            name: result,
            description: "Press Enter to copy".into(),
            icon: "accessories-calculator-symbolic".into(),
            icon_is_path: false,
            // Calculator pins itself to the top — `500` is well
            // above any fuzzy app score (typically 50-250 on the
            // raw nucleo scale).
            score: 500.0,
            provider_name: "Calculator".into(),
            usage_key: None,
            on_activate: Rc::new(move || {
                history.borrow_mut().bump(&entry);
                copy_to_clipboard(&copy_payload);
                toast("Copied", format!("Calculator result: {copy_payload}"));
            }),
        }
    }

    fn info_row(&self, name: &str, description: &str, icon: &str) -> LauncherItem {
        LauncherItem {
            id: "calc:info".into(),
            name: name.into(),
            description: description.into(),
            icon: icon.into(),
            icon_is_path: false,
            score: 500.0,
            provider_name: "Calculator".into(),
            usage_key: None,
            on_activate: Rc::new(|| {}),
        }
    }

    /// Bare `=`: copied results, newest first.
    fn history_rows(&self) -> Vec<LauncherItem> {
        let history = self.history.borrow();
        let rows: Vec<LauncherItem> = history
            .entries()
            .iter()
            .filter_map(|entry| {
                let (expr, result) = split_history(entry)?;
                Some((entry.clone(), expr.to_string(), result.to_string()))
            })
            .enumerate()
            .map(|(idx, (entry, expr, result))| {
                let payload = result.clone();
                let store = Rc::clone(&self.history);
                LauncherItem {
                    id: format!("calc:history:{entry}"),
                    name: result,
                    description: format!("{expr} · Enter: copy · Del: forget"),
                    icon: "document-open-recent-symbolic".into(),
                    icon_is_path: false,
                    score: 400.0 - idx as f64,
                    provider_name: "Calculator".into(),
                    usage_key: None,
                    on_activate: Rc::new(move || {
                        store.borrow_mut().bump(&entry);
                        copy_to_clipboard(&payload);
                        toast("Copied", format!("Calculator result: {payload}"));
                    }),
                }
            })
            .collect();
        if rows.is_empty() {
            vec![self.info_row(
                "No calculator history yet",
                "Copied results show up here",
                "document-open-recent-symbolic",
            )]
        } else {
            rows
        }
    }
}

/// Pipe `text` to `wl-copy`. Failures are logged at warn level;
/// nothing in the UI changes — the user will simply find their
/// clipboard hasn't updated.
fn copy_to_clipboard(text: &str) {
    use std::io::Write;
    use std::process::{Command, Stdio};

    tracing::info!(target: "mshell::launcher", "calculator copy_to_clipboard text={text:?}");
    // Unit tests run inside the owner's live session: never touch the
    // real clipboard from them.
    if cfg!(test) {
        return;
    }
    // Spawn, write and wait all off the GTK main thread — `on_activate`
    // runs there, and even though `wl-copy` forks and exits quickly, it's
    // a real subprocess spawn + blocking wait on every activation.
    let text = text.to_string();
    std::thread::spawn(
        move || match Command::new("wl-copy").stdin(Stdio::piped()).spawn() {
            Ok(mut child) => {
                if let Some(stdin) = child.stdin.as_mut() {
                    let _ = stdin.write_all(text.as_bytes());
                }
                let _ = child.wait();
                tracing::info!(target: "mshell::launcher", "wl-copy done");
            }
            Err(err) => {
                tracing::warn!(?err, "wl-copy spawn failed");
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evaluates_basic_arithmetic() {
        let p = CalculatorProvider::with_options(false, temp_history("basic"));
        assert_eq!(p.evaluate("2+2"), Some("4".into()));
        assert_eq!(p.evaluate("10/4"), Some("2.5".into()));
        assert_eq!(p.evaluate("(1+2)*3"), Some("9".into()));
    }

    #[test]
    fn evaluates_math_functions() {
        let p = CalculatorProvider::with_options(false, temp_history("basic"));
        assert_eq!(p.evaluate("sqrt(16)"), Some("4".into()));
        assert_eq!(p.evaluate("abs(-5)"), Some("5".into()));
    }

    #[test]
    fn rejects_plain_app_names() {
        let p = CalculatorProvider::with_options(false, temp_history("basic"));
        assert!(!p.looks_like_math("firefox"));
        assert!(!p.looks_like_math("file manager"));
    }

    #[test]
    fn rejects_trailing_operator() {
        let p = CalculatorProvider::with_options(false, temp_history("basic"));
        assert!(!p.looks_like_math("2+"));
        assert!(!p.looks_like_math("3*"));
    }

    #[test]
    fn accepts_function_call_without_operator() {
        let p = CalculatorProvider::with_options(false, temp_history("basic"));
        assert!(p.looks_like_math("sqrt(16)"));
    }

    #[test]
    fn empty_query_returns_no_items() {
        let p = CalculatorProvider::with_options(false, temp_history("basic"));
        assert!(p.search("").is_empty());
    }

    fn temp_history(name: &str) -> Rc<RefCell<CommandHistory>> {
        let path = std::env::temp_dir().join(format!(
            "mshell-calc-hist-{}-{name}.json",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        Rc::new(RefCell::new(CommandHistory::load_from(path)))
    }

    #[test]
    fn qalc_candidates_need_numbers_and_qalc_only_syntax() {
        for q in [
            "5 usd to eur",
            "20% of 150",
            "50 + 10%",
            "€20 to try",
            "10 km to mi",
            "$5",
        ] {
            assert!(qalc_candidate(q), "{q}");
        }
        for q in [
            "2+2",
            "foo bar",
            "go to bed",
            "firefox",
            "",
            "to 5",
            "10 km",
        ] {
            assert!(!qalc_candidate(q), "{q}");
        }
    }

    #[test]
    fn qalc_argv_rewrites_percent_of_and_disables_mixed_units() {
        assert_eq!(qalc_rewrite("20% of 150"), "20% * 150");
        assert_eq!(qalc_rewrite(" 15 % OF 200 "), "15 % * 200");
        assert_eq!(
            qalc_argv("10 km to mi"),
            vec!["qalc", "-t", "-s", "conv 0", "10 km to mi"]
        );
    }

    // Real `qalc -t` (5.13.1) answers captured on this machine.
    #[test]
    fn parses_real_qalc_output() {
        assert_eq!(
            parse_qalc_output("5 usd to eur", "€4.2597\n").as_deref(),
            Some("€4.2597")
        );
        assert_eq!(
            parse_qalc_output("20% * 150", "30\n").as_deref(),
            Some("30")
        );
        assert_eq!(
            parse_qalc_output("10 km to mi", "Unrecognized option.\n\n6.213711922 mi\n").as_deref(),
            Some("6.213711922 mi")
        );
        // Unevaluated echoes / leftovers are not answers.
        assert_eq!(parse_qalc_output("1/0", "1 / 0\n"), None);
        assert_eq!(parse_qalc_output("20% of 150", "rem(20, 1 B)\n"), None);
        assert_eq!(parse_qalc_output("x", "x\n"), None);
        assert_eq!(parse_qalc_output("x", ""), None);
    }

    #[test]
    fn qalc_rows_and_fallbacks() {
        let p = CalculatorProvider::with_options(true, temp_history("qalc"));
        p.runner.seed("5 usd to eur", Some("€4.2597"));
        let rows = p.search("5 usd to eur");
        assert_eq!(rows[0].name, "€4.2597");
        // Plain math stays on evalexpr even with qalc present.
        assert_eq!(p.search("2+2")[0].name, "4");
        // Explicit `=` on a cached failure → "No result" row.
        p.runner.seed("blah", None);
        assert_eq!(p.search("= blah")[0].name, "No result");
        // Without qalc: `%` keeps evalexpr modulo; `=` uses evalexpr.
        let p = CalculatorProvider::with_options(false, temp_history("noqalc"));
        assert_eq!(p.search("7%4")[0].name, "3");
        assert!(p.search("5 usd to eur").is_empty());
        assert_eq!(p.search("= 6*7")[0].name, "42");
        assert!(p.handles_command("=1+1"));
        assert!(!p.handles_command("1+1"));
    }

    #[test]
    fn history_records_copies_and_lists_mru() {
        let hist = temp_history("mru");
        let mut p = CalculatorProvider::with_options(false, hist.clone());
        assert_eq!(p.search("=")[0].name, "No calculator history yet");
        // Activation records (the clipboard/toast side effects are
        // compiled out under cfg(test)).
        (p.search("2+2")[0].on_activate)();
        (p.search("3*3")[0].on_activate)();
        let rows = p.search("=");
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["9", "4"]);
        assert!(rows[0].description.starts_with("3*3"));
        assert!(p.can_delete(&rows[1]));
        p.delete_item(&rows[1]);
        let names: Vec<String> = p.search("=").into_iter().map(|r| r.name).collect();
        assert_eq!(names, vec!["9"]);
    }

    #[test]
    fn matching_expression_returns_single_item() {
        let p = CalculatorProvider::with_options(false, temp_history("basic"));
        let items = p.search("2+2");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "4");
    }
}
