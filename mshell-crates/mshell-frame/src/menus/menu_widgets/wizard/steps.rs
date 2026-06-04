//! Ordered list of wizard steps that actually apply to this machine.
//! `StepKind::child_name()` is the static `gtk::Stack` child name set up
//! in the `view!` macro; `build_steps()` filters the full flow down to
//! the applicable steps (hardware-aware), preserving visit order.

use super::hw_info::HwInfo;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StepKind {
    Welcome,
    Theme,
    Keyboard,
    Touchpad,
    Wifi,
    Wallpaper,
    Bar,
    Display,
    Power,
    Twilight,
    Review,
}

impl StepKind {
    /// The static `gtk::Stack` child name (must match the `add_named`
    /// indices in the `view!` macro).
    pub(crate) fn child_name(self) -> &'static str {
        match self {
            StepKind::Welcome => "0",
            StepKind::Theme => "1",
            StepKind::Keyboard => "2",
            StepKind::Touchpad => "3",
            StepKind::Wifi => "4",
            StepKind::Wallpaper => "5",
            StepKind::Bar => "6",
            StepKind::Display => "7",
            StepKind::Power => "8",
            StepKind::Twilight => "9",
            StepKind::Review => "10",
        }
    }

    /// Short label for the Review "edit" jump-buttons.
    pub(crate) fn label(self) -> &'static str {
        match self {
            StepKind::Welcome => "Profile",
            StepKind::Theme => "Theme",
            StepKind::Keyboard => "Keyboard",
            StepKind::Touchpad => "Touchpad",
            StepKind::Wifi => "Wi-Fi",
            StepKind::Wallpaper => "Wallpaper",
            StepKind::Bar => "Bar",
            StepKind::Display => "Display",
            StepKind::Power => "Power",
            StepKind::Twilight => "Night light",
            StepKind::Review => "Review",
        }
    }
}

/// Visit order: Welcome → Theme → Keyboard → [Touchpad] → [Display] →
/// [Power] → Twilight → [Wi-Fi] → Wallpaper → Bar → Review. Hardware-
/// gated steps are dropped when irrelevant.
pub(crate) fn build_steps(hw: &HwInfo) -> Vec<StepKind> {
    let mut v = vec![StepKind::Welcome, StepKind::Theme, StepKind::Keyboard];
    if hw.has_touchpad {
        v.push(StepKind::Touchpad);
    }
    if hw.monitor_count > 1 {
        v.push(StepKind::Display);
    }
    if hw.has_battery {
        v.push(StepKind::Power);
    }
    v.push(StepKind::Twilight);
    if hw.has_wifi {
        v.push(StepKind::Wifi);
    }
    v.push(StepKind::Wallpaper);
    v.push(StepKind::Bar);
    v.push(StepKind::Review);
    v
}

#[cfg(test)]
mod tests {
    use super::super::hw_info::HwInfo;
    use super::*;

    fn laptop() -> HwInfo {
        HwInfo {
            has_touchpad: true,
            has_wifi: true,
            has_battery: true,
            monitor_count: 2,
        }
    }
    fn desktop() -> HwInfo {
        HwInfo {
            has_touchpad: false,
            has_wifi: false,
            has_battery: false,
            monitor_count: 1,
        }
    }

    #[test]
    fn laptop_includes_all_applicable_steps() {
        let s = build_steps(&laptop());
        assert!(s.contains(&StepKind::Touchpad));
        assert!(s.contains(&StepKind::Wifi));
        assert!(s.contains(&StepKind::Power));
        assert!(s.contains(&StepKind::Display));
        assert_eq!(s.first(), Some(&StepKind::Welcome));
        assert_eq!(s.last(), Some(&StepKind::Review));
    }

    #[test]
    fn desktop_skips_hardware_specific_steps() {
        let s = build_steps(&desktop());
        assert!(!s.contains(&StepKind::Touchpad), "no touchpad on desktop");
        assert!(!s.contains(&StepKind::Wifi), "no wifi card");
        assert!(!s.contains(&StepKind::Power), "no battery");
        assert!(!s.contains(&StepKind::Display), "single monitor");
        assert!(s.contains(&StepKind::Theme));
        assert!(s.contains(&StepKind::Twilight));
    }

    #[test]
    fn child_names_are_unique_and_match_index_map() {
        assert_eq!(StepKind::Welcome.child_name(), "0");
        assert_eq!(StepKind::Display.child_name(), "7");
        assert_eq!(StepKind::Power.child_name(), "8");
        assert_eq!(StepKind::Twilight.child_name(), "9");
        assert_eq!(StepKind::Review.child_name(), "10");
    }
}
