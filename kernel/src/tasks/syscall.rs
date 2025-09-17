use crate::syscalls::SYSHANDLER_TABLE;
use core::arch::{asm, naked_asm};

/// Helper function to save and to restore the register states
/// during a system call. `rax` is the system call identifier.
/// The identifier is used to determine the address of the function,
/// which implements the system call.
#[unsafe(naked)]
pub(crate) extern "C" fn syscall_handler() {
    naked_asm!(
        // save context, see x86_64 ABI
        "push rcx",
        "push rdx",
        "push rsi",
        "push rdi",
        "push r8",
        "push r9",
        "push r10",
        "push r11",
        // copy 4th argument to rcx to adhere x86_64 ABI \n\t\
        "mov rcx, r10",
        "sti",

        // Load address of SYSHANDLER_TABLE RIP-relatively, then fetch entry
        "lea rdx, [rip + {sys_handler}]",
        "mov rdx, [rdx]",           // rdx = &SYSHANDLER_TABLE (pointer to table)
        "mov rdx, [rdx + rax*8]",   // rdx = table.handle[rax]
        "call rdx",

        // restore context, see x86_64 ABI \n\t\
        "cli",
        "pop r11",
        "pop r10",
        "pop r9",
        "pop r8",
        "pop rdi",
        "pop rsi",
        "pop rdx",
        "pop rcx",
        "sysretq",
        sys_handler = sym SYSHANDLER_TABLE,
    );
}
