use x86_64::{
    PhysAddr, VirtAddr,
    structures::paging::{
        FrameAllocator, Mapper, OffsetPageTable, PageTable, PageTableFlags, PhysFrame, Size2MiB,
        Size4KiB, mapper::MapToError,
    },
};

// use bootloader::bootinfo::{MemoryMap, MemoryRegionType};
use bootloader_api::info::{MemoryRegionKind, MemoryRegions};

use crate::serial_println;

/// A FrameAllocator that returns usable frames from the bootloader's memory map.
pub struct BootInfoFrameAllocator {
    memory_map: &'static MemoryRegions,
    next: usize,
}

impl BootInfoFrameAllocator {
    /// Create a FrameAllocator from the passed memory map.
    ///
    /// This function is unsafe because the caller must guarantee that the passed
    /// memory map is valid. The main requirement is that all frames that are marked
    /// as `USABLE` in it are really unused.
    pub unsafe fn init(memory_map: &'static MemoryRegions) -> Self {
        BootInfoFrameAllocator {
            memory_map,
            next: 0,
        }
    }

    /// Returns an iterator over the usable frames specified in the memory map.
    fn usable_frames(&self) -> impl Iterator<Item = PhysFrame> {
        // get usable regions from memory map
        let regions = self.memory_map.iter();
        let usable_regions = regions.filter(|r| r.kind == MemoryRegionKind::Usable);
        // map each region to its address range
        let addr_ranges = usable_regions.map(|r| r.start..r.end);
        // transform to an iterator of frame start addresses
        let frame_addresses = addr_ranges.flat_map(|r| r.step_by(4096));
        // create `PhysFrame` types from the start addresses
        frame_addresses.map(|addr| PhysFrame::containing_address(PhysAddr::new(addr)))
    }
}

/// Initialize a new OffsetPageTable.
///
/// This function is unsafe because the caller must guarantee that the
/// complete physical memory is mapped to virtual memory at the passed
/// `physical_memory_offset`. Also, this function must be only called once
/// to avoid aliasing `&mut` references (which is undefined behavior).
pub unsafe fn init(physical_memory_offset: VirtAddr) -> OffsetPageTable<'static> {
    unsafe {
        let level_4_table = active_level_4_table(physical_memory_offset);
        OffsetPageTable::new(level_4_table, physical_memory_offset)
    }
}

/// Returns a mutable reference to the active level 4 table.
///
/// This function is unsafe because the caller must guarantee that the
/// complete physical memory is mapped to virtual memory at the passed
/// `physical_memory_offset`. Also, this function must be only called once
/// to avoid aliasing `&mut` references (which is undefined behavior).
unsafe fn active_level_4_table(physical_memory_offset: VirtAddr) -> &'static mut PageTable {
    use x86_64::registers::control::Cr3;

    let (level_4_table_frame, _) = Cr3::read();

    let phys = level_4_table_frame.start_address();
    let virt = physical_memory_offset + phys.as_u64();
    let page_table_ptr: *mut PageTable = virt.as_mut_ptr();

    unsafe { &mut *page_table_ptr }
}

/// A FrameAllocator that always returns `None`.
pub struct EmptyFrameAllocator;

unsafe impl FrameAllocator<Size4KiB> for EmptyFrameAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame> {
        None
    }
}

unsafe impl FrameAllocator<Size4KiB> for BootInfoFrameAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame> {
        let frame = self.usable_frames().nth(self.next);
        if frame.is_none() {
            serial_println!("Frame allocation failed at index {}", self.next);
            // Count total available frames for debugging
            let total_frames = self.usable_frames().count();
            serial_println!(
                "Total usable frames: {}, requested index: {}",
                total_frames,
                self.next
            );
        }
        self.next += 1;
        frame
    }
}

unsafe impl FrameAllocator<Size2MiB> for BootInfoFrameAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame<Size2MiB>> {
        // Find an aligned 2MiB frame by skipping to next aligned frame
        let frame_size = 0x200000; // 2MiB in bytes

        // Store the starting next value to avoid borrowing issues
        let original_next = self.next;

        // Manually iterate through usable frames to find a suitable 2MiB aligned region
        let mut current_index = 0;
        let mut found_frame: Option<PhysFrame> = None;
        let mut found_index = 0;

        for frame in self.usable_frames() {
            if current_index < original_next {
                current_index += 1;
                continue;
            }

            let addr = frame.start_address().as_u64();

            // Check if this frame is 2MiB aligned
            if addr % frame_size == 0 {
                // Check if we have enough consecutive frames available
                // We need 512 consecutive 4KiB frames (512 * 4KiB = 2MiB)
                let mut frames_available = 0;
                let mut check_index = 0;

                // Check consecutive frames starting from current position
                for frame_check in self.usable_frames().skip(current_index) {
                    let check_addr = frame_check.start_address().as_u64();
                    let expected_addr = addr + (check_index * 4096);

                    if check_addr == expected_addr {
                        frames_available += 1;
                        check_index += 1;
                        if frames_available >= 512 {
                            break;
                        }
                    } else {
                        break; // Non-consecutive frame found
                    }
                }

                if frames_available >= 512 {
                    found_frame = Some(frame);
                    found_index = current_index;
                    break;
                }
            }
            current_index += 1;
        }

        if let Some(frame) = found_frame {
            // Reserve all 512 frames by advancing next pointer
            self.next = found_index + 512;
            let addr = frame.start_address().as_u64();
            serial_println!("2MiB frame allocated at address 0x{:x}", addr);
            return Some(PhysFrame::containing_address(PhysAddr::new(addr)));
        }

