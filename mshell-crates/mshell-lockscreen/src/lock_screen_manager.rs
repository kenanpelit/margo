use crate::lock_screen::{
    LOCK_SCREEN_REVEALER_TRANSITION_DURATION, LockScreenInit, LockScreenInput, LockScreenModel,
    LockScreenOutput,
};
use crate::utils::username::current_username;
use gtk4::glib;
use gtk4::glib::SignalHandlerId;
use gtk4::prelude::{GtkWindowExt, MonitorExt, WidgetExt};
use gtk4_layer_shell::{Layer, LayerShell};
use mshell_auth::fingerprint::{FingerprintAuth, FingerprintEvent};
use mshell_idle::inhibitor::IdleInhibitor;
use mshell_session::session_lock::session_lock;
use relm4::{Component, ComponentController, ComponentParts, ComponentSender, Controller, gtk};
use std::time::Duration;
use tracing::info;

pub struct LockScreenManagerModel {
    lock_screens: Vec<Controller<LockScreenModel>>,
    fingerprint_active: bool,
    fingerprint_cancel: Option<tokio::sync::oneshot::Sender<()>>,
    enable_idle_inhibitor_on_unlock: bool,
    _monitor_added_lock_signal_handler_id: SignalHandlerId,
    _lock_signal_handler_id: SignalHandlerId,
    _unlock_signal_handler_id: SignalHandlerId,
}

#[derive(Debug)]
pub enum LockScreenManagerInput {
    LockScreenCreated(Controller<LockScreenModel>),
    SessionLocked,
    SessionUnlocked,
    CancelFingerprint,
    AuthSuccess,
}

#[derive(Debug)]
pub enum LockScreenManagerOutput {}

pub struct LockScreenManagerInit {}

#[derive(Debug)]
pub enum LockScreenManagerCommandOutput {
    FingerprintEvent(FingerprintEvent),
}

#[relm4::component(pub)]
impl Component for LockScreenManagerModel {
    type CommandOutput = LockScreenManagerCommandOutput;
    type Input = LockScreenManagerInput;
    type Output = LockScreenManagerOutput;
    type Init = LockScreenManagerInit;

    view! {
        #[root]
        gtk::Window {
            set_decorated: false,
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        root.init_layer_shell();
        root.set_layer(Layer::Background);
        root.set_default_size(1, 1);
        root.set_visible(false);

        let sender_clone = sender.clone();
        let monitor_added_lock_signal_handler_id =
            session_lock().connect_monitor(move |_instance, monitor| {
                info!("Lock monitor signal for {:?}", monitor.connector());
                let controller = LockScreenModel::builder()
                    .launch(LockScreenInit {
                        monitor: monitor.clone(),
                    })
                    .forward(sender_clone.input_sender(), |msg| match msg {
                        LockScreenOutput::CancelFingerprint => {
                            LockScreenManagerInput::CancelFingerprint
                        }
                        LockScreenOutput::PasswordAuthSuccess => {
                            LockScreenManagerInput::AuthSuccess
                        }
                    });
                sender_clone.input(LockScreenManagerInput::LockScreenCreated(controller));
            });

        let sender_clone = sender.clone();
        let lock_signal_handler_id = session_lock().connect_locked(move |_| {
            sender_clone.input(LockScreenManagerInput::SessionLocked);
        });

        let sender_clone = sender.clone();
        let unlock_signal_handler_id = session_lock().connect_unlocked(move |_| {
            sender_clone.input(LockScreenManagerInput::SessionUnlocked);
        });

        let model = LockScreenManagerModel {
            lock_screens: Vec::new(),
            fingerprint_active: false,
            fingerprint_cancel: None,
            enable_idle_inhibitor_on_unlock: false,
            _monitor_added_lock_signal_handler_id: monitor_added_lock_signal_handler_id,
            _lock_signal_handler_id: lock_signal_handler_id,
            _unlock_signal_handler_id: unlock_signal_handler_id,
        };

        let widgets = view_output!();

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            LockScreenManagerInput::LockScreenCreated(controller) => {
                if let Some(input) = input_for_new_screen(self.fingerprint_active) {
                    controller.sender().emit(input);
                }
                self.lock_screens.push(controller);
            }
            LockScreenManagerInput::SessionLocked => {
                let inhibitor = IdleInhibitor::global();
                self.enable_idle_inhibitor_on_unlock = inhibitor.get();
                tokio::spawn(async move {
                    let _ = inhibitor.disable().await;
                });
                self.fingerprint_active = true;
                self.start_fingerprint_auth(&sender);
            }
            LockScreenManagerInput::SessionUnlocked => {
                info!("Session unlocked");
                self.fingerprint_active = false;
                if let Some(cancel) = self.fingerprint_cancel.take() {
                    let _ = cancel.send(());
                }
                self.lock_screens.clear();
                if self.enable_idle_inhibitor_on_unlock {
                    tokio::spawn(async move {
                        let inhibitor = IdleInhibitor::global();
                        let _ = inhibitor.enable().await;
                    });
                }
            }
            LockScreenManagerInput::CancelFingerprint => {
                self.fingerprint_active = false;
                if let Some(cancel) = self.fingerprint_cancel.take() {
                    let _ = cancel.send(());
                }
                // Lock screens switch to password-only mode
                for ls in &self.lock_screens {
                    ls.emit(LockScreenInput::ShowPasswordEntry);
                }
            }
            LockScreenManagerInput::AuthSuccess => {
                for ls in &self.lock_screens {
                    ls.emit(LockScreenInput::HideScreen);
                }
                glib::timeout_add_local_once(
                    Duration::from_millis(LOCK_SCREEN_REVEALER_TRANSITION_DURATION as u64),
                    || {
                        session_lock().unlock();
                    },
                );
            }
        }
    }

