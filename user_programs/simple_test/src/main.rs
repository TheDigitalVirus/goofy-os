#![no_std]
#![no_main]

use core::arch::asm;
use core::panic::PanicInfo;

const SYSNO_EXIT: usize = 0;
const SYSNO_WRITE: usize = 1;

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let msg = "Hello from separate user program!\n";

    for _ in 0..20 {
        unsafe {
            syscall2(SYSNO_WRITE, msg.as_ptr() as usize, msg.len());
        }
    }

    unsafe {
        syscall0(SYSNO_EXIT);
    }
}

#[inline(always)]
unsafe fn syscall0(sysno: usize) -> ! {
    unsafe {
        asm!(
            "syscall",
            in("rax") sysno,
            options(noreturn)
        )
    }
}

#[inline(always)]
unsafe fn syscall2(sysno: usize, arg1: usize, arg2: usize) -> usize {
    let res;
    unsafe {
        asm!(
            "syscall",
            in("rax") sysno,
            in("rdi") arg1,
            in("rsi") arg2,
            lateout("rax") res,
        );
    }
    res
}
