use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use colored::Colorize;
use log::{debug, info, warn};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::process::Command;

use crate::config::ServiceScope;

/// Represents the state of a systemd service
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServiceState {
    /// Service is enabled (starts on boot)
    Enabled,
    /// Service is disabled (does not start on boot)
    Disabled,
    /// Service is masked (cannot be started)
    Masked,
    /// Service state is unknown or indeterminate
    Unknown,
}

impl ServiceState {
    pub fn is_enabled(&self) -> bool {
        matches!(self, ServiceState::Enabled)
    }

    #[allow(dead_code)]
    pub fn is_disabled(&self) -> bool {
        matches!(self, ServiceState::Disabled)
    }
}

/// Represents the running state of a systemd service
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServiceActiveState {
    /// Service is currently running
    Active,
    /// Service is not running
    Inactive,
    /// Service failed to start
    Failed,
    /// Service state is unknown
    Unknown,
}

/// State tracking for services (persisted to services-state.yaml)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServicesState {
    /// Timestamp when this state was last updated
    #[serde(default = "Utc::now")]
    pub last_updated: DateTime<Utc>,

    /// Services that are currently enabled by dcli
    #[serde(default)]
    pub enabled_services: Vec<String>,

    /// Services that are currently disabled by dcli
    #[serde(default)]
    pub disabled_services: Vec<String>,
}

impl Default for ServicesState {
    fn default() -> Self {
        Self {
            last_updated: Utc::now(),
            enabled_services: Vec::new(),
            disabled_services: Vec::new(),
        }
    }
}

/// Preview of service changes (for dry-run)
#[derive(Debug, Clone, Default)]
pub struct ServicesPreview {
    pub services_to_enable: Vec<String>,
    pub services_to_disable: Vec<String>,
    pub services_to_start: Vec<String>,
    pub services_to_stop: Vec<String>,
}

impl ServicesPreview {
    pub fn has_changes(&self) -> bool {
        !self.services_to_enable.is_empty()
            || !self.services_to_disable.is_empty()
            || !self.services_to_start.is_empty()
            || !self.services_to_stop.is_empty()
    }
}

/// Report of service sync operations
#[derive(Debug, Clone)]
pub struct ServicesSyncReport {
    pub services_enabled: Vec<String>,
    pub services_disabled: Vec<String>,
    pub services_started: Vec<String>,
    pub services_stopped: Vec<String>,
    pub errors: Vec<ServiceError>,
}

impl ServicesSyncReport {
    pub fn new() -> Self {
        Self {
            services_enabled: Vec::new(),
            services_disabled: Vec::new(),
            services_started: Vec::new(),
            services_stopped: Vec::new(),
            errors: Vec::new(),
        }
    }

    pub fn has_changes(&self) -> bool {
        !self.services_enabled.is_empty()
            || !self.services_disabled.is_empty()
            || !self.services_started.is_empty()
            || !self.services_stopped.is_empty()
    }

    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }
}

/// Error information for service operations
#[derive(Debug, Clone)]
pub struct ServiceError {
    pub service_name: String,
}

/// Service manager for systemd operations
pub struct ServiceManager;

impl ServiceManager {
    /// Get the enabled/disabled state of a service
    pub fn get_service_state(service: &str, scope: ServiceScope) -> Result<ServiceState> {
        debug!(
            "Checking enabled state for service: {} (scope: {:?})",
            service, scope
        );

        let mut cmd = Command::new("systemctl");
        if let Some(flag) = scope.as_flag() {
            cmd.arg(flag);
        }
        cmd.args(["is-enabled", service]);

        let output = cmd
            .output()
            .context(format!("Failed to check if service {} is enabled", service))?;

        let state_str = String::from_utf8_lossy(&output.stdout).trim().to_string();

        let state = match state_str.as_str() {
            "enabled" | "enabled-runtime" | "static" | "indirect" => ServiceState::Enabled,
            "disabled" => ServiceState::Disabled,
            "masked" | "masked-runtime" => ServiceState::Masked,
            _ => ServiceState::Unknown,
        };

        debug!(
            "Service {} enabled state: {:?} (raw: {})",
            service, state, state_str
        );
        Ok(state)
    }

