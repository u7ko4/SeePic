//! A counting global allocator, for finding out where the memory goes.
//!
//! The viewer's own accounting can only see the buffers it holds on purpose;
//! the process's private bytes also include everything the UI framework and the
//! graphics driver keep. This counts the bytes that pass through Rust's
//! allocator, so a growing process total can be attributed to one side or the
//! other instead of guessed at.
//!
//! Two relaxed atomics per allocation, which is below the noise floor here, and
//! the numbers are only formatted when `IV_TRACE_PERF` is set, so the counting
//! costs nothing worth gating at run time.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

/// Bytes currently allocated through this allocator.
pub fn live_bytes() -> usize {
    LIVE.load(Ordering::Relaxed)
}

/// The high-water mark of [`live_bytes`].
pub fn peak_bytes() -> usize {
    PEAK.load(Ordering::Relaxed)
}

fn grew(bytes: usize) {
    let live = LIVE.fetch_add(bytes, Ordering::Relaxed) + bytes;
    PEAK.fetch_max(live, Ordering::Relaxed);
}

/// The system allocator with byte counters.
pub struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller upholds the `GlobalAlloc` contract, and the
        // pointer is passed back unchanged.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            grew(layout.size());
        }
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            grew(layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        // SAFETY: the pointer and layout are the ones this allocator returned.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: the pointer and layout are the ones this allocator returned.
        let new_ptr = unsafe { System.realloc(ptr, layout, new_size) };
        if !new_ptr.is_null() {
            if new_size >= layout.size() {
                grew(new_size - layout.size());
            } else {
                LIVE.fetch_sub(layout.size() - new_size, Ordering::Relaxed);
            }
        }
        new_ptr
    }
}
