//! Reusable offscreen render targets for SHM captures.
//!
//! wlr-screencopy and ext-image-copy-capture SHM frames render into a GLES
//! renderbuffer and read it back. Allocating a full-size renderbuffer for
//! every captured frame (a screen recorder asks for one per output refresh)
//! churns GPU memory for no reason, so captures borrow one from a small pool
//! keyed by size and hand it back afterwards. The read-back
//! (`copy_framebuffer` + `map_texture`) completes before the buffer is
//! returned, so the next capture can't overwrite pixels still being read.

use std::cell::RefCell;

use smithay::backend::renderer::Offscreen;
use smithay::backend::renderer::gles::{GlesError, GlesRenderbuffer, GlesRenderer};
use smithay::utils::{Buffer, Size};

/// Most distinct capture sizes kept around (outputs × region sizes in use).
const POOL_CAP: usize = 4;

/// Small most-recently-used pool of values keyed by size.
#[derive(Debug)]
pub struct SizePool<T> {
    entries: Vec<(Size<i32, Buffer>, T)>,
    cap: usize,
}

impl<T> SizePool<T> {
    pub const fn new(cap: usize) -> Self {
        Self {
            entries: Vec::new(),
            cap,
        }
    }

    /// Take the pooled value for `size`, or create one.
    pub fn take_or_create<E>(
        &mut self,
        size: Size<i32, Buffer>,
        create: impl FnOnce() -> Result<T, E>,
    ) -> Result<T, E> {
        match self.entries.iter().position(|(s, _)| *s == size) {
            Some(idx) => Ok(self.entries.remove(idx).1),
            None => create(),
        }
    }

    /// Return a value for reuse; the least recently returned size is
    /// dropped beyond the cap.
    pub fn put(&mut self, size: Size<i32, Buffer>, value: T) {
        self.entries.retain(|(s, _)| *s != size);
        self.entries.insert(0, (size, value));
        self.entries.truncate(self.cap);
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }
}

thread_local! {
    static CAPTURE_BUFFERS: RefCell<SizePool<GlesRenderbuffer>> =
        const { RefCell::new(SizePool::new(POOL_CAP)) };
}

/// Borrow an Xrgb8888 renderbuffer of `size` for one capture. Give it back
/// with [`return_capture_renderbuffer`] once the read-back is done.
pub fn take_capture_renderbuffer(
    renderer: &mut GlesRenderer,
    size: Size<i32, Buffer>,
) -> Result<GlesRenderbuffer, GlesError> {
    CAPTURE_BUFFERS.with(|pool| {
        pool.borrow_mut().take_or_create(size, || {
            <GlesRenderer as Offscreen<GlesRenderbuffer>>::create_buffer(
                renderer,
                drm_fourcc::DrmFourcc::Xrgb8888,
                size,
            )
        })
    })
}

/// Hand a renderbuffer from [`take_capture_renderbuffer`] back to the pool.
pub fn return_capture_renderbuffer(size: Size<i32, Buffer>, buffer: GlesRenderbuffer) {
    CAPTURE_BUFFERS.with(|pool| pool.borrow_mut().put(size, buffer));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn size(w: i32, h: i32) -> Size<i32, Buffer> {
        Size::from((w, h))
    }

    #[test]
    fn reuses_a_returned_buffer_of_the_same_size() {
        let mut pool = SizePool::new(4);
        let mut created = 0;
        for _ in 0..5 {
            let value = pool
                .take_or_create(size(1920, 1080), || {
                    created += 1;
                    Ok::<_, ()>(created)
                })
                .expect("create");
            pool.put(size(1920, 1080), value);
        }
        assert_eq!(created, 1, "one allocation for repeated same-size captures");
        // A different size gets its own buffer.
        let other = pool.take_or_create(size(800, 600), || Ok::<_, ()>(99));
        assert_eq!(other, Ok(99));
    }

    #[test]
    fn evicts_the_least_recently_returned_size() {
        let mut pool = SizePool::new(2);
        pool.put(size(1, 1), 1);
        pool.put(size(2, 2), 2);
        pool.put(size(3, 3), 3);
        assert_eq!(pool.len(), 2);
        assert_eq!(pool.take_or_create(size(1, 1), || Ok::<_, ()>(0)), Ok(0));
        assert_eq!(pool.take_or_create(size(3, 3), || Ok::<_, ()>(0)), Ok(3));
    }
}
