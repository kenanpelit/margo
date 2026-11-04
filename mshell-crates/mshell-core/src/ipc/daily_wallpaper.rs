//! Daily-wallpaper auto-fetch.

use super::*;

/// Daily-wallpaper auto-fetch (port of the noctalia `daily-wallpaper` plugin).
/// Main-thread glib timers do the *check* (reading the reactive config is only
/// safe on the main thread); the actual blocking download runs on a worker and
/// the resulting path is sent back as `SetWallpaper` so it's applied on main.
pub(super) fn spawn_daily_wallpaper_task(tx: mpsc::UnboundedSender<IPCCommand>) {
    let last_applied: std::rc::Rc<std::cell::RefCell<String>> =
        std::rc::Rc::new(std::cell::RefCell::new(String::new()));

    // Apply shortly after login.
    {
        let tx = tx.clone();
        let last = last_applied.clone();
        glib::timeout_add_local_once(Duration::from_secs(15), move || run_daily_check(&tx, &last));
    }
    // Re-check every 30 min so a date rollover swaps to the new day's image.
    glib::timeout_add_seconds_local(30 * 60, move || {
        run_daily_check(&tx, &last_applied);
        glib::ControlFlow::Continue
    });
}

/// One daily-wallpaper check (main thread). Skips when disabled or already done
/// for today; otherwise kicks the blocking fetch on a worker and applies the
/// result via `SetWallpaper`.
pub(super) fn run_daily_check(
    tx: &mpsc::UnboundedSender<IPCCommand>,
    last: &std::rc::Rc<std::cell::RefCell<String>>,
) {
    // Each reactive-store accessor consumes the subfield, so re-walk the chain.
    if !config_manager()
        .config()
        .wallpaper()
        .daily_wallpaper_enabled()
        .get_untracked()
    {
        return;
    }
    let Some(today) = glib::DateTime::now_local()
        .ok()
        .and_then(|d| d.format("%Y-%m-%d").ok())
        .map(|s| s.to_string())
    else {
        return;
    };
    if *last.borrow() == today {
        return;
    }
    *last.borrow_mut() = today;

    let source = config_manager()
        .config()
        .wallpaper()
        .daily_wallpaper_source()
        .get_untracked();
    let locale = config_manager()
        .config()
        .wallpaper()
        .daily_wallpaper_locale()
        .get_untracked();
    let tx = tx.clone();
    tokio_rt().spawn(async move {
        let fetched = tokio::task::spawn_blocking(move || {
            mshell_cache::wallpaper::fetch_daily_wallpaper(&source, &locale)
        })
        .await;
        match fetched {
            Ok(Ok(path)) => {
                let _ = tx.send(IPCCommand::SetWallpaper(path));
            }
            Ok(Err(e)) => tracing::warn!(error = %e, "daily wallpaper: fetch failed"),
            Err(e) => tracing::warn!(error = %e, "daily wallpaper: task join failed"),
        }
    });
}
