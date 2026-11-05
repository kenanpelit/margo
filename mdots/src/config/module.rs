//! Modules: the legacy/directory/dynamic module types, their manifest,
//! dotfile and secret entries, loading and validation.

use super::*;

/// Module structure - can be legacy (single file), directory-based, Lua, or Nix
#[derive(Debug, Clone)]
pub enum ModuleStructure {
    /// Legacy format: single .yaml file with all content
    Legacy { path: PathBuf, content: PackageList },
    /// Directory format: module/ directory with module.yaml manifest
    Directory(DirectoryModule),
    /// Lua format: single .lua file with dynamic configuration
    Lua(crate::lua::LuaModule),
    /// Nix format: single .nix file with dynamic configuration
    Nix(DynamicModule),
}

/// Dynamic module structure for Nix-based modules
#[allow(dead_code)] // kept: fields parsed from Nix module config; retained for round-trip fidelity
#[derive(Debug, Clone)]
pub struct DynamicModule {
    /// Path to the .nix file
    pub path: PathBuf,
    pub description: String,
    pub packages: Vec<PackageEntry>,
    pub services: ServicesConfig,
    pub conflicts: Vec<String>,
    pub pre_install_hook: Option<String>,
    pub post_install_hook: Option<String>,
    pub hook_behavior: String,
    pub pre_hook_behavior: Option<String>,
    pub post_hook_behavior: Option<String>,
    pub post_disable_hook: Option<String>,
    pub post_disable_behavior: Option<String>,
    pub run_hooks_as_user: RunHooksAsUser,
    /// reserved for future use
    #[allow(dead_code)]
    pub metadata: Option<serde_json::Value>,
    // Informational metadata fields (kept for module manifests, not actively read)
    #[allow(dead_code)]
    pub author: Option<String>,
    #[allow(dead_code)]
    pub version: Option<String>,
    #[allow(dead_code)]
    pub category: Option<String>,
    #[allow(dead_code)]
    pub tags: Vec<String>,
    #[allow(dead_code)]
    pub license: Option<String>,
    #[allow(dead_code)]
    pub upstream_url: Option<String>,
}

impl ModuleStructure {
    /// Get the description from any format
    pub fn description(&self) -> &str {
        match self {
            ModuleStructure::Legacy { content, .. } => &content.description,
            ModuleStructure::Directory(dir) => &dir.manifest.description,
            ModuleStructure::Lua(lua) => &lua.description,
            ModuleStructure::Nix(dyn_mod) => &dyn_mod.description,
        }
    }

    /// Get conflicts from any format
    pub fn conflicts(&self) -> &[String] {
        match self {
            ModuleStructure::Legacy { content, .. } => &content.conflicts,
            ModuleStructure::Directory(dir) => &dir.manifest.conflicts,
            ModuleStructure::Lua(lua) => &lua.conflicts,
            ModuleStructure::Nix(dyn_mod) => &dyn_mod.conflicts,
        }
    }

    /// Get pre-install hook from any format
    pub fn pre_install_hook(&self) -> Option<&str> {
        match self {
            ModuleStructure::Legacy { content, .. } => content.pre_install_hook.as_deref(),
            ModuleStructure::Directory(dir) => dir.manifest.pre_install_hook.as_deref(),
            ModuleStructure::Lua(lua) => lua.pre_install_hook.as_deref(),
            ModuleStructure::Nix(dyn_mod) => dyn_mod.pre_install_hook.as_deref(),
        }
    }

    /// Get post-install hook from any format
    pub fn post_install_hook(&self) -> Option<&str> {
        match self {
            ModuleStructure::Legacy { content, .. } => content.post_install_hook.as_deref(),
            ModuleStructure::Directory(dir) => dir.manifest.post_install_hook.as_deref(),
            ModuleStructure::Lua(lua) => lua.post_install_hook.as_deref(),
            ModuleStructure::Nix(dyn_mod) => dyn_mod.post_install_hook.as_deref(),
        }
    }

    /// Get post-disable hook from any format
    pub fn post_disable_hook(&self) -> Option<&str> {
        match self {
            ModuleStructure::Legacy { content, .. } => content.post_disable_hook.as_deref(),
            ModuleStructure::Directory(dir) => dir.manifest.post_disable_hook.as_deref(),
            ModuleStructure::Lua(lua) => lua.post_disable_hook.as_deref(),
            ModuleStructure::Nix(dyn_mod) => dyn_mod.post_disable_hook.as_deref(),
        }
    }

