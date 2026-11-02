use super::*;
use crate::package::Package;
use std::path::PathBuf;

#[test]
fn test_partition_protected_separates_critical_packages() {
    let (removable, protected) = partition_protected(vec![
        "firefox".to_string(),
        "linux".to_string(),
        "steam".to_string(),
        "glibc".to_string(),
        "pacman".to_string(),
    ]);
    // Ordinary packages stay removable, in order.
    assert_eq!(removable, vec!["firefox".to_string(), "steam".to_string()]);
    // Critical system packages are pulled out and never removed.
    let mut p = protected;
    p.sort();
    assert_eq!(
        p,
        vec![
            "glibc".to_string(),
            "linux".to_string(),
            "pacman".to_string()
        ]
    );
}

#[test]
fn test_sync_marker_roundtrip_signals_interruption() {
    let tmp = tempfile::tempdir().unwrap();
    let state_dir = tmp.path().join("state");

    // A fresh checkout has never been interrupted.
    assert!(!sync_was_interrupted(&state_dir));

    // Beginning a sync drops a marker (creating the state dir if needed).
    begin_sync_marker(&state_dir).unwrap();
    assert!(
        sync_was_interrupted(&state_dir),
        "an in-progress sync must be detectable"
    );

    // A clean completion clears it, so the next run does not warn.
    clear_sync_marker(&state_dir);
    assert!(
        !sync_was_interrupted(&state_dir),
        "a completed sync must clear the marker"
    );
}

#[test]
fn test_clear_sync_marker_is_idempotent_when_absent() {
    let tmp = tempfile::tempdir().unwrap();
    let state_dir = tmp.path().join("state");
    // Clearing a marker that was never written must be a harmless no-op.
    clear_sync_marker(&state_dir);
    assert!(!sync_was_interrupted(&state_dir));
}

fn sp(name: &str, ty: Option<&str>) -> StatePackage {
    StatePackage {
        name: name.to_string(),
        version: None,
        pkg_type: ty.map(|s| s.to_string()),
    }
}

#[test]
fn test_compute_prunable_selects_undeclared_but_installed() {
    let state = vec![
        sp("vim", None),                  // declared → keep
        sp("htop", Some("native")),       // undeclared + installed → prune
        sp("gone", None),                 // undeclared + NOT installed → skip
        sp("org.x.App", Some("flatpak")), // undeclared flatpak + installed → prune
    ];
    let declared: HashSet<String> = ["vim".to_string()].into_iter().collect();
    let installed_native: HashMap<String, String> = [("htop".to_string(), "1".to_string())]
        .into_iter()
        .collect();
    let installed_flatpak: HashSet<String> = ["org.x.App".to_string()].into_iter().collect();

    let (native, flatpak) =
        compute_prunable(&state, &declared, &installed_native, &installed_flatpak);
    assert_eq!(native, vec!["htop".to_string()]);
    assert_eq!(flatpak, vec!["org.x.App".to_string()]);
}

#[test]
fn test_compute_prunable_treats_unknown_type_as_native() {
    let state = vec![sp("mystery", Some("weird"))];
    let installed_native: HashMap<String, String> = [("mystery".to_string(), "1".to_string())]
        .into_iter()
        .collect();
    let (native, flatpak) =
        compute_prunable(&state, &HashSet::new(), &installed_native, &HashSet::new());
    assert_eq!(native, vec!["mystery".to_string()]);
    assert!(flatpak.is_empty());
}

#[test]
fn test_prune_set_filtered_excludes_protected_and_keeps_declared() {
    // Mixed state: a declared pkg (keep), a normal undeclared+installed pkg
    // (prune), a PROTECTED undeclared+installed pkg (must be filtered out),
    // and an undeclared+installed flatpak (prune).
    let state = vec![
        sp("vim", Some("native")),        // declared → keep
        sp("htop", Some("native")),       // undeclared + installed → prune
        sp("systemd", Some("native")),    // protected → never removed
        sp("org.x.App", Some("flatpak")), // undeclared flatpak + installed → prune
    ];
    let declared: HashSet<String> = ["vim".to_string()].into_iter().collect();
    let installed_native: HashMap<String, String> = [
        ("htop".to_string(), "1".to_string()),
        ("systemd".to_string(), "255".to_string()),
    ]
    .into_iter()
    .collect();
    let installed_flatpak: HashSet<String> = ["org.x.App".to_string()].into_iter().collect();

    let (native, flatpak) =
        prune_set_filtered(&state, &declared, &installed_native, &installed_flatpak);
    // "systemd" is protected and must not appear; "htop" must.
    assert_eq!(native, vec!["htop".to_string()]);
    assert!(
        !native.contains(&"systemd".to_string()),
        "protected packages must never be in the prune set"
    );
    assert_eq!(flatpak, vec!["org.x.App".to_string()]);
}