    /// Get the active/running state of a service
    pub fn get_active_state(service: &str, scope: ServiceScope) -> Result<ServiceActiveState> {
        debug!(
            "Checking active state for service: {} (scope: {:?})",
            service, scope
        );

        let mut cmd = Command::new("systemctl");
        if let Some(flag) = scope.as_flag() {
            cmd.arg(flag);
        }
        cmd.args(["is-active", service]);

        let output = cmd
            .output()
            .context(format!("Failed to check if service {} is active", service))?;

        let state_str = String::from_utf8_lossy(&output.stdout).trim().to_string();

        let state = match state_str.as_str() {
            "active" => ServiceActiveState::Active,
            "inactive" => ServiceActiveState::Inactive,
            "failed" => ServiceActiveState::Failed,
            _ => ServiceActiveState::Unknown,
        };

        debug!("Service {} active state: {:?}", service, state);
        Ok(state)
    }

    /// Check if a service exists on the system
    pub fn service_exists(service: &str, scope: ServiceScope) -> bool {
        debug!(
            "Checking if service exists: {} (scope: {:?})",
            service, scope
        );

        let mut cmd = Command::new("systemctl");
        if let Some(flag) = scope.as_flag() {
            cmd.arg(flag);
        }
        cmd.args([
            "list-unit-files",
            "--type=service",
            "--no-pager",
            "--no-legend",
        ]);

        let output = cmd.output();

        if let Ok(output) = output {
            let output_str = String::from_utf8_lossy(&output.stdout);
            // Check for exact service name match (with or without .service suffix)
            let service_with_suffix = if service.ends_with(".service") {
                service.to_string()
            } else {
                format!("{}.service", service)
            };

            let exists = output_str.lines().any(|line| {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if let Some(name) = parts.first() {
                    *name == service_with_suffix || *name == service
                } else {
                    false
                }
            });

            debug!("Service {} exists: {}", service, exists);
            exists
        } else {
            warn!("Failed to check if service {} exists", service);
            false
        }
    }

    /// Validate a service name (prevent command injection)
    pub fn validate_service_name(service: &str) -> Result<()> {
        // Service names should only contain alphanumeric, dash, underscore, dot, @
        if service.is_empty() {
            return Err(anyhow!("Service name cannot be empty"));
        }

        if !service
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.' || c == '@')
        {
            return Err(anyhow!(
                "Invalid service name '{}': service names can only contain alphanumeric characters, dashes, underscores, dots, and @",
                service
            ));
        }

        Ok(())
    }

    /// Enable a service (does not start it)
    pub fn enable_service(service: &str, scope: ServiceScope) -> Result<()> {
        Self::validate_service_name(service)?;

        info!("Enabling service: {} (scope: {:?})", service, scope);

        let mut cmd = Command::new("systemctl");
        if let Some(flag) = scope.as_flag() {
            cmd.arg(flag);
        }
        cmd.args(["enable", service]);

        let status = cmd
            .status()
            .context(format!("Failed to enable service {}", service))?;

        if !status.success() {
            return Err(anyhow!("Failed to enable service {}", service));
        }

        Ok(())
    }

    /// Disable a service (does not stop it)
    pub fn disable_service(service: &str, scope: ServiceScope) -> Result<()> {
        Self::validate_service_name(service)?;

        info!("Disabling service: {} (scope: {:?})", service, scope);

        let mut cmd = Command::new("systemctl");
        if let Some(flag) = scope.as_flag() {
            cmd.arg(flag);
        }
        cmd.args(["disable", service]);

        let status = cmd
            .status()
            .context(format!("Failed to disable service {}", service))?;

        if !status.success() {
            return Err(anyhow!("Failed to disable service {}", service));
        }

        Ok(())
    }

    /// Start a service
    pub fn start_service(service: &str, scope: ServiceScope) -> Result<()> {
        Self::validate_service_name(service)?;

        info!("Starting service: {} (scope: {:?})", service, scope);

        let mut cmd = Command::new("systemctl");
        if let Some(flag) = scope.as_flag() {
            cmd.arg(flag);
        }
        cmd.args(["start", service]);

        let status = cmd
            .status()
            .context(format!("Failed to start service {}", service))?;

        if !status.success() {
            return Err(anyhow!("Failed to start service {}", service));
        }

        Ok(())
    }

