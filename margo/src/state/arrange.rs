//! Tiling-arrangement methods on `MargoState`.
//!
//! Extracted from `state.rs` (state.rs split): the layout-arrangement cluster
//! — `arrange_monitor` (the per-monitor tiling driver, the single largest
//! method), its `arrange_all`/`arrange_monitors` fan-out wrappers, the
//! overview pre-arrange pass, `configure_window_size`, and `enforce_z_order`.
//! Pure `MargoState` glue, no new types.

use super::*;

impl MargoState {
    pub fn arrange_all(&mut self) {
        for mon_idx in 0..self.monitors.len() {
            self.arrange_monitor(mon_idx);
        }
        self.mark_state_dirty();
        self.publish_a11y_window_list();
    }

    /// Arrange just the listed monitors. Used by `open_overview` and
    /// `close_overview` so a multi-monitor setup doesn't pay the cost
    /// of re-laying out outputs that didn't flip overview state. Skips
    /// out-of-range indices defensively — the caller is the same
    /// process that built the list, but `monitors` can shrink under us
    /// during multi-output hot-unplug and we don't want to panic mid-
    /// arrange.
    pub fn arrange_monitors(&mut self, indices: &[usize]) {
        for &idx in indices {
            if idx < self.monitors.len() {
                self.arrange_monitor(idx);
            }
        }
        self.mark_state_dirty();
        self.publish_a11y_window_list();
    }

    /// The tiled-strip position that focus-following layouts (scroller,
    /// vertical scroller, deck) should centre on.
    ///
    /// Normally it is the focused window's own slot. But when focus sits on
    /// a surface that is *not* in the tiled strip — a floating keyring /
    /// polkit dialog, a launcher layer surface, a scratchpad — the naive
    /// lookup returns `None`, and every consumer falls back to slot 0. That
    /// made scroller snap the whole strip to the *first* window the instant a
    /// transient floating dialog grabbed focus, sliding the layout out from
    /// under the window the user was working in — which is exactly why a
    /// keyring prompt that we centre over window 2 ended up sitting over
    /// window 1. Instead, hold the strip on the most-recently-focused window
    /// that is *still tiled*, read from the per-monitor MRU `focus_history`
    /// (most-recent first). Falls back to `None` (→ slot 0) only when nothing
    /// in the history is tiled, e.g. a fresh tag.
    pub(crate) fn focused_tiled_pos(
        &self,
        mon_idx: usize,
        tiled: &[usize],
        focused_idx: Option<usize>,
    ) -> Option<usize> {
        if let Some(pos) = focused_idx.and_then(|fi| tiled.iter().position(|&idx| idx == fi)) {
            return Some(pos);
        }
        let mon = self.monitors.get(mon_idx)?;
        mon.focus_history
            .iter()
            .find_map(|&id| tiled.iter().position(|&idx| self.clients[idx].id == id))
    }

