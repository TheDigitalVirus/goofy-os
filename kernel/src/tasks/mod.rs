pub mod scheduler;
pub mod state;
pub mod switch;
pub mod syscall;
pub mod task;

use core::arch::asm;

use crate::hlt_loop;

pub fn init() {
    scheduler::init();
}

/// Helper function to jump into the user space
///
/// # Safety
///
/// Be sure the the user-level function mapped into the user space.
pub unsafe fn jump_to_user_land(func: extern "C" fn()) -> ! {
    unsafe {
        let ds = 0x1bu64; // User data segment: (3 << 3) | 3 = 27
        let cs = 0x23u64; // User code segment: (4 << 3) | 3 = 35

        asm!(
            "push {0}",
            "push rsp",
            "add QWORD PTR [rsp], 16",
            "pushf",
            "push {1}",
            "push {2}",
            "iretq",
            in(reg) ds,
            in(reg) cs,
            in(reg) func as usize,
            options(nostack)
        );

        hlt_loop();
    }
}

pub fn register_task() {
    // The TSS is already loaded in gdt::init() via load_tss()
    // Attempting to load it again with LTR would cause a General Protection Fault
    // This function can be used for other task registration purposes if needed
}

#[macro_export]
macro_rules! syscall {
    ($arg0:expr) => {
        syscall0($arg0 as u64)
    };

    ($arg0:expr, $arg1:expr) => {
        syscall1($arg0 as u64, $arg1 as u64)
    };

    ($arg0:expr, $arg1:expr, $arg2:expr) => {
        syscall2($arg0 as u64, $arg1 as u64, $arg2 as u64)
    };

    ($arg0:expr, $arg1:expr, $arg2:expr, $arg3:expr) => {
        syscall3($arg0 as u64, $arg1 as u64, $arg2 as u64, $arg3 as u64)
    };

    ($arg0:expr, $arg1:expr, $arg2:expr, $arg3:expr, $arg4:expr) => {
        syscall4(
            $arg0 as u64,
            $arg1 as u64,
            $arg2 as u64,
            $arg3 as u64,
            $arg4 as u64,
        )
    };

    ($arg0:expr, $arg1:expr, $arg2:expr, $arg3:expr, $arg4:expr, $arg5:expr) => {
        syscall5(
            $arg0 as u64,
            $arg1 as u64,
            $arg2 as u64,
            $arg3 as u64,
            $arg4 as u64,
            $arg5 as u64,
        )
    };

    ($arg0:expr, $arg1:expr, $arg2:expr, $arg3:expr, $arg4:expr, $arg5:expr, $arg6:expr) => {
        syscall6(
            $arg0 as u64,
            $arg1 as u64,
            $arg2 as u64,
            $arg3 as u64,
            $arg4 as u64,
            $arg5 as u64,
            $arg6 as u64,
        )
    };

    ($arg0:expr, $arg1:expr, $arg2:expr, $arg3:expr, $arg4:expr, $arg5:expr, $arg6:expr, $arg7:expr) => {
        arch::x86::kernel::syscall7(
            $arg0 as u64,
            $arg1 as u64,
            $arg2 as u64,
            $arg3 as u64,
            $arg4 as u64,
            $arg5 as u64,
            $arg6 as u64,
            $arg7 as u64,
        )
    };
}

#[inline(always)]
#[allow(unused_mut)]
pub fn syscall0(arg0: u64) -> u64 {
    let mut ret: u64;
    unsafe {
        asm!("syscall",
            inlateout("rax") arg0 => ret,
            lateout("rcx") _,
            lateout("r11") _,
            options(preserves_flags, nostack)
        );
    }
    ret
}

#[inline(always)]
#[allow(unused_mut)]
pub fn syscall1(arg0: u64, arg1: u64) -> u64 {
    let mut ret: u64;
    unsafe {
        asm!("syscall",
            inlateout("rax") arg0 => ret,
            in("rdi") arg1,
            lateout("rcx") _,
            lateout("r11") _,
            options(preserves_flags, nostack)
        );
    }
    ret
}

#[inline(always)]
#[allow(unused_mut)]
pub fn syscall2(arg0: u64, arg1: u64, arg2: u64) -> u64 {
    let mut ret: u64;
    unsafe {
        asm!("syscall",
            inlateout("rax") arg0 => ret,
            in("rdi") arg1,
            in("rsi") arg2,
            lateout("rcx") _,
            lateout("r11") _,
            options(preserves_flags, nostack)
        );
    }
    ret
}

#[inline(always)]
#[allow(unused_mut)]
pub fn syscall3(arg0: u64, arg1: u64, arg2: u64, arg3: u64) -> u64 {
    let mut ret: u64;
    unsafe {
        asm!("syscall",
            inlateout("rax") arg0 => ret,
            in("rdi") arg1,
            in("rsi") arg2,
            in("rdx") arg3,
            lateout("rcx") _,
            lateout("r11") _,
            options(preserves_flags, nostack)
        );
    }
    ret
}

#[inline(always)]
#[allow(unused_mut)]
pub fn syscall4(arg0: u64, arg1: u64, arg2: u64, arg3: u64, arg4: u64) -> u64 {
    let mut ret: u64;
    unsafe {
        asm!("syscall",
            inlateout("rax") arg0 => ret,
            in("rdi") arg1,
            in("rsi") arg2,
            in("rdx") arg3,
            in("r10") arg4,
            lateout("rcx") _,
            lateout("r11") _,
            options(preserves_flags, nostack)
        );
    }
    ret
}

#[inline(always)]
#[allow(unused_mut)]
pub fn syscall5(arg0: u64, arg1: u64, arg2: u64, arg3: u64, arg4: u64, arg5: u64) -> u64 {
    let mut ret: u64;
    unsafe {
        asm!("syscall",
            inlateout("rax") arg0 => ret,
            in("rdi") arg1,
            in("rsi") arg2,
            in("rdx") arg3,
            in("r10") arg4,
            in("r8") arg5,
            lateout("rcx") _,
            lateout("r11") _,
            options(preserves_flags, nostack)
        );
    }
    ret
}

#[inline(always)]
#[allow(unused_mut)]
pub fn syscall6(
    arg0: u64,
    arg1: u64,
    arg2: u64,
    arg3: u64,
    arg4: u64,
    arg5: u64,
    arg6: u64,
) -> u64 {
    let mut ret: u64;
    unsafe {
        asm!("syscall",
            inlateout("rax") arg0 => ret,
            in("rdi") arg1,
            in("rsi") arg2,
            in("rdx") arg3,
            in("r10") arg4,
            in("r8") arg5,
            in("r9") arg6,
            lateout("rcx") _,
            lateout("r11") _,
            options(preserves_flags, nostack)
        );
    }
    ret
}
