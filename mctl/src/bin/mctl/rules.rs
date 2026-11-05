//! `mctl rules`: which window rules match a client, and why.

use anyhow::{Result, bail};

pub(super) fn cmd_rules(
    config_override: Option<&std::path::Path>,
    appid: &str,
    title: &str,
    verbose: bool,
) -> Result<()> {
    use margo_config::{WindowRule, parse_config};

    let cfg_path = config_override
        .map(|p| p.to_path_buf())
        .or_else(|| {
            std::env::var_os("XDG_CONFIG_HOME")
                .map(|h| std::path::PathBuf::from(h).join("margo/config.conf"))
        })
        .unwrap_or_else(|| {
            std::path::PathBuf::from(format!(
                "{}/.config/margo/config.conf",
                std::env::var("HOME").unwrap_or_else(|_| ".".to_string())
            ))
        });

    if !cfg_path.exists() {
        bail!("config file not found: {}", cfg_path.display());
    }
    let cfg = parse_config(Some(&cfg_path))
        .map_err(|e| anyhow::anyhow!("parse {}: {e}", cfg_path.display()))?;

    // No filter args provided → behave like `niri msg windows` /
    // `hyprctl rules`: just dump every defined windowrule. Test
    // mode (with --appid/--title) only when the user explicitly
    // asks.
    let no_filter = appid.is_empty() && title.is_empty();
    println!("config: {}", cfg_path.display());
    if no_filter {
        println!(
            "─── all windowrules ({}) ─────────────────────",
            cfg.window_rules.len()
        );
        if cfg.window_rules.is_empty() {
            println!("  (no `windowrule = ...` lines defined)");
        } else {
            for r in &cfg.window_rules {
                print_rule(r);
            }
        }
        if verbose {
            println!(
                "\nTip: pass `--appid X` and/or `--title Y` to see which \
                 rules WOULD apply to a hypothetical window."
            );
        }
        return Ok(());
    }

    println!("query:  appid='{}' title='{}'\n", appid, title);

    // Re-implement the matcher locally — `WindowRule` matching lives
    // in `margo`'s state module, but the rules / patterns themselves
    // live in `margo-config`, which we have here. Reusing the same
    // regex semantics (`regex::Regex` with empty-pattern → match-all,
    // unanchored otherwise) keeps the verdict in lockstep with the
    // compositor's runtime decision.
    let mut matched: Vec<&WindowRule> = Vec::new();
    let mut rejected: Vec<(&WindowRule, &'static str)> = Vec::new();

    for rule in &cfg.window_rules {
        match classify_rule(rule, appid, title) {
            Verdict::Match => matched.push(rule),
            Verdict::Reject(reason) => rejected.push((rule, reason)),
        }
    }

    println!(
        "── matching ({} rule{}) ───────────────────────",
        matched.len(),
        if matched.len() == 1 { "" } else { "s" },
    );
    if matched.is_empty() {
        println!("  (none)");
    } else {
        for r in &matched {
            print_rule(r);
        }
    }

    if verbose && !rejected.is_empty() {
        println!(
            "\n── rejected ({} rule{}) ───────────────────────",
            rejected.len(),
            if rejected.len() == 1 { "" } else { "s" },
        );
        for (r, reason) in rejected {
            println!("  ✗ {}", reason);
            print_rule(r);
        }
    }
    Ok(())
}

enum Verdict {
    Match,
    Reject(&'static str),
}

fn classify_rule(rule: &margo_config::WindowRule, appid: &str, title: &str) -> Verdict {
    let pattern_match = |pat: &str, value: &str| -> bool {
        if pat.is_empty() {
            return true;
        }
        if value.is_empty() {
            return false;
        }
        match regex::Regex::new(pat) {
            Ok(rx) => rx.is_match(value),
            Err(_) => {
                // Fall back to substring match if the pattern won't
                // compile — same behaviour as the compositor's
                // `matches_rule_text`.
                let trimmed = pat.trim_start_matches('^').trim_end_matches('$');
                value.contains(trimmed)
            }
        }
    };

    if !pattern_match(rule.id.as_deref().unwrap_or(""), appid) {
        return Verdict::Reject("appid pattern miss");
    }
    if !pattern_match(rule.title.as_deref().unwrap_or(""), title) {
        return Verdict::Reject("title pattern miss");
    }
    if let Some(p) = rule.exclude_id.as_deref().filter(|p| !p.is_empty())
        && pattern_match(p, appid)
    {
        return Verdict::Reject("exclude_id matched");
    }
    if let Some(p) = rule.exclude_title.as_deref().filter(|p| !p.is_empty())
        && pattern_match(p, title)
    {
        return Verdict::Reject("exclude_title matched");
    }
    Verdict::Match
}

fn print_rule(rule: &margo_config::WindowRule) {
    let id = rule.id.as_deref().unwrap_or("");
    let title = rule.title.as_deref().unwrap_or("");
    let mut bits: Vec<String> = Vec::new();
    if !id.is_empty() {
        bits.push(format!("appid={}", id));
    }
    if !title.is_empty() {
        bits.push(format!("title={}", title));
    }
    if rule.tags != 0 {
        bits.push(format!("tags=0x{:x}", rule.tags));
    }
    if rule.width > 0 || rule.height > 0 {
        bits.push(format!("size={}x{}", rule.width, rule.height));
    }
    if rule.offset_x != 0 || rule.offset_y != 0 {
        bits.push(format!("offset={}+{}", rule.offset_x, rule.offset_y));
    }
    macro_rules! flag {
        ($field:ident, $name:literal) => {
            if let Some(v) = rule.$field {
                bits.push(format!("{}={}", $name, v));
            }
        };
    }
    flag!(is_floating, "isfloating");
    flag!(is_fullscreen, "isfullscreen");
    flag!(is_named_scratchpad, "isnamedscratchpad");
    flag!(no_border, "isnoborder");
    flag!(no_animation, "isnoanimation");
    flag!(no_blur, "noblur");
    flag!(no_focus, "nofocus");
    flag!(allow_csd, "allow_csd");
    flag!(block_out_from_screencast, "block_out_from_screencast");
    if let Some(v) = rule.scroller_proportion {
        bits.push(format!("scroller_proportion={}", v));
    }
    if !bits.is_empty() {
        println!("    {}", bits.join("  "));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(id: &str, title: &str) -> margo_config::WindowRule {
        margo_config::WindowRule {
            id: Some(id.into()),
            title: Some(title.into()),
            ..Default::default()
        }
    }

    fn verdict(r: &margo_config::WindowRule, appid: &str, title: &str) -> Result<(), &'static str> {
        match classify_rule(r, appid, title) {
            Verdict::Match => Ok(()),
            Verdict::Reject(why) => Err(why),
        }
    }

    #[test]
    fn rule_classification() {
        assert_eq!(verdict(&rule("^firefox$", ""), "firefox", "x"), Ok(()));
        assert_eq!(
            verdict(&rule("^firefox$", ""), "firefox-dev", "x"),
            Err("appid pattern miss")
        );
        assert_eq!(
            verdict(
                &rule("", "Picture-in-Picture"),
                "firefox",
                "Picture-in-Picture"
            ),
            Ok(())
        );
        assert_eq!(
            verdict(&rule("firefox", "^PiP$"), "firefox", "Mozilla"),
            Err("title pattern miss")
        );
        assert_eq!(
            verdict(&rule("kitty", ""), "", ""),
            Err("appid pattern miss"),
            "a pattern never matches an empty value"
        );
        // An invalid regex falls back to substring matching.
        assert_eq!(verdict(&rule("^foo(", ""), "xfoo(y", ""), Ok(()));
    }

    #[test]
    fn rule_exclusions() {
        let mut r = rule("firefox", "");
        r.exclude_title = Some("Private".into());
        assert_eq!(verdict(&r, "firefox", "Mozilla Firefox"), Ok(()));
        assert_eq!(
            verdict(&r, "firefox", "Private Browsing"),
            Err("exclude_title matched")
        );
        let mut r = rule("", "");
        r.exclude_id = Some("^steam".into());
        assert_eq!(verdict(&r, "steam_app_1", ""), Err("exclude_id matched"));
        r.exclude_id = Some(String::new());
        assert_eq!(
            verdict(&r, "steam_app_1", ""),
            Ok(()),
            "empty exclusion is ignored"
        );
    }
}
