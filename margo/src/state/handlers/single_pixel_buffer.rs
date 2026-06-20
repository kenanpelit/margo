//! `wp_single_pixel_buffer_v1` delegate.
//!
//! Pure smithay state — no per-protocol handler trait, no policy.
//! GTK4 / kwin clients use this to allocate solid-color regions
//! without a real shm allocation.
