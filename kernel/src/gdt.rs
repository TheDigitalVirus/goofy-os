use crate::Stack;
use x86_64::VirtAddr;
use x86_64::structures::gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector};
use x86_64::structures::tss::TaskStateSegment;

use crate::KERNEL_STACK;
use crate::tasks::scheduler::get_current_interrupt_stack;

pub const STACK_SIZE: usize = 1024 * 100; // 100 KB
pub const DOUBLE_FAULT_IST_INDEX: u16 = 0;
pub const PAGE_FAULT_IST_INDEX: u16 = 1;
pub const GENERAL_PROTECTION_FAULT_IST_INDEX: u16 = 2;

pub static mut TSS: TaskStateSegment = TaskStateSegment::new();
pub static mut GDT: (GlobalDescriptorTable, Selectors) = (
    GlobalDescriptorTable::new(),
    Selectors {
        code: SegmentSelector(0),
        data: SegmentSelector(0),
        tss: SegmentSelector(0),
        user_code: SegmentSelector(0),
        user_data: SegmentSelector(0),
    },
);

pub unsafe fn init_gdt() {
    unsafe {
        TSS.privilege_stack_table[0] = {
            static mut STACK: [u8; STACK_SIZE] = [0; STACK_SIZE];
            VirtAddr::from_ptr(&raw const STACK) + STACK_SIZE as u64
        };
        TSS.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] = {
            static mut STACK: [u8; STACK_SIZE] = [0; STACK_SIZE];

            let stack_start = VirtAddr::from_ptr(&raw const STACK);
            stack_start + STACK_SIZE as u64
        };
        TSS.interrupt_stack_table[PAGE_FAULT_IST_INDEX as usize] = {
            static mut STACK: [u8; STACK_SIZE] = [0; STACK_SIZE];
            VirtAddr::from_ptr(&raw const STACK) + STACK_SIZE as u64
        };
        TSS.interrupt_stack_table[GENERAL_PROTECTION_FAULT_IST_INDEX as usize] = {
            static mut STACK: [u8; STACK_SIZE] = [0; STACK_SIZE];
            VirtAddr::from_ptr(&raw const STACK) + STACK_SIZE as u64
        };

        TSS.privilege_stack_table[0] = KERNEL_STACK.get().unwrap().top();
    }

    unsafe {
        let code = GDT.0.append(Descriptor::kernel_code_segment());
        let data = GDT.0.append(Descriptor::kernel_data_segment());
        let user_data = GDT.0.append(Descriptor::user_data_segment());
        let user_code = GDT.0.append(Descriptor::user_code_segment());
        let tss = GDT.0.append(Descriptor::tss_segment(&TSS));

        GDT.1 = Selectors {
            code,
            data,
            tss,
            user_code,
            user_data,
        };
    }
}

pub struct Selectors {
    pub code: SegmentSelector,
    pub data: SegmentSelector,
    pub tss: SegmentSelector,
    pub user_code: SegmentSelector,
    pub user_data: SegmentSelector,
}

pub fn init() {
    use x86_64::instructions::segmentation::{CS, DS, SS, Segment};
    use x86_64::instructions::tables::load_tss;

    unsafe { init_gdt() };

    unsafe {
        GDT.0.load();

        CS::set_reg(GDT.1.code);
        SS::set_reg(GDT.1.data);
        DS::set_reg(GDT.1.data);
        load_tss(GDT.1.tss);
    }
}

pub unsafe extern "C" fn set_current_kernel_stack() {
    unsafe { set_kernel_stack(get_current_interrupt_stack()) };
}

#[inline(always)]
unsafe fn set_kernel_stack(stack: VirtAddr) {
    unsafe { TSS.privilege_stack_table[0] = stack };
}
