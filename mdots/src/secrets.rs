//! SOPS/age secrets: decrypt declared encrypted files into place during sync.
//!
//! Unlike dotfiles (which are symlinked), secrets are **decrypted and copied**
//! with strict permissions — a symlink would point back at the encrypted blob
//! in the repo, and decrypting in place would leak plaintext into git.
//!
//! mdots does not implement crypto; it shells out to `sops`. See
//! `docs/superpowers/specs/2026-06-30-sops-secrets-design.md`.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};

use crate::config::{Config, ConfigPaths, SecretEntry};

/// Observable state of a declared secret, for `mdots secrets status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SecretState {
    /// The `sops` binary is not installed — nothing can be decrypted.
    SopsMissing,
    /// The encrypted source file does not exist.
    SourceMissing,
    /// No usable age key (sops_key_path set but missing, or no default key).
    KeyMissing,
    /// Plaintext target exists on disk.
    Decrypted,
    /// Everything is in place but the target has not been written yet.
    Pending,
}

/// Parse an octal file-mode string (e.g. "0600"); default `0o600` when absent.
pub(crate) fn parse_mode(mode: Option<&str>) -> Result<u32> {
    let raw = match mode {
        None => return Ok(0o600),
        Some(s) => s.trim(),
    };
    if raw.is_empty() {
        bail!("file mode must not be empty");
    }
    // Accept an optional leading "0o" or "0"; the remainder is octal.
    let digits = raw
        .strip_prefix("0o")
        .or_else(|| raw.strip_prefix("0O"))
        .unwrap_or(raw);
    u32::from_str_radix(digits, 8)
        .map_err(|_| anyhow::anyhow!("invalid octal file mode: {:?}", raw))
}

/// Stable identifier for a secret: its explicit `name`, else the target's
/// file name.
pub(crate) fn secret_name(entry: &SecretEntry) -> String {
    if let Some(name) = &entry.name {
        return name.clone();
    }
    Path::new(&entry.target)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| entry.target.clone())
}

/// Expand a leading `~`/`~/` against `home`; other paths are taken verbatim.
fn expand_tilde(path: &str, home: &Path) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        home.join(rest)
    } else if path == "~" {
        home.to_path_buf()
    } else {
        PathBuf::from(path)
    }
}

/// Lexically normalize a path: drop `.` and resolve `..` without touching the
/// filesystem (the target may not exist yet).
fn normalize_lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Resolve a secret's plaintext target: expand `~`, normalize, and **refuse any
/// target inside `repo_root`** so decrypted plaintext can never land in git.
/// Relative targets are rejected as ambiguous.
pub(crate) fn resolve_secret_target(
    target: &str,
    home: &Path,
    repo_root: &Path,
) -> Result<PathBuf> {
    let normalized = normalize_lexical(&expand_tilde(target, home));
    if !normalized.is_absolute() {
        bail!(
            "secret target {:?} must be an absolute path or start with ~/",
            target
        );
    }
    let repo_norm = normalize_lexical(repo_root);
    if normalized.starts_with(&repo_norm) {
        bail!(
            "secret target {} is inside the config repo {} — refusing to write plaintext into git",
            normalized.display(),
            repo_norm.display()
        );
    }
    Ok(normalized)
}

/// Pure status classification from observable facts (priority order matters).
pub(crate) fn classify_secret_status(
    sops_available: bool,
    source_exists: bool,
    key_available: bool,
    target_exists: bool,
) -> SecretState {
    if !sops_available {
        SecretState::SopsMissing
    } else if !source_exists {
        SecretState::SourceMissing
    } else if !key_available {
        SecretState::KeyMissing
    } else if target_exists {
        SecretState::Decrypted
    } else {
        SecretState::Pending
    }
}

