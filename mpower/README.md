# mpower

**Automatic power-profile manager for margo.**

`mpower` is a small, long-lived user daemon that picks the right
[power-profiles-daemon](https://gitlab.freedesktop.org/upower/power-profiles-daemon)
profile (`performance` / `balanced` / `power-saver`) for you, based on live
CPU load and whether you are on AC or battery. It replaces the external
`ppp-auto-profile` timer/script — everything it does is configurable, and the
whole config is also exposed in the shell under **Settings → Power →
Automatic Power Profile**.

There is no separate service to babysit any more: `mpower` *is* the auto-profile
mechanism.

## What it does

Every *tick* (default 5 s) the daemon:

1. reads the active profile from `powerprofilesctl`,
2. reads AC/battery state from `/sys/class/power_supply`,
3. samples CPU busy% (aggregate **and** the hottest single core) from
   `/proc/stat`,
4. decides a target profile, and switches only when it differs from the
   current one and the cooldown has elapsed.

### Policy

* **On AC** — climb to **performance** on sustained high load, drop back to
  **balanced** on sustained calm:
  * *high* when aggregate busy ≥ `high_avg_percent` **or** the hottest core ≥
    `high_max_percent` — a single pegged core is enough;
  * *low* when aggregate ≤ `low_avg_percent` **and** the hottest core ≤
    `low_max_percent` — everything must be calm;
  * a change needs `high_streak` / `low_streak` consecutive samples, and at
    least `cooldown_seconds` since the last change (anti-flap).
* **On battery** — **balanced**, or **power-saver** at/under
  `battery_saver_below` % charge (`0` disables that). Performance is never
  selected on battery.
* **Manual override** — if the active profile changes to something `mpower`
  didn't set (the bar pill, the Settings dropdown, `powerprofilesctl` by
  hand), `mpower` backs off and leaves your choice alone **until the next AC
  transition**, then resumes managing.

## CLI

```
mpower [run]      Run the daemon (this is what the service does).
mpower status     Print live state, the current reading, and the thresholds.
mpower pause      Suspend auto-switching (leaves the current profile as-is).
mpower resume     Resume auto-switching.
mpower reload     No-op — the daemon re-reads its config every tick.
```

`pause`/`resume` toggle a flag file at `$XDG_RUNTIME_DIR/mpower.paused`, so a
pause is forgotten on logout.

## Configuration

Config lives at **`~/.config/margo/mpower.toml`** (honours `XDG_CONFIG_HOME`).
It is read fresh every tick, so edits — whether from the settings page or by
hand — take effect on the next tick without a restart. A missing or partial
file is filled in from the defaults below, so you only need to write the keys
you want to change.

| Key | Default | Meaning |
|---|---:|---|
| `enabled` | `true` | Master switch. `false` = daemon idles, never changes the profile. |
| `tick_seconds` | `5` | Sample/decide interval. |
| `high_avg_percent` | `35` | AC: aggregate busy% that asks for performance. |
| `high_max_percent` | `85` | AC: hottest-core busy% that asks for performance. |
| `low_avg_percent` | `18` | AC: aggregate busy% at/under which we may drop to balanced. |
| `low_max_percent` | `70` | AC: hottest-core busy% at/under which we may drop to balanced. |
| `high_streak` | `2` | Consecutive high samples before switching to performance. |
| `low_streak` | `3` | Consecutive low samples before dropping to balanced. |
| `cooldown_seconds` | `20` | Minimum seconds between profile changes. |
| `battery_saver_below` | `20` | On battery, use power-saver at/under this charge %. `0` = never. |
| `notify` | `false` | Desktop notification on each profile change. |

Example:

```toml
# Aggressive on AC, frugal on battery.
high_avg_percent = 25
high_streak = 1
battery_saver_below = 30
notify = true
```

The struct is defined once in `src/config.rs` and re-used by the shell
(`mshell-settings` depends on this crate), so the settings UI and the daemon
can never drift apart.

## Build & install

```bash
cargo build --release -p mpower
sudo install -m755 target/release/mpower /usr/bin/mpower
```

Enable the user service (replaces `ppp-auto-profile`):

```bash
# Retire the old auto-profile units if present.
systemctl --user disable --now ppp-auto-profile.timer ppp-auto-profile.service 2>/dev/null || true

# Install + enable mpower.
install -m644 mpower/mpower.service ~/.config/systemd/user/mpower.service
systemctl --user daemon-reload
systemctl --user enable --now mpower.service
```

`mpower status` confirms it is running and shows what it would do right now.

## Why sysfs + `powerprofilesctl` (not D-Bus/UPower)

The daemon polls anyway, so reading `/sys/class/power_supply` each tick
catches plug/unplug within one interval with zero async machinery, and
shelling out to `powerprofilesctl` keeps the dependency footprint to
`serde` + `toml` + `anyhow`. This mirrors what the retired script did, with
in-memory state instead of a state file.