    /// Stop a service
    pub fn stop_service(service: &str, scope: ServiceScope) -> Result<()> {
        Self::validate_service_name(service)?;

        info!("Stopping service: {} (scope: {:?})", service, scope);

        let mut cmd = Command::new("systemctl");
        if let Some(flag) = scope.as_flag() {
            cmd.arg(flag);
        }
        cmd.args(["stop", service]);

        let status = cmd
            .status()
            .context(format!("Failed to stop service {}", service))?;

        if !status.success() {
            return Err(anyhow!("Failed to stop service {}", service));
        }

        Ok(())
    }

    /// Get list of all currently enabled services on the system
    pub fn get_all_enabled_services(scope: crate::config::ServiceScope) -> Result<Vec<String>> {
        debug!("Getting all enabled services");

        let mut cmd = Command::new("systemctl");
        if let Some(flag) = scope.as_flag() {
            cmd.arg(flag);
        }
        cmd.args([
            "list-unit-files",
            "--type=service",
            "--state=enabled",
            "--no-pager",
            "--no-legend",
        ]);
        let output = cmd.output().context("Failed to get enabled services")?;

        let output_str = String::from_utf8_lossy(&output.stdout);
        let services: Vec<String> = output_str
            .lines()
            .filter_map(|line| {
                let parts: Vec<&str> = line.split_whitespace().collect();
                parts.first().map(|s| {
                    // Remove .service suffix if present
                    s.trim_end_matches(".service").to_string()
                })
            })
            .collect();

        debug!("Found {} enabled services", services.len());
        Ok(services)
    }

    /// Preview what service changes would be made (for dry-run)
    pub fn preview_services(
        enabled_services: &[String],
        disabled_services: &[String],
        previous_state: &ServicesState,
        scope: ServiceScope,
    ) -> Result<ServicesPreview> {
        let mut preview = ServicesPreview::default();

        // Check if services configuration has changed
        let enabled_changed = enabled_services != previous_state.enabled_services.as_slice();
        let disabled_changed = disabled_services != previous_state.disabled_services.as_slice();

        // Skip if nothing has changed
        if !enabled_changed && !disabled_changed {
            return Ok(preview);
        }

        // Check services to enable
        for service in enabled_services {
            // Skip invalid service names
            if Self::validate_service_name(service).is_err() {
                continue;
            }

            // Skip if in both lists (conflict)
            if disabled_services.contains(service) {
                continue;
            }

            // Check if service exists
            if !Self::service_exists(service, scope) {
                continue;
            }

            // Get current state
            let current_state = match Self::get_service_state(service, scope) {
                Ok(state) => state,
                Err(_) => continue,
            };

            // Would enable if not already enabled
            if !current_state.is_enabled() {
                preview.services_to_enable.push(service.clone());

                // Would also start if not active
                let active_state = match Self::get_active_state(service, scope) {
                    Ok(state) => state,
                    Err(_) => continue,
                };

                if active_state != ServiceActiveState::Active {
                    preview.services_to_start.push(service.clone());
                }
            }
        }

        // Check services to disable
        for service in disabled_services {
            // Skip invalid service names
            if Self::validate_service_name(service).is_err() {
                continue;
            }

            // Skip if in both lists (conflict)
            if enabled_services.contains(service) {
                continue;
            }

            // Check if service exists
            if !Self::service_exists(service, scope) {
                continue;
            }

            // Get current state
            let current_state = match Self::get_service_state(service, scope) {
                Ok(state) => state,
                Err(_) => continue,
            };

            // Get active state
            let active_state = match Self::get_active_state(service, scope) {
                Ok(state) => state,
                Err(_) => continue,
            };

            // Would stop if currently active
            if active_state == ServiceActiveState::Active {
                preview.services_to_stop.push(service.clone());
            }

            // Would disable if currently enabled
            if current_state.is_enabled() {
                preview.services_to_disable.push(service.clone());
            }
        }

        Ok(preview)
    }

