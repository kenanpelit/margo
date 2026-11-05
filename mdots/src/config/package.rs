//! Package lists: the entry types and reading/writing them as YAML, Lua or Nix.

use super::*;

/// Package entry that can be either a simple string or an object with type specification
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PackageEntry {
    /// Simple package name (e.g., "vim")
    Simple(String),

    /// Package with explicit type
    WithType {
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        r#type: Option<PackageType>,
        /// Flatpak only: the remote to install it from (overrides the host's `flatpak_remote`)
        #[serde(default, skip_serializing_if = "Option::is_none")]
        remote: Option<String>,
    },
}

impl PackageEntry {
    pub fn name(&self) -> &str {
        match self {
            PackageEntry::Simple(name) => {
                // Handle flatpak: and nix: prefix formats
                if let Some(stripped) = name.strip_prefix("flatpak:") {
                    stripped
                } else if let Some(stripped) = name.strip_prefix("nix:") {
                    stripped
                } else {
                    name
                }
            }
            PackageEntry::WithType { name, .. } => name,
        }
    }

    pub fn package_type(&self) -> PackageType {
        match self {
            PackageEntry::Simple(name) => {
                if name.starts_with("flatpak:") {
                    PackageType::Flatpak
                } else if name.starts_with("nix:") {
                    PackageType::Nix
                } else {
                    PackageType::Native
                }
            }
            PackageEntry::WithType { r#type, .. } => r#type.clone().unwrap_or(PackageType::Native),
        }
    }

    /// The package's own Flatpak remote (`remote:`), if it names one.
    pub fn remote(&self) -> Option<&str> {
        match self {
            PackageEntry::Simple(_) => None,
            PackageEntry::WithType { remote, .. } => remote.as_deref(),
        }
    }
}

/// The type of a package entry (native system package, flatpak, or nix)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PackageType {
    /// Native system package (pacman on Arch)
    #[serde(alias = "pacman", alias = "native", rename = "native")]
    Native,
    #[serde(rename = "flatpak")]
    Flatpak,
    /// Nix package (managed via home-manager)
    #[serde(rename = "nix")]
    Nix,
}

/// The system package manager to use for native package operations
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PackageManagerType {
    /// Arch Linux pacman (with AUR helper support)
    Pacman,
}

/// Package list file (base.yaml, host files, legacy modules)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageList {
    /// Description of the package list
    #[serde(default)]
    pub description: String,

    /// List of packages
    #[serde(default)]
    pub packages: Vec<PackageEntry>,

    /// Packages to exclude (only in host files)
    #[serde(default)]
    pub exclude: Vec<String>,

    /// Conflicting modules (only in modules)
    #[serde(default)]
    pub conflicts: Vec<String>,

    /// Pre-install hook script path (only in modules)
    #[serde(default)]
    pub pre_install_hook: Option<String>,

    /// Post-install hook script path (only in modules)
    #[serde(default)]
    pub post_install_hook: Option<String>,

    /// Hook behavior: "ask" (default), "always", "skip", "once" (only in modules)
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

    /// Post-disable hook script path (only in modules)
    /// Runs after a module is disabled during sync
    #[serde(default)]
    pub post_disable_hook: Option<String>,

    /// Post-disable hook behavior: "ask" (default), "always", "skip", "once"
    #[serde(default)]
    pub post_disable_behavior: Option<String>,
}

/// Load a package list file (base, host, or legacy module)
pub fn load_package_list<P: AsRef<Path>>(path: P) -> Result<PackageList> {
    let content = std::fs::read_to_string(path.as_ref())
        .context(format!("Failed to read package list: {:?}", path.as_ref()))?;

    yaml_serde::from_str(&content).context("Failed to parse package list YAML")
}

/// Load a package list from YAML, Lua, or Nix based on file extension.
pub fn load_package_list_any<P: AsRef<Path>>(path: P) -> Result<PackageList> {
    let path = path.as_ref();
    let extension = path.extension().and_then(|e| e.to_str());

    match extension {
        Some("lua") => load_package_list_lua(path),
        Some("nix") => load_package_list_nix(path),
        Some("yaml") | Some("yml") | None => load_package_list(path),
        _ => anyhow::bail!("Unsupported package list type: {:?}", path),
    }
}

