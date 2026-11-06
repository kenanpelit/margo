//! The config effects that drive the frame itself: the frame toggle and
//! inset, the side-menu expansion types and every menu's position.

use super::*;

/// Push the frame's own config effects onto `effects`.
pub(super) fn push_frame_effects(
    effects: &mut EffectScope,
    base_config: &reactive_stores::ArcStore<Config>,
    sender: &ComponentSender<Frame>,
) {
    let config = base_config.clone();
    let sender_clone = sender.clone();
    effects.push(move |_| {
        let config = config.clone();
        let enable_frame = config.clone().bars().frame().enable_frame().get();
        // Subscribe to the inset too, so editing `frameless_gap` re-fires
        // this effect → `SetDrawFrame` re-reads the fresh gap and re-pushes
        // the inset to the bars (live slider, no shell restart).
        let _ = config.bars().frame().frameless_gap().get();
        sender_clone.input(FrameInput::SetDrawFrame(enable_frame));
    });

    let config = base_config.clone();
    let sender_clone = sender.clone();
    effects.push(move |_| {
        let config = config.clone();
        let expansion_type = config.menus().left_menu_expansion_type().get();
        sender_clone.input(FrameInput::SetLeftMenuExpansionType(expansion_type));
    });

    let config = base_config.clone();
    let sender_clone = sender.clone();
    effects.push(move |_| {
        let config = config.clone();
        let expansion_type = config.menus().right_menu_expansion_type().get();
        sender_clone.input(FrameInput::SetRightMenuExpansionType(expansion_type));
    });

    let menu_config = base_config.clone();
    let sender_clone = sender.clone();
    effects.push(move |_| {
        // Collect every menu's position into one ordered Vec. Each
        // `.get()` both reads the value AND subscribes this effect to that
        // menu (so a position change re-fires it), and the collected
        // snapshot lets `RepositionMenus` skip the destructive restack when
        // nothing moved (the config store is coarse: any write re-fires
        // this effect). Each StoreField accessor consumes its receiver, so
        // every read clones the config handle.
        let mut positions: Vec<Position> = Vec::new();
        // One entry per row of the shared position table, so every menu
        // (including the bespoke surfaces) re-fires the restack when moved.
        macro_rules! track {
            ($($menu:ident => $field:ident),* $(,)?) => {
                $(positions.push(menu_config.clone().menus().$field().position().get());)*
            };
        }
        positioned_menus!(track);
        sender_clone.input(FrameInput::RepositionMenus(positions));
    });
}
