#![no_std]
#![cfg_attr(test, no_main)]
#![feature(abi_x86_interrupt)]
#![feature(custom_test_frameworks)]
#![test_runner(crate::test_runner)]
#![reexport_test_harness_main = "test_main"]
#![feature(const_slice_make_iter)]
#![allow(incomplete_features)]
#![feature(generic_const_exprs)]
#![allow(static_mut_refs)]

#[cfg(test)]
use bootloader_api::{BootInfo, entry_point};
use conquer_once::spin::OnceCell;
use raw_cpuid::CpuId;
use x86_64::{
    VirtAddr,
    registers::control::{Cr0, Cr0Flags, Cr3, Cr4, Cr4Flags},
};

use core::arch::asm;
use core::panic::PanicInfo;
use exit::{QemuExitCode, exit_qemu};

#[cfg(processes_enabled)]
use crate::gdt::GDT;

#[cfg(processes_enabled)]
use x86_64::registers::{
    control::{Efer, EferFlags},
    model_specific::{LStar, SFMask, Star},
    rflags::RFlags,
};

extern crate alloc;

pub mod allocator;
pub mod apic;
pub mod desktop;
pub mod errno;
pub mod exit;
pub mod framebuffer;
pub mod fs;
pub mod gdt;
pub mod interrupts;
pub mod irq;
pub mod memory;
pub mod serial;
pub mod surface;
pub mod sysinfo;
pub mod time;
pub mod user_program_loader;

#[cfg(processes_enabled)]
pub mod syscalls;
#[cfg(processes_enabled)]
pub mod tasks;
#[cfg(processes_enabled)]
use tasks::syscall::syscall_handler;

use bootloader_api::config::{BootloaderConfig, Mapping};

pub static BOOTLOADER_CONFIG: BootloaderConfig = {
    let mut config = BootloaderConfig::new_default();
    config.mappings.physical_memory = Some(Mapping::Dynamic);
    config
};

pub static PHYSICAL_MEMORY_OFFSET: OnceCell<x86_64::VirtAddr> = OnceCell::uninit();
pub static KERNEL_STACK: OnceCell<BootStack> = OnceCell::uninit();

pub const STACK_SIZE: usize = 1024 * 16; // 16 KB
pub const INTERRUPT_STACK_SIZE: usize = STACK_SIZE; // Idk, just use the same size for now

pub const BOOT_IST_STACK: ([u8; INTERRUPT_STACK_SIZE],) = ([0; INTERRUPT_STACK_SIZE],);

/// Search the least significant bit
#[inline(always)]
#[allow(dead_code)]
pub(crate) fn lsb(i: usize) -> usize {
    let ret;

    if i == 0 {
        ret = !0usize;
    } else {
        unsafe {
            asm!("bsf {0}, {1}",
                lateout(reg) ret,
                in(reg) i,
                options(nomem, nostack)
            );
        }
    }

    ret
}

/// Search the most significant bit
#[inline(always)]
#[allow(dead_code)]
pub(crate) fn msb(value: usize) -> Option<usize> {
    if value > 0 {
        let ret: usize;

        unsafe {
            asm!("bsr {0}, {1}",
                out(reg) ret,
                in(reg) value,
                options(nomem, nostack)
            );
        }
        Some(ret)
    } else {
        None
    }
}

pub trait Stack {
    fn top(&self) -> VirtAddr;
    fn bottom(&self) -> VirtAddr;
    fn interrupt_top(&self) -> VirtAddr;
    fn interrupt_bottom(&self) -> VirtAddr;
}

#[derive(Copy, Clone)]
pub struct BootStack {
    pub start: VirtAddr,
    pub end: VirtAddr,
    pub ist_start: VirtAddr,
    pub ist_end: VirtAddr,
}

impl BootStack {
    pub const fn new(
        start: VirtAddr,
        end: VirtAddr,
        ist_start: VirtAddr,
        ist_end: VirtAddr,
    ) -> Self {
        Self {
            start,
            end,
            ist_start,
            ist_end,
        }
    }
}

impl Stack for BootStack {
    fn top(&self) -> VirtAddr {
        self.end - 16u64
    }

    fn bottom(&self) -> VirtAddr {
        self.start
    }

    fn interrupt_top(&self) -> VirtAddr {
        self.ist_end - 16u64
    }

    fn interrupt_bottom(&self) -> VirtAddr {
        self.ist_start
    }
}

pub fn init(physical_memory_offset: x86_64::VirtAddr) {
    // Initialize the physical memory offset
    PHYSICAL_MEMORY_OFFSET.init_once(|| physical_memory_offset);

    interrupts::init_idt();
    gdt::init();

    unsafe {
        Cr0::update(|cr0| {
            *cr0 |= Cr0Flags::ALIGNMENT_MASK;
            *cr0 |= Cr0Flags::NUMERIC_ERROR;
            *cr0 |= Cr0Flags::MONITOR_COPROCESSOR;
            // enable cache
            *cr0 &= !(Cr0Flags::CACHE_DISABLE | Cr0Flags::NOT_WRITE_THROUGH);
        });

        let cpuid = CpuId::new();

        Cr4::update(|cr4| {
            // disable performance monitoring counter
            // allow the usage of rdtsc in user space
            *cr4 &= !(Cr4Flags::PERFORMANCE_MONITOR_COUNTER | Cr4Flags::TIMESTAMP_DISABLE);

            let has_pge = match cpuid.get_feature_info() {
                Some(finfo) => finfo.has_pge(),
                None => false,
            };

            if has_pge {
                *cr4 |= Cr4Flags::PAGE_GLOBAL; // enable global pages
            }

            let has_fsgsbase = match cpuid.get_extended_feature_info() {
                Some(efinfo) => efinfo.has_fsgsbase(),
                None => false,
            };

            if has_fsgsbase {
                *cr4 |= Cr4Flags::FSGSBASE;
            }

            let has_mce = match cpuid.get_feature_info() {
                Some(finfo) => finfo.has_mce(),
                None => false,
            };

            if has_mce {
                *cr4 |= Cr4Flags::MACHINE_CHECK_EXCEPTION; // enable machine check exceptions
            }
        });
    };

    #[cfg(processes_enabled)]
    // Enable syscalls
    unsafe {
        serial_println!("User code segment: {:#x}", GDT.1.user_code.0);
        serial_println!("User data segment: {:#x}", GDT.1.user_data.0);
        serial_println!("Kernel code segment: {:#x}", GDT.1.code.0);
        serial_println!("Kernel data segment: {:#x}", GDT.1.data.0);

        Efer::update(|e| *e |= EferFlags::SYSTEM_CALL_EXTENSIONS);
        LStar::write(VirtAddr::new(syscall_handler as u64));
        SFMask::write(RFlags::INTERRUPT_FLAG);

        match Star::write(GDT.1.user_code, GDT.1.user_data, GDT.1.code, GDT.1.data) {
            Ok(()) => serial_println!("Star MSRs written successfully"),
            Err(e) => panic!("Failed to write Star MSRs: {}", e),
        }
    }

    // dirty hack for the demo, only necessary for qemu
    // => enable access for the user space
    if Cr3::read_raw().1 == 0x1000 {
        serial_println!("Applying page table hack for QEMU");

        let p0 = unsafe { core::slice::from_raw_parts_mut(0x3000 as *mut usize, 512) };
        for entry in p0 {
            if *entry != 0 {
                *entry |= 1 << 2;
            }
        }

        unsafe {
            // flush tlb
            Cr3::write_raw(Cr3::read_raw().0, Cr3::read_raw().1);
        }
    }

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
