//! `mdots init --nix` / `--nix-init`: Nix configuration files and the
//! Nix + Home Manager integration.

use super::*;

/// Initialize mdots configuration with Nix configuration files
pub(super) fn run_nix_config_init(paths: &ConfigPaths) -> Result<()> {
    println!("{}", "=== Initializing mdots (Nix Mode) ===".blue());
    println!();
    println!(
        "{}",
        "This will set up your configuration using Nix files instead of YAML.".cyan()
    );
    println!(
        "{}",
        "Nix files allow dynamic, conditional configuration based on system facts.".cyan()
    );
    println!();

    if !crate::nix_eval::is_nix_installed() {
        println!(
            "{}",
            "Nix is not installed. Nix config files require the Nix package manager.".yellow()
        );
        println!(
            "{}",
            "  Install Nix first: curl -L https://nixos.org/nix/install | sh".yellow()
        );
        println!("{}", "  Or run: mdots init --nix-init".yellow());
        println!();
        print!("Continue anyway? [y/N] ");
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("{}", "Cancelled".yellow());
            return Ok(());
        }
        println!();
    }

    if paths.config_dir.exists() {
        println!("{}", "mdots config directory already exists".yellow());
        println!("Location: {}", paths.config_dir.display());
        print!("Reinitialize with Nix config? [y/N] ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        if !input.trim().eq_ignore_ascii_case("y") {
            println!("{}", "Cancelled".yellow());
            return Ok(());
        }
    }

    let pkg_manager_type = crate::config::detect_package_manager_type().unwrap_or_else(|_| {
        println!(
            "{} Could not auto-detect package manager, defaulting to pacman",
            "⚠".yellow()
        );
        crate::config::PackageManagerType::Pacman
    });

    let pkg_manager_str = match pkg_manager_type {
        crate::config::PackageManagerType::Pacman => "pacman",
    };

    println!(
        "{} Detected package manager: {}",
        "→".blue(),
        pkg_manager_str.green()
    );
    println!();

    println!("{} Creating directory structure...", "→".blue());

    fs::create_dir_all(&paths.config_dir)?;
    fs::create_dir_all(paths.config_dir.join("hosts"))?;
    fs::create_dir_all(paths.config_dir.join("modules"))?;
    fs::create_dir_all(paths.config_dir.join("scripts"))?;
    fs::create_dir_all(&paths.state_dir)?;

    println!("  {} {}", "✓".green(), paths.config_dir.display());
    println!("  {} {}/hosts", "✓".green(), paths.config_dir.display());
    println!("  {} {}/modules", "✓".green(), paths.config_dir.display());
    println!("  {} {}/scripts", "✓".green(), paths.config_dir.display());
    println!("  {} {}/state", "✓".green(), paths.config_dir.display());

    copy_user_docs(&paths.config_dir)?;

    let state_gitignore = paths.state_dir.join(".gitignore");
    if !state_gitignore.exists() {
        fs::write(&state_gitignore, "# Ignore all state files\n*\n")?;
        println!("  {} state/.gitignore", "✓".green());
    }

    let root_gitignore = paths.config_dir.join(".gitignore");
    let gitignore_content = "# System packages merged from host (auto-generated, host-specific)\nsystem-packages-*.yaml\n";
    if !root_gitignore.exists() {
        fs::write(&root_gitignore, gitignore_content)?;
        println!("  {} .gitignore", "✓".green());
    }

    let hostname = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "localhost".to_string());

    // Create config.nix as pointer
    let config_nix = paths.config_dir.join("config.nix");
    if !config_nix.exists() {
        println!("{} Creating config.nix (pointer)...", "→".blue());
        let config_content = crate::nix_eval::generate_pointer_config_nix(&hostname);
        fs::write(&config_nix, config_content)?;
        println!("  {} config.nix", "✓".green());
    } else {
        println!("  {} config.nix already exists", "→".yellow());
    }

    // Create host config
    let host_nix = paths.config_dir.join(format!("hosts/{}.nix", hostname));
    if !host_nix.exists() {
        println!("{} Creating hosts/{}.nix...", "→".blue(), hostname);
        let host_content = crate::nix_eval::generate_config_nix(&hostname, pkg_manager_str);
        fs::write(&host_nix, host_content)?;
        println!("  {} hosts/{}.nix", "✓".green(), hostname);
    } else {
        println!("  {} hosts/{}.nix already exists", "→".yellow(), hostname);
    }

    // Create base module
    let base_nix = paths.config_dir.join("modules/base.nix");
    if !base_nix.exists() {
        println!("{} Creating modules/base.nix...", "→".blue());
        let base_content = crate::nix_eval::generate_module_nix("Base system packages");
        fs::write(&base_nix, base_content)?;
        println!("  {} modules/base.nix", "✓".green());
    } else {
        println!("  {} modules/base.nix already exists", "→".yellow());
    }

    // Create example module
    let example_nix = paths.config_dir.join("modules/example.nix");
    if !example_nix.exists() {
        println!("{} Creating modules/example.nix...", "→".blue());
        let example_content = crate::nix_eval::generate_module_nix("Example module");
        fs::write(&example_nix, example_content)?;
        println!("  {} modules/example.nix", "✓".green());
    }

    println!();
    println!("{}", "✓ mdots initialized with Nix configuration!".green());
    println!();
    println!("{}", "Structure:".bold());
    println!("  config.nix          → Points to hosts/{}.nix", hostname);
    println!("  hosts/{}.nix   → Your full configuration", hostname);
    println!("  modules/base.nix    → Base packages");
    println!("  modules/             → Optional package modules");
    println!("  scripts/             → Post-install hook scripts");
    println!();
    println!("{}", "Nix config advantages:".bold());
    println!("  • Conditional packages based on system facts");
    println!("  • Access to Nixpkgs via 'with pkgs;'");
    println!("  • Separate: packages, flatpak_packages, nix_packages");
    println!();
    println!("Next steps:");
    println!("  1. Edit host config: hosts/{}.nix", hostname);
    println!("  2. Edit base packages: modules/base.nix");
    println!("  3. Run: mdots validate");
    println!("  4. Run: mdots module list");
    println!("  5. Run: mdots sync --dry-run");

    Ok(())
}

