//! Reads margo's live keyboard configuration so mkeys' virtual keyboard
//! submits, and its legends are derived from, the REAL active layout —
//! not a bundled `en`/`tr` TOML guess or (worse) a hardcoded `"us"`
//! keymap.

use xkbcommon::xkb;

/// Compile the keymap margo's compositor is actually configured to use,
/// from `~/.config/margo/config.conf`'s `xkb_rules_*` keys — the same
/// RMLVO fields and `Keymap::new_from_names` call margo itself makes
/// (`margo/src/state.rs::reload_config`), so the result matches what the
/// compositor's seat resolves. `None` on any failure (no config file,
/// bad RMLVO, …); callers fall back to a sane default.
pub fn compiled_keymap() -> Option<xkb::Keymap> {
    let cfg = margo_config::parse_config(None).ok()?;
    let ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    let options = (!cfg.xkb_rules.options.is_empty()).then(|| cfg.xkb_rules.options.clone());
    xkb::Keymap::new_from_names(
        &ctx,
        &cfg.xkb_rules.rules,
        &cfg.xkb_rules.model,
        &cfg.xkb_rules.layout,
        &cfg.xkb_rules.variant,
        options,
        xkb::KEYMAP_COMPILE_NO_FLAGS,
    )
}

/// The layout GROUP index currently active on margo's seat, resolved by
/// matching `mctl get keyboard-layout`'s name against `keymap`'s own
/// layout names. Multi-layout setups (`xkb_rules_layout = "us,tr"`)
/// switch which group is active via a keybind margo tracks and we don't,
/// so we ask it. Falls back to group 0 when `mctl` is unavailable or
/// reports a name the keymap doesn't recognise — always correct for the
/// common single-layout case.
pub fn active_layout_group(keymap: &xkb::Keymap) -> u32 {
    let active_name = std::process::Command::new("mctl")
        .args(["get", "keyboard-layout"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| serde_json::from_slice::<serde_json::Value>(&o.stdout).ok())
        .and_then(|v| v.get("keyboard_layout")?.as_str().map(str::to_string));

    let Some(active_name) = active_name else {
        return 0;
    };
    (0..keymap.num_layouts())
        .find(|&i| keymap.layout_get_name(i) == active_name)
        .unwrap_or(0)
}
