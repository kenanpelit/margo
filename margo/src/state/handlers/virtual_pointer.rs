//! Handler: routes `zwlr_virtual_pointer_v1` events into margo's normal
//! input pipeline via `crate::input_handler::handle_input`, wrapped in the
//! synthetic `VirtualPointerInputBackend`.

use smithay::backend::input::InputEvent;

use crate::protocols::virtual_pointer::{
    VirtualPointerAxisEvent, VirtualPointerButtonEvent, VirtualPointerHandler,
    VirtualPointerInputBackend, VirtualPointerManagerState, VirtualPointerMotionAbsoluteEvent,
    VirtualPointerMotionEvent,
};
use crate::state::MargoState;

impl VirtualPointerHandler for MargoState {
    fn virtual_pointer_manager_state(&mut self) -> &mut VirtualPointerManagerState {
        &mut self.virtual_pointer_state
    }

    fn on_virtual_pointer_motion(&mut self, event: VirtualPointerMotionEvent) {
        crate::input_handler::handle_input(
            self,
            InputEvent::<VirtualPointerInputBackend>::PointerMotion { event },
        );
    }

    fn on_virtual_pointer_motion_absolute(&mut self, event: VirtualPointerMotionAbsoluteEvent) {
        crate::input_handler::handle_input(
            self,
            InputEvent::<VirtualPointerInputBackend>::PointerMotionAbsolute { event },
        );
    }

    fn on_virtual_pointer_button(&mut self, event: VirtualPointerButtonEvent) {
        crate::input_handler::handle_input(
            self,
            InputEvent::<VirtualPointerInputBackend>::PointerButton { event },
        );
    }

    fn on_virtual_pointer_axis(&mut self, event: VirtualPointerAxisEvent) {
        crate::input_handler::handle_input(
            self,
            InputEvent::<VirtualPointerInputBackend>::PointerAxis { event },
        );
    }
}

crate::delegate_virtual_pointer!(MargoState);