/// Write `bytes` to `target` atomically and never world-readable: stage into a
/// sibling temp file created `0600`, set the final `mode`, fsync, then rename
/// over the target. On any error the temp file is cleaned up.
pub(crate) fn write_secret_atomically(target: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    let parent = target.parent().ok_or_else(|| {
        anyhow::anyhow!("secret target {} has no parent directory", target.display())
    })?;
    fs::create_dir_all(parent)
        .with_context(|| format!("creating parent directory {}", parent.display()))?;

    let file_name = target
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("secret target {} has no file name", target.display()))?
        .to_string_lossy();
    // Same directory → rename is atomic (same filesystem). PID-tagged to avoid
    // collisions with a concurrent mdots run.
    let tmp = parent.join(format!(".{}.mdots-tmp.{}", file_name, std::process::id()));

    let result = (|| -> Result<()> {
        let mut f = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)
            .with_context(|| format!("creating temp file {}", tmp.display()))?;
        // Belt-and-suspenders: if the temp path somehow pre-existed with wider
        // bits, .mode() (creation-only) would not have tightened it.
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))?;
        f.write_all(bytes)?;
        f.flush()?;
        f.sync_all()?;
        drop(f);
        fs::set_permissions(&tmp, fs::Permissions::from_mode(mode))?;
        fs::rename(&tmp, target)
            .with_context(|| format!("renaming {} -> {}", tmp.display(), target.display()))?;
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Result of attempting to apply one secret.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ApplyResult {
    /// Decrypted and written to the target.
    Decrypted,
    /// Dry-run: would have decrypted, but wrote nothing.
    WouldDecrypt,
    /// Skipped with a human-readable reason (warned, never fatal).
    Skipped(String),
}

/// Outcome of applying one secret, for reporting.
#[derive(Debug)]
pub(crate) struct SecretOutcome {
    pub name: String,
    pub result: ApplyResult,
}

/// Apply a single secret. Never panics; any problem becomes `Skipped(reason)`.
fn apply_one_secret(
    entry: &SecretEntry,
    home: &Path,
    repo_root: &Path,
    key_available: bool,
    sops_available: bool,
    dry_run: bool,
    decryptor: &dyn Fn(&Path) -> Result<Vec<u8>>,
) -> SecretOutcome {
    let name = secret_name(entry);
    let skip = |reason: String| SecretOutcome {
        name: name.clone(),
        result: ApplyResult::Skipped(reason),
    };

    if !sops_available {
        return skip("sops is not installed".to_string());
    }

    let target = match resolve_secret_target(&entry.target, home, repo_root) {
        Ok(t) => t,
        Err(e) => return skip(e.to_string()),
    };

    let source = repo_root.join(&entry.source);
    if !source.exists() {
        return skip(format!("source not found: {}", source.display()));
    }

    let mode = match parse_mode(entry.mode.as_deref()) {
        Ok(m) => m,
        Err(e) => return skip(e.to_string()),
    };

    if !key_available {
        return skip("age key not found (check sops_key_path)".to_string());
    }

    if dry_run {
        return SecretOutcome {
            name,
            result: ApplyResult::WouldDecrypt,
        };
    }

    let plaintext = match decryptor(&source) {
        Ok(bytes) => bytes,
        // Only the decryptor's (sops stderr) message — never plaintext.
        Err(e) => return skip(e.to_string()),
    };

    if let Err(e) = write_secret_atomically(&target, &plaintext, mode) {
        return skip(e.to_string());
    }

    SecretOutcome {
        name,
        result: ApplyResult::Decrypted,
    }
}

/// Apply all declared secrets, returning one outcome each. `key_available`
/// is derived once from `key_path` (a set-but-missing key fails every secret).
pub(crate) fn apply_secrets(
    entries: &[SecretEntry],
    home: &Path,
    repo_root: &Path,
    key_path: Option<&Path>,
    sops_available: bool,
    dry_run: bool,
    decryptor: &dyn Fn(&Path) -> Result<Vec<u8>>,
) -> Vec<SecretOutcome> {
    // No explicit key path → let sops use its own default; treat as available.
    let key_available = key_path.map(|k| k.exists()).unwrap_or(true);
    entries
        .iter()
        .map(|e| {
            apply_one_secret(
                e,
                home,
                repo_root,
                key_available,
                sops_available,
                dry_run,
                decryptor,
            )
        })
        .collect()
}

