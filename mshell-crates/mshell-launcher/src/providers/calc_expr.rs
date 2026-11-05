//! The launcher calculator's arithmetic evaluator.
//!
//! In-tree on purpose: the crate it replaces (`evalexpr`) moved to
//! AGPL-3.0-only at 12.0, and the calculator only needs plain arithmetic
//! over `f64`, so a small recursive-descent parser beats carrying (and
//! pinning) a general expression language.
//!
//! ```text
//! expr  := term (('+' | '-') term)*
//! term  := unary (('*' | '/' | '%') unary)*
//! unary := ('-' | '+') unary | power
//! power := atom ('^' unary)?                 right-associative: 2^3^2 = 512
//! atom  := number | constant | name '(' expr (',' expr)* ')' | '(' expr ')'
//! ```
//!
//! Unary minus binds looser than `^` (`-2^2 = -4`), `%` is the float
//! remainder (sign of the dividend), constants are `pi` and `e`, and the
//! functions are listed in [`call`]. Anything else — an unknown name, the
//! wrong number of arguments, trailing input, or a result that is not a
//! finite number (`1/0`, `sqrt(-1)`) — is `None`, so the launcher shows no
//! answer instead of `inf` / `NaN`.

/// Nesting depth past which an expression is refused rather than risking
/// the stack: no hand-typed calculation comes near it.
const MAX_DEPTH: usize = 64;

/// Evaluate `input`; `None` unless it is well-formed and the result finite.
pub fn eval(input: &str) -> Option<f64> {
    let tokens = tokenize(input)?;
    let mut parser = Parser {
        tokens: &tokens,
        pos: 0,
        depth: 0,
    };
    let value = parser.expr()?;
    if parser.pos != tokens.len() {
        return None;
    }
    value.is_finite().then_some(value)
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Num(f64),
    Name(String),
    Op(char),
    Open,
    Close,
    Comma,
}

fn tokenize(input: &str) -> Option<Vec<Token>> {
    let chars: Vec<char> = input.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit() || c == '.' {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            // An exponent only when digits follow (`1e3`, `2E-4`), so `2e`
            // stays the number 2 next to the constant e (a syntax error).
            if i < chars.len() && matches!(chars[i], 'e' | 'E') {
                let mut j = i + 1;
                if j < chars.len() && matches!(chars[j], '+' | '-') {
                    j += 1;
                }
                if j < chars.len() && chars[j].is_ascii_digit() {
                    i = j;
                    while i < chars.len() && chars[i].is_ascii_digit() {
                        i += 1;
                    }
                }
            }
            let text: String = chars[start..i].iter().collect();
            tokens.push(Token::Num(text.parse().ok()?));
        } else if c.is_ascii_alphabetic() {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            tokens.push(Token::Name(chars[start..i].iter().collect()));
        } else {
            tokens.push(match c {
                '+' | '-' | '*' | '/' | '%' | '^' => Token::Op(c),
                '(' => Token::Open,
                ')' => Token::Close,
                ',' => Token::Comma,
                _ => return None,
            });
            i += 1;
        }
    }
    Some(tokens)
}

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    depth: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn eat(&mut self, token: &Token) -> bool {
        if self.peek() == Some(token) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expr(&mut self) -> Option<f64> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return None;
        }
        let mut value = self.term()?;
        loop {
            if self.eat(&Token::Op('+')) {
                value += self.term()?;
            } else if self.eat(&Token::Op('-')) {
                value -= self.term()?;
            } else {
                break;
            }
        }
        self.depth -= 1;
        Some(value)
    }

    fn term(&mut self) -> Option<f64> {
        let mut value = self.unary()?;
        loop {
            if self.eat(&Token::Op('*')) {
                value *= self.unary()?;
            } else if self.eat(&Token::Op('/')) {
                value /= self.unary()?;
            } else if self.eat(&Token::Op('%')) {
                value %= self.unary()?;
            } else {
                break;
            }
        }
        Some(value)
    }

    fn unary(&mut self) -> Option<f64> {
        // Unary chains (`----2`) recurse too, so they share the depth cap.
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return None;
        }
        let value = if self.eat(&Token::Op('-')) {
            -self.unary()?
        } else if self.eat(&Token::Op('+')) {
            self.unary()?
        } else {
            self.power()?
        };
        self.depth -= 1;
        Some(value)
    }

    fn power(&mut self) -> Option<f64> {
        let base = self.atom()?;
        if self.eat(&Token::Op('^')) {
            // The exponent is a `unary`, which recurses into `power`
            // again: right-associative, and `2^-1` works.
            return Some(base.powf(self.unary()?));
        }
        Some(base)
    }

    fn atom(&mut self) -> Option<f64> {
        match self.peek()?.clone() {
            Token::Num(n) => {
                self.pos += 1;
                Some(n)
            }
            Token::Open => {
                self.pos += 1;
                let value = self.expr()?;
                self.eat(&Token::Close).then_some(value)
            }
            Token::Name(name) => {
                self.pos += 1;
                if !self.eat(&Token::Open) {
                    return constant(&name);
                }
                let mut args = vec![self.expr()?];
                while self.eat(&Token::Comma) {
                    args.push(self.expr()?);
                }
                if !self.eat(&Token::Close) {
                    return None;
                }
                call(&name, &args)
            }
            Token::Op(_) | Token::Close | Token::Comma => None,
        }
    }
}

