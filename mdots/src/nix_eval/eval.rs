use anyhow::{anyhow, Context, Result};
use std::path::Path;
use std::process::Command;

use super::system_facts::SystemFacts;
use super::types::{NixConfigRaw, NixError, NixErrorKind, NixModuleRaw, NixValidationResult};

pub fn check_nix_installed() -> Result<()> {
    if !crate::nix::is_nix_installed() {
        return Err(anyhow!(
            "nix is not installed. Install it with: mdots nix install\n\
             Or visit: https://nixos.org/download.html"
        ));
    }
    Ok(())
}

pub fn evaluate_nix_file_to_json(
    path: &Path,
    system_facts: &SystemFacts,
) -> Result<serde_json::Value> {
    check_nix_installed()?;

    let facts_json = system_facts
        .to_json()
        .context("Failed to serialize system facts")?;

    let tmpdir = tempfile::Builder::new()
        .prefix("mdots-nix-eval-")
        .tempdir()
        .context("Failed to create temp dir")?;

    let facts_path = tmpdir.path().join("facts.json");
    std::fs::write(&facts_path, &facts_json).context("Failed to write system facts")?;

    let facts_path_str = facts_path.to_string_lossy().to_string();
    let file_path = path.to_string_lossy().to_string();

    let expr = format!(
        "let system = builtins.fromJSON (builtins.readFile {}); pkgs = import <nixpkgs> {{}}; in import {} {{ inherit system pkgs; }}",
        shell_quote(&facts_path_str),
        shell_quote(&file_path),
    );

    let output = Command::new(crate::nix::nix_command())
        .args(["eval", "--impure", "--json", "--expr", &expr])
        .output()
        .context("Failed to execute nix eval")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let (message, _line, hint) = parse_nix_error(&stderr);
        return Err(anyhow!(
            "Nix evaluation failed: {}{}",
            message,
            hint.map(|h| format!("\n  HINT: {}", h)).unwrap_or_default()
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value =
        serde_json::from_str(&stdout).context("Failed to parse nix eval output")?;

    Ok(value)
}

pub fn validate_nix_with_eval(path: &Path, system_facts: &SystemFacts) -> NixValidationResult {
    let mut result = NixValidationResult::new();

    if !path.exists() {
        result.add_error(NixError {
            kind: NixErrorKind::FileNotFound,
            message: format!("Nix module file not found: {:?}", path),
            line: None,
            hint: Some("Check that the file path is correct".to_string()),
        });
        return result;
    }

    if let Err(e) = check_nix_installed() {
        result.add_error(NixError {
            kind: NixErrorKind::NixNotInstalled,
            message: e.to_string(),
            line: None,
            hint: Some("Install nix: https://nixos.org/download.html".to_string()),
        });
        return result;
    }

    match evaluate_nix_file_to_json(path, system_facts) {
        Ok(value) => {
            validate_config_structure(&value, path, &mut result);
        }
        Err(e) => {
            let msg = e.to_string();
            let (message, line, hint) = parse_nix_error(&msg);
            result.add_error(NixError {
                kind: NixErrorKind::EvalError,
                message,
                line,
                hint,
            });
        }
    }

    result
}

fn validate_config_structure(
    value: &serde_json::Value,
    path: &Path,
    result: &mut NixValidationResult,
) {
    let obj = match value.as_object() {
        Some(o) => o,
        None => {
            result.add_error(NixError {
                kind: NixErrorKind::InvalidType,
                message: "Nix config must evaluate to an attribute set".to_string(),
                line: None,
                hint: Some("Use: { host = \"myhost\"; ... }".to_string()),
            });
            return;
        }
    };

    let valid_fields: &[&str] = &[
        "host",
        "description",
        "import",
        "enabled_modules",
        "packages",
        "flatpak_packages",
        "nix_packages",
        "exclude",
        "additional_packages",
        "auto_prune",
        "module_processing",
        "strict_package_order",
        "services",
        "enabled_service_profiles",
        "flatpak_scope",
        "update_hooks",
        "default_apps",
        "theming",
        "editor",
        "package_manager",
        "aur_helper",
        "sync_sudo",
        "auto_commit",
        "nix",
        "config_backups",
        "system_backups",
        "run_hooks_as_user",
    ];

    let module_valid_fields: &[&str] = &[
        "description",
        "packages",
        "flatpak_packages",
        "nix_packages",
        "conflicts",
        "services",
        "pre_install_hook",
        "post_install_hook",
        "hook_behavior",
        "pre_hook_behavior",
        "post_hook_behavior",
        "post_disable_hook",
        "post_disable_behavior",
        "run_hooks_as_user",
        "dotfiles",
        "dotfiles_sync",
        "author",
        "version",
        "category",
        "tags",
        "license",
        "upstream_url",
    ];

    let valid_fields_slice: &[&str] = if obj.contains_key("host") {
        valid_fields
    } else {
        module_valid_fields
    };

    for key in obj.keys() {
        if !valid_fields_slice.contains(&key.as_str()) {
            let suggestion = suggest_field_name(key, valid_fields_slice);
            if let Some(suggested) = suggestion {
                result.add_warning(format!(
                    "Unknown field '{}'. Did you mean '{}'?",
                    key, suggested
                ));
            } else {
                result.add_warning(format!("Unknown field '{}'", key));
            }
        }
    }

    if obj.contains_key("host") {
        if let Some(host) = obj.get("host") {
            if !host.is_string() {
                result.add_error(NixError {
                    kind: NixErrorKind::InvalidType,
                    message: "'host' field must be a string".to_string(),
                    line: None,
                    hint: Some("Use: host = \"myhost\";".to_string()),
                });
            }
        }
    } else {
        let path_str = path.to_string_lossy();
        if path_str.contains("/hosts/") || path_str.contains("\\hosts\\") {
            result.add_warning("Host config file has no 'host' field".to_string());
        }
    }

    if let Some(pkgs) = obj.get("packages") {
        if !pkgs.is_array() {
            result.add_error(NixError {
                kind: NixErrorKind::InvalidType,
                message: "'packages' must be a list".to_string(),
                line: None,
                hint: Some("Use: packages = [ \"vim\" \"git\" ];".to_string()),
            });
        }
    }
}

fn suggest_field_name(input: &str, valid: &[&str]) -> Option<String> {
    let input_lower = input.to_lowercase();
    for &field in valid {
        if field.contains(&input_lower) || input_lower.contains(field) {
            return Some(field.to_string());
        }
    }
    for &field in valid {
        if levenshtein_distance(&input_lower, field) <= 2 {
            return Some(field.to_string());
        }
    }
    None
}

// Matrix indices are needed on both axes, so range loops are intentional.
#[allow(clippy::needless_range_loop)]
fn levenshtein_distance(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let a_len = a_chars.len();
    let b_len = b_chars.len();
    if a_len == 0 {
        return b_len;
    }
    if b_len == 0 {
        return a_len;
    }
    let mut matrix = vec![vec![0; b_len + 1]; a_len + 1];
    for i in 0..=a_len {
        matrix[i][0] = i;
    }
    for j in 0..=b_len {
        matrix[0][j] = j;
    }
    for i in 1..=a_len {
        for j in 1..=b_len {
            let cost = if a_chars[i - 1] == b_chars[j - 1] {
                0
            } else {
                1
            };
            matrix[i][j] = std::cmp::min(
                std::cmp::min(matrix[i - 1][j] + 1, matrix[i][j - 1] + 1),
                matrix[i - 1][j - 1] + cost,
            );
        }
    }
    matrix[a_len][b_len]
}

fn parse_nix_error(error: &str) -> (String, Option<u32>, Option<String>) {
    let line = extract_nix_line_number(error);
    let hint = generate_nix_error_hint(error);
    let message = clean_nix_error_message(error);
    (message, line, hint)
}

fn extract_nix_line_number(error: &str) -> Option<u32> {
    let re = regex::Regex::new(r":(\d+):").ok()?;
    re.captures(error)
        .and_then(|caps| caps.get(1))
        .and_then(|m| m.as_str().parse().ok())
}

fn generate_nix_error_hint(error: &str) -> Option<String> {
    let e = error.to_lowercase();
    if e.contains("syntax error") || e.contains("unexpected") {
        return Some("Check for missing semicolons, brackets, or incorrect Nix syntax".to_string());
    }
    if e.contains("undefined variable") {
        return Some(
            "Make sure all referenced variables are defined. Use 'system.*' for system facts"
                .to_string(),
        );
    }
    if e.contains("cannot coerce") || e.contains("expected") {
        return Some(
            "Type mismatch. Nix values: strings, integers, booleans, lists, attribute sets"
                .to_string(),
        );
    }
    if e.contains("permission denied") || e.contains("access denied") {
        return Some("File permission issue. Check that the nix daemon is running".to_string());
    }
    None
}

fn clean_nix_error_message(error: &str) -> String {
    let cleaned = error
        .trim_start_matches("error: ")
        .trim_start_matches("warning: ");
    if cleaned.len() > 300 {
        format!("{}...", &cleaned[..300])
    } else {
        cleaned.to_string()
    }
}

fn shell_quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

pub fn parse_nix_config(value: serde_json::Value) -> Result<NixConfigRaw> {
    serde_json::from_value(value).map_err(|e| anyhow!("Failed to parse nix config JSON: {}", e))
}

pub fn parse_nix_module(value: serde_json::Value) -> Result<NixModuleRaw> {
    serde_json::from_value(value).map_err(|e| anyhow!("Failed to parse nix module JSON: {}", e))
}

pub fn detect_pointer_nix_config(
    path: &Path,
    system_facts: &SystemFacts,
) -> Result<Option<String>> {
    if !path.exists() {
        return Ok(None);
    }

    if check_nix_installed().is_err() {
        return Ok(None);
    }

    match evaluate_nix_file_to_json(path, system_facts) {
        Ok(value) => {
            if let Some(obj) = value.as_object() {
                if obj.len() == 1 && obj.contains_key("host") {
                    return Ok(obj
                        .get("host")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()));
                }
            }
            Ok(None)
        }
        Err(_) => Ok(None),
    }
}
