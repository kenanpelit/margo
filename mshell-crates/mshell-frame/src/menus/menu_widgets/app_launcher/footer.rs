//! The launcher's footer and empty state: the result count, the
//! keybind hint strip (only what the selection can do right now) and the
//! "no matches" headline.

use super::app_launcher::AppLauncherModel;
use relm4::gtk;
use relm4::gtk::prelude::*;

/// One keybind hint = visible key chip + caption + whether it
/// applies to the current selection. Always-rendered hints have
/// `applicable: true` regardless of state; contextual hints (Pin /
/// Remove / Alt) flip their `applicable` flag based on the
/// selected row's capabilities.
///
/// We render *every* hint every time — the inapplicable ones go
/// to `opacity: 0` instead of being removed. That keeps the
/// strip's natural width identical across selections so the
/// launcher menu doesn't slide sideways when navigation toggles
/// the contextual chips.
struct BindHint {
    /// One or more keycaps in the combo — each becomes a `.kbd-key`
    /// cap in the shared keycap idiom (e.g. `&["Ctrl", "⇧", "P"]`).
    keys: &'static [&'static str],
    label: &'static str,
    applicable: bool,
}

/// (Re)render the keybind hint strip at the bottom of the launcher.
/// Walker-style: each hint is a small chip showing the key combo
/// followed by what it does. The chip set is **fixed** in size —
/// always 9 chips, in a stable order — so the strip's intrinsic
/// width never changes as the selection moves. Contextual chips
/// (Alt action / Pin·Unpin / Remove) fade to `opacity: 0` when
/// they don't apply rather than being removed; the slot is still
/// allocated.
pub(super) fn rebuild_binds_strip(strip: &gtk::FlowBox, model: &AppLauncherModel) {
    // Snapshot the selected row's capabilities so we can flip
    // contextual chips on/off without holding two runtime borrows.
    let selected = model
        .selected_id
        .as_ref()
        .and_then(|id| model.results.iter().find(|d| &d.item.id == id));
    let (has_alt, has_pin, has_delete) = if let Some(d) = selected {
        let item = &d.item;
        let rt = model.runtime.borrow();
        (
            rt.alt_action(item).is_some(),
            item.usage_key.is_some(),
            rt.can_delete(item),
        )
    } else {
        (false, false, false)
    };

    // Static caption — toggling between "Pin" and "Unpin" would
    // change the chip's natural width and slide every chip to its
    // right. The row's ★ glyph already shows whether the item is
    // pinned right now; the chip just labels what the keybind does
    // (toggle pin state).
    let pin_label: &'static str = "Pin";

    // Stable order matches the keyboard mental model: activation
    // first, then power keys, then panel-level binds (Exact /
    // Last / Close). Inapplicable chips reserve their slot via
    // opacity 0 so the strip's natural width is identical for
    // every selection.
    // The footer keeps to what the selection can do right now plus the
    // two panel keys; the rest (Ctrl+1…9, Ctrl+E, Ctrl+R, PgUp/PgDn) is
    // on the result count's tooltip, the numbered rows and the ~/=
    // toggle, so the strip stays one quiet line.
    let hints: [BindHint; 6] = [
        BindHint {
            keys: &["↵"],
            label: "Open",
            applicable: true,
        },
        BindHint {
            keys: &["Ctrl", "↵"],
            label: "Alt action",
            applicable: has_alt,
        },
        BindHint {
            keys: &["Ctrl", "⇧", "P"],
            label: pin_label,
            applicable: has_pin,
        },
        BindHint {
            keys: &["Ctrl", "Del"],
            label: "Remove",
            applicable: has_delete,
        },
        BindHint {
            keys: &["Tab"],
            label: "Category",
            applicable: true,
        },
        BindHint {
            keys: &["Esc"],
            label: "Close",
            applicable: true,
        },
    ];

    // Tear down old chips. GTK4 has no `clear()`.
    while let Some(child) = strip.first_child() {
        strip.remove(&child);
    }

    for hint in &hints {
        // Skip inapplicable contextual chips entirely. The FlowBox
        // reflows on its own, so we no longer need opacity-0 spacer
        // chips to keep a single-line strip's width stable.
        if !hint.applicable {
            continue;
        }

        // Shared real-keycap idiom (`_keycap.scss`): a `.kbd-chip`
        // grouping one `.kbd-key` cap per combo key, then a trailing
        // `.kbd-caption` describing the action.
        let chip = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        chip.add_css_class("kbd-chip");

        for cap in hint.keys {
            let key_lbl = gtk::Label::new(Some(cap));
            key_lbl.add_css_class("kbd-key");
            chip.append(&key_lbl);
        }

        // Full label, no ellipsize — the FlowBox wraps chips that
        // don't fit onto the next line instead of truncating them,
        // and a single chip's natural width never forces the panel
        // wider than its own ~one-chip minimum.
        let cap_lbl = gtk::Label::new(Some(hint.label));
        cap_lbl.add_css_class("kbd-caption");
        chip.append(&cap_lbl);

        strip.append(&chip);
    }
}

/// The footer's result count ("1 result", "12 results").
pub(super) fn result_count_label(n: usize) -> String {
    if n == 1 {
        "1 result".to_string()
    } else {
        format!("{n} results")
    }
}

/// The empty state's headline: what was searched for, or that the
/// category has nothing to list yet.
pub(super) fn empty_title(filter: &str) -> String {
    let query = filter.trim();
    if query.is_empty() {
        "Nothing to show here yet".to_string()
    } else {
        format!("No matches for \u{201c}{query}\u{201d}")
    }
}

#[cfg(test)]
mod tests {
    use super::{empty_title, result_count_label};

    #[test]
    fn result_count_reads_naturally() {
        assert_eq!(result_count_label(0), "0 results");
        assert_eq!(result_count_label(1), "1 result");
        assert_eq!(result_count_label(12), "12 results");
    }

    #[test]
    fn empty_state_names_the_query() {
        assert_eq!(empty_title("  "), "Nothing to show here yet");
        assert_eq!(empty_title(" fire "), "No matches for \u{201c}fire\u{201d}");
    }
}
