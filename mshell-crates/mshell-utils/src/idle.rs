use mshell_common::watch;
use mshell_idle::inhibitor::IdleInhibitor;
use relm4::{Component, ComponentSender};

/// Current idle-inhibitor ("keep awake") state. Lets callers that only need
/// the value read it without depending on `mshell-idle` directly.
pub fn idle_inhibited() -> bool {
    IdleInhibitor::global().get()
}

pub fn spawn_idle_inhibitor_watcher<C>(
    sender: &ComponentSender<C>,
    map: impl Fn() -> C::CommandOutput + Send + Sync + 'static,
) where
    C: Component,
    C::CommandOutput: Send + 'static,
{
    let inhibitor = IdleInhibitor::global();

    watch!(sender, [inhibitor.watch(),], |out| {
        let _ = out.send(map());
    });
}
