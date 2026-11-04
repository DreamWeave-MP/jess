//! A global allocator that counts what each thread asks for, so a test can pin how many native
//! allocations a path makes. Luau's heap counts only if Luau allocates through Rust's allocator.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

thread_local! {
    static COUNT: Cell<usize> = const { Cell::new(0) };
    static BYTES: Cell<usize> = const { Cell::new(0) };
}

fn record(size: usize) {
    COUNT.with(|count| count.set(count.get() + 1));
    BYTES.with(|bytes| bytes.set(bytes.get() + size));
}

/// The system allocator, counting.
pub struct Counting;

// SAFETY: every call forwards to the system allocator unchanged; the counters are const-initialized
// thread locals with no destructor, so touching them never allocates.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record(layout.size());
        // SAFETY: the caller's contract, forwarded.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: the caller's contract, forwarded.
        unsafe { System.dealloc(pointer, layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record(layout.size());
        // SAFETY: the caller's contract, forwarded.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        record(new_size);
        // SAFETY: the caller's contract, forwarded.
        unsafe { System.realloc(pointer, layout, new_size) }
    }
}

/// What this thread allocated while `body` ran, as (result, allocations, bytes).
pub fn measure<R>(body: impl FnOnce() -> R) -> (R, usize, usize) {
    let count = COUNT.with(Cell::get);
    let bytes = BYTES.with(Cell::get);
    let result = body();
    (result, COUNT.with(Cell::get) - count, BYTES.with(Cell::get) - bytes)
}
