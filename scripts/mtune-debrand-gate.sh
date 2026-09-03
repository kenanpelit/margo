#!/usr/bin/env bash
# mtune-debrand-gate.sh — fail if any upstream *branding* survives in mtune/.
#
# Scope: the upstream project name ("amberol", any case) and the upstream
# reverse-DNS app-id namespace ("io.bassi" / "io/bassi"). NOT the per-file
# "SPDX-FileCopyrightText: ... Bassi" author headers — GPL-3.0 section 5
# requires those to be retained, so bare author names are deliberately not
# matched here.
set -euo pipefail
cd "$(dirname "$0")/.."

fail=0

# 1. Project name + app-id namespace anywhere under mtune/ (the licence texts
#    are exempt — they carry no upstream branding, just the GPL). Translator
#    team contact URLs (l10n.gnome.org / matrix.to) in .po headers are
#    attribution of who translated and are deliberately not matched.
if rg -n -i --hidden \
      -g '!licenses/**' -g '!target/**' \
      'amberol|io\.bassi|io/bassi' \
      mtune/ ; then
  echo "ERROR: upstream branding (name / app-id) found in mtune/" >&2
  fail=1
fi

if [ "$fail" -ne 0 ]; then
  exit 1
fi
echo "mtune de-brand gate: clean"
