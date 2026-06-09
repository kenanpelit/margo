// ╔═══════════════════════════════════════════════════════════════════════════╗
// ║ app-workspaces — a margo COMPOSITOR PLUGIN (example)                       ║
// ╚═══════════════════════════════════════════════════════════════════════════╝
//
// What a plugin IS: this file + the plugin.toml next to it, in
//   ~/.config/margo/plugins/app-workspaces/
// margo loads it at startup (same Rhai engine + hooks as ~/.config/margo/
// init.rhai); `mctl plugin list / enable / disable` see + toggle it.
//
// What it DOES: when one of the listed apps opens, move its window to a
// fixed "home" tag — so your media / chat windows always land in the same
// place without you dragging them. The window moves there but you stay on
// your current tag (it's waiting on its home tag when you switch to it).
//
// Find an app's id with:  mctl clients
// Edit the table below to taste. Set a tag to 0 to leave an app where it is.

fn home_tag(app) {
    if app == "Spotify"  { return 8; }   // music → tag 8
    if app == "ferdium"  { return 9; }   // messengers → tag 9
    if app == "webcord"  { return 5; }   // Discord → tag 5
    return 0;                            // everything else: don't move
}

on_window_open(|| {
    let app = focused_appid();
    let t = home_tag(app);
    if t > 0 {
        // `tag` moves the focused (just-opened) window to tag `t`. Uncomment
        // the `view` line if you'd rather FOLLOW the window to its home tag.
        dispatch("tag", [tag(t)]);
        // dispatch("view", [tag(t)]);
    }
});

print("plugin app-workspaces loaded");
