//! `.<query>` — quick-pick special characters.
//!
//! Hardcoded ~200 unicode chars across arrows, math, currency,
//! brackets, typography, Greek, and shapes. Activating an entry
//! copies the single character to the wl-clipboard so the user
//! can paste it anywhere — same UX as the calculator's "copy
//! result" flow.
//!
//! ## Behaviour
//!
//! | Query | Result |
//! |---|---|
//! | `.` (bare) | Every entry, in declaration order |
//! | `.<partial>` | Substring filter on name + keywords |
//!
//! `handles_search` returns `false` so we never bleed symbol
//! noise into the regular app list — reach symbols deliberately
//! via the `.` prefix.

use crate::{item::LauncherItem, notify::toast, provider::Provider};
use std::rc::Rc;

/// One symbol entry: the character itself plus a human label and
/// extra search keywords. Static so the table lives in the
/// binary's read-only segment.
struct Symbol {
    char: &'static str,
    name: &'static str,
    keywords: &'static [&'static str],
}

/// The full catalogue. Group order matters only for the bare-`.`
/// browse view; once the user filters, results sort by score.
const SYMBOLS: &[Symbol] = &[
    // ── Arrows ──────────────────────────────────────────
    Symbol { char: "→", name: "Rightwards arrow", keywords: &["arrow", "right", "rarr", "to"] },
    Symbol { char: "←", name: "Leftwards arrow", keywords: &["arrow", "left", "larr", "from"] },
    Symbol { char: "↑", name: "Upwards arrow", keywords: &["arrow", "up", "uarr"] },
    Symbol { char: "↓", name: "Downwards arrow", keywords: &["arrow", "down", "darr"] },
    Symbol { char: "↔", name: "Left-right arrow", keywords: &["arrow", "horizontal", "harr"] },
    Symbol { char: "↕", name: "Up-down arrow", keywords: &["arrow", "vertical", "varr"] },
    Symbol { char: "⇒", name: "Rightwards double arrow", keywords: &["arrow", "implies", "rArr"] },
    Symbol { char: "⇐", name: "Leftwards double arrow", keywords: &["arrow", "lArr"] },
    Symbol { char: "⇑", name: "Upwards double arrow", keywords: &["arrow", "up", "uArr"] },
    Symbol { char: "⇓", name: "Downwards double arrow", keywords: &["arrow", "down", "dArr"] },
    Symbol { char: "⇔", name: "Left-right double arrow", keywords: &["arrow", "iff", "hArr"] },
    Symbol { char: "↩", name: "Leftwards arrow with hook", keywords: &["arrow", "return", "enter"] },
    Symbol { char: "↪", name: "Rightwards arrow with hook", keywords: &["arrow", "hook"] },
    Symbol { char: "⤴", name: "Arrow pointing rightwards then curving upwards", keywords: &["arrow", "curve"] },
    Symbol { char: "⤵", name: "Arrow pointing rightwards then curving downwards", keywords: &["arrow", "curve"] },
    Symbol { char: "⇄", name: "Rightwards arrow over leftwards arrow", keywords: &["arrow", "swap", "exchange"] },
    Symbol { char: "⟶", name: "Long rightwards arrow", keywords: &["arrow", "long", "right"] },
    Symbol { char: "⟵", name: "Long leftwards arrow", keywords: &["arrow", "long", "left"] },
    Symbol { char: "⟷", name: "Long left-right arrow", keywords: &["arrow", "long"] },
    Symbol { char: "⟹", name: "Long rightwards double arrow", keywords: &["arrow", "long", "implies"] },

    // ── Math ────────────────────────────────────────────
    Symbol { char: "±", name: "Plus-minus", keywords: &["plusminus", "pm", "math"] },
    Symbol { char: "∓", name: "Minus-plus", keywords: &["mp", "math"] },
    Symbol { char: "×", name: "Multiplication sign", keywords: &["times", "multiply", "math"] },
    Symbol { char: "÷", name: "Division sign", keywords: &["divide", "math"] },
    Symbol { char: "≠", name: "Not equal to", keywords: &["neq", "ne", "math"] },
    Symbol { char: "≤", name: "Less-than or equal", keywords: &["leq", "le", "math"] },
    Symbol { char: "≥", name: "Greater-than or equal", keywords: &["geq", "ge", "math"] },
    Symbol { char: "≈", name: "Almost equal to", keywords: &["approx", "math"] },
    Symbol { char: "≡", name: "Identical to", keywords: &["equiv", "math"] },
    Symbol { char: "≜", name: "Defined as", keywords: &["def", "math"] },
    Symbol { char: "∞", name: "Infinity", keywords: &["infinity", "infty", "math"] },
    Symbol { char: "∑", name: "Summation", keywords: &["sum", "sigma", "math"] },
    Symbol { char: "∏", name: "Product", keywords: &["product", "prod", "pi", "math"] },
    Symbol { char: "∫", name: "Integral", keywords: &["integral", "int", "math"] },
    Symbol { char: "√", name: "Square root", keywords: &["sqrt", "root", "math"] },
    Symbol { char: "∂", name: "Partial differential", keywords: &["partial", "math"] },
    Symbol { char: "∇", name: "Nabla", keywords: &["nabla", "del", "gradient", "math"] },
    Symbol { char: "∆", name: "Increment / triangle", keywords: &["delta", "math"] },
    Symbol { char: "°", name: "Degree sign", keywords: &["deg", "degree", "math"] },
    Symbol { char: "%", name: "Percent sign", keywords: &["percent", "pct"] },
    Symbol { char: "‰", name: "Per mille", keywords: &["permille", "math"] },
    Symbol { char: "∈", name: "Element of", keywords: &["in", "math", "set"] },
    Symbol { char: "∉", name: "Not an element of", keywords: &["notin", "math", "set"] },
    Symbol { char: "⊂", name: "Subset of", keywords: &["subset", "math"] },
    Symbol { char: "⊃", name: "Superset of", keywords: &["supset", "math"] },
    Symbol { char: "∪", name: "Union", keywords: &["union", "math", "set"] },
    Symbol { char: "∩", name: "Intersection", keywords: &["intersection", "math", "set"] },
    Symbol { char: "∅", name: "Empty set", keywords: &["empty", "math", "null"] },
    Symbol { char: "∀", name: "For all", keywords: &["forall", "math", "logic"] },
    Symbol { char: "∃", name: "There exists", keywords: &["exists", "math", "logic"] },
    Symbol { char: "¬", name: "Logical not", keywords: &["not", "neg", "logic"] },
    Symbol { char: "∧", name: "Logical and", keywords: &["and", "wedge", "logic"] },
    Symbol { char: "∨", name: "Logical or", keywords: &["or", "vee", "logic"] },
    Symbol { char: "⊕", name: "Exclusive or", keywords: &["xor", "oplus", "logic"] },

    // ── Greek (lowercase common) ────────────────────────
    Symbol { char: "α", name: "Alpha", keywords: &["greek", "alpha"] },
    Symbol { char: "β", name: "Beta", keywords: &["greek", "beta"] },
    Symbol { char: "γ", name: "Gamma", keywords: &["greek", "gamma"] },
    Symbol { char: "δ", name: "Delta (lowercase)", keywords: &["greek", "delta"] },
    Symbol { char: "ε", name: "Epsilon", keywords: &["greek", "epsilon"] },
    Symbol { char: "θ", name: "Theta", keywords: &["greek", "theta"] },
    Symbol { char: "λ", name: "Lambda", keywords: &["greek", "lambda"] },
    Symbol { char: "μ", name: "Mu", keywords: &["greek", "mu", "micro"] },
    Symbol { char: "π", name: "Pi", keywords: &["greek", "pi"] },
    Symbol { char: "ρ", name: "Rho", keywords: &["greek", "rho"] },
    Symbol { char: "σ", name: "Sigma", keywords: &["greek", "sigma"] },
    Symbol { char: "τ", name: "Tau", keywords: &["greek", "tau"] },
    Symbol { char: "φ", name: "Phi", keywords: &["greek", "phi"] },
    Symbol { char: "ψ", name: "Psi", keywords: &["greek", "psi"] },
    Symbol { char: "ω", name: "Omega", keywords: &["greek", "omega"] },
    Symbol { char: "Σ", name: "Sigma (uppercase)", keywords: &["greek", "sigma", "sum"] },
    Symbol { char: "Δ", name: "Delta (uppercase)", keywords: &["greek", "delta"] },
    Symbol { char: "Ω", name: "Omega (uppercase)", keywords: &["greek", "omega", "ohm"] },
    Symbol { char: "Φ", name: "Phi (uppercase)", keywords: &["greek", "phi"] },
    Symbol { char: "Ψ", name: "Psi (uppercase)", keywords: &["greek", "psi"] },

    // ── Currency ────────────────────────────────────────
    Symbol { char: "$", name: "Dollar sign", keywords: &["currency", "usd", "dollar"] },
    Symbol { char: "€", name: "Euro sign", keywords: &["currency", "eur", "euro"] },
    Symbol { char: "£", name: "Pound sign", keywords: &["currency", "gbp", "pound"] },
    Symbol { char: "¥", name: "Yen / yuan sign", keywords: &["currency", "yen", "yuan", "jpy", "cny"] },
    Symbol { char: "₺", name: "Turkish lira sign", keywords: &["currency", "lira", "try"] },
    Symbol { char: "₽", name: "Ruble sign", keywords: &["currency", "rub", "ruble"] },
    Symbol { char: "₹", name: "Rupee sign", keywords: &["currency", "inr", "rupee"] },
    Symbol { char: "¢", name: "Cent sign", keywords: &["currency", "cent"] },
    Symbol { char: "₿", name: "Bitcoin sign", keywords: &["currency", "btc", "bitcoin", "crypto"] },

    // ── Typography ──────────────────────────────────────
    Symbol { char: "—", name: "Em dash", keywords: &["dash", "em", "punct"] },
    Symbol { char: "–", name: "En dash", keywords: &["dash", "en", "punct"] },
    Symbol { char: "…", name: "Horizontal ellipsis", keywords: &["ellipsis", "dots", "punct"] },
    Symbol { char: "•", name: "Bullet", keywords: &["bullet", "list", "punct"] },
    Symbol { char: "·", name: "Middle dot", keywords: &["middot", "punct"] },
    Symbol { char: "‘", name: "Left single quote", keywords: &["quote", "lsquo", "punct"] },
    Symbol { char: "’", name: "Right single quote / apostrophe", keywords: &["quote", "rsquo", "punct"] },
    Symbol { char: "“", name: "Left double quote", keywords: &["quote", "ldquo", "punct"] },
    Symbol { char: "”", name: "Right double quote", keywords: &["quote", "rdquo", "punct"] },
    Symbol { char: "«", name: "Left guillemet", keywords: &["quote", "guillemet", "punct"] },
    Symbol { char: "»", name: "Right guillemet", keywords: &["quote", "guillemet", "punct"] },
    Symbol { char: "§", name: "Section sign", keywords: &["section", "punct", "law"] },
    Symbol { char: "¶", name: "Pilcrow / paragraph", keywords: &["pilcrow", "paragraph", "punct"] },
    Symbol { char: "†", name: "Dagger", keywords: &["dagger", "footnote", "punct"] },
    Symbol { char: "‡", name: "Double dagger", keywords: &["dagger", "footnote", "punct"] },
    Symbol { char: "♯", name: "Music sharp", keywords: &["music", "sharp"] },
    Symbol { char: "♭", name: "Music flat", keywords: &["music", "flat"] },
    Symbol { char: "♮", name: "Music natural", keywords: &["music", "natural"] },

    // ── Brand / legal ───────────────────────────────────
    Symbol { char: "©", name: "Copyright", keywords: &["copyright", "legal"] },
    Symbol { char: "®", name: "Registered trademark", keywords: &["registered", "trademark", "legal"] },
    Symbol { char: "™", name: "Trademark", keywords: &["trademark", "tm", "legal"] },
    Symbol { char: "℠", name: "Service mark", keywords: &["servicemark", "sm", "legal"] },

    // ── Shapes ──────────────────────────────────────────
    Symbol { char: "★", name: "Black star", keywords: &["star", "filled", "shape"] },
    Symbol { char: "☆", name: "White star", keywords: &["star", "outline", "shape"] },
    Symbol { char: "♥", name: "Heart suit", keywords: &["heart", "love", "shape"] },
    Symbol { char: "♦", name: "Diamond suit", keywords: &["diamond", "shape"] },
    Symbol { char: "♣", name: "Club suit", keywords: &["club", "shape"] },
    Symbol { char: "♠", name: "Spade suit", keywords: &["spade", "shape"] },
    Symbol { char: "✓", name: "Check mark", keywords: &["check", "tick", "yes", "ok"] },
    Symbol { char: "✗", name: "Ballot X", keywords: &["x", "cross", "no", "wrong"] },
    Symbol { char: "✔", name: "Heavy check mark", keywords: &["check", "tick", "yes", "heavy"] },
    Symbol { char: "✘", name: "Heavy ballot X", keywords: &["x", "cross", "no", "heavy"] },
    Symbol { char: "●", name: "Black circle", keywords: &["circle", "dot", "shape"] },
    Symbol { char: "○", name: "White circle", keywords: &["circle", "outline", "shape"] },
    Symbol { char: "■", name: "Black square", keywords: &["square", "shape"] },
    Symbol { char: "□", name: "White square", keywords: &["square", "outline", "shape"] },
    Symbol { char: "▲", name: "Black up-pointing triangle", keywords: &["triangle", "up", "shape"] },
    Symbol { char: "▼", name: "Black down-pointing triangle", keywords: &["triangle", "down", "shape"] },
    Symbol { char: "◆", name: "Black diamond", keywords: &["diamond", "shape"] },
    Symbol { char: "◇", name: "White diamond", keywords: &["diamond", "outline", "shape"] },

    // ── Box drawing (light) ─────────────────────────────
    Symbol { char: "─", name: "Box drawing horizontal", keywords: &["box", "line", "horizontal"] },
    Symbol { char: "│", name: "Box drawing vertical", keywords: &["box", "line", "vertical"] },
    Symbol { char: "┌", name: "Box drawing top-left", keywords: &["box", "corner"] },
    Symbol { char: "┐", name: "Box drawing top-right", keywords: &["box", "corner"] },
    Symbol { char: "└", name: "Box drawing bottom-left", keywords: &["box", "corner"] },
    Symbol { char: "┘", name: "Box drawing bottom-right", keywords: &["box", "corner"] },
    Symbol { char: "├", name: "Box drawing tee right", keywords: &["box", "tee"] },
    Symbol { char: "┤", name: "Box drawing tee left", keywords: &["box", "tee"] },
    Symbol { char: "┬", name: "Box drawing tee down", keywords: &["box", "tee"] },
    Symbol { char: "┴", name: "Box drawing tee up", keywords: &["box", "tee"] },
    Symbol { char: "┼", name: "Box drawing cross", keywords: &["box", "cross"] },

    // ── Misc useful ─────────────────────────────────────
    Symbol { char: "⌘", name: "Command (Mac)", keywords: &["command", "mac", "key"] },
    Symbol { char: "⌥", name: "Option (Mac)", keywords: &["option", "mac", "key", "alt"] },
    Symbol { char: "⌃", name: "Control (Mac)", keywords: &["control", "ctrl", "mac", "key"] },
    Symbol { char: "⇧", name: "Shift", keywords: &["shift", "key"] },
    Symbol { char: "⏎", name: "Return / enter", keywords: &["return", "enter", "key"] },
    Symbol { char: "⌫", name: "Backspace", keywords: &["backspace", "key"] },
    Symbol { char: "⌦", name: "Delete forward", keywords: &["delete", "key"] },
    Symbol { char: "␣", name: "Space", keywords: &["space", "key"] },
    Symbol { char: "⇥", name: "Tab right", keywords: &["tab", "key"] },
    Symbol { char: "♿", name: "Wheelchair", keywords: &["accessible", "a11y"] },
];