    fn update_cmd(
        &mut self,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            LockScreenManagerCommandOutput::FingerprintEvent(event) => {
                if let FingerprintEvent::Error(e) = &event {
                    info!("Fingerprint error: {e}");
                }
                match route_fingerprint_event(&event) {
                    FingerprintRoute::Screens(input) => {
                        for ls in &self.lock_screens {
                            ls.emit(input.clone());
                        }
                    }
                    FingerprintRoute::AuthSuccess => {
                        sender.input(LockScreenManagerInput::AuthSuccess);
                    }
                    FingerprintRoute::Ignore => {}
                }
            }
        }
    }
}

impl LockScreenManagerModel {
    fn start_fingerprint_auth(&mut self, sender: &ComponentSender<Self>) {
        let username = current_username();
        let cmd_sender = sender.command_sender().clone();
        let (cancel_tx, mut cancel_rx) = tokio::sync::oneshot::channel::<()>();
        self.fingerprint_cancel = Some(cancel_tx);

        tokio::spawn(async move {
            let auth = match FingerprintAuth::new().await {
                Ok(a) => a,
                Err(e) => {
                    info!("fprintd not available: {e}");
                    let _ = cmd_sender.send(LockScreenManagerCommandOutput::FingerprintEvent(
                        FingerprintEvent::Error(e.to_string()),
                    ));
                    return;
                }
            };

            if let Err(e) = auth.start(&username).await {
                info!("fprintd start failed: {e}");
                let _ = cmd_sender.send(LockScreenManagerCommandOutput::FingerprintEvent(
                    FingerprintEvent::Error(e.to_string()),
                ));
                return;
            }

            let _ = cmd_sender.send(LockScreenManagerCommandOutput::FingerprintEvent(
                FingerprintEvent::Ready,
            ));

            let mut unknown_error_count = 0u32;

            loop {
                tokio::select! {
                    _ = &mut cancel_rx => {
                        info!("Fingerprint cancelled");
                        let _ = auth.stop().await;
                        return;
                    }
                    result = auth.wait_for_result() => {
                        match result {
                            Ok(FingerprintEvent::Match) => {
                                let _ = auth.stop().await;
                                let _ = cmd_sender.send(LockScreenManagerCommandOutput::FingerprintEvent(
                                    FingerprintEvent::Match,
                                ));
                                return;
                            }
                            Ok(FingerprintEvent::NoMatch) => {
                                unknown_error_count = 0;
                                let _ = cmd_sender.send(LockScreenManagerCommandOutput::FingerprintEvent(
                                    FingerprintEvent::NoMatch,
                                ));
                                let _ = auth.device.verify_stop().await;
                                let _ = auth.device.verify_start("any").await;
                            }
                            Ok(FingerprintEvent::UnknownError) => {
                                unknown_error_count += 1;
                                if unknown_error_count > 3 {
                                    let _ = auth.stop().await;
                                    let _ = cmd_sender.send(LockScreenManagerCommandOutput::FingerprintEvent(
                                        FingerprintEvent::Error("Device error".into()),
                                    ));
                                    return;
                                }
                                info!("Unknown error, retrying ({unknown_error_count}/3)");
                                let _ = auth.device.verify_stop().await;
                                tokio::time::sleep(Duration::from_secs(2)).await;
                                let _ = auth.device.verify_start("any").await;
                            }
                            Ok(event) => {
                                let _ = auth.stop().await;
                                let _ = cmd_sender.send(LockScreenManagerCommandOutput::FingerprintEvent(event));
                                return;
                            }
                            Err(e) => {
                                let _ = auth.stop().await;
                                let _ = cmd_sender.send(LockScreenManagerCommandOutput::FingerprintEvent(
                                    FingerprintEvent::Error(e.to_string()),
                                ));
                                return;
                            }
                        }
                    }
                }
            }
        });
    }
}

