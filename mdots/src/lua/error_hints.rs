//! Lua error diagnostics: turn a raw `mlua::Error` into a cleaned message plus
//! an optional line number and a human-actionable hint. Pure string processing,
//! kept out of `mod.rs` so the validation logic there stays focused.

/// Parse a Lua error to extract line number and provide helpful hints
pub(super) fn parse_lua_error(error: &mlua::Error) -> (String, Option<u32>, Option<String>) {
    let error_str = error.to_string();

    // Try to extract line number from error message
    // Format is typically: "[string \"filename\"]:line: message"
    let line = extract_line_number(&error_str);

    // Generate helpful hints based on error content
    let hint = generate_error_hint(&error_str);

    // Clean up the error message
    let message = clean_error_message(&error_str);

    (message, line, hint)
}

/// Extract line number from Lua error message
fn extract_line_number(error: &str) -> Option<u32> {
    // Pattern: ]:line: or :line:
    let re = regex::Regex::new(r"]:?(\d+):").ok()?;
    re.captures(error)
        .and_then(|caps| caps.get(1))
        .and_then(|m| m.as_str().parse().ok())
}

/// Generate helpful hints based on error content
fn generate_error_hint(error: &str) -> Option<String> {
    let error_lower = error.to_lowercase();

    if error_lower.contains("unexpected symbol near '='") {
        return Some("Check for missing 'local' keyword or incorrect table syntax".to_string());
    }

    if error_lower.contains("attempt to index a nil value") {
        if error_lower.contains("mdots") {
            return Some(
                "Make sure you're using the correct mdots.* API (e.g., mdots.hardware.cpu_vendor())"
                    .to_string(),
            );
        }
        return Some("A variable is nil when you're trying to access its fields".to_string());
    }

    if error_lower.contains("attempt to call a nil value") {
        return Some(
            "The function you're trying to call doesn't exist. Check the API documentation."
                .to_string(),
        );
    }

    if error_lower.contains("'}' expected") {
        return Some(
            "Missing closing brace '}'. Check that all tables are properly closed.".to_string(),
        );
    }

    if error_lower.contains("'end' expected") {
        return Some(
            "Missing 'end' keyword. Check that all if/for/while/function blocks are closed."
                .to_string(),
        );
    }

    if error_lower.contains("syntax error near") {
        return Some("Check for typos, missing commas, or incorrect Lua syntax.".to_string());
    }

    if error_lower.contains("table expected") {
        return Some("The module must return a table with 'packages' field.".to_string());
    }

    if error_lower.contains("access denied") {
        return Some("File access is restricted for security. Only /sys/, /proc/, and /etc/os-release are allowed.".to_string());
    }

    None
}

/// Clean up error message for display
fn clean_error_message(error: &str) -> String {
    // Remove the "runtime error: " prefix if present
    let cleaned = error
        .trim_start_matches("runtime error: ")
        .trim_start_matches("syntax error: ");

    // Truncate very long messages
    if cleaned.len() > 200 {
        format!("{}...", &cleaned[..200])
    } else {
        cleaned.to_string()
    }
}
