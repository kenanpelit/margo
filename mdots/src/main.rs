mod backend;
mod commands;
mod config;
mod defaults;
mod dotfiles;
mod lua;
mod module;
mod nix;
mod nix_eval;
mod package;
mod process;
mod progress;
mod secrets;
mod service_profile;
mod services;
mod source;
mod theming;
mod tui;
mod ui;

use anyhow::Result;
use clap::{Parser, Subcommand};
use clap_complete::Shell;

use config::ConfigPaths;

#[derive(Parser)]
#[command(name = "mdots")]
#[command(author, version, about = "A declarative package management CLI tool for Linux", long_about = None)]
struct Cli {
    /// Output in JSON format (for programmatic use)
    #[arg(short, long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize mdots configuration directory structure
    Init {
        /// Bootstrap from BlackDon's config
        #[arg(short = 'b', long = "bd")]
        bd: bool,

        /// Advanced setup with Lua configuration files
        #[arg(short = 'a', long = "adv", visible_alias = "lua")]
        adv: bool,

        /// Initialize with Nix configuration files
        #[arg(long = "nix")]
        nix: bool,

        /// Install Nix and set up Home Manager integration
        #[arg(long = "nix-init")]
        nix_init: bool,
    },

    /// Install a package and add to mdots management
    Install {
        /// Package name
        package: String,
    },

    /// Remove a package and untrack from mdots
    Remove {
        /// Package name
        package: String,
    },

    /// Show current configuration and sync status
    Status,

    /// Show declared-vs-installed package diff (read-only)
    Diff,

    /// Sync packages to match configuration
    Sync {
        /// Preview changes without applying
        #[arg(short, long)]
        dry_run: bool,

        /// Remove packages not in configuration
        #[arg(long)]
        prune: bool,

        /// Skip confirmation prompts
        #[arg(long)]
        force: bool,

        /// Skip automatic backup
        #[arg(long)]
        no_backup: bool,

        /// Skip post-install hooks
        #[arg(long)]
        no_hooks: bool,

        /// Force re-sync dotfiles even if already synced
        #[arg(long)]
        force_dotfiles: bool,

        /// Automatically commit changes to git after successful sync
        #[arg(long)]
        auto_commit: bool,
    },

    /// Module management commands
    #[command(alias = "modules")]
    Module {
        #[command(subcommand)]
        action: ModuleAction,
    },

    /// Service profile management commands
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },

    /// SOPS/age secrets management commands
    Secrets {
        #[command(subcommand)]
        action: SecretsAction,
    },

    /// Update system (respects version constraints)
    Update {
        /// Skip automatic backup
        #[arg(long)]
        no_backup: bool,

        /// Skip update hooks
        #[arg(long)]
        no_hooks: bool,

        /// Update VCS packages (e.g., -git packages) by passing --devel to AUR helper
        #[arg(long)]
        devel: bool,
    },

    /// Add unmanaged installed packages to system-packages.yaml
    Merge {
        /// Preview packages that would be added
        #[arg(short, long)]
        dry_run: bool,

        /// Merge currently enabled services instead of packages
        #[arg(long)]
        services: bool,

        /// Merge user-scope services (systemctl --user) instead of system services
        #[arg(long)]
        user: bool,

        /// Merge current default applications
        #[arg(long)]
        defaults: bool,

        /// Include all installed packages (including dependencies) in a separate module
        #[arg(long)]
        include_deps: bool,
    },

    /// Migrate from old structure (packages/) to new structure (hosts/, modules/)
    Migrate {
        /// Preview migration without applying changes
        #[arg(short, long)]
        dry_run: bool,
    },

    /// Find where a package is defined in your mdots config
    Find {
        /// Package name to search for
        package: String,
    },

    /// Remove a package from mdots tracking without uninstalling it
    Forget {
        /// Package name to forget
        package: String,
    },

    /// Validate mdots config structure and modules
    Validate {
        /// Also check if packages exist in repos (slower)
        #[arg(long)]
        check_packages: bool,
    },

    /// Save current configuration as a backup
    SaveConfig,

    /// Restore configuration from a backup
    RestoreConfig {
        /// Optional backup name (will prompt interactively if not provided)
        backup: Option<String>,
    },

    /// Post-install hook management
    Hooks {
        #[command(subcommand)]
        action: HooksAction,
    },

    /// Git repository management
    Repo {
        #[command(subcommand)]
        action: RepoAction,
    },

    /// Backup/snapshot management
    Backup {
        #[command(subcommand)]
        action: Option<BackupAction>,
    },

    /// Restore from backup snapshot
    Restore {
        /// Snapshot ID/name
        snapshot: Option<String>,
    },

    /// Update mdots from git repository
    SelfUpdate,

    /// Nix package manager and Home Manager integration
    Nix {
        #[command(subcommand)]
        action: NixAction,
    },

    /// Search for packages with interactive TUI
    Search,

    /// Edit configuration files with interactive TUI selector
    Edit,

    /// Launch interactive TUI
    Tui,

    /// Generate configuration files
    Generate {
        #[command(subcommand)]
        action: GenerateAction,
    },

    /// Build packages from source using makepkg
    Source {
        #[command(subcommand)]
        action: SourceAction,
    },

    /// Generate a shell completion script (bash, zsh, fish, …)
    Completion {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: Shell,
    },

    /// Print the man page (roff) to stdout
    Man,

    /// Run environment health checks and report pass/warn/fail status
    Doctor,
}

