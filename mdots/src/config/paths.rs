//! `ConfigPaths` and resolving which config file (YAML, Lua or Nix) is active.

use super::*;

/// Configuration paths
#[derive(Debug, Clone)]
pub struct ConfigPaths {
    pub config_dir: PathBuf,
    pub config_file: PathBuf,
    pub packages_dir: PathBuf,
    pub state_dir: PathBuf,
    pub state_file: PathBuf,
    pub hooks_state_file: PathBuf,
    pub services_state_file: PathBuf,
    pub defaults_state_file: PathBuf,
    pub theming_state_file: PathBuf,
    pub config_backups_dir: PathBuf,
}

impl ConfigPaths {
    /// Create configuration paths from environment or defaults.
    /// Checks for config directories in this order:
    /// 1. MDOTS_CONFIG_DIR env var (if set)
    /// 2. ARCH_CONFIG_DIR env var (legacy, if set)
    /// 3. ~/.config/mdots/ (new default)
    /// 4. ~/.config/arch-config/ (legacy fallback)
    pub fn new() -> Result<Self> {
        let config_dir = if let Ok(custom) = std::env::var("MDOTS_CONFIG_DIR") {
            PathBuf::from(custom)
        } else if let Ok(custom) = std::env::var("ARCH_CONFIG_DIR") {
            PathBuf::from(custom)
        } else {
            let home = std::env::var("HOME").context("HOME environment variable not set")?;
            let new_path = PathBuf::from(&home).join(".config/mdots");
            let legacy_path = PathBuf::from(&home).join(".config/arch-config");

            if new_path.exists() {
                new_path
            } else if legacy_path.exists() {
                legacy_path
            } else {
                // Default to new path for fresh installs
                new_path
            }
        };

        let config_file = config_dir.join("config.yaml");
        let packages_dir = config_dir.join("packages");
        let state_dir = config_dir.join("state");
        let state_file = state_dir.join("installed.yaml");
        let hooks_state_file = state_dir.join("hooks-executed.yaml");
        let services_state_file = state_dir.join("services-state.yaml");
        let defaults_state_file = state_dir.join("defaults-state.yaml");
        let theming_state_file = state_dir.join("theming-state.yaml");
        let config_backups_dir = state_dir.join("config-backups");

        Ok(Self {
            config_dir,
            config_file,
            packages_dir,
            state_dir,
            state_file,
            hooks_state_file,
            services_state_file,
            defaults_state_file,
            theming_state_file,
            config_backups_dir,
        })
    }

    /// Get the hosts directory (supports both old and new structure)
    pub fn hosts_dir(&self) -> PathBuf {
        let new_path = self.config_dir.join("hosts");
        if new_path.exists() {
            new_path
        } else {
            // Fallback to old location
            self.packages_dir.join("hosts")
        }
    }

    /// Get the modules directory (supports both old and new structure)
    pub fn modules_dir(&self) -> PathBuf {
        let new_path = self.config_dir.join("modules");
        if new_path.exists() {
            new_path
        } else {
            // Fallback to old location
            self.packages_dir.join("modules")
        }
    }

    /// Get the services profiles directory
    pub fn services_dir(&self) -> PathBuf {
        self.config_dir.join("services")
    }

    /// Get the sources directory (build-from-source configs)
    pub fn sources_dir(&self) -> PathBuf {
        self.config_dir.join("sources")
    }

    /// Get the home-manager directory
    pub fn home_manager_dir(&self) -> PathBuf {
        self.config_dir.join("home-manager")
    }

    /// Get the base packages file (supports both old and new structure)
    pub fn base_packages_file(&self) -> PathBuf {
        let new_nix = self.config_dir.join("modules").join("base.nix");
        let new_lua = self.config_dir.join("modules").join("base.lua");
        let new_yaml = self.config_dir.join("modules").join("base.yaml");

        if new_nix.exists() {
            new_nix
        } else if new_lua.exists() {
            new_lua
        } else if new_yaml.exists() {
            new_yaml
        } else {
            self.packages_dir.join("base.yaml")
        }
    }

    /// Get a host-specific configuration file (supports both old and new structure)
    /// Now supports .yaml, .lua, and .nix files, preferring lua > nix > yaml.
    /// Also supports directory-based structure: hosts/{hostname}/host.lua or host.nix
    pub fn host_packages_file(&self, hostname: &str) -> PathBuf {
        let host_dir = self.config_dir.join("hosts").join(hostname);

        // Check for directory structure
        if host_dir.is_dir() {
            let nix_config = self.config_dir.join("config.nix");
            let lua_config = self.config_dir.join("config.lua");

            // In Nix mode, prefer host.nix; in Lua mode, prefer host.lua
            if nix_config.exists() && !lua_config.exists() {
                let host_nix = host_dir.join("host.nix");
                if host_nix.exists() {
                    return host_nix;
                }
            }
            return host_dir.join("host.lua");
        }

        let lua_filename = format!("{}.lua", hostname);
        let nix_filename = format!("{}.nix", hostname);
        let yaml_filename = format!("{}.yaml", hostname);

        let new_lua_path = self.config_dir.join("hosts").join(&lua_filename);
        let new_nix_path = self.config_dir.join("hosts").join(&nix_filename);
        let new_yaml_path = self.config_dir.join("hosts").join(&yaml_filename);

        if new_lua_path.exists() {
            return new_lua_path;
        }
        if new_nix_path.exists() {
            return new_nix_path;
        }
        if new_yaml_path.exists() {
            return new_yaml_path;
        }

        let old_lua_path = self.packages_dir.join("hosts").join(&lua_filename);
        let old_nix_path = self.packages_dir.join("hosts").join(&nix_filename);
        let old_yaml_path = self.packages_dir.join("hosts").join(&yaml_filename);

        if old_lua_path.exists() {
            return old_lua_path;
        }
        if old_nix_path.exists() {
            return old_nix_path;
        }
        if old_yaml_path.exists() {
            return old_yaml_path;
        }

        new_yaml_path
    }
}