    /// Get pre-install hook behavior (with fallback to hook_behavior)
    pub fn pre_hook_behavior(&self) -> &str {
        match self {
            ModuleStructure::Legacy { content, .. } => content
                .pre_hook_behavior
                .as_deref()
                .unwrap_or(&content.hook_behavior),
            ModuleStructure::Directory(dir) => dir
                .manifest
                .pre_hook_behavior
                .as_deref()
                .unwrap_or(&dir.manifest.hook_behavior),
            ModuleStructure::Lua(lua) => lua
                .pre_hook_behavior
                .as_deref()
                .unwrap_or(&lua.hook_behavior),
            ModuleStructure::Nix(dyn_mod) => dyn_mod
                .pre_hook_behavior
                .as_deref()
                .unwrap_or(&dyn_mod.hook_behavior),
        }
    }

    /// Get post-install hook behavior (with fallback to hook_behavior)
    pub fn post_hook_behavior(&self) -> &str {
        match self {
            ModuleStructure::Legacy { content, .. } => content
                .post_hook_behavior
                .as_deref()
                .unwrap_or(&content.hook_behavior),
            ModuleStructure::Directory(dir) => dir
                .manifest
                .post_hook_behavior
                .as_deref()
                .unwrap_or(&dir.manifest.hook_behavior),
            ModuleStructure::Lua(lua) => lua
                .post_hook_behavior
                .as_deref()
                .unwrap_or(&lua.hook_behavior),
            ModuleStructure::Nix(dyn_mod) => dyn_mod
                .post_hook_behavior
                .as_deref()
                .unwrap_or(&dyn_mod.hook_behavior),
        }
    }

    /// Get all packages from any format
    pub fn packages(&self) -> Vec<PackageEntry> {
        match self {
            ModuleStructure::Legacy { content, .. } => content.packages.clone(),
            ModuleStructure::Directory(dir) => {
                let mut all_packages = Vec::new();
                for pkg_list in &dir.package_lists {
                    all_packages.extend(pkg_list.packages.clone());
                }
                all_packages
            }
            ModuleStructure::Lua(lua) => lua.packages.clone(),
            ModuleStructure::Nix(dyn_mod) => dyn_mod.packages.clone(),
        }
    }

    /// Get the root directory for this module (for resolving relative paths)
    pub fn root_dir(&self) -> PathBuf {
        match self {
            ModuleStructure::Legacy { path, .. } => path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| PathBuf::from(".")),
            ModuleStructure::Directory(dir) => dir.root.clone(),
            ModuleStructure::Lua(lua) => lua
                .path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| PathBuf::from(".")),
            ModuleStructure::Nix(dyn_mod) => dyn_mod
                .path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| PathBuf::from(".")),
        }
    }

    /// Check if this is a directory module
    pub fn is_directory(&self) -> bool {
        matches!(self, ModuleStructure::Directory(_))
    }

    /// Check if this is a Lua module
    pub fn is_lua(&self) -> bool {
        matches!(self, ModuleStructure::Lua(_))
    }

    /// Check if this is a Nix/dynamic module
    pub fn is_nix(&self) -> bool {
        matches!(self, ModuleStructure::Nix(_))
    }

    /// Get the username to run hooks as (None = use sudo/root)
    pub fn run_hooks_as_user(&self) -> Option<String> {
        match self {
            ModuleStructure::Legacy { content, .. } => content.run_hooks_as_user.username(),
            ModuleStructure::Directory(dir) => dir.manifest.run_hooks_as_user.username(),
            ModuleStructure::Lua(lua) => lua.run_hooks_as_user.username(),
            ModuleStructure::Nix(dyn_mod) => dyn_mod.run_hooks_as_user.username(),
        }
    }
}

/// Directory-based module structure
#[derive(Debug, Clone)]
pub struct DirectoryModule {
    /// Root directory of the module
    pub root: PathBuf,

    /// Module manifest (module.yaml)
    pub manifest: ModuleManifest,

    /// Loaded package lists from all package YAML files
    pub package_lists: Vec<PackageList>,

    /// Package file paths (for reference)
    pub package_file_paths: Vec<PathBuf>,

