#![no_std]
#![cfg_attr(test, no_main)]
#![feature(abi_x86_interrupt)]
#![feature(custom_test_frameworks)]
#![test_runner(crate::test_runner)]
#![reexport_test_harness_main = "test_main"]
#![feature(const_slice_make_iter)]
#![allow(incomplete_features)]
#![feature(generic_const_exprs)]

#[cfg(test)]
use bootloader_api::{BootInfo, entry_point};
use conquer_once::spin::OnceCell;
use x86_64::VirtAddr;

use core::panic::PanicInfo;
use exit::{QemuExitCode, exit_qemu};

extern crate alloc;

pub mod allocator;
pub mod apic;
pub mod desktop;
pub mod exit;
pub mod framebuffer;
pub mod fs;
pub mod gdt;
pub mod interrupts;
pub mod memory;
pub mod serial;
pub mod surface;
pub mod sysinfo;
pub mod time;

// #[cfg(processes_enabled)]
pub mod tasks;

use bootloader_api::config::{BootloaderConfig, Mapping};

pub static BOOTLOADER_CONFIG: BootloaderConfig = {
    let mut config = BootloaderConfig::new_default();
    config.mappings.physical_memory = Some(Mapping::Dynamic);
    config
};

pub static PHYSICAL_MEMORY_OFFSET: OnceCell<x86_64::VirtAddr> = OnceCell::uninit();
pub static KERNEL_STACK: OnceCell<BootStack> = OnceCell::uninit();

pub trait Stack {
    fn top(&self) -> VirtAddr;
    fn bottom(&self) -> VirtAddr;
}

#[derive(Copy, Clone)]
pub struct BootStack {
    pub start: VirtAddr,
    pub end: VirtAddr,
}

impl BootStack {
    pub const fn new(start: VirtAddr, end: VirtAddr) -> Self {
        Self { start, end }
    }
}

impl Stack for BootStack {
    fn top(&self) -> VirtAddr {
        self.end - 16u64
    }

    fn bottom(&self) -> VirtAddr {
        self.start
    }
}

pub fn init(physical_memory_offset: x86_64::VirtAddr) {
    // Initialize the physical memory offset
    PHYSICAL_MEMORY_OFFSET.init_once(|| physical_memory_offset);

    interrupts::init_idt();
    gdt::init();

    #[cfg(not(uefi))]
    unsafe {
        interrupts::PICS.lock().initialize()
    };

    // Disable interrupts to prevent switching to processes before they are initialized
    x86_64::instructions::interrupts::disable();
}

pub fn hlt_loop() -> ! {
    loop {
        x86_64::instructions::hlt();
    }
}

pub trait Testable {
    fn run(&self) -> ();
}

impl<T> Testable for T
where
    T: Fn(),
{
    fn run(&self) {
        serial_print!("{}...\t", core::any::type_name::<T>());
        self();
        serial_println!("[ok]");
    }
}

pub fn test_runner(tests: &[&dyn Testable]) {
    serial_println!("Running {} tests", tests.len());
    for test in tests {
        test.run();
    }
    exit_qemu(QemuExitCode::Success);
}

pub fn test_panic_handler(info: &PanicInfo) -> ! {
    serial_println!("[failed]\n");
    serial_println!("Error: {}\n", info);
    exit_qemu(QemuExitCode::Failed);
    hlt_loop();
}

#[cfg(test)]
entry_point!(test_kernel_main, config = &BOOTLOADER_CONFIG);

/// Entry point for `cargo test`
#[cfg(test)]
fn test_kernel_main(boot_info: &'static mut BootInfo) -> ! {
    use x86_64::VirtAddr;

    init(VirtAddr::new(
        boot_info.physical_memory_offset.into_option().unwrap(),
    ));
    test_main();
    hlt_loop();
}

#[cfg(test)]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    test_panic_handler(info)
}

async fn async_number() -> u32 {
    42
}

pub async fn example_task() {
    let number = async_number().await;
    crate::println!("async number: {}", number);
}
