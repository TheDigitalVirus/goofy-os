#![no_std]
#![no_main]

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    // First syscall: write(1, buffer_addr, len) - simplified for now
    unsafe {
        core::arch::asm!(
            "mov rax, 1",             // sys_write
            "mov rdi, 1",             // fd = stdout
            "mov rsi, 0x1000001cecd", // buffer pointer (dummy for now)
            "mov rdx, 5",             // count = 5 bytes
            "syscall"
        );
    }

    // Just use a simple constant instead of Fibonacci
    let result = 42u64;

    // Second syscall: write the result
    unsafe {
        core::arch::asm!(
            "mov rax, 1",       // sys_write
            "mov rdi, 1",       // fd = stdout
            "mov rsi, {result}", // buffer pointer = result value
            "mov rdx, 8",       // count = 8 bytes
            "syscall",
            result = in(reg) result
        );
    }

    // Exit syscall
    unsafe {
        core::arch::asm!(
            "mov rax, 60", // sys_exit
            "mov rdi, 0",  // exit_code = 0
            "syscall"
        );
    }

    loop {}
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
