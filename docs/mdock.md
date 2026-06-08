# Dock (mdock)

**mdock** is margo's application dock — a per-app strip of pinned and running
apps. It runs in two complementary forms off one core:

- a **bar pill** you drop into any bar slot, and
- a **standalone** per-output dock that opens like a menu or floats on its own.

Everything is configurable from **Settings → Widgets → mdock** (or the
`dock` block of your shell profile YAML).

## Standalone styles

The standalone dock has two styles (`dock.style`), distinguished by whether it
is *attached* to the bar:

| Style | What it is |
|---|---|
| **`layer_shell`** (default) | Opens **inside the bar's frame**, exactly like the session menu. `Esc` / click-away closes it; the launcher button opens the app launcher. |
| **`popup`** | Its **own floating layer-shell window**, detached from the bar. Honours **Always / Auto-hide / Toggle** behaviour and a configurable screen **edge**. |

Behaviour (`dock.behavior`, popup style):

- **Always** — pinned to the edge, reserves space.
- **Auto-hide** — hidden until the pointer hits a 1 px edge trigger.
- **Toggle** — shown/hidden on demand.

Position (`dock.position`): `top` / `bottom` / `left` / `right`.

## What's on the dock

- **Per-app icons** — one icon per window class, pinned apps first, then
  running-only apps, with a small **running-count indicator** (1/2/3+ dots).
- A **group divider** between the pinned apps and the running-only ones.
- A **hover preview** card (app name + window titles) when `hover_preview` is on.
- An optional **launcher button**.

## Interacting with an icon

| Action | Result |
|---|---|
| **Left-click** | Focus the app; click again to cycle its windows. |
| **Middle-click** | Launch a fresh instance. |
| **Scroll** | Cycle through that app's windows. |
| **Right-click** | Context menu: Pin / Unpin, Launch, the app's `.desktop` actions, per-window details, move / close the focused window. The menu stays on-screen at any edge and closes on `Esc`. |

Pin / Unpin works by window class, so apps with no matching `.desktop` (custom
terminal classes, etc.) can be pinned too.

## Controlling it from the CLI / keybinds

```bash
mshellctl dock toggle      # show/hide the standalone dock
mshellctl dock show
mshellctl dock hide
mshellctl dock activate N  # focus the Nth pinned app (1-based, dock order),
                           # or launch it if it isn't running
```

`dock activate N` is handy on a keybind. `Super+1..9` usually switches tags on a
tiling setup, so bind it to a free chord — e.g. in `binds.conf`:

```ini
bind = super+alt,1,spawn,mshellctl dock activate 1
bind = super+alt,2,spawn,mshellctl dock activate 2
# …
```

## Configuration keys

The `dock` block (shell profile YAML), all editable from Settings:

| Key | Meaning |
|---|---|
| `in_bar` | Show the dock pill in the bar. |
| `standalone` | Run the standalone dock surface. |
| `style` | `layer_shell` (bar-attached) or `popup` (floating). |
| `behavior` | `always` / `auto_hide` / `toggle` (popup style). |
| `position` | `top` / `bottom` / `left` / `right`. |
| `icon_size` | Icon size in px. |
| `show_running` | Show running apps that aren't pinned. |
| `show_tooltips` / `hover_preview` | Hover info. |
| `ignore` | Window classes to leave off the dock. |
| `icon_overrides` | Map a window class → a themed icon name. |
| `launcher_enabled` / `launcher_icon` / `launcher_command` | Launcher button. |

Pinned apps are remembered per machine and can be reordered by drag-and-drop.
