//! Microbenchmark for the compositor `.conf` parser.
//!
//! `parse_config` runs at startup and on every `mctl reload`, and the heavy
//! paths are the per-line bind / window-rule parsers (regex compile, CSV
//! splitting). Bench a representative config (a spread of option lines, a
//! stack of binds, and several PCRE2 window rules) as a regression shield.
//!
//! Run with `cargo bench -p margo-config`.

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use margo_config::parse_config;

const SAMPLE: &str = "\
# representative margo config
gaps = true
gappih = 8
gappiv = 8
gappoh = 12
gappov = 12
mfact = 0.55
nmaster = 1
border_width = 2
smartgaps = true
focus_on_hover = true
default_layout = tile
taglayout = 1, tile
taglayout = 2, scroller
taglayout = 3, grid
twilight = true
twilight_mode = schedule

bind = super,Return,spawn,kitty
bind = super,d,spawn,mshellctl menu app-launcher
bind = super,g,spawn,mshellctl menu mdash
bind = super,q,killclient
bind = super,f,togglefullscreen
bind = super,space,togglefloating
bind = super,j,focusstack,+1
bind = super,k,focusstack,-1
bind = super+shift,j,movestack,+1
bind = super+shift,k,movestack,-1
bind = super,1,view,1
bind = super,2,view,2
bind = super,3,view,3
bind = super+shift,1,tag,1
bind = super+shift,2,tag,2
bind = super+shift,3,tag,3
bind = super,h,setmfact,-0.05
bind = super,l,setmfact,+0.05
bind = super,Tab,mru_next,1
bind = super+shift,Tab,mru_prev,1

windowrule = float, title:^(Picture-in-Picture)$
windowrule = float, app-id:^(pavucontrol|nm-connection-editor)$
windowrule = tag:2, app-id:^(firefox|Helium|Chromium)$
windowrule = tag:3, app-id:^(code|jetbrains-.*)$
windowrule = opacity:0.95, app-id:^(kitty|Alacritty)$
windowrule = float, app-id:^(org\\.gnome\\.Calculator)$
windowrule = monitor:1, app-id:^(Spotify|spotify)$
windowrule = noborder, title:^(.*— mpv)$
";

fn bench_parse(c: &mut Criterion) {
    let dir = std::env::temp_dir().join(format!("margo-bench-parse-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("bench: create temp dir");
    let path = dir.join("config.conf");
    std::fs::write(&path, SAMPLE).expect("bench: write sample config");

    c.bench_function("parse_config", |b| {
        b.iter(|| parse_config(black_box(Some(path.as_path()))).expect("parse"));
    });

    let _ = std::fs::remove_dir_all(&dir);
}

criterion_group!(benches, bench_parse);
criterion_main!(benches);
