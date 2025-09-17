use crate::{serial_println, tasks::scheduler::do_exit};

#[unsafe(no_mangle)]
pub extern "C" fn sys_exit() {
    serial_println!("enter syscall exit");
    do_exit();
}
