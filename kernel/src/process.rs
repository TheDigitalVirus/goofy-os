use core::arch::asm;

use alloc::vec::Vec;
use lazy_static::lazy_static;
use spin::Mutex;
use x86_64::{
    VirtAddr,
    instructions::interrupts::without_interrupts,
    structures::{
        idt::InterruptStackFrame,
        paging::{FrameAllocator, PageTableFlags, PhysFrame, Size4KiB},
    },
};

use crate::{
    memory::{BootInfoFrameAllocator, ProcessAddressSpace},
    serial_println,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    Ready,
    Running,
    Terminated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessType {
    User,
    Kernel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessError {
    OutOfMemory,
    InvalidProgram,
    InvalidStateTransition,
    InvalidInstructionPointer,
    InvalidStackPointer,
}

/// Memory validation functions to prevent page faults during context switching
impl ProcessError {
    /// Validate that a virtual address is properly aligned and within reasonable bounds
    fn validate_virtual_address(addr: VirtAddr, name: &str) -> Result<(), ProcessError> {
        let addr_u64 = addr.as_u64();

        // Check for null pointer
        if addr_u64 == 0 {
            serial_println!("Invalid {}: null pointer", name);
            return Err(if name.contains("stack") {
                ProcessError::InvalidStackPointer
            } else {
                ProcessError::InvalidInstructionPointer
            });
        }

        // Check alignment - stacks should be 16-byte aligned, code can be 1-byte aligned
        if name.contains("stack") && (addr_u64 % 16) != 0 {
            serial_println!("Invalid {}: not 16-byte aligned (0x{:x})", name, addr_u64);
            return Err(ProcessError::InvalidStackPointer);
        }

        // Check if address is in valid range (not in kernel space for user processes)
        if name.contains("user") && addr_u64 >= 0xFFFF800000000000 {
            serial_println!(
                "Invalid {}: address in kernel space (0x{:x})",
                name,
                addr_u64
            );
            return Err(if name.contains("stack") {
                ProcessError::InvalidStackPointer
            } else {
                ProcessError::InvalidInstructionPointer
            });
        }

        Ok(())
    }

    /// Validate that a stack pointer is within reasonable bounds for the given process type
    fn validate_stack_pointer(sp: VirtAddr, process_type: ProcessType) -> Result<(), ProcessError> {
        let sp_u64 = sp.as_u64();

        match process_type {
            ProcessType::User => {
                Self::validate_virtual_address(sp, "user stack pointer")?;

                // User stack should be in user space (below kernel space)
                if sp_u64 >= 0xFFFF800000000000 {
                    serial_println!("User stack pointer in kernel space: 0x{:x}", sp_u64);
                    return Err(ProcessError::InvalidStackPointer);
                }

                // User stack should be above 0x1000 to avoid null dereferences
                if sp_u64 < 0x1000 {
                    serial_println!("User stack pointer too low: 0x{:x}", sp_u64);
                    return Err(ProcessError::InvalidStackPointer);
                }
            }
            ProcessType::Kernel => {
                Self::validate_virtual_address(sp, "kernel stack pointer")?;

                // Kernel stack can be anywhere in virtual memory, but should be reasonable
                // Check that it's not obviously invalid
                if sp_u64 < 0x1000 {
                    serial_println!("Kernel stack pointer too low: 0x{:x}", sp_u64);
                    return Err(ProcessError::InvalidStackPointer);
                }
            }
        }

        Ok(())
    }

    /// Validate that an instruction pointer is reasonable for the given process type
    fn validate_instruction_pointer(
        ip: VirtAddr,
        process_type: ProcessType,
    ) -> Result<(), ProcessError> {
        let ip_u64 = ip.as_u64();

        match process_type {
            ProcessType::User => {
                Self::validate_virtual_address(ip, "user instruction pointer")?;

                // User code should be in user space
                if ip_u64 >= 0xFFFF800000000000 {
                    serial_println!("User instruction pointer in kernel space: 0x{:x}", ip_u64);
                    return Err(ProcessError::InvalidInstructionPointer);
                }

                // Should be above 0x1000 to avoid null dereferences
                if ip_u64 < 0x1000 {
                    serial_println!("User instruction pointer too low: 0x{:x}", ip_u64);
                    return Err(ProcessError::InvalidInstructionPointer);
                }
            }
            ProcessType::Kernel => {
                Self::validate_virtual_address(ip, "kernel instruction pointer")?;

                // Kernel code should be in kernel space or at reasonable addresses
                if ip_u64 < 0x1000 {
                    serial_println!("Kernel instruction pointer too low: 0x{:x}", ip_u64);
                    return Err(ProcessError::InvalidInstructionPointer);
                }
            }
        }

        Ok(())
    }

    /// Validate that RFLAGS has reasonable values
    fn validate_rflags(rflags: u64) -> Result<(), ProcessError> {
        // RFLAGS bit 1 should always be set (reserved bit)
        if (rflags & 0x2) == 0 {
            serial_println!("Invalid RFLAGS: reserved bit 1 not set (0x{:x})", rflags);
            return Err(ProcessError::InvalidStateTransition);
        }

        // Check that no reserved bits are set that shouldn't be
        let reserved_mask = 0xFFC08028; // Bits that should be zero
        if (rflags & reserved_mask) != 0 {
            serial_println!("Invalid RFLAGS: reserved bits set (0x{:x})", rflags);
            return Err(ProcessError::InvalidStateTransition);
        }

        Ok(())
    }

    /// Validate that a page table frame is valid
    fn validate_page_table_frame(frame: PhysFrame<Size4KiB>) -> Result<(), ProcessError> {
        let frame_addr = frame.start_address().as_u64();

        // Check alignment
        if (frame_addr % 4096) != 0 {
            serial_println!("Page table frame not page-aligned: 0x{:x}", frame_addr);
            return Err(ProcessError::OutOfMemory);
        }

        // Check that it's not null
        if frame_addr == 0 {
            serial_println!("Page table frame is null");
            return Err(ProcessError::OutOfMemory);
        }

        // TODO: Check that frame is within valid physical memory range
        // This would require access to the memory map

        Ok(())
    }
}

#[derive(Clone, Copy)]
pub struct Process {
    pub pid: u32,
    pub state: ProcessState,
    pub process_type: ProcessType,
    pub address_space: ProcessAddressSpace,
    pub stack_pointer: VirtAddr,
    pub instruction_pointer: VirtAddr,
    // Saved register state
    pub registers: RegisterState,
    // Flag to track if this process has valid saved register state
    pub has_saved_state: bool,
}

impl Process {
    /// Validate the process state to ensure safe context switching
    pub fn validate(&self) -> Result<(), ProcessError> {
        // Validate stack pointer
        ProcessError::validate_stack_pointer(self.stack_pointer, self.process_type)?;

        // Validate instruction pointer
        ProcessError::validate_instruction_pointer(self.instruction_pointer, self.process_type)?;

        // Validate page table frame for user processes
        if self.process_type == ProcessType::User {
            ProcessError::validate_page_table_frame(self.address_space.page_table_frame)?;
        }

        // Validate saved register state if it exists
        if self.has_saved_state {
            ProcessError::validate_rflags(self.registers.rflags)?;
            ProcessError::validate_stack_pointer(
                VirtAddr::new(self.registers.rsp),
                self.process_type,
            )?;
            ProcessError::validate_instruction_pointer(
                VirtAddr::new(self.registers.rip),
                self.process_type,
            )?;
        }

        serial_println!("Process {} validation passed", self.pid);
        Ok(())
    }

    pub fn cleanup_resources(&mut self) {
        // Clean up any resources associated with the process
        self.state = ProcessState::Terminated;

        self.address_space.cleanup();

        serial_println!("Cleaning up resources for process with PID {}", self.pid);

        // TODO: Clean up any other resources
    }
}

#[repr(C, align(8))]
#[derive(Clone, Copy, Debug)]
pub struct RegisterState {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rbp: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rbx: u64,
    pub rax: u64,
    pub rip: u64,
    pub rflags: u64,
    pub rsp: u64,
}

impl RegisterState {
    /// Create a new register state with default values for a new process
    pub fn new() -> Self {
        Self {
            r15: 0,
            r14: 0,
            r13: 0,
            r12: 0,
            r11: 0,
            r10: 0,
            r9: 0,
            r8: 0,
            rsi: 0,
            rdi: 0,
            rbp: 0,
            rdx: 0,
            rcx: 0,
            rbx: 0,
            rax: 0,
            rip: 0,
            rflags: 0x202, // Default RFLAGS with interrupts enabled
            rsp: 0,
        }
    }
}

pub struct ProcessManager {
    processes: Vec<Process>,
    current_pid: u32,
    next_pid: u32,
    kernel_cr3: u64,
}

impl ProcessManager {
    pub fn new() -> Self {
        let kernel_cr3: u64;
        unsafe {
            asm!("mov {}, cr3", out(reg) kernel_cr3);
        }
        serial_println!("Kernel CR3: 0x{:x}", kernel_cr3);

        Self {
            processes: Vec::new(),
            current_pid: 0,
            next_pid: 1,
            kernel_cr3,
        }
    }

    pub fn create_process(
        &mut self,
        binary: &[u8],
        frame_allocator: &mut BootInfoFrameAllocator,
        physical_memory_offset: VirtAddr,
    ) -> Result<u32, ProcessError> {
        serial_println!(
            "Creating process with binary data of {} bytes",
            binary.len()
        );

        // Parse the ELF binary
        let elf = goblin::elf::Elf::parse(binary).expect("Failed to parse ELF");

        // Create the address space first
        let mut address_space = ProcessAddressSpace::new(frame_allocator, physical_memory_offset)
            .map_err(|e| {
            serial_println!("Failed to create address space: {:?}", e);
            ProcessError::OutOfMemory
        })?;

        // Allocate a frame for the stack
        let stack_frame = frame_allocator.allocate_frame().ok_or_else(|| {
            serial_println!("Failed to allocate stack frame");
            ProcessError::OutOfMemory
        })?;

        // Map stack at 0x800000 (8MB mark)
        let stack_virtual_addr = VirtAddr::new(0x800000);
        address_space
            .map_user_memory(
                stack_virtual_addr,
                stack_frame.start_address(),
                0x1000, // 4KB stack
                PageTableFlags::PRESENT
                    | PageTableFlags::WRITABLE
                    | PageTableFlags::USER_ACCESSIBLE
                    | PageTableFlags::NO_EXECUTE,
                frame_allocator,
            )
            .map_err(|e| {
                serial_println!("Failed to map stack: {:?}", e);
                ProcessError::OutOfMemory
            })?;

        // Copy program data to the mapped memory through virtual memory
        for (i, ph) in elf.program_headers.iter().enumerate() {
            if ph.p_type != goblin::elf::program_header::PT_LOAD {
                serial_println!("Skipping non-loadable segment {}", i);
                continue;
            }

            let mem_start = ph.p_vaddr;
            let file_start = ph.p_offset as usize;
            let file_end = file_start + ph.p_filesz as usize;

            if file_end > binary.len() {
                serial_println!("Segment {} extends beyond binary data", i);
                return Err(ProcessError::InvalidProgram);
            }

            let segment_data = &binary[file_start..file_end];

            // Calculate how many pages we need for this segment
            let segment_virtual_addr = VirtAddr::new(mem_start & !0xfff); // Page-align the start address
            let segment_end_addr = mem_start + ph.p_memsz;
            let aligned_size = (segment_end_addr + 4095) & !0xfff - (mem_start & !0xfff); // Calculate aligned size
            let pages_needed = aligned_size / 4096;

            // Set appropriate flags based on ELF segment permissions
            let mut segment_flags = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE;
            if ph.p_flags & goblin::elf::program_header::PF_W != 0 {
                segment_flags |= PageTableFlags::WRITABLE;
            }
            if (ph.p_flags & goblin::elf::program_header::PF_X) == 0 {
                segment_flags |= PageTableFlags::NO_EXECUTE;
            }

            // Map each page for this segment
            for page_idx in 0..pages_needed {
                let page_virtual_addr = segment_virtual_addr + (page_idx * 4096);

                // Allocate frame for this page
                let page_frame = frame_allocator.allocate_frame().ok_or_else(|| {
                    serial_println!(
                        "Failed to allocate frame for segment {} page {}",
                        i,
                        page_idx
                    );
                    ProcessError::OutOfMemory
                })?;

                let mapping_result = address_space
                    .map_user_memory(
                        page_virtual_addr,
                        page_frame.start_address(),
                        4096,
                        segment_flags,
                        frame_allocator,
                    )
                    .map_err(|e| {
                        let error_string = alloc::format!("{:?}", e);
                        if error_string.contains("PageAlreadyMapped") {
                            // This is OK - segments can overlap at page boundaries
                            serial_println!(
                                "Page 0x{:x} already mapped, continuing (segment {} page {})",
                                page_virtual_addr.as_u64(),
                                i,
                                page_idx
                            );
                            return ProcessError::InvalidProgram; // Use a different error to handle this case
                        }
                        serial_println!("Failed to map segment {} page {}: {:?}", i, page_idx, e);
                        ProcessError::OutOfMemory
                    });

                // Handle the special case where the page was already mapped
                if let Err(ProcessError::InvalidProgram) = mapping_result {
                    // Page already mapped, skip mapping but continue with data copying
                    serial_println!(
                        "Skipping mapping for already mapped page, continuing with segment {}",
                        i
                    );
                } else {
                    // Check for other errors
                    mapping_result?;
                }

                // Copy segment data to this page if needed
                let page_offset = page_idx * 4096;
                let page_start_addr = segment_virtual_addr.as_u64() + page_offset;
                let original_segment_start = mem_start;
                let original_segment_end = original_segment_start + ph.p_filesz;

                // Calculate what part of this page should contain data
                let data_start_in_page = if page_start_addr < original_segment_start {
                    (original_segment_start - page_start_addr) as usize
                } else {
                    0
                };

                let data_end_in_page = if page_start_addr + 4096 > original_segment_end {
                    if original_segment_end > page_start_addr {
                        (original_segment_end - page_start_addr) as usize
                    } else {
                        0
                    }
                } else {
                    4096
                };

                if data_start_in_page < data_end_in_page {
                    let page_virtual_ptr = (physical_memory_offset
                        + page_frame.start_address().as_u64())
                    .as_mut_ptr::<u8>();

                    // Calculate offset in the source data
                    let src_offset = if page_start_addr >= original_segment_start {
                        (page_start_addr - original_segment_start) as usize
                    } else {
                        0
                    };

                    let copy_size = data_end_in_page - data_start_in_page;

                    if src_offset < segment_data.len() && copy_size > 0 {
                        let actual_copy_size =
                            core::cmp::min(copy_size, segment_data.len() - src_offset);
                        let data_to_copy = &segment_data[src_offset..src_offset + actual_copy_size];

                        unsafe {
                            // Zero out the entire page first
                            core::ptr::write_bytes(page_virtual_ptr, 0, 4096);

                            // Copy the actual data for this page
                            core::ptr::copy_nonoverlapping(
                                data_to_copy.as_ptr(),
                                page_virtual_ptr.add(data_start_in_page),
                                data_to_copy.len(),
                            );
                        }
                    } else {
                        // Zero the page if no data to copy
                        let page_virtual_ptr = (physical_memory_offset
                            + page_frame.start_address().as_u64())
                        .as_mut_ptr::<u8>();
                        unsafe {
                            core::ptr::write_bytes(page_virtual_ptr, 0, 4096);
                        }
                        serial_println!(
                            "Zeroed page {} of segment {} (no data to copy)",
                            page_idx,
                            i
                        );
                    }
                } else {
                    // This page is beyond the file data, just zero it
                    let page_virtual_ptr = (physical_memory_offset
                        + page_frame.start_address().as_u64())
                    .as_mut_ptr::<u8>();
                    unsafe {
                        core::ptr::write_bytes(page_virtual_ptr, 0, 4096);
                    }
                    serial_println!(
                        "Zeroed page {} of segment {} (beyond file data)",
                        page_idx,
                        i
                    );
                }
            }
        }

        let stack_pointer = stack_virtual_addr + 0x1000 - 16; // Stack grows downward, point to top of stack minus 16 bytes for proper 16-byte alignment
        let instruction_pointer = VirtAddr::new(elf.entry); // Start at ELF entry point

        // Validate the addresses before creating the process
        if let Err(e) = ProcessError::validate_stack_pointer(stack_pointer, ProcessType::User) {
            serial_println!("Invalid stack pointer for new process: {:?}", e);
            return Err(e);
        }

        if let Err(e) =
            ProcessError::validate_instruction_pointer(instruction_pointer, ProcessType::User)
        {
            serial_println!("Invalid instruction pointer for new process: {:?}", e);
            return Err(e);
        }

        serial_println!("Setting up process with PID {}", self.next_pid);
        serial_println!("Stack pointer will be at: {:?}", stack_pointer);
        serial_println!("Instruction pointer will be at: {:?}", instruction_pointer);

        let process = Process {
            pid: self.next_pid,
            state: ProcessState::Ready,
            process_type: ProcessType::User,
            address_space,
            stack_pointer,
            instruction_pointer,
            registers: {
                let mut regs = RegisterState::new();
                regs.rsp = stack_pointer.as_u64();
                regs.rip = instruction_pointer.as_u64();
                regs
            },
            has_saved_state: false,
        };

        // Final validation of the created process
        if let Err(e) = process.validate() {
            serial_println!("Created process failed validation: {:?}", e);
            return Err(e);
        }

        let pid = self.next_pid;
        // self.current_pid = pid;
        self.processes.push(process);
        self.next_pid += 1;
        Ok(pid)
    }

    pub fn create_kernel_process(
        &mut self,
        entry_point: VirtAddr,
        stack_ptr: VirtAddr,
    ) -> Result<u32, ProcessError> {
        serial_println!(
            "Creating kernel process with entry point: {:?}",
            entry_point
        );

        // Create a dummy address space for kernel process (it won't be used for page table switching)
        // For kernel processes, we'll use the kernel's page table frame (stored in kernel_cr3)
        let kernel_frame = x86_64::structures::paging::PhysFrame::from_start_address(
            x86_64::PhysAddr::new(self.kernel_cr3 & !0xfff), // Remove flags from CR3
        )
        .map_err(|_| ProcessError::OutOfMemory)?;

        let dummy_address_space = crate::memory::ProcessAddressSpace::dummy(kernel_frame);

        // Validate the addresses before creating the kernel process
        if let Err(e) = ProcessError::validate_stack_pointer(stack_ptr, ProcessType::Kernel) {
            serial_println!("Invalid stack pointer for new kernel process: {:?}", e);
            return Err(e);
        }

        if let Err(e) = ProcessError::validate_instruction_pointer(entry_point, ProcessType::Kernel)
        {
            serial_println!(
                "Invalid instruction pointer for new kernel process: {:?}",
                e
            );
            return Err(e);
        }

        let process = Process {
            pid: self.next_pid,
            state: ProcessState::Ready,
            process_type: ProcessType::Kernel,
            address_space: dummy_address_space,
            stack_pointer: stack_ptr,
            instruction_pointer: entry_point,
            registers: {
                let mut regs = RegisterState::new();
                regs.rsp = stack_ptr.as_u64();
                regs.rip = entry_point.as_u64();
                regs
            },
            has_saved_state: false,
        };

        // Final validation of the created kernel process
        if let Err(e) = process.validate() {
            serial_println!("Created kernel process failed validation: {:?}", e);
            return Err(e);
        }

        let pid = self.next_pid;
        self.processes.push(process);
        self.next_pid += 1;
        serial_println!("Created kernel process with PID: {}", pid);
        Ok(pid)
    }

    pub fn schedule_next(&mut self) -> Option<&Process> {
        // Find the next ready process
        self.processes
            .iter()
            .find(|p| p.state == ProcessState::Ready)
    }

    pub fn has_running_processes(&self) -> bool {
        self.processes
            .iter()
            .any(|p| p.state != ProcessState::Terminated)
    }

    pub fn set_current_pid(&mut self, pid: u32) {
        self.current_pid = pid;
    }

    pub fn get_current_pid(&self) -> u32 {
        self.current_pid
    }

    pub fn get_process(&self, pid: u32) -> Option<&Process> {
        self.processes.iter().find(|p| p.pid == pid)
    }

    pub fn get_process_mut(&mut self, pid: u32) -> Option<&mut Process> {
        self.processes.iter_mut().find(|p| p.pid == pid)
    }

    pub fn get_next_ready_process(&mut self) -> Option<u32> {
        // Simple round-robin scheduling: find next ready process
        let current_index = if self.current_pid == 0 {
            // No current process, start from beginning
            0
        } else {
            // Find current process index and start from next
            self.processes
                .iter()
                .position(|p| p.pid == self.current_pid)
                .map(|i| (i + 1) % self.processes.len())
                .unwrap_or(0)
        };

        // Look for a ready process starting from current_index
        for i in 0..self.processes.len() {
            let index = (current_index + i) % self.processes.len();
            if self.processes[index].state == ProcessState::Ready {
                return Some(self.processes[index].pid);
            }
        }

        // If we can't find a new process but we have a current process, return it
        if self.current_pid != 0 && self.get_process(self.current_pid).is_some() {
            return Some(self.current_pid);
        }

        None
    }

    pub fn get_current_process(&self) -> Option<&Process> {
        if self.current_pid == 0 {
            None
        } else {
            self.get_process(self.current_pid)
        }
    }

    pub fn get_current_process_mut(&mut self) -> Option<&mut Process> {
        if self.current_pid == 0 {
            None
        } else {
            self.get_process_mut(self.current_pid)
        }
    }
}

lazy_static! {
    pub static ref PROCESS_MANAGER: Mutex<ProcessManager> = Mutex::new(ProcessManager::new());
}

// Enhanced scheduling function that can save state from interrupt context
pub fn schedule() -> ! {
    serial_println!("Scheduling...");

    // Only schedule if we're not already in a critical section
    if let Some(mut pm) = PROCESS_MANAGER.try_lock() {
        if let Some(next_pid) = pm.get_next_ready_process() {
            // Clear the current process
            let current_pid = pm.current_pid;
            if let Some(current_process) = pm.get_process_mut(current_pid) {
                current_process.state = ProcessState::Ready;
            }

            // Get and validate the next process
            let mut process = {
                let next_process = pm.get_process_mut(next_pid).unwrap();

                // Validate the process before attempting to switch to it
                if let Err(e) = next_process.validate() {
                    serial_println!("Cannot schedule invalid process {}: {:?}", next_pid, e);
                    next_process.state = ProcessState::Terminated;
                    drop(pm);
                    schedule(); // Try again with the next process
                }

                next_process.state = ProcessState::Running;
                next_process.clone()
            };

            pm.current_pid = next_pid;
            drop(pm);

            serial_println!("Scheduling process {} (validated)", next_pid);
            context_switch_to(&mut process);
        } else {
            // No ready processes, switch back to kernel
            serial_println!("No ready processes, switching back to kernel");
            unsafe {
                // Ensure we switch to a valid kernel page table
                let kernel_cr3 = pm.kernel_cr3;
                if kernel_cr3 == 0 || (kernel_cr3 % 4096) != 0 {
                    panic!("Invalid kernel CR3: 0x{:x}", kernel_cr3);
                }
                asm!("mov cr3, {}", in(reg) kernel_cr3);
                x86_64::instructions::tlb::flush_all();
            }
            pm.current_pid = 0;

            // Halt the CPU until the next interrupt
            loop {
                x86_64::instructions::hlt();
            }
        }
    } else {
        // If we can't get the lock, skip this scheduling round to avoid deadlock
        serial_println!("Failed to acquire PROCESS_MANAGER lock, skipping scheduling");

        // Halt briefly and try again
        x86_64::instructions::hlt();
        schedule();
    }
}

// Function to queue a process without immediately running it
pub fn queue_user_program(
    program: &[u8],
    frame_allocator: &mut BootInfoFrameAllocator,
    physical_memory_offset: VirtAddr,
) -> Result<u32, ProcessError> {
    let mut process_manager = PROCESS_MANAGER.lock();

    match process_manager.create_process(program, frame_allocator, physical_memory_offset) {
        Ok(pid) => {
            serial_println!("Queued process with PID: {}", pid);
            Ok(pid)
        }
        Err(e) => {
            serial_println!("Failed to queue process: {:?}", e);
            Err(e)
        }
    }
}

pub fn context_switch_to(process: &mut Process) -> ! {
    serial_println!("Preparing to switch context to process {}", process.pid);

    // CRITICAL: Validate the process before any context switching
    if let Err(e) = process.validate() {
        serial_println!("Process validation failed: {:?}", e);
        panic!("Cannot switch to invalid process - would cause page fault");
    }

    // Additional validation specific to context switching
    match process.process_type {
        ProcessType::User => {
            // Ensure the address space is valid
            if let Err(e) =
                ProcessError::validate_page_table_frame(process.address_space.page_table_frame)
            {
                serial_println!("Invalid page table frame: {:?}", e);
                panic!("Cannot switch to process with invalid page table");
            }
        }
        ProcessType::Kernel => {
            // For kernel processes, ensure we have a valid kernel CR3
            let kernel_cr3 = x86_64::registers::control::Cr3::read()
                .0
                .start_address()
                .as_u64();
            if kernel_cr3 == 0 {
                panic!("Kernel CR3 is invalid");
            }
        }
    }

    match process.process_type {
        ProcessType::Kernel => {
            serial_println!("Context switching to kernel process {}", process.pid);
            perform_kernel_context_switch(process);
        }
        ProcessType::User => {
            serial_println!("Context switching to user process {}", process.pid);
            let page_table_frame = process.address_space.page_table_frame;
            perform_context_switch(page_table_frame, process);
        }
    }
}

fn perform_kernel_context_switch(process: &mut Process) -> ! {
    serial_println!("Performing kernel context switch to process");

    serial_println!("Switching to kernel process {}", process.pid);
    serial_println!(
        "Entry point: {:?}, Stack: {:?}",
        process.instruction_pointer,
        process.stack_pointer
    );

    if !process.has_saved_state {
        serial_println!(
            "First run of kernel process {}, using simple setup",
            process.pid
        );
        // For first-time kernel processes, use simple setup
        perform_kernel_first_run(process);
    } else {
        serial_println!(
            "Resuming kernel process {}, restoring full state",
            process.pid
        );
        // For resumed processes, restore full register state
        perform_kernel_resume(process);
    }
}

fn perform_kernel_first_run(process: &mut Process) -> ! {
    serial_println!("Setting up first run for kernel process {}", process.pid);

    unsafe {
        // Ensure we're using the kernel's page table
        let kernel_cr3 = x86_64::registers::control::Cr3::read()
            .0
            .start_address()
            .as_u64();
        asm!("mov cr3, {}", in(reg) kernel_cr3);
        x86_64::instructions::tlb::flush_all();

        // Get kernel selectors
        let kernel_code_sel = crate::gdt::GDT.1.code.0 as u64;
        let kernel_data_sel = crate::gdt::GDT.1.data.0 as u64;

        // Use iretq setup to ensure interrupts are enabled properly
        let temp_stack = process.stack_pointer.as_u64() - 128;

        asm!(
            "mov rsp, {temp_stack}",
            "push {ss}",      // SS
            "push {krsp}",    // RSP
            "push 0x202",     // RFLAGS (interrupts enabled)
            "push {cs}",      // CS
            "push {rip}",     // RIP
            temp_stack = in(reg) temp_stack,
            ss = in(reg) kernel_data_sel,
            krsp = in(reg) process.stack_pointer.as_u64(),
            cs = in(reg) kernel_code_sel,
            rip = in(reg) process.instruction_pointer.as_u64(),
        );

        // Set up kernel segments
        asm!(
            "mov ax, {data_sel:x}",
            "mov ds, ax",
            "mov es, ax",
            "mov fs, ax",
            "mov gs, ax",
            data_sel = in(reg) kernel_data_sel as u16,
        );

        // Clear registers for clean start
        asm!(
            "xor rax, rax",
            "xor rbx, rbx",
            "xor rcx, rcx",
            "xor rdx, rdx",
            "xor rsi, rsi",
            "xor rdi, rdi",
            "xor rbp, rbp",
            "xor r8, r8",
            "xor r9, r9",
            "xor r10, r10",
            "xor r11, r11",
            "xor r12, r12",
            "xor r13, r13",
            "xor r14, r14",
            "xor r15, r15",
        );

        // Use iretq to properly enable interrupts
        asm!("iretq", options(noreturn));
    }
}

fn perform_kernel_resume(process: &mut Process) -> ! {
    serial_println!(
        "Restoring kernel process {} with full register state",
        process.pid
    );
    serial_println!("Register state: {:?}", process.registers);

    // Validate the process before attempting to restore it
    if let Err(e) = process.validate() {
        serial_println!("Kernel process validation failed: {:?}", e);
        panic!("Cannot restore invalid kernel process state");
    }

    unsafe {
        // Disable interrupts during critical section
        x86_64::instructions::interrupts::disable();

        // Ensure we're using the kernel's page table
        let kernel_cr3 = x86_64::registers::control::Cr3::read()
            .0
            .start_address()
            .as_u64();
        asm!("mov cr3, {}", in(reg) kernel_cr3);
        x86_64::instructions::tlb::flush_all();

        // Get kernel selectors - construct proper selector values (RPL = 0 for kernel)
        let kernel_code_sel = (crate::gdt::GDT.1.code.index() << 3) as u64;
        let kernel_data_sel = (crate::gdt::GDT.1.data.index() << 3) as u64;

        // Get current stack and ensure we have enough space
        let current_stack: u64;
        asm!("mov {}, rsp", out(reg) current_stack);
        let safe_temp_stack = (current_stack - 512) & !0xF; // 16-byte align and leave plenty of space

        asm!(
            // Switch to safe temporary stack
            "mov rsp, {temp_stack}",

            // Set up kernel data segments
            "mov ax, {data_sel:x}",
            "mov ds, ax",
            "mov es, ax",
            "mov fs, ax",
            "mov gs, ax",

            // Set up iretq frame for kernel mode (pushed in reverse order)
            "push {ss}",       // SS (kernel data selector)
            "push {krsp}",     // RSP (kernel stack pointer)
            "push {rflags}",   // RFLAGS
            "push {cs}",       // CS (kernel code selector)
            "push {rip}",      // RIP (kernel instruction pointer)

            temp_stack = in(reg) safe_temp_stack,
            ss = in(reg) kernel_data_sel,
            krsp = in(reg) process.registers.rsp,
            rflags = in(reg) process.registers.rflags,
            cs = in(reg) kernel_code_sel,
            rip = in(reg) process.registers.rip,
            data_sel = in(reg) kernel_data_sel as u16,
        );

        // Restore registers in smaller groups to avoid register pressure
        asm!(
            "mov r15, {r15}",
            "mov r14, {r14}",
            "mov r13, {r13}",
            "mov r12, {r12}",
            "mov r11, {r11}",
            "mov r10, {r10}",
            "mov r9, {r9}",
            "mov r8, {r8}",
            r15 = in(reg) process.registers.r15,
            r14 = in(reg) process.registers.r14,
            r13 = in(reg) process.registers.r13,
            r12 = in(reg) process.registers.r12,
            r11 = in(reg) process.registers.r11,
            r10 = in(reg) process.registers.r10,
            r9 = in(reg) process.registers.r9,
            r8 = in(reg) process.registers.r8,
        );

        asm!(
            "mov rsi, {rsi}",
            "mov rdi, {rdi}",
            "mov rbp, {rbp}",
            "mov rdx, {rdx}",
            "mov rcx, {rcx}",
            "mov rbx, {rbx}",
            "mov rax, {rax}",
            rsi = in(reg) process.registers.rsi,
            rdi = in(reg) process.registers.rdi,
            rbp = in(reg) process.registers.rbp,
            rdx = in(reg) process.registers.rdx,
            rcx = in(reg) process.registers.rcx,
            rbx = in(reg) process.registers.rbx,
            rax = in(reg) process.registers.rax,
        );

        // Perform iretq to restore RIP, RSP, RFLAGS, and segments
        // This will also re-enable interrupts if they were enabled in saved RFLAGS
        asm!("iretq", options(noreturn));
    }
}

fn perform_context_switch(
    page_table_frame: x86_64::structures::paging::PhysFrame,
    process: &Process,
) -> ! {
    serial_println!(
        "Performing full context switch to user process {}",
        process.pid
    );

    // Validate page table frame before switching
    if let Err(e) = ProcessError::validate_page_table_frame(page_table_frame) {
        serial_println!("Invalid page table frame: {:?}", e);
        panic!("Cannot switch to process with invalid page table frame");
    }

    // Additional safety check: ensure frame address is reasonable
    let frame_addr = page_table_frame.start_address().as_u64();
    if frame_addr == 0 || (frame_addr % 4096) != 0 {
        panic!("Page table frame has invalid address: 0x{:x}", frame_addr);
    }

    serial_println!(
        "Switching to page table at physical address: 0x{:x}",
        frame_addr
    );

    // Switch to the process's page table
    unsafe {
        // Disable interrupts during critical page table switch
        x86_64::instructions::interrupts::disable();

        // Switch page table
        asm!("mov cr3, {}", in(reg) frame_addr);

        // Flush TLB to ensure page table changes take effect
        x86_64::instructions::tlb::flush_all();

        serial_println!("Successfully switched to process page table");
    }

    // Now actually switch to user mode and start executing the process
    switch_to_user_mode_direct(process);
}

fn switch_to_user_mode_direct(process: &Process) -> ! {
    serial_println!("Switching to user mode for process {}", process.pid);

    if !process.has_saved_state {
        serial_println!(
            "First run of user process {}, using simple setup",
            process.pid
        );
        switch_to_user_mode_first_run(process);
    } else {
        serial_println!(
            "Resuming user process {}, restoring full state",
            process.pid
        );
        switch_to_user_mode_resume(process);
    }
}

fn switch_to_user_mode_first_run(process: &Process) -> ! {
    serial_println!("Setting up first run for user process {}", process.pid);

    // Get user mode selectors from GDT - construct with RPL=3
    let user_code_sel = u64::from((crate::gdt::GDT.1.user_code.index() << 3) | 3);
    let user_data_sel = u64::from((crate::gdt::GDT.1.user_data.index() << 3) | 3);

    unsafe {
        // Set up segments
        asm!(
            "mov ax, {0:x}",
            "mov ds, ax",
            "mov es, ax",
            "mov fs, ax",
            "mov gs, ax",
            in(reg) user_data_sel as u16,
        );

        // Simple setup for first run - use the stack and entry point from the process
        asm!(
            // Push values for IRET (in reverse order)
            "push {user_data_sel}",    // SS
            "push {user_stack_ptr}",   // RSP
            "push 0x202",              // RFLAGS (interrupts enabled)
            "push {user_code_sel}",    // CS
            "push {user_ip}",          // RIP

            // Clear all registers for clean start
            "xor rax, rax",
            "xor rbx, rbx",
            "xor rcx, rcx",
            "xor rdx, rdx",
            "xor rsi, rsi",
            "xor rdi, rdi",
            "xor rbp, rbp",
            "xor r8, r8",
            "xor r9, r9",
            "xor r10, r10",
            "xor r11, r11",
            "xor r12, r12",
            "xor r13, r13",
            "xor r14, r14",
            "xor r15, r15",

            // Switch to user mode
            "iretq",
            user_data_sel = in(reg) user_data_sel,
            user_stack_ptr = in(reg) process.stack_pointer.as_u64(),
            user_code_sel = in(reg) user_code_sel,
            user_ip = in(reg) process.instruction_pointer.as_u64(),
            options(noreturn)
        );
    }
}

fn switch_to_user_mode_resume(process: &Process) -> ! {
    // serial_println!(
    //     "Restoring full register state for user process {}",
    //     process.pid
    // );
    // serial_println!("Register state: {:?}", process.registers);

    // Validate the process before attempting to restore it
    if let Err(e) = process.validate() {
        serial_println!("Process validation failed: {:?}", e);
        panic!("Cannot restore invalid process state");
    }

    // Get user mode selectors from GDT - construct with RPL=3
    let user_code_sel = u64::from((crate::gdt::GDT.1.user_code.index() << 3) | 3);
    let user_data_sel = u64::from((crate::gdt::GDT.1.user_data.index() << 3) | 3);

    unsafe {
        // Disable interrupts during critical section
        x86_64::instructions::interrupts::disable();

        // Set up user data segments first
        asm!(
            "mov ax, {data_sel:x}",
            "mov ds, ax",
            "mov es, ax",
            "mov fs, ax",
            "mov gs, ax",
            data_sel = in(reg) user_data_sel as u16,
        );

        // Create a temporary structure on the kernel stack with all the data we need
        // This avoids register pressure issues in the inline assembly
        #[repr(C, packed)]
        struct UserStateRestore {
            r15: u64,
            r14: u64,
            r13: u64,
            r12: u64,
            r11: u64,
            r10: u64,
            r9: u64,
            r8: u64,
            rsi: u64,
            rdi: u64,
            rbp: u64,
            rdx: u64,
            rcx: u64,
            rbx: u64,
            rax: u64,
            rip: u64,
            cs: u64,
            rflags: u64,
            rsp: u64,
            ss: u64,
        }

        let restore_data = UserStateRestore {
            r15: process.registers.r15,
            r14: process.registers.r14,
            r13: process.registers.r13,
            r12: process.registers.r12,
            r11: process.registers.r11,
            r10: process.registers.r10,
            r9: process.registers.r9,
            r8: process.registers.r8,
            rsi: process.registers.rsi,
            rdi: process.registers.rdi,
            rbp: process.registers.rbp,
            rdx: process.registers.rdx,
            rcx: process.registers.rcx,
            rbx: process.registers.rbx,
            rax: process.registers.rax,
            rip: process.registers.rip,
            cs: user_code_sel,
            rflags: process.registers.rflags,
            rsp: process.registers.rsp,
            ss: user_data_sel,
        };
        let restore_ptr = &restore_data as *const UserStateRestore;

        asm!(
            // Get the data pointer into a register
            "mov r11, {restore_ptr}",

            // Set up iretq frame first (before restoring general-purpose registers)
            "push qword ptr [r11 + 152]", // SS
            "push qword ptr [r11 + 144]", // RSP
            "push qword ptr [r11 + 136]", // RFLAGS
            "push qword ptr [r11 + 128]", // CS
            "push qword ptr [r11 + 120]", // RIP

            // Now restore all general-purpose registers
            "mov r15, qword ptr [r11 + 0]",
            "mov r14, qword ptr [r11 + 8]",
            "mov r13, qword ptr [r11 + 16]",
            "mov r12, qword ptr [r11 + 24]",
            "mov r10, qword ptr [r11 + 40]", // restore r10 before r11
            "mov r9, qword ptr [r11 + 48]",
            "mov r8, qword ptr [r11 + 56]",
            "mov rsi, qword ptr [r11 + 64]",
            "mov rdi, qword ptr [r11 + 72]",
            "mov rbp, qword ptr [r11 + 80]",
            "mov rdx, qword ptr [r11 + 88]",
            "mov rcx, qword ptr [r11 + 96]",
            "mov rbx, qword ptr [r11 + 104]",
            "mov rax, qword ptr [r11 + 112]",
            "mov r11, qword ptr [r11 + 32]", // restore r11 last

            // Execute iretq to switch to user mode
            "iretq",
            restore_ptr = in(reg) restore_ptr,
            options(noreturn)
        );
    }
}

pub fn queue_kernel_process(entry_point: fn() -> !) {
    let mut pm = PROCESS_MANAGER.lock();
    let entry_point_addr = VirtAddr::new(entry_point as *const () as u64);

    // Allocate a proper kernel stack
    const KERNEL_STACK_SIZE: usize = 4096 * 4; // 16KB stack
    static mut KERNEL_STACK: [u8; KERNEL_STACK_SIZE] = [0; KERNEL_STACK_SIZE];

    let kernel_stack = VirtAddr::from_ptr(&raw const KERNEL_STACK) + KERNEL_STACK_SIZE as u64;

    match pm.create_kernel_process(entry_point_addr, kernel_stack) {
        Ok(pid) => {
            serial_println!("Created executor kernel process with PID: {}", pid);
        }
        Err(e) => serial_println!("Failed to create executor kernel process: {:?}", e),
    }
}

pub fn kill_process(process: &mut Process) -> Result<(), ProcessError> {
    match process.process_type {
        ProcessType::Kernel => {
            // TODO: Figure out what to clean up
            process.stack_pointer = VirtAddr::zero();
            process.instruction_pointer = VirtAddr::zero();
            // process.address_space.cleanup();
        }
        ProcessType::User => {
            process.cleanup_resources();
        }
    }

    serial_println!("Process killed successfully");

    Ok(())
}

pub fn exit_current_process(exit_code: u8) {
    serial_println!("Exiting current process with exit code {}", exit_code);

    without_interrupts(|| {
        let mut pm = PROCESS_MANAGER
            .try_lock()
            .expect("Failed to acquire PROCESS_MANAGER lock");

        let current_pid = pm.current_pid;
        if current_pid == 0 {
            serial_println!("No current process to exit");
            return;
        }

        // Validate kernel CR3 before switching back to it
        let kernel_cr3 = pm.kernel_cr3;
        if kernel_cr3 == 0 || (kernel_cr3 % 4096) != 0 {
            panic!("Invalid kernel CR3 during process exit: 0x{:x}", kernel_cr3);
        }

        // Switch back to kernel page table BEFORE any cleanup
        unsafe {
            asm!("mov cr3, {}", in(reg) kernel_cr3);
            x86_64::instructions::tlb::flush_all();
            serial_println!(
                "Switched back to kernel page table (CR3: 0x{:x})",
                kernel_cr3
            );
        }

        pm.current_pid = 0;

        let index = pm
            .processes
            .iter()
            .position(|p| p.pid == current_pid)
            .expect("Current process not found");

        let mut process = pm
            .get_process_mut(current_pid)
            .expect("No current process to exit")
            .clone();

        pm.processes.remove(index);

        drop(pm); // Release the lock before calling cleanup

        kill_process(&mut process)
            .unwrap_or_else(|e| serial_println!("Failed to exit process: {:?}", e));

        serial_println!("Current process exited");
    });
}

#[inline(always)]
pub fn save_current_state(frame: &InterruptStackFrame) {
    // Try to acquire the lock, but don't block if we can't
    if let Some(mut pm) = PROCESS_MANAGER.try_lock() {
        let current_process = pm.get_current_process_mut();

        if current_process.is_none() {
            serial_println!("No current process to save state for");
            return;
        }

        let current_process = current_process.unwrap();

        // CRITICAL: Only save state if we were interrupted from user mode
        // Check if the code segment has Ring 3 privilege (user mode)
        let code_segment = frame.code_segment;
        if code_segment.rpl() != x86_64::PrivilegeLevel::Ring3 {
            serial_println!(
                "Skipping state save - interrupted from kernel mode (RPL={})",
                code_segment.rpl() as u8
            );
            return;
        }

        // Also check if we're in user process's address space by validating the IP
        let frame_ip = frame.instruction_pointer.as_u64();
        if frame_ip >= 0xFFFF800000000000 {
            serial_println!(
                "Skipping state save - instruction pointer in kernel space (0x{:x})",
                frame_ip
            );
            return;
        }

        // Validate the interrupt frame before saving
        let frame_sp = frame.stack_pointer.as_u64();
        let frame_flags = frame.cpu_flags.bits();

        // Basic validation of frame contents
        if frame_ip == 0 {
            serial_println!("Warning: saving state with null instruction pointer");
            return;
        }

        if frame_sp == 0 {
            serial_println!("Warning: saving state with null stack pointer");
            return;
        }

        // Validate that we're saving reasonable user-space addresses
        if frame_sp >= 0xFFFF800000000000 {
            serial_println!(
                "Skipping state save - stack pointer in kernel space (0x{:x})",
                frame_sp
            );
            return;
        }

        // Validate RFLAGS from interrupt frame
        if let Err(e) = ProcessError::validate_rflags(frame_flags) {
            serial_println!("Warning: invalid RFLAGS in interrupt frame: {:?}", e);
            // Continue anyway, but with corrected RFLAGS
        }

        current_process.has_saved_state = true;

        // Save general-purpose registers using inline assembly
        // This must be done very carefully to preserve the exact state
        unsafe {
            asm!(
                // Save all general-purpose registers to the RegisterState struct
                "mov qword ptr [{reg_base}], r15",      // offset 0
                "mov qword ptr [{reg_base} + 8], r14",  // offset 8
                "mov qword ptr [{reg_base} + 16], r13", // offset 16
                "mov qword ptr [{reg_base} + 24], r12", // offset 24
                "mov qword ptr [{reg_base} + 32], r11", // offset 32
                "mov qword ptr [{reg_base} + 40], r10", // offset 40
                "mov qword ptr [{reg_base} + 48], r9",  // offset 48
                "mov qword ptr [{reg_base} + 56], r8",  // offset 56
                "mov qword ptr [{reg_base} + 64], rsi", // offset 64
                "mov qword ptr [{reg_base} + 72], rdi", // offset 72
                "mov qword ptr [{reg_base} + 80], rbp", // offset 80
                "mov qword ptr [{reg_base} + 88], rdx", // offset 88
                "mov qword ptr [{reg_base} + 96], rcx", // offset 96
                "mov qword ptr [{reg_base} + 104], rbx",// offset 104
                "mov qword ptr [{reg_base} + 112], rax",// offset 112
                reg_base = in(reg) &mut current_process.registers as *mut RegisterState,
                options(nostack, preserves_flags)
            );
        }

        // Save the interrupt frame information
        current_process.registers.rip = frame_ip;
        current_process.registers.rflags = frame_flags;
        current_process.registers.rsp = frame_sp;

        // Update the process's stack and instruction pointers for consistency
        current_process.stack_pointer = VirtAddr::new(frame_sp);
        current_process.instruction_pointer = VirtAddr::new(frame_ip);

        serial_println!(
            "Saved state for process {} - IP: 0x{:x}, SP: 0x{:x} (FROM USER MODE)",
            current_process.pid,
            frame_ip,
            frame_sp
        );
    } else {
        serial_println!("Could not acquire PROCESS_MANAGER lock for state saving");
    }
}
