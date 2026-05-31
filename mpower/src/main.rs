//! mpower — automatic power-profile manager for margo (daemon + CLI).
//!
//! Replaces the external `ppp-auto-profile` timer/script. A long-lived user
//! daemon: every tick it samples CPU load + AC/battery state and drives
//! power-profiles-daemon per `~/.config/margo/mpower.toml`.
//!
//! Policy:
//!   * **On AC** — go to *performance* on sustained high load (aggregate or a
//!     single hot core), back to *balanced* on sustained calm; streaks +
//!     cooldown damp flapping.
//!   * **On battery** — *balanced*, or *power-saver* below the configured
//!     charge floor. Performance is never selected on battery.
//!   * **Manual override** — if the active profile changes to something
//!     mpower didn't set (the bar pill, the Settings dropdown, `powerprofilesctl`
//!     by hand), mpower backs off and leaves it alone until the next AC
//!     transition, then resumes.
//!
//! CLI: `mpower` / `mpower run` (daemon) · `mpower status` · `mpower pause` ·
//! `mpower resume` · `mpower reload`.

use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use mpower::config::Config;
use mpower::cpu::{self, CpuSample};
use mpower::policy::{self, Band, BALANCED, PERFORMANCE};
use mpower::{ppd, syspower};

fn main() {
    match std::env::args().nth(1).as_deref() {
        None | Some("run") | Some("daemon") => run_daemon(),
        Some("status") => print_status(),
        Some("pause") => set_pause(true),
        Some("resume") => set_pause(false),
        Some("reload") => println!(
            "mpower re-reads {} every tick — nothing to reload.",
            mpower::config_path().display()
        ),
        Some("-h") | Some("--help") | Some("help") => print_help(),
        Some(other) => {
            eprintln!("mpower: unknown command '{other}'\n");
            print_help();
            std::process::exit(2);
        }
    }
}

// ── Daemon ──────────────────────────────────────────────────────────────────

/// In-memory state carried across ticks (no on-disk state file — the daemon
/// is the single owner).
#[derive(Default)]
struct State {
    prev: Option<CpuSample>,
    high_streak: u32,
    low_streak: u32,
    last_switch: Option<Instant>,
    /// The profile mpower last set, used to detect external/manual changes.
    last_set: Option<String>,
    last_on_ac: Option<bool>,
    /// While true, a manual change is in effect — auto-switching is suspended
    /// until the next AC transition.
    manual_override: bool,
}

fn run_daemon() {
    eprintln!(
        "mpower: started (config: {})",
        mpower::config_path().display()
    );
    // Materialise the config on first run so it's discoverable and
    // hand-editable. We run on built-in defaults regardless, but a file the
    // user can see and tweak beats an invisible one.
    let path = mpower::config_path();
    if !path.exists() {
        match Config::default().save() {
            Ok(()) => eprintln!("mpower: wrote default config to {}", path.display()),
            Err(e) => eprintln!("mpower: could not write {}: {e}", path.display()),
        }
    }
    let mut st = State::default();
    loop {
        let cfg = Config::load();
        let tick = cfg.tick_seconds.max(1) as u64;
        if cfg.enabled && !is_paused() {
            tick_once(&cfg, &mut st);
        } else {
            // Disabled/paused: drop transient state so we don't act on a
            // stale CPU delta the moment we resume.
            st.prev = None;
            st.high_streak = 0;
            st.low_streak = 0;
        }
        thread::sleep(Duration::from_secs(tick));
    }
}

fn tick_once(cfg: &Config, st: &mut State) {
    let Some(current) = ppd::get() else {
        return; // power-profiles-daemon unavailable — nothing to manage
    };
    let on_ac = syspower::on_ac();
    let battery = syspower::battery_percent();

    // An AC transition clears any manual override and resets load tracking.
    if let Some(prev_ac) = st.last_on_ac
        && prev_ac != on_ac
    {
        st.manual_override = false;
        st.high_streak = 0;
        st.low_streak = 0;
        st.prev = None;
    }
    st.last_on_ac = Some(on_ac);

    // Manual override: the live profile differs from what we last set → the
    // user (or another tool) took control. Honour it until the next AC flip.
    if let Some(ours) = st.last_set.as_deref()
        && ours != current
    {
        st.manual_override = true;
    }
    if st.manual_override {
        // Keep sampling so deltas are warm when auto resumes.
        st.prev = cpu::sample();
        return;
    }

    let target: Option<&'static str> = if on_ac {
        ac_target(cfg, st)
    } else {
        Some(policy::battery_target(cfg, battery))
    };

    let Some(target) = target else {
        return; // hold current profile
    };
    if target == current {
        // Already where we want to be — adopt it so a later external change
        // is detectable.
        st.last_set = Some(current);
        return;
    }

    // Cooldown gate (anti-flap).
    if let Some(t) = st.last_switch
        && t.elapsed() < Duration::from_secs(cfg.cooldown_seconds as u64)
    {
        return;
    }

    if ppd::set(target) {
        st.last_set = Some(target.to_string());
        st.last_switch = Some(Instant::now());
        st.high_streak = 0;
        st.low_streak = 0;
        if cfg.notify {
            notify(target);
        }
    }
}

