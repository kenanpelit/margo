//! Desktop environment detection helpers for Lua modules
//!
//! Provides the `mdots.desktop.*` API for detecting desktop environments and display servers.

use anyhow::{anyhow, Result};
use mlua::{Lua, Table};
use std::env;
use std::fs;
use std::process::Command;

/// Register desktop environment helpers
pub fn register_desktop_helpers(lua: &Lua) -> Result<()> {
    let globals = lua.globals();
    let mdots: Table = globals
        .get("mdots")
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    let desktop = lua
        .create_table()
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.desktop.environment() -> "kde" | "gnome" | "xfce" | "hyprland" | etc.
    desktop
        .set(
            "environment",
            lua.create_function(|_, ()| Ok(detect_desktop_environment()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.desktop.display_server() -> "wayland" | "x11" | "unknown"
    desktop
        .set(
            "display_server",
            lua.create_function(|_, ()| Ok(detect_display_server()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.desktop.is_wayland() -> boolean
    desktop
        .set(
            "is_wayland",
            lua.create_function(|_, ()| Ok(is_wayland()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.desktop.is_x11() -> boolean
    desktop
        .set(
            "is_x11",
            lua.create_function(|_, ()| Ok(is_x11()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.desktop.window_manager() -> "kwin" | "mutter" | "i3" | "sway" | etc.
    desktop
        .set(
            "window_manager",
            lua.create_function(|_, ()| Ok(detect_window_manager()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.desktop.session_type() -> "x11" | "wayland" | "tty" | "unknown"
    desktop
        .set(
            "session_type",
            lua.create_function(|_, ()| Ok(get_session_type()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.desktop.has_display() -> boolean
    desktop
        .set(
            "has_display",
            lua.create_function(|_, ()| Ok(has_display()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.desktop.compositor() -> compositor name or nil
    desktop
        .set(
            "compositor",
            lua.create_function(|_, ()| Ok(detect_compositor()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.desktop.theme() -> current GTK/Qt theme or nil
    desktop
        .set(
            "theme",
            lua.create_function(|_, ()| Ok(get_desktop_theme()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.desktop.icon_theme() -> current icon theme or nil
    desktop
        .set(
            "icon_theme",
            lua.create_function(|_, ()| Ok(get_icon_theme()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.desktop.screen_resolution() -> "1920x1080" or nil
    desktop
        .set(
            "screen_resolution",
            lua.create_function(|_, ()| Ok(get_screen_resolution()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    mdots
        .set("desktop", desktop)
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    Ok(())
}

/// Detect desktop environment
fn detect_desktop_environment() -> String {
    // Check environment variables first
    if let Ok(de) = env::var("XDG_CURRENT_DESKTOP") {
        let de_lower = de.to_lowercase();
        if de_lower.contains("kde") {
            return "kde".to_string();
        } else if de_lower.contains("gnome") {
            return "gnome".to_string();
        } else if de_lower.contains("xfce") {
            return "xfce".to_string();
        } else if de_lower.contains("lxde") {
            return "lxde".to_string();
        } else if de_lower.contains("lxqt") {
            return "lxqt".to_string();
        } else if de_lower.contains("mate") {
            return "mate".to_string();
        } else if de_lower.contains("cinnamon") {
            return "cinnamon".to_string();
        } else if de_lower.contains("budgie") {
            return "budgie".to_string();
        } else if de_lower.contains("pantheon") {
            return "pantheon".to_string();
        } else if de_lower.contains("hyprland") {
            return "hyprland".to_string();
        }
        return de_lower;
    }

    if let Ok(de) = env::var("DESKTOP_SESSION") {
        return de.to_lowercase();
    }

    // Check for specific processes
    if is_process_running("plasmashell") {
        return "kde".to_string();
    }
    if is_process_running("gnome-shell") {
        return "gnome".to_string();
    }
    if is_process_running("xfce4-session") {
        return "xfce".to_string();
    }
    if is_process_running("Hyprland") {
        return "hyprland".to_string();
    }
    if is_process_running("sway") {
        return "sway".to_string();
    }
    if is_process_running("i3") {
        return "i3".to_string();
    }

    "unknown".to_string()
}

/// Detect display server
fn detect_display_server() -> String {
    if is_wayland() {
        "wayland".to_string()
    } else if is_x11() {
        "x11".to_string()
    } else {
        "unknown".to_string()
    }
}

/// Check if running Wayland
fn is_wayland() -> bool {
    env::var("WAYLAND_DISPLAY").is_ok()
        || env::var("XDG_SESSION_TYPE")
            .map(|s| s.to_lowercase() == "wayland")
            .unwrap_or(false)
}

/// Check if running X11
fn is_x11() -> bool {
    env::var("DISPLAY").is_ok()
        || env::var("XDG_SESSION_TYPE")
            .map(|s| s.to_lowercase() == "x11")
            .unwrap_or(false)
}

/// Detect window manager
fn detect_window_manager() -> String {
    // Check for common window managers
    let wms = [
        ("kwin_x11", "kwin"),
        ("kwin_wayland", "kwin"),
        ("mutter", "mutter"),
        ("xfwm4", "xfwm4"),
        ("openbox", "openbox"),
        ("i3", "i3"),
        ("sway", "sway"),
        ("bspwm", "bspwm"),
        ("awesome", "awesome"),
        ("dwm", "dwm"),
        ("qtile", "qtile"),
        ("Hyprland", "hyprland"),
        ("river", "river"),
        ("wayfire", "wayfire"),
        ("labwc", "labwc"),
    ];

    for (process, name) in &wms {
        if is_process_running(process) {
            return name.to_string();
        }
    }

    // Check WINDOW_MANAGER env var (less reliable)
    if let Ok(wm) = env::var("WINDOW_MANAGER") {
        return wm.to_lowercase();
    }

    "unknown".to_string()
}

/// Get session type from XDG
fn get_session_type() -> String {
    if let Ok(session) = env::var("XDG_SESSION_TYPE") {
        return session.to_lowercase();
    }

    if is_wayland() {
        return "wayland".to_string();
    }
    if is_x11() {
        return "x11".to_string();
    }

    // Check if we're in TTY
    if env::var("TERM")
        .map(|t| t.starts_with("linux"))
        .unwrap_or(false)
    {
        return "tty".to_string();
    }

    "unknown".to_string()
}

/// Check if display is available
fn has_display() -> bool {
    env::var("DISPLAY").is_ok() || env::var("WAYLAND_DISPLAY").is_ok()
}

/// Detect compositor
fn detect_compositor() -> Option<String> {
    let compositors = [
        "picom",
        "compton",
        "xcompmgr",
        "compiz",
        "kwin_x11",
        "kwin_wayland",
        "mutter",
        "Hyprland",
        "sway",
        "wayfire",
    ];

    for comp in &compositors {
        if is_process_running(comp) {
            return Some(comp.to_string());
        }
    }

    None
}

/// Get desktop theme
fn get_desktop_theme() -> Option<String> {
    // Try GTK theme
    if let Ok(home) = env::var("HOME") {
        let gtk3_config = format!("{}/.config/gtk-3.0/settings.ini", home);
        if let Ok(content) = fs::read_to_string(gtk3_config) {
            for line in content.lines() {
                if let Some(theme) = line.strip_prefix("gtk-theme-name=") {
                    return Some(theme.trim().to_string());
                }
            }
        }
    }

    // Try gsettings for GNOME
    if let Ok(output) = Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "gtk-theme"])
        .output()
    {
        if output.status.success() {
            let theme = String::from_utf8_lossy(&output.stdout)
                .trim()
                .trim_matches('\'')
                .to_string();
            if !theme.is_empty() {
                return Some(theme);
            }
        }
    }

    None
}

/// Get icon theme
fn get_icon_theme() -> Option<String> {
    // Try GTK icon theme
    if let Ok(home) = env::var("HOME") {
        let gtk3_config = format!("{}/.config/gtk-3.0/settings.ini", home);
        if let Ok(content) = fs::read_to_string(gtk3_config) {
            for line in content.lines() {
                if let Some(theme) = line.strip_prefix("gtk-icon-theme-name=") {
                    return Some(theme.trim().to_string());
                }
            }
        }
    }

    // Try gsettings
    if let Ok(output) = Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "icon-theme"])
        .output()
    {
        if output.status.success() {
            let theme = String::from_utf8_lossy(&output.stdout)
                .trim()
                .trim_matches('\'')
                .to_string();
            if !theme.is_empty() {
                return Some(theme);
            }
        }
    }

    None
}

/// Get screen resolution
fn get_screen_resolution() -> Option<String> {
    // Try xrandr for X11
    if is_x11() {
        if let Ok(output) = Command::new("xrandr").output() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                if line.contains("*") {
                    // Current resolution line looks like: "   1920x1080     60.00*+"
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if let Some(res) = parts.first() {
                        return Some(res.to_string());
                    }
                }
            }
        }
    }

    // Try wlr-randr for Wayland (wlroots compositors)
    if is_wayland() {
        if let Ok(output) = Command::new("wlr-randr").output() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                if line.contains("current") {
                    // Line looks like: "  1920x1080 px, 60.000000 Hz (current)"
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if let Some(res) = parts.first() {
                        return Some(res.trim().to_string());
                    }
                }
            }
        }
    }

    // Try reading from /sys for framebuffer
    if let Ok(entries) = fs::read_dir("/sys/class/graphics") {
        for entry in entries.filter_map(|e| e.ok()) {
            if entry.file_name().to_string_lossy().starts_with("fb") {
                let modes_path = entry.path().join("modes");
                if let Ok(content) = fs::read_to_string(modes_path) {
                    if let Some(mode) = content.lines().next() {
                        return Some(mode.trim().to_string());
                    }
                }
            }
        }
    }

    None
}

/// Check if a process is running
fn is_process_running(name: &str) -> bool {
    Command::new("pgrep")
        .args(["-x", name])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