    /// Auto-float / re-tile clients for the `Floating` layout. Runs at
    /// the top of every `arrange_monitor` pass (issue #1).
    ///
    /// On a tag whose layout is `Floating`, every governed tiled
    /// client is switched to floating (`floated_by_layout` marks it as
    /// ours) and given a cascaded `float_geom`; the existing
    /// float-apply pass later in `arrange_monitor` then writes that to
    /// `geom`. On any other layout, clients we previously auto-floated
    /// are returned to the tiled set. Clients the user floated by hand
    /// (`is_floating && !floated_by_layout`) are never touched.
    /// Idempotent: a second pass with no state change is a no-op.
    fn reconcile_floating_layout(&mut self, mon_idx: usize) {
        let Some(mon) = self.monitors.get(mon_idx) else {
            return;
        };
        if mon.is_overview {
            return;
        }
        let curtag = mon.pertag.curtag;
        let is_floating_layout =
            mon.pertag.ltidxs.get(curtag).copied() == Some(crate::layout::LayoutId::Floating);
        let tagset = mon.current_tagset();
        let work_area = mon.work_area;

        // Clients on this monitor, visible on the current tagset, that
        // the floating layout governs. Excludes fullscreen (the
        // fullscreen override wins above `is_floating`), scratchpad /
        // overlay (own visibility model), and not-yet-mapped / dying /
        // hidden-group-member clients.
        let governed: Vec<usize> = self
            .clients
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                c.monitor == mon_idx
                    && !c.is_initial_map_pending
                    && c.is_visible_on(mon_idx, tagset)
                    && c.fullscreen_mode == FullscreenMode::Off
                    && !c.is_in_scratchpad
                    && !c.is_named_scratchpad
                    && !c.is_overlay
                    && !c.is_minimized
                    && !c.is_killing
                    && !c.is_hidden_group_member()
            })
            .map(|(i, _)| i)
            .collect();

        if is_floating_layout {
            // A tabbed group is a tiling construct — dissolve any on
            // this tag before floating its members.
            let gids: std::collections::BTreeSet<u32> = governed
                .iter()
                .filter_map(|&i| self.clients[i].group_id)
                .collect();
            for gid in gids {
                self.dissolve_group(gid);
            }

            // Cascade slots already consumed by floating clients on
            // this tag (recomputed each pass → stable / idempotent).
            let mut cascade_index = governed
                .iter()
                .filter(|&&i| self.clients[i].is_floating && self.clients[i].float_geom.width != 0)
                .count();

            for &i in &governed {
                let needs_geom = self.clients[i].float_geom.width == 0;
                // Claim this client if it isn't floating yet (a tiled
                // client entering Floating), or if it *is* floating but
                // under Mosaic's ownership (a tag that just switched
                // Mosaic -> Floating) — either way it becomes ours. A
                // client already ours, or one the user hand-floated
                // (`is_floating && !floated_by_layout`), is left alone.
                let owned_by_other = self.clients[i].floated_by_layout
                    && self.clients[i].auto_float_owner != Some(crate::layout::LayoutId::Floating);
                if !self.clients[i].is_floating || owned_by_other {
                    self.clients[i].is_floating = true;
                    self.clients[i].floated_by_layout = true;
                    self.clients[i].auto_float_owner = Some(crate::layout::LayoutId::Floating);
                }
                if needs_geom {
                    let c = &self.clients[i];
                    let preferred = if c.geom.width > 0 && c.geom.height > 0 {
                        Some((c.geom.width, c.geom.height))
                    } else {
                        None
                    };
                    let seed = crate::layout::place_floating_cascade(
                        work_area,
                        preferred,
                        (c.min_width, c.min_height),
                        (c.max_width, c.max_height),
                        cascade_index,
                    );
                    self.clients[i].float_geom = seed;
                    cascade_index += 1;
                }
                self.clients[i].geom = self.clients[i].float_geom;
            }
        } else {
            for &i in &governed {
                if self.clients[i].floated_by_layout
                    && self.clients[i].auto_float_owner == Some(crate::layout::LayoutId::Floating)
                {
                    self.clients[i].is_floating = false;
                    self.clients[i].floated_by_layout = false;
                    self.clients[i].auto_float_owner = None;
                    // `float_geom` is left intact: switching back to
                    // Floating restores each window to its last spot.
                }
            }
        }
    }

    /// The lowest-numbered tag (1..=`MAX_TAGS`) on `mon_idx` with no client
    /// tagged onto it — a global/sticky client (`tags == u32::MAX`)
    /// occupies every tag, so it rules all of them out. `exclude` (the
    /// tag being evicted *from*) is skipped even if it would otherwise
    /// qualify; there's no point "moving" a window to the tag it's
    /// already on. `None` when every tag already has something.
    fn find_empty_tag(&self, mon_idx: usize, exclude: usize) -> Option<usize> {
        (1..=crate::layout::MAX_TAGS).find(|&tag| {
            if tag == exclude {
                return false;
            }
            let bit = 1u32 << (tag - 1);
            !self
                .clients
                .iter()
                .any(|c| c.monitor == mon_idx && (c.tags & bit) != 0)
        })
    }

    /// Auto-float / re-pack clients for the `Mosaic` layout — GNOME's
    /// content-aware self-arranging desktop
    /// (<https://blogs.gnome.org/tbernard/2023/07/26/rethinking-window-management/>).
    /// Mirrors `reconcile_floating_layout`'s shape closely (both hand
    /// governed clients to `is_floating`/`float_geom`, both leave
    /// user-hand-floated windows alone, both idempotent no-ops on a
    /// change-free pass) but hands off to a different placement algorithm
    /// ([`crate::layout::mosaic_arrange`]) and, crucially, *re-packs every
    /// governed client together on every pass* rather than only seeding
    /// brand-new ones — that's what makes existing windows move aside /
    /// shrink when a new one opens, and re-expand when one closes.
    ///
    /// `auto_float_owner` (not just `floated_by_layout`) gates every read
    /// and write here so this never fights `reconcile_floating_layout` over
    /// a client when a tag switches between the two layouts.
    fn reconcile_mosaic_layout(&mut self, mon_idx: usize) {
        let Some(mon) = self.monitors.get(mon_idx) else {
            return;
        };
        if mon.is_overview {
            return;
        }
        let curtag = mon.pertag.curtag;
        let is_mosaic_layout =
            mon.pertag.ltidxs.get(curtag).copied() == Some(crate::layout::LayoutId::Mosaic);
        let tagset = mon.current_tagset();
        let work_area = mon.work_area;
        let gaps = crate::layout::GapConfig {
            gappih: if self.enable_gaps { mon.gappih } else { 0 },
            gappiv: if self.enable_gaps { mon.gappiv } else { 0 },
            gappoh: if self.enable_gaps { mon.gappoh } else { 0 },
            gappov: if self.enable_gaps { mon.gappov } else { 0 },
        };

        // Same governed-set contract as `reconcile_floating_layout`.
        let governed: Vec<usize> = self
            .clients
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                c.monitor == mon_idx
                    && !c.is_initial_map_pending
                    && c.is_visible_on(mon_idx, tagset)
                    && c.fullscreen_mode == FullscreenMode::Off
                    && !c.is_in_scratchpad
                    && !c.is_named_scratchpad
                    && !c.is_overlay
                    && !c.is_minimized
                    && !c.is_killing
                    && !c.is_hidden_group_member()
            })
            .map(|(i, _)| i)
            .collect();

        if is_mosaic_layout {
            // A tabbed group is a tiling construct — dissolve any on this
            // tag before mosaic-floating its members (same reasoning as
            // `reconcile_floating_layout`).
            let gids: std::collections::BTreeSet<u32> = governed
                .iter()
                .filter_map(|&i| self.clients[i].group_id)
                .collect();
            for gid in gids {
                self.dissolve_group(gid);
            }

            for &i in &governed {
                // Claim this client if it isn't floating yet, or if it's
                // floating under Floating's ownership (a tag that just
                // switched Floating -> Mosaic) — see the matching comment
                // in `reconcile_floating_layout`.
                let owned_by_other = self.clients[i].floated_by_layout
                    && self.clients[i].auto_float_owner != Some(crate::layout::LayoutId::Mosaic);
                if !self.clients[i].is_floating || owned_by_other {
                    self.clients[i].is_floating = true;
                    self.clients[i].floated_by_layout = true;
                    self.clients[i].auto_float_owner = Some(crate::layout::LayoutId::Mosaic);
                }
            }

            // Pack only the clients Mosaic itself owns. A window the user
            // hand-floated before this tag ever became Mosaic
            // (`is_floating && !floated_by_layout`) keeps its own spot,
            // same as `reconcile_floating_layout`. A client mid interactive
            // move/resize is *also* excluded — otherwise this pass would
            // immediately snap it back to its packed slot on every motion
            // tick, fighting the user's own drag (see `interactive_grab`'s
            // doc comment).
            let mut packable: Vec<crate::layout::MosaicClient> = governed
                .iter()
                .copied()
                .filter(|&i| {
                    self.clients[i].floated_by_layout
                        && self.clients[i].auto_float_owner == Some(crate::layout::LayoutId::Mosaic)
                        && !self.clients[i].interactive_grab
                        && !self.clients[i].is_mosaic_stacked
                })
                .map(|i| {
                    let c = &self.clients[i];
                    crate::layout::MosaicClient {
                        index: i,
                        id: c.id,
                        // Deliberately never sourced from this client's own
                        // `geom` — that's whatever layout happened to be
                        // active on this tag a moment ago (Tile's lopsided
                        // master/stack split, Scroller's near-full-width
                        // columns, ...), so it made Mosaic's placement
                        // depend on tiling history instead of being its own
                        // consistent standard. `(0, 0)` always takes
                        // `pack_rows`'s fallback fraction of the *current*
                        // work area instead, clamped by this client's own
                        // (layout-independent) min/max size hints — same
                        // input, same output, no matter what tag/layout
                        // this client was on a second ago.
                        ideal: (0, 0),
                        min: (c.min_width, c.min_height),
                        max: (c.max_width, c.max_height),
                    }
                })
                .collect();

            // Overflow handling: even shrinking every client to its own
            // minimum still doesn't fit — GNOME's "windows that don't fit
            // move to a new workspace" (see `mosaic_overflow_client`'s doc
            // comment). Two strategies, chosen by `mosaic_overflow_stack`:
            // move the offender to an empty tag (default — margo's fixed
            // tag set stands in for infinite dynamic workspaces), or keep
            // it on this tag shrunk to a small corner "peek"
            // (PaperWM-style always-reachable stack). Either way it drops
            // out of `packable` for *this* pass, so `mosaic_arrange` below
            // never sees the client that just left normal packing.
            if self.config.mosaic_auto_overflow_tag
                && let Some(evict_id) =
                    crate::layout::mosaic_overflow_client(work_area, &gaps, &packable)
                && let Some(evict_idx) = self.clients.iter().position(|c| c.id == evict_id)
            {
                if self.config.mosaic_overflow_stack {
                    self.clients[evict_idx].is_mosaic_stacked = true;
                    packable.retain(|c| self.clients[c.index].id != evict_id);
                    self.mark_state_dirty();
                } else if let Some(dest_tag) = self.find_empty_tag(mon_idx, curtag) {
                    let dest_bit = 1u32 << (dest_tag - 1);
                    self.animate_tag_departure(evict_idx);
                    self.clients[evict_idx].old_tags = self.clients[evict_idx].tags;
                    self.clients[evict_idx].is_tag_switching = true;
                    self.clients[evict_idx].animation.running = false;
                    self.clients[evict_idx].tags = dest_bit;
                    packable.retain(|c| self.clients[c.index].id != evict_id);
                    if !self.clients[evict_idx].is_visible_on(mon_idx, tagset) {
                        self.focus_first_visible_or_clear(mon_idx);
                    }
                    self.mark_state_dirty();
                }
            }

            // Position every currently-stacked client (existing ones from
            // a prior pass plus any just marked above) along the work
            // area's bottom-right corner, ascending by id so the row
            // stays stable and gap-free as clients stack/un-stack (ids
            // are monotonic creation order, so this never reshuffles
            // relative to itself the way re-deriving order from `governed`
            // each pass could).
            let mut stacked: Vec<usize> = governed
                .iter()
                .copied()
                .filter(|&i| self.clients[i].is_mosaic_stacked)
                .collect();
            stacked.sort_by_key(|&i| self.clients[i].id);
            for (stack_index, &i) in stacked.iter().enumerate() {
                let peek =
                    crate::layout::mosaic_stack_peek_rect(work_area, &gaps, stack_index as i32);
                self.clients[i].float_geom = peek;
                self.clients[i].geom = peek;
            }

            let placed = crate::layout::mosaic_arrange(work_area, &gaps, &packable);
            for (i, rect) in placed {
                self.clients[i].float_geom = rect;
                self.clients[i].geom = rect;
            }
        } else {
            for &i in &governed {
                if self.clients[i].floated_by_layout
                    && self.clients[i].auto_float_owner == Some(crate::layout::LayoutId::Mosaic)
                {
                    self.clients[i].is_floating = false;
                    self.clients[i].floated_by_layout = false;
                    self.clients[i].auto_float_owner = None;
                    self.clients[i].is_mosaic_stacked = false;
                }
            }
        }
    }

    pub fn arrange_monitor(&mut self, mon_idx: usize) {
        let _span = tracy_client::span!("arrange_monitor");
        if mon_idx >= self.monitors.len() {
            return;
        }
        // Soft-disabled monitor: don't lay out — clients have already
        // been migrated off, and laying out against a panel that isn't
        // being rendered just produces stale geometry.
        if !self.monitors[mon_idx].enabled {
            return;
        }

        // Preserve the old output footprint before any map/unmap/move below.
        // A floating window (or a window moving between monitors) can overlap
        // more than its owner output; both the old and new footprints must be
        // repainted or the neighbouring output keeps stale pixels.
        let mut repaint_outputs = vec![self.monitors[mon_idx].output.clone()];
        for client in self
            .clients
            .iter()
            .filter(|client| client.monitor == mon_idx)
        {
            for output in self.space.outputs_for_element(&client.window) {
                if !repaint_outputs.contains(&output) {
                    repaint_outputs.push(output);
                }
            }
        }

        // Adaptive layout: when `Config::auto_layout` is on AND the
        // user hasn't explicitly picked a layout for the current tag
        // (`pertag.user_picked_layout[curtag]` sticky bit), pick a
        // layout based on the visible-client count and the monitor's
        // aspect ratio. Sets `pertag.ltidxs[curtag]` *before* we read
        // it for `layout` below, so a single arrange pass picks up
        // the new value naturally.
        if self.config.auto_layout && !self.monitors[mon_idx].is_overview {
            self.maybe_apply_adaptive_layout(mon_idx);
        }

        // Floating layout: auto-float / re-tile the current tag's
        // clients before `tiled` is built below (issue #1).
        self.reconcile_floating_layout(mon_idx);
        // Mosaic layout: same idea, content-aware packing instead of a
        // fixed grid — see `reconcile_mosaic_layout`.
        self.reconcile_mosaic_layout(mon_idx);

        let mon = &self.monitors[mon_idx];
        let is_overview = mon.is_overview;
        // Overview path: a single Grid arrangement over the
        // (already-zoomed) work area, holding every tag's clients
        // simultaneously. Mango/Hypr-style geometric continuity —
        // each window keeps a deterministic spot in the thumbnail,
        // and the keyboard-first MRU navigation
        // (`overview_focus_next/prev`) cycles through them with
        // focus + border tracking the selection.
        let mut layout = if is_overview {
            crate::layout::LayoutId::Grid
        } else {
            mon.current_layout()
        };
        let tagset = if is_overview {
            !0
        } else {
            mon.current_tagset()
        };
        let nmaster = mon.current_nmaster();
        let mfact = mon.current_mfact();
        let monitor_area = mon.monitor_area;
        // Apply `overview_zoom` to the work area so the overview Grid
        // arranges every visible window inside a *centered* sub-rect
        // smaller than the full work area — niri's "zoom 0.5" feeling
        // without a true scene-tree transform. Centering keeps the
        // overview rect inside the layer-shell exclusion zone, so the
        // bar and other top/overlay layers stay anchored to the panel
        // edges (niri pattern: top + overlay layers stay at 1.0,
        // background + bottom would zoom in lock-step — margo doesn't
        // depend on the latter today, so we only zoom the workspace
        // surface).
        let work_area = if is_overview {
            let zoom = self.config.overview_zoom.clamp(0.1, 1.0) as f64;
            let wa = mon.work_area;
            let new_w = ((wa.width as f64) * zoom).round() as i32;
            let new_h = ((wa.height as f64) * zoom).round() as i32;
            let dx = (wa.width - new_w) / 2;
            let dy = (wa.height - new_h) / 2;
            crate::layout::Rect {
                x: wa.x + dx,
                y: wa.y + dy,
                width: new_w.max(1),
                height: new_h.max(1),
            }
        } else {
            mon.work_area
        };
        let mut gaps = if is_overview {
            let inner = self.config.overview_gap_inner.max(0);
            let outer = self.config.overview_gap_outer.max(0);
            layout::GapConfig {
                gappih: inner,
                gappiv: inner,
                gappoh: outer,
                gappov: outer,
            }
        } else {
            layout::GapConfig {
                gappih: if self.enable_gaps { mon.gappih } else { 0 },
                gappiv: if self.enable_gaps { mon.gappiv } else { 0 },
                gappoh: if self.enable_gaps { mon.gappoh } else { 0 },
                gappov: if self.enable_gaps { mon.gappov } else { 0 },
            }
        };
        let visible_in_pass = |c: &MargoClient| {
            // Skip clients that haven't gone through their deferred
            // initial map yet — they exist in `self.clients` but
            // haven't been placed in `space` and don't have rules
            // applied. Including them in arrange would map them at
            // the layout's default position, which is exactly the
            // pre-rule flicker we deferred to avoid.
            !c.is_initial_map_pending
                && c.is_visible_on(mon_idx, tagset)
                && (!is_overview || (!c.is_minimized && !c.is_killing && !c.is_in_scratchpad))
        };

        let tiled: Vec<usize> = if is_overview {
            // In overview, the visual cell order should match what
            // alt+Tab walks — so the user can read the grid as
            // "left = most-recently-touched, right = older" (or tag
            // 1-9 / mixed, depending on `overview_cycle_order`).
            // Re-uses the same ordering the cycle path computes.
            self.overview_visible_clients_for_monitor(mon_idx)
        } else {
            self.clients
                .iter()
                .enumerate()
                .filter(|(_, c)| visible_in_pass(c) && c.is_tiled())
                .map(|(i, _)| i)
                .collect()
        };

        let scroller_proportions: Vec<f32> = tiled
            .iter()
            .map(|&i| self.clients[i].scroller_proportion)
            .collect();
        let focused_tiled_pos = self.focused_tiled_pos(mon_idx, &tiled, self.focused_client_idx());

        if !is_overview
            && layout != crate::layout::LayoutId::Floating
            && self.config.smartgaps
            && tiled.len() <= 1
        {
            // Collapse the OUTER gaps for a lone window — but not all the way to
            // 0. Window borders are drawn OUTSET (`render_element_for_client`
            // grows the border rect by `border_width` beyond `c.geom` on every
            // side), so the content must sit far enough inside the work area for
            // the whole border ring to land *within* it. At gap 0 the content is
            // flush to the work-area edge and the outset border spills past it.
            //
            // The two axes need DIFFERENT clamps:
            //
            //  * Left/right (`gappoh`): the work area is flush with the
            //    monitor's physical edges (no side bars). A plain `borderpx`
            //    lands the border's OUTER edge exactly on the screen edge — in a
            //    full-width tile that reads as the border being clipped off the
            //    monitor. Clamp to `2 * borderpx` so the outset border keeps a
            //    `borderpx` margin inside the screen.
            //
            //  * Top/bottom (`gappov`): the work area is already inset from the
            //    monitor by the bar(s), so the border can never reach the screen
            //    edge here. Use just `borderpx` so the outset border's outer edge
            //    lands exactly on the work-area edge, flush against the bar.
            //    `2 * borderpx` would instead leave a `borderpx`-wide strip of
            //    wallpaper between the bar and the border — unnoticeable
            //    left/right (it abuts the bezel) but an obvious gap top/bottom
            //    against the dark bar.
            let bw = self.config.borderpx as i32;
            gaps.gappoh = 2 * bw;
            gaps.gappov = bw;
        }

        // `monly` (port of oniri): when a tag holds exactly one tiled window,
        // maximise it — arrange as Monocle regardless of the active layout, so
        // the lone window fills the work area even in column layouts like
        // scroller (where it would otherwise keep its column width). Pairs with
        // `smartgaps` above, which drops the outer gaps for a single window.
        if !is_overview
            && layout != crate::layout::LayoutId::Floating
            && self.config.monly
            && tiled.len() == 1
        {
            layout = crate::layout::LayoutId::Monocle;
        }

        let ctx = layout::ArrangeCtx {
            work_area,
            tiled: &tiled,
            nmaster,
            mfact,
            gaps: &gaps,
            scroller_proportions: &scroller_proportions,
            default_scroller_proportion: self.config.scroller_default_proportion,
            focused_tiled_pos,
            scroller_structs: self.config.scroller_structs,
            scroller_focus_center: self.config.scroller_focus_center,
            scroller_prefer_center: self.config.scroller_prefer_center,
            scroller_prefer_overspread: self.config.scroller_prefer_overspread,
        };

        // Overview path — mango-ext pattern (`overview(m) { grid(m); }`).
        // Above we forced `layout = Grid` and `tagset = !0` when
        // `is_overview`, and the `tiled` filter at line ~2977 admits
        // floating clients in overview too. So a single Grid arrange
        // over every visible window produces the right shape: 1 window
        // ≈ 90%×90% centred, 2 → side-by-side halves, 4 → 2×2 quarters,
        // 9 → 3×3 evenly. Cells shrink as window count grows, which is
        // the natural Mango/Hypr feel — no fixed 3×3 per-tag thumbnails.
        let mut geometries: Vec<(usize, crate::layout::Rect)> = layout::arrange(layout, &ctx);
        // Floor every layout rect to a positive size. A pathological gap
        // config (e.g. a large `gappov` on a short work area) can drive a
        // master-stack layout's computed width/height negative; a negative
        // size is a protocol error at xdg configure and corrupts
        // border/hit-test math. This is the single choke-point every one
        // of the 11 layouts flows through.
        //
        // A tiled client's min/max-size constraints (its own `xdg_toplevel`
        // request, or a `window_rules.conf` size rule) are deliberately NOT
        // applied here: a tiled window gets exactly the slot its layout
        // computed, the same as dwm / sway / Hyprland / niri / river.
        // Honouring a client-declared minimum meant growing one window past
        // its slot and then shrinking its neighbours — and eating every
        // inter-window and outer gap — to make room; Discord and other
        // Electron apps declare a large one (940×~500), so any layout
        // holding one collapsed to edge-to-edge the moment it appeared. A
        // window that genuinely needs a minimum size can float —
        // `place_floating_cascade` and Mosaic still honour min/max, and so
        // does the floating-geometry clamp in `apply_matched_window_rules`.
        for (_, rect) in &mut geometries {
            rect.width = rect.width.max(1);
            rect.height = rect.height.max(1);
        }

        let now = crate::utils::now_ms();
        // gid → active group member's TARGET slot rect, filled during the
        // loop below and consumed by the hidden-member pre-size pass after
        // it (kills the tab-switch wallpaper flash).
        let mut group_slots: std::collections::HashMap<u32, crate::layout::Rect> =
            std::collections::HashMap::new();
        for (client_idx, mut rect) in geometries {
            // Tabbed group: reserve the tab strip's height at the TOP of the
            // tile and shrink the window content to match, so the strip sits
            // INSIDE the window's allocation (a title-bar band above the
            // content) instead of floating in the gap above it — where it slid
            // under the top bar and ate the outer gap. chip_rects draws the
            // strip at `geom.y - bar_h`, i.e. exactly this reserved band, now
            // within the work area. The shrunk rect is recorded so hidden
            // siblings match it (seamless cycling).
            if self.clients[client_idx].group_active {
                if let Some(gid) = self.clients[client_idx].group_id {
                    let bar_h = self.config.group_bar_height as i32;
                    if bar_h > 0 && rect.height > bar_h {
                        rect.y += bar_h;
                        rect.height -= bar_h;
                    }
                    group_slots.insert(gid, rect);
                }
            }
            let old = self.clients[client_idx].geom;

            // If we're already animating toward exactly this target,
            // leave the in-flight animation alone. arrange_monitor gets
            // called from many event sources (title change → window-
            // rule reapply, focus shift, output resize, scroller pan
            // recompute, …) and a long-running browser like Helium can
            // tick those off every frame while it's playing video. The
            // old behaviour was: each call saw `old != rect` (because
            // `old = c.geom` is the *interpolated* mid-flight value, not
            // the target), restarted the move animation with `initial
            // = old`, and reset `time_started = now`. Result: the
            // animation never finishes — every 16 ms it inches a few
            // pixels toward the target and then resets, producing the
            // exact 1-pixel-per-frame oscillation we kept seeing in the
            // arrange traces (-1794 → -1795 → -1794 → …).
            let already_animating_to_target = self.clients[client_idx].animation.running
                && self.clients[client_idx].animation.current == rect;

            let should_animate = self.config.animations
                && self.config.animation_duration_move > 0
                && !self.clients[client_idx].no_animation
                && !self.clients[client_idx].is_tag_switching
                && old.width > 0
                && old.height > 0
                && old != rect
                && !already_animating_to_target;

            // Diagnostic: every layout decision per visible client.
            // Fires per-client on every tag switch / move / focus
            // arrange — at INFO it floods the journal during normal
            // use (~30-60 lines/sec) and shows up as input latency
            // and journal contention. Trace level keeps it available
            // for `RUST_LOG=margo=trace` debugging without polluting
            // the steady-state log.
            let actual_geom = self.clients[client_idx].window.geometry().size;
            tracing::trace!(
                "arrange[{}]: client_idx={} old={}x{}+{}+{} slot={}x{}+{}+{} actual_buf={}x{} animate={} already_to_target={}",
                self.clients[client_idx].app_id.as_str(),
                client_idx,
                old.width,
                old.height,
                old.x,
                old.y,
                rect.width,
                rect.height,
                rect.x,
                rect.y,
                actual_geom.w,
                actual_geom.h,
                should_animate,
                already_animating_to_target,
            );
            if should_animate {
                // Animate the slot fully — both position AND size lerp
                // from `old` to `rect` over `animation_duration_move`.
                // Combined with the niri-style crossfade that runs in
                // parallel (snapshot rendered on top with fading
                // alpha, scaled to the *current* interpolated slot),
                // this gives the smooth resize transition the user
                // sees from niri/Hyprland's animated layouts: the
                // pre-resize content scales down while the post-
                // resize content fades up.
                //
                // Earlier we used to snap the size to the target on
                // frame 0 (initial.width = rect.width) so the buffer
                // and the slot would always match dimensions — but
                // that left the snapshot fixed at the new slot size
                // for the entire animation, which meant the snapshot
                // was rendered at a *different* size from the captured
                // content for 150 ms and the user saw a stretched/
                // squished version of the pre-resize image. The
                // crossfade infrastructure makes the size-snap
                // unnecessary: we always render BOTH layers at the
                // interpolated slot, and the buffer/slot mismatch on
                // the live layer is hidden under the snapshot until
                // alpha drops.
                let initial = old;
                // niri-style resize transition: if the slot size
                // changes (not just the position), flag a snapshot so
                // the next render captures the *current* surface tree
                // to a `GlesTexture`. While the move animation
                // interpolates the slot from old to new, the render
                // path draws that snapshot scaled to the live slot
                // instead of the live surface — the OLD content stays
                // pinned visually until the client (Electron, slow
                // ack) commits a buffer at the new size, which drops
                // the snapshot. Without this, Helium's 50–100 ms
                // ack-and-reflow window leaks the buffer-vs-slot
                // mismatch onto the screen.
                let slot_size_changed = old.width != rect.width || old.height != rect.height;
                if slot_size_changed && self.clients[client_idx].resize_snapshot.is_none() {
                    self.clients[client_idx].snapshot_pending = true;
                }
                // Spring retarget: if the previous animation was still
                // running, carry its per-channel velocity forward.
                // Without this, the integrator would re-start from rest
                // every time the layout reshuffled mid-flight and the
                // window would visibly hitch — the whole point of the
                // spring clock is that retargets stay continuous.
                // Bezier ignores this field; harmless if it's set.
                // Decide the animation's hard duration. With bezier
                // we honour the user's `animation_duration_move`; with
                // spring we let the physics tell us how long it'll
                // take to settle to within `epsilon` of the target,
                // capped between a sane floor and ceiling so a single
                // bad config value can't produce a 10-second slide.
                let use_spring = self
                    .config
                    .animation_clock_move
                    .eq_ignore_ascii_case("spring");
                let duration_ms = if use_spring {
                    let max_disp = ((rect.x - initial.x).abs())
                        .max((rect.y - initial.y).abs())
                        .max((rect.width - initial.width).abs())
                        .max((rect.height - initial.height).abs())
                        as f64;
                    if max_disp <= 0.5 {
                        // Already at target (sub-pixel). Take the
                        // bezier-style fallback so we still log a
                        // meaningful animation start, but the tick
                        // will settle on the very next frame.
                        self.config.animation_duration_move.max(1)
                    } else {
                        let spring = crate::animation::spring::Spring {
                            from: 0.0,
                            to: max_disp,
                            initial_velocity: 0.0,
                            params: crate::animation::spring::SpringParams::new(
                                self.config.animation_spring_damping_ratio,
                                self.config.animation_spring_stiffness,
                                0.5, // half-pixel epsilon
                            ),
                        };
                        let dur = spring
                            .clamped_duration()
                            .map(|d| d.as_millis() as u32)
                            // Pathological overdamped → fall back.
                            .unwrap_or(self.config.animation_duration_move.max(1));
                        // Clamp: 60 ms floor (one vblank), 1500 ms
                        // ceiling (anything longer is almost certainly
                        // a misconfiguration).
                        dur.clamp(60, 1500)
                    }
                } else {
                    // Overview transitions override the configured
                    // move duration with a snappier value (set by
                    // open_overview/close_overview); falls through to
                    // the user's animation_duration_move otherwise.
                    self.overview_transition_animation_ms
                        .unwrap_or(self.config.animation_duration_move)
                        .max(1)
                };
                self.clients[client_idx].animation = ClientAnimation {
                    should_animate: true,
                    running: true,
                    time_started: now,
                    last_tick_ms: now,
                    duration: duration_ms,
                    initial,
                    current: rect,
                    action: AnimationType::Move,
                    ..Default::default()
                };
                self.clients[client_idx].geom = initial;
            } else if already_animating_to_target {
                // Existing animation still converging on the right
                // target — leave its `time_started`, `initial`, and the
                // current interpolated `c.geom` exactly where they are.
            } else {
                self.clients[client_idx].animation.running = false;
                self.clients[client_idx].geom = rect;
            }
            self.clients[client_idx].is_tag_switching = false;
        }

        // Tabbed groups: pre-size every HIDDEN member to its active
        // sibling's slot. Only the active member is arranged above; the
        // hidden ones otherwise keep a stale size, so when
        // `changegroupactive` cycles to one it reconfigures from that
        // size — leaving a frame where the slot shows the wallpaper
        // before the client redraws (the flash the user reported).
        // Pinning their size means the swap shows their correctly-sized
        // last buffer instantly. Guarded on `geom != slot`, so once
        // settled this configures nothing until the slot actually moves.
        if !group_slots.is_empty() {
            for i in 0..self.clients.len() {
                if self.clients[i].monitor != mon_idx || !self.clients[i].is_hidden_group_member() {
                    continue;
                }
                let slot = self.clients[i]
                    .group_id
                    .and_then(|gid| group_slots.get(&gid).copied());
                if let Some(slot) = slot {
                    if self.clients[i].geom != slot {
                        self.clients[i].geom = slot;
                        self.configure_window_size(i, slot);
                    }
                }
            }
        }

        // Apply fullscreen / floating overrides outside overview. Overview
        // intentionally thumbnails every visible window in the grid.
        if !is_overview {
            for i in 0..self.clients.len() {
                let c = &self.clients[i];
                if c.monitor != mon_idx || !visible_in_pass(c) {
                    continue;
                }
                // Fullscreen geometry per mode:
                //   * Exclusive — full panel, bar will be suppressed
                //     by the render path so the window literally
                //     covers everything.
                //   * WorkArea  — `monitors[mon_idx].work_area`, i.e.
                //     the rect after layer-shell exclusion zones
                //     are subtracted; bar stays drawn on top.
                //   * Off       — fall through to the normal layout /
                //     floating geometry.
                match c.fullscreen_mode {
                    FullscreenMode::Exclusive => {
                        self.clients[i].geom = monitor_area;
                    }
                    FullscreenMode::WorkArea => {
                        self.clients[i].geom = work_area;
                    }
                    FullscreenMode::Off => {
                        if c.is_floating && c.float_geom.width > 0 {
                            self.clients[i].geom = self.clients[i].float_geom;
                        }
                    }
                }
            }
        }

        // Collect windows to show/hide (avoid borrow conflict during space ops)
        let visible: Vec<(Window, Rect, Rect)> = self
            .clients
            .iter()
            .filter(|c| visible_in_pass(c))
            .map(|c| {
                let configure_geom = if c.animation.running {
                    c.animation.current
                } else {
                    c.geom
                };
                (c.window.clone(), c.geom, configure_geom)
            })
            .collect();

        let hidden: Vec<Window> = self
            .clients
            .iter()
            .filter(|c| c.monitor == mon_idx && !visible_in_pass(c))
            .map(|c| c.window.clone())
            .collect();

        for w in hidden {
            self.space.unmap_elem(&w);
        }

        for (window, geom, configure_geom) in visible {
            self.space
                .map_element(window.clone(), (geom.x, geom.y), false);

            if let WindowSurface::Wayland(toplevel) = window.underlying_surface() {
                tracing::debug!(
                    "arrange: setting toplevel size {}x{}",
                    configure_geom.width,
                    configure_geom.height
                );
                toplevel.with_pending_state(|state| {
                    state.size = Some(Size::from((configure_geom.width, configure_geom.height)));
                });
                // Only send the configure if the initial configure has already
                // gone out. The initial configure must be sent during the first
                // commit (see CompositorHandler::commit).
                let initial_sent = with_states(toplevel.wl_surface(), |states| {
                    states
                        .data_map
                        .get::<XdgToplevelSurfaceData>()
                        .and_then(|d| d.lock().ok().map(|d| d.initial_configure_sent))
                        .unwrap_or(false)
                });
                if initial_sent {
                    toplevel.send_pending_configure();
                }
            } else if let WindowSurface::X11(x11) = window.underlying_surface() {
                // X11 (XWayland) clients anchor their menus / popups / tooltips
                // to their OWN absolute position, so they must be told where the
                // compositor actually placed the toplevel. Wayland clients don't
                // need this (the compositor owns their position), but an X11
                // client left thinking it sits elsewhere opens popups detached
                // from the rendered window — the "menus open in the wrong place
                // under XWayland" bug. Mirror the scene rect into the X11 client
                // (guarded so we don't re-configure an already-correct window).
                if geom.width > 0 && geom.height > 0 {
                    let rect = smithay::utils::Rectangle::new(
                        (geom.x, geom.y).into(),
                        (geom.width, geom.height).into(),
                    );
                    if x11.geometry() != rect {
                        tracing::debug!(
                            "arrange: configuring x11 window to {}x{}+{}+{}",
                            geom.width,
                            geom.height,
                            geom.x,
                            geom.y
                        );
                        let _ = x11.configure(rect);
                    }
                }
            }
        }
        self.enforce_z_order();
        crate::border::refresh(self);
        // `map_element` starts with an empty output-membership map. Populate
        // it before the first render/callback cycle so a tag-return whose DRM
        // render is empty can still deliver the waiting wl_surface.frame to
        // the newly-visible window instead of stalling until unrelated damage.
        self.space.refresh();
        for client in self
            .clients
            .iter()
            .filter(|client| client.monitor == mon_idx)
        {
            for output in self.space.outputs_for_element(&client.window) {
                if !repaint_outputs.contains(&output) {
                    repaint_outputs.push(output);
                }
            }
        }
        for output in repaint_outputs {
            self.request_repaint_output(&output);
        }
        // mango 0.16.1 `tag_gather`: compact this monitor's occupied tags
        // to consecutive numbers after every arrange, closing gaps left by
        // closed/moved windows. Remapping only relabels tag numbers — the
        // set of clients visible right now is unchanged (the current
        // view's tag moves with its content), so the geometry just
        // computed above stays valid and this can't feed back into
        // another arrange.
        if self.config.tag_gather {
            self.tag_gather_apply(mon_idx);
        }
        // Refresh the IPC channels so `mctl clients`/`focused`/`status`
        // and any IPC `watch state` subscriber sees the new
        // windows the moment they're laid out. arrange_all already
        // covered both, but arrange_monitor (the path most map/unmap/
        // tag-move events take) didn't — leaving state snapshot + the bar
        // tag-counts stuck on the boot snapshot of zero.
        self.mark_state_dirty();
    }

    /// Pre-compute tiling geometry for the tags the **scroller overview**
    /// is about to show but that aren't currently on screen, so their
    /// windows render at their real tiled slots in the overview cells
    /// without the user having to visit each tag first.
    ///
    /// `arrange_monitor` only ever lays out a monitor's *current* tagset,
    /// so a window mapped onto an unvisited tag keeps whatever stale
    /// `geom` — and surface size — it had at map time. The scroller-
    /// overview render path reads `client.geom` directly and renders the
    /// window's live surface tree, so those windows showed crammed at
    /// their default position/size until the tag was selected once (which
    /// ran a real arrange + configure). This walks every off-screen tag
    /// the strip will show and assigns geom + sends a configure, with no
    /// animation, so the overview is correct from the first open. The
    /// active tag(s) are skipped — they're already laid out live and we
    /// don't want to disturb their in-flight animations.
    pub fn prearrange_overview_tags(&mut self) {
        for mon_idx in 0..self.monitors.len() {
            if !self.monitors[mon_idx].enabled {
                continue;
            }
            let current_tagset = self.monitors[mon_idx].current_tagset();
            for tag in self.scroller_overview_tags(mon_idx) {
                let bit = 1u32 << (tag - 1);
                if bit & current_tagset != 0 {
                    continue; // on screen now — already arranged live.
                }
                self.prearrange_overview_tag(mon_idx, tag);
            }
        }
    }

    /// Lay out a single off-screen `tag` on `mon_idx` (helper for
    /// [`Self::prearrange_overview_tags`]). Mirrors the non-overview
    /// branch of `arrange_monitor` — per-tag layout/nmaster/mfact from
    /// pertag, the same gap + smartgaps rules — but assigns geom directly
    /// (no move animation) and sends each window the configure for its
    /// slot so its buffer matches what the overview cell will scale down.
    fn prearrange_overview_tag(&mut self, mon_idx: usize, tag: usize) {
        let bit = 1u32 << (tag - 1);
        let mon = &self.monitors[mon_idx];
        let layout = mon
            .pertag
            .ltidxs
            .get(tag)
            .copied()
            .unwrap_or_else(|| mon.current_layout());
        let nmaster = mon.pertag.nmasters.get(tag).copied().unwrap_or(1);
        let mfact = mon.pertag.mfacts.get(tag).copied().unwrap_or(0.55);
        let work_area = mon.work_area;
        let monitor_area = mon.monitor_area;
        let mut gaps = layout::GapConfig {
            gappih: if self.enable_gaps { mon.gappih } else { 0 },
            gappiv: if self.enable_gaps { mon.gappiv } else { 0 },
            gappoh: if self.enable_gaps { mon.gappoh } else { 0 },
            gappov: if self.enable_gaps { mon.gappov } else { 0 },
        };

        // Tiled clients on this (mon, tag), in clients-vec order.
        let tiled: Vec<usize> = self
            .clients
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                c.monitor == mon_idx
                    && (c.tags & bit) != 0
                    && !c.is_initial_map_pending
                    && c.is_tiled()
            })
            .map(|(i, _)| i)
            .collect();

        if self.config.smartgaps && tiled.len() <= 1 {
            // Keep room for the OUTSET border — see the matching guard in
            // `arrange_monitor` for the full reasoning. `2 * borderpx`
            // left/right (the work area is flush with the monitor edge there),
            // but only `borderpx` top/bottom so the border lands flush against
            // the bar with no wallpaper strip showing through.
            let bw = self.config.borderpx as i32;
            gaps.gappoh = 2 * bw;
            gaps.gappov = bw;
        }

        let scroller_proportions: Vec<f32> = tiled
            .iter()
            .map(|&i| self.clients[i].scroller_proportion)
            .collect();

        let ctx = layout::ArrangeCtx {
            work_area,
            tiled: &tiled,
            nmaster,
            mfact,
            gaps: &gaps,
            scroller_proportions: &scroller_proportions,
            default_scroller_proportion: self.config.scroller_default_proportion,
            // No live focus on an off-screen tag.
            focused_tiled_pos: None,
            scroller_structs: self.config.scroller_structs,
            scroller_focus_center: self.config.scroller_focus_center,
            scroller_prefer_center: self.config.scroller_prefer_center,
            scroller_prefer_overspread: self.config.scroller_prefer_overspread,
        };

        for (client_idx, mut rect) in layout::arrange(layout, &ctx) {
            // Same contract as `arrange_monitor`: a tiled client keeps the
            // slot the layout gave it; its own min/max-size hint is not
            // applied here.
            rect.width = rect.width.max(1);
            rect.height = rect.height.max(1);
            self.clients[client_idx].animation.running = false;
            self.clients[client_idx].geom = rect;
            self.configure_window_size(client_idx, rect);
        }

        // Floating / fullscreen clients on this tag — the overview
        // thumbnails them too, so give them their intended geometry as
        // well (tiled clients above already returned `None` here).
        for i in 0..self.clients.len() {
            let c = &self.clients[i];
            if c.monitor != mon_idx
                || (c.tags & bit) == 0
                || c.is_initial_map_pending
                || c.is_minimized
                || c.is_killing
                || c.is_in_scratchpad
            {
                continue;
            }
            let rect = match c.fullscreen_mode {
                FullscreenMode::Exclusive => Some(monitor_area),
                FullscreenMode::WorkArea => Some(work_area),
                FullscreenMode::Off if c.is_floating && c.float_geom.width > 0 => {
                    Some(c.float_geom)
                }
                FullscreenMode::Off => None,
            };
            if let Some(rect) = rect {
                self.clients[i].animation.running = false;
                self.clients[i].geom = rect;
                self.configure_window_size(i, rect);
            }
        }
    }

    /// Send `client_idx`'s toplevel a configure sizing it to `geom` (no
    /// position move — the overview places it via `client.geom`). Factored
    /// from `arrange_monitor`'s visible-window loop; only fires once the
    /// initial configure has gone out.
    fn configure_window_size(&mut self, client_idx: usize, geom: Rect) {
        let window = self.clients[client_idx].window.clone();
        if let WindowSurface::Wayland(toplevel) = window.underlying_surface() {
            toplevel.with_pending_state(|state| {
                state.size = Some(Size::from((geom.width, geom.height)));
            });
            let initial_sent = with_states(toplevel.wl_surface(), |states| {
                states
                    .data_map
                    .get::<XdgToplevelSurfaceData>()
                    .and_then(|d| d.lock().ok().map(|d| d.initial_configure_sent))
                    .unwrap_or(false)
            });
            if initial_sent {
                toplevel.send_pending_configure();
            }
        }
    }

    /// Smithay's `Space::map_element` always inserts the touched
    /// element at the top of the stack — there's no way to map at an
    /// explicit z. So every time `arrange_monitor` re-maps a tile-
    /// layer window during a layout change or a move animation, that
    /// tile silently leaps above any floating window (CopyQ,
    /// pavucontrol, picker dialogs) that happened to be on screen.
    ///
    /// To keep "floating sits on top of tiled" actually true, run
    /// this after every `map_element` storm. We re-`raise_element`
    /// floats first, then overlays/scratchpads, in `clients`-vec
    /// forward order — `raise_element` itself moves to top, so the
    /// last raise per band wins, which means the most-recently-
    /// created float of each band ends up at the top of its band
    /// (sane default for "newly opened picker shows on top").
    pub fn enforce_z_order(&mut self) {
        let floats: Vec<smithay::desktop::Window> = self
            .clients
            .iter()
            .filter(|c| (c.is_floating || c.is_in_scratchpad) && !c.is_overlay)
            .map(|c| c.window.clone())
            .collect();
        for w in &floats {
            self.space.raise_element(w, false);
        }
        // A client mid interactive move/resize must render above every
        // other float/tile it may currently overlap, no matter where it
        // sits in `clients` — `arrange_monitor` (and therefore this
        // function) runs on every motion tick of a drag, so without this
        // a resize that grows a window into a neighbour only shows on top
        // when its array index happens to be higher than the neighbour's,
        // which has nothing to do with what the user is actually
        // dragging. Raising it here, after the plain float band above,
        // wins the same last-raise-on-top race in the grabbed client's
        // favour.
        if let Some(w) = self
            .clients
            .iter()
            .find(|c| c.interactive_grab)
            .map(|c| c.window.clone())
        {
            self.space.raise_element(&w, false);
        }
        let overlays: Vec<smithay::desktop::Window> = self
            .clients
            .iter()
            .filter(|c| c.is_overlay)
            .map(|c| c.window.clone())
            .collect();
        for w in &overlays {
            self.space.raise_element(w, false);
        }
    }
}