/// What a freshly created lock screen (a monitor hot-plugged, or a surface
/// created after the lock began) should be told so it matches the others.
fn input_for_new_screen(fingerprint_active: bool) -> Option<LockScreenInput> {
    fingerprint_active.then_some(LockScreenInput::FingerprintReady)
}

/// Where a fingerprint event goes.
#[derive(Debug)]
enum FingerprintRoute {
    /// Fan this input out to every lock screen.
    Screens(LockScreenInput),
    /// A finger matched: unlock.
    AuthSuccess,
    Ignore,
}

/// Route one fprintd event.
fn route_fingerprint_event(event: &FingerprintEvent) -> FingerprintRoute {
    match event {
        FingerprintEvent::Ready => FingerprintRoute::Screens(LockScreenInput::FingerprintReady),
        FingerprintEvent::Scanning => {
            FingerprintRoute::Screens(LockScreenInput::FingerprintScanning)
        }
        FingerprintEvent::Match => FingerprintRoute::AuthSuccess,
        FingerprintEvent::NoMatch => FingerprintRoute::Screens(LockScreenInput::FingerprintFailed),
        // The scanner loop retries these itself; nothing reaches here.
        FingerprintEvent::UnknownError => FingerprintRoute::Ignore,
        // Fingerprint unavailable: the lock screens fall back to password.
        FingerprintEvent::Error(_) => FingerprintRoute::Screens(LockScreenInput::ShowPasswordEntry),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_screen_joins_an_active_fingerprint_session() {
        assert!(matches!(
            input_for_new_screen(true),
            Some(LockScreenInput::FingerprintReady)
        ));
    }

    #[test]
    fn fingerprint_events_fan_out_to_the_screens() {
        let route = |e| route_fingerprint_event(&e);
        assert!(matches!(
            route(FingerprintEvent::Ready),
            FingerprintRoute::Screens(LockScreenInput::FingerprintReady)
        ));
        assert!(matches!(
            route(FingerprintEvent::Scanning),
            FingerprintRoute::Screens(LockScreenInput::FingerprintScanning)
        ));
        assert!(matches!(
            route(FingerprintEvent::NoMatch),
            FingerprintRoute::Screens(LockScreenInput::FingerprintFailed)
        ));
        assert!(matches!(
            route(FingerprintEvent::Match),
            FingerprintRoute::AuthSuccess
        ));
        assert!(matches!(
            route(FingerprintEvent::UnknownError),
            FingerprintRoute::Ignore
        ));
        assert!(matches!(
            route(FingerprintEvent::Error("no device".into())),
            FingerprintRoute::Screens(LockScreenInput::ShowPasswordEntry)
        ));
    }
}
