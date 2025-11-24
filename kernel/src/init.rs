use crate::{
    BOOT_IST_STACK, BootStack, INTERRUPT_STACK_SIZE, KERNEL_STACK, allocator,
    gdt::STACK_SIZE,
    memory,
    memory::BootInfoFrameAllocator,
    println, serial_println,
    sysinfo::{STACK_BASE, get_stack_pointer},
};
use bootloader_api::BootInfo;
use bootloader_api::info::MemoryRegion;
use core::ops::Deref;
use x86_64::VirtAddr;

#[cfg(uefi)]
use crate::arch::x86_64::apic; // TODO: Auto import correct arch

pub const HEAP_START: u64 = 0x_4444_4444_0000;
pub const HEAP_SIZE: u64 = 100 * 1024; // 100 KiB

pub fn start_kernel(boot_info: &'static mut BootInfo) -> (VirtAddr, BootInfoFrameAllocator) {
    unsafe { STACK_BASE = get_stack_pointer() as usize };

    serial_println!("Booting goofy OS...");

    let boot_stack = get_boot_stack(boot_info.memory_regions.deref());
    serial_println!(
        "Kernel stack: {:#x} - {:#x}",
        boot_stack.start.as_u64(),
        boot_stack.end.as_u64()
    );
    KERNEL_STACK.init_once(|| boot_stack);

    let frame = boot_info.framebuffer.as_mut().unwrap();

    let phys_mem_offset = VirtAddr::new(boot_info.physical_memory_offset.into_option().unwrap());

    // Initialize the OS
    crate::init(phys_mem_offset);

    let mut mapper = unsafe { memory::init(phys_mem_offset) };
    let mut frame_allocator = unsafe { BootInfoFrameAllocator::init(&boot_info.memory_regions) };

    match allocator::init_heap(&mut mapper, &mut frame_allocator, HEAP_START, HEAP_SIZE) {
        Ok(_) => serial_println!("Heap initialized"),
        Err(e) => panic!("Heap initialization failed: {:?}", e),
    }

    crate::framebuffer::init(frame, &mut mapper, &mut frame_allocator);

    #[cfg(uefi)]
    {
        unsafe {
            apic::init(
                *boot_info.rsdp_addr.as_ref().unwrap() as usize,
                phys_mem_offset,
                &mut mapper,
                &mut frame_allocator,
            )
        };
    };

    match crate::fs::manager::init_filesystem() {
        Ok(_) => {
            serial_println!("Filesystem initialized successfully!");
            println!("Filesystem ready!");
        }
        Err(e) => {
            serial_println!("Failed to initialize filesystem: {}", e);
            println!("Filesystem initialization failed: {}", e);
        }
    }

    (phys_mem_offset, frame_allocator)
}

fn get_boot_stack(_regions: &[MemoryRegion]) -> BootStack {
    return BootStack::new(
        VirtAddr::new(0x8e0000),
        VirtAddr::new(0x8e0000 + STACK_SIZE as u64),
        VirtAddr::new((BOOT_IST_STACK.0.as_ptr() as usize).try_into().unwrap()),
        VirtAddr::new(
            (BOOT_IST_STACK.0.as_ptr() as usize + INTERRUPT_STACK_SIZE)
                .try_into()
                .unwrap(),
        ),
    );
}
