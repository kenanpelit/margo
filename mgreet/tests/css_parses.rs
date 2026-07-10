//! The stylesheet must parse in the GTK that will render it.
//!
//! A CSS declaration GTK cannot parse is dropped with a warning nobody reads,
//! and the greeter comes up looking exactly as it did before — which is a very
//! expensive way to discover that `@keyframes` or a composite `transform` was
//! not supported. Load the real sheet through a real `GtkCssProvider` and fail
//! on the first parsing error.
//!
//! Skipped where GTK cannot open a display (headless CI). No window is ever
//! created; this only touches the CSS machinery.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::CssProvider;

const STYLE_CSS: &str = include_str!(concat!(env!("OUT_DIR"), "/style.css"));

#[test]
fn the_greeter_stylesheet_parses_without_error() {
    if gtk4::init().is_err() {
        eprintln!("no display; skipping the CSS parse check");
        return;
    }

    let errors: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let provider = CssProvider::new();
    {
        let errors = errors.clone();
        provider.connect_parsing_error(move |_, section, error| {
            errors
                .borrow_mut()
                .push(format!("{}: {error}", section.to_str()));
        });
    }
    provider.load_from_string(STYLE_CSS);

    let errors = errors.borrow();
    assert!(
        errors.is_empty(),
        "GTK rejected the stylesheet:\n{errors:#?}"
    );
}