    /// Sync services based on configuration
    /// This is the main function that orchestrates service changes
    pub fn sync_services(
        enabled_services: &[String],
        disabled_services: &[String],
        previous_state: &ServicesState,
        scope: ServiceScope,
    ) -> Result<ServicesSyncReport> {
        let mut report = ServicesSyncReport::new();

        // Check if services configuration has changed
        let enabled_changed = enabled_services != previous_state.enabled_services.as_slice();
        let disabled_changed = disabled_services != previous_state.disabled_services.as_slice();

        // Skip sync if nothing has changed
        if !enabled_changed && !disabled_changed {
            crate::ui::step("Services", "already in sync");
            debug!("Services configuration unchanged, skipping sync");
            return Ok(report);
        }

        let scope_str = match scope {
            ServiceScope::System => "system",
            ServiceScope::User => "user",
        };
        crate::ui::step("Syncing", &format!("services ({})", scope_str));

        // Validate all service names first
        for service in enabled_services.iter().chain(disabled_services.iter()) {
            if let Err(_e) = Self::validate_service_name(service) {
                report.errors.push(ServiceError {
                    service_name: service.clone(),
                });
            }
        }

        // Check for conflicts (service in both enabled and disabled)
        for service in enabled_services {
            if disabled_services.contains(service) {
                warn!(
                    "Service {} is in both enabled and disabled lists, skipping",
                    service
                );
                report.errors.push(ServiceError {
                    service_name: service.clone(),
                });
            }
        }

        // Process services to enable
        for service in enabled_services {
            // Skip if there were validation errors for this service
            if report.errors.iter().any(|e| e.service_name == *service) {
                continue;
            }

            // Check if service exists
            if !Self::service_exists(service, scope) {
                warn!(
                    "Service {} does not exist on system (scope: {}), skipping",
                    service, scope_str
                );
                report.errors.push(ServiceError {
                    service_name: service.clone(),
                });
                continue;
            }

            // Get current state
            let current_state = match Self::get_service_state(service, scope) {
                Ok(state) => state,
                Err(_e) => {
                    report.errors.push(ServiceError {
                        service_name: service.clone(),
                    });
                    continue;
                }
            };

            // Track if we just enabled this service in this sync
            let mut just_enabled = false;

            // Enable if not already enabled
            if !current_state.is_enabled() {
                match Self::enable_service(service, scope) {
                    Ok(_) => {
                        println!(
                            "  {} {}",
                            "✓".green(),
                            format!("Enabled {}", service).green()
                        );
                        report.services_enabled.push(service.clone());
                        just_enabled = true;
                    }
                    Err(e) => {
                        eprintln!(
                            "  {} {}: {}",
                            "✗".red(),
                            format!("Failed to enable {}", service).red(),
                            e
                        );
                        report.errors.push(ServiceError {
                            service_name: service.clone(),
                        });
                        continue;
                    }
                }
            }

            // Only start if we just enabled it (not already running)
            // This prevents trying to start already-running services on every sync
            if just_enabled {
                let active_state = match Self::get_active_state(service, scope) {
                    Ok(state) => state,
                    Err(_e) => {
                        report.errors.push(ServiceError {
                            service_name: service.clone(),
                        });
                        continue;
                    }
                };

                if active_state != ServiceActiveState::Active {
                    match Self::start_service(service, scope) {
                        Ok(_) => {
                            println!(
                                "  {} {}",
                                "✓".green(),
                                format!("Started {}", service).green()
                            );
                            report.services_started.push(service.clone());
                        }
                        Err(e) => {
                            eprintln!(
                                "  {} {}: {}",
                                "✗".red(),
                                format!("Failed to start {}", service).red(),
                                e
                            );
                            report.errors.push(ServiceError {
                                service_name: service.clone(),
                            });
                        }
                    }
                }
            }
        }

        // Process services to disable
        for service in disabled_services {
            // Skip if there were validation errors for this service
            if report.errors.iter().any(|e| e.service_name == *service) {
                continue;
            }

            // Check if service exists
            if !Self::service_exists(service, scope) {
                warn!(
                    "Service {} does not exist on system (scope: {}), skipping",
                    service, scope_str
                );
                report.errors.push(ServiceError {
                    service_name: service.clone(),
                });
                continue;
            }

            // Get current state
            let current_state = match Self::get_service_state(service, scope) {
                Ok(state) => state,
                Err(_e) => {
                    report.errors.push(ServiceError {
                        service_name: service.clone(),
                    });
                    continue;
                }
            };

            // Stop if currently active
            let active_state = match Self::get_active_state(service, scope) {
                Ok(state) => state,
                Err(_e) => {
                    report.errors.push(ServiceError {
                        service_name: service.clone(),
                    });
                    continue;
                }
            };

            if active_state == ServiceActiveState::Active {
                match Self::stop_service(service, scope) {
                    Ok(_) => {
                        println!(
                            "  {} {}",
                            "✓".green(),
                            format!("Stopped {}", service).green()
                        );
                        report.services_stopped.push(service.clone());
                    }
                    Err(e) => {
                        eprintln!(
                            "  {} {}: {}",
                            "✗".red(),
                            format!("Failed to stop {}", service).red(),
                            e
                        );
                        report.errors.push(ServiceError {
                            service_name: service.clone(),
                        });
                        continue;
                    }
                }
            }

            // Disable if currently enabled
            if current_state.is_enabled() {
                match Self::disable_service(service, scope) {
                    Ok(_) => {
                        println!(
                            "  {} {}",
                            "✓".green(),
                            format!("Disabled {}", service).green()
                        );
                        report.services_disabled.push(service.clone());
                    }
                    Err(e) => {
                        eprintln!(
                            "  {} {}: {}",
                            "✗".red(),
                            format!("Failed to disable {}", service).red(),
                            e
                        );
                        report.errors.push(ServiceError {
                            service_name: service.clone(),
                        });
                    }
                }
            }
        }

        // Print summary
        if report.has_changes() {
            println!();
            if !report.services_enabled.is_empty() {
                println!("Services enabled: {}", report.services_enabled.len());
            }
            if !report.services_disabled.is_empty() {
                println!("Services disabled: {}", report.services_disabled.len());
            }
            if !report.services_started.is_empty() {
                println!("Services started: {}", report.services_started.len());
            }
            if !report.services_stopped.is_empty() {
                println!("Services stopped: {}", report.services_stopped.len());
            }
        } else {
            println!("  No service changes needed");
        }

        if !report.errors.is_empty() {
            println!();
            eprintln!(
                "{}: {} service operations failed",
                "Warning".yellow(),
                report.errors.len()
            );
        }

        Ok(report)
    }
}

