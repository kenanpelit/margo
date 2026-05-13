use notify_rust::Notification;
use mshell_common::watch;
use mshell_services::notification_service;
use relm4::{Component, ComponentSender};
use std::path::PathBuf;

pub fn show_file_saved_notification(summary: String, path: PathBuf) {
    std::thread::spawn(move || {
        let file_path = path.display().to_string();
        let handle = Notification::new()
            .summary(summary.as_str())
            .body(&format!("Saved to {file_path}"))
            .appname("mshell")
            .action("view", "View")
            .action("open_dir", "Show in Files")
            .show();

        if let Ok(handle) = handle {
            handle.wait_for_action(|action| match action {
                "view" => {
                    let _ = std::process::Command::new("xdg-open")
                        .arg(&file_path)
                        .spawn();
                }
                "open_dir" => {
                    if let Some(parent) = PathBuf::from(&file_path).parent() {
                        let _ = std::process::Command::new("xdg-open").arg(parent).spawn();
                    }
                }
                _ => {}
            });
        }
    });
}

pub fn spawn_notifications_watcher<C>(
    sender: &ComponentSender<C>,
    map_notifications: impl Fn() -> C::CommandOutput + Send + Sync + 'static,
) where
    C: Component,
    C::CommandOutput: Send + 'static,
{
    let service = notification_service();
    let notifications = service.notifications.clone();

    // Spawn on the wayle runtime (see `spawn_notification_popups_watcher`
    // below for the rationale).
    let cmd_sender = sender.command_sender().clone();
    let map = std::sync::Arc::new(map_notifications);
    mshell_services::tokio_rt_spawn(async move {
        use ::futures::stream::StreamExt;
        let mut stream = Box::pin(notifications.watch());
        while stream.next().await.is_some() {
            let _ = cmd_sender.send(map());
        }
    });
}

pub fn spawn_notification_popups_watcher<C>(
    sender: &ComponentSender<C>,
    map_notifications: impl Fn() -> C::CommandOutput + Send + Sync + 'static,
) where
    C: Component,
    C::CommandOutput: Send + 'static,
{
    let service = notification_service();
    let notifications = service.popups.clone();

    // We can't use the project-wide `watch!` macro here. It routes the
    // stream through `ComponentSender::command`, which spawns onto
    // relm4's private multi-thread tokio runtime. The wayle
    // `NotificationService` was initialized on `mshell_core::tokio_rt()`
    // — a separate runtime — and its monitoring task lives there.
    // Whatever the cause (waker handoff between runtimes, missed update
    // before first subscription, or both), the relm4-side
    // `WatchStream::poll_next` reliably yields the *initial* value at
    // startup and then never wakes again for subsequent
    // `popups.replace(...)` calls. Spawning the watcher onto the SAME
    // runtime that wayle uses (`tokio_rt`) and forwarding into
    // `sender.command_sender()` from there fixes the missed wakeups —
    // mshell now sees every popup add/remove.
    let cmd_sender = sender.command_sender().clone();
    let map = std::sync::Arc::new(map_notifications);
    mshell_services::tokio_rt_spawn(async move {
        use ::futures::stream::StreamExt;
        let mut stream = Box::pin(notifications.watch());
        while stream.next().await.is_some() {
            let _ = cmd_sender.send(map());
        }
    });
}

pub fn spawn_dnd_watcher<C>(
    sender: &ComponentSender<C>,
    map_dnd: impl Fn() -> C::CommandOutput + Send + Sync + 'static,
) where
    C: Component,
    C::CommandOutput: Send + 'static,
{
    let service = notification_service();
    let dnd = service.dnd.clone();

    watch!(sender, [dnd.watch()], |out| {
        let _ = out.send(map_dnd());
    });
}