impl Default for ConfigPaths {
    fn default() -> Self {
        Self::new().expect("Failed to create default config paths")
    }
}

/// Resolve the effective config file path (config.lua, config.nix, host file, or config.yaml).
pub fn resolve_config_path(paths: &ConfigPaths) -> Result<PathBuf> {
    // Check for config.lua first
    let lua_config_file = paths.config_dir.join("config.lua");
    if lua_config_file.exists() {
        // Check that config.nix doesn't also exist
        let nix_config_file = paths.config_dir.join("config.nix");
        if nix_config_file.exists() {
            anyhow::bail!(
                "Cannot have both config.lua and config.nix. Please remove one.\n\
                 Run 'mdots init --lua' for Lua or 'mdots init --nix' for Nix."
            );
        }

        if let Some(hostname) = crate::lua::detect_pointer_lua_config(&lua_config_file)? {
            let host_file = paths.host_packages_file(&hostname);
            if !host_file.exists() {
                anyhow::bail!(
                    "Host file not found: {:?}\nconfig.lua is a pointer but host file doesn't exist",
                    host_file
                );
            }
            return Ok(host_file);
        }
        return Ok(lua_config_file);
    }

    // Check for config.nix next
    let nix_config_file = paths.config_dir.join("config.nix");
    if nix_config_file.exists() {
        if crate::nix_eval::is_nix_installed() {
            if let Some(hostname) =
                crate::nix_eval::detect_pointer_nix_config_file(&nix_config_file)?
            {
                let host_file = paths.host_packages_file(&hostname);
                if !host_file.exists() {
                    anyhow::bail!(
                        "Host file not found: {:?}\nconfig.nix is a pointer but host file doesn't exist",
                        host_file
                    );
                }
                return Ok(host_file);
            }
            return Ok(nix_config_file);
        } else {
            eprintln!("config.nix found but nix is not installed. Falling back to config.yaml.");
        }
    }

    // Fall back to config.yaml
    let content = std::fs::read_to_string(&paths.config_file).context(format!(
        "Failed to read config file: {:?}",
        paths.config_file
    ))?;

    if is_pointer_config_raw(&content) {
        let config: Config =
            yaml_serde::from_str(&content).context("Failed to parse config.yaml")?;

        let host_file = paths.host_packages_file(&config.host);
        if !host_file.exists() {
            anyhow::bail!(
                "Host file not found: {:?}\nConfig.yaml is a pointer but host file doesn't exist",
                host_file
            );
        }
        return Ok(host_file);
    }

    Ok(paths.config_file.clone())
}

/// Check if the active config file is Lua-based.
pub fn is_lua_config(paths: &ConfigPaths) -> Result<bool> {
    let config_path = resolve_config_path(paths)?;
    Ok(config_path.extension().and_then(|e| e.to_str()) == Some("lua"))
}

/// Check if the active config file is Nix-based.
pub fn is_nix_config(paths: &ConfigPaths) -> Result<bool> {
    let config_path = resolve_config_path(paths)?;
    Ok(config_path.extension().and_then(|e| e.to_str()) == Some("nix"))
}

/// Get preferred and fallback declared-packages paths based on config format.
/// Prefers host directory structure (hosts/{hostname}/) when the config uses
/// directory-based hosts. Falls back to modules/ for file-based hosts or legacy configs.
pub fn declared_packages_paths(paths: &ConfigPaths) -> Result<(PathBuf, PathBuf)> {
    let prefer_lua = is_lua_config(paths).unwrap_or(false);
    let prefer_nix = is_nix_config(paths).unwrap_or(false);

    let (preferred_ext, fallback_ext) = if prefer_nix {
        ("nix", "yaml")
    } else if prefer_lua {
        ("lua", "yaml")
    } else {
        ("yaml", "lua")
    };

    let config_path = resolve_config_path(paths)?;
    let host_filename = config_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    let dir_based_host = config_path
        .parent()
        .map(|p| {
            p.is_dir()
                && (host_filename == "host.lua" || host_filename == "host.nix")
                && config_path.starts_with(paths.hosts_dir())
        })
        .unwrap_or(false);

    let base_dir = if dir_based_host {
        config_path.parent().unwrap().to_path_buf()
    } else {
        paths.modules_dir()
    };

    Ok((
        base_dir.join(format!("declared-packages.{}", preferred_ext)),
        base_dir.join(format!("declared-packages.{}", fallback_ext)),
    ))
}
