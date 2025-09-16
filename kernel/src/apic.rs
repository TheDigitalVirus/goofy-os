use acpi::{AcpiHandler, AcpiTables, InterruptModel};
use lazy_static::lazy_static;
use spin::Mutex;
use x86_64::{
    PhysAddr, VirtAddr,
    structures::paging::{FrameAllocator, Mapper, PhysFrame, Size4KiB},
};

use crate::{interrupts::InterruptIndex, serial_println};

#[derive(Clone)]
struct MyHandler {
    physical_memory_offset: VirtAddr,
}

impl MyHandler {
    pub fn new(physical_memory_offset: VirtAddr) -> Self {
        MyHandler {
            physical_memory_offset,
        }
    }
}

impl AcpiHandler for MyHandler {
    unsafe fn map_physical_region<T>(
        &self,
        physical_address: usize,
        size: usize,
    ) -> acpi::PhysicalMapping<Self, T> {
        let virtual_address = self.physical_memory_offset.as_u64() + physical_address as u64;
        let ptr = virtual_address as *mut T;

        unsafe {
            acpi::PhysicalMapping::new(
                physical_address,
                core::ptr::NonNull::new(ptr).unwrap(),
                size,
                size,
                self.clone(),
            )
        }
    }

    fn unmap_physical_region<T>(_region: &acpi::PhysicalMapping<Self, T>) {
        // No-op for our simple implementation
    }
}

pub fn parse_acpi(rsdp_addr: usize, physical_memory_offset: VirtAddr) -> acpi::InterruptModel {
    let handler = MyHandler::new(physical_memory_offset);
    let acpi_tables = unsafe { AcpiTables::from_rsdp(handler, rsdp_addr).unwrap() };
    let platform = acpi_tables.platform_info().unwrap();

    platform.interrupt_model
}

lazy_static! {
    static ref LAPIC_ADDR: Mutex<LAPICAddress> = Mutex::new(LAPICAddress::new());
}

// Credits to u/xcompute, this is based on their work