    /// Scripts directory (if exists)
    pub scripts_dir: Option<PathBuf>,
}

/// Module manifest (module.yaml in directory-based modules)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleManifest {
    /// Description of the module
    #[serde(default)]
    pub description: String,

    /// Conflicting modules
    #[serde(default)]
    pub conflicts: Vec<String>,

    /// Pre-install hook script path (relative to module directory)
    #[serde(default)]
    pub pre_install_hook: Option<String>,

    /// Post-install hook script path (relative to module directory)
    #[serde(default)]
    pub post_install_hook: Option<String>,

    /// Hook behavior: "ask" (default), "always", "skip", "once"
    /// DEPRECATED: Use pre_hook_behavior and post_hook_behavior for independent control
    #[serde(default = "default_hook_behavior", alias = "hooks_behavior")]
    pub hook_behavior: String,

    /// Pre-install hook behavior (overrides hook_behavior for pre-install hook)
    #[serde(default)]
    pub pre_hook_behavior: Option<String>,

    /// Post-install hook behavior (overrides hook_behavior for post-install hook)
    #[serde(default)]
    pub post_hook_behavior: Option<String>,

    /// Run hooks as specified user instead of with sudo (default: false)
    /// Can be: false (default, use sudo), true (current user), or "username"
    #[serde(default)]
    pub run_hooks_as_user: RunHooksAsUser,

    /// Post-disable hook script path (relative to module directory)
    /// Runs after a module is disabled during sync
    #[serde(default)]
    pub post_disable_hook: Option<String>,

    /// Post-disable hook behavior: "ask" (default), "always", "skip", "once"
    #[serde(default)]
    pub post_disable_behavior: Option<String>,

    /// Explicit list of package files to load (empty = auto-discover)
    #[serde(default)]
    pub package_files: Vec<String>,

    /// Auto-sync dotfiles/ directories to ~/.config/
    #[serde(default)]
    pub dotfiles_sync: Option<bool>,

    /// Explicit dotfiles list with custom source/target
    #[serde(default)]
    pub dotfiles: Vec<DotfileEntry>,

    // === Informational metadata fields (kept on the manifest) ===
    /// Module author (e.g. a username)
    #[serde(default)]
    pub author: Option<String>,

    /// Module version (semver format: X.Y.Z)
    #[serde(default)]
    pub version: Option<String>,

    /// Category for organization (defaults to "other" if not specified)
    #[serde(default)]
    pub category: Option<String>,

    /// Tags for search/filtering
    #[serde(default)]
    pub tags: Vec<String>,

    /// License identifier (e.g., "MIT", "GPL-3.0")
    #[serde(default)]
    pub license: Option<String>,

    /// URL to upstream project/documentation
    #[serde(default)]
    pub upstream_url: Option<String>,
}

pub(super) fn default_hook_behavior() -> String {
    "ask".to_string()
}

/// Dotfile entry with explicit source and target paths
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DotfileEntry {
    /// Source path relative to module root
    pub source: String,

    /// Target path (supports ~ expansion)
    pub target: String,
}

/// A SOPS-encrypted secret: an encrypted source in the config repo that is
/// decrypted into a plaintext target (copied, never symlinked) during sync.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretEntry {
    /// Encrypted source path, relative to the config repo root.
    pub source: String,

    /// Plaintext target path (supports `~` expansion). Must not be inside the
    /// config repo — that guard prevents leaking plaintext into git.
    pub target: String,

    /// Octal file mode for the decrypted target, e.g. "0600" (the default).
    #[serde(default)]
    pub mode: Option<String>,

    /// Stable identifier for `mdots secrets edit/status/list`.
    /// Defaults to the target's file name when absent.
    #[serde(default)]
    pub name: Option<String>,
}

