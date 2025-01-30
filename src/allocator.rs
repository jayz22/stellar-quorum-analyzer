use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct LimitedAllocator {
    limit: usize,
    allocated: AtomicUsize,
}

unsafe impl GlobalAlloc for LimitedAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let new_size = self.allocated.fetch_add(layout.size(), Ordering::SeqCst);
        if new_size > self.limit {
            self.allocated.fetch_sub(layout.size(), Ordering::SeqCst);
            std::ptr::null_mut()
        } else {
            System.alloc(layout)
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        self.allocated.fetch_sub(layout.size(), Ordering::SeqCst);
        System.dealloc(ptr, layout);
    }
}

// limit internal memory allocation to 2 GiB
const MEMORY_LIMIT: usize = 2 * 1024 * 1024 * 1024;

#[global_allocator]
static ALLOCATOR: LimitedAllocator = LimitedAllocator {
    limit: MEMORY_LIMIT,
    allocated: AtomicUsize::new(0),
};
