//! Wayland protocol server-side bindings generated from XML files.

#[allow(dead_code, non_camel_case_types, clippy::all)]
pub mod dwl_ipc {
    use wayland_server;
    use wayland_server::protocol::*;

    pub mod __interfaces {
        use wayland_server::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("../protocols/dwl-ipc-unstable-v2.xml");
    }
    use self::__interfaces::*;

    wayland_scanner::generate_server_code!("../protocols/dwl-ipc-unstable-v2.xml");
}

#[allow(dead_code, non_camel_case_types, clippy::all)]
pub mod wlr_foreign_toplevel {
    use wayland_server;
    use wayland_server::protocol::*;

    pub mod __interfaces {
        use wayland_server::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!(
            "../protocols/wlr-foreign-toplevel-management-unstable-v1.xml"
        );
    }
    use self::__interfaces::*;

    wayland_scanner::generate_server_code!(
        "../protocols/wlr-foreign-toplevel-management-unstable-v1.xml"
    );
}