pub struct SymbolsProvider;

impl SymbolsProvider {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SymbolsProvider {
    fn default() -> Self {
        Self::new()
    }
}

fn match_score(symbol: &Symbol, query: &str) -> Option<f64> {
    let q = query.to_ascii_lowercase();
    if q.is_empty() {
        return Some(0.0);
    }
    let name = symbol.name.to_ascii_lowercase();
    if name.starts_with(&q) {
        return Some(180.0);
    }
    if name.contains(&q) {
        return Some(130.0);
    }
    for kw in symbol.keywords {
        if kw.starts_with(&q) {
            return Some(150.0);
        }
        if kw.contains(&q) {
            return Some(90.0);
        }
    }
    None
}

impl Provider for SymbolsProvider {
    fn name(&self) -> &str {
        "Symbols"
    }

    fn category(&self) -> &str {
        "Insert"
    }

    fn handles_search(&self) -> bool {
        // `.` triggers — never bleed symbols into the regular
        // app browse list.
        false
    }

    fn handles_command(&self, query: &str) -> bool {
        query.trim_start().starts_with('.')
    }

    fn commands(&self) -> Vec<LauncherItem> {
        vec![LauncherItem {
            id: "symbols:palette".into(),
            name: ".symbol".into(),
            description: "Pick a special character — arrows, math, currency, greek …".into(),
            icon: "format-text-symbolic".into(),
            icon_is_path: false,
            score: 0.0,
            provider_name: "Symbols".into(),
            usage_key: None,
            on_activate: Rc::new(|| {}),
        }]
    }

