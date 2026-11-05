//! The dispatch-action catalogue moved to `margo-config` (the config
//! validator needs it, and `margo-config` is the shared leaf crate).
//! Re-exported here so `mctl::actions::{ACTIONS, Group, all_names, …}`
//! keeps resolving.
//!
//! [`markdown`] renders the same catalogue for the docs: the dispatch-action
//! reference in `docs/configuration.md` and `wiki/Configuration.md` is this
//! output, and a test below fails when either page drifts from it.

pub use margo_config::actions::*;

/// Every group, in the order `mctl actions` and the docs list them.
pub const GROUPS: [Group; 8] = [
    Group::Tag,
    Group::Focus,
    Group::Layout,
    Group::Scroller,
    Group::Window,
    Group::Scratchpad,
    Group::Overview,
    Group::System,
];

/// First line of the generated action reference embedded in
/// `docs/configuration.md` and `wiki/Configuration.md`.
pub const MARKDOWN_BEGIN: &str = "<!-- BEGIN dispatch-actions: generated from \
     margo-config/src/actions.rs by `just docs-actions`; do not edit by hand -->";
/// Last line of the generated block.
pub const MARKDOWN_END: &str = "<!-- END dispatch-actions -->";

/// One markdown table cell: whitespace and newlines folded, `|` escaped,
/// and a `<` outside a code span written as `&lt;` so the renderer shows
/// `<vt>` instead of swallowing it as an HTML tag.
fn cell(text: &str) -> String {
    let folded = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out = String::with_capacity(folded.len());
    let mut in_code = false;
    for c in folded.chars() {
        match c {
            '`' => {
                in_code = !in_code;
                out.push(c);
            }
            '|' => out.push_str("\\|"),
            '<' if !in_code => out.push_str("&lt;"),
            _ => out.push(c),
        }
    }
    out
}

/// The whole catalogue as markdown: one `###` section per group, one
/// table row per action (canonical name + aliases, argument shape,
/// summary + detail). Wrapped in [`MARKDOWN_BEGIN`] / [`MARKDOWN_END`].
pub fn markdown() -> String {
    let mut out = String::new();
    out.push_str(MARKDOWN_BEGIN);
    out.push('\n');
    for group in GROUPS {
        let actions: Vec<_> = ACTIONS.iter().filter(|a| a.group == group).collect();
        if actions.is_empty() {
            continue;
        }
        out.push_str(&format!(
            "\n### {}\n\n| Action | Arg | Description |\n| --- | --- | --- |\n",
            group.label()
        ));
        for a in actions {
            let names: Vec<String> = std::iter::once(&a.name)
                .chain(a.aliases)
                .map(|n| format!("`{n}`"))
                .collect();
            let args = if a.args.is_empty() {
                String::new()
            } else {
                format!("`{}`", a.args)
            };
            let description = if a.detail.is_empty() {
                a.summary.to_string()
            } else {
                format!("{} {}", a.summary, a.detail)
            };
            out.push_str(&format!(
                "| {} | {} | {} |\n",
                cell(&names.join(", ")),
                cell(&args),
                cell(&description)
            ));
        }
    }
    out.push('\n');
    out.push_str(MARKDOWN_END);
    out
}

/// `doc` with its generated block (begin marker through end marker)
/// replaced by `block`; `None` when the markers are missing or out of order.
pub fn splice_markdown(doc: &str, block: &str) -> Option<String> {
    let start = doc.find(MARKDOWN_BEGIN)?;
    let end = start + doc[start..].find(MARKDOWN_END)? + MARKDOWN_END.len();
    Some(format!("{}{block}{}", &doc[..start], &doc[end..]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// The docs that embed the generated reference.
    const DOCS: [&str; 2] = ["docs/configuration.md", "wiki/Configuration.md"];

    #[test]
    fn markdown_lists_every_action_and_alias() {
        let md = markdown();
        for a in ACTIONS {
            for name in std::iter::once(&a.name).chain(a.aliases) {
                assert!(md.contains(&format!("`{name}`")), "{name} missing");
            }
        }
        let rows = md.lines().filter(|l| l.starts_with("| `")).count();
        assert_eq!(rows, ACTIONS.len(), "one row per catalogue entry");
    }

    #[test]
    fn cells_escape_pipes_and_fold_newlines() {
        assert_eq!(cell("a|b\n   c"), "a\\|b c");
    }

    #[test]
    fn cells_keep_angle_brackets_visible_outside_code() {
        // A bare `<vt>` would be swallowed as an HTML tag by the renderer.
        assert_eq!(
            cell("terminal <vt> or `<vt>`"),
            "terminal &lt;vt> or `<vt>`"
        );
    }

    #[test]
    fn splice_replaces_only_the_marked_block() {
        let doc = format!("intro\n{MARKDOWN_BEGIN}\nold\n{MARKDOWN_END}\noutro\n");
        assert_eq!(
            splice_markdown(&doc, "NEW").as_deref(),
            Some("intro\nNEW\noutro\n")
        );
        assert_eq!(splice_markdown("no markers", "NEW"), None);
        assert_eq!(
            splice_markdown(&format!("{MARKDOWN_END}\n{MARKDOWN_BEGIN}"), "NEW"),
            None
        );
    }

    /// Gate: every dispatch action is documented, straight from the
    /// catalogue. `MARGO_BLESS_DOCS=1` rewrites the docs instead (that is
    /// what `just docs-actions` runs).
    #[test]
    fn docs_embed_the_current_action_reference() -> std::io::Result<()> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
        let block = markdown();
        let bless = std::env::var_os("MARGO_BLESS_DOCS").is_some();
        for doc in DOCS {
            let path = root.join(doc);
            let text = std::fs::read_to_string(&path)?;
            let Some(updated) = splice_markdown(&text, &block) else {
                return Err(std::io::Error::other(format!(
                    "{doc}: missing the generated dispatch-action block markers"
                )));
            };
            if bless {
                std::fs::write(&path, updated)?;
            } else {
                assert!(
                    updated == text,
                    "{doc}: the dispatch-action reference is stale; run `just docs-actions`"
                );
            }
        }
        Ok(())
    }
}
