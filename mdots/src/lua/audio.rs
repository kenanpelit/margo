//! Audio system detection helpers for Lua modules
//!
//! Provides the `mdots.audio.*` API for detecting audio system configuration.

use anyhow::{anyhow, Result};
use mlua::{Lua, Table};
use std::fs;
use std::path::Path;
use std::process::Command;

/// Register audio system helpers
pub fn register_audio_helpers(lua: &Lua) -> Result<()> {
    let globals = lua.globals();
    let mdots: Table = globals
        .get("mdots")
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    let audio = lua
        .create_table()
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.audio.server() -> "pulseaudio" | "pipewire" | "alsa" | "none"
    audio
        .set(
            "server",
            lua.create_function(|_, ()| Ok(detect_audio_server()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.audio.has_pulseaudio() -> boolean
    audio
        .set(
            "has_pulseaudio",
            lua.create_function(|_, ()| Ok(has_pulseaudio()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.audio.has_pipewire() -> boolean
    audio
        .set(
            "has_pipewire",
            lua.create_function(|_, ()| Ok(has_pipewire()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.audio.has_jack() -> boolean
    audio
        .set(
            "has_jack",
            lua.create_function(|_, ()| Ok(has_jack()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.audio.has_alsa() -> boolean
    audio
        .set(
            "has_alsa",
            lua.create_function(|_, ()| Ok(has_alsa()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.audio.list_cards() -> array of sound card names
    audio
        .set(
            "list_cards",
            lua.create_function(|lua, ()| {
                let cards = list_sound_cards();
                let table = lua.create_table()?;
                for (i, card) in cards.iter().enumerate() {
                    table.set(i + 1, card.clone())?;
                }
                Ok(table)
            })
            .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.audio.default_sink() -> sink name or nil
    audio
        .set(
            "default_sink",
            lua.create_function(|_, ()| Ok(get_default_sink()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.audio.default_source() -> source name or nil
    audio
        .set(
            "default_source",
            lua.create_function(|_, ()| Ok(get_default_source()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    // mdots.audio.bluetooth_available() -> boolean
    audio
        .set(
            "bluetooth_available",
            lua.create_function(|_, ()| Ok(is_bluetooth_audio_available()))
                .map_err(|e| anyhow!("Lua error: {}", e))?,
        )
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    mdots
        .set("audio", audio)
        .map_err(|e| anyhow!("Lua error: {}", e))?;

    Ok(())
}

/// Detect which audio server is running
fn detect_audio_server() -> String {
    // Check for PipeWire first (newer)
    if has_pipewire() {
        return "pipewire".to_string();
    }

    // Check for PulseAudio
    if has_pulseaudio() {
        return "pulseaudio".to_string();
    }

    // Check for JACK
    if has_jack() {
        return "jack".to_string();
    }

    // Fallback to ALSA if sound cards exist
    if has_alsa() {
        return "alsa".to_string();
    }

    "none".to_string()
}

/// Check if PulseAudio is running
fn has_pulseaudio() -> bool {
    // Check if pulseaudio process is running
    if is_process_running("pulseaudio") {
        return true;
    }

    // Check if PulseAudio socket exists
    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        let socket_path = format!("{}/pulse/native", runtime_dir);
        if Path::new(&socket_path).exists() {
            return true;
        }
    }

    // Check using pactl
    Command::new("pactl")
        .args(["info"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Check if PipeWire is running
fn has_pipewire() -> bool {
    // Check if pipewire process is running
    if is_process_running("pipewire") {
        return true;
    }

    // Check if PipeWire socket exists
    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        let socket_path = format!("{}/pipewire-0", runtime_dir);
        if Path::new(&socket_path).exists() {
            return true;
        }
    }

    // Check using pw-cli
    Command::new("pw-cli")
        .args(["info", "0"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Check if JACK is running
fn has_jack() -> bool {
    // Check if jackd process is running
    if is_process_running("jackd") || is_process_running("jackdbus") {
        return true;
    }

    // Check using jack_control
    Command::new("jack_control")
        .args(["status"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Check if ALSA is available
fn has_alsa() -> bool {
    Path::new("/proc/asound").exists() || Path::new("/dev/snd").exists()
}

/// List sound cards
fn list_sound_cards() -> Vec<String> {
    let mut cards = Vec::new();

    // Read from /proc/asound/cards
    if let Ok(content) = fs::read_to_string("/proc/asound/cards") {
        for line in content.lines() {
            // Format: " 0 [PCH            ]: HDA-Intel - HDA Intel PCH"
            if let Some(stripped) = line.strip_prefix(' ') {
                let parts: Vec<&str> = stripped.split('[').collect();
                if parts.len() >= 2 {
                    if let Some(name_part) = parts[1].split(']').next() {
                        cards.push(name_part.trim().to_string());
                    }
                }
            }
        }
    }

    // Alternative: read from /sys/class/sound
    if cards.is_empty() {
        if let Ok(entries) = fs::read_dir("/sys/class/sound") {
            for entry in entries.filter_map(|e| e.ok()) {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with("card") {
                    let id_path = entry.path().join("id");
                    if let Ok(id) = fs::read_to_string(&id_path) {
                        cards.push(id.trim().to_string());
                    }
                }
            }
        }
    }

    cards
}

/// Get default sink (output device)
fn get_default_sink() -> Option<String> {
    // Try PipeWire first
    if has_pipewire() {
        if let Ok(output) = Command::new("pw-cli").args(["info", "all"]).output() {
            // This is a simplified check - real implementation would parse JSON output
            let stdout = String::from_utf8_lossy(&output.stdout);
            if let Some(line) = stdout.lines().find(|l| l.contains("default.audio.sink")) {
                // Extract sink name from line
                return Some(line.trim().to_string());
            }
        }
    }

    // Try PulseAudio
    if has_pulseaudio() {
        if let Ok(output) = Command::new("pactl").args(["get-default-sink"]).output() {
            if output.status.success() {
                return Some(String::from_utf8_lossy(&output.stdout).trim().to_string());
            }
        }
    }

    None
}

/// Get default source (input device)
fn get_default_source() -> Option<String> {
    // Try PipeWire first
    if has_pipewire() {
        if let Ok(output) = Command::new("pw-cli").args(["info", "all"]).output() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if let Some(line) = stdout.lines().find(|l| l.contains("default.audio.source")) {
                return Some(line.trim().to_string());
            }
        }
    }

    // Try PulseAudio
    if has_pulseaudio() {
        if let Ok(output) = Command::new("pactl").args(["get-default-source"]).output() {
            if output.status.success() {
                return Some(String::from_utf8_lossy(&output.stdout).trim().to_string());
            }
        }
    }

    None
}

/// Check if Bluetooth audio is available
fn is_bluetooth_audio_available() -> bool {
    // Check if bluez is available
    if !Path::new("/sys/class/bluetooth").exists() {
        return false;
    }

    // Check if PulseAudio or PipeWire has Bluetooth module
    if has_pulseaudio() {
        if let Ok(output) = Command::new("pactl")
            .args(["list", "modules", "short"])
            .output()
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if stdout.contains("bluetooth") {
                return true;
            }
        }
    }

    if has_pipewire() {
        // PipeWire typically has bluetooth support if bluez is installed
        return true;
    }

    false
}

/// Check if a process is running
fn is_process_running(name: &str) -> bool {
    Command::new("pgrep")
        .args(["-x", name])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