    fn search(&self, query: &str) -> Vec<LauncherItem> {
        let trimmed = query.trim_start();
        if !trimmed.starts_with('.') {
            return Vec::new();
        }
        let filter = trimmed.trim_start_matches('.').trim();

        SYMBOLS
            .iter()
            .enumerate()
            .filter_map(|(idx, sym)| {
                let score = match_score(sym, filter)?;
                let payload = sym.char.to_string();
                let display_name = sym.name.to_string();
                Some(LauncherItem {
                    id: format!("symbols:{idx}"),
                    name: format!("{}  {}", sym.char, sym.name),
                    description: format!("Press Enter to copy '{}'", sym.char),
                    icon: "format-text-symbolic".into(),
                    icon_is_path: false,
                    score,
                    provider_name: "Symbols".into(),
                    usage_key: Some(format!("symbols:{}", sym.char)),
                    on_activate: Rc::new(move || {
                        copy_to_clipboard(&payload);
                        toast(format!("Copied {payload}"), display_name.clone());
                    }),
                })
            })
            .collect()
    }
}

fn copy_to_clipboard(text: &str) {
    use std::io::Write;
    use std::process::{Command, Stdio};
    match Command::new("wl-copy").stdin(Stdio::piped()).spawn() {
        Ok(mut child) => {
            if let Some(stdin) = child.stdin.as_mut() {
                let _ = stdin.write_all(text.as_bytes());
            }
            let _ = child.wait();
        }
        Err(err) => tracing::warn!(?err, "symbols wl-copy failed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn does_not_handle_regular_search() {
        let p = SymbolsProvider::new();
        assert!(p.search("hello").is_empty());
    }

    #[test]
    fn dot_arrow_finds_arrows() {
        let p = SymbolsProvider::new();
        let items = p.search(".arrow");
        assert!(items.iter().any(|i| i.name.contains("Rightwards arrow")));
        assert!(items.iter().any(|i| i.name.contains("Leftwards arrow")));
    }

    #[test]
    fn dot_pi_finds_pi() {
        let p = SymbolsProvider::new();
        let items = p.search(".pi");
        assert!(items.iter().any(|i| i.name.starts_with("π")));
    }

    #[test]
    fn bare_dot_returns_everything() {
        let p = SymbolsProvider::new();
        let items = p.search(".");
        assert_eq!(items.len(), SYMBOLS.len());
    }
}