/// Load a module (detects legacy vs directory format automatically)
pub fn load_module<P: AsRef<Path>>(path: P) -> Result<ModuleStructure> {
    let path = path.as_ref();

    if path.is_file() {
        let extension = path.extension().and_then(|e| e.to_str());

        match extension {
            Some("nix") => {
                let nix_module = crate::nix_eval::load_nix_module_as_module_structure(path)?;
                Ok(nix_module)
            }
            Some("lua") => {
                let lua_module = crate::lua::load_lua_module(path)?;
                Ok(ModuleStructure::Lua(lua_module))
            }
            Some("yaml") | Some("yml") | None => {
                let content = load_package_list(path)?;
                Ok(ModuleStructure::Legacy {
                    path: path.to_path_buf(),
                    content,
                })
            }
            _ => {
                anyhow::bail!("Unsupported module file type: {:?}", path)
            }
        }
    } else if path.is_dir() {
        let nix_manifest_path = path.join("module.nix");
        let lua_manifest_path = path.join("module.lua");
        let yaml_manifest_path = path.join("module.yaml");
        let mut inline_packages: Vec<PackageEntry> = Vec::new();

        let manifest: ModuleManifest = if nix_manifest_path.exists() {
            log::debug!("Loading Nix manifest: {:?}", nix_manifest_path);
            let (manifest, pkgs) = crate::nix_eval::load_nix_directory_module(&nix_manifest_path)?;
            inline_packages = pkgs;
            manifest
        } else if lua_manifest_path.exists() {
            log::debug!("Loading Lua manifest: {:?}", lua_manifest_path);
            let lua_dir = crate::lua::load_lua_directory_module(&lua_manifest_path)?;
            inline_packages = lua_dir.packages;
            lua_dir.manifest
        } else if yaml_manifest_path.exists() {
            let manifest_content = std::fs::read_to_string(&yaml_manifest_path).context(
                format!("Failed to read module.yaml: {:?}", yaml_manifest_path),
            )?;
            yaml_serde::from_str(&manifest_content).context("Failed to parse module.yaml")?
        } else {
            let discovered_files = discover_package_files(path)?;
            if discovered_files.is_empty() {
                anyhow::bail!(
                    "Directory module must contain 'module.nix', 'module.lua', 'module.yaml', or package files: {:?}",
                    path
                );
            }

            let module_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown");

            log::debug!(
                "Creating default manifest for package-only directory module: {:?}",
                path
            );
            ModuleManifest {
                description: format!("Module: {}", module_name),
                package_files: Vec::new(),
                dotfiles: Vec::new(),
                dotfiles_sync: None,
                conflicts: Vec::new(),
                pre_install_hook: None,
                post_install_hook: None,
                hook_behavior: "ask".to_string(),
                pre_hook_behavior: None,
                post_hook_behavior: None,
                run_hooks_as_user: RunHooksAsUser::Bool(false),
                post_disable_hook: None,
                post_disable_behavior: None,
                author: None,
                version: None,
                category: None,
                tags: Vec::new(),
                license: None,
                upstream_url: None,
            }
        };

        // Discover or load package files
        let mut package_file_paths = if manifest.package_files.is_empty() {
            discover_package_files(path)?
        } else {
            manifest
                .package_files
                .iter()
                .map(|f| path.join(f))
                .collect()
        };

        // Load all package lists
        let mut package_lists = Vec::new();
        for pkg_path in &package_file_paths {
            if !pkg_path.exists() {
                anyhow::bail!("Package file not found: {:?}", pkg_path);
            }
            let pkg_list = load_package_list_any(pkg_path)?;
            package_lists.push(pkg_list);
        }

        if !inline_packages.is_empty() {
            package_file_paths.push(lua_manifest_path.clone());
            package_lists.push(PackageList {
                description: "Packages defined in module.lua".to_string(),
                packages: inline_packages,
                exclude: Vec::new(),
                conflicts: Vec::new(),
                pre_install_hook: None,
                post_install_hook: None,
                hook_behavior: "ask".to_string(),
                pre_hook_behavior: None,
                post_hook_behavior: None,
                run_hooks_as_user: RunHooksAsUser::Bool(false),
                post_disable_hook: None,
                post_disable_behavior: None,
            });
        }

        // Check for scripts and dotfiles directories
        let scripts_dir = path.join("scripts");
        let scripts_dir = if scripts_dir.exists() && scripts_dir.is_dir() {
            Some(scripts_dir)
        } else {
            None
        };

        Ok(ModuleStructure::Directory(DirectoryModule {
            root: path.to_path_buf(),
            manifest,
            package_lists,
            package_file_paths,
            scripts_dir,
        }))
    } else {
        anyhow::bail!("Module path is neither a file nor directory: {:?}", path)
    }
}

