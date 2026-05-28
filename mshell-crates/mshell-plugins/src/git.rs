//! Git operations behind the manager: fetch a source's registry and
//! install a plugin's folder. Both use shallow sparse clones so only the
//! needed bytes are pulled.
//!
//! Security: URLs and paths are passed as `git` arguments (never through a
//! shell), with `--` guarding against option injection, so a malicious
//! source URL can't smuggle extra flags or shell metacharacters.

use crate::PluginError;
use crate::manifest::Registry;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Fetch and parse a source's root `registry.toml` via a sparse clone.
pub fn fetch_registry(url: &str) -> Result<Registry, PluginError> {
    let tmp = scratch_dir("reg");
    let result = (|| {
        clone_sparse(url, &tmp)?;
        sparse_set(&tmp, "registry.toml")?;
        let path = tmp.join("registry.toml");
        let text = std::fs::read_to_string(&path)
            .map_err(|e| PluginError::Git(format!("source has no registry.toml ({e})")))?;
        toml::from_str::<Registry>(&text).map_err(|e| PluginError::Parse(e.to_string()))
    })();
    let _ = std::fs::remove_dir_all(&tmp);
    result
}

/// Sparse-clone `entry_dir` from `url` and copy it to `dest` (replacing any
/// existing contents). Caller validates the resulting manifest.
pub fn install_plugin(url: &str, entry_dir: &str, dest: &Path) -> Result<(), PluginError> {
    if entry_dir.trim().is_empty() || entry_dir.contains("..") {
        return Err(PluginError::Git(format!("invalid plugin dir `{entry_dir}`")));
    }
    let tmp = scratch_dir("inst");
    let result = (|| {
        clone_sparse(url, &tmp)?;
        sparse_set(&tmp, entry_dir)?;
        let src = tmp.join(entry_dir);
        if !src.is_dir() {
            return Err(PluginError::Git(format!(
                "plugin folder `{entry_dir}` not found in source"
            )));
        }
        if dest.exists() {
            std::fs::remove_dir_all(dest)?;
        }
        copy_dir_all(&src, dest)?;
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&tmp);
    result
}

// ── internals ───────────────────────────────────────────────────────────────

fn clone_sparse(url: &str, into: &Path) -> Result<(), PluginError> {
    run_git(&[
        "clone",
        "--filter=blob:none",
        "--sparse",
        "--depth=1",
        "--quiet",
        "--",
        url,
        &into.to_string_lossy(),
    ])
}

fn sparse_set(repo: &Path, pattern: &str) -> Result<(), PluginError> {
    run_git(&[
        "-C",
        &repo.to_string_lossy(),
        "sparse-checkout",
        "set",
        "--no-cone",
        pattern,
    ])
}

fn run_git(args: &[&str]) -> Result<(), PluginError> {
    let out = Command::new("git")
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0") // never block on credential prompts
        .output()
        .map_err(|e| PluginError::Git(format!("failed to run git: {e}")))?;
    if out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(PluginError::Git(err.trim().to_string()))
    }
}

fn scratch_dir(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("mplugins-{tag}-{nanos}"))
}

/// Recursive directory copy, skipping any `.git` metadata.
fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            if entry.file_name() == ".git" {
                continue;
            }
            copy_dir_all(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}