#[test]
fn test_compute_prunable_keeps_declared_packages() {
    let state = vec![sp("keep", None)];
    let declared: HashSet<String> = ["keep".to_string()].into_iter().collect();
    let installed_native: HashMap<String, String> = [("keep".to_string(), "1".to_string())]
        .into_iter()
        .collect();
    let (native, flatpak) = compute_prunable(&state, &declared, &installed_native, &HashSet::new());
    assert!(native.is_empty(), "declared package must never be pruned");
    assert!(flatpak.is_empty());
}

fn pkg(name: &str, ty: PackageType) -> Package {
    Package {
        name: name.to_string(),
        package_type: ty,
        remote: None,
    }
}

#[test]
fn test_compute_installable_selects_only_missing_packages() {
    let declared = vec![
        pkg("vim", PackageType::Native),      // installed → skip
        pkg("htop", PackageType::Native),     // missing → install
        pkg("org.x.A", PackageType::Flatpak), // installed → skip
        pkg("org.y.B", PackageType::Flatpak), // missing → install
    ];
    let installed_native: HashMap<String, String> =
        [("vim".to_string(), "1".to_string())].into_iter().collect();
    let installed_flatpak: HashSet<String> = ["org.x.A".to_string()].into_iter().collect();

    let (native, flatpak) = compute_installable(&declared, &installed_native, &installed_flatpak);
    assert_eq!(native, vec!["htop".to_string()]);
    assert_eq!(flatpak, vec!["org.y.B".to_string()]);
}

#[test]
fn test_compute_installable_ignores_nix_packages() {
    // Nix packages are applied via home-manager, not this diff.
    let declared = vec![pkg("hello", PackageType::Nix)];
    let (native, flatpak) = compute_installable(&declared, &HashMap::new(), &HashSet::new());
    assert!(native.is_empty());
    assert!(flatpak.is_empty());
}

// --- Dry-run smoke: config-tree loading + plan computation (no pacman) ------
//
// BOUNDARY: `compute_installable` and `compute_prune_preview` are pure
// functions that take already-resolved package/installed sets. The pacman
// query (`PackageManager::get_installed_native_packages`) is invoked by
// `sync::run()` before the dry-run guard and is NOT called here. These tests
// exercise config loading (pure file I/O) and plan computation (pure logic)
// in isolation — no external binaries are required.
//
// `ConfigPaths` is built as a struct literal pointing at the tempdir, so
// there is no env-var mutation and no race across parallel tests.

fn make_paths(root: &std::path::Path) -> ConfigPaths {
    ConfigPaths {
        config_dir: root.to_path_buf(),
        config_file: root.join("config.yaml"),
        packages_dir: root.join("packages"),
        state_dir: root.join("state"),
        state_file: root.join("state/installed.yaml"),
        hooks_state_file: root.join("state/hooks-executed.yaml"),
        services_state_file: root.join("state/services-state.yaml"),
        defaults_state_file: root.join("state/defaults-state.yaml"),
        theming_state_file: root.join("state/theming-state.yaml"),
        config_backups_dir: root.join("state/config-backups"),
    }
}

