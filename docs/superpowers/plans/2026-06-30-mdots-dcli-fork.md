# mdots — fork of dcli into the margo workspace — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Vendor the standalone `dcli` declarative package manager (~37k LOC) into the margo workspace as a top-level `mdots` crate, full functional surface preserved, adapted to margo's conventions and CI gates.

**Architecture:** `mdots/` is a self-contained binary crate (peer of `mctl`/`mplay`/`mvpn`), depending on no margo crate and depended on by none — a workspace member purely for unified build/CI/packaging. The fork is mechanical: copy `dcli/src` verbatim, rename the `dcli` identity to `mdots` (binary, config home, env vars, help/man/completion strings), then satisfy margo's `just check` gate.

**Tech Stack:** Rust (edition 2021 for this crate, workspace is 2024), clap + clap_complete + clap_mangen, mlua (lua54, vendored), ratatui + crossterm, serde_yaml, indicatif, walkdir, anyhow.

**Source of truth being forked:** `~/.kod/dcli` @ branch `main`, commit `09ed3c7`. The standalone `kenanpelit/dcli` repo is retired after this (manual GitHub step by the user — not part of this plan).

## Global Constraints

- **Licence:** `GPL-3.0-or-later`. No 0BSD headers, no "Don" author entry carried in (0BSD permits this).
- **Crate + binary name:** `mdots`. Top-level `mdots/`, **not** under `mshell-crates/`.
- **Config home:** `~/.config/mdots` (env override `MDOTS_CONFIG_DIR`). NOT under `~/.config/margo/`.
- **Compat:** ship `/usr/bin/dcli → mdots` symlink so `dcli sync` (the user's mshell pipewire-restart trigger) keeps working.
- **Scope:** full surface verbatim — pacman, nix/nix_eval, flatpak, Lua, ratatui TUI, services, SOPS/age secrets, AUR. No behaviour changes beyond what CI forces.
- **panic-ratchet is down-only:** `scripts/panic-baseline.txt` (currently `329`) may only be raised with a documented rationale in the same commit; it must equal the exact measured count afterward.
- **CI gate to pass:** `just check` = fmt + clippy `--all-targets -D warnings` + `scripts/panic-ratchet.sh` + design-lint + test.
- **No out-of-src assets:** dcli has zero `include_str!/include_bytes!`; vendoring `src/` + `Cargo.toml` is self-contained.
- **`--bd` / "BlackDon" init bootstrap** is an upstream feature referencing an external example repo — leave it functional and untouched; it is not part of the identity scrub.

---

### Task 1: Vendor source + create crate + workspace member (compiles)

Bring dcli's source in unchanged and make it build as `mdots`. Runtime identity (clap name, config dir) is still "dcli" at the end of this task — that is expected and fixed in Task 2/3.

**Files:**
- Create: `mdots/src/**` (copy of `~/.kod/dcli/src/**`, 79 files)
- Create: `mdots/Cargo.toml`
- Modify: `Cargo.toml` (root, `[workspace] members`)

**Interfaces:**
- Produces: a buildable `mdots` binary crate. Central config resolver `config::ConfigPaths::new()` at `mdots/src/config/mod.rs:1334` (env `DCLI_CONFIG_DIR` → else `~/.config/dcli`). Later tasks rename these.

- [ ] **Step 1: Copy the source tree**

```bash
cd /repo/archive/.kod/margo
mkdir -p mdots
cp -a ~/.kod/dcli/src mdots/src
```

- [ ] **Step 2: Write `mdots/Cargo.toml`**

Copy dcli's deps verbatim; change identity + opt into workspace lints. Write `mdots/Cargo.toml`:

```toml
[package]
name = "mdots"
version = "1.1.0"
edition = "2021"
rust-version = "1.74"
authors = ["Kenan Pelit <kenanpelit@gmail.com>"]
description = "Declarative package + dotfiles manager for the margo desktop (forked from dcli)"
license = "GPL-3.0-or-later"
repository = "https://github.com/kenanpelit/margo"

[[bin]]
name = "mdots"
path = "src/main.rs"

[lints]
workspace = true

[dependencies]
clap = { version = "4.5", features = ["derive", "cargo"] }
clap_complete = "4.5"
clap_mangen = "0.2"
serde = { version = "1.0", features = ["derive"] }
serde_yaml = "0.9"
serde_json = "1.0"
anyhow = "1.0"
thiserror = "1.0"
colored = "2.1"
env_logger = "0.11"
log = "0.4"
walkdir = "2.5"
glob = "0.3"
which = "7.0"
chrono = { version = "0.4", features = ["serde"] }
regex = "1.10"
hostname = "0.4"
indicatif = "0.17"
ratatui = "0.28"
crossterm = "0.28"
tempfile = "3.8"
mlua = { version = "0.10", features = ["lua54", "vendored", "serialize"] }
dirs = "5.0"

[profile.release]
opt-level = 3
lto = false
codegen-units = 1
strip = true
```

(Note: the `[profile.release]` block in a workspace member is ignored by cargo with a warning; leave it out if cargo complains. The workspace root owns profiles.)

- [ ] **Step 3: Add `mdots` to the workspace members**

Modify root `Cargo.toml` — add `"mdots",` to the top-level group in `[workspace] members` (next to `"mctl",`):

```toml
    "mctl",
    "mdots",
    "mkeys",
```

- [ ] **Step 4: Build**

Run: `cargo build -p mdots 2>&1 | tail -20`
Expected: `Finished` (warnings allowed at this stage). If `[profile.release]` triggers a hard error, delete that block from `mdots/Cargo.toml` and rebuild.

- [ ] **Step 5: Smoke-run (identity still "dcli" — expected)**

Run: `./target/debug/mdots --help | head -3`
Expected: prints help with the program name still shown as `dcli` (renamed in Task 3).

- [ ] **Step 6: Commit**

```bash
git add mdots Cargo.toml
git commit -m "feat(mdots): vendor dcli source as a workspace crate

Verbatim copy of dcli (kenanpelit/dcli @ 09ed3c7) src/ + Cargo.toml as a
top-level mdots crate. Relicensed GPL-3.0-or-later, opts into the
workspace clippy allow-list. Builds as -p mdots; runtime identity is
scrubbed dcli->mdots in following commits.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 2: Runtime identity — config home `~/.config/mdots`, env var, migration

The functional core of the rename: where mdots reads config, plus a one-shot migration so the user's existing `~/.config/dcli` manifest works on day one.

**Files:**
- Modify: `mdots/src/config/mod.rs` (resolver at lines ~1331–1366)
- Modify: `mdots/src/secrets.rs` (test literals `.config/dcli` at lines 538, 549, 558, 567, 568)

**Interfaces:**
- Consumes: `config::ConfigPaths::new()` from Task 1.
- Produces: config home resolves to `~/.config/mdots`; env override `MDOTS_CONFIG_DIR`; `migrate_legacy_dcli_dir()` runs once before resolution.

- [ ] **Step 1: Read the current resolver to anchor the edit**

Run: `sed -n '1325,1370p' mdots/src/config/mod.rs`
Expected: shows the `DCLI_CONFIG_DIR` branch and `PathBuf::from(&home).join(".config/dcli")` default (~line 1340).

- [ ] **Step 2: Rename env var + default path + add migration**

In `mdots/src/config/mod.rs`, in `ConfigPaths::new()`:
- `std::env::var("DCLI_CONFIG_DIR")` → `std::env::var("MDOTS_CONFIG_DIR")`
- `.join(".config/dcli")` → `.join(".config/mdots")`
- Immediately before computing `config_dir`, call a new migration helper.

Add this helper near the top of the resolver's `impl`/module (uses the existing `copy_dir_recursive` from `commands::migrate` — make it `pub(crate)` if not already):

```rust
/// One-shot: if the user still has a legacy ~/.config/dcli and no
/// ~/.config/mdots yet, copy it across so the rename is transparent.
/// Honoured only when MDOTS_CONFIG_DIR is unset.
fn migrate_legacy_dcli_dir(home: &str) {
    if std::env::var("MDOTS_CONFIG_DIR").is_ok() {
        return;
    }
    let new_dir = PathBuf::from(home).join(".config/mdots");
    let old_dir = PathBuf::from(home).join(".config/dcli");
    if new_dir.exists() || !old_dir.is_dir() {
        return;
    }
    if let Err(e) = crate::commands::migrate::copy_dir_recursive(&old_dir, &new_dir) {
        log::warn!("mdots: legacy ~/.config/dcli migration failed: {e}");
    } else {
        log::info!("mdots: migrated ~/.config/dcli -> ~/.config/mdots");
    }
}
```

Call `migrate_legacy_dcli_dir(&home);` right after `home` is read and before `config_dir` is computed.

- [ ] **Step 3: Fix the secrets path-jail test literals**

In `mdots/src/secrets.rs`, in the `#[cfg(test)]` module, replace `.config/dcli` with `.config/mdots` at the test assertions (lines ~538, 549, 558, 567, 568). The runtime jail uses `paths.config_dir` dynamically, so only the test literals change.

Run: `cd /repo/archive/.kod/margo && sed -i 's#\.config/dcli#.config/mdots#g' mdots/src/secrets.rs`
Then verify no functional (non-test) line was touched: `grep -n "config/mdots" mdots/src/secrets.rs` — all hits should be inside the test module.

- [ ] **Step 4: Build + run the secrets tests**

Run: `cargo test -p mdots secrets 2>&1 | tail -15`
Expected: secrets path-resolution tests PASS against `.config/mdots`.

- [ ] **Step 5: Verify config home resolves new + migrates**

```bash
cargo build -p mdots
rm -rf /tmp/mdots-mig-test && mkdir -p /tmp/mdots-mig-test/.config/dcli/modules
HOME=/tmp/mdots-mig-test ./target/debug/mdots status 2>&1 | head -5 || true
ls /tmp/mdots-mig-test/.config/mdots/ 2>&1
```
Expected: `~/.config/mdots/` now exists (migrated from the seeded legacy dir).

- [ ] **Step 6: Commit**

```bash
git add mdots/src/config/mod.rs mdots/src/secrets.rs
git commit -m "feat(mdots): config home ~/.config/mdots + MDOTS_CONFIG_DIR + legacy migration

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 3: CLI identity — clap name, about, completion, man

Make the user-visible program identity `mdots`: the clap command name (drives `--help`, the man page, and completion scripts), the about strings, and the completion generator's binary-name argument.

**Files:**
- Modify: `mdots/src/main.rs:28` (`#[command(name = "dcli")]`) + about string + doc-comments naming dcli
- Modify: `mdots/src/commands/completion.rs:14` (`generate(shell, &mut cmd, "dcli", out)`)

**Interfaces:**
- Consumes: clap `Cli` from Task 1.
- Produces: `mdots --help`, `mdots man`, `mdots completion <shell>` all self-identify as `mdots`.

- [ ] **Step 1: Rename the clap command + about**

In `mdots/src/main.rs`: `#[command(name = "dcli")]` → `#[command(name = "mdots")]`. Update the `about = "A declarative package management CLI tool for Linux"` if it says dcli (it doesn't, leave wording). Update any subcommand doc-comments that literally say "dcli" (e.g. `/// Initialize dcli configuration…` → `/// Initialize mdots configuration…`).

Run: `sed -i 's/name = "dcli"/name = "mdots"/' mdots/src/main.rs`
Then by hand fix the ~11 doc-comment "dcli" mentions: `grep -n "dcli" mdots/src/main.rs` and edit each `dcli` → `mdots`.

- [ ] **Step 2: Fix the completion generator binary name**

In `mdots/src/commands/completion.rs:14`: `generate(shell, &mut cmd, "dcli", out)` → `generate(shell, &mut cmd, "mdots", out)`.

Run: `sed -i 's/generate(shell, &mut cmd, "dcli"/generate(shell, \&mut cmd, "mdots"/' mdots/src/commands/completion.rs`

- [ ] **Step 3: Build + verify identity**

```bash
cargo build -p mdots
./target/debug/mdots --help | head -2
./target/debug/mdots completion zsh | head -3
./target/debug/mdots man | head -3
```
Expected: help line 1 names `mdots`; completion script references `mdots`; man `.TH "MDOTS"`.

- [ ] **Step 4: Update the completion/man unit tests if they assert "dcli"**

Run: `grep -n "dcli" mdots/src/commands/completion.rs mdots/src/commands/man.rs`
Edit any in-test assertion that expects the binary name `dcli` → `mdots`. Then:
Run: `cargo test -p mdots completion man 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add mdots/src/main.rs mdots/src/commands/completion.rs mdots/src/commands/man.rs
git commit -m "feat(mdots): rename CLI identity dcli->mdots (clap name, completion, man)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 4: Bulk literal sweep + repo-URL repoint + test fixups

Sweep the remaining ~700 cosmetic `dcli` literals (help text, comments, generated example-config strings, Lua module docs) to `mdots`, then fix the things a blanket replace must NOT blindly change: GitHub repo URLs and selfupdate.

**Files:**
- Modify: `mdots/src/**` (broad, identity strings)
- Modify: `mdots/src/commands/selfupdate.rs` (repo URL/release source)

**Interfaces:**
- Consumes: everything from Tasks 1–3.
- Produces: no remaining functional `dcli` references except deliberately-kept ones (`--bd`/BlackDon).

- [ ] **Step 1: Inventory before the sweep**

Run: `grep -rniIc "dcli" mdots/src | grep -v ':0$' | sort -t: -k2 -rn | head -30`
Note the URL-bearing files (`selfupdate.rs`, anything with `github.com/kenanpelit/dcli`) — handle them in Step 3, exclude from the blind sweep.

- [ ] **Step 2: Blanket lowercase/uppercase identity sweep, excluding URLs**

```bash
cd /repo/archive/.kod/margo
# Replace the word dcli -> mdots and Dcli -> Mdots, but first protect URLs.
grep -rln "github.com/kenanpelit/dcli" mdots/src   # list URL files to review later
# Sweep everything; URLs become kenanpelit/mdots and are corrected in Step 3.
grep -rl --null -i "dcli" mdots/src | xargs -0 sed -i \
  -e 's/dcli/mdots/g' -e 's/Dcli/Mdots/g' -e 's/DCLI/MDOTS/g'
```
Note: `DCLI_CONFIG_DIR` was already renamed in Task 2; this pass also catches any other `DCLI_*`.

- [ ] **Step 3: Repoint repo URLs (margo is the home now)**

`kenanpelit/mdots` does not exist. In every file the Step 1 list flagged (notably `mdots/src/commands/selfupdate.rs`), repoint the GitHub base from `kenanpelit/mdots` (the bad sweep result) to `kenanpelit/margo`:

```bash
grep -rl "kenanpelit/mdots" mdots/src | xargs sed -i 's#kenanpelit/mdots#kenanpelit/margo#g'
```
Then read `mdots/src/commands/selfupdate.rs` and confirm it still compiles sensibly: self-update now points at margo releases (which won't carry a standalone `mdots` asset), so the command degrades to a graceful "no update found"/error rather than updating. That is acceptable — install path is margo's `just`/PKGBUILD. Do NOT delete the command (full-surface verbatim).

- [ ] **Step 4: Confirm only intended `dcli` survivors remain**

Run: `grep -rni "dcli" mdots/src`
Expected survivors: only references to the upstream `--bd`/BlackDon bootstrap or historical "forked from dcli" provenance notes. Anything else: fix by hand.

- [ ] **Step 5: Build + run the full test suite**

```bash
cargo build -p mdots 2>&1 | tail -5
cargo test -p mdots 2>&1 | tail -20
```
Expected: builds; tests PASS (fix any test asserting the old `dcli` string/path).

- [ ] **Step 6: Commit**

```bash
git add mdots/src
git commit -m "feat(mdots): scrub remaining dcli identity strings; repoint repo URLs to margo

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 5: CI green-up — rustfmt + clippy `-D warnings`

Bring the vendored crate to margo's lint bar. Largest unknown; work incrementally.

**Files:** `mdots/src/**` (as clippy dictates)

- [ ] **Step 1: Format**

Run: `cargo fmt -p mdots`
Then: `git diff --stat mdots/`
Review the reformat is whitespace-only.

- [ ] **Step 2: Measure the clippy debt**

Run: `cargo clippy -p mdots --all-targets 2>&1 | grep -cE "^warning|^error"`
Expected: a number N (the debt). The crate opts into the workspace allow-list (`[lints] workspace = true`), so the noisy advisory lints are already silenced.

- [ ] **Step 3: Fix warnings to zero (idiomatic, not `#[allow]`-spam)**

Run: `cargo clippy -p mdots --all-targets 2>&1 | tail -60`
Work through each warning with the idiomatic fix (the user asked for a tasteful port, not minimal hacks). Only add a targeted `#[allow(clippy::lint)]` with a one-line reason where the lint is genuinely wrong for the code. Re-run until clean. Iterate in small commits if the debt is large.

- [ ] **Step 4: Verify clippy is green under `-D warnings`**

Run: `cargo clippy -p mdots --all-targets -- -D warnings 2>&1 | tail -5`
Expected: `Finished` with no error.

- [ ] **Step 5: Commit**

```bash
git add mdots/src
git commit -m "style(mdots): fmt + clippy -D warnings clean

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 6: panic-ratchet baseline

Raise the global baseline by exactly the vendored crate's measured panic-prone count.

**Files:** Modify `scripts/panic-baseline.txt`

- [ ] **Step 1: Measure the new total with the script's own rules**

Run: `./scripts/panic-ratchet.sh 2>&1 | head -3`
Expected: `FAIL: count rose above the baseline (NEW > 329)`. Note `NEW` (the printed count) — this already excludes test modules, `build.rs`, `tests/`, `benches/` exactly as the ratchet measures.

- [ ] **Step 2: Set the baseline to the measured count**

Run: `printf '%s\n' "<NEW>" > scripts/panic-baseline.txt` (substitute the number from Step 1).

- [ ] **Step 3: Verify the ratchet is green**

Run: `./scripts/panic-ratchet.sh`
Expected: `OK: at baseline.`

- [ ] **Step 4: Commit (with rationale — required by the ratchet)**

```bash
git add scripts/panic-baseline.txt
git commit -m "chore(panic-ratchet): raise baseline for vendored mdots crate

mdots is a standalone CLI binary — a panic there cannot take down the
compositor or the shell, unlike margo/mshell. Baseline raised from 329
to <NEW> for the ~116 unwrap/expect sites vendored from dcli. To be
ratcheted down over time as the crate is hardened.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 7: Build wiring — justfile

Make `mdots` build + install through the same path as the other CLI tools.

**Files:** Modify `justfile`

**Interfaces:**
- Consumes: the buildable `mdots` crate.
- Produces: `just dots` recipe; `mdots` folded into `just all`.

- [ ] **Step 1: Read the existing `cli` recipe + `all` target**

Run: `sed -n '43,52p' justfile`
Expected: the `cli:` recipe (cargo build `-p …` then `sudo install -m755 …`) and `all: margo shell cli`.

- [ ] **Step 2: Add a `dots` recipe**

After the `cli:` recipe in `justfile`, add:

```make
# Build + install mdots (declarative package/dotfiles manager).
dots:
    cargo build --release -p mdots
    sudo install -m755 target/release/mdots {{bindir}}/mdots
    sudo ln -sf mdots {{bindir}}/dcli
```

- [ ] **Step 3: Fold `dots` into `all`**

Change `all: margo shell cli` → `all: margo shell cli dots`.

- [ ] **Step 4: Verify the recipe builds (skip install)**

Run: `just --dry-run dots 2>&1 | head` and `cargo build --release -p mdots 2>&1 | tail -3`
Expected: dry-run lists the three commands; release build `Finished`.

- [ ] **Step 5: Commit**

```bash
git add justfile
git commit -m "build(mdots): add 'just dots' recipe + fold into 'just all'

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 8: Packaging — install.sh + PKGBUILD + generated man/completions

Ship `mdots` (and the `dcli` compat symlink) through margo's cross-distro installer and the Arch package, with a generated man page and shell completions committed where margo keeps them.

**Files:**
- Create: `man/mdots.1` (generated)
- Create: `contrib/completions/{mdots.bash,_mdots,mdots.fish}` (generated)
- Modify: `install.sh` (binary loop ~line 295, completion block ~384, build `-p` list ~258)
- Modify: `PKGBUILD` (build `-p` list ~230/262, package() bin loop ~282, completion block)

- [ ] **Step 1: Generate the man page + completions into margo's committed locations**

```bash
cd /repo/archive/.kod/margo
cargo build --release -p mdots
./target/release/mdots man > man/mdots.1
./target/release/mdots completion bash > contrib/completions/mdots.bash
./target/release/mdots completion zsh  > contrib/completions/_mdots
./target/release/mdots completion fish > contrib/completions/mdots.fish
head -1 man/mdots.1   # expect .TH "MDOTS" ...
```

- [ ] **Step 2: install.sh — add mdots to the build `-p` list**

In `install.sh` near line 258 (`-p mctl -p mlock -p mlayout …`), append `-p mdots`.

- [ ] **Step 3: install.sh — add mdots to the binary install loop + compat symlink**

In the `for bin in margo start-margo mctl … mplay \` loop (~line 295), add `mdots` to the list. After the loop body installs binaries, add the compat symlink:

```bash
  # dcli compatibility symlink (mdots was forked from dcli; keeps the
  # user's `dcli sync` muscle memory + mshell pipewire trigger working).
  ln -sf mdots "${DESTDIR}/usr/bin/dcli" 2>/dev/null || \
    sudo ln -sf mdots /usr/bin/dcli
```
(Match the surrounding install helper style — if the script uses `install_file`, mirror it; the symlink uses the same privilege path as the binaries above it.)

- [ ] **Step 4: install.sh — install mdots completions**

After the mctl completion block (~lines 384–389), add the parallel three:

```bash
  install_file 644 "${REPO_ROOT}/contrib/completions/mdots.bash" \
    "/usr/share/bash-completion/completions/mdots"
  install_file 644 "${REPO_ROOT}/contrib/completions/_mdots" \
    "/usr/share/zsh/site-functions/_mdots"
  install_file 644 "${REPO_ROOT}/contrib/completions/mdots.fish" \
    "/usr/share/fish/vendor_completions.d/mdots.fish"
```
The man page installs automatically via the existing `for manpage in "${REPO_ROOT}"/man/*.1` loop (line 304) now that `man/mdots.1` exists.

- [ ] **Step 5: PKGBUILD — mirror the same three changes**

- Add `-p mdots` to the `cargo build` `-p` lists (~line 230 and the `--package` block ~262: add `--package mdots`).
- Add `mdots` to the `for bin in … mplay \` package() loop (~line 282).
- After the package() binary loop, add the compat symlink: `ln -sf mdots "$pkgdir/usr/bin/dcli"`.
- Add the three `install -Dm644 contrib/completions/mdots.* …` lines mirroring mctl's completion install (find mctl's completion block in PKGBUILD and parallel it). `man/mdots.1` is picked up by the existing `for manpage in man/*.1` loop (~line 358).

- [ ] **Step 6: Lint the shell + verify dry paths**

Run: `bash -n install.sh && bash -n PKGBUILD 2>/dev/null; grep -n "mdots" install.sh PKGBUILD`
Expected: both parse; `mdots` appears in build list, bin loop, completions; `dcli` symlink present.

- [ ] **Step 7: Commit**

```bash
git add man/mdots.1 contrib/completions/mdots.* install.sh PKGBUILD
git commit -m "build(mdots): package via install.sh + PKGBUILD (binary, man, completions, dcli symlink)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 9: Final gate + changelog + smoke test

Prove the whole thing is green and document it.

**Files:** Modify `docs/changelog.md` (or the release-notes path margo uses)

- [ ] **Step 1: Full CI gate**

Run: `just check 2>&1 | tail -30`
Expected: fmt clean, clippy `-D warnings` clean, `panic-ratchet OK: at baseline`, design-lint pass, tests pass. Fix anything red (loop back to the owning task).

- [ ] **Step 2: Smoke test against the user's real config (read-only)**

```bash
./target/release/mdots --version
./target/release/mdots status 2>&1 | head -20
./target/release/mdots sync --dry-run 2>&1 | head -20
dcli --version 2>/dev/null || ./target/release/mdots --version   # compat name
```
Expected: identifies as `mdots`; `status` reads `~/.config/mdots` (migrated from the user's `~/.config/dcli`); `sync --dry-run` previews without applying.

- [ ] **Step 3: Changelog entry (English)**

Add an entry under the current `1.1.0` section of `docs/changelog.md`:

```markdown
- **mdots**: forked the standalone `dcli` declarative package/dotfiles
  manager into the workspace as `mdots` (config home `~/.config/mdots`,
  `dcli` compat symlink, GPL-3.0). Built/packaged via `just dots`,
  install.sh, and the PKGBUILD.
```

- [ ] **Step 4: Commit + push**

```bash
git add docs/changelog.md
git commit -m "docs(changelog): note the mdots (ex-dcli) fork under 1.1.0

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
git push origin main
```

- [ ] **Step 5: Hand back to the user**

Report: `just check` green; `mdots` installs via `just dots`; the user runs their usual rebuild flow. Remind the user of the two manual follow-ups: (a) archive the `kenanpelit/dcli` GitHub repo, (b) the `~/.config` symlink reorganisation to `~/.config/mdots` is a separate later job.

---

## Self-Review

**Spec coverage:**
- Full-surface vendoring → Task 1 ✓
- GPL-3.0 relicence / drop 0BSD+Don → Task 1 (Cargo.toml) ✓
- Config home `~/.config/mdots` + env + migration → Task 2 ✓
- dcli→mdots identity scrub (CLI/completion/man) → Task 3; bulk strings + URLs → Task 4 ✓
- clippy `-D warnings` + fmt → Task 5 ✓
- panic-ratchet baseline raise → Task 6 ✓
- justfile / install.sh / PKGBUILD + dcli compat symlink → Tasks 7–8 ✓
- man/completions → Task 8 ✓
- Verification (`just check` + smoke) → Task 9 ✓
- Out-of-scope `~/.config` symlink reorg + dcli-repo archive flagged as manual follow-ups → Task 9 Step 5 ✓
- mlua/ratatui kept (load-bearing) → Task 1 deps ✓

**Placeholder scan:** `<NEW>` in Task 6 is a runtime-measured count, deliberately substituted at execution (the command to obtain it is given) — not a plan gap. No "TBD/implement later".

**Type consistency:** `copy_dir_recursive` referenced in Task 2 is dcli's existing `commands/migrate.rs:163` fn — Task 2 Step 2 notes making it `pub(crate)`. `migrate_legacy_dcli_dir` is defined once (Task 2) and called once. Config resolver `ConfigPaths::new()` named consistently across Tasks 1–2. Completion generator string `"mdots"` consistent (Task 3) with the completion files generated in Task 8.

**Risks restated:** Task 5 (clippy debt) is the unbounded one — execute incrementally. Task 6 must use the script's count, not a raw grep. Task 2 migration is the correctness-critical step for not breaking the user's daily flow.
