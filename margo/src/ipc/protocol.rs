//! IPC request grammar: one whitespace-tokenised line per request.

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Verb {
    Get,
    Watch,
    Dispatch,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Request {
    pub verb: Verb,
    /// For get/watch: the topic. For dispatch: the action name.
    pub head: String,
    /// Remaining whitespace-separated tokens.
    pub args: Vec<String>,
}

/// Parse a single request line. Returns a human-readable error string
/// (sent back to the client as `{"error":…}`) on malformed input.
pub fn parse_request(line: &str) -> Result<Request, String> {
    let mut toks = line.split_whitespace();
    let verb = match toks.next() {
        Some("get") => Verb::Get,
        Some("watch") => Verb::Watch,
        Some("dispatch") => Verb::Dispatch,
        Some(other) => return Err(format!("unknown verb: {other}")),
        None => return Err("empty request".into()),
    };
    let head = toks
        .next()
        .ok_or_else(|| "missing topic/action".to_string())?
        .to_string();
    let args = toks.map(str::to_string).collect();
    Ok(Request { verb, head, args })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_get_state() {
        let r = parse_request("get state").unwrap();
        assert_eq!(r.verb, Verb::Get);
        assert_eq!(r.head, "state");
        assert!(r.args.is_empty());
    }

    #[test]
    fn parses_dispatch_with_args() {
        let r = parse_request("dispatch view 4").unwrap();
        assert_eq!(r.verb, Verb::Dispatch);
        assert_eq!(r.head, "view");
        assert_eq!(r.args, vec!["4".to_string()]);
    }

    #[test]
    fn rejects_unknown_verb() {
        assert!(parse_request("frobnicate state").is_err());
    }

    #[test]
    fn rejects_empty() {
        assert!(parse_request("   ").is_err());
    }
}