#[derive(Subcommand)]
enum BackupAction {
    /// List backup snapshots
    List,

    /// Delete a snapshot
    Delete {
        /// Snapshot ID/name
        snapshot: String,
    },

    /// Check backup configuration
    Check,
}

#[derive(Subcommand)]
enum HooksAction {
    /// List all hooks and their execution status
    List,

    /// Reset a hook to "not run" state (will run on next sync)
    Reset {
        /// Module name
        module: String,
        /// Reset the pre-install hook instead of post-install
        #[arg(long)]
        pre: bool,
        /// Reset the disable hook instead of post-install
        #[arg(long)]
        disable: bool,
    },

    /// Skip a hook permanently (mark as "don't run")
    Skip {
        /// Module name
        module: String,
        /// Skip the pre-install hook instead of post-install
        #[arg(long)]
        pre: bool,
        /// Skip the disable hook instead of post-install
        #[arg(long)]
        disable: bool,
    },

    /// Manually run a module's hook
    Run {
        /// Module name
        module: String,
        /// Run the pre-install hook instead of post-install
        #[arg(long)]
        pre: bool,
        /// Run the disable hook instead of post-install
        #[arg(long)]
        disable: bool,
    },
}

#[derive(Subcommand)]
enum RepoAction {
    /// Initialize git repository for mdots config
    Init,

    /// Clone existing mdots config repository
    Clone,

    /// Commit and push changes
    Push,

    /// Pull updates from remote
    Pull,

    /// Show repository status
    Status,
}

#[derive(Subcommand)]
enum ModuleAction {
    /// List all available modules
    List,

    /// Enable a module
    Enable {
        /// Module name(s) or path(s)
        module_names: Vec<String>,

        /// Enable modules without prompting to run sync afterward
        #[arg(long)]
        skip_sync: bool,
    },

    /// Disable a module
    Disable {
        /// Module name or path
        name: Option<String>,
    },

    /// Run a module's post-install hook
    RunHook {
        /// Module name or path (if not provided, shows interactive list)
        name: Option<String>,
    },

    /// Create a new module with template
    Create {
        /// Module path (e.g., "sddm" or "login-managers/sddm")
        path: String,

        /// Force Lua format even if using YAML config
        #[arg(long)]
        lua: bool,

        /// Force Nix format even if using YAML config
        #[arg(long)]
        nix: bool,
    },
}

#[derive(Subcommand)]
enum ServiceAction {
    /// List all available service profiles
    List,

    /// Enable a service profile
    Enable {
        /// Service profile name
        name: Option<String>,
    },

    /// Disable a service profile
    Disable {
        /// Service profile name
        name: Option<String>,
    },

    /// Show details of a service profile
    Show {
        /// Service profile name
        name: String,
    },
}

#[derive(Subcommand)]
enum SecretsAction {
    /// Show the status of declared secrets (read-only)
    Status,

    /// Decrypt declared secrets into place (without a full sync)
    Sync {
        /// Show what would change without writing anything
        #[arg(long)]
        dry_run: bool,

        /// Remove plaintext targets of secrets that are no longer declared
        #[arg(long)]
        prune: bool,
    },

    /// List declared secrets (from config, no filesystem access)
    List,

    /// Edit an encrypted secret with `sops`
    Edit {
        /// Secret name (see `mdots secrets list`)
        name: String,
    },

    /// Generate an age key if one does not already exist
    Keygen,
}

#[derive(Subcommand)]
enum GenerateAction {
    /// Generate hardware.lua module with auto-detection
    Hardware {
        /// Overwrite existing hardware.lua if it exists
        #[arg(short, long)]
        force: bool,
    },
    /// Generate service.lua module with detected services
    Service {
        /// Overwrite existing service.lua if it exists
        #[arg(short, long)]
        force: bool,
    },
    /// Generate storage.lua module with detected storage configuration
    Storage {
        /// Overwrite existing storage.lua if it exists
        #[arg(short, long)]
        force: bool,
    },
    /// Generate util.lua module with system utilities
    Util {
        /// Overwrite existing util.lua if it exists
        #[arg(short, long)]
        force: bool,
    },
}

