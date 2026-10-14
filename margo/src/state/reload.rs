//! Config reload on `MargoState`: explicit (`mctl reload`) and
//! watcher-driven (`auto_reload_config`) reloads, and the config-file
//! watcher's lifecycle. Extracted from `state.rs`.

use super::*;

impl MargoState {
    /// Explicit reload (`mctl reload`, the reload keybind).
    pub fn reload_config(&mut self) -> Result<()> {
        self.reload_config_inner(false)
    }

    /// Reload requested by the `auto_reload_config` watcher. Skipped when
    /// the watched files' content is unchanged since the last applied
    /// reload — e.g. Settings wrote a fragment *and* ran `mctl reload`,
    /// so the watcher's follow-up event would only repeat that reload.
    pub fn reload_config_on_change(&mut self) -> Result<()> {
        if self.config_fingerprint == Some(config_fingerprint(&self.config_sources)) {
            tracing::debug!("config auto-reload: content unchanged, skipping");
            return Ok(());
        }
        self.reload_config_inner(true)
    }

    fn reload_config_inner(&mut self, auto: bool) -> Result<()> {
        // Validate first. The parser is permissive (silent defaults
        // on malformed values), so a "successful" parse can still mean
        // the user's intent was misread. Run the structured validator
        // and bail before swapping config if it found errors —
        // compositor stays on the previous good config.
        match margo_config::validator::validate_config(self.config_path.as_deref()) {
            Ok(report) => {
                self.last_reload_diagnostics = report.diagnostics.clone();
                if report.has_errors() {
                    // Trigger the C2 on-screen banner. 10 s ought to be
                    // long enough to read "your config is broken, run
                    // mctl check-config" without being a pest.
                    self.config_error_overlay_until =
                        Some(std::time::Instant::now() + std::time::Duration::from_secs(10));
                    self.request_repaint();
                    let err_count = report.errors().count();
                    return Err(anyhow::anyhow!(
                        "config has {err_count} error(s) — run `mctl check-config` for details"
                    ));
                }
            }
            Err(e) => {
                tracing::warn!(error = ?e, "config validator could not read file");
                // Fall through: let parse_config produce the canonical
                // error so the caller's message says "I/O failure"
                // rather than "validator missing".
            }
        }

        let (new_config, sources) =
            margo_config::parse_config_with_defaults_and_sources(self.config_path.as_deref())
                .with_context(|| "reload margo config")?;
        self.config_fingerprint = Some(config_fingerprint(&sources));
        self.config_sources = sources;

        // Save this generation now that it's confirmed to parse (the
        // validator above already confirmed it has no errors). Mirrors
        // the boot-path save in main.rs — see
        // docs/superpowers/specs/2026-09-01-config-generations-rollback-design.md.
        if let Some(path) = self
            .config_path
            .as_deref()
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var("HOME")
                    .ok()
                    .map(|h| std::path::PathBuf::from(h).join(".config/margo/config.conf"))
            })
            && let Ok(raw) = std::fs::read_to_string(&path)
        {
            if auto {
                let _ = generations::save_coalescing(&raw, 20, GENERATION_COALESCE_WINDOW);
            } else {
                let _ = generations::save(&raw, 20);
            }
        }

        // Successful reload — clear any stale diagnostics + overlay
        // (warnings from the validation pass above are still in
        // last_reload_diagnostics, intentionally; the user can still
        // query them via mctl config-errors).
        self.config_error_overlay_until = None;
        // A reload re-parses the rulebook from scratch — a `windowrule-once`
        // the user is actively iterating on should be able to fire again.
        self.rule_once_consumed.clear();

        if let Some(keyboard) = self.seat.get_keyboard() {
            let xkb_options = new_config.xkb_rules.options_or_none();
            keyboard
                .set_xkb_config(
                    self,
                    smithay::input::keyboard::XkbConfig {
                        rules: &new_config.xkb_rules.rules,
                        model: &new_config.xkb_rules.model,
                        layout: &new_config.xkb_rules.layout,
                        variant: &new_config.xkb_rules.variant,
                        options: xkb_options,
                    },
                )
                .map_err(|e| anyhow::anyhow!("reload xkb config: {e:?}"))?;
            keyboard.change_repeat_info(new_config.repeat_rate, new_config.repeat_delay);
        }

        self.input_keyboard.repeat_rate = new_config.repeat_rate;
        self.input_keyboard.repeat_delay = new_config.repeat_delay;

        for device in &mut self.libinput_devices {
            crate::libinput_config::apply_to_device(device, &new_config);
        }

        for mon in &mut self.monitors {
            mon.gappih = new_config.gappih as i32;
            mon.gappiv = new_config.gappiv as i32;
            mon.gappoh = new_config.gappoh as i32;
            mon.gappov = new_config.gappov as i32;
        }

        self.animation_curves = AnimationCurves::bake(&new_config);
        self.enable_gaps = new_config.enable_gaps;
        DISABLE_MIDDLE_PASTE.store(
            new_config.disable_middle_paste,
            std::sync::atomic::Ordering::Relaxed,
        );
        // Re-decode the overview backdrop image if the configured path
        // changed (incl. cleared → `None`), so Settings → Overview
        // applies live. Only re-reads the file when the path actually
        // differs — avoids a needless decode on every `mctl reload`.
        if new_config.overview_backdrop_image != self.config.overview_backdrop_image {
            self.overview_backdrop = new_config
                .overview_backdrop_image
                .as_deref()
                .and_then(crate::wallpaper::WallpaperState::load_exact);
        }
        // `client.border_width` is seeded from `config.borderpx` at map time
        // (see `state/data.rs`), so a bare config swap would leave every
        // already-open window on the OLD border width — `borderpx` would only
        // affect windows opened after the reload. Re-apply the new global to
        // every client that was tracking it (i.e. whose width still equals the
        // old global), leaving per-window `windowrule` border overrides alone.
        let old_borderpx = self.config.borderpx;
        let new_borderpx = new_config.borderpx;
        // `wp_color_management_v1` (HDR opt-in): flip the global live
        // instead of requiring a re-login — see
        // `ColorManagementState::set_enabled`.
        if new_config.color_management != self.config.color_management {
            let display_handle = self.display_handle.clone();
            self.color_management_state
                .set_enabled::<Self>(&display_handle, new_config.color_management);
        }
        self.config = new_config;
        self.title_rules_exist = config_has_title_rules(&self.config);

        // Re-apply the file-logging knobs live so `mctl config reload`
        // (and Settings → Logging via the compositor `.conf`) takes effect
        // without a restart. `keep_sessions` only matters at the next start.
        if let Some(h) = crate::LOG_HANDLE.get() {
            let _ = h.set_enabled(self.config.log_to_file);
            let _ = h.set_level(&self.config.log_file_level);
        }

        if new_borderpx != old_borderpx {
            for client in self.clients.iter_mut() {
                if client.border_width == old_borderpx {
                    client.border_width = new_borderpx;
                }
            }
        }
        // Per-tag tiling layouts: re-assert the configured `taglayout`
        // directives on reload so Settings → Tiling Layout "Apply"
        // (which writes the config + runs `mctl reload`) takes effect
        // live. `seed_taglayouts` only touches tags that have an
        // explicit directive, so unconfigured "(default)" tags keep
        // whatever the running session set. `current_layout()` reads
        // `ltidxs[curtag]` directly, so the `arrange_all()` below
        // re-tiles the visible tag immediately.
        let taglayouts = self.config.taglayouts.clone();
        if !taglayouts.is_empty() {
            for mon in &mut self.monitors {
                mon.pertag.seed_taglayouts(&taglayouts);
            }
        }
        // Apply the global `default_layout` to every tag that has neither an
        // explicit `taglayout` override nor a live user-picked layout, so
        // changing "Default layout" in Settings → Tiling Layout takes effect
        // on reload (not only at the next start). user_picked tags (a manual
        // `setlayout`) are left alone — the session keeps them. Runs before
        // `apply_tag_rules_to_monitor`, so a per-tag `tagrule` layout still
        // wins over the global default (precedence: taglayout > tagrule >
        // default).
        if let Some(def) = LayoutId::from_name(&self.config.default_layout) {
            let override_tags: std::collections::HashSet<usize> =
                taglayouts.iter().map(|(t, _)| *t as usize).collect();
            for mon in &mut self.monitors {
                for tag in 1..mon.pertag.ltidxs.len() {
                    let picked = mon
                        .pertag
                        .user_picked_layout
                        .get(tag)
                        .copied()
                        .unwrap_or(false);
                    if !override_tags.contains(&tag) && !picked {
                        mon.pertag.ltidxs[tag] = def;
                    }
                }
            }
        }
        // Reload re-establishes "what the file says" — invalidate the
        // theme baseline so a subsequent `mctl theme default` resets
        // to the freshly-parsed values.
        self.theme_baseline = None;
        for idx in 0..self.clients.len() {
            self.reapply_rules(idx, WindowRuleReason::Reload);
        }
        for mon_idx in 0..self.monitors.len() {
            self.apply_tag_rules_to_monitor(mon_idx);
        }
        self.arrange_all();
        self.mark_state_dirty();
        self.request_repaint();
        // Config swap may have flipped twilight on/off or changed
        // day/night temps — force a resample so the new values
        // take effect immediately instead of waiting for the next
        // tick. Drop the schedule cache first so edited presets /
        // a changed schedule dir are re-read on this resample.
        self.twilight_schedule_cache = None;
        self.tick_twilight();
        // `auto_reload_config` itself, or the source/include tree, may
        // have just changed — start/stop/re-sync the watcher to match.
        self.sync_config_watcher();
        self.arm_cursor_hide_timer();
        tracing::info!("config reloaded");
        Ok(())
    }

    /// Start/stop/re-sync the config auto-reload watcher
    /// (`auto_reload_config`, default on) to match the live config.
    /// Called once right after startup (from `main.rs`, once
    /// `set_config_reload_sender` has wired up where its signal goes)
    /// and again at the end of every [`reload_config`](Self::reload_config).
    pub fn sync_config_watcher(&mut self) {
        if !self.config.auto_reload_config {
            self.config_watcher = None;
            return;
        }
        let Some(tx) = self.config_reload_tx.clone() else {
            // Startup ordering: called once before the sender is wired
            // up (no-op) and once right after (does the real work) —
            // see `main.rs`. Never hit from `reload_config` itself,
            // since that can't run before startup finishes.
            return;
        };
        if self.config_sources.is_empty() {
            // Startup: the boot parse in main.rs doesn't report its
            // file list, so walk the tree once here.
            self.config_sources =
                margo_config::config_source_files(self.config_path.as_deref()).unwrap_or_default();
            self.config_fingerprint = Some(config_fingerprint(&self.config_sources));
        }
        let paths =
            crate::config_watcher::watch_list(self.config_path.as_deref(), &self.config_sources);
        match &mut self.config_watcher {
            Some(watcher) => watcher.update(&paths),
            None => {
                self.config_watcher =
                    crate::config_watcher::ConfigWatcher::new(&paths, move || {
                        let _ = tx.send(());
                    });
            }
        }
    }

    /// Wire the signal `sync_config_watcher` sends on a settled
    /// filesystem change into `reload_tx` — a `calloop::channel`
    /// whose receiver end `main.rs` has already inserted into the
    /// event loop, calling `reload_config` on the compositor's own
    /// thread when a message arrives. Called once, right after the
    /// event loop (and therefore the channel) exists.
    pub fn set_config_reload_sender(&mut self, reload_tx: calloop::channel::Sender<()>) {
        self.config_reload_tx = Some(reload_tx);
        self.sync_config_watcher();
    }
}

/// Auto-reload saves landing within this window of the previous
/// auto-reload generation replace it instead of adding a new one.
const GENERATION_COALESCE_WINDOW: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// Content hash of the config file tree; a missing file hashes as a
/// distinct marker so creating a `source`d file still counts as a change.
fn config_fingerprint(paths: &[std::path::PathBuf]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for path in paths {
        path.hash(&mut hasher);
        std::fs::read(path).ok().hash(&mut hasher);
    }
    hasher.finish()
}
