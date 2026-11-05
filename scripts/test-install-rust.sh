#!/usr/bin/env bash
# Tests for install.sh's ensure_rust (Debian path), with rustup / rustc /
# cargo / curl stubbed on PATH. The installer must build with the toolchain
# rust-toolchain.toml pins and must never change the user's global rustup
# default (`rustup default …`, or a bootstrap that sets one).
#   ./scripts/test-install-rust.sh
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/.." && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

fail=0
check() { # name condition-exit
    if [ "$2" = 0 ]; then echo "ok   $1"; else echo "FAIL $1"; fail=1; fi
}
pin="$(sed -n 's/^channel = "\(.*\)"$/\1/p' "$root/rust-toolchain.toml")"
min="$(sed -n 's/^rust-version = "1\.\([0-9]\+\).*/\1/p' "$root/Cargo.toml")"

# install.sh minus its trailing `main "$@"`, next to the files it reads.
sed '$d' "$root/install.sh" > "$tmp/install.sh"
cp "$root/Cargo.toml" "$root/rust-toolchain.toml" "$tmp/"

stub() { # dir name [body]
    mkdir -p "$1"
    printf '#!/bin/sh\necho "%s $*" >> "%s/calls"\n%s\n' "$2" "$tmp" "${3:-}" > "$1/$2"
    chmod +x "$1/$2"
}
# A PATH with only the basics install.sh needs, plus the stubs in $1.
run_ensure_rust() { # stub-dir
    : > "$tmp/calls"
    mkdir -p "$tmp/sys"
    for t in sh sed cat mkdir chmod printf readlink dirname; do ln -sf "$(command -v "$t")" "$tmp/sys/$t"; done
    ( PATH="$1:$tmp/sys" HOME="$tmp/home"
      # shellcheck disable=SC1091
      . "$tmp/install.sh"; ensure_rust ) >/dev/null 2>&1 || echo "ensure_rust failed" >> "$tmp/calls"
}
no_default_change() { ! grep -qE '^rustup default|^rustup-init .*--default-toolchain [^n]' "$tmp/calls"; }
installs_pin() { grep -qE "^rustup toolchain install .*\b${pin//./\\.}\$" "$tmp/calls"; }
ok() { if "$@"; then echo 0; else echo 1; fi; }

# 1. rustup present, default toolchain older than rust-version.
rm -rf "$tmp/b1"; stub "$tmp/b1" rustup; stub "$tmp/b1" cargo
stub "$tmp/b1" rustc 'echo "rustc 1.70.0 (old)"'
run_ensure_rust "$tmp/b1"
check "rustup + old default: installs the pinned $pin" "$(ok installs_pin)"
check "rustup + old default: leaves the default alone" "$(ok no_default_change)"

# 2. rustup present, default toolchain recent enough.
rm -rf "$tmp/b2"; stub "$tmp/b2" rustup; stub "$tmp/b2" cargo
stub "$tmp/b2" rustc "echo \"rustc 1.$((min + 5)).0 (new)\""
run_ensure_rust "$tmp/b2"
check "rustup + recent default: installs the pinned $pin" "$(ok installs_pin)"
check "rustup + recent default: leaves the default alone" "$(ok no_default_change)"

# 3. No rustup, a recent system toolchain: used as-is, nothing installed.
rm -rf "$tmp/b3"; stub "$tmp/b3" cargo
stub "$tmp/b3" rustc "echo \"rustc 1.$((min + 5)).0 (system)\""
stub "$tmp/b3" curl
run_ensure_rust "$tmp/b3"
check "system toolchain: no rustup bootstrap" "$(ok eval '! grep -qE "^(curl|rustup)" "$tmp/calls"')"

# 4. Nothing installed: bootstrap rustup without a default, then the pin.
rm -rf "${tmp:?}/b4" "${tmp:?}/home"
# The stub curl prints a rustup-init that installs stub rustup/cargo into
# ~/.cargo/bin and the env file install.sh sources.
mkdir -p "$tmp/b4"
cat > "$tmp/b4/curl" <<EOF
#!/bin/sh
echo "curl \$*" >> "$tmp/calls"
cat <<'INIT'
echo "rustup-init \$*" >> "$tmp/calls"
mkdir -p "\$HOME/.cargo/bin"
for t in rustup cargo; do
  printf '#!/bin/sh\necho "%s \$*" >> "$tmp/calls"\n' "\$t" > "\$HOME/.cargo/bin/\$t"
  chmod +x "\$HOME/.cargo/bin/\$t"
done
echo 'PATH="\$HOME/.cargo/bin:\$PATH"' > "\$HOME/.cargo/env"
INIT
EOF
chmod +x "$tmp/b4/curl"
run_ensure_rust "$tmp/b4"
check "bootstrap: runs rustup-init" "$(ok grep -q '^rustup-init ' "$tmp/calls")"
check "bootstrap: sets no default toolchain" "$(ok grep -qE '^rustup-init .*--default-toolchain none' "$tmp/calls")"
check "bootstrap: installs the pinned $pin" "$(ok installs_pin)"
check "bootstrap: succeeds" "$(ok eval '! grep -q "ensure_rust failed" "$tmp/calls"')"

exit "$fail"