#[derive(Subcommand)]
enum NixAction {
    /// Install Nix and Home Manager
    Install,

    /// Run Home Manager switch
    Switch,

    /// Update Nix channels and run Home Manager switch
    Update,

    /// Search Nixpkgs for packages
    Search {
        /// Search query
        query: String,
    },

    /// Show Nix and Home Manager status
    Status,
}

#[derive(Subcommand)]
enum SourceAction {
    /// List all declared sources and their install status
    List,

    /// Build and install sources (all by default, or a specific one by name)
    Build {
        /// Source name to build (builds all if not specified)
        name: Option<String>,
    },

    /// Force clean rebuild of sources (all by default, or a specific one by name)
    Rebuild {
        /// Source name to rebuild (rebuilds all if not specified)
        name: Option<String>,
    },

    /// Uninstall a source-built package via pacman
    Remove {
        /// Source package name to remove
        name: String,
    },

    /// Show install status of all sources
    Status,
}

fn main() -> Result<()> {
    // Initialize logger
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let cli = Cli::parse();

    // Completion and man-page generation must work without any config — e.g.
    // at package build time, in a sandbox that may have no usable HOME — so
    // handle them before constructing ConfigPaths.
    if let Commands::Completion { shell } = &cli.command {
        commands::completion::run(*shell);
        return Ok(());
    }
    if let Commands::Man = &cli.command {
        return commands::man::run();
    }

    let paths = ConfigPaths::new()?;

    match cli.command {
        Commands::Init {
            bd,
            adv,
            nix,
            nix_init,
        } => {
            commands::init::run(&paths, bd, adv, nix, nix_init)?;
        }
        Commands::Install { package } => {
            commands::simple::install(&package, &paths)?;
        }
        Commands::Remove { package } => {
            commands::simple::remove(&package, &paths)?;
        }
        Commands::Status => {
            commands::status::run(&paths, cli.json)?;
        }
        Commands::Diff => {
            commands::diff::run(&paths)?;
        }
        Commands::Module { action } => match action {
            ModuleAction::List => {
                commands::module::list(&paths, cli.json)?;
            }
            ModuleAction::Enable {
                module_names,
                skip_sync,
            } => {
                if module_names.is_empty() {
                    commands::module::enable_interactive(&paths, skip_sync)?;
                } else {
                    commands::module::enable(&paths, &module_names, cli.json, skip_sync)?;
                }
            }
            ModuleAction::Disable { name } => {
                if let Some(name) = name {
                    commands::module::disable(&paths, &name, cli.json)?;
                } else {
                    commands::module::disable_interactive(&paths)?;
                }
            }
            ModuleAction::RunHook { name } => {
                if let Some(name) = name {
                    commands::module::run_hook(&paths, &name)?;
                } else {
                    commands::module::run_hook_interactive(&paths)?;
                }
            }
            ModuleAction::Create { path, lua, nix } => {
                commands::module::create(&paths, &path, lua, nix)?;
            }
        },
        Commands::Service { action } => match action {
            ServiceAction::List => {
                commands::service::list(&paths, cli.json)?;
            }
            ServiceAction::Enable { name } => {
                if let Some(name) = name {
                    commands::service::enable(&paths, &name, cli.json)?;
                } else {
                    commands::service::enable_interactive(&paths)?;
                }
            }
            ServiceAction::Disable { name } => {
                if let Some(name) = name {
                    commands::service::disable(&paths, &name, cli.json)?;
                } else {
                    commands::service::disable_interactive(&paths)?;
                }
            }
            ServiceAction::Show { name } => {
                commands::service::show(&paths, &name)?;
            }
        },
        Commands::Secrets { action } => match action {
            SecretsAction::Status => {
                commands::secrets::status(&paths, cli.json)?;
            }
            SecretsAction::Sync { dry_run, prune } => {
                commands::secrets::sync(&paths, dry_run, prune, cli.json)?;
            }
            SecretsAction::List => {
                commands::secrets::list(&paths, cli.json)?;
            }
            SecretsAction::Edit { name } => {
                commands::secrets::edit(&paths, &name)?;
            }
            SecretsAction::Keygen => {
                commands::secrets::keygen(&paths)?;
            }
        },
        Commands::Update {
            no_backup,
            no_hooks,
            devel,
        } => {
            commands::update::run(&paths, no_backup, no_hooks, devel)?;
        }
        Commands::Merge {
            dry_run,
            services,
            user,
            defaults,
            include_deps,
        } => {
            commands::merge::run(&paths, dry_run, services, user, defaults, include_deps)?;
        }
        Commands::Migrate { dry_run } => {
            commands::migrate::run(&paths, dry_run)?;
        }
        Commands::Find { package } => {
            commands::find::run(&paths, &package, cli.json)?;
        }
        Commands::Forget { package } => {
            commands::forget::run(&paths, &package)?;
        }
        Commands::Sync {
            dry_run,
            prune,
            force,
            no_backup,
            no_hooks,
            force_dotfiles,
            auto_commit,
        } => {
            commands::sync::run(
                &paths,
                dry_run,
                prune,
                force,
                no_backup,
                no_hooks,
                force_dotfiles,
                cli.json,
                auto_commit,
            )?;
        }
        Commands::Validate { check_packages } => {
            commands::validate::run(&paths, check_packages, cli.json)?;
        }
        Commands::SaveConfig => {
            commands::config_backup::save_config(&paths, "manual", cli.json)?;
        }
        Commands::RestoreConfig { backup } => {
            commands::config_backup::restore_config(&paths, backup, cli.json)?;
        }
        Commands::Hooks { action } => match action {
            HooksAction::List => {
                commands::hooks::list(&paths, cli.json)?;
            }
            HooksAction::Reset {
                module,
                pre,
                disable,
            } => {
                commands::hooks::reset(&paths, &module, pre, disable)?;
            }
            HooksAction::Skip {
                module,
                pre,
                disable,
            } => {
                commands::hooks::skip(&paths, &module, pre, disable)?;
            }
            HooksAction::Run {
                module,
                pre,
                disable,
            } => {
                commands::hooks::run(&paths, &module, pre, disable)?;
            }
        },
        Commands::Repo { action } => match action {
            RepoAction::Init => {
                commands::repo::init(&paths)?;
            }
            RepoAction::Clone => {
                commands::repo::clone(&paths)?;
            }
            RepoAction::Push => {
                commands::repo::push(&paths)?;
            }
            RepoAction::Pull => {
                commands::repo::pull(&paths)?;
            }
            RepoAction::Status => {
                commands::repo::status(&paths)?;
            }
        },
        Commands::Backup { action } => match action {
            Some(BackupAction::List) => {
                commands::backup::list(&paths)?;
            }
            Some(BackupAction::Delete { snapshot }) => {
                commands::backup::delete(&paths, snapshot)?;
            }
            Some(BackupAction::Check) => {
                commands::backup::check_config(&paths)?;
            }
            None => {
                commands::backup::create(&paths)?;
            }
        },
        Commands::Restore { snapshot } => {
            commands::backup::restore(&paths, snapshot)?;
        }
        Commands::SelfUpdate => {
            commands::selfupdate::run()?;
        }
        Commands::Nix { action } => match action {
            NixAction::Install => {
                commands::nix::install(&paths)?;
            }
            NixAction::Switch => {
                commands::nix::switch(&paths)?;
            }
            NixAction::Update => {
                commands::nix::update(&paths)?;
            }
            NixAction::Search { query } => {
                commands::nix::search(&query)?;
            }
            NixAction::Status => {
                commands::nix::status(&paths, cli.json)?;
            }
        },
        Commands::Search => {
            commands::search::run(&paths)?;
        }
        Commands::Edit => {
            commands::edit::run(&paths)?;
        }
        Commands::Tui => {
            let terminal = tui::terminal::init()?;
            let result = tui::run(paths, terminal);
            tui::terminal::restore()?;
            result?;
        }
        Commands::Generate { action } => match action {
            GenerateAction::Hardware { force } => {
                commands::generate::hardware(&paths, force)?;
            }
            GenerateAction::Service { force } => {
                commands::generate::service(&paths, force)?;
            }
            GenerateAction::Storage { force } => {
                commands::generate::storage(&paths, force)?;
            }
            GenerateAction::Util { force } => {
                commands::generate::util(&paths, force)?;
            }
        },
        Commands::Source { action } => match action {
            SourceAction::List => {
                commands::source::list(&paths)?;
            }
            SourceAction::Build { name } => {
                commands::source::build(&paths, name.as_deref())?;
            }
            SourceAction::Rebuild { name } => {
                commands::source::rebuild(&paths, name.as_deref())?;
            }
            SourceAction::Remove { name } => {
                commands::source::remove(&name)?;
            }
            SourceAction::Status => {
                commands::source::status(&paths)?;
            }
        },
        Commands::Completion { shell } => {
            commands::completion::run(shell);
        }
        Commands::Man => {
            commands::man::run()?;
        }
        Commands::Doctor => {
            let exit_code = commands::doctor::run(&paths)?;
            if exit_code != 0 {
                std::process::exit(exit_code);
            }
        }
    }

    Ok(())
}
