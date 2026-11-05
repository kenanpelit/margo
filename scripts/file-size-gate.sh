#!/usr/bin/env bash
# Every first-party Rust file is capped at 2000 lines unless explicitly listed
# in file-size-limits.txt. Raising/removing a rule requires FILE-SIZE-RAISE:
# <reason> on the commit (including merge resolutions). Malformed rules fail.
# BASE defaults to FILE_SIZE_GATE_BASE, PANIC_RATCHET_BASE, then origin/main.
set -euo pipefail
exec python3 "$(dirname "$0")/file-size-gate.py" "$@"
