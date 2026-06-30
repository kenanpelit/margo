pub mod builder;
pub mod pkgbuild;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use crate::config::ConfigPaths;

/// A declarative build-from-source configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceConfig {
    /// Package name (used as pacman package name after install)
    pub name: String,

    /// Optional description
    #[serde(default)]
    pub description: String,

    /// Git URL to clone from
    pub url: String,

    /// Branch, tag, or commit to checkout (default: repo default branch)
    #[serde(default)]
    pub branch: Option<String>,

    /// Build-time dependencies (makedepends) — removed after build if --rmdeps
    #[serde(default)]
    pub dependencies: Vec<String>,

    /// Runtime dependencies (depends) — kept after install
    #[serde(default)]
    pub runtime_dependencies: Vec<String>,

    /// Commands to run in the build() function
    #[serde(default)]
    pub build_commands: Vec<String>,

    /// Commands to run in the package() function
    #[serde(default)]
    pub package_commands: Vec<String>,

    /// Path to a custom PKGBUILD to use instead of generating one (relative to source config dir)
    #[serde(default)]
    pub custom_pkgbuild: Option<String>,

    /// Keep build directory between builds for faster rebuilds (default: false = temp dir)
    #[serde(default)]
    pub cache_builds: bool,
}

/// Wraps a SourceConfig with its filesystem path for context
#[derive(Debug, Clone)]
pub struct SourceInfo {
    pub config: SourceConfig,
    /// Directory containing the source.yaml/source.lua (for resolving relative paths)
    pub config_dir: PathBuf,
}

impl SourceInfo {
    /// Resolve custom_pkgbuild path relative to the config directory
    pub fn custom_pkgbuild_path(&self) -> Option<PathBuf> {
        self.config
            .custom_pkgbuild
            .as_ref()
            .map(|p| self.config_dir.join(p))
    }
}

/// YAML wrapper for source.yaml files
#[derive(Debug, Deserialize)]
struct SourceFileYaml {
    name: String,
    #[serde(default)]
    description: String,
    source: SourceSection,
    #[serde(default)]
    build: BuildSection,
    #[serde(default)]
    custom_pkgbuild: Option<String>,
    #[serde(default)]
    cache_builds: bool,
}

#[derive(Debug, Deserialize)]
struct SourceSection {
    url: String,
    #[serde(default)]
    branch: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct BuildSection {
    #[serde(default)]
    dependencies: Vec<String>,
    #[serde(default)]
    runtime_dependencies: Vec<String>,
    #[serde(default)]
    build_commands: Vec<String>,
    #[serde(default)]
    package_commands: Vec<String>,
}

impl From<SourceFileYaml> for SourceConfig {
    fn from(f: SourceFileYaml) -> Self {
        Self {
            name: f.name,
            description: f.description,
            url: f.source.url,
            branch: f.source.branch,
            dependencies: f.build.dependencies,
            runtime_dependencies: f.build.runtime_dependencies,
            build_commands: f.build.build_commands,
            package_commands: f.build.package_commands,
            custom_pkgbuild: f.custom_pkgbuild,
            cache_builds: f.cache_builds,
        }
    }
}

/// Load a source config from a YAML file
pub fn load_source_yaml(path: &Path) -> Result<SourceConfig> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read source config: {}", path.display()))?;
    let parsed: SourceFileYaml = serde_yaml::from_str(&content)
        .with_context(|| format!("Failed to parse source config: {}", path.display()))?;
    Ok(parsed.into())
}

/// Load a source config from a Lua file
pub fn load_source_lua(path: &Path) -> Result<SourceConfig> {
    let lua_module = crate::lua::load_lua_source(path)?;
    Ok(lua_module)
}

/// Discover all source configs in the sources directory
pub fn discover_sources(paths: &ConfigPaths) -> Result<Vec<SourceInfo>> {
    let sources_dir = paths.sources_dir();
    if !sources_dir.exists() {
        return Ok(Vec::new());
    }

    let mut sources = Vec::new();

    for entry in WalkDir::new(&sources_dir)
        .max_depth(3)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        let path = entry.path();
        let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");

        let config = match file_name {
            "source.yaml" => load_source_yaml(path)
                .with_context(|| format!("Failed to load {}", path.display()))?,
            "source.lua" => load_source_lua(path)
                .with_context(|| format!("Failed to load {}", path.display()))?,
            _ => {
                // Also support flat files: {name}.yaml / {name}.lua directly in sources_dir
                let parent = path.parent().unwrap_or(&sources_dir);
                if parent != sources_dir {
                    continue;
                }
                let ext = path.extension().and_then(|s| s.to_str());
                match ext {
                    Some("yaml") => load_source_yaml(path)
                        .with_context(|| format!("Failed to load {}", path.display()))?,
                    Some("lua") => load_source_lua(path)
                        .with_context(|| format!("Failed to load {}", path.display()))?,
                    _ => continue,
                }
            }
        };

        let config_dir = path.parent().unwrap_or(&sources_dir).to_path_buf();

        sources.push(SourceInfo { config, config_dir });
    }

    sources.sort_by(|a, b| a.config.name.cmp(&b.config.name));
    Ok(sources)
}

/// Find a specific source by name
pub fn find_source(paths: &ConfigPaths, name: &str) -> Result<SourceInfo> {
    let sources = discover_sources(paths)?;
    sources
        .into_iter()
        .find(|s| s.config.name == name)
        .with_context(|| {
            format!(
                "Source '{}' not found. Run 'mdots source list' to see available sources.",
                name
            )
        })
}
