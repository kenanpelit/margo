use std::io::{ErrorKind, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};

use tracing::{error, info};

use crate::service::IPCHandle;

/// Per-session socket so two margo sessions don't collide.
fn socket_path() -> String {
    let display = std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "wayland-0".into());
    format!("/tmp/margo-mkeys-{display}.sock")
}

pub struct Ipc {
    socket: Option<UnixListener>,
}

impl Ipc {
    pub fn init() -> Self {
        let path = socket_path();
        match UnixListener::bind(&path) {
            Ok(listener) => Self {
                socket: Some(listener),
            },
            Err(e) if e.kind() == ErrorKind::AddrInUse => {
                // The path exists: either a live instance, or a stale socket
                // from a crash. If nothing accepts a connection, it's stale.
                if UnixStream::connect(&path).is_ok() {
                    info!("mkeys: another instance is already running");
                    Self { socket: None }
                } else {
                    let _ = std::fs::remove_file(&path);
                    match UnixListener::bind(&path) {
                        Ok(listener) => Self {
                            socket: Some(listener),
                        },
                        Err(e) => {
                            error!("mkeys: cannot bind {path}: {e}");
                            Self { socket: None }
                        }
                    }
                }
            }
            Err(e) => {
                error!("mkeys: cannot bind {path}: {e}");
                Self { socket: None }
            }
        }
    }

    pub fn is_single_instance(&self) -> bool {
        self.socket.is_some()
    }

    pub fn clean_up() {
        if let Err(e) = std::fs::remove_file(socket_path()) {
            if e.kind() != ErrorKind::NotFound {
                error!("mkeys: socket cleanup failed: {e}");
            }
        }
    }
}

impl Drop for Ipc {
    fn drop(&mut self) {
        if self.socket.is_some() {
            Ipc::clean_up();
        }
    }
}

impl IPCHandle for Ipc {
    fn send(&self, data: &[u8]) {
        if let Ok(mut stream) = UnixStream::connect(socket_path()) {
            let _ = stream.write_all(data);
        }
    }

    fn read(&self) -> Vec<u8> {
        if let Some(listener) = &self.socket {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let mut data = Vec::new();
                    let _ = stream.read_to_end(&mut data);
                    return data;
                }
                Err(e) => {
                    info!("mkeys: accept failed: {e}");
                    return vec![];
                }
            }
        }
        vec![]
    }
}
