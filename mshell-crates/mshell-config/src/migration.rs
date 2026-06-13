//! Profile schema versioning + stepped migration.
//!
//! `config_version` is a **file-format meta key**, not a runtime-config field:
//! it lives at the top of each `~/.config/margo/mshell/profiles/<name>.yaml`
//! but is deliberately absent from the [`Config`](crate::schema::config::Config)
//! struct (serde ignores the extra key on read). That keeps the reactive
//! `Store`/`Patch` derives on `Config` untouched while still letting us reshape
//! the on-disk format safely.
//!
//! ## Why this exists (the serde-default trap)
//!
//! `#[serde(default)]` only protects *parsing* — a missing field falls back to
//! a default, it does not let us *change* an existing profile's value or
//! rename/reshape a key. When the correct value for an existing user differs
//! from the default, that is a **migration**, not a default (see
//! `mshell-frame/DESIGN.md` §9). This module is the one place those one-shot,
//! ordered transforms live.
//!
//! ## How to add a migration
//!
//! 1. Bump [`CONFIG_VERSION`].
//! 2. Add a `N => migrate_vN_to_vN1(doc)` arm to [`apply_step`] that mutates the
//!    raw `serde_yaml::Value` (rename a key, fill an intended value, drop a dead
//!    block). Keep it idempotent-friendly: it only ever runs once per profile.
//! 3. Add a round-trip test below (old-shape fixture → [`migrate_yaml`] → assert
//!    the new shape + that re-running is a no-op).
//!
//! On load, [`migrate_yaml`] runs as a pre-pass before figment reads the file;
//! if it changed anything the migrated YAML is written back atomically, so the
//! upgrade is applied exactly once. On save, [`stamp_version`] keeps the current
//! version pinned at the top of the serialized profile.

use serde_yaml::{Mapping, Value};

/// The current on-disk profile format version. A profile file with a lower (or
/// absent → treated as 0) `config_version` is migrated up to this on load.
pub const CONFIG_VERSION: u32 = 1;

/// The YAML key carrying the format version.
const VERSION_KEY: &str = "config_version";

/// Outcome of [`migrate_yaml`].
#[derive(Debug, Clone)]
pub struct Migrated {
    /// The migrated YAML, stamped with the current [`CONFIG_VERSION`].
    pub yaml: String,
    /// The version the input declared (absent → 0).
    pub from: u32,
    /// Whether migration changed anything (false = already current + no
    /// transform applied → caller can skip the write-back).
    pub changed: bool,
}

/// Read the `config_version` from a parsed profile document (absent → 0).
fn read_version(doc: &Value) -> u32 {
    doc.get(VERSION_KEY)
        .and_then(Value::as_u64)
        .map(|v| v as u32)
        .unwrap_or(0)
}

/// Set `config_version` at the top of a mapping document.
fn set_version(map: &mut Mapping, version: u32) {
    map.insert(
        Value::String(VERSION_KEY.to_string()),
        Value::Number(version.into()),
    );
}

/// Apply the single migration step that takes a profile from `from` to
/// `from + 1`, mutating the raw document in place.
///
/// v0 → v1 is the versioning baseline: profiles predating `config_version` are
/// stamped to v1, the format is otherwise unchanged, so there is no field
/// transform here yet. The first real reshape adds its branch — e.g.
/// `if from == 1 { migrate_v1_to_v2(doc); }` — and bumps [`CONFIG_VERSION`].
fn apply_step(from: u32, doc: &mut Mapping) {
    let _ = (from, doc);
}

/// Migrate a profile YAML string up to [`CONFIG_VERSION`], applying each step in
/// order. Returns the stamped YAML and whether anything changed.
///
/// A document that is not a YAML mapping (empty / malformed-but-parseable) is
/// returned stamped but otherwise untouched — figment will fall back to
/// defaults for it downstream.
pub fn migrate_yaml(yaml: &str) -> Result<Migrated, serde_yaml::Error> {
    let mut doc: Value = serde_yaml::from_str(yaml)?;
    let from = read_version(&doc);

    // An empty document parses as Null; treat it as an empty mapping so we can
    // still stamp a version onto it.
    if doc.is_null() {
        doc = Value::Mapping(Mapping::new());
    }

    let Value::Mapping(map) = &mut doc else {
        // Non-mapping top level (a bare scalar/sequence): nothing to migrate.
        return Ok(Migrated {
            yaml: yaml.to_string(),
            from,
            changed: false,
        });
    };

    // Apply each step from the declared version up to current.
    let target = CONFIG_VERSION;
    for v in from..target {
        apply_step(v, map);
    }
    set_version(map, target);

    let out = serde_yaml::to_string(&doc)?;
    // `changed` is true if the version moved OR a step rewrote the body. We
    // approximate "a step rewrote the body" by comparing against a stamp-only
    // pass of the original: if migrating and merely stamping differ, a step
    // touched content.
    let changed = from != target || {
        let mut stamped_only: Value = serde_yaml::from_str(yaml)?;
        if stamped_only.is_null() {
            stamped_only = Value::Mapping(Mapping::new());
        }
        if let Value::Mapping(m) = &mut stamped_only {
            set_version(m, from.max(target));
        }
        serde_yaml::to_string(&stamped_only)? != out
    };

    Ok(Migrated {
        yaml: out,
        from,
        changed,
    })
}

