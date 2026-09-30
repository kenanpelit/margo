use relm4::RelmApp;
use tracing::info;

use crate::{
    config::Config,
    layout::{assets::LayoutAssets, parse::LayoutDefinition},
    native::VirtualKeyboard,
    ui::{StyleAssets, UIModel},
};

use super::IPCHandle;

pub trait KeyboardHandle {
    fn key_press(&mut self, key: evdev::KeyCode);
    fn key_release(&mut self, key: evdev::KeyCode);

    fn append_mod(&mut self, key: evdev::KeyCode);
    fn remove_mod(&mut self, key: evdev::KeyCode);

    fn append_lock(&mut self, key: evdev::KeyCode);
    fn remove_lock(&mut self, key: evdev::KeyCode);

    fn destroy(&mut self);
}

/// The resident side: owns the GTK app, the virtual keyboard, and the IPC
/// listener (whose reads drive the `quit` command from message clients).
pub struct AppService<N: IPCHandle + Send + 'static> {
    ipc_handle: N,
}

impl<N: IPCHandle + Send + 'static> AppService<N> {
    pub fn new(ipc_handle: N) -> Self {
        Self { ipc_handle }
    }

    pub fn run(self) {
        let config = Config::load();
        let layout_str = LayoutAssets::by_name(&config.layout);
        let mut layout = LayoutDefinition::from_toml(&layout_str)
            .or_else(|_| LayoutDefinition::from_toml(&LayoutAssets::by_name("en")))
            .expect("bundled en layout must parse");
        // Compiled once and shared with both the legend pass below and
        // the virtual keyboard's submitted keymap — previously each
        // independently re-read config.conf and re-compiled xkbcommon's
        // keymap on every launch.
        let keymap = crate::xkb_config::compiled_keymap();

        // Re-point single-character legends at margo's actual active xkb
        // layout — a no-op if `keymap` is None, so the bundled TOML
        // stays the fallback. MUST run before extra_rows::apply(), which
        // appends its own static (not xkb-derived) digit/F-key legends.
        crate::layout::live_legends::apply(&mut layout, keymap.as_ref());
        crate::layout::extra_rows::apply(&mut layout, &config);

        let keyboard = VirtualKeyboard::new(keymap);

        let app = RelmApp::new("org.margo.mkeys");
        relm4::set_global_css(&StyleAssets::get_default_style_file());

        info!("mkeys: starting UI ({} layout)", config.layout);
        app.with_args(vec![]).run::<UIModel>((
            Box::new(keyboard) as Box<dyn KeyboardHandle>,
            Box::new(self.ipc_handle) as Box<dyn IPCHandle + Send>,
            layout,
            config,
        ));
    }
}
