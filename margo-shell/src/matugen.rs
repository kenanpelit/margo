//! `mshell matugen` — wallpaper → Material You palette → margo/mshell renkleri.
//!
//! Yapılan iş:
//!   1. Bir hedef wallpaper'a karar ver:
//!        • CLI arg verildiyse onu kullan
//!        • Aksi halde `state.json`'dan aktif output'un `active_tag_mask`'ını
//!          oku, en alt bit'i tag numarasına çevir ve `mshell.toml`'un
//!          `[wallpaper.tags]` map'inden path'i çek.
//!   2. `matugen image <wallpaper> -c <config>` subprocess'ini çalıştır.
//!      Config `~/.config/margo/matugen/config.toml` (override: env
//!      `MARGO_MATUGEN_CONFIG`).
//!   3. `mctl reload` ile margo'ya yeni `margo-colors.conf`'u
//!      tekrar parse ettir (config.conf bu dosyayı `source =` ile
//!      include ediyor).
//!   4. notify-send + stdout mesajı bas.
//!
//! mshell tarafı kendi temasını henüz hot-swap etmiyor — `mshell-colors.toml`
//! üretiliyor ama otomatik yüklenmiyor (layered TOML overlay loader sonraki
//! turda eklenecek).

use anyhow::{Context, Result, anyhow, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::get_config;

pub fn run_cli(wallpaper_arg: Option<PathBuf>) -> Result<()> {
    let wallpaper = match wallpaper_arg {
        Some(p) => p,
        None => resolve_active_wallpaper()?,
    };

    if !wallpaper.exists() {
        bail!("wallpaper bulunamadı: {}", wallpaper.display());
    }

    let config = matugen_config_path();
    if !config.exists() {
        bail!("matugen config bulunamadı: {}", config.display());
    }

    if which("matugen").is_none() {
        bail!("matugen yüklü değil (pacman -S matugen)");
    }

    let mode = std::env::var("MARGO_MATUGEN_MODE").unwrap_or_else(|_| "dark".to_string());
    let scheme = std::env::var("MARGO_MATUGEN_TYPE")
        .unwrap_or_else(|_| "scheme-tonal-spot".to_string());
    let prefer =
        std::env::var("MARGO_MATUGEN_PREFER").unwrap_or_else(|_| "saturation".to_string());

    let status = Command::new("matugen")
        .arg("image")
        .arg(&wallpaper)
        .arg("-c")
        .arg(&config)
        .arg("--mode")
        .arg(&mode)
        .arg("--type")
        .arg(&scheme)
        .arg("--prefer")
        .arg(&prefer)
        .arg("-q")
        .status()
        .context("matugen subprocess'i başlatılamadı")?;

    if !status.success() {
        bail!("matugen exit kodu: {}", status);
    }

    // margo'yu best-effort reload et — yoksa sessiz geç.
    if which("mctl").is_some() {
        let _ = Command::new("mctl").arg("reload").status();
    }

    notify_user(&wallpaper);

    println!(
        "matugen: {} → palette renderlandı, mctl reload tetiklendi",
        wallpaper.display()
    );
    Ok(())
}

fn matugen_config_path() -> PathBuf {
    if let Ok(env) = std::env::var("MARGO_MATUGEN_CONFIG") {
        return PathBuf::from(env);
    }
    home_dir()
        .join(".config")
        .join("margo")
        .join("matugen")
        .join("config.toml")
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

fn state_json_path() -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let uid = unsafe { libc::getuid() };
            PathBuf::from(format!("/run/user/{uid}"))
        });
    base.join("margo").join("state.json")
}

/// Resolve the wallpaper currently shown on the active output by
/// consulting state.json + mshell's `[wallpaper.tags]` map.
fn resolve_active_wallpaper() -> Result<PathBuf> {
    let state_path = state_json_path();
    let raw = std::fs::read(&state_path)
        .with_context(|| format!("state.json okunamadı: {}", state_path.display()))?;
    let state: serde_json::Value =
        serde_json::from_slice(&raw).context("state.json parse edilemedi")?;

    let outputs = state
        .get("outputs")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow!("state.json'da `outputs` yok"))?;

    let active = outputs
        .iter()
        .find(|o| {
            o.get("active")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        })
        .ok_or_else(|| anyhow!("aktif output bulunamadı"))?;

    // 1) state.json'da output'un kendi `wallpaper` alanı doluysa onu kullan
    //    (margo `tagrule = id:N,wallpaper:…` yazılmışsa burası dolu olur).
    if let Some(wp) = active
        .get("wallpaper")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        return Ok(expand_path(wp));
    }

    // 2) mshell.toml'un [wallpaper.tags] map'i — aktif tag'in path'i.
    let mask = active
        .get("active_tag_mask")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| anyhow!("aktif output'ta active_tag_mask yok"))?;

    if mask == 0 {
        bail!("aktif output'ta hiçbir tag açık değil (mask=0)");
    }

    let tag = (mask as u32).trailing_zeros() + 1; // 1-indexed
    let key = tag.to_string();

    let (cfg, _) =
        get_config(None).map_err(|e| anyhow!("mshell config okunamadı: {e}"))?;
    let raw_path = cfg
        .wallpaper
        .tags
        .get(&key)
        .ok_or_else(|| anyhow!("[wallpaper.tags] tag={tag} için kayıt yok"))?;

    Ok(expand_path(raw_path))
}

fn expand_path(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        home_dir().join(rest)
    } else if p == "~" {
        home_dir()
    } else {
        PathBuf::from(p)
    }
}

fn which(bin: &str) -> Option<PathBuf> {
    std::env::var_os("PATH")?
        .to_string_lossy()
        .split(':')
        .map(|d| Path::new(d).join(bin))
        .find(|p| p.is_file())
}

fn notify_user(wallpaper: &Path) {
    if which("notify-send").is_none() {
        return;
    }
    let name = wallpaper
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let _ = Command::new("notify-send")
        .arg("🎨 mshell matugen")
        .arg(format!("{name} → tema güncellendi"))
        .arg("-t")
        .arg("2500")
        .status();
}
