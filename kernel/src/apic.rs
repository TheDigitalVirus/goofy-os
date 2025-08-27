use acpi::{AcpiTables, Handle, Handler, PciAddress, aml::AmlError, platform::InterruptModel};
use x86_64::{VirtAddr, instructions::port::Port};

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

impl Handler for MyHandler {
    unsafe fn map_physical_region<T>(
        &self,
        physical_address: usize,
        size: usize,
    ) -> acpi::PhysicalMapping<Self, T> {
        let virtual_address = self.physical_memory_offset.as_u64() + physical_address as u64;
        let ptr = virtual_address as *mut T;

        acpi::PhysicalMapping {
            physical_start: physical_address,
            virtual_start: core::ptr::NonNull::new(ptr).unwrap(),
            region_length: size,
            mapped_length: size,
            handler: self.clone(),
        }
    }

    fn unmap_physical_region<T>(_region: &acpi::PhysicalMapping<Self, T>) {
        // No-op for our simple implementation
    }

    // Memory read/write operations
    fn read_u8(&self, address: usize) -> u8 {
        let virtual_address = self.physical_memory_offset.as_u64() + address as u64;
        unsafe { core::ptr::read_volatile(virtual_address as *const u8) }
    }

    fn read_u16(&self, address: usize) -> u16 {
        let virtual_address = self.physical_memory_offset.as_u64() + address as u64;
        unsafe { core::ptr::read_volatile(virtual_address as *const u16) }
    }

    fn read_u32(&self, address: usize) -> u32 {
        let virtual_address = self.physical_memory_offset.as_u64() + address as u64;
        unsafe { core::ptr::read_volatile(virtual_address as *const u32) }
    }

    fn read_u64(&self, address: usize) -> u64 {
        let virtual_address = self.physical_memory_offset.as_u64() + address as u64;
        unsafe { core::ptr::read_volatile(virtual_address as *const u64) }
    }

    fn write_u8(&self, address: usize, value: u8) {
        let virtual_address = self.physical_memory_offset.as_u64() + address as u64;
        unsafe { core::ptr::write_volatile(virtual_address as *mut u8, value) }
    }

    fn write_u16(&self, address: usize, value: u16) {
        let virtual_address = self.physical_memory_offset.as_u64() + address as u64;
        unsafe { core::ptr::write_volatile(virtual_address as *mut u16, value) }
    }

    fn write_u32(&self, address: usize, value: u32) {
        let virtual_address = self.physical_memory_offset.as_u64() + address as u64;
        unsafe { core::ptr::write_volatile(virtual_address as *mut u32, value) }
    }

    fn write_u64(&self, address: usize, value: u64) {
        let virtual_address = self.physical_memory_offset.as_u64() + address as u64;
        unsafe { core::ptr::write_volatile(virtual_address as *mut u64, value) }
    }

    // I/O port operations
    fn read_io_u8(&self, port: u16) -> u8 {
        unsafe {
            let mut port = Port::new(port);
            port.read()
        }
    }

    fn read_io_u16(&self, port: u16) -> u16 {
        unsafe {
            let mut port = Port::new(port);
            port.read()
        }
    }

    fn read_io_u32(&self, port: u16) -> u32 {
        unsafe {
            let mut port = Port::new(port);
            port.read()
        }
    }

    fn write_io_u8(&self, port: u16, value: u8) {
        unsafe {
            let mut port = Port::new(port);
            port.write(value);
        }
    }

    fn write_io_u16(&self, port: u16, value: u16) {
        unsafe {
            let mut port = Port::new(port);
            port.write(value);
        }
    }

    fn write_io_u32(&self, port: u16, value: u32) {
        unsafe {
            let mut port = Port::new(port);
            port.write(value);
        }
    }

    // PCI operations - stub implementations
    fn read_pci_u8(&self, _address: PciAddress, _offset: u16) -> u8 {
        0 // Stub implementation
    }

    fn read_pci_u16(&self, _address: PciAddress, _offset: u16) -> u16 {
        0 // Stub implementation
    }

    fn read_pci_u32(&self, _address: PciAddress, _offset: u16) -> u32 {
        0 // Stub implementation
    }

    fn write_pci_u8(&self, _address: PciAddress, _offset: u16, _value: u8) {
        // Stub implementation
    }

    fn write_pci_u16(&self, _address: PciAddress, _offset: u16, _value: u16) {
        // Stub implementation
    }

    fn write_pci_u32(&self, _address: PciAddress, _offset: u16, _value: u32) {
        // Stub implementation
    }

    // Time and synchronization operations - stub implementations
    fn nanos_since_boot(&self) -> u64 {
        0 // Stub implementation
    }

    fn stall(&self, _microseconds: u64) {
        // Stub implementation - could use a busy loop or integrate with the timer
    }

    fn sleep(&self, _nanoseconds: u64) {
        // Stub implementation - would yield to scheduler in a real implementation
    }

    // Mutex operations - stub implementations for single-threaded OS // TODO
    fn create_mutex(&self) -> Handle {
        Handle(0) // Stub implementation
    }

    fn acquire(&self, _handle: Handle, _timeout: u16) -> Result<(), AmlError> {
        Ok(()) // Stub implementation
    }

    fn release(&self, _handle: Handle) {
        // Stub implementation
    }
}

pub fn parse_acpi(
    rsdp_addr: usize,
    physical_memory_offset: VirtAddr,
) -> acpi::platform::InterruptModel {
    let handler = MyHandler::new(physical_memory_offset);
    let acpi_tables = unsafe { AcpiTables::from_rsdp(handler, rsdp_addr).unwrap() };
    let (model, _) = InterruptModel::new(&acpi_tables).unwrap();

    model
}
