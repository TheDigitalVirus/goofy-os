use crate::tasks::task::TaskFrame;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::rc::Rc;
use core::cell::RefCell;
use core::sync::atomic::{AtomicU32, Ordering};

use crate::serial_println;
use crate::tasks::switch::switch;
use crate::tasks::task::{Task, TaskId, TaskQueue, TaskStatus};

static TID_COUNTER: AtomicU32 = AtomicU32::new(0);

pub(crate) struct Scheduler {
    /// task id which is currently running
    current_task: Rc<RefCell<Task>>,
    /// task id of the idle task
    idle_task: Rc<RefCell<Task>>,
    /// queue of tasks, which are ready
    ready_queue: TaskQueue,
    /// queue of tasks, which are finished and can be released
    finished_tasks: VecDeque<TaskId>,
    /// map between task id and task control block
    tasks: BTreeMap<TaskId, Rc<RefCell<Task>>>,
}

impl Scheduler {
    pub fn new() -> Scheduler {
        let tid = TaskId::from(TID_COUNTER.fetch_add(1, Ordering::SeqCst));
        let idle_task = Rc::new(RefCell::new(Task::new_idle(tid)));
        let mut tasks = BTreeMap::new();

        tasks.insert(tid, idle_task.clone());

        Scheduler {
            current_task: idle_task.clone(),
            idle_task: idle_task.clone(),
            ready_queue: TaskQueue::new(),
            finished_tasks: VecDeque::new(),
            tasks,
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

    pub fn spawn(&mut self, func: extern "C" fn()) -> TaskId {
        serial_println!("Spawning new task...");

        // Create the new task.
        let tid = self.get_tid();

        serial_println!("New task id is {}", tid);

        let task = Rc::new(RefCell::new(Task::new(tid, TaskStatus::Ready)));

        serial_println!("Spawning task {}", tid);

        task.borrow_mut().create_stack_frame(func);

        serial_println!("Created stack frame for task {}", tid);

        // Add it to the task lists.
        self.ready_queue.push(task.clone());
        self.tasks.insert(tid, task);

        serial_println!("Creating task {}", tid);

        tid
    }

    pub fn exit(&mut self) -> ! {
        if self.current_task.borrow().status != TaskStatus::Idle {
            serial_println!("finish task with id {}", self.current_task.borrow().id);
            self.current_task.borrow_mut().status = TaskStatus::Finished;
        } else {
            panic!("unable to terminate idle task");
        }

        self.reschedule();

        panic!("Terminated task gets computation time");
    }

    pub fn get_current_taskid(&self) -> TaskId {
        self.current_task.borrow().id
    }

    pub fn schedule(&mut self) {
        // do we have finished tasks? => drop tasks => deallocate implicitly the stack
        while let Some(id) = self.finished_tasks.pop_front() {
            if self.tasks.remove(&id).is_none() {
                serial_println!("[warn] Unable to drop task {}", id);
            } else {
                serial_println!("Drop task {}", id);
            }
        }

        // Get information about the current task.
        let (current_id, current_stack_pointer, current_status) = {
            let mut borrowed = self.current_task.borrow_mut();
            (
                borrowed.id,
                &mut borrowed.last_stack_pointer as *mut usize,
                borrowed.status,
            )
        };

        // do we have a task, which is ready?
        let mut next_task = self.ready_queue.pop();
        if next_task.is_none()
            && current_status != TaskStatus::Running
            && current_status != TaskStatus::Idle
        {
            serial_println!("Switch to idle task");
            // current task isn't able to run and no other task available
            // => switch to the idle task
            next_task = Some(self.idle_task.clone());
        }

        if let Some(next_task) = next_task {
            let (new_id, new_stack_pointer) = {
                let mut borrowed = next_task.borrow_mut();
                borrowed.status = TaskStatus::Running;
                (borrowed.id, borrowed.last_stack_pointer)
            };

            if current_status == TaskStatus::Running {
                serial_println!("Add task {} to ready queue", current_id);
                self.current_task.borrow_mut().status = TaskStatus::Ready;
                self.ready_queue.push(self.current_task.clone());
            } else if current_status == TaskStatus::Finished {
                serial_println!("Task {} finished", current_id);
                self.current_task.borrow_mut().status = TaskStatus::Invalid;
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

            self.current_task = next_task;

            unsafe {
                switch(current_stack_pointer, new_stack_pointer);
            }
        }
    }

    pub fn reschedule(&mut self) {
        self.schedule();
    }
}

static mut SCHEDULER: Option<Scheduler> = None;

/// Initialize module, must be called once, and only once
pub(crate) fn init() {
    unsafe {
        SCHEDULER = Some(Scheduler::new());
    }
}

/// Create a new kernel task
pub fn spawn(func: extern "C" fn()) -> TaskId {
    unsafe { SCHEDULER.as_mut().unwrap().spawn(func) }
}

/// Trigger the scheduler to switch to the next available task
pub fn reschedule() {
    unsafe { SCHEDULER.as_mut().unwrap().reschedule() }
}

/// Terminate the current running task
pub fn do_exit() -> ! {
    unsafe {
        SCHEDULER.as_mut().unwrap().exit();
    }
}

/// Get the TaskID of the current running task
pub fn get_current_taskid() -> TaskId {
    unsafe { SCHEDULER.as_ref().unwrap().get_current_taskid() }
}
