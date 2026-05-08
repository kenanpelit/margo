# Margo manual session checklist

A reproducible set of post-install / post-reboot checks that exercise the
parts of margo nested-smoke can't reach: real DRM output paths, layer
shells, portal handshakes, multi-monitor cursor warping, the lock screen,
clipboard managers, day/night colour shifts, and notification daemons.

Run each section in order; the order matters because later steps rely
on earlier ones (e.g. notifications need a working dbus + portal
session).

If any step fails, capture `journalctl --user -u margo-session.service
-n 200` plus `mctl status` and file an issue. The "expected" lines under
each step are exact substring matches; if the actual output differs,
that's the regression.

---

## 0. Bring-up

* [ ] Reboot to a clean session via your DM (greetd / sddm / lemurs).
* [ ] Pick the **margo** session from the DM session list.
* [ ] Type your password — see [§ 5 Lock screen](#5-lock-screen) for the
      keyboard-input regression history; this first login exercises the
      same `text_input_v3` / `input_method_v2` plumbing.

After login, terminal one-liner sanity:
```sh
mctl status | head
```

Expected lines (substring match):
* `output=<your output name> active=true`
* `layout=` non-empty
* `tag[N] state=active focused=true` for at least one tag

---

## 1. Layer shells (status bar / launcher / notifications)

* [ ] Status bar (noctalia / waybar) renders on the configured edge.
* [ ] Bar widgets pick up focus changes:
      * Click a different window in the layout → bar's title slot
        updates within one frame.
      * Run `mctl dispatch togglefullscreen` from another terminal →
        bar's fullscreen indicator updates immediately.
* [ ] Launcher / app-runner (rofi / fuzzel / noctalia launcher):
      * Press the bind, popup appears with the `animation_type_open`
        from the layer rule (zoom for `layer_name:^(rofi|fuzzel|launcher).*`
        per the user's config).
      * Type into the launcher → keys land (xdg_popup.grab + keyboard
        focus routing both work).
      * Press Esc → popup dismisses and focus drops back to the
        previously-focused toplevel (no "stuck on launcher" regression).

## 2. Notifications

* [ ] `notify-send -a Test "Hello" "from margo"` displays a toast.
* [ ] Toast slides in / out per the `noctalia-notification` layer rule
      (or fades silently if `noanim:1` is in your layerrule for the OSD).
* [ ] Toast doesn't steal focus from the active window (no keyboard
      hand-off to a layer surface that didn't request keyboard
      interactivity).
* [ ] `notify-send -u critical "Critical"` shows but doesn't lock input.

## 3. Clipboard

* [ ] `wl-copy "hello"` then `wl-paste` round-trips text.
* [ ] CopyQ / clipse picks up the same clipboard event (data-control
      protocol).
* [ ] Across XWayland: `xclip -sel clip -o` returns the same string
      (XWayland clipboard bridge alive).
* [ ] After the source process exits, `wl-paste` still returns the
      string — wl-clip-persist or similar persistor is running.

## 4. Multi-monitor cursor + tag move

* [ ] Move the cursor to the secondary output. Cursor visibly crosses
      the seam (no re-warp).
* [ ] On the secondary output, focus a window and `mctl dispatch tagmon
      right` (or your `tagmon` bind). Window jumps to the primary; cursor
      can stay where it is.
* [ ] On the primary, tag-switch to a tag pinned to the secondary
      (`tagrule = id:N,monitor_name:eDP-1` for some N). margo warps the
      focus to that monitor — you're now on the secondary.
* [ ] `mctl dispatch focusmon left` and `right` toggle between outputs.

## 5. Lock screen

* [ ] `mctl dispatch spawn 'qs -c noctalia-shell ipc call lockScreen lock'`
      (or your lock bind, e.g. `alt+l`).
* [ ] Black/blur lock surface appears on every output.
* [ ] Type your password — the password dots (or characters) update
      with each keystroke. Keyboard-input regressions historically
      manifested here first.
* [ ] Press Enter → unlock, your previous focus is restored.
* [ ] Repeat with a stuck lock: kill the lock client (`pkill -9 qs`),
      then `super+ctrl+alt+BackSpace` (the user's `force_unlock` bind)
      tears the lock surface down.

## 6. Window rules

* [ ] Open the apps in your `windowrule` lines and confirm:
      * Floating size / offset matches `width / height / offsetx / offsety`.
      * Tags match `tags:N` mask (visible when you switch to that tag).
      * `block_out_from_screencast:1` clients render solid black inside
        wf-recorder / OBS captures.
      * `isnamedscratchpad:1` apps appear hidden by default and fly in
        on the toggle bind (see § 7).
* [ ] Run `scripts/smoke-rules.sh` from the source tree — it walks the
      same rules programmatically.

## 7. Scratchpads

The user has four named scratchpads: `dropdown-terminal`,
`yazi-scratchpad`, `clipse`, `wiremix`. For each:

* [ ] First press of the toggle bind launches the app at its rule-defined
      geometry (visible from frame zero — the bootstrap path puts it
      visible immediately).
* [ ] Same bind a second time hides it (window unmaps from the scene;
      another window receives focus).
* [ ] Third press shows it again on the **cursor's** monitor (not the
      previously focused monitor — `scratchpad_cross_monitor=1` follows
      pointer).
* [ ] With one scratchpad visible, pressing a different scratchpad's
      bind hides the first and shows the second (`single_scratchpad=1`).
* [ ] Emergency: any window that ends up "stuck floating / scratchpad",
      focus it and press `super+ctrl+Escape` (`unscratchpad`); it
      returns to the layout.

## 8. Animations

* [ ] Open a new toplevel: zoom + fade-in over `animation_duration_open`.
* [ ] Close it: scale-down + fade-out over `animation_duration_close`.
* [ ] `super+r` (set_proportion) on a scroller window: smooth slide,
      border tracks the surface with no offset.
* [ ] Tag switch (`super+1` … `super+9`): outgoing windows snapshot-slide
      out, incoming windows slide in from the opposite edge.
* [ ] Focus another window: border colour cross-fades over
      `animation_duration_focus` (120 ms by default).
* [ ] Open/close noctalia panels: layer-rule animation kind applies
      (zoom for launcher, slide for control-center, none for
      OSD/volume/notification toasts).

## 9. Day / night colour shift

* [ ] `sunsetr` (or your gamma-control client) fires at the configured
      sunset time.
* [ ] Output gradually warms (gamma LUT change visible).
* [ ] Toggling sunsetr off restores the default gamma without a flash.

## 10. Portal file picker

* [ ] In a browser (Helium / Chromium), click "Upload file".
* [ ] xdg-desktop-portal-{gtk,gnome} dialog appears as a floating
      window (your windowrule has `appid:^xdg-desktop-portal-...$` →
      `width:960 height:720`).
* [ ] Arrow keys / Enter / Escape work inside the portal — popup grab
      routed correctly.
* [ ] Selecting a file closes the portal and the upload proceeds.

## 11. Screen recording / screencopy

* [ ] `wf-recorder -f /tmp/test.mp4` for ~3 s, Ctrl-C → file produced.
* [ ] `grim -g "$(slurp)" /tmp/region.png` → region screenshot.
* [ ] Block-out rules: a password manager window in the recording
      shows as solid black, not transparent.

## 12. XWayland sanity

* [ ] `DISPLAY=$DISPLAY xeyes` runs (Xwayland is up).
* [ ] Steam / Discord / Spotify (XWayland clients) show the correct
      cursor size — no shrinkage on hover (XCURSOR_SIZE export works).
* [ ] Browser self-activation (clicking a link, switching tabs)
      doesn't bounce you to a different tag (`view_current_to_back`
      regression fix).

## 13. Resource usage when idle

After 30 s of doing nothing:

* [ ] `top -bn1 -p $(pgrep -d, margo)` shows margo around 0–1 % CPU.
* [ ] `intel_gpu_top` (or `nvtop` / `radeontop`) shows the GPU at
      idle / low render rate when nothing is animating.

If margo CPU is pinned at >5 % idle, that's the hot-loop regression
class — capture `journalctl --user -u margo-session.service -n 1000 |
grep border` and investigate.

---

## Reset between runs

Some checks pollute state (focused tag, opened scratchpads, layered
notifications). Reset for the next pass with:

```sh
mctl dispatch view 1
pkill -KILL kitty mpv yazi clipse wiremix 2>/dev/null
```

Then continue from § 1.
