use alloc::alloc::{GlobalAlloc, Layout};
use core::sync::atomic::{AtomicUsize, Ordering};

use linked_list_allocator::LockedHeap;

use x86_64::{
    VirtAddr,
    structures::paging::{
        FrameAllocator, Mapper, Page, PageTableFlags, Size4KiB, mapper::MapToError,
    },
};

use crate::serial_println;

#[global_allocator]
pub static mut ALLOCATOR: CountingAllocator = CountingAllocator::empty();

pub fn init_heap(
    mapper: &mut impl Mapper<Size4KiB>,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    heap_start: u64,
    heap_size: u64,
) -> Result<(), MapToError<Size4KiB>> {
    let page_range = {
        let heap_start = VirtAddr::new(heap_start);
        let heap_end = heap_start + heap_size - 1u64;
        let heap_start_page = Page::containing_address(heap_start);
        let heap_end_page = Page::containing_address(heap_end);
        Page::range_inclusive(heap_start_page, heap_end_page)
    };
    for page in page_range {
        let frame = frame_allocator
            .allocate_frame()
            .ok_or(MapToError::FrameAllocationFailed)?;
        let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE;
        unsafe { mapper.map_to(page, frame, flags, frame_allocator)?.flush() };
    }

    unsafe {
        init_allocator(&raw mut ALLOCATOR, heap_start, heap_size);
    }

    serial_println!(
        "Heap initialized successfully at {:#x} with size {} bytes",
        heap_start,
        heap_size
    );

    Ok(())
}

unsafe fn init_allocator(allocator: *mut CountingAllocator, start: u64, size: u64) {
    unsafe {
        (*allocator)
            .inner
            .lock()
            .init(start as *mut u8, size as usize);
        (*allocator).allocated.store(0, Ordering::SeqCst);
    };
}

pub struct CountingAllocator {
    inner: LockedHeap,
    allocated: AtomicUsize,
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { self.inner.alloc(layout) };
        if !ptr.is_null() {
            self.allocated.fetch_add(layout.size(), Ordering::SeqCst);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { self.inner.dealloc(ptr, layout) };
        self.allocated.fetch_sub(layout.size(), Ordering::SeqCst);
    }
}

impl CountingAllocator {
    pub const fn empty() -> Self {
        CountingAllocator {
            inner: LockedHeap::empty(),
            allocated: AtomicUsize::new(0),
        }
    }

    pub fn allocated(&self) -> usize {
        self.allocated.load(Ordering::SeqCst)
    }
}
