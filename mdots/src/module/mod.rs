use anyhow::{Context, Result};
use walkdir::WalkDir;

use crate::config::{load_module, ConfigPaths};

/// Module manager for handling module operations
pub struct ModuleManager {
    paths: ConfigPaths,
}

impl ModuleManager {
    pub fn new(paths: ConfigPaths) -> Self {
        Self { paths }
    }

    /// List all available modules (handles both legacy and directory formats)
    pub fn list_modules(&self) -> Result<Vec<ModuleInfo>> {
        let modules_dir = self.paths.modules_dir();
        if !modules_dir.exists() {
            return Ok(Vec::new());
        }

        let mut modules = Vec::new();
        let mut seen_names = std::collections::HashSet::new();

        // First, collect all directory modules to know which files to skip
        let mut dir_module_paths = std::collections::HashSet::new();
        for entry in WalkDir::new(&modules_dir)
            .max_depth(3)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_dir())
        {
            let path = entry.path();
            if path == modules_dir {
                continue;
            }
            // Check for both module.lua and module.yaml
            if path.join("module.lua").exists() || path.join("module.yaml").exists() {
                dir_module_paths.insert(path.to_path_buf());
            }
        }

        // Find all legacy YAML files and Lua files
        for entry in WalkDir::new(&modules_dir)
            .max_depth(3)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .filter(|e| {
                let ext = e.path().extension().and_then(|s| s.to_str());
                ext == Some("yaml") || ext == Some("lua")
            })
        {
            let path = entry.path();
            let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");

            // Skip module.yaml and module.lua files (they're part of directory modules)
            if path.file_name() == Some(std::ffi::OsStr::new("module.yaml"))
                || path.file_name() == Some(std::ffi::OsStr::new("module.lua"))
            {
                continue;
            }

            // Skip files that are inside directory modules
            let mut is_inside_dir_module = false;
            for dir_module in &dir_module_paths {
                if path.starts_with(dir_module) {
                    is_inside_dir_module = true;
                    break;
                }
            }
            if is_inside_dir_module {
                continue;
            }

            let rel_path = path
                .strip_prefix(&modules_dir)
                .context("Failed to strip prefix")?;

            let module_name = rel_path
                .to_str()
                .context("Invalid UTF-8 in path")?
                .trim_end_matches(".yaml")
                .trim_end_matches(".lua")
                .to_string();

            // Load module using new loader
            let module = crate::config::load_module(path)?;
            let pkg_count = module.packages().len();

            modules.push(ModuleInfo {
                name: module_name.clone(),
                description: module.description().to_string(),
                package_count: pkg_count,
                conflicts: module.conflicts().to_vec(),
                post_install_hook: module.post_install_hook().map(|s| s.to_string()),
                is_directory: module.is_directory(),
                is_lua: ext == "lua",
            });

            seen_names.insert(module_name);
        }

