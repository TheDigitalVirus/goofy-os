use crate::PHYSICAL_MEMORY_OFFSET;
use crate::memory::{BootInfoFrameAllocator, ProcessAddressSpace};
use goblin::elf::Elf;
use goblin::elf::program_header::PT_LOAD;
use x86_64::VirtAddr;
use x86_64::structures::paging::{FrameAllocator, PageTableFlags, PhysFrame, Size4KiB};

pub struct UserProgram {
    pub entry_point: VirtAddr,
    pub stack_pointer: VirtAddr,
    pub address_space: ProcessAddressSpace,
}

pub fn load_elf(
    elf_data: &[u8],
    frame_allocator: &mut BootInfoFrameAllocator,
) -> Result<UserProgram, &'static str> {
    let elf = Elf::parse(elf_data).map_err(|_| "Failed to parse ELF")?;

    let phys_mem_offset = PHYSICAL_MEMORY_OFFSET
        .get()
        .ok_or("Physical memory offset not initialized")?;

    let mut address_space = ProcessAddressSpace::new(frame_allocator, *phys_mem_offset)
        .map_err(|_| "Failed to create process address space")?;

    for ph in elf.program_headers {
        if ph.p_type == PT_LOAD {
            let start_addr = ph.p_vaddr;
            let end_addr = start_addr + ph.p_memsz;
            let file_size = ph.p_filesz;
            let file_offset = ph.p_offset;

            let start_page = x86_64::structures::paging::Page::<Size4KiB>::containing_address(
                VirtAddr::new(start_addr),
            );
            let end_page = x86_64::structures::paging::Page::<Size4KiB>::containing_address(
                VirtAddr::new(end_addr - 1),
            );

            let page_range =
                x86_64::structures::paging::Page::range_inclusive(start_page, end_page);

            for page in page_range {
                let frame: PhysFrame<Size4KiB> = frame_allocator
                    .allocate_frame()
                    .ok_or("Failed to allocate frame")?;

                let flags = PageTableFlags::PRESENT
                    | PageTableFlags::WRITABLE
                    | PageTableFlags::USER_ACCESSIBLE;
                // TODO: Handle flags properly based on ph.p_flags (R/W/X)

                address_space
                    .map_user_memory(
                        page.start_address(),
                        frame.start_address(),
                        0,
                        flags,
                        frame_allocator,
                    )
                    .map_err(|_| "Failed to map user memory")?;

                // Zero the frame
                let frame_virt = *phys_mem_offset + frame.start_address().as_u64();
                unsafe {
                    core::ptr::write_bytes(frame_virt.as_mut_ptr::<u8>(), 0, 4096);
                }

                // Copy data
                let page_offset = page.start_address().as_u64();

                // Calculate overlap between page and segment file data
                let segment_file_start = start_addr;
                let segment_file_end = start_addr + file_size;

                let page_start = page_offset;
                let page_end = page_offset + 4096;

                let overlap_start = core::cmp::max(segment_file_start, page_start);
                let overlap_end = core::cmp::min(segment_file_end, page_end);

                if overlap_start < overlap_end {
                    let copy_len = (overlap_end - overlap_start) as usize;
                    let src_offset = (overlap_start - start_addr) + file_offset;
                    let dst_offset = overlap_start - page_start;

                    unsafe {
                        core::ptr::copy_nonoverlapping(
                            elf_data.as_ptr().add(src_offset as usize),
                            frame_virt.as_mut_ptr::<u8>().add(dst_offset as usize),
                            copy_len,
                        );
                    }
                }
            }
        }
    }

    // Allocate stack
    let stack_start = VirtAddr::new(0x80000000);
    let stack_size = 4096 * 4; // 16KB
    let stack_end = stack_start + stack_size;

    let page_range = x86_64::structures::paging::Page::range_inclusive(
        x86_64::structures::paging::Page::<Size4KiB>::containing_address(stack_start),
        x86_64::structures::paging::Page::<Size4KiB>::containing_address(stack_end - 1u64),
    );

    for page in page_range {
        let frame: PhysFrame<Size4KiB> = frame_allocator
            .allocate_frame()
            .ok_or("Failed to allocate stack frame")?;
        let flags =
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE;
        address_space
            .map_user_memory(
                page.start_address(),
                frame.start_address(),
                0,
                flags,
                frame_allocator,
            )
            .map_err(|_| "Failed to map stack")?;
    }

    Ok(UserProgram {
        entry_point: VirtAddr::new(elf.entry),
        stack_pointer: stack_end,
        address_space,
    })
}