#[test]
fn dry_run_plan_loads_minimal_config_tree_and_resolves_packages() {
    // Config tree:
    //   <root>/config.yaml                (full, not a pointer)
    //   <root>/modules/trivial.yaml
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    std::fs::create_dir_all(root.join("modules")).unwrap();
    std::fs::create_dir_all(root.join("state")).unwrap();

    std::fs::write(
        root.join("config.yaml"),
        b"host: test-host\nenabled_modules:\n  - trivial\npackages:\n  - vim\n",
    )
    .unwrap();
    std::fs::write(
        root.join("modules/trivial.yaml"),
        b"description: Trivial test module\npackages:\n  - git\n",
    )
    .unwrap();

    let paths = make_paths(root);

    // Config loading is pure file I/O — no external binaries.
    let config = load_config(&paths).expect("load_config must succeed");
    assert_eq!(config.host, "test-host");
    assert!(config.enabled_modules.contains(&"trivial".to_string()));

    // Declared-package resolution reads YAML — no pacman call.
    let pkg_manager = PackageManager::new(paths.clone());
    let declared = pkg_manager
        .get_declared_packages(&config)
        .expect("get_declared_packages must succeed");

    let names: Vec<&str> = declared.iter().map(|p| p.name.as_str()).collect();
    assert!(
        names.contains(&"vim"),
        "host package 'vim' must appear in declared set"
    );
    assert!(
        names.contains(&"git"),
        "module package 'git' (from trivial.yaml) must appear in declared set"
    );

    // Dry-run plan: empty installed sets simulate a fresh system.
    let (to_install, flatpak_to_install) =
        compute_installable(&declared, &HashMap::new(), &HashSet::new());

    assert!(
        to_install.contains(&"vim".to_string()),
        "vim must be in the install plan on a fresh system"
    );
    assert!(
        to_install.contains(&"git".to_string()),
        "git (from module) must be in the install plan on a fresh system"
    );
    assert!(
        flatpak_to_install.is_empty(),
        "no flatpak packages declared in minimal config"
    );
}

#[test]
fn dry_run_plan_already_installed_package_not_reinstalled() {
    // If the installed map already contains "vim", it must not appear in the
    // install plan — pure logic, no external binaries.
    let declared = vec![pkg("vim", PackageType::Native)];
    let installed: HashMap<String, String> = [("vim".to_string(), "9.1.0001-1".to_string())]
        .into_iter()
        .collect();
    let (to_install, _) = compute_installable(&declared, &installed, &HashSet::new());
    assert!(
        to_install.is_empty(),
        "installed packages must not appear in the dry-run install plan"
    );
}

#[test]
fn dry_run_plan_empty_state_means_nothing_to_prune() {
    // No state file → `compute_prune_preview` must return empty lists.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::create_dir_all(root.join("state")).unwrap();

    let paths = make_paths(root);
    let declared_names: HashSet<String> = ["vim".to_string()].into_iter().collect();
    let installed_native: HashMap<String, String> = HashMap::new();
    let installed_flatpak: HashSet<String> = HashSet::new();

    let (native, flatpak) = compute_prune_preview(
        &paths,
        &declared_names,
        &installed_native,
        &installed_flatpak,
    );
    assert!(
        native.is_empty(),
        "no state file → nothing to prune (native)"
    );
    assert!(
        flatpak.is_empty(),
        "no state file → nothing to prune (flatpak)"
    );
}

#[test]
fn prune_state_missing_file_means_nothing_tracked() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = make_paths(tmp.path());
    let state = load_prune_state(&paths).expect("a missing state file is not an error");
    assert!(state.packages.is_empty());
}

#[test]
fn prune_state_corrupt_file_is_a_hard_error() {
    // A corrupt state file used to be skipped silently (`if let Ok`), so
    // `--prune` quietly did nothing. It must refuse instead.
    let tmp = tempfile::tempdir().unwrap();
    let paths = make_paths(tmp.path());
    std::fs::create_dir_all(&paths.state_dir).unwrap();
    std::fs::write(&paths.state_file, "packages: [unterminated\n  - :::\n").unwrap();
    let err = load_prune_state(&paths).expect_err("corrupt state must not be ignored");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("refusing to prune"),
        "unexpected message: {msg}"
    );
    assert!(
        msg.contains("installed.yaml"),
        "message must name the file: {msg}"
    );
}

#[test]
fn package_state_render_roundtrips_through_loader() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = make_paths(tmp.path());
    let declared = vec![
        pkg("vim", PackageType::Native),
        pkg("org.gimp.GIMP", PackageType::Flatpak),
    ];
    let text = render_package_state(&declared, |p| {
        (p.name == "vim").then(|| "9.1-1".to_string())
    });
    write_package_state(&paths, &text).unwrap();
    assert!(!paths.state_file.with_extension("mdots-tmp").exists());
    let state = load_state_file(&paths).unwrap();
    assert_eq!(state.packages.len(), 2);
    assert_eq!(state.packages[0].name, "vim");
    assert_eq!(state.packages[0].version.as_deref(), Some("9.1-1"));
    assert_eq!(state.packages[0].pkg_type.as_deref(), Some("native"));
    assert_eq!(state.packages[1].name, "org.gimp.GIMP");
    assert_eq!(state.packages[1].version, None);
    assert_eq!(state.packages[1].pkg_type.as_deref(), Some("flatpak"));
}