/// AC policy: sample CPU, update streaks, decide performance↔balanced.
/// `None` means "hold the current profile".
fn ac_target(cfg: &Config, st: &mut State) -> Option<&'static str> {
    let cur = cpu::sample()?;
    let Some(prev) = st.prev.take() else {
        st.prev = Some(cur);
        return None; // first sample — no delta yet
    };
    let busy = cpu::busy(&prev, &cur);
    st.prev = Some(cur);
    let (avg, max) = busy?;

    let need_high = cfg.high_streak.max(1);
    let need_low = cfg.low_streak.max(1);

    match policy::classify(avg, max, cfg) {
        Band::High => {
            st.high_streak = (st.high_streak + 1).min(need_high);
            st.low_streak = 0;
        }
        Band::Low => {
            st.low_streak = (st.low_streak + 1).min(need_low);
            st.high_streak = 0;
        }
        Band::Mid => {
            st.high_streak = 0;
            st.low_streak = 0;
        }
    }

    if st.high_streak >= need_high {
        Some(PERFORMANCE)
    } else if st.low_streak >= need_low {
        Some(BALANCED)
    } else {
        None
    }
}

fn notify(profile: &str) {
    let (icon, title) = match profile {
        PERFORMANCE => ("power-profile-performance-symbolic", "Performance"),
        policy::POWER_SAVER => ("power-profile-power-saver-symbolic", "Power Saver"),
        _ => ("power-profile-balanced-symbolic", "Balanced"),
    };
    let _ = std::process::Command::new("notify-send")
        .args([
            "-a",
            "mpower",
            "-i",
            icon,
            &format!("Power profile → {title}"),
            "",
        ])
        .status();
}

// ── Pause flag ────────────────────────────────────────────────────────────

fn pause_path() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(std::env::temp_dir)
        .join("mpower.paused")
}

fn is_paused() -> bool {
    pause_path().exists()
}

fn set_pause(pause: bool) {
    let path = pause_path();
    if pause {
        let _ = std::fs::write(&path, b"1");
        println!("mpower: auto-switching paused (until `mpower resume`)");
    } else {
        let _ = std::fs::remove_file(&path);
        println!("mpower: auto-switching resumed");
    }
}

// ── status / help ───────────────────────────────────────────────────────────

fn print_status() {
    let cfg = Config::load();
    let current = ppd::get();
    let on_ac = syspower::on_ac();
    let battery = syspower::battery_percent();

    // Two quick samples for a live reading.
    let live = {
        let a = cpu::sample();
        thread::sleep(Duration::from_millis(300));
        let b = cpu::sample();
        match (a, b) {
            (Some(a), Some(b)) => cpu::busy(&a, &b),
            _ => None,
        }
    };

    println!("mpower status");
    println!("  enabled:         {}", cfg.enabled);
    println!("  paused:          {}", is_paused());
    println!(
        "  power source:    {}",
        if on_ac { "AC" } else { "battery" }
    );
    if let Some(b) = battery {
        println!("  battery:         {b}%");
    }
    println!(
        "  current profile: {}",
        current.as_deref().unwrap_or("unavailable (no power-profiles-daemon)")
    );
    match live {
        Some((avg, max)) => println!(
            "  cpu now:         avg {avg:.0}% / hottest core {max:.0}%"
        ),
        None => println!("  cpu now:         (sampling)"),
    }
    println!(
        "  → performance:   avg ≥ {}% or core ≥ {}%  × {} samples",
        cfg.high_avg_percent,
        cfg.high_max_percent,
        cfg.high_streak.max(1)
    );
    println!(
        "  → balanced:      avg ≤ {}% and core ≤ {}%  × {} samples",
        cfg.low_avg_percent,
        cfg.low_max_percent,
        cfg.low_streak.max(1)
    );
    println!("  cooldown:        {}s", cfg.cooldown_seconds);
    println!("  tick:            {}s", cfg.tick_seconds.max(1));
    println!(
        "  battery saver:   {}",
        if cfg.battery_saver_below > 0 {
            format!("power-saver ≤ {}%", cfg.battery_saver_below)
        } else {
            "off".to_string()
        }
    );
    println!("  config file:     {}", mpower::config_path().display());
}

fn print_help() {
    println!(
        "mpower — automatic power-profile manager for margo

USAGE:
    mpower [run]        Run the daemon (default).
    mpower status       Print live state + thresholds.
    mpower pause        Suspend auto-switching (leaves current profile).
    mpower resume       Resume auto-switching.
    mpower reload       (no-op — the daemon re-reads its config every tick)

CONFIG:
    {}
    Edit it directly or via Settings → Power → Automatic Power Profile.",
        mpower::config_path().display()
    );
}
