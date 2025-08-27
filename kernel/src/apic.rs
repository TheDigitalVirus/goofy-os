use acpi::{AcpiHandler, AcpiTables};
use x86_64::VirtAddr;

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
        // No-op for our simple implementation since we don't need to unmap
        // the physical memory mapping in our OS design
    }
}

pub fn parse_acpi(rsdp_addr: usize, physical_memory_offset: VirtAddr) -> acpi::InterruptModel {
    let handler = MyHandler::new(physical_memory_offset);
    let acpi_tables = unsafe { AcpiTables::from_rsdp(handler, rsdp_addr).unwrap() };
    let platform_info = acpi_tables.platform_info().unwrap();
    platform_info.interrupt_model
}
