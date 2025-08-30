use crate::{hlt_loop, print, println, process::save_current_state, serial_println};
use core::arch::asm;
use lazy_static::lazy_static;
use pic8259::ChainedPics;
use ps2_mouse::{Mouse, MouseState};
use spin::{self, lazy::Lazy};
use spinning_top::Spinlock;
use x86_64::{
    instructions::{
        interrupts::{disable, enable, without_interrupts},
        port::PortReadOnly,
    },
    structures::idt::{InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode},
};

pub static MOUSE: Lazy<Spinlock<Mouse>> = Lazy::new(|| Spinlock::new(Mouse::new()));

pub const PIC_1_OFFSET: u8 = 32;
pub const PIC_2_OFFSET: u8 = PIC_1_OFFSET + 8;
pub const SYSCALL_INTERRUPT: u8 = 0x80;
pub const KEYBOARD_INTERRUPT: u8 = PIC_1_OFFSET + 1;
pub const MOUSE_INTERRUPT: u8 = PIC_1_OFFSET + 12;

const PROCESS_EXITED: u64 = u64::MAX; // Special value to indicate process exit

pub static PICS: spin::Mutex<ChainedPics> =
    spin::Mutex::new(unsafe { ChainedPics::new(PIC_1_OFFSET, PIC_2_OFFSET) });

#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum InterruptIndex {
    Timer = PIC_1_OFFSET,
    Keyboard = KEYBOARD_INTERRUPT,
    Mouse = MOUSE_INTERRUPT,
}

impl InterruptIndex {
    fn as_u8(self) -> u8 {
        self as u8
    }
}

lazy_static! {
    static ref IDT: InterruptDescriptorTable = {
        let mut idt = InterruptDescriptorTable::new();
        idt.breakpoint.set_handler_fn(breakpoint_handler);
        idt.page_fault.set_handler_fn(page_fault_handler);

        unsafe {
            idt.double_fault
                .set_handler_fn(double_fault_handler)
                .set_stack_index(crate::gdt::DOUBLE_FAULT_IST_INDEX);
        }

        idt[InterruptIndex::Timer.as_u8()].set_handler_fn(timer_handler);
        idt[InterruptIndex::Keyboard.as_u8()].set_handler_fn(keyboard_interrupt_handler);
        idt[InterruptIndex::Mouse.as_u8()].set_handler_fn(mouse_interrupt_handler);

        // Set up syscall handler with DPL 3 to allow user mode access
        // Use a custom gate instead of the x86-interrupt attribute
        unsafe {
            let syscall_entry = syscall_handler_asm as *const () as u64;
            idt[SYSCALL_INTERRUPT]
                .set_handler_addr(x86_64::VirtAddr::new(syscall_entry))
                .set_privilege_level(x86_64::PrivilegeLevel::Ring3);
        }

        idt.general_protection_fault.set_handler_fn(general_protection_fault_handler);
        // idt.security_exception
        //     .set_handler_fn(general_protection_fault_handler);

        idt
    };
}

pub fn init_idt() {
    IDT.load();
}

pub fn init_mouse() {
    MOUSE.lock().init().unwrap();
    MOUSE.lock().set_on_complete(on_complete);
}

// This will be fired when a packet is finished being processed.
fn on_complete(mouse_state: MouseState) {
    crate::task::mouse::add_mouse_state(mouse_state);
}

// An example interrupt based on https://os.phil-opp.com/hardware-interrupts/. The ps2 mouse is configured to fire
// interrupts at PIC offset 12.
extern "x86-interrupt" fn mouse_interrupt_handler(_stack_frame: InterruptStackFrame) {
    let mut port = PortReadOnly::new(0x60);
    let packet = unsafe { port.read() };

    // I know this is a bad practice but we are sort of forced to do this here
    // I spent 3h trying to do it otherwise but none of the solutions worked.
    without_interrupts(|| {
        MOUSE.lock().process_packet(packet);
    });

    unsafe {
        PICS.lock()
            .notify_end_of_interrupt(InterruptIndex::Mouse.as_u8());
    }
}

extern "x86-interrupt" fn general_protection_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: u64,
) {
    println!("EXCEPTION: GENERAL PROTECTION FAULT\n{:#?}", stack_frame);
    serial_println!(
        "General Protection Fault occurred. Error code: {}",
        error_code
    );
    serial_println!("{:#?}", stack_frame);

    hlt_loop();
}

extern "x86-interrupt" fn breakpoint_handler(stack_frame: InterruptStackFrame) {
    println!("EXCEPTION: BREAKPOINT\n{:#?}", stack_frame);
}

extern "x86-interrupt" fn page_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: PageFaultErrorCode,
) {
    use x86_64::registers::control::Cr2;

    println!("EXCEPTION: PAGE FAULT",);
    println!("Accessed Address: {:?}", Cr2::read());
    println!("Error Code: {:?}", error_code);
    println!("{:#?}", stack_frame);

    serial_println!("Page Fault occurred at address: {:?}", Cr2::read());
    serial_println!("Error Code: {:?}", error_code);
    serial_println!("{:#?}", stack_frame);

    hlt_loop();
}

