//! Settings -> Guide -> Features: a curated tour, sliced from
//! `docs/features.md`'s "Compositor" and "Desktop shell (mshell)"
//! sections (its other two `##` sections -- a table, and a list of
//! website links -- don't fit this renderer or this page; see the design
//! spec).

use super::markdown::{self, Block};
use relm4::gtk;
use relm4::gtk::prelude::*;

const FEATURES_MD: &str = include_str!("../../../../docs/features.md");
const INCLUDED_HEADINGS: &[&str] = &["Compositor", "Desktop shell (mshell)"];

pub fn build() -> gtk::Widget {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 16);
    page.set_margin_top(8);
    page.set_margin_start(4);
    page.set_margin_end(4);

    for heading in INCLUDED_HEADINGS {
        let Some(section_text) = slice_section(FEATURES_MD, heading) else {
            continue;
        };

        let heading_label = gtk::Label::new(None);
        heading_label.set_markup(&format!(
            "<span size=\"large\" weight=\"bold\">{}</span>",
            heading
        ));
        heading_label.set_halign(gtk::Align::Start);
        page.append(&heading_label);

        for block in markdown::transform(&section_text) {
            let label = gtk::Label::new(None);
            label.set_wrap(true);
            label.set_halign(gtk::Align::Start);
            label.set_xalign(0.0);
            match block {
                Block::Paragraph(markup) => label.set_markup(&markup),
                Block::Bullet(markup) => label.set_markup(&format!("•  {}", markup)),
            }
            page.append(&label);
        }
    }

    let scroller = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&page)
        .build();
    scroller.upcast()
}

/// Extract the body of the first `## {heading}` section (everything up to
/// the next `## ` line or end of file), excluding the heading line itself.
fn slice_section(markdown: &str, heading: &str) -> Option<String> {
    let marker = format!("## {heading}");
    let start = markdown.find(&marker)?;
    let after_heading = &markdown[start + marker.len()..];
    let body_start = after_heading.find('\n').map(|i| i + 1).unwrap_or(0);
    let body = &after_heading[body_start..];
    let end = body.find("\n## ").unwrap_or(body.len());
    Some(body[..end].trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slices_a_known_section_out_of_a_fixture() {
        let fixture = "\
# Title

## First

Body of first.

## Second

Body of second.
";
        assert_eq!(
            slice_section(fixture, "First"),
            Some("Body of first.".to_string())
        );
        assert_eq!(
            slice_section(fixture, "Second"),
            Some("Body of second.".to_string())
        );
    }

    #[test]
    fn missing_heading_returns_none() {
        let fixture = "## Only\n\nbody\n";
        assert_eq!(slice_section(fixture, "Nonexistent"), None);
    }

    #[test]
    fn the_two_included_headings_exist_in_the_real_file() {
        // Guards against `docs/features.md` being restructured (headings
        // renamed/removed) without this tab being updated to match --
        // fails loudly here instead of silently rendering an empty tab.
        for heading in INCLUDED_HEADINGS {
            assert!(
                slice_section(FEATURES_MD, heading).is_some(),
                "docs/features.md no longer has a \"## {heading}\" section \
                 -- update INCLUDED_HEADINGS in features_tab.rs"
            );
        }
    }
}