#[test]
fn hook_hash_is_stable_sha256() {
    // Fixed digest: the identity must not depend on the Rust release.
    assert_eq!(
        hook_script_hash("echo hi\n"),
        "sha256:ab08508fdf5ca4da5c4995987bc41c56c048aaa5eeb046417ae4049b7d40286e"
    );
    let h = hook_script_hash("echo hi\n");
    assert!(h.starts_with("sha256:"));
    assert_eq!(h.len(), "sha256:".len() + 64);
    assert_ne!(h, hook_script_hash("echo bye\n"));
}

fn write_hook(root: &std::path::Path, body: &str) -> PathBuf {
    let p = root.join("hook.sh");
    std::fs::write(&p, body).unwrap();
    p
}

fn stored_hash(paths: &ConfigPaths, module: &str) -> Option<String> {
    let content = std::fs::read_to_string(&paths.hooks_state_file).ok()?;
    let yaml: serde_yaml::Value = serde_yaml::from_str(&content).ok()?;
    yaml.get("hooks")?
        .as_sequence()?
        .iter()
        .find(|h| h.get("module").and_then(|m| m.as_str()) == Some(module))?
        .get("script_hash")?
        .as_str()
        .map(str::to_string)
}

#[test]
fn executed_hook_records_sha256_and_is_recognised() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = make_paths(tmp.path());
    let hook = write_hook(tmp.path(), "echo once\n");
    mark_hook_executed(&paths, "m", &hook).unwrap();
    assert_eq!(
        stored_hash(&paths, "m"),
        Some(hook_script_hash("echo once\n"))
    );
    assert!(matches!(
        check_hook_status(&paths, "m", &hook).unwrap(),
        HookStatus::Executed
    ));
    std::fs::write(&hook, "echo changed\n").unwrap();
    assert!(matches!(
        check_hook_status(&paths, "m", &hook).unwrap(),
        HookStatus::Modified
    ));
}

#[test]
fn legacy_default_hasher_entry_is_accepted_once_and_migrated() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = make_paths(tmp.path());
    let hook = write_hook(tmp.path(), "echo legacy\n");
    std::fs::create_dir_all(&paths.state_dir).unwrap();
    let legacy = legacy_hook_script_hash("echo legacy\n");
    std::fs::write(
        &paths.hooks_state_file,
        format!(
            "hooks:\n- module: m\n  script: {}\n  script_hash: '{legacy}'\n  executed_at: x\n  status: executed\n- module: other\n  status: skipped\n",
            hook.display()
        ),
    )
    .unwrap();

    // An existing user's already-run hook must not re-run as root.
    assert!(matches!(
        check_hook_status(&paths, "m", &hook).unwrap(),
        HookStatus::Executed
    ));
    // ...and the entry is rewritten with the SHA-256 identity.
    assert_eq!(
        stored_hash(&paths, "m"),
        Some(hook_script_hash("echo legacy\n"))
    );
    // Other entries survive the rewrite.
    assert!(matches!(
        check_hook_status(&paths, "other", &hook).unwrap(),
        HookStatus::Skipped
    ));
    assert!(matches!(
        check_hook_status(&paths, "m", &hook).unwrap(),
        HookStatus::Executed
    ));
}

#[test]
fn legacy_entry_for_a_changed_script_is_modified() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = make_paths(tmp.path());
    let hook = write_hook(tmp.path(), "echo new\n");
    std::fs::create_dir_all(&paths.state_dir).unwrap();
    let legacy = legacy_hook_script_hash("echo old\n");
    std::fs::write(
        &paths.hooks_state_file,
        format!("hooks:\n- module: m\n  script_hash: '{legacy}'\n  status: executed\n"),
    )
    .unwrap();
    assert!(matches!(
        check_hook_status(&paths, "m", &hook).unwrap(),
        HookStatus::Modified
    ));
    assert_eq!(
        stored_hash(&paths, "m"),
        Some(legacy),
        "no migration on mismatch"
    );
}
