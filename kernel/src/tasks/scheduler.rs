use crate::errno::{Error, Result};
use crate::irq::irqsave;
use crate::tasks::register_task;
use crate::tasks::task::{NO_PRIORITIES, PriorityTaskQueue, TaskFrame, TaskPriority};
use alloc::collections::{BTreeMap, VecDeque};
use alloc::sync::Arc;
use core::sync::atomic::{AtomicU32, Ordering};
use spinning_top::Spinlock;
use x86_64::VirtAddr;
use x86_64::registers::control::Cr3;
use x86_64::structures::paging::{PhysFrame, Size4KiB};

use crate::memory::ProcessAddressSpace;
use crate::serial_println;
use crate::tasks::switch::switch;
use crate::tasks::task::{Task, TaskId, TaskStatus};

static TID_COUNTER: AtomicU32 = AtomicU32::new(0);

pub(crate) struct Scheduler {
    /// task id which is currently running
    current_task: Arc<Spinlock<Task>>,
    /// task id of the idle task
    idle_task: Arc<Spinlock<Task>>,
    /// queue of tasks, which are ready
    ready_queue: PriorityTaskQueue,
    /// queue of tasks, which are finished and can be released
    finished_tasks: VecDeque<TaskId>,
    // map between task id and task control block
    tasks: BTreeMap<TaskId, Arc<Spinlock<Task>>>,
    /// Kernel page table frame
    kernel_page_table: PhysFrame<Size4KiB>,
}

impl Scheduler {
    pub fn new() -> Scheduler {
        let tid = TaskId::from(TID_COUNTER.fetch_add(1, Ordering::SeqCst));
        let idle_task = Arc::new(Spinlock::new(Task::new_idle(tid)));
        let mut tasks = BTreeMap::new();

        tasks.insert(tid, idle_task.clone());

        let (kernel_page_table, _) = Cr3::read();

        Scheduler {
            current_task: idle_task.clone(),
            idle_task: idle_task.clone(),
            ready_queue: PriorityTaskQueue::new(),
            finished_tasks: VecDeque::<TaskId>::new(),
            tasks,
            kernel_page_table,
        }
    }

    fn get_tid(&self) -> TaskId {
        loop {
            let id = TaskId::from(TID_COUNTER.fetch_add(1, Ordering::SeqCst));

            if !self.tasks.contains_key(&id) {
                return id;
            }
        }
    }

    pub fn spawn(&mut self, func: extern "C" fn(), prio: TaskPriority) -> Result<TaskId> {
        let closure = || {
            let prio_number: usize = prio.into().into();

            if prio_number >= NO_PRIORITIES {
                return Err(Error::BadPriority);
            }

            // Create the new task.
            let tid = self.get_tid();
            let task = Arc::new(Spinlock::new(Task::new(tid, TaskStatus::Ready, prio)));

            task.lock().create_stack_frame(func);

            // Add it to the task lists.
            self.ready_queue.push(task.clone());
            self.tasks.insert(tid, task);

            serial_println!("Creating task {}", tid);

            Ok(tid)
        };

        irqsave(closure)
    }

    pub fn spawn_process(
        &mut self,
        func: extern "C" fn(),
        prio: TaskPriority,
        address_space: ProcessAddressSpace,
    ) -> Result<TaskId> {
        let closure = || {
            let prio_number: usize = prio.into().into();

            if prio_number >= NO_PRIORITIES {
                return Err(Error::BadPriority);
            }

            // Create the new task.
            let tid = self.get_tid();
            let task = Arc::new(Spinlock::new(Task::new(tid, TaskStatus::Ready, prio)));

            task.lock().address_space = Some(address_space);
            task.lock().create_stack_frame(func);

            // Add it to the task lists.
            self.ready_queue.push(task.clone());
            self.tasks.insert(tid, task);

            serial_println!("Creating process task {}", tid);

            Ok(tid)
        };

        irqsave(closure)
    }

    pub fn exit(&mut self) {
        if self.current_task.lock().status != TaskStatus::Idle {
            serial_println!("finish task with id {}", self.current_task.lock().id);
            self.current_task.lock().status = TaskStatus::Finished;
        } else {
            panic!("unable to terminate idle task");
        }
    }

    pub fn abort(&mut self) {
        if self.current_task.lock().status != TaskStatus::Idle {
            serial_println!("abort task with id {}", self.current_task.lock().id);
            self.current_task.lock().status = TaskStatus::Finished;
        } else {
            panic!("unable to terminate idle task");
        }
    }

    #[allow(dead_code)]
    pub fn block_current_task(&mut self) -> Arc<Spinlock<Task>> {
        let closure = || {
            if self.current_task.lock().status == TaskStatus::Running {
                serial_println!("block task {}", self.current_task.lock().id);

                self.current_task.lock().status = TaskStatus::Blocked;
                self.current_task.clone()
            } else {
                panic!("unable to block task {}", self.current_task.lock().id);
            }
        };

        irqsave(closure)
    }

    #[allow(dead_code)]
    pub fn wakeup_task(&mut self, task: Arc<Spinlock<Task>>) {
        let closure = || {
            if task.lock().status == TaskStatus::Blocked {
                serial_println!("wakeup task {}", task.lock().id);

                task.lock().status = TaskStatus::Ready;
                self.ready_queue.push(task.clone());
            }
        };

        irqsave(closure);
    }

