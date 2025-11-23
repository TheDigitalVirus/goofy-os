#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(kernel::test_runner)]
#![reexport_test_harness_main = "test_main"]

use core::panic::PanicInfo;

extern crate alloc;

use bootloader_api::config::{BootloaderConfig, Mapping};
use bootloader_api::{BootInfo, entry_point};

use kernel::gdt::STACK_SIZE;

#[cfg(processes_enabled)]
use kernel::tasks::task::NORMAL_PRIORITY;
#[cfg(processes_enabled)]
use kernel::tasks::{init, jump_to_user_land, scheduler, spawn_process};

use kernel::user_program_loader;
use kernel::{println, serial_println};
use x86_64::VirtAddr;
use x86_64::instructions::interrupts;

const USER_PROGRAM_BYTES: &[u8] =
    include_bytes!("../../target/x86_64-unknown-none/release/simple_test");

static mut USER_ENTRY_POINT: VirtAddr = VirtAddr::zero();
static mut USER_STACK_POINTER: VirtAddr = VirtAddr::zero();

#[cfg(processes_enabled)]
extern "C" fn start_user_program() {
    unsafe {
        let entry = USER_ENTRY_POINT;
        let stack = USER_STACK_POINTER;
        let func: extern "C" fn() = core::mem::transmute(entry.as_u64());
        jump_to_user_land(func, stack);
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
    let (_phys_mem_offset, mut frame_allocator) = kernel::init::start_kernel(boot_info);

    #[cfg(processes_enabled)]
    {
        serial_println!("Init scheduler...");
        init();
        serial_println!("Spawn tasks...");

        scheduler::spawn(foo, NORMAL_PRIORITY).unwrap();

        // Load user program
        let user_program = user_program_loader::load_elf(USER_PROGRAM_BYTES, &mut frame_allocator)
            .expect("Failed to load user program");
        unsafe {
            USER_ENTRY_POINT = user_program.entry_point;
            USER_STACK_POINTER = user_program.stack_pointer;
        };

        spawn_process(
            start_user_program,
            NORMAL_PRIORITY,
            user_program.address_space,
        )
        .unwrap();

        serial_println!("Reschedule...");

        kernel::interrupts::init_mouse();
        interrupts::enable();

        scheduler::schedule();
        serial_println!("Returned to kernel_main!");
    }

    #[cfg(not(processes_enabled))]
    {
        kernel::interrupts::init_mouse();
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

    kernel::desktop::main::run_desktop();
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