/// Targets that were written previously but are no longer declared.
pub(crate) fn compute_orphans(old: &[PathBuf], declared: &[PathBuf]) -> Vec<PathBuf> {
    old.iter()
        .filter(|t| !declared.contains(t))
        .cloned()
        .collect()
}

/// Remove orphaned plaintext targets from disk; return the ones removed.
pub(crate) fn prune_orphan_targets(orphans: &[PathBuf]) -> Vec<PathBuf> {
    orphans
        .iter()
        .filter(|t| t.exists() && fs::remove_file(t).is_ok())
        .cloned()
        .collect()
}

/// Tracks the plaintext targets mdots has materialized, so removed secrets can
/// be pruned. Stored as `secrets-state.yaml` (consistent with other mdots state).
#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct SecretsState {
    #[serde(default)]
    pub decrypted_targets: Vec<String>,
}

fn secrets_state_path(state_dir: &Path) -> PathBuf {
    state_dir.join("secrets-state.yaml")
}

pub(crate) fn load_secrets_state(state_dir: &Path) -> Result<SecretsState> {
    let path = secrets_state_path(state_dir);
    if !path.exists() {
        return Ok(SecretsState::default());
    }
    let content = fs::read_to_string(&path).context("reading secrets state file")?;
    serde_yaml::from_str(&content).context("parsing secrets state file")
}

pub(crate) fn save_secrets_state(state_dir: &Path, state: &SecretsState) -> Result<()> {
    fs::create_dir_all(state_dir).context("creating state directory")?;
    let yaml = serde_yaml::to_string(state).context("serializing secrets state")?;
    fs::write(secrets_state_path(state_dir), yaml).context("writing secrets state file")
}

/// Whether the `sops` binary is available on `PATH`.
pub(crate) fn sops_available() -> bool {
    which::which("sops").is_ok()
}

/// Expand a leading `~` in a configured key path against `$HOME`.
pub(crate) fn resolve_key_path(sops_key_path: Option<&str>, home: &Path) -> Option<PathBuf> {
    sops_key_path.map(|p| expand_tilde(p, home))
}

