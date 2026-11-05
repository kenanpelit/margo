# skip-cfg-test.awk — shared "skip test-only Rust items" prefix for the
# line-oriented gates (panic-ratchet.sh, design-lint.sh L10–L12).
#
# Prepend this program to a gate's own rules: every line belonging to an
# item marked #[cfg(test)], #[test] or #[tokio::test] is consumed with
# `next`, so the rules after it only ever see production code. Only the
# item's own span is skipped — a test-only `fn`, `use` or `mod tests;`
# ends at its closing brace or `;`, and the production code after it is
# scanned again (the old "everything after the first #[cfg(test)]" rule
# hid whole files).
#
# The item starts at the attribute: the rest of that line, then following
# lines, until its first `{` (braced item: track brace depth to the matching
# `}`) or its first `;` (non-braced item such as `use super::*;`). Braces
# inside string/char literals are not understood; a gate that needs that
# precision should not be grep-shaped.
#
# A gate needing a per-file reset of its own state must run that `FNR == 1`
# rule BEFORE this program (a skipped first line never reaches later rules).

function _sct_open(text) {
    _sct_depth = gsub(/\{/, "{", text) - gsub(/\}/, "}", text)
    _sct_arming = 0
    if (_sct_depth > 0) _sct_in_test = 1
}

FNR == 1 { _sct_in_test = 0; _sct_depth = 0; _sct_arming = 0 }

# Inside a braced test-only item: skip and track depth to its close.
_sct_in_test {
    _sct_line = $0
    _sct_depth += gsub(/\{/, "{", _sct_line) - gsub(/\}/, "}", _sct_line)
    if (_sct_depth <= 0) _sct_in_test = 0
    next
}

# After a test attribute: wait for the item's `{` or `;`.
_sct_arming {
    _sct_line = $0
    sub(/\/\/.*/, "", _sct_line)
    if (_sct_line ~ /\{/) {
        # A `;` before the first `{` ends a non-braced item first.
        if (_sct_line ~ /^[^{]*;/) { _sct_arming = 0; next }
        _sct_open(_sct_line)
        next
    }
    if (_sct_line ~ /;/) _sct_arming = 0
    next
}

{
    _sct_line = $0
    sub(/\/\/.*/, "", _sct_line)
}
_sct_line ~ /#\[(cfg\(test\)|test|tokio::test)\]/ {
    # The item may start on the attribute line itself:
    # `#[cfg(test)] fn helper() {}` or `#[cfg(test)] use super::*;`.
    match(_sct_line, /#\[(cfg\(test\)|test|tokio::test)\]/)
    _sct_rest = substr(_sct_line, RSTART + RLENGTH)
    if (_sct_rest ~ /^[^{]*;/) next
    if (_sct_rest ~ /\{/) { _sct_open(_sct_rest); next }
    _sct_arming = 1
    next
}