/// Discover package files in a module directory (excludes module.yaml/module.nix/module.lua)
pub(super) fn discover_package_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let excluded = [
        std::ffi::OsStr::new("module.yaml"),
        std::ffi::OsStr::new("module.nix"),
        std::ffi::OsStr::new("module.lua"),
    ];

    for entry in std::fs::read_dir(dir).context(format!("Failed to read directory: {:?}", dir))? {
        let entry = entry?;
        let path = entry.path();

        if path.is_file()
            && !excluded.contains(&path.file_name().unwrap_or_default())
            && let Some(ext) = path.extension().and_then(|e| e.to_str())
            && (ext == "yaml" || ext == "nix" || ext == "lua")
        {
            files.push(path);
        }
    }

    files.sort();
    Ok(files)
}

/// Validation result for modules
#[derive(Debug, Clone)]
pub struct ModuleValidationResult {
    #[allow(dead_code)] // kept: populated for diagnostics; not yet surfaced to the user
    pub module_name: String,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl ModuleValidationResult {
    pub fn new(module_name: String) -> Self {
        Self {
            module_name,
            errors: Vec::new(),
            warnings: Vec::new(),
        }
    }

    pub fn is_clean(&self) -> bool {
        self.errors.is_empty() && self.warnings.is_empty()
    }
}

/// Validate a module structure
pub fn validate_module(module: &ModuleStructure, module_name: &str) -> ModuleValidationResult {
    let mut result = ModuleValidationResult::new(module_name.to_string());

    match module {
        ModuleStructure::Legacy { content, .. } => {
            // Validate legacy module
            if content.description.is_empty() {
                result
                    .warnings
                    .push("Module has no description".to_string());
            }

            // Empty pre_install_hook is fine - just ignore it
            // Only validate if hook is actually specified with a non-empty value
            if let Some(hook) = &content.pre_install_hook
                && !hook.is_empty()
            {
                // Hook is specified - for legacy modules, we can't validate the path
                // since it's resolved relative to config_dir at runtime
                // Just check that it's not obviously invalid
            }

            // Empty post_install_hook is fine - just ignore it
            // Only validate if hook is actually specified with a non-empty value
            if let Some(hook) = &content.post_install_hook
                && !hook.is_empty()
            {
                // Hook is specified - for legacy modules, we can't validate the path
                // since it's resolved relative to config_dir at runtime
                // Just check that it's not obviously invalid
            }

            // Validate no duplicate packages
            let mut seen = std::collections::HashSet::new();
            for pkg in &content.packages {
                let name = pkg.name();
                if !seen.insert(name) {
                    result.errors.push(format!("Duplicate package: {}", name));
                }
            }
        }
        ModuleStructure::Directory(dir) => {
            // Validate directory module
            if dir.manifest.description.is_empty() {
                result
                    .warnings
                    .push("Module has no description".to_string());
            }

            // Validate package files exist and are non-empty
            if dir.package_lists.is_empty() {
                result
                    .warnings
                    .push("No package files found (module has no packages)".to_string());
            }

            // Check for empty package files
            for (idx, pkg_list) in dir.package_lists.iter().enumerate() {
                if pkg_list.packages.is_empty() {
                    let file_name = dir
                        .package_file_paths
                        .get(idx)
                        .and_then(|p| p.file_name())
                        .and_then(|n| n.to_str())
                        .unwrap_or("unknown");
                    result
                        .warnings
                        .push(format!("Package file '{}' is empty", file_name));
                }
            }

            // Validate pre-install hook exists if specified (non-empty)
            if let Some(hook) = &dir.manifest.pre_install_hook
                && !hook.is_empty()
            {
                // Hook is specified - validate it exists
                let hook_path = if Path::new(hook).is_absolute() {
                    PathBuf::from(hook)
                } else {
                    dir.root.join(hook)
                };

                if !hook_path.exists() {
                    result.errors.push(format!(
                        "pre_install_hook script not found: {} (resolved to {:?})",
                        hook, hook_path
                    ));
                } else if !hook_path.is_file() {
                    result
                        .errors
                        .push(format!("pre_install_hook is not a file: {}", hook));
                }

                // If hook is in scripts/ subdirectory, ensure it's in our scripts dir
                if hook.starts_with("scripts/") && dir.scripts_dir.is_none() {
                    result.errors.push(
                            "pre_install_hook references 'scripts/' but scripts directory doesn't exist".to_string()
                        );
                }
            }

            // Validate post-install hook exists if specified (non-empty)
            if let Some(hook) = &dir.manifest.post_install_hook
                && !hook.is_empty()
            {
                // Hook is specified - validate it exists
                let hook_path = if Path::new(hook).is_absolute() {
                    PathBuf::from(hook)
                } else {
                    dir.root.join(hook)
                };

                if !hook_path.exists() {
                    result.errors.push(format!(
                        "post_install_hook script not found: {} (resolved to {:?})",
                        hook, hook_path
                    ));
                } else if !hook_path.is_file() {
                    result
                        .errors
                        .push(format!("post_install_hook is not a file: {}", hook));
                }

                // If hook is in scripts/ subdirectory, ensure it's in our scripts dir
                if hook.starts_with("scripts/") && dir.scripts_dir.is_none() {
                    result.errors.push(
                            "post_install_hook references 'scripts/' but scripts directory doesn't exist".to_string()
                        );
                }
            }

            // Warn if scripts directory exists but no hook
            if dir.scripts_dir.is_some() && dir.manifest.post_install_hook.is_none() {
                result.warnings.push(
                    "scripts/ directory exists but no post_install_hook is configured".to_string(),
                );
            }

            // Note: dotfiles/ directory is allowed - users can handle dotfiles in their post-install hooks

            // Note: We don't check for duplicate packages across files
            // because the same package may legitimately appear in multiple modules
            // (user may enable one module but not another)

            // Validate package files in manifest actually exist
            for specified_file in &dir.manifest.package_files {
                let file_path = dir.root.join(specified_file);
                if !file_path.exists() {
                    result.errors.push(format!(
                        "Package file specified in manifest not found: {}",
                        specified_file
                    ));
                }
            }
        }
        ModuleStructure::Lua(lua) => {
            // Validate Lua module
            if lua.description.is_empty() {
                result
                    .warnings
                    .push("Module has no description".to_string());
            }

            // Validate no duplicate packages
            let mut seen = std::collections::HashSet::new();
            for pkg in &lua.packages {
                let name = pkg.name();
                if !seen.insert(name) {
                    result.errors.push(format!("Duplicate package: {}", name));
                }
            }

            // Validate hook paths if specified
            if let Some(hook) = &lua.pre_install_hook
                && !hook.is_empty()
            {
                let hook_path = lua
                    .path
                    .parent()
                    .map(|p| p.join(hook))
                    .unwrap_or_else(|| std::path::PathBuf::from(hook));
                if !hook_path.exists() {
                    result
                        .errors
                        .push(format!("pre_install_hook script not found: {}", hook));
                }
            }

            if let Some(hook) = &lua.post_install_hook
                && !hook.is_empty()
            {
                let hook_path = lua
                    .path
                    .parent()
                    .map(|p| p.join(hook))
                    .unwrap_or_else(|| std::path::PathBuf::from(hook));
                if !hook_path.exists() {
                    result
                        .errors
                        .push(format!("post_install_hook script not found: {}", hook));
                }
            }
        }
        ModuleStructure::Nix(dyn_mod) => {
            if dyn_mod.description.is_empty() {
                result
                    .warnings
                    .push("Module has no description".to_string());
            }

            let mut seen = std::collections::HashSet::new();
            for pkg in &dyn_mod.packages {
                let name = pkg.name();
                if !seen.insert(name) {
                    result.errors.push(format!("Duplicate package: {}", name));
                }
            }

            if let Some(hook) = &dyn_mod.pre_install_hook
                && !hook.is_empty()
            {
                let hook_path = dyn_mod
                    .path
                    .parent()
                    .map(|p| p.join(hook))
                    .unwrap_or_else(|| std::path::PathBuf::from(hook));
                if !hook_path.exists() {
                    result
                        .errors
                        .push(format!("pre_install_hook script not found: {}", hook));
                }
            }

            if let Some(hook) = &dyn_mod.post_install_hook
                && !hook.is_empty()
            {
                let hook_path = dyn_mod
                    .path
                    .parent()
                    .map(|p| p.join(hook))
                    .unwrap_or_else(|| std::path::PathBuf::from(hook));
                if !hook_path.exists() {
                    result
                        .errors
                        .push(format!("post_install_hook script not found: {}", hook));
                }
            }
        }
    }

    result
}
