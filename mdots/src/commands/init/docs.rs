//! Copy the user-facing documentation into the new configuration.

use super::*;

/// User-facing documentation files to include in arch-config
pub(super) const USER_DOCS: &[&str] = &[
    "CHEAT-SHEET.md",
    "MDOTS-LUA-API.md",
    "DIRECTORY-MODULES.md",
    "LUA-HOSTS.md",
    "LUA-MODULES.md",
    "SERVICES.md",
];

/// Copy user-facing documentation to arch-config/docs
pub(super) fn copy_user_docs(config_dir: &Path) -> Result<()> {
    // Find the mdots installation directory by looking for the docs folder
    // Try common locations in order of preference
    let exe_path = std::env::current_exe().ok();

    // Build list of possible documentation paths
    let mut possible_paths: Vec<std::path::PathBuf> = Vec::new();

    // Development: relative to executable (target/debug or target/release)
    // e.g., /home/user/mdots/target/release/mdots -> /home/user/mdots/docs
    if let Some(ref exe) = exe_path
        && let Some(docs_path) = exe
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.parent())
            .map(|p| p.join("docs"))
    {
        possible_paths.push(docs_path);
    }

    // Compile-time path: embedded from CARGO_MANIFEST_DIR during build.
    // Debug builds only — baking the absolute manifest dir into a *release*
    // binary leaks the build directory (makepkg's "reference to $srcdir"
    // warning) and is useless off the build machine anyway. `env!` values
    // are not touched by `--remap-path-prefix`, so gating it out is the only
    // way to keep the packaged binary clean. Dev iteration (`cargo run`) is
    // already covered by the exe-relative path above; system/local installs
    // use the `/usr/share/mdots/docs` and `~/.local/share/mdots/docs` paths
    // below.
    #[cfg(debug_assertions)]
    {
        let manifest_docs = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs");
        possible_paths.push(manifest_docs);
    }

    // Installed: /usr/share/mdots/docs (system-wide install)
    possible_paths.push(std::path::PathBuf::from("/usr/share/mdots/docs"));

    // Local install: ~/.local/share/mdots/docs
    if let Some(data_local) = dirs::data_local_dir() {
        possible_paths.push(data_local.join("mdots/docs"));
    }

    let docs_src = possible_paths
        .into_iter()
        .find(|p| p.exists() && p.is_dir());

    let Some(docs_src) = docs_src else {
        println!(
            "  {} Documentation not found, skipping docs copy",
            "→".yellow()
        );
        return Ok(());
    };

    let docs_dest = config_dir.join("docs");
    fs::create_dir_all(&docs_dest).context("Failed to create docs directory")?;

    let mut copied = 0;
    for doc_file in USER_DOCS {
        let src_file = docs_src.join(doc_file);
        let dest_file = docs_dest.join(doc_file);

        if src_file.exists() {
            fs::copy(&src_file, &dest_file)
                .with_context(|| format!("Failed to copy {}", doc_file))?;
            copied += 1;
        }
    }

    if copied > 0 {
        println!("  {} docs/ ({} files)", "✓".green(), copied);
    }

    Ok(())
}

/// Recursively copy a directory
pub(super) fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> Result<()> {
    fs::create_dir_all(dst)
        .with_context(|| format!("Failed to create directory: {}", dst.display()))?;

    for entry in
        fs::read_dir(src).with_context(|| format!("Failed to read directory: {}", src.display()))?
    {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if file_type.is_dir() {
            // Skip .git directory
            if entry.file_name() == ".git" {
                continue;
            }
            copy_dir_recursive(&src_path, &dst_path)?;
        } else if file_type.is_symlink() {
            // Handle symlinks - read the link and recreate it
            match fs::read_link(&src_path) {
                Ok(link_target) => {
                    // Try to create the symlink, but don't fail if it errors
                    // (broken symlinks are common in dotfiles)
                    let _ = std::os::unix::fs::symlink(&link_target, &dst_path);
                }
                Err(_) => {
                    // Skip broken symlinks
                    continue;
                }
            }
        } else {
            // Regular file - copy it
            match fs::copy(&src_path, &dst_path) {
                Ok(_) => {}
                Err(e) => {
                    // Log warning but continue with other files
                    eprintln!("Warning: Failed to copy {}: {}", src_path.display(), e);
                }
            }
        }
    }

    Ok(())
}