/// Decrypt all declared secrets into place. Per-secret failures are warned and
/// skipped — never fatal — so a broken secret cannot abort `mdots sync`.
///
/// Wired into `run_post_sync_steps`; also the core of `mdots secrets sync`.
pub fn sync_secrets(
    paths: &ConfigPaths,
    config: &Config,
    dry_run: bool,
    should_prune: bool,
    json: bool,
) -> Result<()> {
    let state_dir = &paths.state_dir;
    let prior = load_secrets_state(state_dir).unwrap_or_default();

    // Nothing declared and nothing tracked → no work, no noise.
    if config.secrets.is_empty() && prior.decrypted_targets.is_empty() {
        return Ok(());
    }

    if !json && !config.secrets.is_empty() {
        crate::ui::step("Decrypting", &format!("{} secret(s)", config.secrets.len()));
    }

    let home = PathBuf::from(std::env::var("HOME").context("HOME environment variable not set")?);
    let repo_root = &paths.config_dir;
    let key_path = resolve_key_path(config.sops_key_path.as_deref(), &home);
    let sops = sops_available();

    if !sops && !config.secrets.is_empty() && !json {
        crate::ui::warn("Decrypting", "sops is not installed — skipping all secrets");
    }

    let decryptor = |source: &Path| run_sops_decrypt(source, key_path.as_deref());
    let outcomes = apply_secrets(
        &config.secrets,
        &home,
        repo_root,
        key_path.as_deref(),
        sops,
        dry_run,
        &decryptor,
    );

    for o in &outcomes {
        match &o.result {
            ApplyResult::Decrypted => {
                if !json {
                    crate::ui::detail(&o.name);
                }
            }
            ApplyResult::WouldDecrypt => {
                if !json {
                    crate::ui::detail(&format!("would decrypt {}", o.name));
                }
            }
            ApplyResult::Skipped(reason) => {
                if !json {
                    crate::ui::warn(
                        "Skipped",
                        &format!("{} — {} (left untouched)", o.name, reason),
                    );
                }
            }
        }
    }

    // State + prune only on a real run.
    if !dry_run {
        let declared_targets: Vec<PathBuf> = config
            .secrets
            .iter()
            .filter_map(|e| resolve_secret_target(&e.target, &home, repo_root).ok())
            .collect();

        let prior_targets: Vec<PathBuf> =
            prior.decrypted_targets.iter().map(PathBuf::from).collect();
        let orphans = compute_orphans(&prior_targets, &declared_targets);

        if should_prune && !orphans.is_empty() {
            let removed = prune_orphan_targets(&orphans);
            if !json {
                for r in &removed {
                    crate::ui::detail(&format!("pruned orphan {}", r.display()));
                }
            }
        } else if !orphans.is_empty() && !json {
            crate::ui::warn(
                "Secrets",
                &format!(
                    "{} orphaned secret(s) left — run with --prune to remove",
                    orphans.len()
                ),
            );
        }

        // New state = declared targets currently materialized on disk.
        let new_state = SecretsState {
            decrypted_targets: declared_targets
                .iter()
                .filter(|t| t.exists())
                .map(|t| t.to_string_lossy().into_owned())
                .collect(),
        };
        save_secrets_state(state_dir, &new_state)?;
    }

    Ok(())
}