fn constant(name: &str) -> Option<f64> {
    match name {
        "pi" => Some(std::f64::consts::PI),
        "e" => Some(std::f64::consts::E),
        _ => None,
    }
}

/// The calculator's functions. `log(x)` is the base-10 logarithm, as on a
/// pocket calculator; `log(x, b)` takes an explicit base.
fn call(name: &str, args: &[f64]) -> Option<f64> {
    let one = |f: fn(f64) -> f64| match args {
        [x] => Some(f(*x)),
        _ => None,
    };
    match name {
        "sqrt" => one(f64::sqrt),
        "sin" => one(f64::sin),
        "cos" => one(f64::cos),
        "tan" => one(f64::tan),
        "asin" => one(f64::asin),
        "acos" => one(f64::acos),
        "atan" => one(f64::atan),
        "ln" => one(f64::ln),
        "log2" => one(f64::log2),
        "log10" => one(f64::log10),
        "exp" => one(f64::exp),
        "abs" => one(f64::abs),
        "floor" => one(f64::floor),
        "ceil" => one(f64::ceil),
        "round" => one(f64::round),
        "log" => match args {
            [x] => Some(x.log10()),
            [x, base] => Some(x.log(*base)),
            _ => None,
        },
        "min" => args.iter().copied().reduce(f64::min),
        "max" => args.iter().copied().reduce(f64::max),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::eval;

    #[test]
    fn precedence_and_associativity() {
        assert_eq!(eval("1 + 2 * 3 - 4 / 2"), Some(5.0));
        assert_eq!(eval("(1 + 2) * 3"), Some(9.0));
        assert_eq!(eval("10 - 4 - 3"), Some(3.0));
        assert_eq!(eval("64 / 4 / 2"), Some(8.0));
        assert_eq!(eval("2^3^2"), Some(512.0));
        assert_eq!(eval("-2^2"), Some(-4.0));
        assert_eq!(eval("(-2)^2"), Some(4.0));
        assert_eq!(eval("2^-2"), Some(0.25));
        assert_eq!(eval("-7 % 4"), Some(-3.0));
    }

    #[test]
    fn numbers_and_names() {
        assert_eq!(eval("1e3"), Some(1000.0));
        assert_eq!(eval("2.5E-1"), Some(0.25));
        assert_eq!(eval(".25"), Some(0.25));
        assert_eq!(eval("2e"), None);
        assert_eq!(eval("1.2.3"), None);
        assert_eq!(eval("pi"), Some(std::f64::consts::PI));
        assert_eq!(eval("pie"), None);
        assert_eq!(eval("log(8, 2)"), Some(3.0));
        assert_eq!(eval("max(1, 5, 3)"), Some(5.0));
    }

    #[test]
    fn malformed_input_is_none() {
        for q in [
            "", "()", "1 +", "* 2", "(1", "1)", "1 2", "sqrt", "sqrt(", "sqrt(1,)", "1 = 1",
            "1 # 2",
        ] {
            assert_eq!(eval(q), None, "{q:?}");
        }
    }

    #[test]
    fn depth_is_capped() {
        let ok = format!("{}1{}", "(".repeat(30), ")".repeat(30));
        assert_eq!(eval(&ok), Some(1.0));
        let deep = format!("{}1{}", "(".repeat(10_000), ")".repeat(10_000));
        assert_eq!(eval(&deep), None);
        assert_eq!(eval(&format!("{}1", "-".repeat(10_000))), None);
    }
}
