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

use crate::{item::LauncherItem, notify::toast, provider::Provider};
use evalexpr::{Value, eval};
use regex::Regex;
use std::rc::Rc;

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
}

impl Default for CalculatorProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl CalculatorProvider {
    pub fn new() -> Self {
        Self {
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
        if expr.chars().all(|c| c.is_ascii_alphabetic() || c.is_whitespace()) {
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
        let rewritten = self.int_literal_re.replace_all(&rewritten, |c: &regex::Captures| {
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
        .map(|re| re.replace_all(input, value.to_string().as_str()).into_owned())
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

    fn search(&self, query: &str) -> Vec<LauncherItem> {
        let trimmed = query.trim();
        if !self.looks_like_math(trimmed) {
            return Vec::new();
        }
        let Some(result) = self.evaluate(trimmed) else {
            return Vec::new();
        };
        let copy_payload = result.clone();
        vec![LauncherItem {
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
                copy_to_clipboard(&copy_payload);
                toast("Copied", format!("Calculator result: {copy_payload}"));
            }),
        }]
    }
}

/// Pipe `text` to `wl-copy`. Failures are logged at warn level;
/// nothing in the UI changes — the user will simply find their
/// clipboard hasn't updated.
fn copy_to_clipboard(text: &str) {
    use std::io::Write;
    use std::process::{Command, Stdio};

    tracing::info!(target: "mshell::launcher", "calculator copy_to_clipboard text={text:?}");
    match Command::new("wl-copy").stdin(Stdio::piped()).spawn() {
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evaluates_basic_arithmetic() {
        let p = CalculatorProvider::new();
        assert_eq!(p.evaluate("2+2"), Some("4".into()));
        assert_eq!(p.evaluate("10/4"), Some("2.5".into()));
        assert_eq!(p.evaluate("(1+2)*3"), Some("9".into()));
    }

    #[test]
    fn evaluates_math_functions() {
        let p = CalculatorProvider::new();
        assert_eq!(p.evaluate("sqrt(16)"), Some("4".into()));
        assert_eq!(p.evaluate("abs(-5)"), Some("5".into()));
    }

    #[test]
    fn rejects_plain_app_names() {
        let p = CalculatorProvider::new();
        assert!(!p.looks_like_math("firefox"));
        assert!(!p.looks_like_math("file manager"));
    }

    #[test]
    fn rejects_trailing_operator() {
        let p = CalculatorProvider::new();
        assert!(!p.looks_like_math("2+"));
        assert!(!p.looks_like_math("3*"));
    }

    #[test]
    fn accepts_function_call_without_operator() {
        let p = CalculatorProvider::new();
        assert!(p.looks_like_math("sqrt(16)"));
    }

    #[test]
    fn empty_query_returns_no_items() {
        let p = CalculatorProvider::new();
        assert!(p.search("").is_empty());
    }

    #[test]
    fn matching_expression_returns_single_item() {
        let p = CalculatorProvider::new();
        let items = p.search("2+2");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "4");
    }
}