/// Initialize Nix and Home Manager integration
pub(super) fn run_nix_init(paths: &ConfigPaths) -> Result<()> {
    println!(
        "{}",
        "=== Initializing Nix & Home Manager Integration ===".blue()
    );
    println!();

    // Step 1: Install nix if not already installed (do this FIRST,
    // before load_config which may try to evaluate config.nix)
    if crate::nix::is_nix_installed() {
        println!("{} Nix is already installed", "✓".green());

        // Ensure nix-daemon is running (needed before any nix eval)
        if !crate::nix::is_nix_daemon_running() {
            println!("{} nix-daemon is not running, starting it...", "→".blue());
            if let Err(e) = crate::nix::start_nix_daemon() {
                eprintln!(
                    "{} Failed to start nix-daemon: {}\n  Please start it manually: sudo systemctl start nix-daemon",
                    "✗".red(),
                    e
                );
                std::process::exit(1);
            }
            println!("{} nix-daemon started successfully", "✓".green());
        }
    } else {
        // Autodetect package manager for nix installation (can't load config yet)
        let pm_type = crate::config::detect_package_manager_type()?;
        println!("{} Installing Nix...", "→".blue());
        crate::nix::install_nix(&pm_type)?;
        println!("{} Nix installed successfully", "✓".green());
        println!();
        println!(
            "{}",
            "Note: You may need to log out and log back in for PATH changes to take effect."
                .yellow()
        );
        println!(
            "{}",
            "Then run 'mdots init --nix-init' again to continue.".yellow()
        );
        return Ok(());
    }

    // Now it's safe to load the config (nix and daemon are running)
    let config = load_config(paths)?;

    // Step 2: Offer to migrate existing home-manager config
    let migrated = crate::nix::migrate_existing_hm(paths)?;
    if migrated {
        println!("{} Existing home-manager config migrated", "✓".green());
    } else {
        // Step 3: Create home-manager directory
        println!("{} Creating home-manager directory...", "→".blue());
        std::fs::create_dir_all(paths.home_manager_dir())
            .context("Failed to create home-manager directory")?;

        // Get username and home directory
        let username = std::env::var("USER")
            .or_else(|_| std::env::var("LOGNAME"))
            .unwrap_or_else(|_| "user".to_string());
        let home_dir = dirs::home_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| format!("/home/{}", username));

        // Generate home.nix template
        let home_nix_path = paths.home_manager_dir().join("home.nix");
        crate::nix::generate_home_nix_template(&username, &home_dir, &home_nix_path)?;
        println!("  {} Created {}", "✓".green(), home_nix_path.display());

        // Generate empty mdots-packages.nix
        let mdots_packages_path = paths.home_manager_dir().join("mdots-packages.nix");
        crate::nix::generate_mdots_packages_nix(&[], &mdots_packages_path)?;
        println!(
            "  {} Created {}",
            "✓".green(),
            mdots_packages_path.display()
        );
    }

    // Step 4: Ask about flakes
    println!();
    print!(
        "Enable flakes for Home Manager? (Nix flakes provide better reproducibility, recommended) [Y/n] "
    );
    io::stdout().flush()?;
    let mut flake_input = String::new();
    io::stdin().read_line(&mut flake_input)?;
    let use_flakes = !flake_input.trim().eq_ignore_ascii_case("n");
    println!(
        "  {} Using {} mode",
        "→".blue(),
        if use_flakes { "flake" } else { "channel" }
    );

    if use_flakes {
        let hostname = config.host.clone();
        let hm_dir = paths.home_manager_dir();

        // Create hosts/ directory structure if migrating from flat
        if !hm_dir.join("hosts").exists() {
            // Migrate existing mdots-packages.nix from root if it exists
            let old_mdots = hm_dir.join("mdots-packages.nix");
            if old_mdots.exists() {
                println!(
                    "  {} Migrating from flat to per-host structure...",
                    "→".blue()
                );
            }
        }

        // Create this host's per-host directory
        let host_dir = hm_dir.join("hosts").join(&hostname);
        let is_new_host = !host_dir.exists();
        std::fs::create_dir_all(&host_dir)
            .context("Failed to create host home-manager directory")?;

        // Generate mdots-packages.nix for this host
        let mdots_packages_path = host_dir.join("mdots-packages.nix");
        if !mdots_packages_path.exists() || is_new_host {
            crate::nix::generate_mdots_packages_nix(&[], &mdots_packages_path)?;
            println!(
                "  {} Created hosts/{}/mdots-packages.nix",
                "✓".green(),
                hostname
            );
        }

        // Create packages.nix hint for this host (if not exists)
        let packages_nix = host_dir.join("packages.nix");
        if !packages_nix.exists() {
            let hint = format!(
                "{{ pkgs, ... }}:\n{{\n  # Per-host packages for {}\n  home.packages = with pkgs; [\n    # Add packages here\n  ];\n}}\n",
                hostname
            );
            std::fs::write(&packages_nix, hint)?;
            println!("  {} Created hosts/{}/packages.nix", "✓".green(), hostname);
        }

        // Generate shared home.nix (only if it doesn't exist)
        let home_nix_path = hm_dir.join("home.nix");
        if !home_nix_path.exists() {
            crate::nix::generate_shared_home_nix(&home_nix_path)?;
            println!("  {} Created shared home.nix", "✓".green());
        }

        // Generate per-host flake.nix (only if it doesn't exist)
        let flake_nix_path = hm_dir.join("flake.nix");
        if !flake_nix_path.exists() {
            let system_arch = crate::nix::detect_system_arch();
            crate::nix::generate_per_host_flake_nix(&system_arch, &flake_nix_path)?;
            println!("  {} Created flake.nix", "✓".green());
        }

        // Generate flake.lock (only if it doesn't exist)
        if !hm_dir.join("flake.lock").exists() {
            crate::nix::generate_flake_lock(paths)?;
        }
    }

    // Step 5: Set up channels (only if not using flakes)
    if !use_flakes {
        if crate::nix::is_home_manager_installed() {
            println!("{} Home Manager is already installed", "✓".green());
        } else {
            println!("{} Setting up nix channels...", "→".blue());
            crate::nix::setup_channels(
                &config.nix.nixpkgs_channel,
                &config.nix.home_manager_channel,
            )?;
            println!("{} Channels set up", "✓".green());

            println!("{} Installing Home Manager...", "→".blue());
            crate::nix::install_home_manager()?;
            println!("{} Home Manager installed successfully", "✓".green());
        }
    } else {
        if crate::nix::is_home_manager_installed() {
            println!("{} Home Manager is already installed", "✓".green());
        } else {
            println!("{} Installing Home Manager via nix-shell...", "→".blue());
            // For flakes, we still need home-manager binary installed
            // Use the channel approach for the binary even if config uses flakes
            crate::nix::setup_channels(
                &config.nix.nixpkgs_channel,
                &config.nix.home_manager_channel,
            )?;
            crate::nix::install_home_manager()?;
            println!("{} Home Manager installed successfully", "✓".green());
        }
    }

    // Step 6: Update host config to enable nix
    println!("{} Enabling Nix in host config...", "→".blue());
    update_host_config_nix(paths, use_flakes)?;

    // Step 7: Ensure nix is on PATH for the user's shell
    ensure_nix_on_path();

    println!();
    println!(
        "{}",
        "✓ Nix & Home Manager integration initialized!".green()
    );
    println!();
    println!(
        "Home Manager config location: {}",
        paths.home_manager_dir().display()
    );
    if use_flakes {
        println!("Mode: Flakes (flake.nix + flake.lock)");
        println!();
        println!("Next steps:");
        println!(
            "  1. Edit your config: {}",
            "nano ~/.config/mdots/home-manager/home.nix".cyan()
        );
        println!(
            "  2. Add nix packages to modules with {}",
            "type: nix".cyan()
        );
        println!("  3. Run {} to apply changes", "mdots sync".cyan());
        println!(
            "  4. Run {} to update flake inputs and apply",
            "mdots nix update".cyan()
        );
    } else {
        println!("Mode: Channels");
        println!();
        println!("Next steps:");
        println!(
            "  1. Edit your config: {}",
            "nano ~/.config/mdots/home-manager/home.nix".cyan()
        );
        println!(
            "  2. Add nix packages to modules with {}",
            "type: nix".cyan()
        );
        println!("  3. Run {} to apply changes", "mdots sync".cyan());
        println!(
            "  4. Run {} to manually apply home-manager",
            "mdots nix switch".cyan()
        );
    }

    Ok(())
}

