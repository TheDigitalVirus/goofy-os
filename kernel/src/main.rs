#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(kernel::test_runner)]
#![reexport_test_harness_main = "test_main"]

use core::ops::Deref;
use core::panic::PanicInfo;

extern crate alloc;

use bootloader_api::info::MemoryRegion;
use bootloader_api::{BootInfo, entry_point};
#[cfg(uefi)]
use kernel::apic;
use kernel::gdt::STACK_SIZE;

#[cfg(processes_enabled)]
use kernel::syscalls::{SYSNO_EXIT, SYSNO_WRITE};

use kernel::sysinfo::{STACK_BASE, get_stack_pointer};

#[cfg(processes_enabled)]
use kernel::syscall;
#[cfg(processes_enabled)]
use kernel::tasks::task::NORMAL_PRIORITY;
#[cfg(processes_enabled)]
use kernel::tasks::{init, jump_to_user_land, scheduler, syscall0, syscall2};

use kernel::{BOOT_IST_STACK, INTERRUPT_STACK_SIZE};

use kernel::{BootStack, KERNEL_STACK, interrupts as kernel_interrupts};
use kernel::{desktop::main::run_desktop, memory::BootInfoFrameAllocator, println, serial_println};

use bootloader_api::config::{BootloaderConfig, Mapping};
use kernel::{allocator, memory};
use x86_64::VirtAddr;
use x86_64::instructions::interrupts;

#[cfg(processes_enabled)]
extern "C" fn user_foo() {
    let str = b"Hello from user_foo!\n\0";

    // try to use directly the serial device
    // println!("Hello from COM1!");

    for _ in 0..20 {
        syscall!(SYSNO_WRITE, str.as_ptr() as u64, str.len());
    }

    #[allow(forgetting_references)]
    core::mem::forget(str);
    syscall!(SYSNO_EXIT);
}

#[cfg(processes_enabled)]
extern "C" fn create_user_foo() {
    serial_println!("jump to user land");
    unsafe {
        jump_to_user_land(user_foo);
    }
}

#[cfg(processes_enabled)]
extern "C" fn foo() {
    for _ in 0..20 {
        serial_println!("hello from task {}", scheduler::get_current_taskid());
    }
}

pub static BOOTLOADER_CONFIG: BootloaderConfig = {
    let mut config = BootloaderConfig::new_default();
    config.mappings.physical_memory = Some(Mapping::Dynamic);
    config.mappings.kernel_stack = Mapping::FixedAddress(0x8e0000);
    config.kernel_stack_size = STACK_SIZE as u64;
    config
};

entry_point!(kernel_main, config = &BOOTLOADER_CONFIG);

fn kernel_main(boot_info: &'static mut BootInfo) -> ! {
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
    kernel::init(phys_mem_offset);

    let mut mapper = unsafe { memory::init(phys_mem_offset) };
    let mut frame_allocator = unsafe { BootInfoFrameAllocator::init(&boot_info.memory_regions) };

    allocator::init_heap(&mut mapper, &mut frame_allocator).expect("heap initialization failed");

    kernel::framebuffer::init(frame, &mut mapper, &mut frame_allocator);

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

    unsafe {
        memory::enable_user_memory_access(phys_mem_offset);
    }

    #[cfg(processes_enabled)]
    {
        serial_println!("Init scheduler...");
        init();
        serial_println!("Spawn tasks...");

        scheduler::spawn(foo, NORMAL_PRIORITY).unwrap();
        scheduler::spawn(create_user_foo, NORMAL_PRIORITY).unwrap();

        serial_println!("Reschedule...");

        kernel_interrupts::init_mouse();
        interrupts::enable();

        scheduler::reschedule();
        serial_println!("Returned to kernel_main!");
    }

    #[cfg(not(processes_enabled))]
    {
        kernel_interrupts::init_mouse();
        interrupts::enable();
    }

    // Some tests for the heap allocator
    let heap_value = alloc::boxed::Box::new(41);
    println!("heap_value at {:p}", heap_value);

    let heap_vector = alloc::vec![1, 2, 3, 4, 5];
    println!("heap_vector at {:p}", heap_vector.as_ptr());
    let heap_string = alloc::string::String::from("Hello from the heap!");
    println!("heap_string at {:p}", heap_string.as_ptr());

    #[cfg(test)]
    test_main();

    interrupts::enable();

    match kernel::fs::manager::init_filesystem() {
        Ok(_) => {
            serial_println!("Filesystem initialized successfully!");
            println!("Filesystem ready!");
        }
        Err(e) => {
            serial_println!("Failed to initialize filesystem: {}", e);
            println!("Filesystem initialization failed: {}", e);
        }
    }

    #[cfg(test)]
    test_main();

    run_desktop();
}

#[cfg(not(test))]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    println!("Panic occurred: {}", info);
    serial_println!("[failed]\n");
    serial_println!("Error: {}\n", info);
    kernel::hlt_loop();
}

#[cfg(test)]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    kernel::test_panic_handler(info)
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