// https://wiki.osdev.org/APIC
#[allow(non_camel_case_types)]
#[derive(Debug, Clone, Copy)]
#[repr(isize)]
#[allow(dead_code)]
pub enum APICOffset {
    R0x00 = 0x0,      // RESERVED = 0x00
    R0x10 = 0x10,     // RESERVED = 0x10
    Ir = 0x20,        // ID Register
    Vr = 0x30,        // Version Register
    R0x40 = 0x40,     // RESERVED = 0x40
    R0x50 = 0x50,     // RESERVED = 0x50
    R0x60 = 0x60,     // RESERVED = 0x60
    R0x70 = 0x70,     // RESERVED = 0x70
    Tpr = 0x80,       // Text Priority Register
    Apr = 0x90,       // Arbitration Priority Register
    Ppr = 0xA0,       // Processor Priority Register
    Eoi = 0xB0,       // End of Interrupt
    Rrd = 0xC0,       // Remote Read Register
    Ldr = 0xD0,       // Logical Destination Register
    Dfr = 0xE0,       // DFR
    Svr = 0xF0,       // Spurious (Interrupt) Vector Register
    Isr1 = 0x100,     // In-Service Register 1
    Isr2 = 0x110,     // In-Service Register 2
    Isr3 = 0x120,     // In-Service Register 3
    Isr4 = 0x130,     // In-Service Register 4
    Isr5 = 0x140,     // In-Service Register 5
    Isr6 = 0x150,     // In-Service Register 6
    Isr7 = 0x160,     // In-Service Register 7
    Isr8 = 0x170,     // In-Service Register 8
    Tmr1 = 0x180,     // Trigger Mode Register 1
    Tmr2 = 0x190,     // Trigger Mode Register 2
    Tmr3 = 0x1A0,     // Trigger Mode Register 3
    Tmr4 = 0x1B0,     // Trigger Mode Register 4
    Tmr5 = 0x1C0,     // Trigger Mode Register 5
    Tmr6 = 0x1D0,     // Trigger Mode Register 6
    Tmr7 = 0x1E0,     // Trigger Mode Register 7
    Tmr8 = 0x1F0,     // Trigger Mode Register 8
    Irr1 = 0x200,     // Interrupt Request Register 1
    Irr2 = 0x210,     // Interrupt Request Register 2
    Irr3 = 0x220,     // Interrupt Request Register 3
    Irr4 = 0x230,     // Interrupt Request Register 4
    Irr5 = 0x240,     // Interrupt Request Register 5
    Irr6 = 0x250,     // Interrupt Request Register 6
    Irr7 = 0x260,     // Interrupt Request Register 7
    Irr8 = 0x270,     // Interrupt Request Register 8
    Esr = 0x280,      // Error Status Register
    R0x290 = 0x290,   // RESERVED = 0x290
    R0x2A0 = 0x2A0,   // RESERVED = 0x2A0
    R0x2B0 = 0x2B0,   // RESERVED = 0x2B0
    R0x2C0 = 0x2C0,   // RESERVED = 0x2C0
    R0x2D0 = 0x2D0,   // RESERVED = 0x2D0
    R0x2E0 = 0x2E0,   // RESERVED = 0x2E0
    LvtCmci = 0x2F0,  // LVT Corrected Machine Check Interrupt (CMCI) Register
    Icr1 = 0x300,     // Interrupt Command Register 1
    Icr2 = 0x310,     // Interrupt Command Register 2
    LvtT = 0x320,     // LVT Timer Register
    LvtTsr = 0x330,   // LVT Thermal Sensor Register
    LvtPmcr = 0x340,  // LVT Performance Monitoring Counters Register
    LvtLint0 = 0x350, // LVT LINT0 Register
    LvtLint1 = 0x360, // LVT LINT1 Register
    LvtE = 0x370,     // LVT Error Register
    Ticr = 0x380,     // Initial Count Register (for Timer)
    Tccr = 0x390,     // Current Count Register (for Timer)
    R0x3A0 = 0x3A0,   // RESERVED = 0x3A0
    R0x3B0 = 0x3B0,   // RESERVED = 0x3B0
    R0x3C0 = 0x3C0,   // RESERVED = 0x3C0
    R0x3D0 = 0x3D0,   // RESERVED = 0x3D0
    Tdcr = 0x3E0,     // Divide Configuration Register (for Timer)
    R0x3F0 = 0x3F0,   // RESERVED = 0x3F0
}

pub struct LAPICAddress {
    address: *mut u32,
}

unsafe impl Send for LAPICAddress {}
unsafe impl Sync for LAPICAddress {}

impl LAPICAddress {
    pub fn new() -> Self {
        Self {
            address: core::ptr::null_mut(),
        }
    }
}

unsafe fn init_local_apic(
    local_apic_addr: usize,
    mapper: &mut impl Mapper<Size4KiB>,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
) {
    let virtual_address = map_apic(local_apic_addr as u64, mapper, frame_allocator);

    let lapic_pointer = virtual_address.as_mut_ptr::<u32>();
    LAPIC_ADDR.lock().address = lapic_pointer;

    unsafe {
        init_timer(lapic_pointer);
        init_keyboard(lapic_pointer);
        init_mouse(lapic_pointer);
    };
}

unsafe fn init_timer(lapic_pointer: *mut u32) {
    let svr = unsafe { lapic_pointer.offset(APICOffset::Svr as isize / 4) };
    unsafe { svr.write_volatile(svr.read_volatile() | 0x100) };

    // Configure timer
    // Vector 0x20, Periodic Mode (bit 17), Not masked (bit 16 = 0)
    let lvt_timer = unsafe { lapic_pointer.offset(APICOffset::LvtT as isize / 4) };
    unsafe { lvt_timer.write_volatile(0x20 | (1 << 17)) };

    // Set divider to 16
    let tdcr = unsafe { lapic_pointer.offset(APICOffset::Tdcr as isize / 4) };
    unsafe { tdcr.write_volatile(0x3) };

    // Set initial count - smaller value for more frequent interrupts
    let ticr = unsafe { lapic_pointer.offset(APICOffset::Ticr as isize / 4) };
    unsafe { ticr.write_volatile(2_500_000) }; // Very slow // TODO: Spead this up when more processes

    serial_println!("Local APIC timer initialized");
}