/// Decrypt an encrypted source with `sops --decrypt`, returning plaintext bytes.
/// `key_path`, when given, is exported as `SOPS_AGE_KEY_FILE`. Only sops's
/// stderr is ever surfaced — never its stdout (which is the plaintext).
pub(crate) fn run_sops_decrypt(source: &Path, key_path: Option<&Path>) -> Result<Vec<u8>> {
    let mut cmd = std::process::Command::new("sops");
    cmd.arg("--decrypt").arg(source);
    if let Some(key) = key_path {
        cmd.env("SOPS_AGE_KEY_FILE", key);
    }
    let output = cmd
        .output()
        .with_context(|| format!("running sops --decrypt {}", source.display()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "sops failed to decrypt {}: {}",
            source.display(),
            stderr.trim()
        );
    }
    Ok(output.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SecretEntry;
    use std::path::PathBuf;

    fn entry(target: &str, name: Option<&str>, mode: Option<&str>) -> SecretEntry {
        SecretEntry {
            source: "secrets/x.sops".to_string(),
            target: target.to_string(),
            mode: mode.map(str::to_string),
            name: name.map(str::to_string),
        }
    }

    // --- parse_mode -----------------------------------------------------------

    #[test]
    fn parse_mode_defaults_to_0600_when_absent() {
        assert_eq!(parse_mode(None).unwrap(), 0o600);
    }

    #[test]
    fn parse_mode_parses_octal_strings() {
        assert_eq!(parse_mode(Some("0600")).unwrap(), 0o600);
        assert_eq!(parse_mode(Some("0644")).unwrap(), 0o644);
        assert_eq!(parse_mode(Some("600")).unwrap(), 0o600);
        assert_eq!(parse_mode(Some("0700")).unwrap(), 0o700);
    }

    #[test]
    fn parse_mode_rejects_invalid() {
        assert!(parse_mode(Some("xyz")).is_err());
        assert!(parse_mode(Some("0999")).is_err(), "9 is not an octal digit");
        assert!(parse_mode(Some("")).is_err());
    }

    // --- secret_name ----------------------------------------------------------

    #[test]
    fn secret_name_uses_explicit_name_when_present() {
        assert_eq!(
            secret_name(&entry("~/.config/app/.env", Some("app-env"), None)),
            "app-env"
        );
    }

    #[test]
    fn secret_name_falls_back_to_target_file_name() {
        assert_eq!(
            secret_name(&entry("~/.config/app/.env", None, None)),
            ".env"
        );
    }

    // --- resolve_secret_target ------------------------------------------------

    fn home() -> PathBuf {
        PathBuf::from("/home/alice")
    }
    fn repo() -> PathBuf {
        PathBuf::from("/home/alice/.config/mdots")
    }

    #[test]
    fn resolve_target_expands_tilde() {
        let got = resolve_secret_target("~/.config/app/.env", &home(), &repo()).unwrap();
        assert_eq!(got, PathBuf::from("/home/alice/.config/app/.env"));
    }

    #[test]
    fn resolve_target_rejects_path_inside_repo() {
        let err = resolve_secret_target("/home/alice/.config/mdots/secrets/leak", &home(), &repo());
        assert!(
            err.is_err(),
            "must refuse plaintext targets inside the repo"
        );
    }

    #[test]
    fn resolve_target_rejects_repo_escape_via_dotdot() {
        let err = resolve_secret_target("~/.config/mdots/../mdots/x", &home(), &repo());
        assert!(
            err.is_err(),
            "lexical normalization must catch .. that lands back in the repo"
        );
    }

    #[test]
    fn resolve_target_allows_sibling_with_shared_prefix() {
        // /home/alice/.config/mdots-other is NOT inside /home/alice/.config/mdots
        assert!(resolve_secret_target("~/.config/mdots-other/x", &home(), &repo()).is_ok());
    }

    #[test]
    fn resolve_target_allows_outside_repo() {
        assert!(resolve_secret_target("~/.ssh/id_ed25519", &home(), &repo()).is_ok());
    }

    #[test]
    fn resolve_target_rejects_relative_path() {
        assert!(
            resolve_secret_target("foo/bar", &home(), &repo()).is_err(),
            "a relative target is ambiguous and must be rejected"
        );
    }

    // --- classify_secret_status -----------------------------------------------

    #[test]
    fn classify_sops_missing_has_top_priority() {
        assert_eq!(
            classify_secret_status(false, true, true, true),
            SecretState::SopsMissing
        );
    }

    #[test]
    fn classify_source_missing_before_key() {
        assert_eq!(
            classify_secret_status(true, false, false, false),
            SecretState::SourceMissing
        );
    }

    #[test]
    fn classify_key_missing() {
        assert_eq!(
            classify_secret_status(true, true, false, false),
            SecretState::KeyMissing
        );
    }

    #[test]
    fn classify_decrypted_when_target_present() {
        assert_eq!(
            classify_secret_status(true, true, true, true),
            SecretState::Decrypted
        );
    }

    #[test]
    fn classify_pending_when_everything_ok_but_no_target_yet() {
        assert_eq!(
            classify_secret_status(true, true, true, false),
            SecretState::Pending
        );
    }

    // --- write_secret_atomically ----------------------------------------------

    use std::os::unix::fs::PermissionsExt;

    fn mode_of(p: &Path) -> u32 {
        std::fs::metadata(p).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn write_creates_target_with_content_and_0600() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("app/.env");
        write_secret_atomically(&target, b"TOKEN=abc", 0o600).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"TOKEN=abc");
        assert_eq!(mode_of(&target), 0o600);
    }

    #[test]
    fn write_applies_requested_custom_mode() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("netrc");
        write_secret_atomically(&target, b"data", 0o640).unwrap();
        assert_eq!(mode_of(&target), 0o640);
    }

    #[test]
    fn write_overwrites_existing_target() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("secret");
        std::fs::write(&target, b"OLD").unwrap();
        write_secret_atomically(&target, b"NEW", 0o600).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"NEW");
    }

    #[test]
    fn write_leaves_no_temp_files_behind() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("creds");
        write_secret_atomically(&target, b"x", 0o600).unwrap();
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains("mdots-tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temp files leaked: {:?}", leftovers);
    }

    // --- apply_secrets (orchestration with an injected decryptor) -------------

    /// A test world: a `repo` dir (config root, where encrypted sources live)
    /// and a `home` dir (where plaintext targets land), both in tempdirs.
    struct World {
        repo: tempfile::TempDir,
        home: tempfile::TempDir,
    }

    impl World {
        fn new() -> Self {
            World {
                repo: tempfile::tempdir().unwrap(),
                home: tempfile::tempdir().unwrap(),
            }
        }
        /// Create an encrypted source file in the repo and return an entry whose
        /// target is `~/<rel_target>` (inside this world's home).
        fn secret(&self, rel_source: &str, rel_target: &str, mode: Option<&str>) -> SecretEntry {
            let src = self.repo.path().join(rel_source);
            std::fs::create_dir_all(src.parent().unwrap()).unwrap();
            std::fs::write(&src, b"ENC").unwrap();
            SecretEntry {
                source: rel_source.to_string(),
                target: format!("~/{}", rel_target),
                mode: mode.map(str::to_string),
                name: None,
            }
        }
        fn target(&self, rel_target: &str) -> PathBuf {
            self.home.path().join(rel_target)
        }
    }

    fn ok_decryptor(_src: &Path) -> Result<Vec<u8>> {
        Ok(b"PLAINTEXT".to_vec())
    }

    #[test]
    fn apply_decrypts_and_writes_each_secret() {
        let w = World::new();
        let entries = vec![
            w.secret("secrets/a.sops", ".config/a/.env", None),
            w.secret("secrets/b.sops", ".netrc", Some("0640")),
        ];
        let outcomes = apply_secrets(
            &entries,
            w.home.path(),
            w.repo.path(),
            None,
            true,
            false,
            &ok_decryptor,
        );
        assert!(outcomes.iter().all(|o| o.result == ApplyResult::Decrypted));
        assert_eq!(
            std::fs::read(w.target(".config/a/.env")).unwrap(),
            b"PLAINTEXT"
        );
        assert_eq!(mode_of(&w.target(".config/a/.env")), 0o600);
        assert_eq!(mode_of(&w.target(".netrc")), 0o640);
    }

    #[test]
    fn apply_skips_all_when_sops_missing() {
        let w = World::new();
        let entries = vec![w.secret("secrets/a.sops", ".env", None)];
        let outcomes = apply_secrets(
            &entries,
            w.home.path(),
            w.repo.path(),
            None,
            false, // sops not available
            false,
            &ok_decryptor,
        );
        assert!(matches!(outcomes[0].result, ApplyResult::Skipped(_)));
        assert!(
            !w.target(".env").exists(),
            "nothing written when sops absent"
        );
    }

    #[test]
    fn apply_skips_target_inside_repo() {
        let w = World::new();
        // target points back into the repo → must be refused
        let repo_inside = w.repo.path().join("leak.env");
        let entry = SecretEntry {
            source: "secrets/a.sops".to_string(),
            target: repo_inside.to_string_lossy().into_owned(),
            mode: None,
            name: None,
        };
        std::fs::create_dir_all(w.repo.path().join("secrets")).unwrap();
        std::fs::write(w.repo.path().join("secrets/a.sops"), b"ENC").unwrap();
        let outcomes = apply_secrets(
            &[entry],
            w.home.path(),
            w.repo.path(),
            None,
            true,
            false,
            &ok_decryptor,
        );
        assert!(matches!(outcomes[0].result, ApplyResult::Skipped(_)));
        assert!(
            !repo_inside.exists(),
            "plaintext must never be written into the repo"
        );
    }

    #[test]
    fn apply_skips_missing_source() {
        let w = World::new();
        let entry = SecretEntry {
            source: "secrets/does-not-exist.sops".to_string(),
            target: "~/.env".to_string(),
            mode: None,
            name: None,
        };
        let outcomes = apply_secrets(
            &[entry],
            w.home.path(),
            w.repo.path(),
            None,
            true,
            false,
            &ok_decryptor,
        );
        assert!(matches!(outcomes[0].result, ApplyResult::Skipped(_)));
    }

    #[test]
    fn apply_skips_when_key_path_set_but_missing() {
        let w = World::new();
        let entries = vec![w.secret("secrets/a.sops", ".env", None)];
        let missing_key = w.home.path().join("no-such-key.txt");
        let outcomes = apply_secrets(
            &entries,
            w.home.path(),
            w.repo.path(),
            Some(&missing_key),
            true,
            false,
            &ok_decryptor,
        );
        assert!(matches!(outcomes[0].result, ApplyResult::Skipped(_)));
        assert!(!w.target(".env").exists());
    }

    #[test]
    fn apply_dry_run_writes_nothing() {
        let w = World::new();
        let entries = vec![w.secret("secrets/a.sops", ".env", None)];
        let outcomes = apply_secrets(
            &entries,
            w.home.path(),
            w.repo.path(),
            None,
            true,
            true, // dry_run
            &ok_decryptor,
        );
        assert_eq!(outcomes[0].result, ApplyResult::WouldDecrypt);
        assert!(!w.target(".env").exists(), "dry-run must not write");
    }

    #[test]
    fn apply_skips_on_decryptor_error() {
        let w = World::new();
        let entries = vec![w.secret("secrets/a.sops", ".env", None)];
        let failing = |_src: &Path| -> Result<Vec<u8>> { bail!("bad key") };
        let outcomes = apply_secrets(
            &entries,
            w.home.path(),
            w.repo.path(),
            None,
            true,
            false,
            &failing,
        );
        assert!(matches!(outcomes[0].result, ApplyResult::Skipped(_)));
        assert!(!w.target(".env").exists());
    }

    // --- compute_orphans ------------------------------------------------------

    #[test]
    fn compute_orphans_finds_undeclared_targets() {
        let old = vec![PathBuf::from("/h/.env"), PathBuf::from("/h/.netrc")];
        let declared = vec![PathBuf::from("/h/.env")];
        assert_eq!(
            compute_orphans(&old, &declared),
            vec![PathBuf::from("/h/.netrc")]
        );
    }

    #[test]
    fn compute_orphans_empty_when_all_declared() {
        let old = vec![PathBuf::from("/h/.env")];
        let declared = vec![PathBuf::from("/h/.env"), PathBuf::from("/h/.netrc")];
        assert!(compute_orphans(&old, &declared).is_empty());
    }

    // --- prune + state --------------------------------------------------------

    #[test]
    fn prune_removes_orphan_files() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("orphan.env");
        std::fs::write(&f, b"x").unwrap();
        let removed = prune_orphan_targets(std::slice::from_ref(&f));
        assert_eq!(removed, vec![f.clone()]);
        assert!(!f.exists());
    }

    #[test]
    fn prune_ignores_already_absent_files() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("gone.env");
        assert!(prune_orphan_targets(&[f]).is_empty());
    }

    #[test]
    fn secrets_state_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let st = SecretsState {
            decrypted_targets: vec!["/h/.env".to_string()],
        };
        save_secrets_state(dir.path(), &st).unwrap();
        let loaded = load_secrets_state(dir.path()).unwrap();
        assert_eq!(loaded.decrypted_targets, vec!["/h/.env".to_string()]);
    }

    #[test]
    fn secrets_state_defaults_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_secrets_state(dir.path())
            .unwrap()
            .decrypted_targets
            .is_empty());
    }

    // --- Integration: real sops/age round-trip (self-skips when binaries absent) ---
    //
    // Gates on `sops_available() && which::which("age-keygen").is_ok()`.
    // When either binary is absent the test prints a notice and returns — it
    // does NOT fail, so CI stays green on any host.
    //
    // The test exercises the full encrypt → decrypt cycle through the real
    // `run_sops_decrypt` decryptor, using a freshly generated age key and a
    // temporary repo + home directory.  No env-var mutation: `apply_secrets`
    // accepts `home` and `repo_root` as explicit `&Path` arguments.

    #[test]
    fn sops_age_secrets_round_trip() {
        if !sops_available() || which::which("age-keygen").is_err() {
            eprintln!(
                "skipping sops/age round-trip: sops or age-keygen not installed on this host"
            );
            return;
        }

        let repo = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();

        // 1. Generate an age key.
        let key_dir = repo.path().join("age-keys");
        std::fs::create_dir_all(&key_dir).unwrap();
        let key_file = key_dir.join("keys.txt");

        let keygen = std::process::Command::new("age-keygen")
            .arg("-o")
            .arg(&key_file)
            .output()
            .expect("age-keygen must be runnable");
        assert!(
            keygen.status.success(),
            "age-keygen failed: {}",
            String::from_utf8_lossy(&keygen.stderr)
        );

        // 2. Extract the public recipient (age1…).
        let pubkey_out = std::process::Command::new("age-keygen")
            .arg("-y")
            .arg(&key_file)
            .output()
            .expect("age-keygen -y must be runnable");
        assert!(
            pubkey_out.status.success(),
            "age-keygen -y failed: {}",
            String::from_utf8_lossy(&pubkey_out.stderr)
        );
        let recipient = String::from_utf8_lossy(&pubkey_out.stdout)
            .trim()
            .to_string();
        assert!(
            recipient.starts_with("age1"),
            "recipient must start with age1, got: {:?}",
            recipient
        );

        // 3. Write a plaintext YAML file and encrypt it in-place with sops.
        let secrets_dir = repo.path().join("secrets");
        std::fs::create_dir_all(&secrets_dir).unwrap();
        let plain_path = secrets_dir.join("creds.yaml");
        let plaintext = b"token: s3cr3t_value\n";
        std::fs::write(&plain_path, plaintext).unwrap();

        let enc_status = std::process::Command::new("sops")
            .args([
                "--encrypt",
                "--age",
                &recipient,
                "--in-place",
                plain_path.to_str().unwrap(),
            ])
            .status()
            .expect("sops encrypt must be runnable");
        assert!(enc_status.success(), "sops --encrypt failed");

        // Rename to the source path the entry will reference.
        let enc_path = secrets_dir.join("creds.sops.yaml");
        std::fs::rename(&plain_path, &enc_path).unwrap();

        // 4. Run apply_secrets with the real run_sops_decrypt decryptor.
        let entry = SecretEntry {
            source: "secrets/creds.sops.yaml".to_string(),
            target: "~/.config/mdots-test/creds.yaml".to_string(),
            mode: Some("0600".to_string()),
            name: Some("test-creds".to_string()),
        };

        let key_file_ref = key_file.clone();
        let decryptor = |source: &Path| run_sops_decrypt(source, Some(&key_file_ref));
        let outcomes = apply_secrets(
            &[entry],
            home.path(),
            repo.path(),
            Some(&key_file),
            true,  // sops_available
            false, // dry_run = false: actually decrypt
            &decryptor,
        );

        assert_eq!(outcomes.len(), 1, "one entry must produce one outcome");
        assert_eq!(
            outcomes[0].result,
            ApplyResult::Decrypted,
            "secret must be Decrypted, got: {:?}",
            outcomes[0].result
        );

        // 5. Target must exist with the original plaintext and mode 0600.
        let target = home.path().join(".config/mdots-test/creds.yaml");
        assert!(
            target.exists(),
            "decrypted target must exist at {:?}",
            target
        );

        let content = std::fs::read(&target).unwrap();
        assert_eq!(
            content, plaintext,
            "decrypted content must equal the original plaintext"
        );

        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&target).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "target must have mode 0600, got {:04o}", mode);
    }
}
