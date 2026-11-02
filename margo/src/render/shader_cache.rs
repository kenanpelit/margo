//! Compile-once slot for the custom GLES programs.
//!
//! Every shader module keeps its program in a thread-local (the GLES
//! context lives on the render thread). A failed compile is remembered as
//! well: a driver that rejects a shader rejects it every time, and retrying
//! on every element build re-ran the compiler and logged an error per frame.

use std::fmt::Debug;

/// State of one cached program.
#[derive(Debug, Default)]
pub enum ShaderSlot<T> {
    /// Not compiled yet.
    #[default]
    Pending,
    /// Compiled and usable.
    Ready(T),
    /// Compilation failed once; never retried for this renderer thread.
    Failed,
}

impl<T> ShaderSlot<T> {
    /// The compiled program, if compilation succeeded earlier.
    pub fn ready_mut(&mut self) -> Option<&mut T> {
        match self {
            Self::Ready(program) => Some(program),
            Self::Pending | Self::Failed => None,
        }
    }

    /// The compiled program, compiling it on first use. A failure is logged
    /// once (as `name`) and cached, so later calls return `None` without
    /// calling `compile` again.
    pub fn get_or_compile<E: Debug>(
        &mut self,
        name: &str,
        compile: impl FnOnce() -> Result<T, E>,
    ) -> Option<&mut T> {
        if matches!(self, Self::Pending) {
            *self = match compile() {
                Ok(program) => Self::Ready(program),
                Err(e) => {
                    tracing::error!("{name} shader compile failed, disabling it: {e:?}");
                    Self::Failed
                }
            };
        }
        self.ready_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_is_cached_and_not_retried() {
        let mut slot = ShaderSlot::<u32>::Pending;
        let mut calls = 0;
        for _ in 0..3 {
            let got = slot.get_or_compile("test", || {
                calls += 1;
                Err::<u32, _>("bad driver")
            });
            assert!(got.is_none());
        }
        assert_eq!(calls, 1, "a failed compile must not be retried every frame");
        assert!(matches!(slot, ShaderSlot::Failed));
    }

    #[test]
    fn success_is_cached() {
        let mut slot = ShaderSlot::Pending;
        let mut calls = 0;
        for _ in 0..3 {
            let got = slot.get_or_compile("test", || {
                calls += 1;
                Ok::<_, ()>(7_u32)
            });
            assert_eq!(got.copied(), Some(7));
        }
        assert_eq!(calls, 1);
        assert_eq!(slot.ready_mut().copied(), Some(7));
    }
}
