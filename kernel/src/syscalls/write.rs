use alloc::string::String;

use crate::serial_println;

#[unsafe(no_mangle)]
pub extern "C" fn sys_write(s: *mut u8, len: usize) -> isize {
    serial_println!("enter syscall write");
    let str = unsafe { String::from_raw_parts(s, len, len) };
    serial_println!("[sys_write] {}", str);
    core::mem::forget(str);

    len as isize
}
