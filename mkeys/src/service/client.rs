use super::IPCHandle;

/// The non-server side: connects to the running instance and sends one verb.
pub struct MessageService<T: IPCHandle> {
    ipc_handle: T,
}

impl<T: IPCHandle> MessageService<T> {
    pub fn new(ipc_handle: T) -> Self {
        Self { ipc_handle }
    }

    pub fn send(&self, verb: &[u8]) {
        self.ipc_handle.send(verb);
    }
}