    pub fn get_current_taskid(&self) -> TaskId {
        irqsave(|| self.current_task.lock().id)
    }

    /// Determines the start address of the stack
    pub fn get_current_interrupt_stack(&self) -> VirtAddr {
        irqsave(|| (*self.current_task.lock().stack).interrupt_top())
    }

    pub fn schedule(&mut self) -> Option<(*mut usize, usize, PhysFrame<Size4KiB>)> {
        // do we have finished tasks? => drop tasks => deallocate implicitly the stack
        if let Some(id) = self.finished_tasks.pop_front() {
            if self.tasks.remove(&id).is_none() {
                serial_println!("[warn] Unable to drop task {}", id);
            } else {
                serial_println!("Drop task {}", id);
            }
        }

        // Get information about the current task.
        let (current_id, current_stack_pointer, current_prio, current_status) = {
            let mut locked = self.current_task.lock();
            (
                locked.id,
                &mut locked.last_stack_pointer as *mut usize,
                locked.prio,
                locked.status,
            )
        };

        // do we have a task, which is ready?
        let mut next_task;
        if current_status == TaskStatus::Running {
            next_task = self.ready_queue.pop_with_prio(current_prio);
        } else {
            next_task = self.ready_queue.pop();
        }

        if next_task.is_none()
            && current_status != TaskStatus::Running
            && current_status != TaskStatus::Idle
        {
            serial_println!("Switch to idle task");
            // current task isn't able to run and no other task available
            // => switch to the idle task
            next_task = Some(self.idle_task.clone());
        }

        if let Some(new_task) = next_task {
            let (new_id, new_stack_pointer) = {
                let mut locked = new_task.lock();
                locked.status = TaskStatus::Running;
                (locked.id, locked.last_stack_pointer)
            };

            if current_status == TaskStatus::Running {
                self.current_task.lock().status = TaskStatus::Ready;
                self.ready_queue.push(self.current_task.clone());
            } else if current_status == TaskStatus::Finished {
                serial_println!("Task {} finished", current_id);
                self.current_task.lock().status = TaskStatus::Invalid;
                // release the task later, because the stack is required
                // to call the function "switch"
                // => push id to a queue and release the task later
                self.finished_tasks.push_back(current_id);
            }

            serial_println!(
                "Switching task from {} to {} (stack {:#X} => {:#X})",
                current_id,
                new_id,
                unsafe { *current_stack_pointer },
                new_stack_pointer
            );

            self.current_task = new_task;

            let new_cr3 = if let Some(ref space) = self.current_task.lock().address_space {
                space.page_table_frame
            } else {
                self.kernel_page_table
            };

            return Some((current_stack_pointer, new_stack_pointer, new_cr3));
        }

        None
    }
}

pub(crate) static SCHEDULER: Spinlock<Option<Scheduler>> = Spinlock::new(None);

/// Initialize module, must be called once, and only once
pub fn init() {
    *SCHEDULER.lock() = Some(Scheduler::new());

    serial_println!("Scheduler initialized");

    register_task();

    serial_println!("Scheduler task registered");
}

/// Create a new kernel task
pub fn spawn(func: extern "C" fn(), prio: TaskPriority) -> Result<TaskId> {
    SCHEDULER.lock().as_mut().unwrap().spawn(func, prio)
}

/// Create a new process task
pub fn spawn_process(
    func: extern "C" fn(),
    prio: TaskPriority,
    address_space: ProcessAddressSpace,
) -> Result<TaskId> {
    SCHEDULER
        .lock()
        .as_mut()
        .unwrap()
        .spawn_process(func, prio, address_space)
}

/// Timer interrupt  call scheduler to switch to the next available task
pub fn schedule() {
    irqsave(|| {
        let switch_info = {
            let mut guard = SCHEDULER.lock();
            if let Some(scheduler) = guard.as_mut() {
                scheduler.schedule()
            } else {
                None
            }
        };

        if let Some((old_sp, new_sp, new_cr3)) = switch_info {
            unsafe {
                let (_, flags) = Cr3::read();
                Cr3::write(new_cr3, flags);
                switch(old_sp, new_sp);
            }
        }
    });
}

/// Terminate the current running task
pub fn do_exit() -> ! {
    irqsave(|| {
        let mut guard = SCHEDULER.lock();
        if let Some(scheduler) = guard.as_mut() {
            scheduler.exit();
        }
    });

    schedule();

    loop {}
}

/// Terminate the current running task
pub fn abort() -> ! {
    irqsave(|| {
        let mut guard = SCHEDULER.lock();
        if let Some(scheduler) = guard.as_mut() {
            scheduler.abort();
        }
    });

    schedule();

    loop {}
}

pub(crate) fn get_current_interrupt_stack() -> VirtAddr {
    SCHEDULER
        .lock()
        .as_mut()
        .unwrap()
        .get_current_interrupt_stack()
}

#[allow(dead_code)]
pub(crate) fn block_current_task() -> Arc<Spinlock<Task>> {
    SCHEDULER.lock().as_mut().unwrap().block_current_task()
}

#[allow(dead_code)]
pub(crate) fn wakeup_task(task: Arc<Spinlock<Task>>) {
    SCHEDULER.lock().as_mut().unwrap().wakeup_task(task)
}

/// Get the TaskID of the current running task
pub fn get_current_taskid() -> TaskId {
    SCHEDULER.lock().as_ref().unwrap().get_current_taskid()
}