extern "x86-interrupt" fn double_fault_handler(
    stack_frame: InterruptStackFrame,
    _error_code: u64,
) -> ! {
    serial_println!("Double fault occurred, halting the system.");
    serial_println!("Stack frame: {:#?}", stack_frame);

    println!("EXCEPTION: DOUBLE FAULT\n{:#?}", stack_frame);

    hlt_loop();
}

extern "x86-interrupt" fn timer_handler(stack_frame: InterruptStackFrame) {
    save_current_state(&stack_frame);

    print!(".");
    serial_println!("TIMER");

    // Notify the Programmable Interrupt Controller (PIC) that the interrupt has been handled
    unsafe {
        PICS.lock()
            .notify_end_of_interrupt(InterruptIndex::Timer.as_u8());
    }

    // Switch to the next task, passing the interrupt frame to save current process state
    crate::process::schedule();
}

extern "x86-interrupt" fn keyboard_interrupt_handler(_stack_frame: InterruptStackFrame) {
    use x86_64::instructions::port::Port;

    let mut port = Port::new(0x60);
    let scancode: u8 = unsafe { port.read() };
    crate::kernel_processes::keyboard::add_scancode(scancode);

    unsafe {
        PICS.lock()
            .notify_end_of_interrupt(InterruptIndex::Keyboard.as_u8());
    }
}

pub fn syscall_handler_asm() {
    unsafe {
        asm!(
            // CRITICAL: Save RCX and R11 first - they contain return address and RFLAGS
            "push rcx",      // Return RIP (set by syscall instruction)
            "push r11",      // Saved RFLAGS (set by syscall instruction)

            // Save other caller-saved registers
            "push rax",      // syscall number
            "push rbx",
            "push rdx",      // arg3
            "push rsi",      // arg2
            "push rdi",      // arg1
            "push rbp",
            "push r8",
            "push r9",
            "push r10",
            "push r12",
            "push r13",
            "push r14",
            "push r15",

            // Prepare arguments for Rust function in proper order
            // C calling convention: RDI, RSI, RDX, RCX, R8, R9
            // We want: syscall_handler_rust_debug(rax, rdi, rsi, rdx)
            // Use the stack to preserve the original values temporarily
            "push rdx",      // Save original rdx (arg3) temporarily
            "push rsi",      // Save original rsi (arg2) temporarily
            "push rdi",      // Save original rdi (arg1) temporarily
            "push rax",      // Save original rax (syscall number) temporarily

            // Set up arguments in C calling convention order (from stack)
            "pop rdi",       // 1st arg: syscall number (was in rax)
            "pop rsi",       // 2nd arg: arg1 (was in rdi)
            "pop rdx",       // 3rd arg: arg2 (was in rsi)
            "pop rcx",       // 4th arg: arg3 (was in rdx)

            "call {}",       // Call the Rust handler

            // The argument preparation above used 4 pushes/pops that balanced out
            // Now restore all saved registers in reverse order
            "pop r15",
            "pop r14",
            "pop r13",
            "pop r12",
            "pop r10",
            "pop r9",
            "pop r8",
            "pop rbp",
            "pop rdi",       // Original rdi
            "pop rsi",       // Original rsi
            "pop rdx",       // Original rdx
            "pop rbx",
            "add rsp, 8",    // Skip rax (it contains the return value, don't restore it)

            // CRITICAL: Restore RCX and R11 for sysretq
            "pop r11",       // Restore RFLAGS
            "pop rcx",       // Restore return RIP

            // Return to user mode - sysretq uses RCX (return RIP) and R11 (RFLAGS)
            "sysretq",

            sym syscall_handler_rust_debug,
            options(noreturn)
        );
    }
} // TODO Debug version to figure out correct register values
extern "C" fn syscall_handler_rust_debug(rax: u64, rdi: u64, rsi: u64, rdx: u64) -> u64 {
    serial_println!("=== SYSCALL START ===");
    serial_println!("Syscall handler (Rust) called");
    serial_println!(
        "Syscall: rax={}, rdi={}, rsi=0x{:x}, rdx={}",
        rax,
        rdi,
        rsi,
        rdx
    );

    let result = handle_syscall(rax, rdi, rsi, rdx);
    serial_println!("Syscall completed, result: {}", result);

    // Check if process exited
    if result == PROCESS_EXITED {
        serial_println!("Marking process as terminated...");

        disable();

        crate::process::exit_current_process(rdi as u8);

        serial_println!("Process marked for exit, returning to scheduler...");

        enable(); // Just to be sure

        crate::process::schedule();
    }

    serial_println!("About to return from syscall...");
    serial_println!("=== SYSCALL END ===");

    // DO NOT call schedule() here! The syscall should return directly to user mode
    // Calling schedule() here causes register corruption and double faults
    // The sysretq instruction in the assembly handler will return to user mode correctly

    result // Return the syscall result to the assembly handler
}