pub(super) fn load_package_list_nix(path: &Path) -> Result<PackageList> {
    let nix_module = crate::nix_eval::load_nix_module(path)?;

    Ok(PackageList {
        description: nix_module.description,
        packages: nix_module.packages,
        exclude: Vec::new(),
        conflicts: nix_module.conflicts,
        pre_install_hook: nix_module.pre_install_hook,
        post_install_hook: nix_module.post_install_hook,
        hook_behavior: nix_module.hook_behavior,
        pre_hook_behavior: nix_module.pre_hook_behavior,
        post_hook_behavior: nix_module.post_hook_behavior,
        run_hooks_as_user: nix_module.run_hooks_as_user,
        post_disable_hook: nix_module.post_disable_hook,
        post_disable_behavior: nix_module.post_disable_behavior,
    })
}

/// Write a package list to YAML, Lua, or Nix based on file extension.
pub fn write_package_list_any<P: AsRef<Path>>(path: P, list: &PackageList) -> Result<()> {
    let path = path.as_ref();
    let extension = path.extension().and_then(|e| e.to_str());

    let content = match extension {
        Some("nix") => package_list_to_nix(list),
        Some("lua") => package_list_to_lua(list),
        Some("yaml") | Some("yml") | None => {
            yaml_serde::to_string(list).context("Failed to serialize package list")?
        }
        _ => anyhow::bail!("Unsupported package list type: {:?}", path),
    };

    // Atomic write (temp sibling + rename) so an interrupt mid-write can't
    // corrupt the package manifest inside the user's tracked dotfiles repo.
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".mdots-tmp");
    let tmp = std::path::PathBuf::from(tmp);
    std::fs::write(&tmp, content).context(format!("Failed to write package list: {:?}", tmp))?;
    std::fs::rename(&tmp, path).context(format!("Failed to write package list: {:?}", path))?;

    Ok(())
}

pub(super) fn load_package_list_lua(path: &Path) -> Result<PackageList> {
    let lua_module = crate::lua::load_lua_module(path)?;

    Ok(PackageList {
        description: lua_module.description,
        packages: lua_module.packages,
        exclude: Vec::new(),
        conflicts: lua_module.conflicts,
        pre_install_hook: lua_module.pre_install_hook,
        post_install_hook: lua_module.post_install_hook,
        hook_behavior: lua_module.hook_behavior,
        pre_hook_behavior: lua_module.pre_hook_behavior,
        post_hook_behavior: lua_module.post_hook_behavior,
        run_hooks_as_user: lua_module.run_hooks_as_user,
        post_disable_hook: None,
        post_disable_behavior: None,
    })
}

pub(super) fn package_list_to_lua(list: &PackageList) -> String {
    let mut out = String::new();
    out.push_str("return {\n");

    if !list.description.is_empty() {
        out.push_str("    description = ");
        out.push_str(&lua_string(&list.description));
        out.push_str(",\n");
    }

    if list.packages.is_empty() {
        out.push_str("    packages = {},\n");
    } else {
        out.push_str("    packages = {\n");
        for entry in &list.packages {
            out.push_str("        ");
            out.push_str(&format_package_entry_lua(entry));
            out.push_str(",\n");
        }
        out.push_str("    },\n");
    }

    if !list.conflicts.is_empty() {
        out.push_str("    conflicts = {\n");
        for conflict in &list.conflicts {
            out.push_str("        ");
            out.push_str(&lua_string(conflict));
            out.push_str(",\n");
        }
        out.push_str("    },\n");
    }

    if let Some(ref hook) = list.pre_install_hook {
        out.push_str("    pre_install_hook = ");
        out.push_str(&lua_string(hook));
        out.push_str(",\n");
    }

    if let Some(ref hook) = list.post_install_hook {
        out.push_str("    post_install_hook = ");
        out.push_str(&lua_string(hook));
        out.push_str(",\n");
    }

    if !list.hook_behavior.is_empty() && list.hook_behavior != "ask" {
        out.push_str("    hook_behavior = ");
        out.push_str(&lua_string(&list.hook_behavior));
        out.push_str(",\n");
    }

    if let Some(ref behavior) = list.pre_hook_behavior {
        out.push_str("    pre_hook_behavior = ");
        out.push_str(&lua_string(behavior));
        out.push_str(",\n");
    }

    if let Some(ref behavior) = list.post_hook_behavior {
        out.push_str("    post_hook_behavior = ");
        out.push_str(&lua_string(behavior));
        out.push_str(",\n");
    }

    out.push_str("}\n");
    out
}

