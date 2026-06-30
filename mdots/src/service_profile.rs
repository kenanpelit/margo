//! Service profile management
//!
//! Manages service profiles in the services/ directory.
//! Service profiles are Lua files that define systemd services to enable/disable.

use anyhow::Result;
use std::path::PathBuf;
use walkdir::WalkDir;

use crate::config::{ConfigPaths, ServiceProfile};
use crate::lua::service_profile::load_service_profile;

/// Information about a service profile for listing
#[derive(Debug, Clone)]
pub struct ServiceProfileInfo {
    pub name: String,
    pub description: String,
    pub enabled_services: Vec<String>,
    pub disabled_services: Vec<String>,
    pub conflicts: Vec<String>,
    pub is_enabled: bool,
}

/// Manager for service profiles
pub struct ServiceProfileManager {
    paths: ConfigPaths,
}

impl ServiceProfileManager {
    /// Create a new ServiceProfileManager
    pub fn new(paths: ConfigPaths) -> Self {
        Self { paths }
    }

    /// List all available service profiles
    pub fn list_profiles(&self, enabled_profiles: &[String]) -> Result<Vec<ServiceProfileInfo>> {
        let services_dir = self.paths.services_dir();

        if !services_dir.exists() {
            return Ok(Vec::new());
        }

        let mut profiles = Vec::new();

        // Walk the services directory looking for .lua files
        for entry in WalkDir::new(&services_dir)
            .max_depth(2) // Allow one level of subdirectories
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();

            // Only process .lua files
            if path.extension().and_then(|e| e.to_str()) != Some("lua") {
                continue;
            }

            // Skip if it's a directory
            if path.is_dir() {
                continue;
            }

            // Get relative name for the profile
            let relative_path = path.strip_prefix(&services_dir).unwrap_or(path);
            let name = relative_path
                .with_extension("")
                .to_string_lossy()
                .to_string();

            // Try to load the profile
            match load_service_profile(path) {
                Ok(profile) => {
                    let is_enabled = enabled_profiles.contains(&name);
                    profiles.push(ServiceProfileInfo {
                        name,
                        description: profile.description,
                        enabled_services: profile.services.enabled,
                        disabled_services: profile.services.disabled,
                        conflicts: profile.conflicts,
                        is_enabled,
                    });
                }
                Err(e) => {
                    log::warn!("Failed to load service profile {:?}: {}", path, e);
                }
            }
        }

        // Sort by name
        profiles.sort_by(|a, b| a.name.cmp(&b.name));

        Ok(profiles)
    }

    /// Load a specific service profile by name
    pub fn load_profile(&self, name: &str) -> Result<ServiceProfile> {
        let path = self.resolve_profile_path(name)?;
        load_service_profile(&path)
    }

    /// Resolve a profile name to its file path
    pub fn resolve_profile_path(&self, name: &str) -> Result<PathBuf> {
        let services_dir = self.paths.services_dir();

        // Check for direct match
        let lua_path = services_dir.join(format!("{}.lua", name));
        if lua_path.exists() {
            return Ok(lua_path);
        }

        // Check in subdirectories
        for entry in WalkDir::new(&services_dir)
            .max_depth(2)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("lua") {
                continue;
            }

            let relative_path = path.strip_prefix(&services_dir).unwrap_or(path);
            let profile_name = relative_path
                .with_extension("")
                .to_string_lossy()
                .to_string();

            if profile_name == name {
                return Ok(path.to_path_buf());
            }
        }

        anyhow::bail!(
            "Service profile '{}' not found in {}",
            name,
            services_dir.display()
        )
    }

    /// Check for conflicts between a profile and already enabled profiles
    pub fn check_conflicts(
        &self,
        profile_name: &str,
        enabled_profiles: &[String],
    ) -> Result<Vec<String>> {
        let profile = self.load_profile(profile_name)?;

        let mut conflicts = Vec::new();
        for enabled in enabled_profiles {
            if profile.conflicts.contains(enabled) {
                conflicts.push(enabled.clone());
            }
        }

        // Also check reverse conflicts
        for enabled in enabled_profiles {
            if let Ok(enabled_profile) = self.load_profile(enabled) {
                if enabled_profile
                    .conflicts
                    .contains(&profile_name.to_string())
                    && !conflicts.contains(enabled)
                {
                    conflicts.push(enabled.clone());
                }
            }
        }

        Ok(conflicts)
    }
}