// Minimal syscall handler for testing
pub fn syscall_handler_asm_minimal() {
    unsafe {
        asm!(
            // Minimal syscall handler - syscall instruction already saved RCX=RIP, R11=RFLAGS
            // CRITICAL: Save RCX (return address) and R11 (RFLAGS) first!

            "push rcx",      // Save return address (MOST IMPORTANT!)
            "push r11",      // Save RFLAGS

            // Check if stack needs alignment (should be 0 after push operations)
            // Use r12 instead of r11 to avoid conflict with RFLAGS
            "mov r12, rsp",
            "and r12, 0xF",  // Check last 4 bits
            "sub rsp, r12",  // Subtract to align (0 if already aligned)
            "push r12",      // Save the alignment offset

            "push rax",      // syscall number
            "push rdi",      // arg1
            "push rsi",      // arg2
            "push rdx",      // arg3
            "push r8",       // Preserve a few more registers for safety
            "push r9",
            "push r10",

            // Arguments are already in the right registers for Rust function call
            // rdi=rax (syscall number), rsi=rdi (arg1), rdx=rsi (arg2), rcx=rdx (arg3)
            "mov rcx, rdx",  // Move arg3 to 4th parameter position
            "mov rdx, rsi",  // Move arg2 to 3rd parameter position
            "mov rsi, rdi",  // Move arg1 to 2nd parameter position
            "mov rdi, rax",  // Move syscall number to 1st parameter position

            "call {}",       // Call minimal Rust handler

            // Restore in reverse order
            "pop r10",
            "pop r9",
            "pop r8",
            "pop rdx",       // Original arg3
            "pop rsi",       // Original arg2
            "pop rdi",       // Original arg1
            // Don't restore rax - it contains the return value
            "add rsp, 8",    // Skip the saved rax

            // Restore stack alignment
            "pop r12",       // Get alignment offset
            "add rsp, r12",  // Restore original alignment

            "pop r11",       // Restore RFLAGS
            "pop rcx",       // Restore return address

            // Return to user space - RCX and R11 now have correct values
            "sysretq",            sym syscall_handler_rust_minimal,
            options(noreturn)
        );
    }
} // Minimal Rust handler
extern "C" fn syscall_handler_rust_minimal(rax: u64, rdi: u64, rsi: u64, rdx: u64) -> u64 {
    serial_println!("=== MINIMAL SYSCALL ===");
    serial_println!("Syscall {}: args({}, 0x{:x}, {})", rax, rdi, rsi, rdx);

    let result = match rax {
        1 => {
            // sys_write - just return 0 for now to keep it simple
            serial_println!("sys_write (minimal) -> 0");
            0
        }
        _ => {
            serial_println!("Unknown syscall -> MAX");
            u64::MAX
        }
    };

    serial_println!("=== MINIMAL SYSCALL DONE ===");
    result
}

fn handle_syscall(number: u64, arg1: u64, arg2: u64, arg3: u64) -> u64 {
    match number {
        1 => sys_write(arg1, arg2, arg3),
        60 => sys_exit(arg1),
        _ => {
            serial_println!("Unknown syscall: {}", number);
            u64::MAX // Error
        }
    }
}

fn sys_write(fd: u64, buf_ptr: u64, count: u64) -> u64 {
    serial_println!(
        "sys_write called: fd={}, buf_ptr=0x{:x}, count={}",
        fd,
        buf_ptr,
        count
    );

    // Validate parameters to prevent issues with garbage values
    if fd != 1 {
        serial_println!("Write to unsupported fd: {}", fd);
        return 0;
    }

    // Sanity check on count - prevent huge garbage values
    if count > 1024 * 1024 {
        // More than 1MB is suspicious
        serial_println!("Write count too large ({}), treating as 0", count);
        return 0;
    }

    // Validate buffer pointer for user space
    if buf_ptr == 0 {
        serial_println!("Write with null buffer pointer");
        return 0;
    }

    if buf_ptr >= 0xFFFF800000000000 {
        serial_println!("Write with kernel space buffer pointer: 0x{:x}", buf_ptr);
        return 0;
    }

    // stdout - for now just acknowledge the write without actually reading the buffer
    // (since we'd need to properly map user memory to read it)
    serial_println!("Write to stdout: {} bytes", count);
    count // Return number of bytes "written"
}

fn sys_exit(exit_code: u64) -> u64 {
    serial_println!("sys_exit called with code: {}", exit_code);
    serial_println!("Process exiting...");

    // Instead of immediately cleaning up, just mark the process for termination
    // The scheduler will handle the actual cleanup on the next timer tick
    serial_println!("Process marked for termination with code: {}", exit_code);

    // Return special value to indicate process exit
    PROCESS_EXITED
}

#[cfg(test)]
mod tests {
    #[test_case]
    fn test_breakpoint_exception() {
        // invoke a breakpoint exception
        x86_64::instructions::interrupts::int3();
    }
}