        serial_println!("2MiB frame allocation failed - no suitable aligned region found");
        None
    }
}
#[derive(Clone, Copy)]
pub struct ProcessAddressSpace {
    pub page_table_frame: PhysFrame<Size4KiB>,
    physical_memory_offset: VirtAddr,
}

impl ProcessAddressSpace {
    pub fn new(
        frame_allocator: &mut impl FrameAllocator<Size4KiB>,
        physical_memory_offset: VirtAddr,
    ) -> Result<Self, MapToError<Size4KiB>> {
        // Create a new page table for the process
        let page_table_frame = frame_allocator
            .allocate_frame()
            .ok_or(MapToError::FrameAllocationFailed)?;

        // Initialize the page table with kernel mappings
        // but separate user space
        let page_table_virt = physical_memory_offset + page_table_frame.start_address().as_u64();
        let page_table_ptr: *mut PageTable = page_table_virt.as_mut_ptr();

        // Zero out the new page table
        unsafe {
            let page_table = &mut *page_table_ptr;
            page_table.zero();

            // Copy kernel mappings from current page table
            // We need to copy ALL kernel mappings to ensure kernel remains accessible
            let current_table = active_level_4_table(physical_memory_offset);

            // Copy ALL non-empty entries to preserve all kernel mappings
            for i in 0..512 {
                if !current_table[i].is_unused() {
                    page_table[i] = current_table[i].clone();
                }
            }

            // Recursively map the L4 page table to itself for easy access
            page_table[510].set_addr(
                page_table_frame.start_address(),
                PageTableFlags::PRESENT | PageTableFlags::WRITABLE,
            );
        }

        Ok(ProcessAddressSpace {
            page_table_frame,
            physical_memory_offset,
        })
    }

    pub fn map_user_memory(
        &mut self,
        virtual_addr: VirtAddr,
        physical_addr: PhysAddr,
        _size: u64, // Currently unused - for future multi-page mappings
        flags: PageTableFlags,
        frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    ) -> Result<(), MapToError<Size4KiB>> {
        use x86_64::structures::paging::Page;

        // Get access to the process's page table
        let page_table_virt =
            self.physical_memory_offset + self.page_table_frame.start_address().as_u64();
        let page_table_ptr: *mut PageTable = page_table_virt.as_mut_ptr();

        unsafe {
            let page_table_ref = &mut *page_table_ptr;
            let mut mapper = OffsetPageTable::new(page_table_ref, self.physical_memory_offset);

            let frame = PhysFrame::containing_address(physical_addr);
            let page = Page::containing_address(virtual_addr);

            // Map memory with user accessible flags
            let final_flags = flags | PageTableFlags::USER_ACCESSIBLE;

            mapper
                .map_to(page, frame, final_flags, frame_allocator)?
                .flush();

            // Check what the page table entry actually contains after mapping
        }
        Ok(())
    }

    pub fn cleanup(&mut self) {
        serial_println!(
            "Cleaning up address space for page table frame: {:?}",
            self.page_table_frame.start_address()
        );

        // Deallocate the page table frame
        let page_table_virt =
            self.physical_memory_offset + self.page_table_frame.start_address().as_u64();
        let page_table_ptr: *mut PageTable = page_table_virt.as_mut_ptr();

        unsafe {
            let page_table_ref = &mut *page_table_ptr;
            page_table_ref.zero();
        }
    }

    /// Create a dummy ProcessAddressSpace for kernel processes
    /// Kernel processes don't need their own page tables
    pub fn dummy(page_table_frame: PhysFrame<Size4KiB>) -> Self {
        ProcessAddressSpace {
            page_table_frame,
            physical_memory_offset: VirtAddr::new(0), // Not used for kernel processes
        }
    }
}

pub unsafe fn enable_user_memory_access(physical_memory_offset: VirtAddr) {
    // FIXME: HADOUKEN
    unsafe {
        for entry in active_level_4_table(physical_memory_offset).iter_mut() {
            if entry.is_unused() {
                continue;
            }

            entry.set_flags(entry.flags() | PageTableFlags::USER_ACCESSIBLE);

            if let Ok(level_3_page_table) = entry.frame().map(|frame: PhysFrame| {
                &mut *(physical_memory_offset + frame.start_address().as_u64())
                    .as_mut_ptr::<PageTable>()
            }) {
                for entry in level_3_page_table.iter_mut() {
                    if entry.is_unused() {
                        continue;
                    }

                    entry.set_flags(entry.flags() | PageTableFlags::USER_ACCESSIBLE);

                    if let Ok(level_2_page_table) = entry.frame().map(|frame| {
                        &mut *(physical_memory_offset + frame.start_address().as_u64())
                            .as_mut_ptr::<PageTable>()
                    }) {
                        for entry in level_2_page_table.iter_mut() {
                            if entry.is_unused() {
                                continue;
                            }

                            entry.set_flags(entry.flags() | PageTableFlags::USER_ACCESSIBLE);

                            if let Ok(level_1_page_table) = entry.frame().map(|frame| {
                                &mut *(physical_memory_offset + frame.start_address().as_u64())
                                    .as_mut_ptr::<PageTable>()
                            }) {
                                for entry in level_1_page_table.iter_mut() {
                                    if entry.is_unused() {
                                        continue;
                                    }

                                    entry
                                        .set_flags(entry.flags() | PageTableFlags::USER_ACCESSIBLE);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
