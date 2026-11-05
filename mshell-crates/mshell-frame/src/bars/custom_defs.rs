//! Which bar pills a `bars.widgets.custom_widgets` edit affects.
//!
//! A custom pill reads its definition once, when the bar builds it
//! (`BarModel::build_widget` resolves `!Custom <name>` to the first
//! entry with that name). The slot guards only rebuild when a slot's
//! *list* changes, so editing a definition (label, exec, icon, menu, a
//! plugin's updated panel) needs its own rebuild of exactly the pills
//! that resolve to a changed definition.

use mshell_config::schema::bar_widgets::BarWidget;
use mshell_config::schema::config::CustomWidgetConfig;

/// The names whose resolved definition (first entry by name, the one
/// `build_widget` uses) differs between `old` and `new`, including
/// added and removed names. Sorted, no duplicates.
pub(crate) fn changed_names(old: &[CustomWidgetConfig], new: &[CustomWidgetConfig]) -> Vec<String> {
    let first = |defs: &'_ [CustomWidgetConfig], name: &str| -> Option<CustomWidgetConfig> {
        defs.iter().find(|c| c.name == name).cloned()
    };
    let mut names: Vec<String> = old.iter().chain(new).map(|c| c.name.clone()).collect();
    names.sort();
    names.dedup();
    names.retain(|n| first(old, n) != first(new, n));
    names
}

/// Whether the pill `kind` must be rebuilt for the `changed` names: a
/// `Custom` pill resolving to one, or a Hidden Bar drawer when one of
/// the pills it collapses (`drawer_contents`, every drawer's widgets)
/// is such a pill — the drawer builds its children itself.
pub(crate) fn affected(
    kind: &BarWidget,
    changed: &[String],
    drawer_contents: &[BarWidget],
) -> bool {
    let is_changed = |w: &BarWidget| matches!(w, BarWidget::Custom(n) if changed.contains(n));
    match kind {
        BarWidget::HiddenBar | BarWidget::HiddenBarNamed(_) => {
            drawer_contents.iter().any(is_changed)
        }
        other => is_changed(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(name: &str, label: &str) -> CustomWidgetConfig {
        CustomWidgetConfig {
            name: name.into(),
            label: label.into(),
            ..CustomWidgetConfig::default()
        }
    }

    #[test]
    fn only_a_changed_definition_counts() {
        let old = [def("vpn", "on"), def("mail", "3")];
        assert!(changed_names(&old, &old).is_empty(), "a re-notify");
        let new = [def("vpn", "off"), def("mail", "3")];
        assert_eq!(changed_names(&old, &new), ["vpn"]);
        // Added and removed names.
        let new = [def("mail", "3"), def("cpu", "x")];
        assert_eq!(changed_names(&old, &new), ["cpu", "vpn"]);
        // Reordering alone changes nothing.
        let new = [def("mail", "3"), def("vpn", "on")];
        assert!(changed_names(&old, &new).is_empty());
    }

    #[test]
    fn a_shadowed_duplicate_does_not_count() {
        // build_widget resolves the first entry; editing a later
        // duplicate leaves the pill as it is.
        let old = [def("vpn", "on"), def("vpn", "dup")];
        let new = [def("vpn", "on"), def("vpn", "edited")];
        assert!(changed_names(&old, &new).is_empty());
        let new = [def("vpn", "edited"), def("vpn", "dup")];
        assert_eq!(changed_names(&old, &new), ["vpn"]);
    }

    #[test]
    fn affected_pills_and_drawers() {
        let changed = vec!["vpn".to_string()];
        let vpn = BarWidget::Custom("vpn".into());
        let mail = BarWidget::Custom("mail".into());
        assert!(affected(&vpn, &changed, &[]));
        assert!(!affected(&mail, &changed, &[]));
        assert!(!affected(&BarWidget::Clock, &changed, &[]));
        assert!(affected(
            &BarWidget::HiddenBar,
            &changed,
            &[mail.clone(), vpn.clone()]
        ));
        assert!(affected(
            &BarWidget::HiddenBarNamed("tray".into()),
            &changed,
            std::slice::from_ref(&vpn)
        ));
        assert!(!affected(&BarWidget::HiddenBar, &changed, &[mail]));
        assert!(!affected(&vpn, &[], &[]));
    }
}
