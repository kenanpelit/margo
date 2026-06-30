pub mod eval;
pub mod system_facts;
pub mod types;

use anyhow::{Context, Result};
use std::path::Path;

use crate::config::{
    Config, DynamicModule, ModuleManifest, ModuleStructure, PackageEntry, PackageType,
};

use eval::{
    check_nix_installed, detect_pointer_nix_config, evaluate_nix_file_to_json, parse_nix_config,
    parse_nix_module, validate_nix_with_eval,
};
use system_facts::SystemFacts;
use types::{NixModule, NixModuleRaw, NixValidationResult};

pub fn load_nix_config(path: &Path) -> Result<Config> {
    let facts = SystemFacts::collect();
    let value = evaluate_nix_file_to_json(path, &facts)
        .with_context(|| format!("Failed to evaluate Nix config: {:?}", path))?;
    let raw: types::NixConfigRaw = parse_nix_config(value)
        .with_context(|| format!("Failed to parse Nix config JSON from {:?}", path))?;
    Ok(raw.into_config())
}

pub fn load_nix_module(path: &Path) -> Result<NixModule> {
    let facts = SystemFacts::collect();
    let value = evaluate_nix_file_to_json(path, &facts)
        .with_context(|| format!("Failed to evaluate Nix module: {:?}", path))?;
    let raw: NixModuleRaw = parse_nix_module(value)
        .with_context(|| format!("Failed to parse Nix module JSON from {:?}", path))?;
    Ok(raw.into_nix_module(path.to_path_buf()))
}

pub fn load_nix_module_as_module_structure(path: &Path) -> Result<ModuleStructure> {
    let nix_module = load_nix_module(path)?;

    Ok(ModuleStructure::Nix(DynamicModule {
        path: nix_module.path,
        description: nix_module.description,
        packages: nix_module.packages,
        services: nix_module.services,
        conflicts: nix_module.conflicts,
        pre_install_hook: nix_module.pre_install_hook,
        post_install_hook: nix_module.post_install_hook,
        hook_behavior: nix_module.hook_behavior,
        pre_hook_behavior: nix_module.pre_hook_behavior,
        post_hook_behavior: nix_module.post_hook_behavior,
        post_disable_hook: nix_module.post_disable_hook,
        post_disable_behavior: nix_module.post_disable_behavior,
        run_hooks_as_user: nix_module.run_hooks_as_user,
        metadata: nix_module.metadata,
        author: nix_module.author,
        version: nix_module.version,
        category: nix_module.category,
        tags: nix_module.tags,
        license: nix_module.license,
        upstream_url: nix_module.upstream_url,
    }))
}

pub fn load_nix_directory_module(path: &Path) -> Result<(ModuleManifest, Vec<PackageEntry>)> {
    let facts = SystemFacts::collect();
    let value = evaluate_nix_file_to_json(path, &facts)
        .with_context(|| format!("Failed to evaluate Nix directory module: {:?}", path))?;
    let raw: NixModuleRaw = parse_nix_module(value)
        .with_context(|| format!("Failed to parse Nix module JSON from {:?}", path))?;

    let manifest = raw.clone().into_module_manifest();

    let mut packages: Vec<PackageEntry> =
        raw.packages.iter().map(|e| e.to_package_entry()).collect();
    for fp in &raw.flatpak_packages {
        packages.push(PackageEntry::WithType {
            name: fp.clone(),
            r#type: Some(PackageType::Flatpak),
        });
    }
    for np in &raw.nix_packages {
        packages.push(PackageEntry::WithType {
            name: np.clone(),
            r#type: Some(PackageType::Nix),
        });
    }

    Ok((manifest, packages))
}

pub fn validate_nix_module_detailed(path: &Path) -> NixValidationResult {
    let facts = SystemFacts::collect();
    validate_nix_with_eval(path, &facts)
}

pub fn is_nix_installed() -> bool {
    check_nix_installed().is_ok()
}

pub fn detect_pointer_nix_config_file(path: &Path) -> Result<Option<String>> {
    let facts = SystemFacts::collect();
    detect_pointer_nix_config(path, &facts)
}

pub fn generate_config_nix(hostname: &str, package_manager: &str) -> String {
    format!(
        r#"{{ system, pkgs }}:

{{
  host = "{hostname}";

  # packages managed by native package manager ({package_manager})
  packages = [
    # "vim"
    # "git"
    # "htop"
  ];

  # flatpak packages (installed via flatpak)
  flatpak_packages = [
    # "com.spotify.Client"
    # "org.mozilla.firefox"
  ];

  # nix packages (installed via home-manager)
  nix_packages = with pkgs; [
    # ripgrep
    # fd
    # bat
  ];

  enabled_modules = [
    # "base"
    # "hardware"
  ];

  # Conditional packages based on system facts
  # packages = if system.hardware.has_nvidia then [ "nvidia-driver" ] else [];

  # Backup configuration
  config_backups = {{
    enabled = true;
    max_backups = 5;
  }};

  # System backup configuration (timeshift/snapper)
  # system_backups = {{
  #   enabled = true;
  #   backup_on_sync = true;
  #   backup_on_update = false;
  #   tool = "timeshift";
  #   snapper_config = "root";
  #   max_backups = 5;
  # }};

  # Nix configuration
  nix = {{
    enabled = false;
    home_manager_enabled = false;
    flake_enabled = false;
  }};
}}"#,
        hostname = hostname,
        package_manager = package_manager,
    )
}

pub fn generate_module_nix(name: &str) -> String {
    format!(
        r#"{{ system, pkgs }}:

{{
  description = "{name} module";

  packages = [
    # Add your packages here
  ];

  flatpak_packages = [
    # "com.example.App"
  ];

  nix_packages = with pkgs; [
    # Add nix packages here
  ];

  # Conditional packages based on system facts
  # packages = if system.hardware.is_laptop then [ "tlp" ] else [];

  conflicts = [];

  # services = {{
  #   enabled = [ "example.service" ];
  #   disabled = [];
  #   scope = "system";
  # }};
}}"#,
        name = name,
    )
}

pub fn generate_pointer_config_nix(hostname: &str) -> String {
    format!(
        r#"{{ system, pkgs }}:

{{
  host = "{hostname}";
}}"#,
        hostname = hostname,
    )
}
