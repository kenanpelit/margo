//! A markdown *subset* -> Pango markup transform, covering exactly the
//! constructs `docs/features.md`'s Compositor / Desktop shell sections use
//! today: **bold**, `code`, [links](url), and `- ` bullets. Not a general
//! CommonMark parser -- anything else (tables, nested lists, images,
//! headers inside the slice) passes through as literal escaped text
//! rather than being silently dropped, which is what the last test below
//! pins down.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Paragraph(String),
    Bullet(String),
}

pub fn transform(markdown: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut paragraph_lines: Vec<String> = Vec::new();
    let mut bullet_lines: Vec<String> = Vec::new();

    let flush_paragraph = |blocks: &mut Vec<Block>, lines: &mut Vec<String>| {
        if !lines.is_empty() {
            blocks.push(Block::Paragraph(inline(&lines.join(" "))));
            lines.clear();
        }
    };
    let flush_bullet = |blocks: &mut Vec<Block>, lines: &mut Vec<String>| {
        if !lines.is_empty() {
            blocks.push(Block::Bullet(inline(&lines.join(" "))));
            lines.clear();
        }
    };

    for raw_line in markdown.lines() {
        let line = raw_line.trim_end();

        if line.trim().is_empty() {
            flush_paragraph(&mut blocks, &mut paragraph_lines);
            flush_bullet(&mut blocks, &mut bullet_lines);
            continue;
        }

        if let Some(rest) = line.trim_start().strip_prefix("- ") {
            // A new bullet starts: flush whatever bullet was accumulating
            // (a previous item), then start this one. A bare paragraph
            // never continues into a bullet or vice versa.
            flush_bullet(&mut blocks, &mut bullet_lines);
            flush_paragraph(&mut blocks, &mut paragraph_lines);
            bullet_lines.push(rest.trim().to_string());
            continue;
        }

        if !bullet_lines.is_empty() && line.starts_with("  ") {
            // Indented continuation of the current bullet.
            bullet_lines.push(line.trim().to_string());
            continue;
        }

        flush_bullet(&mut blocks, &mut bullet_lines);
        paragraph_lines.push(line.trim().to_string());
    }
    flush_bullet(&mut blocks, &mut bullet_lines);
    flush_paragraph(&mut blocks, &mut paragraph_lines);

    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_paragraph() {
        let out = transform("Just plain text.");
        assert_eq!(out, vec![Block::Paragraph("Just plain text.".to_string())]);
    }

    #[test]
    fn bold_becomes_pango_b() {
        let out = transform("Some **bold** text.");
        assert_eq!(
            out,
            vec![Block::Paragraph("Some <b>bold</b> text.".to_string())]
        );
    }

    #[test]
    fn code_becomes_pango_tt() {
        let out = transform("Run `mctl reload`.");
        assert_eq!(
            out,
            vec![Block::Paragraph("Run <tt>mctl reload</tt>.".to_string())]
        );
    }

    #[test]
    fn link_becomes_pango_anchor() {
        let out = transform("See [Configuration](configuration.md).");
        assert_eq!(
            out,
            vec![Block::Paragraph(
                "See <a href=\"configuration.md\">Configuration</a>.".to_string()
            )]
        );
    }

    #[test]
    fn bullet_lines_become_bullet_blocks() {
        let out = transform("- First item\n- Second item");
        assert_eq!(
            out,
            vec![
                Block::Bullet("First item".to_string()),
                Block::Bullet("Second item".to_string()),
            ]
        );
    }

    #[test]
    fn multiline_bullet_continuation_joins_with_a_space() {
        // features.md wraps long bullets onto an indented continuation
        // line, matching normal markdown paragraph-wrap conventions.
        let out = transform("- First line of the bullet\n  continues here.");
        assert_eq!(
            out,
            vec![Block::Bullet(
                "First line of the bullet continues here.".to_string()
            )]
        );
    }

    #[test]
    fn blank_line_separates_paragraphs() {
        let out = transform("First paragraph.\n\nSecond paragraph.");
        assert_eq!(
            out,
            vec![
                Block::Paragraph("First paragraph.".to_string()),
                Block::Paragraph("Second paragraph.".to_string()),
            ]
        );
    }

    #[test]
    fn unhandled_construct_passes_through_as_literal_text() {
        // A table row -- not one of the four handled constructs. Must not
        // panic or vanish; it renders as plain (Pango-escaped) text.
        let out = transform("| Tool | Role |");
        assert_eq!(out, vec![Block::Paragraph("| Tool | Role |".to_string())]);
    }

    #[test]
    fn angle_brackets_in_plain_text_are_escaped() {
        // Guards against literal "<" / ">" / "&" in the source breaking
        // the Pango markup the transform hands to GtkLabel::set_markup.
        let out = transform("Use `<tag>` syntax & such.");
        assert_eq!(
            out,
            vec![Block::Paragraph(
                "Use <tt>&lt;tag&gt;</tt> syntax &amp; such.".to_string()
            )]
        );
    }
}

/// Apply the four inline constructs (bold, code, link) plus Pango-markup
/// escaping, in one left-to-right pass over the line. Escaping happens
/// character-by-character as literal text is copied through; the three
/// constructs below emit their own literal `<...>` tags directly (not
/// escaped), which is safe because none of their *content* is re-escaped
/// after being placed inside the tag -- it goes through this same
/// character loop first.
fn inline(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '*' && chars.get(i + 1) == Some(&'*') {
            if let Some(end) = find_closing(&chars, i + 2, "**") {
                let inner: String = chars[i + 2..end].iter().collect();
                out.push_str("<b>");
                out.push_str(&inline(&inner));
                out.push_str("</b>");
                i = end + 2;
                continue;
            }
        }
        if chars[i] == '`' {
            if let Some(end) = find_closing(&chars, i + 1, "`") {
                let inner: String = chars[i + 1..end].iter().collect();
                out.push_str("<tt>");
                out.push_str(&escape(&inner));
                out.push_str("</tt>");
                i = end + 1;
                continue;
            }
        }
        if chars[i] == '[' {
            if let Some(close_bracket) = find_char(&chars, i + 1, ']') {
                if chars.get(close_bracket + 1) == Some(&'(') {
                    if let Some(close_paren) = find_char(&chars, close_bracket + 2, ')') {
                        let label: String = chars[i + 1..close_bracket].iter().collect();
                        let url: String = chars[close_bracket + 2..close_paren].iter().collect();
                        out.push_str(&format!(
                            "<a href=\"{}\">{}</a>",
                            escape(&url),
                            escape(&label)
                        ));
                        i = close_paren + 1;
                        continue;
                    }
                }
            }
        }
        escape_char_into(chars[i], &mut out);
        i += 1;
    }
    out
}

fn find_closing(chars: &[char], from: usize, needle: &str) -> Option<usize> {
    let needle: Vec<char> = needle.chars().collect();
    let mut i = from;
    while i + needle.len() <= chars.len() {
        if chars[i..i + needle.len()] == needle[..] {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn find_char(chars: &[char], from: usize, target: char) -> Option<usize> {
    (from..chars.len()).find(|&i| chars[i] == target)
}

fn escape(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        escape_char_into(c, &mut out);
    }
    out
}

fn escape_char_into(c: char, out: &mut String) {
    match c {
        '<' => out.push_str("&lt;"),
        '>' => out.push_str("&gt;"),
        '&' => out.push_str("&amp;"),
        _ => out.push(c),
    }
}
