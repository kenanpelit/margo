//! Client-side Wayland protocol bindings for margo IPC.

#[allow(dead_code, non_camel_case_types, clippy::all)]
pub mod dwl_ipc {
    use wayland_client;
    use wayland_client::protocol::*;

    pub mod __interfaces {
        use wayland_client::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("../protocols/dwl-ipc-unstable-v2.xml");
    }
    use self::__interfaces::*;

    wayland_scanner::generate_client_code!("../protocols/dwl-ipc-unstable-v2.xml");
}