        // Find all directory modules
        for entry in WalkDir::new(&modules_dir)
            .max_depth(3)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_dir())
        {
            let path = entry.path();

            // Skip the modules_dir itself
            if path == modules_dir {
                continue;
            }

            // Check if this directory contains a module.lua or module.yaml
            let has_lua_manifest = path.join("module.lua").exists();
            let has_yaml_manifest = path.join("module.yaml").exists();
            if !has_lua_manifest && !has_yaml_manifest {
                continue;
            }

            let rel_path = path
                .strip_prefix(&modules_dir)
                .context("Failed to strip prefix")?;

            let module_name = rel_path
                .to_str()
                .context("Invalid UTF-8 in path")?
                .to_string();

            // Skip if we already have a legacy module with this name
            if seen_names.contains(&module_name) {
                log::warn!(
                    "Skipping directory module '{}' - conflicts with legacy module",
                    module_name
                );
                continue;
            }

            // Load directory module
            let module = crate::config::load_module(path)?;
            let pkg_count = module.packages().len();

            modules.push(ModuleInfo {
                name: module_name.clone(),
                description: module.description().to_string(),
                package_count: pkg_count,
                conflicts: module.conflicts().to_vec(),
                post_install_hook: module.post_install_hook().map(|s| s.to_string()),
                is_directory: module.is_directory(),
                is_lua: false,
            });

            seen_names.insert(module_name);
        }

        // Sort by name
        modules.sort_by(|a, b| a.name.cmp(&b.name));

        Ok(modules)
    }

    /// Resolve module name to full path (handles short names, full paths, directory modules, and Lua modules)
    pub fn resolve_module_path(&self, module_input: &str) -> Result<String> {
        let modules_dir = self.paths.modules_dir();

        // Check for naming conflicts (yaml + lua + directory with same base name)
        let base_name = module_input.split('/').next().unwrap();
        let yaml_path = modules_dir.join(format!("{}.yaml", base_name));
        let lua_path = modules_dir.join(format!("{}.lua", base_name));
        let dir_path = modules_dir.join(base_name);

        let yaml_exists = yaml_path.exists();
        let lua_exists = lua_path.exists();
        let dir_exists = dir_path.exists()
            && (dir_path.join("module.lua").exists() || dir_path.join("module.yaml").exists());

        let count = [yaml_exists, lua_exists, dir_exists]
            .iter()
            .filter(|&&x| x)
            .count();

        if count > 1 {
            anyhow::bail!(
                "Naming conflict: multiple module formats exist for '{}'. Found: {}{}{}",
                base_name,
                if yaml_exists { "YAML " } else { "" },
                if lua_exists { "Lua " } else { "" },
                if dir_exists { "Directory" } else { "" }
            );
        }

        // If input contains /, treat as full relative path
        if module_input.contains('/') {
            let module_yaml = modules_dir.join(format!("{}.yaml", module_input));
            let module_lua = modules_dir.join(format!("{}.lua", module_input));
            let module_dir = modules_dir.join(module_input);

            // Check nesting depth (warn if > 2 levels)
            let depth = module_input.matches('/').count();
            if depth > 1 {
                log::warn!(
                    "Module '{}' is nested deeper than recommended (max 2 levels)",
                    module_input
                );
            }

            // Prefer directory module over file, then yaml, then lua
            if (module_dir.exists()
                && (module_dir.join("module.lua").exists()
                    || module_dir.join("module.yaml").exists()))
                || module_yaml.exists()
                || module_lua.exists()
            {
                return Ok(module_input.to_string());
            } else {
                anyhow::bail!(
                    "Module '{}' not found at {:?}, {:?}, or {:?}",
                    module_input,
                    module_yaml,
                    module_lua,
                    module_dir
                );
            }
        }

        // Search for module by name (files and directories)
        let mut found_modules = Vec::new();

        // Search for file modules (yaml and lua)
        for entry in WalkDir::new(&modules_dir)
            .max_depth(3)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let path = entry.path();

            // Skip module.yaml and module.lua files (they're part of directory modules)
            if path.file_name() == Some(std::ffi::OsStr::new("module.yaml"))
                || path.file_name() == Some(std::ffi::OsStr::new("module.lua"))
            {
                continue;
            }

            let ext = path.extension().and_then(|s| s.to_str());
            let is_module_file = ext == Some("yaml") || ext == Some("lua");

            if path.file_stem().and_then(|s| s.to_str()) == Some(module_input) && is_module_file {
                let rel_path = path
                    .strip_prefix(&modules_dir)
                    .context("Failed to strip prefix")?;
                let module_path = rel_path
                    .to_str()
                    .context("Invalid UTF-8")?
                    .trim_end_matches(".yaml")
                    .trim_end_matches(".lua");
                found_modules.push(module_path.to_string());
            }
        }

        // Search for directory modules
        for entry in WalkDir::new(&modules_dir)
            .max_depth(3)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_dir())
        {
            let path = entry.path();

            if path == modules_dir {
                continue;
            }

            // Check if it's a directory module (has module.lua or module.yaml)
            if !path.join("module.lua").exists() && !path.join("module.yaml").exists() {
                continue;
            }

            if path.file_name().and_then(|s| s.to_str()) == Some(module_input) {
                let rel_path = path
                    .strip_prefix(&modules_dir)
                    .context("Failed to strip prefix")?;
                let module_path = rel_path.to_str().context("Invalid UTF-8")?;
                found_modules.push(module_path.to_string());
            }
        }

        match found_modules.len() {
            0 => anyhow::bail!(
                "Module '{}' not found. Run 'mdots module list' to see available modules.",
                module_input
            ),
            1 => Ok(found_modules[0].clone()),
            _ => {
                let mut err_msg = format!("Multiple modules found with name '{}':\n", module_input);
                for m in &found_modules {
                    err_msg.push_str(&format!("  - {}\n", m));
                }
                err_msg.push_str(&format!(
                    "Please specify the full path (e.g., 'mdots module enable {}')",
                    found_modules[0]
                ));
                anyhow::bail!(err_msg);
            }
        }
    }

    /// Check if a module conflicts with any enabled modules
    pub fn check_conflicts(
        &self,
        module_name: &str,
        enabled_modules: &[String],
    ) -> Result<Vec<String>> {
        // Resolve module path for any format (yaml, lua, or directory)
        let modules_dir = self.paths.modules_dir();
        let module_file = modules_dir.join(format!("{}.yaml", module_name));
        let module_lua = modules_dir.join(format!("{}.lua", module_name));
        let module_dir = modules_dir.join(module_name);

        let module_path = if module_dir.exists()
            && (module_dir.join("module.yaml").exists() || module_dir.join("module.lua").exists())
        {
            module_dir
        } else if module_file.exists() {
            module_file
        } else if module_lua.exists() {
            module_lua
        } else {
            return Ok(Vec::new());
        };

        let loaded_module = load_module(&module_path)?;
        let mut conflicts = Vec::new();

        for conflict in loaded_module.conflicts() {
            if enabled_modules.contains(conflict) {
                conflicts.push(conflict.clone());
            }
        }

        Ok(conflicts)
    }
}

#[allow(dead_code)] // kept: module metadata parsed for introspection; not all fields consumed yet
#[derive(Debug, Clone)]
pub struct ModuleInfo {
    pub name: String,
    pub description: String,
    pub package_count: usize,
    pub conflicts: Vec<String>,
    pub post_install_hook: Option<String>,
    pub is_directory: bool,
    pub is_lua: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_module_manager_creation() {
        let paths = ConfigPaths::default();
        let _manager = ModuleManager::new(paths);
    }
}
