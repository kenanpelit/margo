#!/usr/bin/env bash
# design-lint.sh — mechanical enforcement of DESIGN.md §15 (W1.7).
#
# Each rule is a "this pattern must NOT appear" grep over mshell-crates/;
# a clean tree prints nothing and exits 0. The rule text, rationale and
# the carved exceptions live in mshell-crates/mshell-frame/DESIGN.md §15 —
# if a rule false-positives on legitimate code, tighten the grep or carve
# the exception THERE first, then mirror it here. Don't loosen a rule to
# make CI green.
#
# Covered: L1–L7 (all hard gates — the tree was audited clean when this
# gate shipped). L8 (main-thread blocking) is advisory-only per DESIGN.md
# (grep can't prove the thread) and L9 (fmt/clippy) already has its own
# CI steps.
set -uo pipefail
cd "$(dirname "$0")/.."

scss_styled="mshell-crates/mshell-style/scss/03-primitives mshell-crates/mshell-style/scss/04-components"
scss_all="mshell-crates/mshell-style/scss"
fail=0

# Strip line comments (// … and single-line /* … */) so commented-out
# examples and rule discussions never trip the gates.
strip_comments() {
    sed -e 's|//.*||' -e 's|/\*.*\*/||'
}

report() { # $1 = rule id, $2 = rule title, $3 = violations (may be empty)
    if [ -n "$3" ]; then
        echo "FAIL $1 — $2"
        echo "$3" | sed 's/^/    /'
        echo "    (rule + exceptions: mshell-crates/mshell-frame/DESIGN.md §15)"
        fail=1
    fi
}

# L1 — no hardcoded hex outside 01-tokens (matugen owns colour).
# Exceptions: comments. Even var(--x, #fallback) is forbidden — matugen
# always defines the token.
hits=$(grep -rnE '#[0-9a-fA-F]{3,8}\b' $scss_styled 2>/dev/null \
    | strip_comments | grep -E '#[0-9a-fA-F]{3,8}\b' || true)
report "L1" "hardcoded hex colour (use a matugen token)" "$hits"

# L2 — no raw px for spacing / radius / font sizes.
# Exceptions: 0–3px hairline/sub-grid micro-insets, in shorthand too
# (the spacing scale starts at --space-1 = 4px, so there is no sub-4
# token); calc()/var() expressions anchored to a token; and dimension
# properties (min-width/height, -gtk-icon-size, …) which the grep
# doesn't target. The awk pass flags a line only if it carries a raw
# px value ≥ 4.
hits=$(grep -rnE '(padding|margin|gap|border-radius|font-size)[^;{]*:[^;{]*[0-9]+px' $scss_styled 2>/dev/null \
    | strip_comments \
    | grep -E '(padding|margin|gap|border-radius|font-size)[^;{]*:[^;{]*[0-9]+px' \
    | grep -v 'calc(' | grep -v 'var(' \
    | awk '{ bad = 0; line = $0
             while (match(line, /[0-9]+px/)) {
                 n = substr(line, RSTART, RLENGTH); sub(/px/, "", n)
                 if (n + 0 >= 4) bad = 1
                 line = substr(line, RSTART + RLENGTH)
             }
             if (bad) print $0 }' || true)
report "L2" "raw px spacing/radius/font ≥4 (use --space-*/--radius-*/--font-*)" "$hits"

# L3 — --radius-widget / --radius-window only in bar-widget / frame styles.
# Exceptions: *bar_widget* files (those ARE the .ok-bar-widget rules),
# _bar.scss / _frame.scss, and comments.
hits=$(grep -rn 'radius-widget\|radius-window' mshell-crates/mshell-style/scss/04-components 2>/dev/null \
    | grep -v 'bar_widget' | grep -vE '_(bar|frame)\.scss' \
    | strip_comments | grep -E 'radius-(widget|window)' || true)
report "L3" "--radius-widget/--radius-window outside bar/frame styles" "$hits"

# L4 — no literal transition duration; durations come from --motion-*.
# Exception: a trailing stagger *delay* after var(--ease…) is allowed,
# which is why the gate only fires on transition lines that never
# reference var(--motion at all.
hits=$(grep -rnE 'transition:[^;]*[0-9]+(ms|s)\b' $scss_all 2>/dev/null \
    | strip_comments | grep -E 'transition:' \
    | grep -v 'var(--motion' || true)
report "L4" "literal transition duration (use var(--motion-*))" "$hits"

# L5 — no gtk::Popover as a bar widget's primary surface (layer-shell
# menus only). Exception: right-click context menus via PopoverMenu.
hits=$(grep -rn 'Popover::new\|set_popover' mshell-crates/mshell-frame/src/bars/bar_widgets 2>/dev/null \
    | strip_comments | grep -E 'Popover::new|set_popover' \
    | grep -v 'PopoverMenu' || true)
report "L5" "gtk::Popover as a bar-widget primary surface (use a layer-shell menu)" "$hits"

# L6 — no add_css_class("") (use set_css_classes(&[]) for "none").
hits=$(grep -rnE 'add_css_class\(\s*""' --include='*.rs' mshell-crates mshell/src 2>/dev/null \
    | strip_comments | grep -E 'add_css_class\(\s*""' || true)
report "L6" 'add_css_class("") (use set_css_classes(&[]))' "$hits"

# L7 — no GtkDragSource/DropTarget for settings list reorder (use the
# shared reorder_dnd GestureDrag helper).
hits=$(grep -rn 'DragSource\|DropTarget' mshell-crates/mshell-settings/src 2>/dev/null \
    | strip_comments | grep -E 'DragSource|DropTarget' || true)
report "L7" "DragSource/DropTarget row reorder (use reorder_dnd)" "$hits"

if [ "$fail" -ne 0 ]; then
    exit 1
fi
echo "design-lint OK: L1–L7 clean."