/// Update host config to enable nix
pub(super) fn update_host_config_nix(paths: &ConfigPaths, use_flakes: bool) -> Result<()> {
    let config_path = crate::config::resolve_config_path(paths)?;

    if config_path.extension().and_then(|e| e.to_str()) == Some("lua") {
        // For Lua configs, we just print instructions
        println!(
            "  {} Lua config detected. Add this to your config:",
            "→".yellow()
        );
        println!("    nix = {{");
        println!("        enabled = true,");
        println!("        home_manager_enabled = true,");
        if use_flakes {
            println!("        flake_enabled = true,");
        }
        println!("    }}");
        return Ok(());
    }

    // For Nix configs, print instructions (injecting into Nix attrsets is fragile)
    if config_path.extension().and_then(|e| e.to_str()) == Some("nix") {
        println!(
            "  {} Nix config detected. Add this inside your host attribute set:",
            "→".yellow()
        );
        println!("    nix = {{");
        println!("      enabled = true;");
        println!("      home_manager_enabled = true;");
        if use_flakes {
            println!("      flake_enabled = true;");
        }
        println!("      nixpkgs_channel = \"nixpkgs-unstable\";");
        println!("      home_manager_channel = \"release-25.05\";");
        println!("    }};");
        return Ok(());
    }

    // For YAML configs, add nix section
    let content = std::fs::read_to_string(&config_path)?;

    // Check if nix section already exists
    if content.contains("nix:") {
        // Update flake_enabled if needed
        if use_flakes && !content.contains("flake_enabled") {
            let updated = content.replace(
                "home_manager_enabled: true",
                "home_manager_enabled: true\n  flake_enabled: true",
            );
            std::fs::write(&config_path, updated)?;
            println!("  {} Added flake_enabled: true to nix config", "✓".green());
        } else {
            println!("  {} Nix section already exists in config", "→".yellow());
        }
        return Ok(());
    }

    // Add nix section at the end
    let flake_line = if use_flakes {
        "  flake_enabled: true\n"
    } else {
        ""
    };
    let nix_section = format!(
        r#"
# Nix integration
nix:
  enabled: true
  home_manager_enabled: true
  {flake_line}  nixpkgs_channel: nixpkgs-unstable
  home_manager_channel: release-25.05
"#,
        flake_line = flake_line
    );

    let updated = format!("{}{}", content, nix_section);
    std::fs::write(&config_path, updated)?;
    println!(
        "  {} Added nix section to {}",
        "✓".green(),
        config_path.display()
    );

    Ok(())
}

