#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    // Test syscalls

    // Syscall 1: write to stdout
    syscall_write(1, b"Hello from user mode!\n");

    // Test fibonacci calculation - use smaller number to avoid deep recursion
    let result = fibonacci(7);
    syscall_write(1, b"Fibonacci(7) = ");

    // Convert number to string and write it
    let mut buffer = [0u8; 20];
    let len = format_number_to_buffer(result, &mut buffer);
    syscall_write(1, &buffer[..len]);
    // syscall_write(1, b"\n");

    // Syscall 3: exit
    syscall_exit(42);
}

fn fibonacci(n: u64) -> u64 {
    if n <= 1 {
        n
    } else {
        fibonacci(n - 1) + fibonacci(n - 2)
    }
}

fn format_number_to_buffer(mut n: u64, buffer: &mut [u8]) -> usize {
    if n == 0 {
        buffer[0] = b'0';
        return 1;
    }

    let mut len = 0;
    let mut temp = n;

    // Count digits
    while temp > 0 {
        len += 1;
        temp /= 10;
    }

    // Fill buffer backwards
    let mut pos = len;
    while n > 0 && pos > 0 {
        pos -= 1;
        buffer[pos] = (n % 10) as u8 + b'0';
        n /= 10;
    }

    len
}

// Syscall implementations using the modern syscall instruction
fn syscall_write(fd: u64, buf: &[u8]) {
    unsafe {
        core::arch::asm!(
            "syscall",
            in("rax") 1u64,     // syscall number for write
            in("rdi") fd,       // file descriptor
            in("rsi") buf.as_ptr(),  // buffer
            in("rdx") buf.len(),     // length
            out("rcx") _,       // syscall clobbers RCX (return address)
            out("r11") _,       // syscall clobbers R11 (saved RFLAGS)
            options(nostack, preserves_flags)
        );
    }
}

fn syscall_exit(code: u64) -> ! {
    unsafe {
        core::arch::asm!(
            "syscall",
            in("rax") 60u64,     // syscall number for exit
            in("rdi") code,     // exit code
            options(noreturn, nostack)
        );
    };

    loop {}
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    syscall_exit(1)
}