/// Load services state from YAML file
pub fn load_services_state(state_file: &Path) -> Result<ServicesState> {
    if !state_file.exists() {
        debug!("Services state file does not exist, returning default state");
        return Ok(ServicesState::default());
    }

    let content = fs::read_to_string(state_file).context(format!(
        "Failed to read services state file: {:?}",
        state_file
    ))?;

    let state: ServicesState =
        serde_yaml::from_str(&content).context("Failed to parse services state YAML")?;

    debug!("Loaded services state from {:?}", state_file);
    Ok(state)
}

/// Save services state to YAML file
pub fn save_services_state(state_file: &Path, state: &ServicesState) -> Result<()> {
    // Create parent directory if it doesn't exist
    if let Some(parent) = state_file.parent() {
        fs::create_dir_all(parent)
            .context(format!("Failed to create state directory: {:?}", parent))?;
    }

    let yaml =
        serde_yaml::to_string(state).context("Failed to serialize services state to YAML")?;

    fs::write(state_file, yaml).context(format!(
        "Failed to write services state file: {:?}",
        state_file
    ))?;

    debug!("Saved services state to {:?}", state_file);
    Ok(())
}

/// Create updated services state from sync report
pub fn create_updated_state(
    enabled_services: &[String],
    disabled_services: &[String],
) -> ServicesState {
    ServicesState {
        last_updated: Utc::now(),
        enabled_services: enabled_services.to_vec(),
        disabled_services: disabled_services.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_service_name() {
        assert!(ServiceManager::validate_service_name("sshd").is_ok());
        assert!(ServiceManager::validate_service_name("NetworkManager").is_ok());
        assert!(ServiceManager::validate_service_name("getty@tty1").is_ok());
        assert!(ServiceManager::validate_service_name("bluetooth.service").is_ok());

        assert!(ServiceManager::validate_service_name("").is_err());
        assert!(ServiceManager::validate_service_name("service; rm -rf /").is_err());
        assert!(ServiceManager::validate_service_name("service && malicious").is_err());
    }

    #[test]
    fn test_service_state_methods() {
        assert!(ServiceState::Enabled.is_enabled());
        assert!(!ServiceState::Disabled.is_enabled());

        assert!(ServiceState::Disabled.is_disabled());
        assert!(!ServiceState::Enabled.is_disabled());
    }
}