pub(super) fn format_package_entry_lua(entry: &PackageEntry) -> String {
    match entry {
        PackageEntry::Simple(name) => lua_string(name),
        PackageEntry::WithType { name, r#type, .. } => match r#type {
            Some(PackageType::Flatpak) => {
                format!("{{ name = {}, type = \"flatpak\" }}", lua_string(name))
            }
            _ => lua_string(name),
        },
    }
}

pub(super) fn lua_string(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 2);
    out.push('"');
    for ch in input.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

/// Serialize a PackageList to Nix format
pub fn package_list_to_nix(list: &PackageList) -> String {
    let mut out = String::new();
    out.push_str("{ system, pkgs }:\n\n{\n");

    if !list.description.is_empty() {
        out.push_str(&format!("  description = \"{}\";\n", list.description));
    }

    out.push_str("  packages = [\n");
    for entry in &list.packages {
        out.push_str(&format!("    \"{}\"\n", entry.name()));
    }
    out.push_str("  ];\n");

    if !list.exclude.is_empty() {
        out.push_str("  exclude = [\n");
        for e in &list.exclude {
            out.push_str(&format!("    \"{}\"\n", e));
        }
        out.push_str("  ];\n");
    }

    if !list.conflicts.is_empty() {
        out.push_str("  conflicts = [\n");
        for c in &list.conflicts {
            out.push_str(&format!("    \"{}\"\n", c));
        }
        out.push_str("  ];\n");
    }

    if let Some(ref hook) = list.pre_install_hook {
        out.push_str(&format!("  pre_install_hook = \"{}\";\n", hook));
    }
    if let Some(ref hook) = list.post_install_hook {
        out.push_str(&format!("  post_install_hook = \"{}\";\n", hook));
    }

    if !list.hook_behavior.is_empty() && list.hook_behavior != "ask" {
        out.push_str(&format!("  hook_behavior = \"{}\";\n", list.hook_behavior));
    }

    out.push_str("}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_package_entry_simple() {
        let entry = PackageEntry::Simple("vim".to_string());
        assert_eq!(entry.name(), "vim");
        assert_eq!(entry.package_type(), PackageType::Native);
    }

    #[test]
    fn package_entry_reads_a_flatpak_remote() {
        let entries: Vec<PackageEntry> = yaml_serde::from_str(
            "- vim\n- name: org.gnome.Builder\n  type: flatpak\n  remote: gnome-nightly\n- name: io.ente.auth\n  type: flatpak\n",
        )
        .unwrap();
        assert_eq!(entries[0].remote(), None);
        assert_eq!(entries[1].remote(), Some("gnome-nightly"));
        assert_eq!(entries[1].package_type(), PackageType::Flatpak);
        assert_eq!(entries[2].remote(), None);
        // Written back only when set.
        assert!(
            !yaml_serde::to_string(&entries[2])
                .unwrap()
                .contains("remote")
        );
    }

    #[test]
    fn test_package_entry_flatpak_prefix() {
        let entry = PackageEntry::Simple("flatpak:com.spotify.Client".to_string());
        assert_eq!(entry.name(), "com.spotify.Client");
        assert_eq!(entry.package_type(), PackageType::Flatpak);
    }

    #[test]
    fn test_package_entry_with_type() {
        let entry = PackageEntry::WithType {
            name: "vim".to_string(),
            r#type: Some(PackageType::Native),
            remote: None,
        };
        assert_eq!(entry.name(), "vim");
        assert_eq!(entry.package_type(), PackageType::Native);
    }
}