/// Detect the user's shell from $SHELL and ensure ~/.nix-profile/bin is on PATH
pub(super) fn ensure_nix_on_path() {
    let shell = std::env::var("SHELL").unwrap_or_default();
    let shell_name = std::path::Path::new(&shell)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");

    let nix_profile_bin = "$HOME/.nix-profile/bin";
    let path_line: Option<String> = match shell_name {
        "fish" => {
            let fish_conf_d = dirs::home_dir()
                .map(|h| h.join(".config/fish/conf.d"))
                .unwrap_or_else(|| std::path::PathBuf::from("~/.config/fish/conf.d"));
            let nix_file = fish_conf_d.join("nix.fish");

            if !nix_file.exists() {
                let _ = std::fs::create_dir_all(&fish_conf_d);
                let content = format!(
                    "{}\n{}\n",
                    "# nix profile PATH set by mdots",
                    "fish_add_path --move --prepend $HOME/.nix-profile/bin"
                );
                if std::fs::write(&nix_file, &content).is_ok() {
                    println!(
                        "{} Added {} to {}",
                        "✓".green(),
                        nix_profile_bin.cyan(),
                        nix_file.display()
                    );
                }
            }
            None
        }
        "bash" => Some(format!(
            "\n# nix profile PATH set by mdots\nexport PATH=\"{}:\"$PATH\n",
            nix_profile_bin
        )),
        "zsh" => Some(format!(
            "\n# nix profile PATH set by mdots\nexport PATH=\"{}:\"$PATH\n",
            nix_profile_bin
        )),
        _ => None,
    };

    if let Some(line) = path_line {
        let rc_file = match shell_name {
            "bash" => dirs::home_dir()
                .map(|h| h.join(".bashrc"))
                .unwrap_or_else(|| std::path::PathBuf::from("~/.bashrc")),
            "zsh" => dirs::home_dir()
                .map(|h| h.join(".zshrc"))
                .unwrap_or_else(|| std::path::PathBuf::from("~/.zshrc")),
            _ => return,
        };

        if let Ok(existing) = std::fs::read_to_string(&rc_file)
            && !existing.contains("mdots")
            && !existing.contains(nix_profile_bin)
            && std::fs::write(&rc_file, format!("{}{}", existing, line)).is_ok()
        {
            println!(
                "{} Added {} to {}",
                "✓".green(),
                nix_profile_bin.cyan(),
                rc_file.display()
            );
        }
    }
}