unsafe fn init_keyboard(lapic_pointer: *mut u32) {
    let keyboard_register = unsafe { lapic_pointer.offset(APICOffset::LvtLint1 as isize / 4) };
    unsafe { keyboard_register.write_volatile(InterruptIndex::Keyboard as u8 as u32) };
}

unsafe fn init_mouse(lapic_pointer: *mut u32) {
    // Configure LINT0 for mouse interrupts (IRQ 12)
    let mouse_register = unsafe { lapic_pointer.offset(APICOffset::LvtLint0 as isize / 4) };
    unsafe { mouse_register.write_volatile(InterruptIndex::Mouse as u8 as u32) };
}

unsafe fn init_io_apic(
    ioapic_address: usize,
    mapper: &mut impl Mapper<Size4KiB>,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
) {
    let virt_addr = map_apic(ioapic_address as u64, mapper, frame_allocator);

    let ioapic_pointer = virt_addr.as_mut_ptr::<u32>();

    // Configure keyboard interrupt (IRQ 1 -> interrupt vector 33)
    unsafe {
        // IRQ 1 uses redirection entry 1: registers 0x12 (low) and 0x13 (high)
        ioapic_pointer.offset(0).write_volatile(0x12); // Select keyboard redirection entry low
        ioapic_pointer
            .offset(4)
            .write_volatile(InterruptIndex::Keyboard as u8 as u32); // Vector + delivery mode (fixed=000)

        ioapic_pointer.offset(0).write_volatile(0x13); // Select keyboard redirection entry high
        ioapic_pointer.offset(4).write_volatile(0); // Destination (CPU 0)
    }

    // Configure mouse interrupt (IRQ 12 -> interrupt vector 44)
    unsafe {
        // IRQ 12 uses redirection entry 12: registers 0x28 (low) and 0x29 (high)
        ioapic_pointer.offset(0).write_volatile(0x28); // Select mouse redirection entry low (0x10 + 12*2)
        ioapic_pointer
            .offset(4)
            .write_volatile(InterruptIndex::Mouse as u8 as u32); // Vector + delivery mode (fixed=000)

        ioapic_pointer.offset(0).write_volatile(0x29); // Select mouse redirection entry high
        ioapic_pointer.offset(4).write_volatile(0); // Destination (CPU 0)
    }
}

fn map_apic(
    physical_address: u64,
    mapper: &mut impl Mapper<Size4KiB>,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
) -> VirtAddr {
    use x86_64::structures::paging::Page;
    use x86_64::structures::paging::PageTableFlags as Flags;

    let physical_address = PhysAddr::new(physical_address);
    let page = Page::containing_address(VirtAddr::new(physical_address.as_u64()));
    let frame = PhysFrame::containing_address(physical_address);

    let flags = Flags::PRESENT | Flags::WRITABLE | Flags::NO_CACHE;

    unsafe {
        mapper
            .map_to(page, frame, flags, frame_allocator)
            .expect("APIC mapping failed")
            .flush();
    }

    page.start_address()
}

pub fn end_interrupt() {
    unsafe {
        let lapic_ptr = LAPIC_ADDR.lock().address;
        lapic_ptr
            .offset(APICOffset::Eoi as isize / 4)
            .write_volatile(0);
    }
}

pub unsafe fn init(
    rsdp_addr: usize,
    physical_memory_offset: VirtAddr,
    page_table: &mut impl Mapper<Size4KiB>,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
) {
    let model = parse_acpi(rsdp_addr, physical_memory_offset);

    match model {
        InterruptModel::Apic(apic) => {
            let io_apic_address = apic.io_apics[0].address;
            unsafe {
                init_io_apic(
                    io_apic_address as usize,
                    &mut *page_table,
                    &mut *frame_allocator,
                )
            };

            let local_apic_address = apic.local_apic_address;
            unsafe {
                init_local_apic(
                    local_apic_address as usize,
                    &mut *page_table,
                    &mut *frame_allocator,
                )
            };
        }
        _ => panic!("Unsupported interrupt model"),
    }
}