/// Pin `config_version: CONFIG_VERSION` at the top of an already-serialized
/// profile YAML (used on save, since the `Config` struct itself does not carry
/// the key). A non-mapping document is returned unchanged.
pub fn stamp_version(yaml: &str) -> String {
    let Ok(mut doc) = serde_yaml::from_str::<Value>(yaml) else {
        return yaml.to_string();
    };
    let Value::Mapping(map) = &mut doc else {
        return yaml.to_string();
    };
    set_version(map, CONFIG_VERSION);
    serde_yaml::to_string(&doc).unwrap_or_else(|_| yaml.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Strip the version key so we can compare bodies independent of stamping.
    fn body_without_version(yaml: &str) -> Value {
        let mut v: Value = serde_yaml::from_str(yaml).unwrap();
        if let Value::Mapping(m) = &mut v {
            m.remove(Value::String(VERSION_KEY.to_string()));
        }
        v
    }

    #[test]
    fn unversioned_profile_is_stamped_and_body_preserved() {
        let old = "\
clipboard:
  max_entries: 42
idle:
  dim_enabled: true
";
        let m = migrate_yaml(old).unwrap();
        assert_eq!(m.from, 0, "absent version reads as 0");
        assert!(m.changed, "stamping a v0 profile is a change");

        let out: Value = serde_yaml::from_str(&m.yaml).unwrap();
        assert_eq!(
            read_version(&out),
            CONFIG_VERSION,
            "migrated profile carries the current version"
        );
        // The v1 baseline applies no field transform, so every original key
        // survives untouched.
        assert_eq!(
            body_without_version(&m.yaml),
            body_without_version(old),
            "v1 baseline preserves the whole body"
        );
    }

    #[test]
    fn current_version_profile_is_unchanged() {
        let current = format!("config_version: {CONFIG_VERSION}\nclipboard:\n  max_entries: 7\n");
        let m = migrate_yaml(&current).unwrap();
        assert_eq!(m.from, CONFIG_VERSION);
        assert!(!m.changed, "a current profile needs no migration");
    }

    #[test]
    fn migration_is_idempotent() {
        let old = "idle:\n  lock_enabled: false\n";
        let once = migrate_yaml(old).unwrap();
        let twice = migrate_yaml(&once.yaml).unwrap();
        assert!(
            !twice.changed,
            "re-migrating an already-migrated profile is a no-op"
        );
        assert_eq!(once.yaml, twice.yaml, "migration reaches a fixed point");
    }

    #[test]
    fn stamp_adds_version_to_versionless_yaml() {
        let yaml = "general:\n  foo: 1\n";
        let stamped = stamp_version(yaml);
        let v: Value = serde_yaml::from_str(&stamped).unwrap();
        assert_eq!(read_version(&v), CONFIG_VERSION);
    }

    #[test]
    fn empty_document_gets_a_version() {
        let m = migrate_yaml("").unwrap();
        let v: Value = serde_yaml::from_str(&m.yaml).unwrap();
        assert_eq!(read_version(&v), CONFIG_VERSION);
    }

    #[test]
    fn non_mapping_document_is_left_alone() {
        let m = migrate_yaml("- a\n- b\n").unwrap();
        assert!(!m.changed);
        assert_eq!(m.yaml, "- a\n- b\n");
    }

    #[test]
    fn bundled_profiles_migrate_clean_and_still_parse_as_config() {
        for name in ["default", "margo"] {
            let yaml = crate::config_utils::bundled_profile_yaml(name)
                .unwrap_or_else(|| panic!("bundled profile {name} missing"));
            let m = migrate_yaml(yaml).expect("bundled profile migrates");
            let v: Value = serde_yaml::from_str(&m.yaml).unwrap();
            assert_eq!(
                read_version(&v),
                CONFIG_VERSION,
                "bundled {name} ends at current version"
            );
            // The migrated YAML must still deserialize into the live Config
            // type — the strongest "didn't break the shape" guarantee.
            serde_yaml::from_str::<crate::schema::config::Config>(&m.yaml)
                .unwrap_or_else(|e| panic!("migrated bundled {name} no longer parses: {e}"));
        }
    }
}
